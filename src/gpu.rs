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
const TRI_STRIDE: u64 = 36; // pos, normal, colour: 3 × vec3<f32>
const EDGE_STRIDE: u64 = 12; // pos: vec3<f32>
const LINE_STRIDE: u64 = 28; // pos: vec3<f32>, colour: vec4<f32>
const UNIFORM_SIZE: u64 = 112; // mat4 + 3 × vec4

const SCENE_WGSL: &str = r#"
struct U { view_proj: mat4x4<f32>, head: vec4<f32>, key: vec4<f32>, edge: vec4<f32> };
@group(0) @binding(0) var<uniform> u: U;

struct TriOut { @builtin(position) clip: vec4<f32>, @location(0) color: vec3<f32> };

@vertex
fn vs_tri(@location(0) pos: vec3<f32>, @location(1) normal: vec3<f32>, @location(2) color: vec3<f32>) -> TriOut {
    var o: TriOut;
    o.clip = u.view_proj * vec4<f32>(pos, 1.0);
    let n = normalize(normal);
    let k = 0.30 + 0.50 * max(dot(n, u.head.xyz), 0.0) + 0.30 * max(dot(n, u.key.xyz), 0.0);
    o.color = min(color * k, vec3<f32>(1.0));
    return o;
}

@fragment
fn fs_tri(i: TriOut) -> @location(0) vec4<f32> {
    return vec4<f32>(i.color, 1.0);
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

/// One frame's worth of geometry, already flattened for the GPU.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub view_proj: Mat4,
    pub head_light: Vec3,
    pub key_light: Vec3,
    pub edge_color: [f32; 4],
    /// Per vertex: position, normal, colour (gamma 0..1). Three per triangle.
    pub tris: Vec<[f32; 9]>,
    /// Line list: pairs of positions.
    pub edges: Vec<[f32; 3]>,
    /// Coloured, depth-tested lines (grid, axes): pairs of position + RGBA.
    pub lines: Vec<[f32; 7]>,
    /// Translucent triangles (origin planes): position + RGBA, depth-tested
    /// against the bodies but not written to the depth buffer.
    pub glass: Vec<[f32; 7]>,
}

impl Scene {
    /// Flatten meshes for drawing. Edges are the same ones the CPU renderer
    /// drew (sharp corners + silhouettes), nudged toward the viewer so they
    /// win the depth test against their own faces but stay hidden behind
    /// other geometry.
    pub fn build(meshes: &[(&Mesh, [f32; 3])], cam: &OrbitCamera, aspect: f32) -> Scene {
        let mut s = Scene {
            view_proj: cam.view_proj(aspect),
            head_light: -cam.basis().0,
            key_light: Vec3::new(0.35, -0.45, 0.82).normalize(),
            edge_color: [25.0 / 255.0, 28.0 / 255.0, 34.0 / 255.0, 1.0],
            ..Default::default()
        };
        let nudge = cam.distance * 0.002;
        for (mesh, color) in meshes {
            let front: Vec<bool> = (0..mesh.tris.len())
                .map(|t| {
                    let [a, b, c] = mesh.tris[t].map(|i| mesh.positions[i as usize]);
                    mesh.normal(t).dot(cam.to_eye((a + b + c) / 3.0)) > 0.0
                })
                .collect();
            for (t, tri) in mesh.tris.iter().enumerate() {
                let n = mesh.normal(t);
                for &i in tri {
                    let p = mesh.positions[i as usize];
                    s.tris.push([p.x, p.y, p.z, n.x, n.y, n.z, color[0], color[1], color[2]]);
                }
            }
            for e in mesh_edges(mesh) {
                let (f0, f1) = (front[e.tris[0]], front[e.tris[1]]);
                if (f0 != f1) || (e.sharp && (f0 || f1)) {
                    for i in [e.a, e.b] {
                        let p = mesh.positions[i as usize];
                        let q = p + cam.to_eye(p) * nudge;
                        s.edges.push([q.x, q.y, q.z]);
                    }
                }
            }
        }
        s
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
                label: Some("opencad vertices"),
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
    edge_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    glass_pipeline: wgpu::RenderPipeline,
    blit_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    scene_group: wgpu::BindGroup,
    blit_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    tris: GrowBuffer,
    edges: GrowBuffer,
    lines: GrowBuffer,
    glass: GrowBuffer,
    counts: (u32, u32, u32, u32),
    targets: Option<Targets>,
}

impl Gpu {
    /// `screen_format` is the format of the surface the image is copied onto.
    pub fn new(device: &wgpu::Device, screen_format: wgpu::TextureFormat) -> Self {
        let scene_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("opencad scene"),
            source: wgpu::ShaderSource::Wgsl(SCENE_WGSL.into()),
        });
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("opencad blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT_WGSL.into()),
        });

        let uniforms = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("opencad uniforms"),
            contents: &[0u8; UNIFORM_SIZE as usize],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let scene_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("opencad scene layout"),
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
            label: Some("opencad scene group"),
            layout: &scene_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
        });
        let scene_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("opencad scene pipeline layout"),
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

        let tri_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
        let tri_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opencad triangles"),
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
            depth_stencil: Some(depth(true)),
            multisample: msaa,
            fragment: Some(wgpu::FragmentState {
                module: &scene_module,
                entry_point: Some("fs_tri"),
                compilation_options: Default::default(),
                targets: &opaque,
            }),
            multiview_mask: None,
            cache: None,
        });

        let edge_attrs = wgpu::vertex_attr_array![0 => Float32x3];
        let edge_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opencad edges"),
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
            label: Some("opencad lines"),
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

        let glass_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opencad glass"),
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
            label: Some("opencad blit layout"),
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
            label: Some("opencad blit pipeline layout"),
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });
        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("opencad blit"),
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
            label: Some("opencad blit sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Gpu {
            tri_pipeline,
            edge_pipeline,
            line_pipeline,
            glass_pipeline,
            blit_pipeline,
            uniforms,
            scene_group,
            blit_layout,
            sampler,
            tris: GrowBuffer::new(device, "opencad triangles"),
            edges: GrowBuffer::new(device, "opencad edges"),
            lines: GrowBuffer::new(device, "opencad lines"),
            glass: GrowBuffer::new(device, "opencad glass"),
            counts: (0, 0, 0, 0),
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
        let msaa = tex("opencad msaa", COLOR_FORMAT, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT)
            .create_view(&Default::default());
        let depth = tex("opencad depth", DEPTH_FORMAT, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT)
            .create_view(&Default::default());
        let resolved = tex(
            "opencad resolved",
            COLOR_FORMAT,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let resolved_view = resolved.create_view(&Default::default());
        let blit_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("opencad blit group"),
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
        self.tris.upload(device, queue, &f32_bytes(&scene.tris));
        self.edges.upload(device, queue, &f32_bytes(&scene.edges));
        self.lines.upload(device, queue, &f32_bytes(&scene.lines));
        self.glass.upload(device, queue, &f32_bytes(&scene.glass));
        self.counts = (scene.tris.len() as u32, scene.edges.len() as u32, scene.lines.len() as u32, scene.glass.len() as u32);

        let t = self.targets.as_ref().expect("targets were just created");
        let resolved_view = t.resolved.create_view(&Default::default());
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("opencad scene"),
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
        if self.counts.3 > 0 {
            pass.set_pipeline(&self.glass_pipeline);
            pass.set_vertex_buffer(0, self.glass.buffer.slice(..self.counts.3 as u64 * LINE_STRIDE));
            pass.draw(0..self.counts.3, 0..1);
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
        build_extrude(&[sk], &Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: height }).unwrap()
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
            let bodies = [(&tall, [1.0, 0.0, 0.0]), (&short, [0.0, 1.0, 0.0])];
            let list: Vec<(&Mesh, [f32; 3])> = order.iter().map(|&i| bodies[i]).collect();
            let scene = Scene::build(&list, &cam, 1.0);
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
        let scene = Scene::build(&[(&tall, [1.0, 0.0, 0.0])], &top_camera(), 1.0);
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
            let mut scene = Scene::build(&[(&block, [1.0, 0.0, 0.0])], &cam, 1.0);
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
            let mut scene = Scene::build(&[(&block, [1.0, 0.0, 0.0])], &cam, 1.0);
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
        let s = Scene::build(&[(&m, [0.5, 0.5, 0.5])], &cam, 1.5);
        assert_eq!(s.tris.len(), m.tris.len() * 3);
        assert_eq!(s.edges.len() % 2, 0);
        assert!(!s.edges.is_empty() && s.edges.len() <= 2 * 12, "only visible box edges");
        assert_eq!(s.uniform_bytes().len() as u64, UNIFORM_SIZE);
    }
}
