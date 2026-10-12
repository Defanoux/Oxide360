//! 2D sketches on planes: data model, editing tools, hit testing, deletion.
//! Sketch geometry is f64 (CAD precision); rendering converts to f32.
//! Entities reference points by index and constraints reference entities/points
//! by index, so connected geometry moves together when the solver runs.

use crate::camera::Ray;
use glam::{DVec2, Vec2, Vec3};
use serde::{Deserialize, Serialize};

/// Smallest line length / circle radius the tools will create (mm).
pub const MIN_SIZE: f64 = 0.01;

// ---------- Planes ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OriginPlane {
    XY,
    XZ,
    YZ,
}

impl OriginPlane {
    pub const ALL: [OriginPlane; 3] = [OriginPlane::XY, OriginPlane::XZ, OriginPlane::YZ];

    pub fn name(self) -> &'static str {
        match self {
            OriginPlane::XY => "XY",
            OriginPlane::XZ => "XZ",
            OriginPlane::YZ => "YZ",
        }
    }

    /// u and v are the sketch's local X and Y axes. normal = u × v.
    pub fn plane(self) -> Plane {
        let (u, v) = match self {
            OriginPlane::XY => (Vec3::X, Vec3::Y), // normal +Z
            OriginPlane::XZ => (Vec3::X, Vec3::Z), // normal -Y
            OriginPlane::YZ => (Vec3::Y, Vec3::Z), // normal +X
        };
        Plane { origin: Vec3::ZERO, u, v }
    }

    /// Camera yaw/pitch (degrees) that faces the plane with u right and v up.
    pub fn view_angles(self) -> (f32, f32) {
        match self {
            OriginPlane::XY => (-90.0, 90.0),
            OriginPlane::XZ => (-90.0, 0.0),
            OriginPlane::YZ => (0.0, 0.0),
        }
    }
}

/// A sketch plane lying on a flat face of a body. `u` and `v` are the
/// sketch's X and Y axes; `u × v` points out of the body.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FacePlane {
    pub origin: glam::DVec3,
    pub u: glam::DVec3,
    pub v: glam::DVec3,
    /// Set when the face was the far end of an extrude or revolve (it then
    /// moves with that feature's distance or angle).
    #[serde(default)]
    pub follow: Option<crate::model::Follow>,
}

impl FacePlane {
    /// The sketch plane for a flat face with outward normal `n` through `point`.
    /// X runs horizontally (as the camera's "right" does when looking straight
    /// at the face), and the origin is the point of the plane nearest the world
    /// origin — so on faces parallel to XY/XZ/YZ, sketch coordinates line up
    /// with the world.
    pub fn from_face(n: glam::DVec3, point: glam::DVec3) -> Self {
        let n = n.normalize();
        let horizontal = (n.x * n.x + n.y * n.y).sqrt();
        let u = if horizontal < 1e-6 { glam::DVec3::X } else { glam::DVec3::Z.cross(n).normalize() };
        let v = n.cross(u);
        FacePlane { origin: n * n.dot(point), u, v, follow: None }
    }

    pub fn normal(&self) -> glam::DVec3 {
        self.u.cross(self.v)
    }

    /// For drawing and picking (f32).
    pub fn plane(&self) -> Plane {
        Plane { origin: self.origin.as_vec3(), u: self.u.as_vec3(), v: self.v.as_vec3() }
    }
}

/// Rendering-side plane (f32).
#[derive(Clone, Copy, Debug)]
pub struct Plane {
    pub origin: Vec3,
    pub u: Vec3,
    pub v: Vec3,
}

impl Plane {
    pub fn normal(&self) -> Vec3 {
        self.u.cross(self.v)
    }

    pub fn to_world(&self, p: DVec2) -> Vec3 {
        let p = p.as_vec2();
        self.origin + self.u * p.x + self.v * p.y
    }

    /// Where a ray hits the plane, in plane-local 2D. None if the ray is
    /// parallel to the plane or the plane is behind the ray origin.
    pub fn intersect(&self, ray: &Ray) -> Option<DVec2> {
        self.intersect_t(ray).map(|(_, p)| p)
    }

    /// Like `intersect`, also returning the ray distance `t` (for picking the nearest plane).
    pub fn intersect_t(&self, ray: &Ray) -> Option<(f32, DVec2)> {
        let n = self.normal();
        let denom = n.dot(ray.dir);
        if denom.abs() < 1e-6 {
            return None;
        }
        // n · (O + tD - P0) = 0  ->  t = n · (P0 - O) / (n · D)
        let t = n.dot(self.origin - ray.origin) / denom;
        if t < 0.0 {
            return None;
        }
        let rel = ray.at(t) - self.origin;
        Some((t, Vec2::new(rel.dot(self.u), rel.dot(self.v)).as_dvec2()))
    }
}

// ---------- Grid ----------

/// Grid spacing that adapts to zoom, using the 1-2-5 sequence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpec {
    pub step: f64,
    pub major_every: i64,
}

impl GridSpec {
    /// About 30 grid cells across the camera distance.
    pub fn for_distance(distance: f32) -> Self {
        let raw = (distance as f64 / 30.0).max(1e-4);
        let mag = 10f64.powf(raw.log10().floor());
        let norm = raw / mag;
        let (nice, major_every) = if norm <= 1.0 + 1e-9 {
            (1.0, 5)
        } else if norm <= 2.0 + 1e-9 {
            (2.0, 5)
        } else if norm <= 5.0 + 1e-9 {
            (5.0, 2)
        } else {
            (10.0, 5)
        };
        Self { step: nice * mag, major_every }
    }

    pub fn snap(&self, p: DVec2) -> DVec2 {
        (p / self.step).round() * self.step
    }
}

/// Grid lines on a plane around `center` (plane-local), `half_lines` each way,
/// as world segments with a "major line" flag. The two axes are left out
/// (they're drawn separately, in colour).
pub fn grid_lines(plane: &Plane, center: DVec2, g: GridSpec, half_lines: i64) -> Vec<(Vec3, Vec3, bool)> {
    let ci = (center / g.step).round();
    let (cx, cy) = (ci.x as i64, ci.y as i64);
    let n = half_lines;
    let (x0, x1) = ((cx - n) as f64 * g.step, (cx + n) as f64 * g.step);
    let (y0, y1) = ((cy - n) as f64 * g.step, (cy + n) as f64 * g.step);
    let mut out = Vec::with_capacity(4 * n as usize + 2);
    for k in -n..=n {
        for (idx, vertical) in [(cx + k, true), (cy + k, false)] {
            if idx == 0 {
                continue;
            }
            let v = idx as f64 * g.step;
            let (a, b) = if vertical {
                (DVec2::new(v, y0), DVec2::new(v, y1))
            } else {
                (DVec2::new(x0, v), DVec2::new(x1, v))
            };
            out.push((plane.to_world(a), plane.to_world(b), idx.rem_euclid(g.major_every) == 0));
        }
    }
    out
}

// ---------- Sketch data ----------

pub type PointId = usize;
pub type EntityId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Entity {
    Line { a: PointId, b: PointId },
    Circle { center: PointId, radius: f64 },
    /// A point on its own (the Point tool), so it isn't tidied away.
    Point { p: PointId },
    /// Part of a circle, counter-clockwise from `a` to `b` around `center`.
    /// Its radius is |a − center| (the solver keeps |b − center| equal).
    Arc { center: PointId, a: PointId, b: PointId },
}

/// Geometric constraints. Each (except Fix) is one equation for the solver.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Constraint {
    Horizontal(EntityId),
    Vertical(EntityId),
    Length(EntityId, f64),
    Diameter(EntityId, f64),
    Parallel(EntityId, EntityId),
    Perpendicular(EntityId, EntityId),
    /// Equal length (two lines) or equal radius (two circles).
    Equal(EntityId, EntityId),
    /// Line tangent to circle.
    Tangent(EntityId, EntityId),
    /// A point lying on a circle.
    OnCircle(PointId, EntityId),
    /// A point lying on a line (extended): a line started or ended on another.
    OnLine(PointId, EntityId),
    /// A point locked onto one of the sketch's origin axes (0 = X, 1 = Y).
    OnAxis(PointId, u8),
    /// An arc's radius.
    Radius(EntityId, f64),
    /// Internal (never stored): an arc's two ends stay the same distance from
    /// its centre. The solver adds one per arc.
    ArcEnds(EntityId),
    /// The first point stays halfway between the other two (a centre
    /// rectangle's centre between opposite corners).
    Midpoint(PointId, PointId, PointId),
    /// Two lines at a set angle (degrees, 0..180, measured counter-clockwise
    /// from the first line to the second; lines have no direction, so 30° and
    /// 210° are the same thing).
    Angle(EntityId, EntityId, f64),
    /// Two points a set distance apart.
    Distance(PointId, PointId, f64),
    /// Point locked where it is.
    Fix(PointId),
}

impl Constraint {
    pub fn entities(&self) -> Vec<EntityId> {
        use Constraint::*;
        match *self {
            Horizontal(e) | Vertical(e) | Length(e, _) | Diameter(e, _) => vec![e],
            Parallel(a, b) | Perpendicular(a, b) | Equal(a, b) | Tangent(a, b) | Angle(a, b, _) => vec![a, b],
            OnCircle(_, c) | OnLine(_, c) | Radius(c, _) | ArcEnds(c) => vec![c],
            Fix(_) | Distance(..) | Midpoint(..) | OnAxis(..) => vec![],
        }
    }

    /// The points it ties directly (not through an entity).
    pub fn points(&self) -> Vec<PointId> {
        match *self {
            Constraint::Fix(p) => vec![p],
            Constraint::Distance(p, q, _) => vec![p, q],
            Constraint::OnCircle(p, _) | Constraint::OnLine(p, _) => vec![p],
            Constraint::Midpoint(p, a, b) => vec![p, a, b],
            Constraint::OnAxis(p, _) => vec![p],
            _ => vec![],
        }
    }

    /// Short label for the constraint list and viewport glyphs.
    pub fn glyph(&self) -> &'static str {
        use Constraint::*;
        match self {
            Horizontal(_) => "H",
            Vertical(_) => "V",
            Length(..) => "Length",
            Diameter(..) => "Diameter",
            Parallel(..) => "//",
            Perpendicular(..) => "Perp",
            Equal(..) => "=",
            Tangent(..) => "Tan",
            Angle(..) => "Angle",
            Distance(..) => "Distance",
            OnCircle(..) | OnLine(..) => "On",
            Midpoint(..) => "Mid",
            Radius(..) => "R",
            ArcEnds(..) => "",
            OnAxis(_, 0) => "On X axis",
            OnAxis(..) => "On Y axis",
            Fix(_) => "Fix",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    pub name: String,
    /// The origin plane, unless `face` is set.
    pub plane: OriginPlane,
    /// Set when the sketch was started on a face of a body. Older files have none.
    #[serde(default)]
    pub face: Option<FacePlane>,
    pub points: Vec<DVec2>,
    pub entities: Vec<Entity>,
    pub constraints: Vec<Constraint>,
    /// Where each dimension's label was dragged to (sketch units, from its
    /// default spot), by constraint index; missing entries are zero.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_shift: Vec<DVec2>,
    /// Dimensions shown for reference only (by constraint index): they report
    /// the geometry and are never solved. Missing entries mean driving.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub driven: Vec<bool>,
}

/// A line as unit normal n and offset d: n·p + d is the signed distance of p.
fn line_eq(sk: &Sketch, e: EntityId) -> Option<(DVec2, f64)> {
    match sk.entities.get(e)? {
        Entity::Line { a, b } => {
            let (a, b) = (sk.points[*a], sk.points[*b]);
            let t = b - a;
            if t.length() < 1e-12 {
                return None;
            }
            let n = t.perp() / t.length();
            Some((n, -n.dot(a)))
        }
        _ => None,
    }
}

/// Which side of each line `hint` is on (+1 / −1).
fn sides(eqs: &[(DVec2, f64)], hint: DVec2) -> Vec<f64> {
    eqs.iter().map(|(n, d)| if n.dot(hint) + d >= 0.0 { 1.0 } else { -1.0 }).collect()
}

/// The circle touching three lines (extended) in the region the cursor is in.
/// Lines meeting in a triangle have one such circle per region (the inside
/// one, and one outside each side); two parallel lines and a third crossing
/// them give the circle between the parallel pair.
pub fn tangent_circle_3(sk: &Sketch, lines: [EntityId; 3], hint: DVec2) -> Result<(DVec2, f64), String> {
    let eqs: Vec<(DVec2, f64)> = lines.iter().map(|&e| line_eq(sk, e)).collect::<Option<_>>().ok_or("pick three lines")?;
    let parallel = |i: usize, j: usize| eqs[i].0.perp_dot(eqs[j].0).abs() < 1e-9;
    if parallel(0, 1) && parallel(1, 2) {
        return Err("all three lines are parallel — no circle can touch all of them".into());
    }
    let s = sides(&eqs, hint);
    // n_i·c + d_i = s_i·r for each line: three equations in cx, cy, r.
    let m = [
        [eqs[0].0.x, eqs[0].0.y, -s[0]],
        [eqs[1].0.x, eqs[1].0.y, -s[1]],
        [eqs[2].0.x, eqs[2].0.y, -s[2]],
    ];
    let rhs = [-eqs[0].1, -eqs[1].1, -eqs[2].1];
    let det3 = |m: [[f64; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let det = det3(m);
    if det.abs() < 1e-12 {
        return Err("no circle touches all three lines on this side — move the cursor between them".into());
    }
    let col = |k: usize| {
        let mut mk = m;
        for (row, &v) in mk.iter_mut().zip(&rhs) {
            row[k] = v;
        }
        det3(mk) / det
    };
    let (c, r) = (DVec2::new(col(0), col(1)), col(2));
    if !(r > 1e-9) || !c.is_finite() {
        return Err("no circle touches all three lines on this side — move the cursor between them".into());
    }
    Ok((c, r))
}

/// The circle touching two lines (extended), sitting in the cursor's region,
/// sized so its centre is as close to the cursor as it can be.
pub fn tangent_circle_2(sk: &Sketch, lines: [EntityId; 2], hint: DVec2) -> Result<(DVec2, f64), String> {
    let eqs: Vec<(DVec2, f64)> = lines.iter().map(|&e| line_eq(sk, e)).collect::<Option<_>>().ok_or("pick two lines")?;
    let s = sides(&eqs, hint);
    let ((n1, d1), (n2, d2)) = (eqs[0], eqs[1]);
    let det = n1.perp_dot(n2);
    if det.abs() < 1e-9 {
        // Parallel: the circle fills the gap, centred across from the cursor.
        let (h1, h2) = (n1.dot(hint) + d1, n2.dot(hint) + d2);
        let gap = (h1 - s[0] * 0.0).abs() + h2.abs();
        let between = (n1.dot(n2) > 0.0) != (s[0] == s[1]);
        if !between || gap < 1e-9 {
            return Err("put the cursor between the two parallel lines".into());
        }
        let r = gap / 2.0;
        return Ok((hint - n1 * (h1 - s[0] * r), r));
    }
    // n_i·c + d_i = s_i·r  →  c(r) = c0 + r·k (a line of centres): pick r so
    // the centre is nearest the cursor.
    let solve = |b1: f64, b2: f64| DVec2::new(b1 * n2.y - n1.y * b2, n1.x * b2 - b1 * n2.x) / det;
    let c0 = solve(-d1, -d2);
    let k = solve(s[0], s[1]);
    let r = ((hint - c0).dot(k) / k.length_squared().max(1e-30)).max(0.0);
    if r < 1e-9 {
        return Err("move the cursor further from where the lines meet".into());
    }
    Ok((c0 + k * r, r))
}

/// An arc a tool is about to make: drawn counter-clockwise from `start` by
/// `sweep` (radians). `flipped`: the clicks run clockwise, so the stored arc
/// swaps its ends. `end_angle`: where the last click's end of the arc is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcShape {
    pub center: DVec2,
    pub radius: f64,
    pub start: f64,
    pub sweep: f64,
    pub end_angle: f64,
    pub flipped: bool,
}

/// What the 3-Point Arc's third click snapped to (see `Editor::arc3_snap`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arc3Snap {
    None,
    HalfCircle,
    GridCenter(DVec2),
}

/// Circle through three points (None if they're in a line).
pub fn circumcircle(a: DVec2, b: DVec2, c: DVec2) -> Option<(DVec2, f64)> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    let scale = (b - a).length().max((c - a).length()).max(1e-12);
    if d.abs() < 1e-9 * scale * scale {
        return None;
    }
    let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let center = DVec2::new(ux, uy);
    Some((center, center.distance(a)))
}

/// The circle touching three lines (extended): the incircle of the triangle
/// they make. None if two of them are parallel. (Checks tangent_circle_3.)
#[cfg(test)]
pub fn incircle(sk: &Sketch, l1: EntityId, l2: EntityId, l3: EntityId) -> Option<(DVec2, f64)> {
    let line = |e| match sk.entities.get(e)? {
        Entity::Line { a, b } => Some((sk.points[*a], sk.points[*b])),
        _ => None,
    };
    let meet = |(a, b): (DVec2, DVec2), (c, d): (DVec2, DVec2)| -> Option<DVec2> {
        let (r, s) = (b - a, d - c);
        let den = r.perp_dot(s);
        (den.abs() > 1e-9 * r.length() * s.length()).then(|| a + r * ((c - a).perp_dot(s) / den))
    };
    let (p1, p2, p3) = (line(l1)?, line(l2)?, line(l3)?);
    let (a, b, c) = (meet(p2, p3)?, meet(p1, p3)?, meet(p1, p2)?);
    let (la, lb, lc) = (b.distance(c), a.distance(c), a.distance(b));
    let per = la + lb + lc;
    if per < 1e-12 {
        return None;
    }
    let center = (a * la + b * lb + c * lc) / per;
    let area = ((b - a).perp_dot(c - a) * 0.5).abs();
    Some((center, 2.0 * area / per))
}

/// Add a circle (centre c, radius r) that passes through the given clicks;
/// clicks on existing points are kept on it with OnCircle constraints.
fn add_circle_through(sk: &mut Sketch, c: DVec2, r: f64, through: &[Anchor]) -> Vec<Constraint> {
    let center = sk.resolve(Anchor::Free(c));
    sk.entities.push(Entity::Circle { center, radius: r });
    let circle = sk.entities.len() - 1;
    let mut seen = Vec::new();
    through
        .iter()
        .filter_map(|a| match *a {
            Anchor::Point(p) => Some(p),
            _ => None,
        })
        .filter(|p| {
            let fresh = !seen.contains(p);
            seen.push(*p);
            fresh
        })
        .map(|p| Constraint::OnCircle(p, circle))
        .collect()
}

/// A click target while drawing: an existing point (snapped) or a free spot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Anchor {
    Point(PointId),
    Free(DVec2),
    /// On an existing line (at this spot): the new point is tied to the line.
    OnLine(EntityId, DVec2),
    /// On one of the sketch's origin axes (0 = X, 1 = Y): the new point is
    /// locked onto that axis.
    OnAxis(u8, DVec2),
}

/// Something selectable in a sketch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    Point(PointId),
    Entity(EntityId),
}

/// Distance from p to segment ab.
pub fn dist_to_segment(p: DVec2, a: DVec2, b: DVec2) -> f64 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

impl Sketch {
    /// Point 0 is always the projected origin. It never moves.
    pub const ORIGIN: PointId = 0;

    pub fn new(name: String, plane: OriginPlane) -> Self {
        Self {
            name,
            plane,
            face: None,
            points: vec![DVec2::ZERO],
            entities: Vec::new(),
            constraints: Vec::new(),
            label_shift: Vec::new(),
            driven: Vec::new(),
        }
    }

    /// A sketch on a body face.
    pub fn on_face(name: String, face: FacePlane) -> Self {
        Self { face: Some(face), ..Self::new(name, OriginPlane::XY) }
    }

    /// The plane this sketch lies on (for drawing and picking).
    pub fn sketch_plane(&self) -> Plane {
        match self.face {
            Some(f) => f.plane(),
            None => self.plane.plane(),
        }
    }

    /// The plane in full precision (origin, u, v) for the geometry kernel.
    pub fn frame(&self) -> (glam::DVec3, glam::DVec3, glam::DVec3) {
        match self.face {
            Some(f) => (f.origin, f.u, f.v),
            None => {
                let p = self.plane.plane();
                (p.origin.as_dvec3(), p.u.as_dvec3(), p.v.as_dvec3())
            }
        }
    }

    /// Camera yaw/pitch (degrees) looking straight at the sketch.
    pub fn view_angles(&self) -> (f32, f32) {
        match self.face {
            Some(f) => crate::camera::angles_for_dir(f.normal().as_vec3()),
            None => self.plane.view_angles(),
        }
    }

    pub fn plane_name(&self) -> &'static str {
        if self.face.is_some() { "face" } else { self.plane.name() }
    }

    /// False if the anchor refers to a point that no longer exists
    /// (e.g. the cursor snapped to a point that an undo just removed).
    pub fn is_valid_anchor(&self, a: Anchor) -> bool {
        match a {
            Anchor::Point(id) => id < self.points.len(),
            Anchor::Free(p) => p.is_finite(),
            Anchor::OnLine(e, p) => self.is_line(e) && p.is_finite(),
            Anchor::OnAxis(axis, p) => axis < 2 && p.is_finite(),
        }
    }

    pub fn is_valid_sel(&self, s: Sel) -> bool {
        match s {
            Sel::Point(p) => p < self.points.len(),
            Sel::Entity(e) => e < self.entities.len(),
        }
    }

    pub fn pos(&self, a: Anchor) -> DVec2 {
        match a {
            Anchor::Point(id) => self.points[id],
            Anchor::Free(p) | Anchor::OnLine(_, p) | Anchor::OnAxis(_, p) => p,
        }
    }

    fn resolve(&mut self, a: Anchor) -> PointId {
        match a {
            Anchor::Point(id) => id,
            Anchor::Free(p) | Anchor::OnLine(_, p) | Anchor::OnAxis(_, p) => {
                self.points.push(p);
                self.points.len() - 1
            }
        }
    }

    pub fn is_line(&self, e: EntityId) -> bool {
        matches!(self.entities.get(e), Some(Entity::Line { .. }))
    }

    pub fn is_arc(&self, e: EntityId) -> bool {
        matches!(self.entities.get(e), Some(Entity::Arc { .. }))
    }

    /// A circle or an arc: something with a centre and a radius.
    pub fn is_round(&self, e: EntityId) -> bool {
        self.is_circle(e) || self.is_arc(e)
    }

    /// Centre, radius, start angle and counter-clockwise sweep of an arc.
    pub fn arc_geometry(&self, e: EntityId) -> Option<(DVec2, f64, f64, f64)> {
        match *self.entities.get(e)? {
            Entity::Arc { center, a, b } => {
                let (c, pa, pb) = (self.points[center], self.points[a], self.points[b]);
                let a0 = (pa - c).y.atan2((pa - c).x);
                let a1 = (pb - c).y.atan2((pb - c).x);
                let mut sweep = (a1 - a0).rem_euclid(std::f64::consts::TAU);
                if sweep < 1e-12 {
                    sweep = std::f64::consts::TAU;
                }
                Some((c, pa.distance(c), a0, sweep))
            }
            _ => None,
        }
    }

    pub fn is_circle(&self, e: EntityId) -> bool {
        matches!(self.entities.get(e), Some(Entity::Circle { .. }))
    }

    pub fn is_fixed(&self, p: PointId) -> bool {
        p == Self::ORIGIN || self.constraints.contains(&Constraint::Fix(p))
    }

    pub fn line_length(&self, e: EntityId) -> Option<f64> {
        match self.entities.get(e)? {
            Entity::Line { a, b } => Some(self.points[*a].distance(self.points[*b])),
            _ => None,
        }
    }

    pub fn entity_name(&self, e: EntityId) -> String {
        match self.entities.get(e) {
            Some(Entity::Line { .. }) => format!("Line{}", e + 1),
            Some(Entity::Circle { .. }) => format!("Circle{}", e + 1),
            Some(Entity::Point { .. }) => format!("Point{}", e + 1),
            Some(Entity::Arc { .. }) => format!("Arc{}", e + 1),
            None => "?".into(),
        }
    }

    /// Closest selectable thing within `tol` mm. Points win over curves.
    pub fn hit(&self, p: DVec2, tol: f64) -> Option<Sel> {
        let point = self
            .points
            .iter()
            .enumerate()
            .map(|(id, q)| (id, q.distance(p)))
            .filter(|&(_, d)| d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((id, _)) = point {
            return Some(Sel::Point(id));
        }
        self.entities
            .iter()
            .enumerate()
            .map(|(id, e)| {
                let d = match *e {
                    Entity::Line { a, b } => dist_to_segment(p, self.points[a], self.points[b]),
                    Entity::Circle { center, radius } => (p.distance(self.points[center]) - radius).abs(),
                    Entity::Point { .. } => f64::INFINITY, // picked as its point
                    Entity::Arc { .. } => {
                        let (c, r, a0, sweep) = self.arc_geometry(id).expect("an arc");
                        let ang = (p - c).y.atan2((p - c).x);
                        if (ang - a0).rem_euclid(std::f64::consts::TAU) <= sweep {
                            (p.distance(c) - r).abs()
                        } else {
                            let ends = [c + DVec2::from_angle(a0) * r, c + DVec2::from_angle(a0 + sweep) * r];
                            ends[0].distance(p).min(ends[1].distance(p))
                        }
                    }
                };
                (id, d)
            })
            .filter(|&(_, d)| d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| Sel::Entity(id))
    }

    /// Delete entities and points (a deleted point takes its entities with it).
    /// Constraints that referenced anything deleted are removed, unused points
    /// are dropped (except the origin), and all indices are renumbered.
    pub fn delete(&mut self, sel: &[Sel]) {
        let mut dead_pts: Vec<bool> = vec![false; self.points.len()];
        let mut dead_ents: Vec<bool> = vec![false; self.entities.len()];
        for s in sel {
            match *s {
                Sel::Point(p) if p != Self::ORIGIN && p < dead_pts.len() => dead_pts[p] = true,
                Sel::Entity(e) if e < dead_ents.len() => dead_ents[e] = true,
                _ => {}
            }
        }
        for (i, e) in self.entities.iter().enumerate() {
            let uses_dead = match *e {
                Entity::Line { a, b } => dead_pts[a] || dead_pts[b],
                Entity::Circle { center, .. } => dead_pts[center],
                Entity::Point { p } => dead_pts[p],
                Entity::Arc { center, a, b } => dead_pts[center] || dead_pts[a] || dead_pts[b],
            };
            if uses_dead {
                dead_ents[i] = true;
            }
        }

        // Drop constraints touching deleted things.
        self.retain_constraints(|c| {
            !c.entities().iter().any(|&e| dead_ents[e]) && !c.points().iter().any(|&p| dead_pts[p])
        });

        // Compact entities.
        let mut ent_map = vec![usize::MAX; self.entities.len()];
        let mut kept = Vec::new();
        for (i, e) in self.entities.iter().enumerate() {
            if !dead_ents[i] {
                ent_map[i] = kept.len();
                kept.push(*e);
            }
        }
        self.entities = kept;

        // Points still in use.
        let mut used = vec![false; self.points.len()];
        used[Self::ORIGIN] = true;
        for e in &self.entities {
            match *e {
                Entity::Line { a, b } => {
                    used[a] = true;
                    used[b] = true;
                }
                Entity::Circle { center, .. } => used[center] = true,
                Entity::Point { p } => used[p] = true,
                Entity::Arc { center, a, b } => {
                    used[center] = true;
                    used[a] = true;
                    used[b] = true;
                }
            }
        }
        // A Fix or distance on a point that no longer has geometry is meaningless.
        self.retain_constraints(|c| c.points().iter().all(|&p| used[p]));

        let mut pt_map = vec![usize::MAX; self.points.len()];
        let mut pts = Vec::new();
        for (i, p) in self.points.iter().enumerate() {
            if used[i] {
                pt_map[i] = pts.len();
                pts.push(*p);
            }
        }
        self.points = pts;

        for e in &mut self.entities {
            match e {
                Entity::Line { a, b } => {
                    *a = pt_map[*a];
                    *b = pt_map[*b];
                }
                Entity::Circle { center, .. } => *center = pt_map[*center],
                Entity::Point { p } => *p = pt_map[*p],
                Entity::Arc { center, a, b } => {
                    *center = pt_map[*center];
                    *a = pt_map[*a];
                    *b = pt_map[*b];
                }
            }
        }
        use Constraint::*;
        for c in &mut self.constraints {
            *c = match *c {
                Horizontal(e) => Horizontal(ent_map[e]),
                Vertical(e) => Vertical(ent_map[e]),
                Length(e, d) => Length(ent_map[e], d),
                Diameter(e, d) => Diameter(ent_map[e], d),
                Parallel(a, b) => Parallel(ent_map[a], ent_map[b]),
                Perpendicular(a, b) => Perpendicular(ent_map[a], ent_map[b]),
                Equal(a, b) => Equal(ent_map[a], ent_map[b]),
                Tangent(a, b) => Tangent(ent_map[a], ent_map[b]),
                Angle(a, b, d) => Angle(ent_map[a], ent_map[b], d),
                Distance(p, q, d) => Distance(pt_map[p], pt_map[q], d),
                OnCircle(p, c) => OnCircle(pt_map[p], ent_map[c]),
                OnLine(p, c) => OnLine(pt_map[p], ent_map[c]),
                Midpoint(p, a, b) => Midpoint(pt_map[p], pt_map[a], pt_map[b]),
                Radius(e, r) => Radius(ent_map[e], r),
                ArcEnds(e) => ArcEnds(ent_map[e]),
                OnAxis(p, axis) => OnAxis(pt_map[p], axis),
                Fix(p) => Fix(pt_map[p]),
            };
        }
    }
}

// ---------- Constraint actions ----------

/// Constraint buttons, in toolbar order.
pub const CONSTRAINT_ACTIONS: [&str; 8] =
    ["Dimension", "Horizontal", "Vertical", "Parallel", "Perpendicular", "Equal", "Tangent", "Fix"];

impl Sketch {
    /// Keep the constraints `keep` says yes to (and their label positions
    /// and driven flags).
    pub fn retain_constraints(&mut self, mut keep: impl FnMut(&Constraint) -> bool) {
        let shifts = std::mem::take(&mut self.label_shift);
        let driven = std::mem::take(&mut self.driven);
        let (mut kept, mut kept_driven) = (Vec::new(), Vec::new());
        let mut i = 0;
        self.constraints.retain(|c| {
            let k = keep(c);
            if k {
                kept.push(shifts.get(i).copied().unwrap_or(DVec2::ZERO));
                kept_driven.push(driven.get(i).copied().unwrap_or(false));
            }
            i += 1;
            k
        });
        while kept.last() == Some(&DVec2::ZERO) {
            kept.pop();
        }
        while kept_driven.last() == Some(&false) {
            kept_driven.pop();
        }
        self.label_shift = kept;
        self.driven = kept_driven;
    }

    /// Remove one constraint (and its label position and driven flag).
    pub fn remove_constraint(&mut self, ci: usize) {
        if ci < self.constraints.len() {
            self.constraints.remove(ci);
            if ci < self.label_shift.len() {
                self.label_shift.remove(ci);
            }
            if ci < self.driven.len() {
                self.driven.remove(ci);
                while self.driven.last() == Some(&false) {
                    self.driven.pop();
                }
            }
        }
    }

    /// Is constraint `ci` a driven (reference) dimension?
    pub fn is_driven(&self, ci: usize) -> bool {
        self.driven.get(ci).copied().unwrap_or(false)
    }

    pub fn set_driven(&mut self, ci: usize, driven: bool) {
        if self.driven.len() <= ci {
            if !driven {
                return;
            }
            self.driven.resize(ci + 1, false);
        }
        self.driven[ci] = driven;
        while self.driven.last() == Some(&false) {
            self.driven.pop();
        }
    }

    /// What a dimension constraint measures on the geometry right now.
    pub fn measure(&self, c: &Constraint) -> Option<f64> {
        match *c {
            Constraint::Length(e, _) => self.line_length(e),
            Constraint::Distance(p, q, _) => Some(self.points.get(p)?.distance(*self.points.get(q)?)),
            Constraint::Diameter(e, _) => match self.entities.get(e)? {
                Entity::Circle { radius, .. } => Some(2.0 * radius),
                _ => None,
            },
            Constraint::Radius(e, _) => self.arc_geometry(e).map(|(_, r, _, _)| r),
            Constraint::Angle(a, b, _) => self.angle_between(a, b),
            _ => None,
        }
    }

    /// Where a dimension's label was dragged to (sketch units from default).
    pub fn label_shift(&self, ci: usize) -> DVec2 {
        self.label_shift.get(ci).copied().unwrap_or(DVec2::ZERO)
    }

    pub fn set_label_shift(&mut self, ci: usize, shift: DVec2) {
        if self.label_shift.len() <= ci {
            self.label_shift.resize(ci + 1, DVec2::ZERO);
        }
        self.label_shift[ci] = shift;
    }

    /// The same geometry and constraints (ignores where labels sit, which
    /// doesn't change the shape — no need to rebuild the model for it).
    pub fn same_shape(&self, other: &Sketch) -> bool {
        self.plane == other.plane
            && self.face == other.face
            && self.points == other.points
            && self.entities == other.entities
            && self.constraints == other.constraints
            && self.driven == other.driven
    }

    /// How many lines/circles use each point: 2 or more means it's a joint
    /// where they're connected.
    pub fn point_uses(&self) -> Vec<usize> {
        let mut n = vec![0; self.points.len()];
        for e in &self.entities {
            match *e {
                Entity::Line { a, b } => {
                    n[a] += 1;
                    n[b] += 1;
                }
                Entity::Circle { center, .. } => n[center] += 1,
                Entity::Point { .. } => {}
                Entity::Arc { a, b, center } => {
                    n[a] += 1;
                    n[b] += 1;
                    n[center] += 1;
                }
            }
        }
        // Tied onto another line or circle: a joint too.
        for c in &self.constraints {
            if let Constraint::OnLine(p, _) | Constraint::OnCircle(p, _) | Constraint::OnAxis(p, _) = *c {
                if p < n.len() {
                    n[p] += 1;
                }
            }
        }
        n
    }

    /// Distance from a point to a line (extended).
    #[cfg(test)]
    pub fn line_distance(&self, e: EntityId, p: DVec2) -> Option<f64> {
        match self.entities.get(e)? {
            Entity::Line { a, b } => {
                let (a, b) = (self.points[*a], self.points[*b]);
                let d = b - a;
                Some(d.perp_dot(p - a).abs() / d.length().max(1e-12))
            }
            _ => None,
        }
    }

    /// The two lines meeting at corner point `p`, with their far ends.
    pub fn corner_lines(&self, p: PointId) -> Option<(EntityId, PointId, EntityId, PointId)> {
        let mut found = Vec::new();
        for (e, ent) in self.entities.iter().enumerate() {
            match *ent {
                Entity::Line { a, b } if a == p => found.push((e, b)),
                Entity::Line { a, b } if b == p => found.push((e, a)),
                Entity::Line { .. } | Entity::Point { .. } => {}
                // Anything round ending here: not a plain corner.
                Entity::Circle { center, .. } if center == p => return None,
                Entity::Arc { a, b, .. } if a == p || b == p => return None,
                Entity::Circle { .. } | Entity::Arc { .. } => {}
            }
        }
        match found.as_slice() {
            &[(l1, q1), (l2, q2)] => Some((l1, q1, l2, q2)),
            _ => None,
        }
    }

    /// The corner where lines `l1` and `l2` (and only they) meet.
    pub fn corner_between(&self, l1: EntityId, l2: EntityId) -> Option<PointId> {
        let ends = |e: EntityId| match self.entities.get(e) {
            Some(&Entity::Line { a, b }) => Some([a, b]),
            _ => None,
        };
        let (e1, e2) = (ends(l1)?, ends(l2)?);
        e1.into_iter()
            .filter(|p| e2.contains(p))
            .find(|&p| self.corner_lines(p).is_some_and(|(x, _, y, _)| (x, y) == (l1, l2) || (x, y) == (l2, l1)))
    }

    /// Shared part of fillet/chamfer: move the corner's two line ends to new
    /// points at `t` from the corner, keep the old corner point as a "virtual
    /// sharp" held where the lines meet, and turn lengths measured to the
    /// corner into distances to that sharp. Returns (line1, line2, new end 1,
    /// new end 2, the constraints to add).
    fn cut_corner(&mut self, p: PointId, t: f64) -> Result<(EntityId, EntityId, PointId, PointId, Vec<Constraint>), String> {
        let (l1, q1, l2, q2) = self.corner_lines(p).ok_or("pick a corner where exactly two lines meet")?;
        let corner = self.points[p];
        let (d1, d2) = (self.points[q1] - corner, self.points[q2] - corner);
        if t >= d1.length() - MIN_SIZE || t >= d2.length() - MIN_SIZE {
            return Err("too large for these lines".into());
        }
        let (t1, t2) = (corner + d1.normalize() * t, corner + d2.normalize() * t);
        self.points.push(t1);
        let n1 = self.points.len() - 1;
        self.points.push(t2);
        let n2 = self.points.len() - 1;
        for (l, n) in [(l1, n1), (l2, n2)] {
            if let Entity::Line { a, b } = &mut self.entities[l] {
                if *a == p {
                    *a = n;
                } else {
                    *b = n;
                }
            }
        }
        // Lengths of the two lines were measured to the corner: keep that.
        for c in &mut self.constraints {
            if let Constraint::Length(l, d) = *c {
                if l == l1 {
                    *c = Constraint::Distance(q1, p, d);
                } else if l == l2 {
                    *c = Constraint::Distance(q2, p, d);
                }
            }
        }
        self.entities.push(Entity::Point { p });
        Ok((l1, l2, n1, n2, vec![Constraint::OnLine(p, l1), Constraint::OnLine(p, l2)]))
    }

    /// Round the corner at point `p` with an arc of radius `r` tangent to both
    /// lines. Returns the constraints to add (tangents, the radius, and the
    /// virtual sharp's ties).
    pub fn fillet_corner(&mut self, p: PointId, r: f64) -> Result<Vec<Constraint>, String> {
        if !(r > 0.0) {
            return Err("the radius must be greater than zero".into());
        }
        let (_, q1, _, q2) = self.corner_lines(p).ok_or("pick a corner where exactly two lines meet")?;
        let corner = self.points[p];
        let (u1, u2) = ((self.points[q1] - corner).normalize(), (self.points[q2] - corner).normalize());
        let half = u1.dot(u2).clamp(-1.0, 1.0).acos() / 2.0;
        if half < 1e-6 || std::f64::consts::FRAC_PI_2 - half < 1e-6 {
            return Err("the lines are in line — there's no corner to round".into());
        }
        let mut trial = self.clone();
        let (l1, l2, n1, n2, mut out) = trial.cut_corner(p, r / half.tan())?;
        let center = corner + (u1 + u2).normalize() * (r / half.sin());
        trial.points.push(center);
        let c = trial.points.len() - 1;
        let (e1, e2) = (trial.points[n1] - center, trial.points[n2] - center);
        let (a, b) = if e1.perp_dot(e2) > 0.0 { (n1, n2) } else { (n2, n1) };
        trial.entities.push(Entity::Arc { center: c, a, b });
        let arc = trial.entities.len() - 1;
        out.extend([Constraint::Tangent(l1, arc), Constraint::Tangent(l2, arc), Constraint::Radius(arc, r)]);
        *self = trial;
        Ok(out)
    }

    /// Bevel the corner at point `p`, cutting `d` off each line.
    pub fn chamfer_corner(&mut self, p: PointId, d: f64) -> Result<Vec<Constraint>, String> {
        if !(d > 0.0) {
            return Err("the distance must be greater than zero".into());
        }
        let mut trial = self.clone();
        let (_, _, n1, n2, mut out) = trial.cut_corner(p, d)?;
        trial.entities.push(Entity::Line { a: n1, b: n2 });
        out.extend([Constraint::Distance(n1, p, d), Constraint::Distance(n2, p, d)]);
        *self = trial;
        Ok(out)
    }

    /// Where the perpendicular from `p` meets line `e` (extended).
    pub fn line_foot(&self, e: EntityId, p: DVec2) -> Option<DVec2> {
        match self.entities.get(e)? {
            Entity::Line { a, b } => {
                let (a, b) = (self.points[*a], self.points[*b]);
                let d = b - a;
                Some(a + d * ((p - a).dot(d) / d.length_squared().max(1e-30)))
            }
            _ => None,
        }
    }

    /// The angle from line `a` to line `b`, counter-clockwise, in 0..180°.
    pub fn angle_between(&self, a: EntityId, b: EntityId) -> Option<f64> {
        let dir = |e| match self.entities.get(e)? {
            Entity::Line { a, b } => Some(self.points[*b] - self.points[*a]),
            _ => None,
        };
        let (u, v) = (dir(a)?, dir(b)?);
        let deg = u.perp_dot(v).atan2(u.dot(v)).to_degrees().rem_euclid(180.0);
        Some(if deg >= 180.0 - 1e-12 { 0.0 } else { deg })
    }

    /// True if an equivalent constraint already exists (dimensions match by
    /// entity regardless of value; symmetric pairs match in either order).
    pub fn has_similar(&self, c: &Constraint) -> bool {
        use Constraint::*;
        self.constraints.iter().any(|x| match (*x, *c) {
            (Length(a, _), Length(b, _)) | (Diameter(a, _), Diameter(b, _)) | (Radius(a, _), Radius(b, _)) => a == b,
            (Angle(a, b, _), Angle(c, d, _)) | (Distance(a, b, _), Distance(c, d, _)) => {
                (a, b) == (c, d) || (a, b) == (d, c)
            }
            (Parallel(a, b), Parallel(c, d))
            | (Perpendicular(a, b), Perpendicular(c, d))
            | (Equal(a, b), Equal(c, d)) => (a, b) == (c, d) || (a, b) == (d, c),
            (x, c) => x == c,
        })
    }

    /// Which constraint actions apply to a selection and what each would add.
    pub fn constraint_options(&self, sel: &[Sel]) -> Vec<(&'static str, Vec<Constraint>)> {
        use Constraint::*;
        let (mut lines, mut circles, mut points) = (Vec::new(), Vec::new(), Vec::new());
        for s in sel {
            match *s {
                Sel::Point(p) => points.push(p),
                Sel::Entity(e) if self.is_line(e) => lines.push(e),
                Sel::Entity(e) if self.is_round(e) => circles.push(e),
                Sel::Entity(_) => {}
            }
        }
        let only_lines = !lines.is_empty() && circles.is_empty() && points.is_empty();
        let mut out: Vec<(&'static str, Vec<Constraint>)> = Vec::new();
        match (lines.as_slice(), circles.as_slice(), points.as_slice()) {
            ([l], [], []) => out.push(("Dimension", vec![Length(*l, self.line_length(*l).unwrap_or(0.0))])),
            ([], [c], []) => {
                // A circle gets a diameter, an arc a radius (as in Fusion).
                if let Some(Entity::Circle { radius, .. }) = self.entities.get(*c) {
                    out.push(("Dimension", vec![Diameter(*c, 2.0 * radius)]));
                } else if let Some((_, r, _, _)) = self.arc_geometry(*c) {
                    out.push(("Dimension", vec![Radius(*c, r)]));
                }
            }
            ([a, b], [], []) => {
                // Dimension on two lines measures the angle between them (as
                // in Fusion); parallel lines have no angle to dimension.
                if let Some(deg) = self.angle_between(*a, *b).filter(|d| *d > 1e-6) {
                    out.push(("Dimension", vec![Angle(*a, *b, deg)]));
                }
                out.push(("Parallel", vec![Parallel(*a, *b)]));
                out.push(("Perpendicular", vec![Perpendicular(*a, *b)]));
                out.push(("Equal", vec![Equal(*a, *b)]));
            }
            ([], [a, b], []) => out.push(("Equal", vec![Equal(*a, *b)])),
            ([l], [c], []) => out.push(("Tangent", vec![Tangent(*l, *c)])),
            // Two points: the distance between them.
            ([], [], [p, q]) if p != q => {
                out.push(("Dimension", vec![Distance(*p, *q, self.points[*p].distance(self.points[*q]))]));
            }
            _ => {}
        }
        if only_lines {
            out.push(("Horizontal", lines.iter().map(|&e| Horizontal(e)).collect()));
            out.push(("Vertical", lines.iter().map(|&e| Vertical(e)).collect()));
        }
        if lines.is_empty() && circles.is_empty() {
            out.push(("Fix", points.iter().filter(|&&p| p != Self::ORIGIN).map(|&p| Fix(p)).collect()));
        }
        for (_, cs) in &mut out {
            cs.retain(|c| !self.has_similar(c));
        }
        out.retain(|(_, cs)| !cs.is_empty());
        out
    }
}

// ---------- Horizontal / vertical inference ----------

/// Lines drawn within this many degrees of horizontal or vertical snap onto it
/// and get the matching constraint automatically.
pub const ALIGN_SNAP_DEG: f64 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Horizontal,
    Vertical,
}

/// Is the segment a→b close enough to horizontal or vertical to snap?
pub fn infer_align(a: DVec2, b: DVec2) -> Option<Align> {
    let d = (b - a).abs();
    if d.length() < MIN_SIZE {
        return None;
    }
    let deg = d.y.atan2(d.x).to_degrees(); // 0 = horizontal, 90 = vertical
    if deg <= ALIGN_SNAP_DEG {
        Some(Align::Horizontal)
    } else if deg >= 90.0 - ALIGN_SNAP_DEG {
        Some(Align::Vertical)
    } else {
        None
    }
}

// ---------- Tools ----------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Select,
    Line,
    /// Two opposite corners make four lines, kept horizontal and vertical.
    Rectangle,
    /// Two clicks for one side (any angle), a third for the other side.
    Rectangle3Point,
    /// The centre, then a corner; stays centred on that point.
    RectangleCenter,
    /// A lone sketch point.
    Point,
    /// Centre, then a point on the edge (Fusion's "Center Diameter Circle").
    Circle,
    /// Two points at opposite ends of a diameter.
    Circle2Point,
    /// Three points on the circle.
    Circle3Point,
    /// Two lines it touches, then a spot near where it should sit.
    Circle2Tangent,
    /// Arc: start, end, then a point it passes through.
    Arc3Point,
    /// Arc: centre, start (sets the radius), then where it ends.
    ArcCenter,
    /// Round a corner where two lines meet (pick the corner, type a radius).
    SketchFillet,
    /// Bevel a corner where two lines meet.
    SketchChamfer,
    /// Three lines it touches.
    Circle3Tangent,
    /// Pick geometry for a constraint ("Dimension", "Parallel", …): the
    /// constraint is added as soon as the picks are enough for it.
    Constrain(&'static str),
}

impl Tool {
    /// The arc tools, in the order of the toolbar's arc menu.
    pub const ARCS: [Tool; 2] = [Tool::Arc3Point, Tool::ArcCenter];

    /// The rectangle tools, in the order of the toolbar's rectangle menu.
    pub const RECTANGLES: [Tool; 3] = [Tool::Rectangle, Tool::Rectangle3Point, Tool::RectangleCenter];

    /// The circle tools, in the order of the toolbar's circle menu.
    pub const CIRCLES: [Tool; 5] =
        [Tool::Circle, Tool::Circle2Point, Tool::Circle3Point, Tool::Circle2Tangent, Tool::Circle3Tangent];

    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Line => "Line",
            Tool::Rectangle => "2-Point Rectangle",
            Tool::Rectangle3Point => "3-Point Rectangle",
            Tool::RectangleCenter => "Center Rectangle",
            Tool::Point => "Point",
            Tool::Circle => "Center Diameter Circle",
            Tool::Circle2Point => "2-Point Circle",
            Tool::Circle3Point => "3-Point Circle",
            Tool::Circle2Tangent => "2-Tangent Circle",
            Tool::Circle3Tangent => "3-Tangent Circle",
            Tool::Arc3Point => "3-Point Arc",
            Tool::ArcCenter => "Center Point Arc",
            Tool::SketchFillet => "Fillet",
            Tool::SketchChamfer => "Chamfer",
            Tool::Constrain(label) => label,
        }
    }

    /// How many lines a tangent-circle tool needs picked (0 for other tools).
    pub fn tangent_lines(self) -> usize {
        match self {
            Tool::Circle2Tangent => 2,
            Tool::Circle3Tangent => 3,
            _ => 0,
        }
    }
}

/// Drawing tool state machine. `start` is the first click of an unfinished
/// entity; nothing is added until the entity is complete, so cancelling never
/// leaves stray points.
#[derive(Default)]
pub struct Editor {
    pub tool: Tool,
    pub start: Option<Anchor>,
    /// 3-point circle: the second click.
    pub second: Option<Anchor>,
    /// Tangent circles: the lines picked so far.
    pub tangent_lines: Vec<EntityId>,
    pub selection: Vec<Sel>,
}

impl Editor {
    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.cancel();
    }

    pub fn cancel(&mut self) {
        self.start = None;
        self.second = None;
        self.tangent_lines.clear();
    }

    /// Is something half-made (a line started, circle points or lines picked)?
    pub fn busy(&self) -> bool {
        self.start.is_some() || self.second.is_some() || !self.tangent_lines.is_empty()
    }

    /// Tangent-circle tools: pick a line it should touch. Picking the same
    /// line twice or a non-line does nothing.
    pub fn pick_tangent_line(&mut self, sk: &Sketch, e: EntityId) {
        let need = self.tool.tangent_lines();
        if need > 0 && sk.is_line(e) && !self.tangent_lines.contains(&e) && self.tangent_lines.len() < need {
            self.tangent_lines.push(e);
        }
    }

    /// The circle a 2-point / 3-point / tangent tool would make with the
    /// cursor at `at` (for the preview and for the click that makes it).
    pub fn circle_preview(&self, sk: &Sketch, at: DVec2) -> Option<(DVec2, f64)> {
        let pos = |a: Anchor| sk.is_valid_anchor(a).then(|| sk.pos(a));
        match self.tool {
            Tool::Circle2Point => {
                let p = pos(self.start?)?;
                Some(((p + at) * 0.5, p.distance(at) * 0.5))
            }
            Tool::Circle3Point => circumcircle(pos(self.start?)?, pos(self.second?)?, at),
            Tool::Circle2Tangent | Tool::Circle3Tangent => self.tangent_circle(sk, at).ok(),
            _ => None,
        }
    }

    /// Click for the Line / Circle tools.
    /// Line tool: where the end point will really go and which alignment it
    /// gets. A free end close to horizontal/vertical is moved exactly onto it;
    /// an existing point stays put (the constraint pulls it in when solved).
    pub fn line_end(&self, sk: &Sketch, at: Anchor) -> (Anchor, Option<Align>) {
        let (Tool::Line, Some(start)) = (self.tool, self.start) else { return (at, None) };
        if !sk.is_valid_anchor(start) || !sk.is_valid_anchor(at) {
            return (at, None);
        }
        let a = sk.pos(start);
        let align = infer_align(a, sk.pos(at));
        let at = match (align, at) {
            (Some(Align::Horizontal), Anchor::Free(p)) => Anchor::Free(DVec2::new(p.x, a.y)),
            (Some(Align::Vertical), Anchor::Free(p)) => Anchor::Free(DVec2::new(a.x, p.y)),
            _ => at,
        };
        (at, align)
    }

    fn circle_preview_from(&self, sk: &Sketch, start: Anchor, second: Option<Anchor>, at: DVec2) -> Option<(DVec2, f64)> {
        let probe = Editor { start: Some(start), second, ..Editor { tool: self.tool, ..Default::default() } };
        probe.circle_preview(sk, at).filter(|&(_, r)| r >= MIN_SIZE)
    }

    /// The arc an arc tool would make with the cursor at `at` (third click).
    pub fn arc_preview(&self, sk: &Sketch, at: DVec2) -> Option<ArcShape> {
        let pos = |a: Anchor| sk.is_valid_anchor(a).then(|| sk.pos(a));
        let (p, q) = (pos(self.start?)?, pos(self.second?)?);
        let angle = |c: DVec2, x: DVec2| (x - c).y.atan2((x - c).x);
        match self.tool {
            Tool::Arc3Point => {
                // Through start p, the cursor, and end q.
                let (c, r) = circumcircle(p, at, q)?;
                let (ap, am, aq) = (angle(c, p), angle(c, at), angle(c, q));
                let ccw = |from: f64, to: f64| (to - from).rem_euclid(std::f64::consts::TAU);
                // Going counter-clockwise from p, do we pass the cursor before q?
                let passes = ccw(ap, am) < ccw(ap, aq);
                let (start, sweep) = if passes { (ap, ccw(ap, aq)) } else { (aq, ccw(aq, ap)) };
                Some(ArcShape { center: c, radius: r, start, sweep, end_angle: aq, flipped: !passes })
            }
            Tool::ArcCenter => {
                // Centre p, starting at q, ending in the cursor's direction —
                // whichever way round is shorter.
                let r = p.distance(q);
                let (aq, ae) = (angle(p, q), angle(p, at));
                let ccw = (ae - aq).rem_euclid(std::f64::consts::TAU);
                let (start, sweep, flipped) =
                    if ccw <= std::f64::consts::PI { (aq, ccw, false) } else { (ae, std::f64::consts::TAU - ccw, true) };
                Some(ArcShape { center: p, radius: r, start, sweep, end_angle: ae, flipped })
            }
            _ => None,
        }
    }

    /// 3-Point Arc, third click: the point to use for the raw cursor `raw`.
    /// The point is never grid-snapped, but the arc's *centre* can snap:
    /// to the chord midpoint (a half circle) when within `tol`, else — with
    /// `grid` on — to a grid point on the chord's perpendicular bisector.
    pub fn arc3_snap(&self, sk: &Sketch, raw: DVec2, grid: Option<GridSpec>, tol: f64) -> (DVec2, Arc3Snap) {
        let pos = |a: Option<Anchor>| a.filter(|&a| sk.is_valid_anchor(a)).map(|a| sk.pos(a));
        let (Tool::Arc3Point, Some(p), Some(q)) = (self.tool, pos(self.start), pos(self.second)) else {
            return (raw, Arc3Snap::None);
        };
        let chord = q - p;
        if chord.length() < MIN_SIZE {
            return (raw, Arc3Snap::None);
        }
        let mid = (p + q) * 0.5;
        let n = chord.perp() / chord.length();
        let side = if n.dot(raw - mid) < 0.0 { -1.0 } else { 1.0 };
        let Some((c, _)) = circumcircle(p, raw, q) else { return (raw, Arc3Snap::None) };
        if c.distance(mid) <= tol {
            return (mid + n * side * chord.length() * 0.5, Arc3Snap::HalfCircle);
        }
        if let Some(g) = grid {
            let gp = g.snap(c);
            // Only grid points that really lie on the bisector count.
            let on_bisector = (gp - mid).dot(chord.normalize()).abs() < 1e-6 * g.step;
            if on_bisector && gp.distance(c) <= tol {
                let r = gp.distance(p);
                let dir = (raw - gp).try_normalize().unwrap_or(n * side);
                return (gp + dir * r, Arc3Snap::GridCenter(gp));
            }
        }
        (raw, Arc3Snap::None)
    }

    /// The four corners a rectangle tool would make with the cursor at `at`
    /// (in order around it), for the preview and the click that makes it.
    pub fn rectangle_preview(&self, sk: &Sketch, at: DVec2) -> Option<[DVec2; 4]> {
        let pos = |a: Anchor| sk.is_valid_anchor(a).then(|| sk.pos(a));
        let p = pos(self.start?)?;
        match self.tool {
            Tool::Rectangle => Some([p, DVec2::new(at.x, p.y), at, DVec2::new(p.x, at.y)]),
            Tool::RectangleCenter => {
                let d = at - p;
                Some([p - d, DVec2::new(p.x + d.x, p.y - d.y), p + d, DVec2::new(p.x - d.x, p.y + d.y)])
            }
            Tool::Rectangle3Point => {
                let q = pos(self.second?)?;
                let side = q - p;
                if side.length() < MIN_SIZE {
                    return None;
                }
                let n = side.perp() / side.length();
                let h = n.dot(at - q);
                Some([p, q, q + n * h, p + n * h])
            }
            _ => None,
        }
    }

    /// The tangent circle for the lines picked so far, in the cursor's region.
    pub fn tangent_circle(&self, sk: &Sketch, at: DVec2) -> Result<(DVec2, f64), String> {
        match (self.tool, self.tangent_lines.as_slice()) {
            (Tool::Circle2Tangent, &[a, b]) => tangent_circle_2(sk, [a, b], at),
            (Tool::Circle3Tangent, &[a, b, c]) => tangent_circle_3(sk, [a, b, c], at),
            _ => Err("pick the lines first".into()),
        }
    }

    /// Tangent-circle tools: the click that places the circle (once the lines
    /// are picked). Returns the tangent constraints to add, or why it can't.
    pub fn tangent_click(&mut self, sk: &mut Sketch, at: DVec2) -> Result<Vec<Constraint>, String> {
        let (c, r) = self.tangent_circle(sk, at)?;
        if r < MIN_SIZE {
            return Err("that circle would be too small".into());
        }
        let lines = std::mem::take(&mut self.tangent_lines);
        let center = sk.resolve(Anchor::Free(c));
        sk.entities.push(Entity::Circle { center, radius: r });
        let circle = sk.entities.len() - 1;
        Ok(lines.iter().map(|&l| Constraint::Tangent(l, circle)).collect())
    }

    /// Click for the drawing tools. Returns constraints the new geometry
    /// should get (horizontal/vertical); the caller adds each through the
    /// solver so a redundant one is simply skipped.
    pub fn click(&mut self, sk: &mut Sketch, at: Anchor) -> Vec<Constraint> {
        // These tools pick existing geometry instead (see pick_tangent_line,
        // tangent_click, and the app's constraint picking).
        if matches!(
            self.tool,
            Tool::Select
                | Tool::Constrain(_)
                | Tool::Circle2Tangent
                | Tool::Circle3Tangent
                | Tool::SketchFillet
                | Tool::SketchChamfer
        ) {
            return Vec::new();
        }
        // Never act on points that have gone away (undo/delete): just reset the tool.
        let gone = |a: Option<Anchor>| a.is_some_and(|a| !sk.is_valid_anchor(a));
        if !sk.is_valid_anchor(at) || gone(self.start) || gone(self.second) {
            self.cancel();
            return Vec::new();
        }
        // A point dropped on a line stays on it: every new point made from an
        // OnLine click gets an OnLine constraint.
        let mut ties: Vec<Constraint> = Vec::new();
        let mut resolve = |sk: &mut Sketch, a: Anchor| {
            let p = sk.resolve(a);
            match a {
                Anchor::OnLine(e, _) => ties.push(Constraint::OnLine(p, e)),
                Anchor::OnAxis(axis, _) => ties.push(Constraint::OnAxis(p, axis)),
                _ => {}
            }
            p
        };
        if self.tool == Tool::Point {
            if !matches!(at, Anchor::Point(_)) {
                let p = resolve(sk, at);
                sk.entities.push(Entity::Point { p });
            }
            return ties;
        }
        let Some(start) = self.start else {
            self.start = Some(at);
            return Vec::new();
        };
        let (at, align) = self.line_end(sk, at);
        let size = sk.pos(start).distance(sk.pos(at));
        match self.tool {
            Tool::Line => {
                if size < MIN_SIZE {
                    self.start = None; // same spot twice: end the chain
                    return Vec::new();
                }
                let a = resolve(sk, start);
                let b = resolve(sk, at);
                sk.entities.push(Entity::Line { a, b });
                let e = sk.entities.len() - 1;
                // Clicking an existing point closes the shape; otherwise keep chaining.
                self.start = match at {
                    Anchor::Point(_) => None,
                    Anchor::Free(_) | Anchor::OnLine(..) | Anchor::OnAxis(..) => Some(Anchor::Point(b)),
                };
                let mut out: Vec<Constraint> = align
                    .map(|al| match al {
                        Align::Horizontal => Constraint::Horizontal(e),
                        Align::Vertical => Constraint::Vertical(e),
                    })
                    .into_iter()
                    .collect();
                out.extend(ties);
                out
            }
            Tool::Rectangle3Point if self.second.is_none() => {
                if size >= MIN_SIZE {
                    self.second = Some(at); // first side done; next click sets the other side
                }
                Vec::new()
            }
            Tool::Rectangle3Point => {
                let second = self.second.take().expect("checked above");
                self.start = None;
                let probe = Editor { tool: self.tool, start: Some(start), second: Some(second), ..Default::default() };
                let Some(c) = probe.rectangle_preview(sk, sk.pos(at)) else { return Vec::new() };
                if (c[2] - c[1]).length() < MIN_SIZE {
                    return Vec::new();
                }
                let a = resolve(sk, start);
                let b = resolve(sk, second);
                let cc = sk.resolve(Anchor::Free(c[2]));
                let d = sk.resolve(Anchor::Free(c[3]));
                let first = sk.entities.len();
                for (u, v) in [(a, b), (b, cc), (cc, d), (d, a)] {
                    sk.entities.push(Entity::Line { a: u, b: v });
                }
                // Square corners and opposite sides parallel, at any angle.
                let mut out = vec![
                    Constraint::Perpendicular(first, first + 1),
                    Constraint::Parallel(first, first + 2),
                    Constraint::Parallel(first + 1, first + 3),
                ];
                out.extend(ties);
                out
            }
            Tool::RectangleCenter => {
                self.start = None;
                let probe = Editor { tool: self.tool, start: Some(start), ..Default::default() };
                let Some(c) = probe.rectangle_preview(sk, sk.pos(at)) else { return Vec::new() };
                if (c[1] - c[0]).length() < MIN_SIZE || (c[3] - c[0]).length() < MIN_SIZE {
                    return Vec::new();
                }
                // The centre is a point of its own that stays in the middle.
                let center = resolve(sk, start);
                if !sk.entities.iter().any(|e| matches!(e, Entity::Point { p } if *p == center))
                    && !sk.entities.iter().any(|e| match *e {
                        Entity::Line { a, b } => a == center || b == center,
                        Entity::Circle { center: c, .. } => c == center,
                        Entity::Arc { center: c, a, b } => c == center || a == center || b == center,
                        Entity::Point { .. } => false,
                    })
                {
                    sk.entities.push(Entity::Point { p: center });
                }
                let corner = c.map(|q| sk.resolve(Anchor::Free(q)));
                let first = sk.entities.len();
                for k in 0..4 {
                    sk.entities.push(Entity::Line { a: corner[k], b: corner[(k + 1) % 4] });
                }
                let mut out = vec![
                    Constraint::Horizontal(first),
                    Constraint::Vertical(first + 1),
                    Constraint::Horizontal(first + 2),
                    Constraint::Vertical(first + 3),
                    Constraint::Midpoint(center, corner[0], corner[2]),
                ];
                out.extend(ties);
                out
            }
            Tool::Rectangle => {
                self.start = None;
                let (p, q) = (sk.pos(start), sk.pos(at));
                if (q.x - p.x).abs() < MIN_SIZE || (q.y - p.y).abs() < MIN_SIZE {
                    return Vec::new(); // a flat "rectangle": ignore it
                }
                let a = resolve(sk, start);
                let c = resolve(sk, at);
                let b = sk.resolve(Anchor::Free(DVec2::new(q.x, p.y)));
                let d = sk.resolve(Anchor::Free(DVec2::new(p.x, q.y)));
                let first = sk.entities.len();
                for (u, v) in [(a, b), (b, c), (c, d), (d, a)] {
                    sk.entities.push(Entity::Line { a: u, b: v });
                }
                let mut out = vec![
                    Constraint::Horizontal(first),
                    Constraint::Vertical(first + 1),
                    Constraint::Horizontal(first + 2),
                    Constraint::Vertical(first + 3),
                ];
                out.extend(ties);
                out
            }
            Tool::Circle => {
                self.start = None;
                // Edge put on a line right where the circle would touch it
                // (the app snaps it there when it's close): tangent.
                let touching = match at {
                    Anchor::OnLine(e, q) => {
                        let c = sk.pos(start);
                        sk.line_foot(e, c).filter(|f| f.distance(q) < 1e-9 * (1.0 + size)).map(|_| e)
                    }
                    _ => None,
                };
                if size < MIN_SIZE {
                    return Vec::new();
                }
                let center = resolve(sk, start);
                sk.entities.push(Entity::Circle { center, radius: size });
                let mut out = ties;
                if let Some(e) = touching {
                    out.push(Constraint::Tangent(e, sk.entities.len() - 1));
                }
                out
            }
            Tool::Circle2Point => {
                self.start = None;
                let Some((c, r)) = self.circle_preview_from(sk, start, None, sk.pos(at)) else { return Vec::new() };
                add_circle_through(sk, c, r, &[start, at])
            }
            Tool::Circle3Point => {
                let Some(second) = self.second else {
                    if size >= MIN_SIZE {
                        self.second = Some(at);
                    }
                    return Vec::new();
                };
                self.start = None;
                self.second = None;
                let Some((c, r)) = self.circle_preview_from(sk, start, Some(second), sk.pos(at)) else {
                    return Vec::new();
                };
                add_circle_through(sk, c, r, &[start, second, at])
            }
            Tool::Arc3Point | Tool::ArcCenter if self.second.is_none() => {
                if size >= MIN_SIZE {
                    self.second = Some(at);
                }
                Vec::new()
            }
            Tool::Arc3Point | Tool::ArcCenter => {
                let second = self.second.take().expect("checked above");
                self.start = None;
                let probe = Editor { tool: self.tool, start: Some(start), second: Some(second), ..Default::default() };
                let Some(arc) = probe.arc_preview(sk, sk.pos(at)) else { return Vec::new() };
                if arc.radius < MIN_SIZE || arc.sweep < 1e-6 {
                    return Vec::new();
                }
                // Which clicks are which end, and which are made fresh.
                let (from, to, center) = match self.tool {
                    Tool::Arc3Point => (start, second, Anchor::Free(arc.center)),
                    _ => (second, Anchor::Free(arc.center + DVec2::from_angle(arc.end_angle) * arc.radius), start),
                };
                let c = resolve(sk, center);
                let pa = resolve(sk, from);
                let pb = resolve(sk, to);
                // Stored counter-clockwise from a to b.
                let (a, b) = if arc.flipped { (pb, pa) } else { (pa, pb) };
                sk.entities.push(Entity::Arc { center: c, a, b });
                ties
            }
            Tool::Circle2Tangent | Tool::Circle3Tangent | Tool::Constrain(_) | Tool::Point => Vec::new(),
            Tool::SketchFillet | Tool::SketchChamfer => Vec::new(),
            Tool::Select => Vec::new(),
        }
    }

    /// Select tool: clicking toggles an item; clicking empty space clears.
    pub fn select_click(&mut self, hit: Option<Sel>) {
        match hit {
            Some(s) => {
                if let Some(i) = self.selection.iter().position(|&x| x == s) {
                    self.selection.remove(i);
                } else {
                    self.selection.push(s);
                }
            }
            None => self.selection.clear(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::OrbitCamera;

    fn free(x: f64, y: f64) -> Anchor {
        Anchor::Free(DVec2::new(x, y))
    }

    fn line_tool() -> Editor {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed
    }

    #[test]
    fn plane_normals_face_the_camera_views() {
        for p in OriginPlane::ALL {
            let mut c = OrbitCamera::default();
            let (yaw, pitch) = p.view_angles();
            c.look_from(yaw, pitch);
            let (forward, right, up) = c.basis();
            let pl = p.plane();
            assert!(forward.dot(pl.normal()) < -0.99, "{:?} faces camera", p);
            assert!(right.dot(pl.u) > 0.99, "{:?} u is screen-right", p);
            assert!(up.dot(pl.v) > 0.99, "{:?} v is screen-up", p);
        }
    }

    #[test]
    fn face_planes_are_square_and_line_up_with_the_camera() {
        use glam::DVec3;
        let faces = [
            (DVec3::Z, DVec3::new(3.0, 4.0, 12.5)),
            (DVec3::NEG_Z, DVec3::new(0.0, 0.0, -2.0)),
            (DVec3::NEG_Y, DVec3::new(7.0, -5.0, 1.0)),
            (DVec3::X, DVec3::new(30.0, 1.0, 2.0)),
            (DVec3::new(1.0, -1.0, 1.0), DVec3::new(10.0, 0.0, 0.0)),
        ];
        for (n, p) in faces {
            let f = FacePlane::from_face(n, p);
            let n = n.normalize();
            assert!((f.u.length() - 1.0).abs() < 1e-12 && (f.v.length() - 1.0).abs() < 1e-12);
            assert!(f.u.dot(f.v).abs() < 1e-12);
            assert!((f.normal() - n).length() < 1e-12, "u × v is the outward normal");
            assert!((f.origin - p).dot(n).abs() < 1e-9, "origin lies on the face plane");
            assert!(f.origin.cross(n).length() < 1e-9, "origin is the plane point nearest the world origin");
            // Looking straight at it, u is screen-right and v screen-up.
            let sk = Sketch::on_face("S".into(), f);
            let mut c = OrbitCamera::default();
            let (yaw, pitch) = sk.view_angles();
            c.look_from(yaw, pitch);
            let (forward, right, up) = c.basis();
            assert!(forward.dot(-n.as_vec3()) > 0.999, "{n}");
            assert!(right.dot(f.u.as_vec3()) > 0.999 && up.dot(f.v.as_vec3()) > 0.999, "{n}");
        }
        // The top face of a 10-thick part is the XY plane lifted by 10.
        let top = FacePlane::from_face(DVec3::Z, DVec3::new(5.0, 5.0, 10.0));
        assert_eq!((top.origin, top.u, top.v), (DVec3::new(0.0, 0.0, 10.0), DVec3::X, DVec3::Y));
    }

    #[test]
    fn ray_hits_plane_at_expected_local_point() {
        let pl = OriginPlane::XZ.plane();
        let ray = Ray { origin: Vec3::new(5.0, -100.0, 7.0), dir: Vec3::Y };
        let hit = pl.intersect(&ray).unwrap();
        assert!((hit - DVec2::new(5.0, 7.0)).length() < 1e-5);
        assert!((pl.to_world(hit) - Vec3::new(5.0, 0.0, 7.0)).length() < 1e-5);
    }

    #[test]
    fn parallel_or_backward_ray_misses() {
        let pl = OriginPlane::XY.plane();
        assert!(pl.intersect(&Ray { origin: Vec3::new(0.0, 0.0, 5.0), dir: Vec3::X }).is_none());
        assert!(pl.intersect(&Ray { origin: Vec3::new(0.0, 0.0, 5.0), dir: Vec3::Z }).is_none());
    }

    #[test]
    fn camera_pick_lands_on_plane_under_cursor() {
        let mut c = OrbitCamera::default();
        c.look_from(-90.0, 89.0);
        let hit = OriginPlane::XY.plane().intersect(&c.ray_from_ndc(1.0, Vec2::ZERO)).unwrap();
        assert!(hit.length() < 1e-3);
    }

    #[test]
    fn grid_step_follows_zoom() {
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let g = GridSpec::for_distance(300.0);
        assert!(close(g.step, 10.0) && g.major_every == 5);
        let g = GridSpec::for_distance(100.0);
        assert!(close(g.step, 5.0) && g.major_every == 2);
        assert!(close(GridSpec::for_distance(50.0).step, 2.0));
        assert!(close(GridSpec::for_distance(20.0).step, 1.0));
        assert!(close(GridSpec::for_distance(3000.0).step, 100.0));
    }

    #[test]
    fn grid_lines_skip_axes_and_mark_majors() {
        let g = GridSpec { step: 10.0, major_every: 5 };
        let lines = grid_lines(&OriginPlane::XY.plane(), DVec2::ZERO, g, 10);
        assert_eq!(lines.len(), 2 * 20, "21 positions each way minus the axis");
        assert!(lines.iter().all(|(a, b, _)| a.z == 0.0 && b.z == 0.0));
        assert_eq!(lines.iter().filter(|l| l.2).count(), 2 * 4, "±50 and ±100 in both directions");
    }

    #[test]
    fn grid_snap_rounds_to_nearest_intersection() {
        let g = GridSpec { step: 5.0, major_every: 2 };
        assert_eq!(g.snap(DVec2::new(13.7, -6.2)), DVec2::new(15.0, -5.0));
        assert_eq!(g.snap(DVec2::new(-2.4, 2.6)), DVec2::new(0.0, 5.0));
    }

    #[test]
    fn triangle_closes_and_shares_points() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, free(0.0, 10.0));
        ed.click(&mut sk, Anchor::Point(1));
        assert_eq!(sk.entities.len(), 3);
        assert_eq!(sk.points.len(), 4);
        assert_eq!(ed.start, None);
    }

    #[test]
    fn cancel_leaves_no_stray_points() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(3.0, 3.0));
        ed.cancel();
        assert_eq!(sk.points.len(), 1);
        assert!(sk.entities.is_empty());
    }

    #[test]
    fn double_click_ends_chain() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, Anchor::Point(2));
        assert_eq!(sk.entities.len(), 1);
        assert_eq!(ed.start, None);
    }

    #[test]
    fn circle_from_center_and_edge() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Point(Sketch::ORIGIN));
        ed.click(&mut sk, free(3.0, 4.0));
        assert_eq!(sk.entities, vec![Entity::Circle { center: 0, radius: 5.0 }]);
        assert_eq!(sk.points.len(), 1);
    }

    #[test]
    fn stale_anchors_after_undo_do_not_crash() {
        // Draw a chain, then simulate an undo that removes its points while the
        // tool still remembers one of them.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let before = sk.clone();
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 0.0)); // start is now Point(2)
        assert_eq!(ed.start, Some(Anchor::Point(2)));
        sk = before; // undo: only the origin is left
        assert!(!sk.is_valid_anchor(Anchor::Point(2)));
        ed.click(&mut sk, free(5.0, 5.0)); // must not panic
        assert_eq!(ed.start, None, "tool reset instead of using a missing point");
        ed.click(&mut sk, Anchor::Point(7)); // clicking a stale snap target
        assert!(sk.entities.is_empty());
        assert!(!sk.is_valid_sel(Sel::Entity(0)) && sk.is_valid_sel(Sel::Point(0)));
    }

    #[test]
    fn nearly_horizontal_or_vertical_lines_snap_and_infer() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 0.0));
        // 3° off horizontal: end point is pulled level and Horizontal is suggested.
        let c = ed.click(&mut sk, free(40.0, 40.0 * 3f64.to_radians().tan()));
        assert_eq!(c, vec![Constraint::Horizontal(0)]);
        assert_eq!(sk.points[2], DVec2::new(40.0, 0.0));
        // Near vertical from there.
        let c = ed.click(&mut sk, free(41.5, 30.0));
        assert_eq!(c, vec![Constraint::Vertical(1)]);
        assert_eq!(sk.points[3], DVec2::new(40.0, 30.0));
        // 45°: nothing inferred, point stays where clicked.
        let c = ed.click(&mut sk, free(60.0, 50.0));
        assert!(c.is_empty());
        assert_eq!(sk.points[4], DVec2::new(60.0, 50.0));
    }

    #[test]
    fn inference_keeps_existing_points_where_they_are() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 10.0));
        ed.click(&mut sk, free(30.0, 40.0)); // diagonal, no inference
        ed.cancel();
        ed.click(&mut sk, free(31.0, 0.0));
        // Ending on the existing point (30, 40) — almost vertical. The point isn't
        // moved here; the suggested Vertical constraint does that when solved.
        let c = ed.click(&mut sk, Anchor::Point(2));
        assert_eq!(c, vec![Constraint::Vertical(1)]);
        assert_eq!(sk.points[2], DVec2::new(30.0, 40.0));
    }

    #[test]
    fn rectangle_tool_makes_four_lines_four_points_and_hv_constraints() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        assert!(ed.click(&mut sk, free(10.0, 5.0)).is_empty());
        let cs = ed.click(&mut sk, free(40.0, 25.0));
        assert_eq!(sk.points.len(), 1 + 4, "origin + four corners");
        assert_eq!(sk.entities.len(), 4);
        assert_eq!(
            cs,
            vec![Constraint::Horizontal(0), Constraint::Vertical(1), Constraint::Horizontal(2), Constraint::Vertical(3)]
        );
        // Corners in order around the rectangle, closed.
        let corners: Vec<DVec2> = sk
            .entities
            .iter()
            .map(|e| match *e {
                Entity::Line { a, .. } => sk.points[a],
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            corners,
            vec![DVec2::new(10.0, 5.0), DVec2::new(40.0, 5.0), DVec2::new(40.0, 25.0), DVec2::new(10.0, 25.0)]
        );
        assert!(matches!(sk.entities[3], Entity::Line { b: 1, .. }), "last side returns to the first corner");
        assert_eq!(ed.start, None, "ready for the next rectangle");
        // Starting on an existing point reuses it; a flat rectangle is ignored.
        ed.click(&mut sk, Anchor::Point(Sketch::ORIGIN));
        assert!(ed.click(&mut sk, free(30.0, 0.0)).is_empty());
        assert_eq!(sk.entities.len(), 4);
    }

    #[test]
    fn two_lines_offer_an_angle_dimension_at_their_current_angle() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let a = sk.resolve(Anchor::Free(DVec2::new(10.0, 0.0)));
        let b = sk.resolve(Anchor::Free(DVec2::new(30.0, 0.0)));
        let c = sk.resolve(Anchor::Free(DVec2::new(10.0, 0.0) + DVec2::from_angle(30f64.to_radians()) * 20.0));
        sk.entities.push(Entity::Line { a, b });
        sk.entities.push(Entity::Line { a, b: c });
        let opts = sk.constraint_options(&[Sel::Entity(0), Sel::Entity(1)]);
        let dim = opts.iter().find(|(l, _)| *l == "Dimension").expect("Dimension on two lines");
        assert!(matches!(dim.1[0], Constraint::Angle(0, 1, d) if (d - 30.0).abs() < 1e-9));
        // Lines have no direction: flipping one gives the same angle.
        sk.entities[1] = Entity::Line { a: c, b: a };
        assert!((sk.angle_between(0, 1).unwrap() - 30.0).abs() < 1e-9);
        // Already there: not offered again.
        sk.constraints.push(Constraint::Angle(1, 0, 150.0));
        let opts = sk.constraint_options(&[Sel::Entity(0), Sel::Entity(1)]);
        assert!(opts.iter().all(|(l, cs)| *l != "Dimension" || cs.is_empty()));
    }

    #[test]
    fn two_points_dimension_to_their_distance_and_labels_follow_deletes() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, free(10.0, 0.0)); // end chain
        ed.click(&mut sk, free(20.0, 5.0));
        ed.click(&mut sk, free(30.0, 5.0));
        // Points 2 (end of the first line) and 3 (start of the second).
        let opts = sk.constraint_options(&[Sel::Point(2), Sel::Point(3)]);
        let dim = opts.iter().find(|(l, _)| *l == "Dimension").unwrap();
        assert!(matches!(dim.1[0], Constraint::Distance(2, 3, d) if (d - 125f64.sqrt()).abs() < 1e-9));
        // Labels stay with their constraints when others are deleted.
        sk.constraints = vec![Constraint::Horizontal(0), Constraint::Distance(2, 3, 11.0), Constraint::Length(1, 10.0)];
        sk.set_label_shift(1, DVec2::new(1.0, 2.0));
        sk.set_label_shift(2, DVec2::new(3.0, 4.0));
        sk.remove_constraint(0);
        assert_eq!((sk.label_shift(0), sk.label_shift(1)), (DVec2::new(1.0, 2.0), DVec2::new(3.0, 4.0)));
        sk.delete(&[Sel::Entity(0)]); // takes point 1 with it → the distance goes
        assert_eq!(sk.constraints.len(), 1);
        assert!(matches!(sk.constraints[0], Constraint::Length(0, _)));
        assert_eq!(sk.label_shift(0), DVec2::new(3.0, 4.0), "the length's label kept its spot");
        // Moving a label isn't a change of shape.
        let mut moved = sk.clone();
        moved.set_label_shift(0, DVec2::new(9.0, 9.0));
        assert!(moved.same_shape(&sk) && moved != sk);
    }

    #[test]
    fn shared_points_are_counted_as_joints() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 5.0));
        let uses = sk.point_uses();
        assert!(uses.iter().skip(1).all(|&n| n == 2), "every corner joins two lines: {uses:?}");
    }

    #[test]
    fn circumcircle_and_incircle() {
        let (c, r) = circumcircle(DVec2::new(10.0, 0.0), DVec2::new(0.0, 10.0), DVec2::new(-10.0, 0.0)).unwrap();
        assert!(c.length() < 1e-12 && (r - 10.0).abs() < 1e-12);
        assert!(circumcircle(DVec2::ZERO, DVec2::X, DVec2::X * 2.0).is_none(), "points in a line");
        // 3-4-5 right triangle: inradius 1, centre (1, 1).
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        // A closed triangle: the last click lands back on the first point.
        for at in [free(0.0, 0.0), free(4.0, 0.0), free(0.0, 3.0), Anchor::Point(1)] {
            ed.click(&mut sk, at);
        }
        let (c, r) = incircle(&sk, 0, 1, 2).unwrap();
        assert!((c - DVec2::new(1.0, 1.0)).length() < 1e-9 && (r - 1.0).abs() < 1e-9, "{c} {r}");
    }

    #[test]
    fn two_and_three_point_circles() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle2Point);
        assert!(ed.click(&mut sk, free(0.0, 0.0)).is_empty());
        assert_eq!(ed.circle_preview(&sk, DVec2::new(10.0, 0.0)), Some((DVec2::new(5.0, 0.0), 5.0)));
        ed.click(&mut sk, free(10.0, 0.0));
        assert!(matches!(sk.entities[0], Entity::Circle { radius, .. } if (radius - 5.0).abs() < 1e-12));
        assert_eq!(sk.points.len(), 2, "origin + centre; free clicks leave no stray points");
        // Through the origin and another existing point: kept on the circle.
        ed.set_tool(Tool::Circle3Point);
        ed.click(&mut sk, Anchor::Point(Sketch::ORIGIN));
        ed.click(&mut sk, free(0.0, 20.0));
        let cs = ed.click(&mut sk, free(20.0, 0.0));
        assert_eq!(cs, vec![Constraint::OnCircle(Sketch::ORIGIN, 1)]);
        assert!(matches!(sk.entities[1], Entity::Circle { radius, .. } if (radius - 200f64.sqrt()).abs() < 1e-9));
        // Three points in a line make nothing.
        ed.click(&mut sk, free(0.0, -5.0));
        ed.click(&mut sk, free(1.0, -5.0));
        assert!(ed.click(&mut sk, free(2.0, -5.0)).is_empty());
        assert_eq!(sk.entities.len(), 2);
    }

    #[test]
    fn tangent_circle_tools_pick_lines_then_make_the_circle() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(30.0, 0.0));
        ed.cancel();
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(0.0, 30.0));
        ed.set_tool(Tool::Circle2Tangent);
        assert!(ed.click(&mut sk, free(5.0, 5.0)).is_empty(), "plain clicks don't draw");
        ed.pick_tangent_line(&sk, 0);
        ed.pick_tangent_line(&sk, 0); // twice: ignored
        assert!(ed.tangent_click(&mut sk, DVec2::new(6.0, 4.0)).is_err(), "needs two lines");
        ed.pick_tangent_line(&sk, 1);
        let cs = ed.tangent_click(&mut sk, DVec2::new(6.0, 4.0));
        assert_eq!(cs, Ok(vec![Constraint::Tangent(0, 2), Constraint::Tangent(1, 2)]));
        assert!(ed.tangent_lines.is_empty(), "ready for the next one");
    }

    /// Lines (as entity ids) through the given point pairs.
    fn lines(sk: &mut Sketch, segs: &[((f64, f64), (f64, f64))]) -> Vec<EntityId> {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        segs.iter()
            .map(|&(a, b)| {
                ed.cancel();
                ed.click(sk, free(a.0, a.1));
                ed.click(sk, free(b.0, b.1));
                sk.entities.len() - 1
            })
            .collect()
    }

    fn touches(sk: &Sketch, e: EntityId, (c, r): (DVec2, f64)) -> bool {
        (sk.line_distance(e, c).unwrap() - r).abs() < 1e-9
    }

    #[test]
    fn three_tangent_circle_works_on_any_three_lines() {
        // Three sides of a square: two parallel lines and one across them.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = lines(&mut sk, &[((0.0, 0.0), (30.0, 0.0)), ((0.0, 30.0), (30.0, 30.0)), ((0.0, 0.0), (0.0, 30.0))]);
        let circle = tangent_circle_3(&sk, [l[0], l[1], l[2]], DVec2::new(10.0, 12.0)).unwrap();
        assert!((circle.0 - DVec2::new(15.0, 15.0)).length() < 1e-9 && (circle.1 - 15.0).abs() < 1e-9, "{circle:?}");
        // Cursor on the other side of the crossing line: the circle goes there.
        let left = tangent_circle_3(&sk, [l[0], l[1], l[2]], DVec2::new(-5.0, 12.0)).unwrap();
        assert!((left.0 - DVec2::new(-15.0, 15.0)).length() < 1e-9);
        for c in [circle, left] {
            assert!(l.iter().all(|&e| touches(&sk, e, c)));
        }
        // Outside the parallel pair there's nothing: say so.
        assert!(tangent_circle_3(&sk, [l[0], l[1], l[2]], DVec2::new(10.0, 40.0)).is_err());
        // Three lines that don't meet each other at all still work (a triangle
        // of extended lines), including the circles outside the triangle.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = lines(&mut sk, &[((0.0, 0.0), (4.0, 0.0)), ((10.0, 10.0), (12.0, 20.0)), ((-10.0, 15.0), (-12.0, 25.0))]);
        for hint in [DVec2::new(0.0, 5.0), DVec2::new(0.0, -5.0)] {
            let c = tangent_circle_3(&sk, [l[0], l[1], l[2]], hint).unwrap();
            assert!(l.iter().all(|&e| touches(&sk, e, c)), "{hint}: {c:?}");
        }
        // All parallel: explained.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = lines(&mut sk, &[((0.0, 0.0), (9.0, 0.0)), ((0.0, 5.0), (9.0, 5.0)), ((0.0, 9.0), (9.0, 9.0))]);
        assert!(tangent_circle_3(&sk, [l[0], l[1], l[2]], DVec2::new(1.0, 2.0)).unwrap_err().contains("parallel"));
    }

    #[test]
    fn two_tangent_circle_is_exact_and_follows_the_cursor() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = lines(&mut sk, &[((0.0, 0.0), (30.0, 0.0)), ((0.0, 0.0), (20.0, 20.0))]);
        for hint in [DVec2::new(10.0, 2.0), DVec2::new(20.0, 3.0), DVec2::new(-5.0, 2.0)] {
            let c = tangent_circle_2(&sk, [l[0], l[1]], hint).unwrap();
            assert!(touches(&sk, l[0], c) && touches(&sk, l[1], c), "{hint}: {c:?}");
        }
        // Further out along the wedge → bigger circle.
        let near = tangent_circle_2(&sk, [l[0], l[1]], DVec2::new(10.0, 2.0)).unwrap().1;
        let far = tangent_circle_2(&sk, [l[0], l[1]], DVec2::new(25.0, 5.0)).unwrap().1;
        assert!(far > near);
        // Parallel lines: the circle fills the gap.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = lines(&mut sk, &[((0.0, 0.0), (30.0, 0.0)), ((0.0, 10.0), (30.0, 10.0))]);
        let c = tangent_circle_2(&sk, [l[0], l[1]], DVec2::new(12.0, 3.0)).unwrap();
        assert!((c.0 - DVec2::new(12.0, 5.0)).length() < 1e-9 && (c.1 - 5.0).abs() < 1e-9, "{c:?}");
        assert!(tangent_circle_2(&sk, [l[0], l[1]], DVec2::new(12.0, 30.0)).is_err());
    }

    fn arc3_editor(sk: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> Editor {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Arc3Point);
        ed.click(sk, free(a.0, a.1));
        ed.click(sk, free(b.0, b.1));
        ed
    }

    #[test]
    fn arc3_near_a_semicircle_snaps_its_centre_to_the_chord_midpoint() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let ed = arc3_editor(&mut sk, (0.0, 0.0), (20.0, 0.0));
        // Third point a bit off the true half circle (which would be (10, 10)).
        let (pt, kind) = ed.arc3_snap(&sk, DVec2::new(10.0, 9.0), None, 2.0);
        assert_eq!(kind, Arc3Snap::HalfCircle);
        let (c, r) = circumcircle(DVec2::new(0.0, 0.0), pt, DVec2::new(20.0, 0.0)).unwrap();
        assert!(c.distance(DVec2::new(10.0, 0.0)) < 1e-9 && (r - 10.0).abs() < 1e-9, "{c:?} {r}");
        // The other side of the chord stays on that side.
        let (pt, _) = ed.arc3_snap(&sk, DVec2::new(10.0, -9.0), None, 2.0);
        assert!((pt - DVec2::new(10.0, -10.0)).length() < 1e-9, "{pt:?}");
    }

    #[test]
    fn arc3_centre_snaps_to_a_grid_point_on_the_bisector() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let ed = arc3_editor(&mut sk, (0.0, 0.0), (20.0, 0.0));
        let grid = GridSpec { step: 5.0, major_every: 5 };
        // Centre on the bisector x = 10 at y = -15 (grid point (10, -15)); aim near it.
        let want = DVec2::new(10.0, -15.0);
        let r = want.distance(DVec2::ZERO);
        let raw = want + DVec2::new(0.3, 1.0).normalize() * (r + 0.2);
        let (pt, kind) = ed.arc3_snap(&sk, raw, Some(grid), 1.5);
        assert_eq!(kind, Arc3Snap::GridCenter(want));
        let (c, _) = circumcircle(DVec2::ZERO, pt, DVec2::new(20.0, 0.0)).unwrap();
        assert!(c.distance(want) < 1e-9, "{c:?}");
        // Idempotent: snapping the snapped point changes nothing.
        let (again, k2) = ed.arc3_snap(&sk, pt, Some(grid), 1.5);
        assert!(again.distance(pt) < 1e-9 && k2 == kind);
        // Grid snap off: no centre snap.
        assert_eq!(ed.arc3_snap(&sk, raw, None, 1.5), (raw, Arc3Snap::None));
    }

    #[test]
    fn arc3_third_point_is_untouched_when_nothing_snaps() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let ed = arc3_editor(&mut sk, (0.0, 0.0), (20.0, 0.0));
        let raw = DVec2::new(7.3, 4.1);
        assert_eq!(ed.arc3_snap(&sk, raw, Some(GridSpec { step: 5.0, major_every: 5 }), 0.5), (raw, Arc3Snap::None));
    }

    #[test]
    fn corner_between_two_lines_finds_their_shared_corner() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        // A chain (the line tool continues from the last end), then a stray line.
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for (x, y) in [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)] {
            ed.click(&mut sk, free(x, y));
        }
        ed.cancel();
        let mut l = vec![0, 1];
        l.extend(lines(&mut sk, &[((20.0, 0.0), (30.0, 0.0))]));
        let p = sk.corner_between(l[0], l[1]).expect("they meet");
        assert_eq!(sk.corner_lines(p).map(|c| (c.0, c.2)), Some((l[0], l[1])));
        assert_eq!(sk.corner_between(l[1], l[0]), Some(p));
        assert_eq!(sk.corner_between(l[0], l[2]), None);
        assert_eq!(sk.corner_between(l[0], l[0]), None);
    }

    #[test]
    fn lines_started_or_ended_on_a_line_are_tied_to_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let base = lines(&mut sk, &[((0.0, 0.0), (40.0, 0.0))])[0];
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::OnLine(base, DVec2::new(15.0, 0.0)));
        let cs = ed.click(&mut sk, free(15.0, 20.0));
        let p = sk.points.len() - 2; // the start point, made on the base line
        assert!(cs.contains(&Constraint::OnLine(p, base)), "{cs:?}");
        sk.constraints.extend(cs); // (the app adds them through the solver)
        assert!(sk.point_uses()[p] >= 2, "shown as a joint");
        // The Point tool: a lone point that stays (and can go on a line too).
        ed.set_tool(Tool::Point);
        assert!(ed.click(&mut sk, free(50.0, 50.0)).is_empty());
        let ties = ed.click(&mut sk, Anchor::OnLine(base, DVec2::new(30.0, 0.0)));
        assert_eq!(ties.len(), 1);
        assert_eq!(sk.entities.iter().filter(|e| matches!(e, Entity::Point { .. })).count(), 2);
        let lone = sk.points.len() - 2;
        sk.delete(&[Sel::Entity(1)]); // deleting something else keeps the lone point
        assert!(sk.points.iter().any(|q| *q == DVec2::new(50.0, 50.0)), "{lone}");
    }

    #[test]
    fn points_placed_on_an_origin_axis_are_locked_to_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::OnAxis(0, DVec2::new(12.0, 0.0)));
        let cs = ed.click(&mut sk, Anchor::OnAxis(1, DVec2::new(0.0, 9.0)));
        assert!(cs.contains(&Constraint::OnAxis(1, 0)) && cs.contains(&Constraint::OnAxis(2, 1)), "{cs:?}");
        sk.constraints.extend(cs);
        assert!(sk.point_uses()[1] >= 2, "shown as locked");
        // Rectangle corners and circle centres lock the same way.
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::OnAxis(0, DVec2::new(30.0, 0.0)));
        let cs = ed.click(&mut sk, free(35.0, 0.0));
        assert_eq!(cs, vec![Constraint::OnAxis(3, 0)]);
    }

    #[test]
    fn arc_tools_make_counter_clockwise_arcs_through_the_clicks() {
        // 3-point: start (10,0), end (0,10), through a point on the short way.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Arc3Point);
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, free(0.0, 10.0));
        let mid = DVec2::from_angle(std::f64::consts::FRAC_PI_4) * 10.0;
        ed.click(&mut sk, Anchor::Free(mid));
        let (c, r, a0, sweep) = sk.arc_geometry(0).unwrap();
        assert!(c.length() < 1e-9 && (r - 10.0).abs() < 1e-9);
        assert!(a0.abs() < 1e-9 && (sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "{a0} {sweep}");
        // Same ends through a point on the far side: the long way round.
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, free(0.0, 10.0));
        ed.click(&mut sk, free(-10.0, 0.0));
        let (_, _, _, sweep) = sk.arc_geometry(1).unwrap();
        assert!((sweep - 1.5 * std::f64::consts::PI).abs() < 1e-9, "{sweep}");
        // Centre point arc: centre, start, direction of the end (shorter way).
        ed.set_tool(Tool::ArcCenter);
        ed.click(&mut sk, free(50.0, 0.0));
        ed.click(&mut sk, free(60.0, 0.0));
        ed.click(&mut sk, free(50.0, -30.0)); // clockwise quarter
        let (c, r, _, sweep) = sk.arc_geometry(2).unwrap();
        assert!((c - DVec2::new(50.0, 0.0)).length() < 1e-9 && (r - 10.0).abs() < 1e-9);
        assert!((sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        // Dimension on an arc is its radius.
        let opts = sk.constraint_options(&[Sel::Entity(2)]);
        assert!(matches!(opts[0].1[0], Constraint::Radius(2, r) if (r - 10.0).abs() < 1e-9));
    }

    #[test]
    fn fillet_needs_a_real_corner_and_room() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for at in [free(0.0, 0.0), free(10.0, 0.0), free(10.0, 10.0)] {
            ed.click(&mut sk, at);
        }
        assert!(sk.fillet_corner(2, 20.0).is_err(), "radius bigger than the lines allow");
        assert!(sk.fillet_corner(1, 2.0).is_err(), "an end with one line isn't a corner");
        let cs = sk.fillet_corner(2, 2.0).unwrap();
        assert!(cs.contains(&Constraint::Radius(3, 2.0)), "{cs:?}");
        assert!(sk.entities.iter().any(|e| matches!(e, Entity::Point { p: 2 })), "the sharp corner stays as a point");
    }

    #[test]
    fn select_tool_does_not_draw() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.click(&mut sk, free(1.0, 1.0));
        ed.click(&mut sk, free(9.0, 9.0));
        assert!(sk.entities.is_empty() && ed.start.is_none());
    }

    #[test]
    fn hit_prefers_points_then_nearest_curve() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, free(30.0, 0.0));
        ed.cancel();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, free(20.0, 20.0));
        ed.click(&mut sk, free(25.0, 20.0));
        assert_eq!(sk.hit(DVec2::new(10.2, 0.1), 1.0), Some(Sel::Point(1)));
        assert_eq!(sk.hit(DVec2::new(20.0, 0.5), 1.0), Some(Sel::Entity(0)));
        assert_eq!(sk.hit(DVec2::new(20.0, 25.3), 1.0), Some(Sel::Entity(1)));
        assert_eq!(sk.hit(DVec2::new(50.0, 50.0), 1.0), None);
    }

    #[test]
    fn selection_toggles_and_clears() {
        let mut ed = Editor::default();
        ed.select_click(Some(Sel::Entity(0)));
        ed.select_click(Some(Sel::Entity(1)));
        ed.select_click(Some(Sel::Entity(0)));
        assert_eq!(ed.selection, vec![Sel::Entity(1)]);
        ed.select_click(None);
        assert!(ed.selection.is_empty());
    }

    fn labels(opts: &[(&'static str, Vec<Constraint>)]) -> Vec<&'static str> {
        opts.iter().map(|(l, _)| *l).collect()
    }

    #[test]
    fn constraint_options_match_selection() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 1.0));
        ed.click(&mut sk, free(11.0, 9.0)); // lines 0, 1
        ed.cancel();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, free(30.0, 30.0));
        ed.click(&mut sk, free(33.0, 34.0)); // circle 2, r = 5

        let one = sk.constraint_options(&[Sel::Entity(0)]);
        assert_eq!(labels(&one), vec!["Dimension", "Horizontal", "Vertical"]);
        assert!(matches!(one[0].1[0], Constraint::Length(0, l) if (l - 101f64.sqrt()).abs() < 1e-9));

        let two = sk.constraint_options(&[Sel::Entity(0), Sel::Entity(1)]);
        assert_eq!(labels(&two), vec!["Dimension", "Parallel", "Perpendicular", "Equal", "Horizontal", "Vertical"]);
        assert!(matches!(two[0].1[0], Constraint::Angle(0, 1, _)), "dimensioning two lines gives their angle");
        assert_eq!(two[4].1, vec![Constraint::Horizontal(0), Constraint::Horizontal(1)]);

        let circ = sk.constraint_options(&[Sel::Entity(2)]);
        assert_eq!(circ[0].1, vec![Constraint::Diameter(2, 10.0)]);

        let tan = sk.constraint_options(&[Sel::Entity(1), Sel::Entity(2)]);
        assert_eq!(labels(&tan), vec!["Tangent"]);

        let pts = sk.constraint_options(&[Sel::Point(0), Sel::Point(2)]);
        assert_eq!(labels(&pts), vec!["Dimension", "Fix"], "two points: their distance, or fix them");
        assert!(matches!(pts[0].1[0], Constraint::Distance(0, 2, _)));
        assert_eq!(pts[1].1, vec![Constraint::Fix(2)], "origin is already fixed");

        // Existing constraints are not offered again.
        sk.constraints.push(Constraint::Horizontal(0));
        sk.constraints.push(Constraint::Length(0, 3.0));
        assert_eq!(labels(&sk.constraint_options(&[Sel::Entity(0)])), vec!["Vertical"]);
        sk.constraints.push(Constraint::Parallel(1, 0));
        assert!(!labels(&sk.constraint_options(&[Sel::Entity(0), Sel::Entity(1)])).contains(&"Parallel"));
    }

    #[test]
    fn delete_renumbers_everything() {
        // Two separate lines + a circle, with constraints on each.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 10.0));
        ed.click(&mut sk, free(10.0, 10.0)); // line 0: points 1,2
        ed.cancel();
        ed.click(&mut sk, free(0.0, 20.0));
        ed.click(&mut sk, free(10.0, 25.0)); // line 1: points 3,4
        ed.cancel();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, free(50.0, 50.0));
        ed.click(&mut sk, free(55.0, 50.0)); // circle 2: point 5
        sk.constraints = vec![
            Constraint::Horizontal(0),
            Constraint::Length(1, 11.0),
            Constraint::Tangent(1, 2),
            Constraint::Fix(5),
        ];
        sk.delete(&[Sel::Entity(0)]);
        assert_eq!(sk.entities.len(), 2);
        assert_eq!(sk.points.len(), 4, "origin + line's 2 points + circle centre");
        assert_eq!(sk.entities[0], Entity::Line { a: 1, b: 2 });
        assert!(matches!(sk.entities[1], Entity::Circle { center: 3, .. }));
        assert_eq!(
            sk.constraints,
            vec![Constraint::Length(0, 11.0), Constraint::Tangent(0, 1), Constraint::Fix(3)]
        );
    }

    #[test]
    fn deleting_a_point_removes_its_lines() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = line_tool();
        ed.click(&mut sk, free(0.0, 10.0));
        ed.click(&mut sk, free(10.0, 10.0));
        ed.click(&mut sk, free(10.0, 0.0)); // lines 0 (1-2) and 1 (2-3)
        ed.cancel();
        sk.delete(&[Sel::Point(3)]);
        assert_eq!(sk.entities, vec![Entity::Line { a: 1, b: 2 }]);
        assert_eq!(sk.points.len(), 3);
        sk.delete(&[Sel::Point(Sketch::ORIGIN)]);
        assert_eq!(sk.points.len(), 3, "origin cannot be deleted");
    }
}
