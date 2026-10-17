//! Orbit camera, projection helpers and picking rays.
//! Pure math (glam only) so everything here is unit-tested without a window.
//! World is Z-up, units are millimetres.

use glam::{Mat4, Vec2, Vec3, Vec4};

pub const FOV_Y: f32 = 45.0 * std::f32::consts::PI / 180.0;
const MAX_PITCH: f32 = 90.0 * std::f32::consts::PI / 180.0; // straight down/up is allowed

/// How the 3D view is projected onto the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Projection {
    /// Far things look smaller, like a real camera.
    #[default]
    Perspective,
    /// Parallel lines stay parallel; sizes don't change with depth.
    Orthographic,
    /// Perspective, but orthographic when looking straight along an axis
    /// (sketch views, front/top/side).
    PerspectiveOrthoFaces,
}

impl Projection {
    pub const ALL: [Projection; 3] =
        [Projection::Perspective, Projection::Orthographic, Projection::PerspectiveOrthoFaces];

    pub fn name(self) -> &'static str {
        match self {
            Projection::Perspective => "Perspective",
            Projection::Orthographic => "Orthographic",
            Projection::PerspectiveOrthoFaces => "Perspective with Ortho Faces",
        }
    }
}

/// Orbit camera: stores what it looks at and how it's angled.
/// The eye position is derived (spherical -> Cartesian).
pub struct OrbitCamera {
    pub target: Vec3,
    pub distance: f32,
    pub yaw: f32,   // around world Z, radians
    pub pitch: f32, // up/down from the XY plane, radians
    /// Turned about the view direction (the view cube's roll arrows), radians.
    pub roll: f32,
    pub projection: Projection,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: 300.0,
            // Home looks from the front-right-top corner, like Fusion.
            yaw: -45f32.to_radians(),
            pitch: 30f32.to_radians(),
            roll: 0.0,
            projection: Projection::default(),
        }
    }
}

impl OrbitCamera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + self.distance * Vec3::new(cp * cy, cp * sy, sp)
    }

    /// (forward, right, up) — the same basis the view matrix uses.
    /// "Right" comes from the yaw alone, so it stays well defined when looking
    /// straight down or up (where forward × Z would be zero).
    pub fn basis(&self) -> (Vec3, Vec3, Vec3) {
        let forward = (self.target - self.eye()).normalize();
        let (sy, cy) = self.yaw.sin_cos();
        let right = Vec3::new(-sy, cy, 0.0);
        let up = right.cross(forward);
        // Rolled: the screen's right and up turn about the view direction.
        let (sr, cr) = self.roll.sin_cos();
        (forward, right * cr + up * sr, up * cr - right * sr)
    }

    fn near_far(&self) -> (f32, f32) {
        ((self.distance * 0.001).max(0.01), self.distance * 100.0)
    }

    /// True when the current view should be drawn orthographically.
    pub fn is_ortho(&self) -> bool {
        match self.projection {
            Projection::Perspective => false,
            Projection::Orthographic => true,
            Projection::PerspectiveOrthoFaces => {
                let f = self.basis().0.abs();
                f.max_element() > 2f32.to_radians().cos() // within ~2° of an axis
            }
        }
    }

    /// Half the visible height at the target (mm). Perspective and
    /// orthographic use the same value so switching modes keeps the scale.
    pub fn half_height(&self) -> f32 {
        self.distance * (FOV_Y * 0.5).tan()
    }

    /// Orthographic views look from far back so nothing in front of the
    /// target gets clipped, whatever the zoom.
    fn ortho_back(&self) -> f32 {
        (self.distance * 10.0).max(10_000.0)
    }

    /// Where rays start: the eye in perspective, a far point in orthographic.
    pub fn view_origin(&self) -> Vec3 {
        if self.is_ortho() {
            self.target - self.basis().0 * self.ortho_back()
        } else {
            self.eye()
        }
    }

    pub fn view(&self) -> Mat4 {
        glam::camera::rh::view::look_at_mat4(self.view_origin(), self.target, self.basis().2)
    }

    /// WebGPU/DirectX convention: clip z in [0, 1], Y up. This is what wgpu expects.
    pub fn proj(&self, aspect: f32) -> Mat4 {
        if self.is_ortho() {
            let h = self.half_height();
            let back = self.ortho_back();
            glam::camera::rh::proj::directx::orthographic(-h * aspect, h * aspect, -h, h, 1.0, back * 2.0)
        } else {
            let (near, far) = self.near_far();
            glam::camera::rh::proj::directx::perspective(FOV_Y, aspect, near, far)
        }
    }

    /// Direction from a world point toward the viewer (for shading/culling).
    pub fn to_eye(&self, p: Vec3) -> Vec3 {
        if self.is_ortho() { -self.basis().0 } else { (self.eye() - p).normalize_or_zero() }
    }

    /// Distance of a world point in front of the viewer, along the view direction.
    pub fn depth(&self, p: Vec3) -> f32 {
        self.basis().0.dot(p - self.view_origin())
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    pub fn orbit(&mut self, dx: f32, dy: f32) {
        const SENS: f32 = 0.008; // radians per pixel
        // A rolled view: the drag measured along the unrolled screen axes.
        let (sr, cr) = self.roll.sin_cos();
        let (dx, dy) = (dx * cr + dy * sr, dy * cr - dx * sr);
        self.yaw -= dx * SENS;
        self.pitch = (self.pitch + dy * SENS).clamp(-MAX_PITCH, MAX_PITCH);
    }

    pub fn pan(&mut self, dx: f32, dy: f32) {
        let (_, right, up) = self.basis();
        let scale = self.distance * 0.0015;
        self.target += (-right * dx + up * dy) * scale;
    }

    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (-scroll * 0.002).exp()).clamp(1.0, 50_000.0);
    }

    /// Zoom toward (or away from) what's under the cursor, so that spot stays
    /// put on screen.
    pub fn zoom_at(&mut self, scroll: f32, aspect: f32, ndc: Vec2) {
        let (forward, _, _) = self.basis();
        let ray = self.ray_from_ndc(aspect, ndc);
        // The point under the cursor at the target's depth.
        let denom = ray.dir.dot(forward);
        let hit = if denom.abs() > 1e-6 { ray.at((self.target - ray.origin).dot(forward) / denom) } else { self.target };
        let before = self.distance;
        self.zoom(scroll);
        let k = self.distance / before;
        self.target = hit + (self.target - hit) * k;
    }

    /// Point the camera along a direction given as yaw/pitch in degrees.
    #[cfg(test)]
    pub fn look_from(&mut self, yaw_deg: f32, pitch_deg: f32) {
        self.yaw = yaw_deg.to_radians();
        self.pitch = pitch_deg.to_radians().clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// The ray through a point on screen (given in NDC, -1..1).
    /// Built from the camera basis directly — no matrix inverse, so no precision loss.
    /// Perspective rays fan out from the eye; orthographic rays are parallel.
    pub fn ray_from_ndc(&self, aspect: f32, ndc: Vec2) -> Ray {
        let (forward, right, up) = self.basis();
        if self.is_ortho() {
            let h = self.half_height();
            let origin = self.view_origin() + right * (ndc.x * h * aspect) + up * (ndc.y * h);
            Ray { origin, dir: forward }
        } else {
            let h = (FOV_Y * 0.5).tan();
            let dir = forward + right * (ndc.x * h * aspect) + up * (ndc.y * h);
            Ray { origin: self.eye(), dir: dir.normalize() }
        }
    }
}

/// P(t) = origin + t * dir
#[derive(Clone, Copy, Debug)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }
}

// ---------- View cube ----------

/// Faces of the view cube: (outward normal, tangent a, tangent b, label),
/// with a × b = normal so each face's corners run counter-clockwise from outside.
pub const CUBE_FACES: [(Vec3, Vec3, Vec3, &str); 6] = [
    (Vec3::Z, Vec3::X, Vec3::Y, "TOP"),
    (Vec3::NEG_Z, Vec3::X, Vec3::NEG_Y, "BOTTOM"),
    (Vec3::NEG_Y, Vec3::X, Vec3::Z, "FRONT"),
    (Vec3::Y, Vec3::NEG_X, Vec3::Z, "BACK"),
    (Vec3::X, Vec3::Y, Vec3::Z, "RIGHT"),
    (Vec3::NEG_X, Vec3::NEG_Y, Vec3::Z, "LEFT"),
];

/// One clickable patch of the view cube. Each face is split 3 × 3: the middle
/// looks at that face, the side strips at an edge (two faces), the corners
/// at a corner (three faces). `dir` points from the model toward the viewer.
#[derive(Clone, Copy, Debug)]
pub struct CubeCell {
    pub face: usize,
    pub quad: [Vec3; 4],
    pub dir: Vec3,
}

/// All 54 cells of a cube spanning -1..1.
pub fn cube_cells() -> Vec<CubeCell> {
    let bands = [(-1.0f32, -0.5f32, -1.0f32), (-0.5, 0.5, 0.0), (0.5, 1.0, 1.0)];
    let mut out = Vec::with_capacity(54);
    for (face, &(n, a, b, _)) in CUBE_FACES.iter().enumerate() {
        for &(s0, s1, sa) in &bands {
            for &(t0, t1, sb) in &bands {
                let p = |s: f32, t: f32| n + a * s + b * t;
                out.push(CubeCell {
                    face,
                    quad: [p(s0, t0), p(s1, t0), p(s1, t1), p(s0, t1)],
                    dir: (n + a * sa + b * sb).normalize(),
                });
            }
        }
    }
    out
}

/// A tiny single-stroke font for the cube's face labels: each letter is a few
/// polylines in a 0.6 × 1 box (x right, y up). Being plain geometry, the
/// letters can lie *on* the cube faces and turn and foreshorten with them.
fn glyph(c: char) -> &'static [&'static [(f32, f32)]] {
    match c {
        'A' => &[&[(0.0, 0.0), (0.3, 1.0), (0.6, 0.0)], &[(0.12, 0.4), (0.48, 0.4)]],
        'B' => &[
            &[(0.0, 0.0), (0.0, 1.0), (0.42, 1.0), (0.55, 0.9), (0.55, 0.62), (0.42, 0.52), (0.0, 0.52)],
            &[(0.42, 0.52), (0.58, 0.4), (0.58, 0.12), (0.44, 0.0), (0.0, 0.0)],
        ],
        'C' => &[&[(0.58, 0.85), (0.45, 1.0), (0.15, 1.0), (0.0, 0.8), (0.0, 0.2), (0.15, 0.0), (0.45, 0.0), (0.58, 0.15)]],
        'E' => &[&[(0.58, 1.0), (0.0, 1.0), (0.0, 0.0), (0.58, 0.0)], &[(0.0, 0.52), (0.45, 0.52)]],
        'F' => &[&[(0.58, 1.0), (0.0, 1.0), (0.0, 0.0)], &[(0.0, 0.52), (0.45, 0.52)]],
        'G' => &[&[
            (0.58, 0.85),
            (0.45, 1.0),
            (0.15, 1.0),
            (0.0, 0.8),
            (0.0, 0.2),
            (0.15, 0.0),
            (0.45, 0.0),
            (0.6, 0.15),
            (0.6, 0.45),
            (0.32, 0.45),
        ]],
        'H' => &[&[(0.0, 0.0), (0.0, 1.0)], &[(0.6, 0.0), (0.6, 1.0)], &[(0.0, 0.52), (0.6, 0.52)]],
        'I' => &[&[(0.3, 0.0), (0.3, 1.0)], &[(0.12, 1.0), (0.48, 1.0)], &[(0.12, 0.0), (0.48, 0.0)]],
        'K' => &[&[(0.0, 0.0), (0.0, 1.0)], &[(0.6, 1.0), (0.0, 0.42)], &[(0.2, 0.6), (0.6, 0.0)]],
        'L' => &[&[(0.0, 1.0), (0.0, 0.0), (0.55, 0.0)]],
        'M' => &[&[(0.0, 0.0), (0.0, 1.0), (0.3, 0.45), (0.6, 1.0), (0.6, 0.0)]],
        'N' => &[&[(0.0, 0.0), (0.0, 1.0), (0.6, 0.0), (0.6, 1.0)]],
        'O' => &[&[
            (0.15, 0.0),
            (0.45, 0.0),
            (0.6, 0.2),
            (0.6, 0.8),
            (0.45, 1.0),
            (0.15, 1.0),
            (0.0, 0.8),
            (0.0, 0.2),
            (0.15, 0.0),
        ]],
        'P' => &[&[(0.0, 0.0), (0.0, 1.0), (0.42, 1.0), (0.58, 0.88), (0.58, 0.62), (0.42, 0.5), (0.0, 0.5)]],
        'R' => &[
            &[(0.0, 0.0), (0.0, 1.0), (0.42, 1.0), (0.58, 0.88), (0.58, 0.62), (0.42, 0.5), (0.0, 0.5)],
            &[(0.3, 0.5), (0.6, 0.0)],
        ],
        'T' => &[&[(0.0, 1.0), (0.6, 1.0)], &[(0.3, 1.0), (0.3, 0.0)]],
        _ => &[],
    }
}

/// The label of a cube face as 3D line segments lying on that face, centred,
/// reading left-to-right when you look straight at the face.
pub fn cube_label_strokes(face: usize) -> Vec<(Vec3, Vec3)> {
    let (n, a, b, label) = CUBE_FACES[face];
    let h = 0.3; // letter height in cube units (faces span -1..1)
    let advance = 0.85 * h;
    let width = label.len() as f32 * advance - 0.25 * h;
    let mut out = Vec::new();
    for (i, c) in label.chars().enumerate() {
        let x0 = -width / 2.0 + i as f32 * advance;
        for line in glyph(c) {
            for w in line.windows(2) {
                let at = |(x, y): (f32, f32)| n + a * (x0 + x * h) + b * ((y - 0.5) * h);
                out.push((at(w[0]), at(w[1])));
            }
        }
    }
    out
}

/// Camera yaw/pitch (degrees) that looks at the model from direction `dir`.
/// Straight up/down keeps the front of the model at the bottom of the screen.
pub fn angles_for_dir(dir: Vec3) -> (f32, f32) {
    let d = dir.normalize();
    let yaw = if d.x.abs() < 1e-6 && d.y.abs() < 1e-6 { -90.0 } else { d.y.atan2(d.x).to_degrees() };
    (yaw, d.z.clamp(-1.0, 1.0).asin().to_degrees())
}

/// Where a view-cube click should take the camera. Clicking the view you're
/// already looking straight at flips to the opposite one (front → back,
/// a corner → the corner diagonally across).
pub fn cube_click_target(looking_from: Vec3, clicked: Vec3) -> Vec3 {
    if looking_from.normalize().dot(clicked.normalize()) > 0.9998 { -clicked } else { clicked }
}

/// Signed shortest turn from angle `a` to `b` (radians), in -π..π.
pub fn shortest_turn(a: f32, b: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let d = (b - a).rem_euclid(tau);
    if d > std::f32::consts::PI { d - tau } else { d }
}

/// Where the camera is: enough to restore or animate a view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    pub target: Vec3,
    pub distance: f32,
}

impl Pose {
    /// In-between pose for t in 0..1: turns the short way round and zooms
    /// evenly (geometric), so long zooms don't rush at the start.
    pub fn lerp(a: Pose, b: Pose, t: f32) -> Pose {
        Pose {
            yaw: a.yaw + shortest_turn(a.yaw, b.yaw) * t,
            pitch: a.pitch + (b.pitch - a.pitch) * t,
            roll: a.roll + shortest_turn(a.roll, b.roll) * t,
            target: a.target.lerp(b.target, t),
            distance: a.distance * (b.distance / a.distance).powf(t),
        }
    }
}

impl OrbitCamera {
    pub fn pose(&self) -> Pose {
        Pose { yaw: self.yaw, pitch: self.pitch, roll: self.roll, target: self.target, distance: self.distance }
    }

    pub fn set_pose(&mut self, p: Pose) {
        self.yaw = p.yaw;
        self.roll = p.roll;
        self.pitch = p.pitch.clamp(-MAX_PITCH, MAX_PITCH);
        self.target = p.target;
        self.distance = p.distance;
    }

    /// A view-cube point on screen relative to the cube centre: x right,
    /// y down, in cube units (multiply by the pixel scale). Also returns how
    /// far into the screen it is (bigger = farther).
    pub fn cube_point(&self, p: Vec3) -> (Vec2, f32) {
        let (f, r, u) = self.basis();
        (Vec2::new(p.dot(r), -p.dot(u)), p.dot(f))
    }

    /// Fit (F6): the same view direction, moved and zoomed so every point
    /// shows with a margin (a tenth of the view each side). None if there's
    /// nothing to fit.
    pub fn fit(&self, pts: &[Vec3], aspect: f32) -> Option<Pose> {
        let (lo, hi) = pts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
        if !(lo.x <= hi.x) {
            return None;
        }
        let center = (lo + hi) * 0.5;
        let (forward, right, up) = self.basis();
        // How far the points reach from the centre across and up the screen, and in depth.
        let (mut half_w, mut half_h, mut depth) = (0f32, 0f32, 0f32);
        for p in pts {
            let d = *p - center;
            half_w = half_w.max(d.dot(right).abs());
            half_h = half_h.max(d.dot(up).abs());
            depth = depth.max(d.dot(forward).abs());
        }
        let need = (half_h.max(half_w / aspect.max(1e-3)) / 0.8).max(1.0);
        // In perspective the nearest points look bigger: stand back by their depth too.
        let back = if self.is_ortho() { 0.0 } else { depth };
        let distance = (need / (FOV_Y * 0.5).tan() + back).clamp(1.0, 50_000.0);
        Some(Pose { target: center, distance, ..self.pose() })
    }

    /// Looking straight at one face of the cube (front, top, …): the view
    /// cube then offers to roll the view.
    pub fn face_on(&self) -> bool {
        self.basis().0.abs().max_element() > 0.5f32.to_radians().cos()
    }

    /// The pose rolled a step (45°) about the view direction: `clockwise` as
    /// seen on screen. Snaps to whole steps.
    pub fn rolled(&self, clockwise: bool) -> Pose {
        let step = std::f32::consts::FRAC_PI_4;
        let k = (self.roll / step).round() + if clockwise { 1.0 } else { -1.0 };
        Pose { roll: (k * step).rem_euclid(std::f32::consts::TAU), ..self.pose() }
    }

    /// Is this cube face turned toward the viewer?
    pub fn cube_face_visible(&self, face: usize) -> bool {
        CUBE_FACES[face].0.dot(-self.basis().0) > 1e-3
    }

    /// Which cell (index into `cube_cells()`) is under a point given in cube units.
    pub fn cube_hit(&self, cells: &[CubeCell], at: Vec2) -> Option<usize> {
        cells.iter().position(|c| {
            if !self.cube_face_visible(c.face) {
                return false;
            }
            let q: Vec<Vec2> = c.quad.iter().map(|&p| self.cube_point(p).0).collect();
            // Inside a convex quad: same side of all four edges.
            let sides: Vec<f32> = (0..4).map(|i| (q[(i + 1) % 4] - q[i]).perp_dot(at - q[i])).collect();
            sides.iter().all(|&s| s >= 0.0) || sides.iter().all(|&s| s <= 0.0)
        })
    }
}

/// Parameter `s` of the point on the line `origin + s * dir` closest to the ray.
/// None if the ray runs parallel to the line. Used for dragging along an axis.
pub fn closest_param_on_line(ray: &Ray, origin: Vec3, dir: Vec3) -> Option<f32> {
    let w0 = origin - ray.origin;
    let (a, b, c) = (dir.dot(dir), dir.dot(ray.dir), ray.dir.dot(ray.dir));
    let (d, e) = (dir.dot(w0), ray.dir.dot(w0));
    let denom = a * c - b * b;
    if denom.abs() < 1e-6 * a * c {
        return None;
    }
    Some((b * e - c * d) / denom)
}

/// Screen pixel -> NDC. `min`/`size` describe the viewport rectangle.
pub fn ndc_from_pixel(px: Vec2, min: Vec2, size: Vec2) -> Vec2 {
    Vec2::new(
        (px.x - min.x) / size.x * 2.0 - 1.0,
        1.0 - (px.y - min.y) / size.y * 2.0, // screen Y points down, NDC Y up
    )
}

/// Clip-space point -> screen pixel. Caller must guarantee w > 0.
pub fn pixel_from_clip(clip: Vec4, min: Vec2, size: Vec2) -> Vec2 {
    let ndc = clip.truncate() / clip.w;
    Vec2::new(
        min.x + (ndc.x + 1.0) * 0.5 * size.x,
        min.y + (1.0 - ndc.y) * 0.5 * size.y,
    )
}

/// World point -> screen pixel, or None if behind the camera.
pub fn project(vp: &Mat4, p: Vec3, min: Vec2, size: Vec2) -> Option<Vec2> {
    let clip = *vp * p.extend(1.0);
    (clip.w > 1e-6).then(|| pixel_from_clip(clip, min, size))
}

/// Clip a segment against the near plane (clip z = 0). Returns clip-space
/// endpoints, or None if the whole segment is behind the camera.
pub fn clip_segment(vp: &Mat4, a: Vec3, b: Vec3) -> Option<(Vec4, Vec4)> {
    let mut ca = *vp * a.extend(1.0);
    let mut cb = *vp * b.extend(1.0);
    match (ca.z >= 0.0, cb.z >= 0.0) {
        (false, false) => return None,
        (true, true) => {}
        // z(t) = za + t(zb - za) = 0  ->  t = za / (za - zb)
        (true, false) => cb = ca + (cb - ca) * (ca.z / (ca.z - cb.z)),
        (false, true) => ca = ca + (cb - ca) * (ca.z / (ca.z - cb.z)),
    }
    Some((ca, cb))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: Vec2 = Vec2::new(0.0, 0.0);
    const SIZE: Vec2 = Vec2::new(800.0, 600.0);

    #[test]
    fn eye_is_distance_from_target() {
        let c = OrbitCamera::default();
        assert!(((c.eye() - c.target).length() - c.distance).abs() < 1e-3);
    }

    #[test]
    fn target_projects_to_screen_centre() {
        let c = OrbitCamera::default();
        let p = project(&c.view_proj(SIZE.x / SIZE.y), c.target, MIN, SIZE).unwrap();
        assert!((p - SIZE * 0.5).length() < 0.5);
    }

    #[test]
    fn pitch_is_clamped() {
        let mut c = OrbitCamera::default();
        c.orbit(0.0, 100_000.0);
        assert!(c.pitch <= MAX_PITCH + 1e-6);
    }

    #[test]
    fn pixel_ndc_roundtrip() {
        let px = Vec2::new(123.0, 456.0);
        let ndc = ndc_from_pixel(px, MIN, SIZE);
        let back = pixel_from_clip(ndc.extend(0.5).extend(1.0), MIN, SIZE);
        assert!((back - px).length() < 1e-3);
    }

    #[test]
    fn ray_through_pixel_projects_back_to_same_pixel() {
        let c = OrbitCamera::default();
        let aspect = SIZE.x / SIZE.y;
        let vp = c.view_proj(aspect);
        for px in [Vec2::new(10.0, 20.0), Vec2::new(400.0, 300.0), Vec2::new(790.0, 590.0)] {
            let ray = c.ray_from_ndc(aspect, ndc_from_pixel(px, MIN, SIZE));
            for t in [50.0, 300.0, 2000.0] {
                let back = project(&vp, ray.at(t), MIN, SIZE).unwrap();
                assert!((back - px).length() < 0.05, "px {px} t {t} -> {back}");
            }
        }
    }

    #[test]
    fn ortho_rays_are_parallel_and_round_trip() {
        let mut c = OrbitCamera::default();
        c.projection = Projection::Orthographic;
        let aspect = SIZE.x / SIZE.y;
        let vp = c.view_proj(aspect);
        assert!((project(&vp, c.target, MIN, SIZE).unwrap() - SIZE * 0.5).length() < 0.5);
        let r1 = c.ray_from_ndc(aspect, Vec2::new(-0.8, 0.3));
        let r2 = c.ray_from_ndc(aspect, Vec2::new(0.6, -0.9));
        assert!(r1.dir.dot(r2.dir) > 0.99999, "parallel");
        for px in [Vec2::new(10.0, 20.0), Vec2::new(790.0, 590.0)] {
            let ray = c.ray_from_ndc(aspect, ndc_from_pixel(px, MIN, SIZE));
            let back = project(&vp, ray.at(c.ortho_back()), MIN, SIZE).unwrap();
            assert!((back - px).length() < 0.05, "{px} -> {back}");
        }
    }

    #[test]
    fn ortho_and_perspective_match_scale_at_target() {
        let mut c = OrbitCamera::default();
        let aspect = SIZE.x / SIZE.y;
        let (_, right, _) = c.basis();
        let p = c.target + right * 50.0;
        let a = project(&c.view_proj(aspect), p, MIN, SIZE).unwrap();
        c.projection = Projection::Orthographic;
        let b = project(&c.view_proj(aspect), p, MIN, SIZE).unwrap();
        assert!((a - b).length() < 0.5, "{a} vs {b}");
    }

    #[test]
    fn ortho_faces_switches_only_when_axis_aligned() {
        let mut c = OrbitCamera::default();
        c.projection = Projection::PerspectiveOrthoFaces;
        assert!(!c.is_ortho());
        c.look_from(-90.0, 0.0); // looking along +Y
        assert!(c.is_ortho());
        c.look_from(-90.0, 89.0); // top view, as used for XY sketches
        assert!(c.is_ortho());
    }

    #[test]
    fn top_view_is_exactly_straight_down() {
        // A point and the same point 80 mm higher must land on the same pixel in ortho.
        let mut c = OrbitCamera::default();
        c.projection = Projection::Orthographic;
        c.look_from(-90.0, 90.0);
        let (f, r, u) = c.basis();
        assert!((f - Vec3::NEG_Z).length() < 1e-6 && (r - Vec3::X).length() < 1e-6 && (u - Vec3::Y).length() < 1e-6);
        let vp = c.view_proj(SIZE.x / SIZE.y);
        let a = project(&vp, Vec3::new(20.0, 10.0, 0.0), MIN, SIZE).unwrap();
        let b = project(&vp, Vec3::new(20.0, 10.0, 80.0), MIN, SIZE).unwrap();
        assert!((a - b).length() < 1e-3, "{a} vs {b}");
        assert!(a.x > SIZE.x * 0.5 && a.y < SIZE.y * 0.5, "+X is right, +Y is up");
    }

    #[test]
    fn view_cube_has_26_distinct_views() {
        let cells = cube_cells();
        assert_eq!(cells.len(), 54);
        let mut dirs: Vec<(i32, i32, i32)> = cells
            .iter()
            .map(|c| {
                let d = c.dir * 1000.0;
                (d.x.round() as i32, d.y.round() as i32, d.z.round() as i32)
            })
            .collect();
        dirs.sort();
        dirs.dedup();
        assert_eq!(dirs.len(), 26, "6 faces + 12 edges + 8 corners");
        for c in &cells {
            // Every cell's corners run counter-clockwise seen from outside.
            let n = (c.quad[1] - c.quad[0]).cross(c.quad[2] - c.quad[1]);
            assert!(n.dot(CUBE_FACES[c.face].0) > 0.0);
        }
    }

    #[test]
    fn cube_labels_lie_on_their_faces_and_read_correctly() {
        for (face, &(n, a, b, label)) in CUBE_FACES.iter().enumerate() {
            assert!(label.chars().all(|c| !glyph(c).is_empty()), "every letter of {label} has strokes");
            let strokes = cube_label_strokes(face);
            assert!(strokes.len() >= label.len());
            for (p, q) in strokes {
                for x in [p, q] {
                    assert!((x.dot(n) - 1.0).abs() < 1e-6, "{label}: on the face plane");
                    assert!(x.dot(a).abs() < 0.9 && x.dot(b).abs() < 0.9, "{label}: inside the face");
                }
            }
            // Text runs along `a` and up along `b`; a × b points out of the face,
            // so seen from outside it reads left to right, not mirrored.
            assert!((a.cross(b) - n).length() < 1e-6);
        }
    }

    #[test]
    fn cube_views_match_named_directions() {
        let close = |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3;
        assert!(close(angles_for_dir(Vec3::NEG_Y), (-90.0, 0.0)), "front = the XZ sketch view");
        assert!(close(angles_for_dir(Vec3::X), (0.0, 0.0)), "right = the YZ sketch view");
        assert!(close(angles_for_dir(Vec3::Z), (-90.0, 90.0)), "top = the XY sketch view");
        assert!(close(angles_for_dir(Vec3::NEG_Z), (-90.0, -90.0)));
        let corner = angles_for_dir(Vec3::new(1.0, -1.0, 1.0));
        assert!(close(corner, (-45.0, 35.26439)), "{corner:?}");
    }

    #[test]
    fn clicking_the_middle_of_the_front_face_picks_front() {
        let mut c = OrbitCamera::default();
        c.look_from(-90.0, 0.0); // looking at the front
        let cells = cube_cells();
        let hit = c.cube_hit(&cells, Vec2::ZERO).unwrap();
        assert_eq!(CUBE_FACES[cells[hit].face].3, "FRONT");
        assert!((cells[hit].dir - Vec3::NEG_Y).length() < 1e-6, "centre cell = face view");
        // Near the top-right of the front face: the front-right-top corner.
        let hit = c.cube_hit(&cells, Vec2::new(0.9, -0.9)).unwrap();
        assert!((cells[hit].dir - Vec3::new(1.0, -1.0, 1.0).normalize()).length() < 1e-6);
        assert!(c.cube_hit(&cells, Vec2::new(3.0, 3.0)).is_none(), "outside the cube");
    }

    #[test]
    fn pose_animation_ends_exactly_and_zooms_evenly() {
        let a = Pose { yaw: 3.0, pitch: 0.2, roll: 0.0, target: Vec3::ZERO, distance: 100.0 };
        let b = Pose { yaw: -3.0, pitch: -0.4, roll: 0.0, target: Vec3::new(10.0, 0.0, 0.0), distance: 400.0 };
        let mid = Pose::lerp(a, b, 0.5);
        assert!((mid.distance - 200.0).abs() < 1e-3, "geometric middle of 100 and 400");
        assert!(mid.yaw > 3.0, "goes the short way across ±180°");
        let end = Pose::lerp(a, b, 1.0);
        assert!((shortest_turn(end.yaw, b.yaw)).abs() < 1e-5 && (end.pitch - b.pitch).abs() < 1e-6);
        assert!((end.target - b.target).length() < 1e-6 && (end.distance - b.distance).abs() < 1e-3);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor_still() {
        for projection in [Projection::Perspective, Projection::Orthographic] {
            let mut c = OrbitCamera { projection, ..OrbitCamera::default() };
            let (aspect, ndc) = (1.6, Vec2::new(0.4, -0.3));
            let ray = c.ray_from_ndc(aspect, ndc);
            // a point under the cursor somewhere in the scene
            let p = ray.at(250.0);
            for scroll in [120.0, -300.0] {
                c.zoom_at(scroll, aspect, ndc);
                let clip = c.view_proj(aspect) * p.extend(1.0);
                let now = Vec2::new(clip.x, clip.y) / clip.w;
                // Exact for points at the target's depth; close for others.
                let r = c.ray_from_ndc(aspect, ndc);
                let off = (p - r.origin).cross(r.dir).length();
                assert!(off < 1.0 || (now - ndc).length() < 0.05, "{projection:?} {scroll}: {now} vs {ndc}");
            }
        }
        // At the target's depth it's exact.
        let mut c = OrbitCamera::default();
        let (aspect, ndc) = (1.0, Vec2::new(0.5, 0.5));
        let (fwd, _, _) = c.basis();
        let ray = c.ray_from_ndc(aspect, ndc);
        let p = ray.at((c.target - ray.origin).dot(fwd) / ray.dir.dot(fwd));
        c.zoom_at(200.0, aspect, ndc);
        let clip = c.view_proj(aspect) * p.extend(1.0);
        assert!((Vec2::new(clip.x, clip.y) / clip.w - ndc).length() < 1e-4);
    }

    #[test]
    fn clicking_the_current_view_flips_to_the_opposite() {
        let front = Vec3::NEG_Y;
        assert_eq!(cube_click_target(front, front), Vec3::Y, "front again → back");
        assert_eq!(cube_click_target(Vec3::Z, front), front, "different view → just go there");
        let corner = Vec3::new(1.0, -1.0, 1.0).normalize();
        let mut c = OrbitCamera::default();
        let (yaw, pitch) = angles_for_dir(corner);
        c.look_from(yaw, pitch);
        let opposite = cube_click_target(-c.basis().0, corner);
        assert!((opposite + corner).length() < 1e-5, "corner → the corner diagonally across");
    }

    #[test]
    fn home_looks_at_front_right_top() {
        let eye_dir = -OrbitCamera::default().basis().0;
        assert!(eye_dir.x > 0.0 && eye_dir.y < 0.0 && eye_dir.z > 0.0);
    }

    #[test]
    fn camera_turns_the_short_way_round() {
        let pi = std::f32::consts::PI;
        assert!((shortest_turn(0.0, 1.0) - 1.0).abs() < 1e-6);
        assert!((shortest_turn(3.0, -3.0) - (2.0 * pi - 6.0)).abs() < 1e-5, "across ±180°");
        assert!((shortest_turn(-170f32.to_radians(), 170f32.to_radians()) + 20f32.to_radians()).abs() < 1e-5);
    }

    #[test]
    fn closest_point_on_axis_to_ray() {
        let ray = Ray { origin: Vec3::new(5.0, -100.0, 30.0), dir: Vec3::Y };
        let s = closest_param_on_line(&ray, Vec3::ZERO, Vec3::Z).unwrap();
        assert!((s - 30.0).abs() < 1e-4);
        assert!(closest_param_on_line(&ray, Vec3::ZERO, Vec3::Y).is_none(), "parallel");
    }

    #[test]
    fn segment_through_camera_is_clipped_to_near_plane() {
        let mut c = OrbitCamera::default();
        c.target = Vec3::new(0.0, 0.0, 10.0);
        c.distance = 10.0;
        c.look_from(-90.0, 0.0);
        let vp = c.view_proj(1.5);
        let (a, b) = clip_segment(&vp, Vec3::new(0.0, -100.0, 0.0), Vec3::new(0.0, 100.0, 0.0)).unwrap();
        assert!(a.w > 0.0 && b.w > 0.0);
        assert!(a.z.abs() < 1e-4);
        assert!(clip_segment(&vp, Vec3::new(-5.0, -100.0, 0.0), Vec3::new(5.0, -100.0, 0.0)).is_none());
    }

    #[test]
    fn rolling_turns_the_view_45_degrees_clockwise_on_screen() {
        let mut c = OrbitCamera { yaw: -90f32.to_radians(), pitch: 0.0, ..Default::default() };
        assert!(c.face_on(), "the front view");
        let x = c.cube_point(Vec3::X).0;
        assert!((x - Vec2::new(1.0, 0.0)).length() < 1e-5, "X points right: {x}");
        c.set_pose(c.rolled(true));
        assert!(c.face_on(), "still straight at the front");
        let x = c.cube_point(Vec3::X).0;
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!((x - Vec2::new(h, h)).length() < 1e-5, "X turned clockwise, to right-and-down (screen y is down): {x}");
        // Back the other way, twice: 45° the other side.
        c.set_pose(c.rolled(false));
        c.set_pose(c.rolled(false));
        assert!((c.cube_point(Vec3::X).0 - Vec2::new(h, -h)).length() < 1e-5);
        // Orbiting a rolled view: a drag moves what faces you the way the
        // mouse went on screen, as it does unrolled.
        let moved = |c: &OrbitCamera, dx: f32, dy: f32| {
            let mut d = OrbitCamera { ..*c };
            let p = Vec3::new(0.3, -1.0, 0.2);
            let before = d.cube_point(p).0;
            d.orbit(dx, dy);
            d.cube_point(p).0 - before
        };
        let flat = OrbitCamera { yaw: -90f32.to_radians(), pitch: 0.0, ..Default::default() };
        for (dx, dy) in [(4.0, 0.0), (0.0, 4.0), (3.0, -2.0)] {
            let (a, b) = (moved(&flat, dx, dy), moved(&c, dx, dy));
            assert!(a.normalize().dot(b.normalize()) > 0.99, "{dx},{dy}: {a} unrolled vs {b} rolled");
        }
    }

    #[test]
    fn fit_shows_everything_with_a_margin_in_both_projections() {
        let pts: Vec<Vec3> = [(0.0, 0.0, 0.0), (400.0, 0.0, 0.0), (400.0, 60.0, 0.0), (0.0, 60.0, 30.0), (200.0, 30.0, 90.0)].map(|(x, y, z)| Vec3::new(x, y, z)).to_vec();
        for projection in [Projection::Orthographic, Projection::Perspective] {
            let mut c = OrbitCamera { projection, ..Default::default() };
            let aspect = 1.6;
            c.set_pose(c.fit(&pts, aspect).unwrap());
            let vp = c.view_proj(aspect);
            let mut biggest = 0f32;
            for p in &pts {
                let clip = vp * p.extend(1.0);
                let ndc = Vec2::new(clip.x, clip.y) / clip.w;
                assert!(ndc.x.abs() <= 0.95 && ndc.y.abs() <= 0.95, "{projection:?}: {p} at {ndc}");
                biggest = biggest.max(ndc.x.abs()).max(ndc.y.abs());
            }
            assert!(biggest > 0.5, "{projection:?}: it fills the view, not a speck ({biggest})");
            // The direction is kept.
            let home = OrbitCamera::default();
            assert!((c.yaw - home.yaw).abs() < 1e-6 && (c.pitch - home.pitch).abs() < 1e-6);
        }
        assert!(OrbitCamera::default().fit(&[], 1.0).is_none());
    }
}
