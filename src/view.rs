//! Drawing helpers: turn world, sketch and body geometry into egui shapes.
//!
//! Solids are drawn on the CPU: back faces are culled, the rest are shaded and
//! depth-sorted far-to-near (painter's algorithm), with outline edges sorted in
//! the same list so hidden edges stay hidden. Exact for convex shapes; a GPU
//! depth buffer (wgpu) replaces this when bodies get complex.

use crate::camera::{CUBE_FACES, CubeCell, OrbitCamera, clip_segment, cube_label_strokes, pixel_from_clip, project};
use crate::model::{Mesh, mesh_edges};
use crate::profile::Region;
use crate::sketch::{GridSpec, Plane, grid_lines};
use egui::{Color32, Stroke};
use glam::{DVec2, Mat4, Vec2, Vec3, Vec4};

pub const CIRCLE_SEGMENTS: usize = 96;
pub const GRID_HALF_LINES: i64 = 45;

pub fn to_pos(v: Vec2) -> egui::Pos2 {
    egui::pos2(v.x, v.y)
}

pub fn to_vec(p: egui::Pos2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// World axis colours: X red, Y green, Z blue.
pub fn axis_color(axis: Vec3) -> Color32 {
    if axis.x.abs() > 0.5 {
        Color32::from_rgb(220, 70, 70)
    } else if axis.y.abs() > 0.5 {
        Color32::from_rgb(70, 200, 90)
    } else {
        Color32::from_rgb(80, 130, 230)
    }
}

/// Distance in pixels from p to the screen segment ab.
pub fn dist_to_segment_px(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

fn shade(c: Color32, k: f32) -> Color32 {
    let f = |v: u8| (v as f32 * k).clamp(0.0, 255.0) as u8;
    Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
}

/// Everything needed to turn world geometry into pixels this frame.
pub struct View<'a> {
    pub painter: &'a egui::Painter,
    pub vp: Mat4,
    pub min: Vec2,
    pub size: Vec2,
    /// When set, every line drawn is also recorded here (screen space), so
    /// what was drawn can be clicked on (dimensions).
    pub trace: Option<&'a std::cell::RefCell<Vec<[Vec2; 2]>>>,
}

impl View<'_> {
    pub fn project(&self, p: Vec3) -> Option<Vec2> {
        project(&self.vp, p, self.min, self.size)
    }

    fn seg(&self, a: Vec2, b: Vec2, stroke: Stroke) {
        self.painter.line_segment([to_pos(a), to_pos(b)], stroke);
        if let Some(t) = self.trace {
            t.borrow_mut().push([a, b]);
        }
    }

    /// A dotted line (4 px dashes, 4 px gaps on screen).
    pub fn dashed(&self, a: Vec3, b: Vec3, stroke: Stroke) {
        let (Some(pa), Some(pb)) = (self.project(a), self.project(b)) else { return };
        let len = pa.distance(pb);
        let dir = (pb - pa) / len.max(1e-6);
        let mut t = 0.0;
        while t < len {
            self.seg(pa + dir * t, pa + dir * (t + 4.0).min(len), stroke);
            t += 8.0;
        }
    }

    /// A dashed arc (see `arc`): the dash pattern runs along the whole curve
    /// on screen, so it reads as one dashed line.
    pub fn dashed_arc(&self, plane: &Plane, center: DVec2, radius: f64, a0: f64, sweep: f64, stroke: Stroke) {
        let n = ((sweep / std::f64::consts::TAU * CIRCLE_SEGMENTS as f64).ceil() as usize).max(2);
        let pts: Vec<Vec2> = (0..=n)
            .filter_map(|i| self.project(plane.to_world(center + DVec2::from_angle(a0 + sweep * i as f64 / n as f64) * radius)))
            .collect();
        let mut phase = 0.0f32; // distance travelled along the curve
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let len = a.distance(b);
            let mut t = 0.0;
            while t < len {
                // 4 px on, 4 px off.
                let pos = phase + t;
                let in_dash = pos.rem_euclid(8.0) < 4.0;
                let step = (4.0 - pos.rem_euclid(4.0)).min(len - t).max(1e-3);
                if in_dash {
                    self.seg(a + (b - a) * (t / len), a + (b - a) * ((t + step) / len), stroke);
                }
                t += step;
            }
            phase += len;
        }
    }

    pub fn dashed_circle(&self, plane: &Plane, center: DVec2, radius: f64, stroke: Stroke) {
        self.dashed_arc(plane, center, radius, 0.0, std::f64::consts::TAU, stroke);
    }

    pub fn line(&self, a: Vec3, b: Vec3, stroke: Stroke) {
        if let Some((ca, cb)) = clip_segment(&self.vp, a, b) {
            let pa = pixel_from_clip(ca, self.min, self.size);
            let pb = pixel_from_clip(cb, self.min, self.size);
            self.seg(pa, pb, stroke);
        }
    }

    /// Part of a circle: counter-clockwise from angle `a0` by `sweep` (radians).
    pub fn arc(&self, plane: &Plane, center: DVec2, radius: f64, a0: f64, sweep: f64, stroke: Stroke) {
        let n = ((sweep / std::f64::consts::TAU * CIRCLE_SEGMENTS as f64).ceil() as usize).max(2);
        let pt = |i: usize| plane.to_world(center + DVec2::from_angle(a0 + sweep * i as f64 / n as f64) * radius);
        for i in 0..n {
            self.line(pt(i), pt(i + 1), stroke);
        }
    }

    /// A radius dimension: from the centre to the edge (one arrow, at the
    /// edge), carried out to the label. Starts at angle `at`; `shift` moves
    /// the label and the line turns to point at it.
    pub fn radius_dimension(
        &self,
        plane: &Plane,
        center: DVec2,
        radius: f64,
        at: f64,
        shift: DVec2,
        text: String,
        color: Color32,
    ) -> Option<egui::Rect> {
        let d0 = DVec2::from_angle(at);
        let pc = self.project(plane.to_world(center))?;
        let pe0 = self.project(plane.to_world(center + d0 * radius))?;
        let px_per_unit = (pe0.distance(pc) as f64 / radius.max(1e-12)).max(1e-9);
        let label = center + d0 * (radius + 24.0 / px_per_unit) + shift;
        let dir = (label - center).normalize_or(d0);
        let edge = center + dir * radius;
        let (pe, pl) = (self.project(plane.to_world(edge))?, self.project(plane.to_world(label))?);
        let stroke = Stroke::new(1.0, color);
        self.seg(pc, pe, stroke);
        self.arrow(pe, (pc - pe).normalize_or_zero() * 8.0, stroke);
        if (label - center).length() > radius {
            self.seg(pe, pl, stroke);
        }
        let right = pl.x >= pc.x;
        let end = pl + Vec2::new(if right { 14.0 } else { -14.0 }, 0.0);
        self.seg(pl, end, stroke);
        let align = if right { egui::Align2::LEFT_CENTER } else { egui::Align2::RIGHT_CENTER };
        Some(self.label(end + Vec2::new(if right { 4.0 } else { -4.0 }, 0.0), align, text, color))
    }

    pub fn circle(&self, plane: &Plane, center: DVec2, radius: f64, stroke: Stroke) {
        let pt = |i: usize| {
            let a = i as f64 / CIRCLE_SEGMENTS as f64 * std::f64::consts::TAU;
            plane.to_world(center + radius * DVec2::new(a.cos(), a.sin()))
        };
        for i in 0..CIRCLE_SEGMENTS {
            self.line(pt(i), pt(i + 1), stroke);
        }
    }

    /// Grid drawn with egui (CPU renderer only; the GPU renderer draws it
    /// with depth so bodies hide it properly).
    pub fn grid(&self, plane: &Plane, center: DVec2, g: GridSpec) {
        let minor = Stroke::new(1.0, Color32::from_gray(60));
        let major = Stroke::new(1.0, Color32::from_gray(85));
        for (a, b, is_major) in grid_lines(plane, center, g, GRID_HALF_LINES) {
            self.line(a, b, if is_major { major } else { minor });
        }
    }

    pub fn text(&self, at: Vec2, text: String, color: Color32) {
        self.painter.text(to_pos(at), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(12.0), color);
    }

    /// Text placed `gap` px from `at` in direction `dir`, aligned so the whole
    /// label sits on that side (never across the line it describes).
    pub fn text_beside(&self, at: Vec2, dir: Vec2, gap: f32, text: String, color: Color32) {
        let pick = |d: f32| {
            if d > 0.3 {
                egui::Align::Min
            } else if d < -0.3 {
                egui::Align::Max
            } else {
                egui::Align::Center
            }
        };
        let align = egui::Align2([pick(dir.x), pick(dir.y)]);
        self.painter.text(to_pos(at + dir * gap), align, text, egui::FontId::proportional(12.0), color);
    }

    /// Text on a dark plate. Returns the plate's rectangle (for click testing).
    pub fn label(&self, at: Vec2, align: egui::Align2, text: String, color: Color32) -> egui::Rect {
        let galley = self.painter.layout_no_wrap(text, egui::FontId::proportional(13.0), color);
        let rect = align.anchor_size(to_pos(at), galley.size());
        self.painter.rect_filled(rect.expand(3.0), 3.0, Color32::from_rgba_unmultiplied(28, 28, 28, 230));
        self.painter.galley(rect.min, galley, color);
        rect.expand(3.0)
    }

    fn arrow(&self, tip: Vec2, back: Vec2, stroke: Stroke) {
        let side = back.perp() * 0.4;
        self.seg(tip, tip + back + side, stroke);
        self.seg(tip, tip + back - side, stroke);
    }

    /// Linear dimension between plane points a and b, drawn on the side of the
    /// line facing away from `away_from` (screen space). Returns the label rect.
    /// An angle dimension between two lines: an arc between them around where
    /// they meet, with the value on it. Returns the label's rectangle.
    /// `shift` moves the label from its default spot (sketch units); the arc
    /// follows it out or in.
    pub fn angle_dimension(
        &self,
        plane: &Plane,
        l1: (DVec2, DVec2),
        l2: (DVec2, DVec2),
        shift: DVec2,
        text: String,
        color: Color32,
    ) -> Option<egui::Rect> {
        let (m1, m2) = ((l1.0 + l1.1) * 0.5, (l2.0 + l2.1) * 0.5);
        let (d1, d2) = (l1.1 - l1.0, l2.1 - l2.0);
        let den = d1.perp_dot(d2);
        // Where the (extended) lines meet; parallel lines: between their middles.
        let x = if den.abs() > 1e-9 * d1.length() * d2.length() {
            l1.0 + d1 * ((l2.0 - l1.0).perp_dot(d2) / den)
        } else {
            (m1 + m2) * 0.5
        };
        let (u, v) = (m1 - x, m2 - x);
        let r0 = (u.length().min(v.length()) * 0.4).max(1e-6);
        let a0 = u.y.atan2(u.x);
        let mut da = v.y.atan2(v.x) - a0;
        while da > std::f64::consts::PI {
            da -= std::f64::consts::TAU;
        }
        while da < -std::f64::consts::PI {
            da += std::f64::consts::TAU;
        }
        let at = x + DVec2::from_angle(a0 + da * 0.5) * r0 * 1.3 + shift;
        let r = (at.distance(x) / 1.3).max(1e-6);
        let pts: Vec<DVec2> = (0..=24).map(|i| x + DVec2::from_angle(a0 + da * i as f64 / 24.0) * r).collect();
        for w in pts.windows(2) {
            self.line(plane.to_world(w[0]), plane.to_world(w[1]), Stroke::new(1.0, color));
        }
        // Where the arc reaches past a line (or stops short of it), continue
        // the line dotted so it's clear which line the arc measures.
        for ((a, b), dir) in [(l1, DVec2::from_angle(a0)), (l2, DVec2::from_angle(a0 + da))] {
            let (ta, tb) = ((a - x).dot(dir), (b - x).dot(dir));
            let (near, far) = (ta.min(tb), ta.max(tb));
            let gap = if r > far {
                Some((far, r))
            } else if r < near {
                Some((r, near))
            } else {
                None
            };
            if let Some((t0, t1)) = gap {
                self.dashed(plane.to_world(x + dir * t0), plane.to_world(x + dir * t1), Stroke::new(1.0, color));
            }
        }
        let p = self.project(plane.to_world(at))?;
        Some(self.label(p, egui::Align2::CENTER_CENTER, text, color))
    }

    /// `shift` (sketch units) moves the label: across the line moves the whole
    /// dimension line out, in, or to the other side; along it slides the text.
    pub fn linear_dimension(
        &self,
        plane: &Plane,
        a: DVec2,
        b: DVec2,
        away_from: Vec2,
        shift: DVec2,
        text: String,
        color: Color32,
    ) -> Option<egui::Rect> {
        let pa = self.project(plane.to_world(a))?;
        let pb = self.project(plane.to_world(b))?;
        let dir = (pb - pa).normalize_or_zero();
        if dir == Vec2::ZERO {
            return None;
        }
        let mid = (pa + pb) * 0.5;
        let mut n = dir.perp();
        if (mid - away_from).dot(n) < 0.0 {
            n = -n;
        }
        let mid2 = (a + b) * 0.5;
        let moved = match (self.project(plane.to_world(mid2 + shift)), self.project(plane.to_world(mid2))) {
            (Some(p), Some(q)) => p - q,
            _ => Vec2::ZERO,
        };
        let off = 28.0 + moved.dot(n);
        let along = moved.dot(dir);
        let side = if off >= 0.0 { 1.0 } else { -1.0 };
        let stroke = Stroke::new(1.0, color);
        self.seg(pa + n * 4.0 * side, pa + n * (off + 6.0 * side), stroke);
        self.seg(pb + n * 4.0 * side, pb + n * (off + 6.0 * side), stroke);
        let (da, db) = (pa + n * off, pb + n * off);
        self.seg(da, db, stroke);
        self.arrow(da, dir * 8.0, stroke);
        self.arrow(db, -dir * 8.0, stroke);
        // Text slid past an end: extend the dimension line out to it.
        let text_at = mid + n * off + dir * along;
        let half = (pb - pa).length() * 0.5;
        if along.abs() > half {
            self.seg(if along > 0.0 { db } else { da }, text_at, stroke);
        }
        Some(self.label(text_at, egui::Align2::CENTER_CENTER, text, color))
    }

    /// Diameter dimension: leader from the circle edge out to a label.
    /// A diameter dimension: a line right across the circle through its
    /// centre, arrows at both edges, carried out to the label. `shift` moves
    /// the label (sketch units); the diameter line turns to point at it.
    pub fn diameter_dimension(
        &self,
        plane: &Plane,
        center: DVec2,
        radius: f64,
        shift: DVec2,
        text: String,
        color: Color32,
    ) -> Option<egui::Rect> {
        let d0 = DVec2::new(1.0, 1.0) * std::f64::consts::FRAC_1_SQRT_2;
        let pc = self.project(plane.to_world(center))?;
        let pe0 = self.project(plane.to_world(center + d0 * radius))?;
        // Screen pixels per sketch unit here, to place the default label 30 px out.
        let px_per_unit = (pe0.distance(pc) as f64 / radius.max(1e-12)).max(1e-9);
        let label = center + d0 * (radius + 30.0 / px_per_unit) + shift;
        let dir = (label - center).normalize_or(d0);
        let (e1, e2) = (center + dir * radius, center - dir * radius);
        let (p1, p2, pl) = (
            self.project(plane.to_world(e1))?,
            self.project(plane.to_world(e2))?,
            self.project(plane.to_world(label))?,
        );
        let stroke = Stroke::new(1.0, color);
        let across = (p1 - p2).normalize_or_zero();
        self.seg(p2, p1, stroke);
        self.arrow(p1, -across * 8.0, stroke);
        self.arrow(p2, across * 8.0, stroke);
        // Out from the edge to the label (only if the label is outside).
        if (label - center).length() > radius {
            self.seg(p1, pl, stroke);
        }
        let right = pl.x >= pc.x;
        let end = pl + Vec2::new(if right { 14.0 } else { -14.0 }, 0.0);
        self.seg(pl, end, stroke);
        let align = if right { egui::Align2::LEFT_CENTER } else { egui::Align2::RIGHT_CENTER };
        Some(self.label(end + Vec2::new(if right { 4.0 } else { -4.0 }, 0.0), align, text, color))
    }

    /// Flat translucent fill of a profile region.
    pub fn fill_region(&self, plane: &Plane, region: &Region, color: Color32) {
        let (v, tris) = region.triangulate();
        let pts: Vec<Option<Vec2>> = v.iter().map(|&p| self.project(plane.to_world(p))).collect();
        let mut mesh = egui::Mesh::default();
        for t in tris {
            if let [Some(a), Some(b), Some(c)] = t.map(|i| pts[i]) {
                let base = mesh.vertices.len() as u32;
                for p in [a, b, c] {
                    mesh.colored_vertex(to_pos(p), color);
                }
                mesh.add_triangle(base, base + 1, base + 2);
            }
        }
        self.painter.add(egui::Shape::mesh(mesh));
    }

    /// Flat translucent fill of some world triangles (a highlighted face).
    pub fn fill_tris(&self, tris: impl IntoIterator<Item = [Vec3; 3]>, color: Color32) {
        let mut mesh = egui::Mesh::default();
        for t in tris {
            if let [Some(a), Some(b), Some(c)] = t.map(|p| self.project(p)) {
                let base = mesh.vertices.len() as u32;
                for p in [a, b, c] {
                    mesh.colored_vertex(to_pos(p), color);
                }
                mesh.add_triangle(base, base + 1, base + 2);
            }
        }
        self.painter.add(egui::Shape::mesh(mesh));
    }

    /// A draggable arrow from `base` to `tip` (world). Returns its screen ends.
    pub fn arrow3d(&self, base: Vec3, tip: Vec3, color: Color32) -> Option<(Vec2, Vec2)> {
        let a = self.project(base)?;
        let b = self.project(tip)?;
        let d = (b - a).normalize_or_zero();
        if d == Vec2::ZERO {
            return None;
        }
        let head = b - d * 14.0;
        self.seg(a, head, Stroke::new(3.0, color));
        let side = d.perp() * 7.0;
        self.painter.add(egui::Shape::convex_polygon(
            vec![to_pos(b), to_pos(head + side), to_pos(head - side)],
            color,
            Stroke::NONE,
        ));
        self.painter.circle_filled(to_pos(a), 4.0, color);
        Some((a, b))
    }

    /// The view cube: `center`/`scale` in pixels (scale = half the cube's size).
    /// `hover`: the view direction under the mouse. Every cell with that
    /// direction lights up — all three patches of a corner, both of an edge.
    pub fn view_cube(
        &self,
        cam: &OrbitCamera,
        cells: &[CubeCell],
        center: Vec2,
        scale: f32,
        hover: Option<glam::Vec3>,
        show_axes: bool,
    ) {
        let to_screen = |p: glam::Vec3| center + cam.cube_point(p).0 * scale;
        let toward = -cam.basis().0;
        let border = Stroke::new(1.0, Color32::from_rgb(70, 78, 92));

        // Axis triad from the cube's origin corner (-1,-1,-1), FRONT-LEFT-BOTTOM:
        // the one corner where +X, +Y and +Z all run along cube edges. From the
        // home view X and Z show and Y runs back along the hidden left-bottom
        // edge. Parts behind the cube are drawn first so it covers them.
        let origin = glam::Vec3::splat(-1.0);
        let axes: &[(glam::Vec3, &str, f32)] = if show_axes {
            &[(glam::Vec3::X, "X", 2.7), (glam::Vec3::Y, "Y", 2.7), (glam::Vec3::Z, "Z", 2.7)]
        } else {
            &[]
        };
        let draw_axis = |axis: glam::Vec3, name: &str, len: f32| {
            let c = axis_color(axis);
            self.seg(to_screen(origin), to_screen(origin + axis * len), Stroke::new(2.0, c));
            self.painter.text(
                to_pos(to_screen(origin + axis * (len + 0.3))),
                egui::Align2::CENTER_CENTER,
                name,
                egui::FontId::proportional(scale * 0.34),
                c,
            );
        };
        let behind = |axis: glam::Vec3, len: f32| cam.cube_point(origin + axis * (len * 0.5)).1 > 0.0;
        for &(axis, name, len) in axes {
            if behind(axis, len) {
                draw_axis(axis, name, len);
            }
        }

        for c in cells {
            if !cam.cube_face_visible(c.face) {
                continue;
            }
            let lit = CUBE_FACES[c.face].0.dot(toward).max(0.0);
            let fill = if hover.is_some_and(|h| h.dot(c.dir) > 0.9999) {
                Color32::from_rgb(70, 140, 235)
            } else {
                shade(Color32::from_rgb(190, 198, 212), 0.55 + 0.45 * lit)
            };
            let pts = c.quad.iter().map(|&p| to_pos(to_screen(p))).collect();
            self.painter.add(egui::Shape::convex_polygon(pts, fill, Stroke::new(0.5, Color32::from_rgb(150, 158, 172))));
        }
        for (face, &(n, a, b, _)) in CUBE_FACES.iter().enumerate() {
            if !cam.cube_face_visible(face) {
                continue;
            }
            let corners = [n - a - b, n + a - b, n + a + b, n - a + b];
            for k in 0..4 {
                self.seg(to_screen(corners[k]), to_screen(corners[(k + 1) % 4]), border);
            }
            // The label lies on the face, so it tilts and foreshortens with it.
            let ink = Stroke::new((scale * 0.045).max(1.0), Color32::from_rgb(45, 52, 64));
            for (p, q) in cube_label_strokes(face) {
                self.seg(to_screen(p), to_screen(q), ink);
            }
        }
        for &(axis, name, len) in axes {
            if !behind(axis, len) {
                draw_axis(axis, name, len);
            }
        }
    }

    /// Small house icon (Home button).
    pub fn house_icon(painter: &egui::Painter, r: egui::Rect, color: Color32) {
        let (w, h) = (r.width(), r.height());
        let p = |x: f32, y: f32| egui::pos2(r.left() + x * w, r.top() + y * h);
        painter.add(egui::Shape::convex_polygon(vec![p(0.5, 0.0), p(1.0, 0.5), p(0.0, 0.5)], color, Stroke::NONE));
        painter.add(egui::Shape::convex_polygon(
            vec![p(0.18, 0.45), p(0.82, 0.45), p(0.82, 1.0), p(0.18, 1.0)],
            color,
            Stroke::NONE,
        ));
    }

    /// Small downward triangle (menu button).
    pub fn menu_icon(painter: &egui::Painter, r: egui::Rect, color: Color32) {
        let c = r.center();
        let s = r.width().min(r.height()) * 0.35;
        painter.add(egui::Shape::convex_polygon(
            vec![c + egui::vec2(-s, -s * 0.5), c + egui::vec2(s, -s * 0.5), c + egui::vec2(0.0, s * 0.7)],
            color,
            Stroke::NONE,
        ));
    }

    /// Shaded solids with outline edges.
    pub fn solids(&self, solids: &[(&Mesh, Color32)], cam: &OrbitCamera) {
        enum Prim {
            Tri([Vec2; 3], Color32),
            Edge(Vec2, Vec2),
        }
        let key_light = Vec3::new(0.35, -0.45, 0.82).normalize();
        // Headlight along the view direction. It must not depend on where a
        // triangle is: flat faces then get one exact colour, so the
        // triangulation of a cap can't show through as different shades.
        let head_light = -cam.basis().0;
        let edge_col = Color32::from_rgb(25, 28, 34);
        let mut prims: Vec<(f32, Prim)> = Vec::new();

        for (mesh, base) in solids {
            let clip: Vec<Vec4> = mesh.positions.iter().map(|p| self.vp * p.extend(1.0)).collect();
            let screen = |i: u32| -> Option<Vec2> {
                let c = clip[i as usize];
                (c.z >= 0.0 && c.w > 1e-6).then(|| pixel_from_clip(c, self.min, self.size))
            };
            let front: Vec<bool> = (0..mesh.tris.len())
                .map(|t| {
                    let [a, b, c] = mesh.tris[t].map(|i| mesh.positions[i as usize]);
                    mesh.normal(t).dot(cam.to_eye((a + b + c) / 3.0)) > 0.0
                })
                .collect();
            for (t, tri) in mesh.tris.iter().enumerate() {
                if !front[t] {
                    continue;
                }
                let (Some(a), Some(b), Some(c)) = (screen(tri[0]), screen(tri[1]), screen(tri[2])) else {
                    continue;
                };
                let n = mesh.normal(t);
                let centre = tri.map(|i| mesh.positions[i as usize]).iter().copied().sum::<Vec3>() / 3.0;
                let k = 0.30 + 0.50 * n.dot(head_light).max(0.0) + 0.30 * n.dot(key_light).max(0.0);
                let depth = cam.depth(centre);
                prims.push((depth, Prim::Tri([a, b, c], shade(*base, k))));
            }
            for e in mesh_edges(mesh) {
                let (f0, f1) = (front[e.tris[0]], front[e.tris[1]]);
                let silhouette = f0 != f1;
                if !(silhouette || (e.sharp && (f0 || f1))) {
                    continue;
                }
                if let (Some(a), Some(b)) = (screen(e.a), screen(e.b)) {
                    // Slightly nearer than its faces so it draws on top of them.
                    let mid = (mesh.positions[e.a as usize] + mesh.positions[e.b as usize]) * 0.5;
                    let depth = cam.depth(mid) - cam.distance * 0.002;
                    prims.push((depth, Prim::Edge(a, b)));
                }
            }
        }

        prims.sort_by(|x, y| y.0.total_cmp(&x.0)); // far to near
        let mut mesh = egui::Mesh::default();
        for (_, p) in prims {
            let base = mesh.vertices.len() as u32;
            match p {
                Prim::Tri(v, col) => {
                    for q in v {
                        mesh.colored_vertex(to_pos(q), col);
                    }
                    mesh.add_triangle(base, base + 1, base + 2);
                }
                Prim::Edge(a, b) => {
                    let d = (b - a).normalize_or_zero().perp() * 0.8;
                    for q in [a + d, b + d, b - d, a - d] {
                        mesh.colored_vertex(to_pos(q), edge_col);
                    }
                    mesh.add_triangle(base, base + 1, base + 2);
                    mesh.add_triangle(base, base + 2, base + 3);
                }
            }
        }
        self.painter.add(egui::Shape::mesh(mesh));
    }
}
