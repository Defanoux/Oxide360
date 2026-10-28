//! GPU rendering of solids with a real depth buffer (wgpu).
//!
//! Each frame the bodies are drawn into an offscreen image with a depth buffer
//! and 4x multisampling (smooth edges), then that image is copied onto egui's
//! screen inside the viewport rectangle. Depth testing makes every overlap
//! correct — no draw-order guessing — and the work happens on the GPU.
//!
//! This file only depends on wgpu and glam, so it is tested headless.

use crate::camera::OrbitCamera;
use crate::model::{Mesh, mesh_edges};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

const MSAA: u32 = 4;
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const TRI_STRIDE: u64 = 48; // pos, normal, colour, finish: 4 × vec3<f32>
const EDGE_STRIDE: u64 = 12; // pos: vec3<f32>
const LINE_STRIDE: u64 = 28; // pos: vec3<f32>, colour: vec4<f32>
const UNIFORM_SIZE: u64 = 112; // mat4 + 3 × vec4

const SCENE_WGSL: &str = r#"
struct U { view_proj: mat4x4<f32>, head: vec4<f32>, key: vec4<f32>, edge: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;

struct TriOut { @builtin(position) clip: vec4<f32>, @location(0) color: vec4<f32> };

// finish: (metal, gloss, opacity); (0, 0, 1) is the plain look.
@vertex
fn vs_tri(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>, @location(3) finish: vec3<f32>) -> TriOut {
    var o: TriOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    let n = normalize(normal);
    let metal = finish.x;
    let gloss = finish.y;
    let k = 0.30 + 0.50 * max(dot(n, u.head.xyz), 0.0) + 0.30 * max(dot(n, u.key.xyz), 0.0);
    // The surroundings a face mirrors (a studio: light above, dark floor),
    // blurred toward grey the less glossy it is. The headlight is the view.
    let nv = max(dot(n, u.head.xyz), 1e-4);
    let r = 2.0 * nv * n - u.head.xyz;
    let sky = mix(0.95, 0.7, clamp(r.z, 0.0, 1.0));
    let env = mix(0.5, mix(0.22, sky, smoothstep(-0.08, 0.08, r.z)), gloss);
    // Metals mirror it tinted by their colour; the rest a little, more at grazing angles.
    let mirror = metal * 0.85 + (1.0 - metal) * gloss * (0.08 + 0.4 * pow(1.0 - nv, 5.0));
    let diffuse = color * k * (1.0 - 0.75 * metal);
    let tint = mix(vec3<f32>(1.0), color, metal);
    // Highlights from both lights; glossier = smaller and brighter.
    let power = exp2(3.0 + 6.0 * gloss);
    let half_key = normalize(u.key.xyz + u.head.xyz);
    let spec = gloss * (0.6 * pow(max(dot(n, half_key), 1e-4), power) + 0.2 * pow(nv, power));
    o.color = vec4<f32>(min(diffuse + tint * (env * mirror + spec), vec3<f32>(1.0)), finish.z);
    return o;
}

@fragment
fn fs_tri(i: TriOut) -> @location(0) vec4<f32> {
    return i.color;
}

@vertex
fn vs_edge(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return u.view_proj * vec4<f32>(pos, 1.0);
}

@fragment
fn fs_edge() -> @location(0) vec4<f32> {
    return u.edge;
}

struct LineOut { @builtin(position) clip: vec4<f32>, @location(0) color: vec4<f32> };

@vertex
fn vs_line(@location(0) pos: vec3<f32>, @location(1) color: vec4<f32>) -> LineOut {
    var o: LineOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    o.color = color;
    return o;
}

@fragment
fn fs_line(i: LineOut) -> @location(0) vec4<f32> {
    return i.color;
}
"#;

const BLIT_WGSL: &str = r#"
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

struct BOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

// One triangle that covers the whole viewport.
@vertex
fn vs_blit(@builtin(vertex_index) i: u32) -> BOut {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    var o: BOut;
    o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    o.uv = vec2<f32>(x, y);
    return o;
}

@fragment
fn fs_blit(i: BOut) -> @location(0) vec4<f32> {
    return textureSample(tex, samp, i.uv);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let lo = c / 12.92;
    let hi = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(hi, lo, c <= vec3<f32>(0.04045));
}

// For sRGB screens: egui colours are gamma-encoded, so decode before the GPU re-encodes.
@fragment
fn fs_blit_srgb(i: BOut) -> @location(0) vec4<f32> {
    let c = textureSample(tex, samp, i.uv);
    if (c.a <= 0.0) { return vec4<f32>(0.0); }
    return vec4<f32>(to_linear(c.rgb / c.a) * c.a, c.a);
}
"#;

/// How bodies are drawn (Fusion's Visual Style).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Style {
    Shaded,
    /// Shaded, with the edges you can see (the usual look).
    #[default]
    ShadedEdges,
    /// Shaded, with the edges behind showing faintly through.
    ShadedHidden,
    /// Edges only (corners and outlines), no faces.
    Wireframe,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Shaded, Style::ShadedEdges, Style::ShadedHidden, Style::Wireframe];

    pub fn name(self) -> &'static str {
        match self {
            Style::Shaded => "Shaded",
            Style::ShadedEdges => "Shaded with Visible Edges",
            Style::ShadedHidden => "Shaded with Hidden Edges",
            Style::Wireframe => "Wireframe",
        }
    }
}

/// How a body's surface looks besides its colour (from its appearance).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Finish {
    /// 0..1: 1 for metals.
    pub metal: f32,
    /// 0..1: highlight size and strength.
    pub gloss: f32,
    /// 0..1: below 1 the body is see-through.
    pub opacity: f32,
}

impl Finish {
    /// No appearance: the usual flat shading.
    pub const PLAIN: Finish = Finish { metal: 0.0, gloss: 0.0, opacity: 1.0 };
}

/// One frame's worth of geometry, already flattened for the GPU.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub view_proj: Mat4,
    pub head_light: Vec3,
    pub key_light: Vec3,
    pub edge_color: [f32; 4],
    /// Per vertex: position, normal, colour (gamma 0..1), finish (metal,
    /// gloss, opacity). Three per triangle.
    pub tris: Vec<[f32; 12]>,
    /// Line list: pairs of positions.
    pub edges: Vec<[f32; 3]>,
    /// Coloured, depth-tested lines (grid, axes): pairs of position + RGBA.
    pub lines: Vec<[f32; 7]>,
    /// Translucent triangles (origin planes): position + RGBA, depth-tested
    /// against the bodies but not written to the depth buffer.
    pub glass: Vec<[f32; 7]>,
    /// Lines drawn over everything, seen through the bodies (hidden edges):
    /// position + RGBA.
    pub xray: Vec<[f32; 7]>,
}

impl Scene {
    /// Flatten meshes for drawing. Edges are the same ones the CPU renderer
    /// drew (sharp corners + silhouettes), nudged toward the viewer so they
    /// win the depth test against their own faces but stay hidden behind
    /// other geometry.
    pub fn build(meshes: &[(&Mesh, [f32; 3], Finish)], cam: &OrbitCamera, aspect: f32, style: Style) -> Scene {
        let mut s = Scene {
            view_proj: cam.view_proj(aspect),
            head_light: -cam.basis().0,
            key_light: Vec3::new(0.35, -0.45, 0.82).normalize(),
            // Dark lines on the faces; with no faces (wireframe), light ones on the dark background.
            edge_color: if style == Style::Wireframe { [0.78, 0.81, 0.86, 1.0] } else { [25.0 / 255.0, 28.0 / 255.0, 34.0 / 255.0, 1.0] },
            ..Default::default()
        };
        let nudge = cam.distance * 0.002;
        for (mesh, color, finish) in meshes {
            let front: Vec<bool> = (0..mesh.tris.len())
                .map(|t| {
                    let [a, b, c] = mesh.tris[t].map(|i| mesh.positions[i as usize]);
                    mesh.normal(t).dot(cam.to_eye((a + b + c) / 3.0)) > 0.0
                })
                .collect();
            if style != Style::Wireframe {
                for (t, tri) in mesh.tris.iter().enumerate() {
                    let n = mesh.normal(t);
                    for &i in tri {
                        let p = mesh.positions[i as usize];
                        s.tris.push([p.x, p.y, p.z, n.x, n.y, n.z, color[0], color[1], color[2], finish.metal, finish.gloss, finish.opacity]);
                    }
                }
            }
            for e in mesh_edges(mesh) {
                let (f0, f1) = (front[e.tris[0]], front[e.tris[1]]);
                let outline = f0 != f1;
                let seen = outline || (e.sharp && (f0 || f1));
                let (a, b) = (mesh.positions[e.a as usize], mesh.positions[e.b as usize]);
                match style {
                    Style::Shaded => {}
                    // Wireframe: every corner edge and the outline, nothing hides them.
                    Style::Wireframe => {
                        if e.sharp || outline {
                            s.edges.extend([[a.x, a.y, a.z], [b.x, b.y, b.z]]);
                        }
                    }
                    Style::ShadedEdges | Style::ShadedHidden => {
                        if seen {
                            for p in [a, b] {
                                let q = p + cam.to_eye(p) * nudge;
                                s.edges.push([q.x, q.y, q.z]);
                            }
                        }
                        // The corner edges behind, faint, through the faces.
                        if style == Style::ShadedHidden && e.sharp && !seen {
                            let c = [0.12, 0.13, 0.16, 0.45];
                            s.xray.extend([a, b].map(|p| [p.x, p.y, p.z, c[0], c[1], c[2], c[3]]));
                        }
                    }
                }
            }
        }
        s
    }

    /// Colour some triangles of the mesh whose triangles start at triangle
    /// `first` (Section Analysis's cut faces).
    pub fn tint(&mut self, first: usize, which: &[bool], color: [f32; 3]) {
        for (t, _) in which.iter().enumerate().filter(|(_, on)| **on) {
            for v in 0..3 {
                if let Some(x) = self.tris.get_mut((first + t) * 3 + v) {
                    x[6..9].copy_from_slice(&color);
                }
            }
        }
    }

    /// Give some triangles of the mesh whose triangles start at triangle
    /// `first` a look of their own (faces with their own appearance).
    pub fn restyle(&mut self, first: usize, which: &[bool], color: [f32; 3], finish: Finish) {
        for (t, _) in which.iter().enumerate().filter(|(_, on)| **on) {
            for v in 0..3 {
                if let Some(x) = self.tris.get_mut((first + t) * 3 + v) {
                    x[6..12].copy_from_slice(&[color[0], color[1], color[2], finish.metal, finish.gloss, finish.opacity]);
                }
            }
        }
    }

    /// Add depth-tested lines such as the grid. They're nudged a hair toward
    /// the viewer, so a line lying exactly on a face (the grid under a part
    /// seen from below) shows on that face, while anything properly in front
    /// of it still hides it — like lines drawn on a clear sheet.
    pub fn add_lines(&mut self, segments: &[(Vec3, Vec3, [f32; 4])], cam: &OrbitCamera) {
        let nudge = cam.distance * 0.001;
        for &(a, b, c) in segments {
            for p in [a, b] {
                let q = p + cam.to_eye(p) * nudge;
                self.lines.push([q.x, q.y, q.z, c[0], c[1], c[2], c[3]]);
            }
        }
    }

    /// Add translucent triangles. They're pushed a hair away from the viewer,
    /// so a body face lying in the same plane covers them cleanly.
    pub fn add_glass(&mut self, tris: &[([Vec3; 3], [f32; 4])], cam: &OrbitCamera) {
        let nudge = cam.distance * 0.0005;
        for &(corners, c) in tris {
            for p in corners {
                let q = p - cam.to_eye(p) * nudge;
                self.glass.push([q.x, q.y, q.z, c[0], c[1], c[2], c[3]]);
            }
        }
    }

    fn uniform_bytes(&self) -> Vec<u8> {
        let mut v: Vec<f32> = self.view_proj.to_cols_array().to_vec();
        v.extend([self.head_light.x, self.head_light.y, self.head_light.z, 0.0]);
        v.extend([self.key_light.x, self.key_light.y, self.key_light.z, 0.0]);
        v.extend(self.edge_color);
        v.iter().flat_map(|f| f.to_le_bytes()).collect()
    }
}

/// Solid triangles, then see-through ones (drawn after, blended, not hiding
/// what's behind them).
fn split_clear(tris: &[[f32; 12]]) -> (Vec<[f32; 12]>, Vec<[f32; 12]>) {
    tris.iter().partition(|v| v[11] >= 1.0)
}

fn f32_bytes<const N: usize>(items: &[[f32; N]]) -> Vec<u8> {
    items.iter().flatten().flat_map(|f| f.to_le_bytes()).collect()
}

struct Targets {
    size: [u32; 2],
    msaa: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::Texture,
    blit_group: wgpu::BindGroup,
}

/// A vertex buffer that grows as needed.
struct GrowBuffer {
    buffer: wgpu::Buffer,
    capacity: u64,
}

impl GrowBuffer {
    fn new(device: &wgpu::Device, label: &str) -> Self {
        let capacity = 64 * 1024;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { buffer, capacity }
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if data.len() as u64 > self.capacity {
            self.capacity = (data.len() as u64).next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("oxide360 vertices"),
                size: self.capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        queue.write_buffer(&self.buffer, 0, data);
    }
}

pub struct Gpu {
    tri_pipeline: wgpu::RenderPipeline,
    clear_pipeline: wgpu::RenderPipeline,
    edge_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    scene_group: wgpu::BindGroup,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    tris: GrowBuffer,
    clear: GrowBuffer,
    clear_count: u32,
    edges: GrowBuffer,
    lines: GrowBuffer,
    glass: GrowBuffer,
    xray_pipeline: wgpu::RenderPipeline,
    xray: GrowBuffer,
    counts: (u32, u32, u32, u32),
    xray_count: u32,
    targets: Option<Targets>,
}

impl Gpu {
    /// `screen_format` is the format of the surface the image is copied onto.
    pub fn new(device: &wgpu::Device, screen_format: wgpu::TextureFormat) -> Self {
        let scene_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oxide360 scene"),
            source: wgpu::ShaderSource::Wgsl(SCENE_WGSL.into()),
        });
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("oxide360 blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });

        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("oxide360 uniforms"),
            contents: &[0u8; UNIFORM_SIZE as usize],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oxide360 scene layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(UNIFORM_SIZE),
                },
                count: None,
            }],
        });
        let scene_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oxide360 scene group"),
            layout: &scene_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let scene_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oxide360 scene pipeline layout"),
            bind_group_layouts: &[Some(&scene_layout)],
            immediate_size: 0,
        });

        let depth = |write: bool| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let msaa = wgpu::MultisampleState { count: MSAA, mask: !0, alpha_to_coverage_enabled: false };
        let opaque = [Some(wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];

        let tri_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x3];
        let blended = [Some(wgpu::ColorTargetState {
            format: COLOR_FORMAT,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let tri_pipeline_with = |label: &str, solid: bool| device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &scene_module,
                entry_point: Some("vs_tri"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: TRI_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &tri_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(depth(solid)),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some("fs_tri"),
                compilation_options: Default::default(),
                targets: if solid { &opaque } else { &blended },
            }),
            multiview_mask: None,
            cache: None,
        });
        let tri_pipeline = tri_pipeline_with("oxide360 triangles", true);
        // See-through bodies (glass): blended over what's behind, hiding nothing.
        let clear_pipeline = tri_pipeline_with("oxide360 clear triangles", false);

        let edge_attrs = wgpu::vertex_attr_array![0 => Float32x3];
        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oxide360 edges"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &scene_module,
                entry_point: Some("vs_edge"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: EDGE_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &edge_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(depth(false)),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some("fs_edge"),
                compilation_options: Default::default(),
                targets: &opaque,
            }),
            multiview_mask: None,
            cache: None,
        });

        let line_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4];
        let line_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oxide360 lines"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &scene_module,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: LINE_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &line_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(depth(false)),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some("fs_line"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        // Hidden edges: lines that ignore the depth buffer (drawn after the bodies).
        let xray_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oxide360 hidden edges"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &scene_module,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: LINE_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &line_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { depth_compare: Some(wgpu::CompareFunction::Always), ..depth(false) }),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some("fs_line"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oxide360 glass"),
            layout: Some(&scene_pl),
            vertex: wgpu::VertexState {
                module: &scene_module,
                entry_point: Some("vs_line"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: LINE_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &line_attrs,
                })],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
            depth_stencil: Some(depth(false)),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some("fs_line"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOR_FORMAT,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("oxide360 blit layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let blit_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("oxide360 blit pipeline layout"),
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("oxide360 blit"),
            layout: Some(&blit_pl),
            vertex: wgpu::VertexState {
                module: &blit_module,
                entry_point: Some("vs_blit"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_module,
                entry_point: Some(if screen_format.is_srgb() { "fs_blit_srgb" } else { "fs_blit" }),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: screen_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("oxide360 blit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Gpu {
            tri_pipeline,
            clear_pipeline,
            edge_pipeline,
            line_pipeline,
            glass_pipeline,
            blit_pipeline,
            uniforms,
            scene_group,
            blit_layout,
            sampler,
            tris: GrowBuffer::new(device, "oxide360 triangles"),
            clear: GrowBuffer::new(device, "oxide360 clear triangles"),
            clear_count: 0,
            edges: GrowBuffer::new(device, "oxide360 edges"),
            lines: GrowBuffer::new(device, "oxide360 lines"),
            glass: GrowBuffer::new(device, "oxide360 glass"),
            xray_pipeline,
            xray: GrowBuffer::new(device, "oxide360 hidden edges"),
            counts: (0, 0, 0, 0),
            xray_count: 0,
            targets: None,
        }
    }

    fn ensure_targets(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 };
        let tex = |label: &str, format, samples, usage| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent,
                mip_level_count: 1,
                sample_count: samples,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let msaa = tex("oxide360 msaa", COLOR_FORMAT, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT)
            .create_view(&Default::default());
        let depth = tex("oxide360 depth", DEPTH_FORMAT, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT)
            .create_view(&Default::default());
        let resolved = tex(
            "oxide360 resolved",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let resolved_view = resolved.create_view(&Default::default());
        let blit_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("oxide360 blit group"),
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&resolved_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        self.targets = Some(Targets { size, msaa, depth, resolved, blit_group });
    }

    /// Draw the scene into the offscreen image (`size` in physical pixels).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        scene: &Scene,
    ) {
        if size[0] == 0 || size[1] == 0 {
            return;
        }
        self.ensure_targets(device, size);
        queue.write_buffer(&self.uniforms, 0, &scene.uniform_bytes());
        let (solid, clear) = split_clear(&scene.tris);
        self.tris.upload(device, queue, &f32_bytes(&solid));
        self.clear.upload(device, queue, &f32_bytes(&clear));
        self.clear_count = clear.len() as u32;
        self.edges.upload(device, queue, &f32_bytes(&scene.edges));
        self.lines.upload(device, queue, &f32_bytes(&scene.lines));
        self.glass.upload(device, queue, &f32_bytes(&scene.glass));
        self.counts = (solid.len() as u32, scene.edges.len() as u32, scene.lines.len() as u32, scene.glass.len() as u32);
        self.xray.upload(device, queue, &f32_bytes(&scene.xray));
        self.xray_count = scene.xray.len() as u32;

        let t = self.targets.as_ref().expect("targets were just created");
        let resolved_view = t.resolved.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("oxide360 scene"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &t.msaa,
                depth_slice: None,
                resolve_target: Some(&resolved_view),
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Discard },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &t.depth,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        pass.set_bind_group(0, &self.scene_group, &[]);
        if self.counts.0 > 0 {
            pass.set_pipeline(&self.tri_pipeline);
            pass.set_vertex_buffer(0, self.tris.buffer.slice(..self.counts.0 as u64 * TRI_STRIDE));
            pass.draw(0..self.counts.0, 0..1);
        }
        if self.clear_count > 0 {
            pass.set_pipeline(&self.clear_pipeline);
            pass.set_vertex_buffer(0, self.clear.buffer.slice(..self.clear_count as u64 * TRI_STRIDE));
            pass.draw(0..self.clear_count, 0..1);
        }
        if self.counts.3 > 0 {
            pass.set_pipeline(&self.glass_pipeline);
            pass.set_vertex_buffer(0, self.glass.buffer.slice(..self.counts.3 as u64 * LINE_STRIDE));
            pass.draw(0..self.counts.3, 0..1);
        }
        if self.xray_count > 0 {
            pass.set_pipeline(&self.xray_pipeline);
            pass.set_vertex_buffer(0, self.xray.buffer.slice(..self.xray_count as u64 * LINE_STRIDE));
            pass.draw(0..self.xray_count, 0..1);
        }
        if self.counts.1 > 0 {
            pass.set_pipeline(&self.edge_pipeline);
            pass.set_vertex_buffer(0, self.edges.buffer.slice(..self.counts.1 as u64 * EDGE_STRIDE));
            pass.draw(0..self.counts.1, 0..1);
        }
        if self.counts.2 > 0 {
            pass.set_pipeline(&self.line_pipeline);
            pass.set_vertex_buffer(0, self.lines.buffer.slice(..self.counts.2 as u64 * LINE_STRIDE));
            pass.draw(0..self.counts.2, 0..1);
        }
    }

    /// Copy the rendered image into the current render pass's viewport.
    pub fn blit(&self, pass: &mut wgpu::RenderPass<'_>) {
        if let Some(t) = &self.targets {
            pass.set_pipeline(&self.blit_pipeline);
            pass.set_bind_group(0, &t.blit_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// The rendered image (used by the tests to read pixels back).
    #[cfg(test)]
    pub fn image(&self) -> Option<&wgpu::Texture> {
        self.targets.as_ref().map(|t| &t.resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Projection;
    use crate::model::{Extrude, build_extrude};
    use crate::sketch::{Anchor, Editor, OriginPlane, Sketch, Tool};
    use glam::DVec2;

    fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).ok()?;
        pollster::block_on(adapter.request_device(&Default::default())).ok()
    }

    fn rect_body(x0: f64, y0: f64, x1: f64, y1: f64, height: f64) -> Mesh {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for p in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(p.0, p.1)));
        }
        ed.click(&mut sk, Anchor::Point(1));
        build_extrude(&[sk], &Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: height, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None }).unwrap()
    }

    /// Render and read the image back as RGBA bytes.
    fn render_pixels(scene: &Scene, size: [u32; 2], target: wgpu::TextureFormat) -> Option<Vec<u8>> {
        let (device, queue) = device()?;
        let mut gpu = Gpu::new(&device, target);
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.render(&device, &queue, &mut enc, size, scene);
        let row = (size[0] * 4).div_ceil(256) * 256;
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: (row * size[1]) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: gpu.image().unwrap(),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        buf.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buf.get_mapped_range(..).unwrap();
        let mut out = Vec::new();
        for y in 0..size[1] {
            let start = (y * row) as usize;
            out.extend_from_slice(&data[start..start + size[0] as usize * 4]);
        }
        Some(out)
    }

    fn top_camera() -> OrbitCamera {
        let mut cam = OrbitCamera::default();
        cam.projection = Projection::Orthographic;
        cam.look_from(-90.0, 90.0);
        cam.distance = 120.0;
        cam
    }

    #[test]
    fn nearer_body_hides_farther_one() {
        // A tall red box and a short green box overlap in the middle.
        // From above, the tall one (nearer) must win where they overlap,
        // regardless of the order they're drawn in.
        let tall = rect_body(-20.0, -20.0, 20.0, 20.0, 30.0);
        let short = rect_body(-40.0, -10.0, 40.0, 10.0, 5.0);
        let cam = top_camera();
        for order in [[0usize, 1], [1, 0]] {
            let bodies = [(&tall, [1.0, 0.0, 0.0], Finish::PLAIN), (&short, [0.0, 1.0, 0.0], Finish::PLAIN)];
            let list: Vec<(&Mesh, [f32; 3], Finish)> = order.iter().map(|&i| bodies[i]).collect();
            let scene = Scene::build(&list, &cam, 1.0, Style::ShadedEdges);
            let Some(px) = render_pixels(&scene, [64, 64], wgpu::TextureFormat::Rgba8Unorm) else {
                eprintln!("no GPU adapter available; skipping");
                return;
            };
            let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
            let centre = at(32, 32);
            assert!(centre[0] > 100 && centre[1] < 30 && centre[3] == 255, "centre is the tall red box: {centre:?}");
            // View is ±49.7 mm wide; pixel 10 is at x ≈ -33 mm: inside the green box only.
            let side = at(10, 32);
            assert!(side[1] > 100 && side[0] < 30, "edge of view is the green box: {side:?}");
            let empty = at(2, 2);
            assert_eq!(empty[3], 0, "background stays transparent");
        }
    }

    #[test]
    fn blit_copies_the_image_onto_the_screen_format() {
        let Some((device, queue)) = device() else { return };
        // Both screen-format variants must build valid pipelines.
        let _ = Gpu::new(&device, wgpu::TextureFormat::Bgra8UnormSrgb);
        let mut gpu = Gpu::new(&device, wgpu::TextureFormat::Rgba8Unorm);
        let tall = rect_body(-20.0, -20.0, 20.0, 20.0, 30.0);
        let scene = Scene::build(&[(&tall, [1.0, 0.0, 0.0], Finish::PLAIN)], &top_camera(), 1.0, Style::ShadedEdges);
        let mut enc = device.create_command_encoder(&Default::default());
        gpu.render(&device, &queue, &mut enc, [32, 32], &scene);
        // A stand-in for egui's screen: opaque blue, then the image on top.
        let screen = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d { width: 32, height: 32, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = screen.create_view(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLUE), store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            });
            gpu.blit(&mut pass);
        }
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 32,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo { texture: &screen, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            wgpu::TexelCopyBufferInfo {
                buffer: &buf,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: None },
            },
            wgpu::Extent3d { width: 32, height: 32, depth_or_array_layers: 1 },
        );
        queue.submit([enc.finish()]);
        buf.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buf.get_mapped_range(..).unwrap();
        let px = |x: usize, y: usize| [data[y * 256 + x * 4], data[y * 256 + x * 4 + 1], data[y * 256 + x * 4 + 2]];
        assert!(px(16, 16)[0] > 100 && px(16, 16)[2] < 30, "body shows through: {:?}", px(16, 16));
        assert_eq!(px(1, 1), [0, 0, 255], "background keeps egui's pixels (grid etc.)");
    }

    #[test]
    fn grid_is_hidden_by_parts_above_it_but_shows_from_below() {
        // A red block standing on the XY plane and a blue "grid line" on that
        // plane running under it and out past both sides.
        let block = rect_body(-20.0, -20.0, 20.0, 20.0, 20.0);
        let blue = [0.0, 0.0, 1.0, 1.0];
        let line = [(Vec3::new(-45.0, 0.0, 0.0), Vec3::new(45.0, 0.0, 0.0), blue)];
        let look = |pitch: f32| {
            let mut cam = top_camera();
            cam.look_from(-90.0, pitch);
            let mut scene = Scene::build(&[(&block, [1.0, 0.0, 0.0], Finish::PLAIN)], &cam, 1.0, Style::ShadedEdges);
            scene.add_lines(&line, &cam);
            render_pixels(&scene, [64, 64], wgpu::TextureFormat::Rgba8Unorm)
        };
        // The 1-px line falls between the two middle rows, so with anti-aliasing
        // it shows as half-strength blue on both. Add the two rows up.
        let blue = |px: &[u8], x: usize| [31usize, 32].iter().map(|&y| px[(y * 64 + x) * 4 + 2] as u32).sum::<u32>();
        let Some(from_top) = look(90.0) else { return };
        assert_eq!(blue(&from_top, 32), 0, "from above, the block hides the grid under it");
        assert!(blue(&from_top, 4) >= 200, "beyond the block the grid is visible");
        let from_below = look(-90.0).unwrap();
        assert!(blue(&from_below, 32) >= 200, "from below, the grid shows on the block's bottom face");
        assert!(blue(&from_below, 4) >= 200);
    }

    #[test]
    fn glass_sits_behind_bodies_even_when_lying_in_their_face() {
        let block = rect_body(-20.0, -20.0, 20.0, 20.0, 20.0);
        let blue = [0.0, 0.0, 1.0, 1.0];
        let quad_at = |z: f32| {
            let c = |x: f32, y: f32| Vec3::new(x, y, z);
            [([c(-45.0, -45.0), c(45.0, -45.0), c(45.0, 45.0)], blue), ([c(-45.0, -45.0), c(45.0, 45.0), c(-45.0, 45.0)], blue)]
        };
        let look = |z: f32| {
            let cam = top_camera();
            let mut scene = Scene::build(&[(&block, [1.0, 0.0, 0.0], Finish::PLAIN)], &cam, 1.0, Style::ShadedEdges);
            scene.add_glass(&quad_at(z), &cam);
            render_pixels(&scene, [64, 64], wgpu::TextureFormat::Rgba8Unorm)
        };
        let b = |px: &[u8], x: usize, y: usize| px[(y * 64 + x) * 4 + 2];
        let Some(under) = look(0.0) else { return };
        assert!(b(&under, 32, 32) < 30, "a plane under the block is covered by it");
        assert!(b(&under, 4, 32) > 200, "beyond the block the plane shows");
        let flush = look(20.0).unwrap();
        assert!(b(&flush, 32, 32) < 30, "a plane lying in the block's top face stays behind it");
    }

    #[test]
    fn scene_flattening_matches_mesh() {
        let m = rect_body(0.0, 0.0, 10.0, 10.0, 10.0);
        let cam = OrbitCamera::default();
        let s = Scene::build(&[(&m, [0.5, 0.5, 0.5], Finish::PLAIN)], &cam, 1.5, Style::ShadedEdges);
        assert_eq!(s.tris.len(), m.tris.len() * 3);
        assert_eq!(s.edges.len() % 2, 0);
        assert!(!s.edges.is_empty() && s.edges.len() <= 2 * 12, "only visible box edges");
        assert_eq!(s.uniform_bytes().len() as u64, UNIFORM_SIZE);
        assert!(s.xray.is_empty());
    }

    #[test]
    fn visual_styles_draw_what_they_say() {
        let m = rect_body(0.0, 0.0, 10.0, 10.0, 10.0);
        let cam = OrbitCamera::default();
        let build = |st| Scene::build(&[(&m, [0.5, 0.5, 0.5], Finish::PLAIN)], &cam, 1.5, st);
        let shaded = build(Style::Shaded);
        assert!(!shaded.tris.is_empty() && shaded.edges.is_empty());
        // From the home corner three of a box's twelve edges are behind it.
        let hidden = build(Style::ShadedHidden);
        assert_eq!(hidden.edges, build(Style::ShadedEdges).edges, "the visible ones as usual");
        assert_eq!(hidden.xray.len(), 2 * 3, "and the three behind, faintly");
        let wire = build(Style::Wireframe);
        assert!(wire.tris.is_empty(), "no faces");
        assert_eq!(wire.edges.len(), 2 * 12, "all twelve edges");
    }


    #[test]
    fn gloss_and_metal_change_how_a_body_is_lit() {
        let m = rect_body(-20.0, -20.0, 20.0, 20.0, 30.0);
        let mut cam = OrbitCamera::default();
        cam.distance = 120.0;
        // The body's pixels, added up (red + green + blue).
        let light = |finish: Finish| {
            let scene = Scene::build(&[(&m, [0.6, 0.6, 0.6], finish)], &cam, 1.0, Style::Shaded);
            let px = render_pixels(&scene, [96, 96], wgpu::TextureFormat::Rgba8Unorm)?;
            Some(px.chunks(4).filter(|p| p[3] == 255).map(|p| p[0] as u32 + p[1] as u32 + p[2] as u32).sum::<u32>())
        };
        let Some(plain) = light(Finish::PLAIN) else { return };
        let glossy = light(Finish { metal: 0.0, gloss: 0.8, opacity: 1.0 }).unwrap();
        let rubber = light(Finish { metal: 0.0, gloss: 0.08, opacity: 1.0 }).unwrap();
        let metal = light(Finish { metal: 1.0, gloss: 0.5, opacity: 1.0 }).unwrap();
        assert!(glossy > plain + plain / 20, "gloss adds highlights: {glossy} vs {plain}");
        assert!(rubber >= plain && rubber < glossy, "rubber is nearly plain: {rubber}");
        assert!(metal.abs_diff(plain) > plain / 20, "metal looks different: {metal} vs {plain}");
    }

    #[test]
    fn restyled_triangles_take_their_own_look() {
        let m = rect_body(0.0, 0.0, 10.0, 10.0, 10.0);
        let cam = OrbitCamera::default();
        let mut s = Scene::build(&[(&m, [0.5, 0.5, 0.5], Finish::PLAIN), (&m, [0.5, 0.5, 0.5], Finish::PLAIN)], &cam, 1.5, Style::Shaded);
        let n = m.tris.len();
        let mut which = vec![false; n];
        which[1] = true;
        let glass = Finish { metal: 0.0, gloss: 1.0, opacity: 0.3 };
        s.restyle(n, &which, [1.0, 0.0, 0.0], glass);
        let looks: Vec<usize> = (0..2 * n).filter(|t| s.tris[t * 3][6..12] == [1.0, 0.0, 0.0, 0.0, 1.0, 0.3]).collect();
        assert_eq!(looks, [n + 1], "only that triangle of the second body");
        assert_eq!(split_clear(&s.tris).1.len(), 3, "and it's drawn see-through");
    }

    #[test]
    fn see_through_bodies_show_what_is_behind() {
        // From above: a clear red block standing on a wide green slab.
        let block = rect_body(-20.0, -20.0, 20.0, 20.0, 30.0);
        let slab = rect_body(-40.0, -40.0, 40.0, 0.0, 5.0);
        let glass = Finish { metal: 0.0, gloss: 1.0, opacity: 0.3 };
        let scene = Scene::build(&[(&block, [1.0, 0.0, 0.0], glass), (&slab, [0.0, 1.0, 0.0], Finish::PLAIN)], &top_camera(), 1.0, Style::Shaded);
        let Some(px) = render_pixels(&scene, [64, 64], wgpu::TextureFormat::Rgba8Unorm) else { return };
        let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        // Lower half of the image is y < 0: the slab under the block.
        let over_slab = at(32, 40);
        assert!(over_slab[1] > 60 && over_slab[0] > 60 && over_slab[3] == 255, "green through red glass: {over_slab:?}");
        let over_nothing = at(32, 24);
        assert!(over_nothing[3] > 0 && over_nothing[3] < 200, "the background shows through: {over_nothing:?}");
    }

    /// A row of boxes in a few appearances, for looking at:
    /// `SHOT_DIR=<dir> cargo test appearance_sheet -- --ignored`.
    #[test]
    #[ignore]
    fn appearance_sheet() {
        let m = rect_body(-20.0, -20.0, 20.0, 20.0, 30.0);
        let mut cam = OrbitCamera::default();
        cam.distance = 120.0;
        let looks: Vec<(&str, [u8; 3], Finish)> = vec![("plain", [140, 160, 190], Finish::PLAIN)]
            .into_iter()
            .chain(["Steel - Satin", "Aluminum - Polished", "Brass - Polished", "Plastic - Glossy (Red)", "Plastic - Matte (Grey)", "Rubber - Black", "Paint - Enamel Glossy (Blue)", "Oak", "Glass - Clear"].iter().map(|n| {
                let a = crate::appearance::by_name(n).unwrap();
                (*n, a.color, Finish { metal: a.metal, gloss: a.gloss, opacity: a.opacity })
            }))
            .collect();
        let n = 160usize;
        let mut sheet = vec![0u8; looks.len() * n * n * 4];
        for (i, (_, c, f)) in looks.iter().enumerate() {
            let col = c.map(|v| v as f32 / 255.0);
            let scene = Scene::build(&[(&m, col, *f)], &cam, 1.0, Style::ShadedEdges);
            let px = render_pixels(&scene, [n as u32, n as u32], wgpu::TextureFormat::Rgba8Unorm).unwrap();
            for y in 0..n {
                for x in 0..n {
                    let s = &px[(y * n + x) * 4..][..4];
                    let a = s[3] as f32 / 255.0;
                    let bg = [62.0, 66.0, 74.0];
                    let d = ((y * looks.len() * n) + i * n + x) * 4;
                    for k in 0..3 {
                        sheet[d + k] = (s[k] as f32 + bg[k] * (1.0 - a)).min(255.0) as u8;
                    }
                    sheet[d + 3] = 255;
                }
            }
        }
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let f = std::fs::File::create(format!("{dir}/appearances.png")).unwrap();
        let mut enc = png::Encoder::new(f, (looks.len() * n) as u32, n as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.write_header().unwrap().write_image_data(&sheet).unwrap();
    }
}
