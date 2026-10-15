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

    /// `for_distance`, in steps that suit the design's units: inch designs
    /// get 1/16, 1/8, 1/4, 1/2, 1, 2, 6, 12 in… with a major line every inch
    /// (or foot); metric ones keep 1-2-5 steps.
    pub fn for_units(distance: f32, unit: crate::units::Unit) -> Self {
        use crate::units::Unit;
        if !matches!(unit, Unit::In | Unit::Ft) {
            return Self::for_distance(distance);
        }
        let raw_in = (distance as f64 / 30.0).max(1e-4) / 25.4;
        const STEPS: [f64; 14] = [1.0 / 64.0, 1.0 / 32.0, 1.0 / 16.0, 0.125, 0.25, 0.5, 1.0, 2.0, 6.0, 12.0, 24.0, 60.0, 120.0, 240.0];
        let step = STEPS.iter().copied().find(|&s| s >= raw_in - 1e-12).unwrap_or(240.0);
        let major_every = match step {
            s if s < 1.0 => (1.0 / s).round() as i64, // a major line every inch
            s if s < 12.0 => (12.0 / s).round() as i64, // …every foot
            _ => 5,
        };
        Self { step: step * 25.4, major_every }
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
    /// A fit-point spline: the smooth curve through `Sketch::splines[s]`
    /// (ordinary sketch points, so they drag and take constraints).
    Spline { s: usize },
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
    /// An offset copy (second) stays a set distance from its original
    /// (first): a line that far from the original's line, or an arc/circle
    /// sharing the original's centre with its radius that much bigger or
    /// smaller. The Offset tool adds one per piece, all the same distance.
    /// The sign keeps the side: left of the original line's a→b (+) or right
    /// (−); a bigger radius (+) or smaller (−). Shown and edited as the size.
    Offset(EntityId, EntityId, f64),
    /// Point locked where it is.
    Fix(PointId),
    /// Two points in the same place (Fusion's Coincident on two points).
    Coincident(PointId, PointId),
    /// Two lines on one straight line.
    Collinear(EntityId, EntityId),
    /// Two circles / arcs round one centre.
    Concentric(EntityId, EntityId),
}

impl Constraint {
    /// A dimension's value as shown and edited (an offset's size, not its side).
    pub fn value(&self) -> Option<f64> {
        use Constraint::*;
        match *self {
            Length(_, d) | Diameter(_, d) | Angle(_, _, d) | Distance(_, _, d) | Radius(_, d) => Some(d),
            Offset(_, _, d) => Some(d.abs()),
            _ => None,
        }
    }

    /// Set a dimension's value (an offset keeps its side).
    pub fn set_value(&mut self, v: f64) {
        use Constraint::*;
        match self {
            Length(_, d) | Diameter(_, d) | Angle(_, _, d) | Distance(_, _, d) | Radius(_, d) => *d = v,
            Offset(_, _, d) => *d = v.abs().copysign(*d),
            _ => {}
        }
    }

    pub fn entities(&self) -> Vec<EntityId> {
        use Constraint::*;
        match *self {
            Horizontal(e) | Vertical(e) | Length(e, _) | Diameter(e, _) => vec![e],
            Parallel(a, b) | Perpendicular(a, b) | Equal(a, b) | Tangent(a, b) | Angle(a, b, _) | Offset(a, b, _) | Collinear(a, b) | Concentric(a, b) => vec![a, b],
            OnCircle(_, c) | OnLine(_, c) | Radius(c, _) | ArcEnds(c) => vec![c],
            Fix(_) | Distance(..) | Midpoint(..) | OnAxis(..) | Coincident(..) => vec![],
        }
    }

    /// The points it ties directly (not through an entity).
    pub fn points(&self) -> Vec<PointId> {
        match *self {
            Constraint::Fix(p) => vec![p],
            Constraint::Distance(p, q, _) | Constraint::Coincident(p, q) => vec![p, q],
            Constraint::OnCircle(p, _) | Constraint::OnLine(p, _) => vec![p],
            Constraint::Midpoint(p, a, b) => vec![p, a, b],
            Constraint::OnAxis(p, _) => vec![p],
            _ => vec![],
        }
    }

    /// Its full name (for the tooltip on a glyph).
    pub fn name(&self) -> &'static str {
        use Constraint::*;
        match self {
            Horizontal(_) => "Horizontal",
            Vertical(_) => "Vertical",
            Length(..) | Diameter(..) | Radius(..) | Angle(..) | Distance(..) => "Dimension",
            Parallel(..) => "Parallel",
            Perpendicular(..) => "Perpendicular",
            Equal(..) => "Equal",
            Tangent(..) => "Tangent",
            Offset(..) => "Offset",
            Coincident(..) | OnCircle(..) | OnLine(..) | OnAxis(..) => "Coincident",
            Collinear(..) => "Collinear",
            Concentric(..) => "Concentric",
            Midpoint(..) => "Midpoint",
            ArcEnds(..) => "",
            Fix(_) => "Fix",
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
            Offset(..) => "Offset",
            Coincident(..) => "Coin",
            Collinear(..) => "Coll",
            Concentric(..) => "Conc",
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
    /// Set when the sketch is on a construction plane (index into the design's
    /// planes): the kernel puts it wherever that plane is now. `face` keeps
    /// the plane as it was when the sketch was made.
    #[serde(default)]
    pub plane_ref: Option<usize>,
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
    /// Each spline's fit points, in order (see `Entity::Spline`). Older files have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splines: Vec<Vec<PointId>>,
    /// A unit a dimension was typed in that differs from the design's ("1 in"
    /// in a mm design), by constraint index: it keeps showing in that unit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dim_units: Vec<Option<crate::units::Unit>>,
    /// A dimension typed as an expression ("width * 2"), by constraint
    /// index: it follows the design's parameters when they change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dim_exprs: Vec<Option<String>>,
    /// Entities that are construction geometry (by entity index): drawn
    /// dashed, ignored when finding regions to extrude, still constrainable.
    /// Missing entries mean normal geometry.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub construction: Vec<bool>,
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

/// A regular polygon: its centre, the radius of the circle through its
/// corners, and the corners in order.
#[derive(Clone, Debug)]
pub struct PolygonShape {
    pub center: DVec2,
    pub radius: f64,
    pub verts: Vec<DVec2>,
}

/// A fillet / chamfer shown before it is made.
#[derive(Clone, Copy, Debug)]
pub struct CornerCut {
    /// Where the cut meets each line.
    pub a: DVec2,
    pub b: DVec2,
    /// A fillet's arc: (centre, radius, start angle, sweep), counter-clockwise.
    pub arc: Option<(DVec2, f64, f64, f64)>,
}

/// "5" → (5, None); "2, 5" or "2 5" → (2, Some(5)), in mm. For the size box
/// of chamfers, which can take a different distance along each side. Bare
/// numbers are in `unit` (the design's); each can carry its own ("0.1 in, 3 mm").
pub fn parse_sizes(text: &str, unit: crate::units::Unit) -> Option<(f64, Option<f64>)> {
    use crate::units::parse_length;
    // One value, maybe with a unit ("0.5 in", pasted from Measure).
    if let Some(v) = parse_length(text, unit) {
        return Some((v, None));
    }
    let parts: Vec<&str> = if text.contains([',', ';']) {
        text.split([',', ';']).map(str::trim).collect()
    } else {
        text.split_whitespace().collect()
    };
    match parts[..] {
        [a, b] => Some((parse_length(a, unit)?, Some(parse_length(b, unit)?))),
        _ => None,
    }
}

/// What the 3-Point Arc's third click snapped to (see `Editor::arc3_snap`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Arc3Snap {
    None,
    HalfCircle,
    GridCenter(DVec2),
}

/// Nearest and farthest distance from `c` to the rectangle `lo`–`hi`
/// (nearest is 0 when `c` is inside it).
fn rect_distances(c: DVec2, lo: DVec2, hi: DVec2) -> (f64, f64) {
    let nearest = c.clamp(lo, hi);
    let far = DVec2::new((c.x - lo.x).abs().max((c.x - hi.x).abs()), (c.y - lo.y).abs().max((c.y - hi.y).abs()));
    (c.distance(nearest), far.length())
}

/// Does the segment `p`–`q` touch the rectangle `lo`–`hi` (Liang–Barsky)?
fn segment_touches_rect(p: DVec2, q: DVec2, lo: DVec2, hi: DVec2) -> bool {
    let d = q - p;
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (dir, from, min, max) in [(d.x, p.x, lo.x, hi.x), (d.y, p.y, lo.y, hi.y)] {
        if dir.abs() < 1e-12 {
            if from < min || from > max {
                return false;
            }
        } else {
            let (mut u0, mut u1) = ((min - from) / dir, (max - from) / dir);
            if u0 > u1 {
                std::mem::swap(&mut u0, &mut u1);
            }
            t0 = t0.max(u0);
            t1 = t1.min(u1);
            if t0 > t1 {
                return false;
            }
        }
    }
    true
}

/// What a Trim click would remove: a stretch of line, or of a circle / arc
/// (centre, radius, start angle, sweep).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrimPiece {
    Line(DVec2, DVec2),
    Arc(DVec2, f64, f64, f64),
}

/// Where two circles cross (none when they don't, or only touch).
pub(crate) fn circle_circle(c1: DVec2, r1: f64, c2: DVec2, r2: f64) -> Vec<DVec2> {
    let d = c1.distance(c2);
    if d < 1e-12 || d > r1 + r2 || d < (r1 - r2).abs() {
        return Vec::new();
    }
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h2 = r1 * r1 - a * a;
    if h2 <= 1e-12 * r1 * r1 {
        return Vec::new(); // touching: not a crossing
    }
    let u = (c2 - c1) / d;
    let m = c1 + u * a;
    let h = h2.sqrt();
    vec![m + u.perp() * h, m - u.perp() * h]
}

/// Parameters t where `p + t·d` crosses the circle (centre `c`, radius `r`).
pub(crate) fn line_circle(p: DVec2, d: DVec2, c: DVec2, r: f64) -> Vec<f64> {
    let f = p - c;
    let (qa, qb, qc) = (d.length_squared(), 2.0 * f.dot(d), f.length_squared() - r * r);
    let disc = qb * qb - 4.0 * qa * qc;
    if qa < 1e-18 || disc < 0.0 {
        return Vec::new();
    }
    let s = disc.sqrt();
    if s < 1e-9 * qb.abs().max(1e-12) {
        return Vec::new(); // grazing: not a crossing
    }
    vec![(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)]
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
    /// The sketch's X (0) or Y (1) axis, for constraints.
    Axis(u8),
}

/// `at` turned onto the nearest quarter turn (0°, 90°, 180°, 270°) around
/// `center` when it's within `tol_deg` of one, at the same distance.
pub fn snap_quarter_turn(center: DVec2, at: DVec2, tol_deg: f64) -> DVec2 {
    let v = at - center;
    let r = v.length();
    if r < 1e-12 {
        return at;
    }
    let deg = v.y.atan2(v.x).to_degrees();
    let nearest = (deg / 90.0).round() * 90.0;
    if (deg - nearest).abs() <= tol_deg { center + DVec2::from_angle(nearest.to_radians()) * r } else { at }
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
            plane_ref: None,
            points: vec![DVec2::ZERO],
            entities: Vec::new(),
            constraints: Vec::new(),
            label_shift: Vec::new(),
            driven: Vec::new(),
            dim_units: Vec::new(),
            dim_exprs: Vec::new(),
            splines: Vec::new(),
            construction: Vec::new(),
        }
    }

    /// A sketch on a body face.
    pub fn on_face(name: String, face: FacePlane) -> Self {
        Self { face: Some(face), ..Self::new(name, OriginPlane::XY) }
    }

    /// A sketch on construction plane `k`, which is now at `frame`.
    pub fn on_plane(name: String, frame: FacePlane, k: usize) -> Self {
        Self { face: Some(FacePlane { follow: None, ..frame }), plane_ref: Some(k), ..Self::new(name, OriginPlane::XY) }
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
            Sel::Axis(k) => k < 2,
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

    /// The points an entity is made of.
    pub fn entity_points(&self, e: EntityId) -> Vec<PointId> {
        match self.entities.get(e) {
            Some(&Entity::Line { a, b }) => vec![a, b],
            Some(&Entity::Circle { center, .. }) => vec![center],
            Some(&Entity::Arc { center, a, b }) => vec![center, a, b],
            Some(&Entity::Point { p }) => vec![p],
            Some(&Entity::Spline { s }) => self.splines.get(s).cloned().unwrap_or_default(),
            None => Vec::new(),
        }
    }

    /// Where a spline's fit points are now.
    pub fn spline_fit(&self, s: usize) -> Vec<DVec2> {
        self.splines.get(s).map_or(Vec::new(), |l| l.iter().map(|&p| self.points[p]).collect())
    }

    /// Points along spline `s` (see `spline::polyline`).
    pub fn spline_curve(&self, s: usize) -> Vec<DVec2> {
        crate::spline::polyline(&self.spline_fit(s))
    }

    /// Geometry brought in by Project: construction, and locked where it lies.
    pub fn is_projected(&self, e: EntityId) -> bool {
        let pts: Vec<PointId> = match self.entities.get(e) {
            Some(&Entity::Line { a, b }) => vec![a, b],
            Some(&Entity::Circle { center, .. }) => vec![center],
            Some(&Entity::Arc { center, a, b }) => vec![center, a, b],
            _ => return false,
        };
        self.is_construction(e) && pts.iter().all(|&p| p != Self::ORIGIN && self.constraints.contains(&Constraint::Fix(p)))
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
                    Entity::Spline { s } => crate::spline::distance(&self.spline_fit(s), p),
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
            // Last, the sketch's own axes (through its origin).
            .or_else(|| {
                let (dx, dy) = (p.y.abs(), p.x.abs());
                (dx.min(dy) <= tol).then_some(Sel::Axis(if dx <= dy { 0 } else { 1 }))
            })
    }

    /// Drag-select: what the rectangle `a`–`b` takes in. `crossing` (dragging
    /// right to left, as in Fusion) takes anything the rectangle touches;
    /// otherwise only what lies completely inside it. Free-standing points
    /// come along; a line's end points don't (deleting one would take the
    /// whole line with it).
    pub fn select_in_rect(&self, a: DVec2, b: DVec2, crossing: bool) -> Vec<Sel> {
        let (lo, hi) = (a.min(b), a.max(b));
        let inside = |p: DVec2| p.x >= lo.x && p.x <= hi.x && p.y >= lo.y && p.y <= hi.y;
        let mut out = Vec::new();
        for (e, ent) in self.entities.iter().enumerate() {
            let hit = match *ent {
                Entity::Point { p } => {
                    if inside(self.points[p]) {
                        out.push(Sel::Point(p));
                    }
                    continue;
                }
                Entity::Line { a, b } => {
                    let (p, q) = (self.points[a], self.points[b]);
                    if crossing { segment_touches_rect(p, q, lo, hi) } else { inside(p) && inside(q) }
                }
                Entity::Circle { center, radius } => {
                    let c = self.points[center];
                    let (near, far) = rect_distances(c, lo, hi);
                    if crossing {
                        // Touches the rectangle: the rim comes within reach, or lies inside it.
                        near <= radius && (far >= radius || inside(c + DVec2::X * radius))
                    } else {
                        inside(c - DVec2::splat(radius)) && inside(c + DVec2::splat(radius))
                    }
                }
                Entity::Arc { .. } => {
                    let Some((c, r, a0, sweep)) = self.arc_geometry(e) else { continue };
                    let pts: Vec<DVec2> = (0..=48).map(|i| c + DVec2::from_angle(a0 + sweep * i as f64 / 48.0) * r).collect();
                    if crossing {
                        pts.windows(2).any(|w| segment_touches_rect(w[0], w[1], lo, hi))
                    } else {
                        pts.iter().all(|&p| inside(p))
                    }
                }
                Entity::Spline { s } => {
                    let pts = self.spline_curve(s);
                    if crossing {
                        pts.windows(2).any(|w| segment_touches_rect(w[0], w[1], lo, hi))
                    } else {
                        pts.iter().all(|&p| inside(p))
                    }
                }
            };
            if hit {
                out.push(Sel::Entity(e));
            }
        }
        out
    }

    /// Everything joined to `start` through shared points: the lines, arcs and
    /// circles of its chain and their points (the origin excepted). Double-click
    /// selects this, like Fusion.
    pub fn connected(&self, start: Sel) -> Vec<Sel> {
        let ends = |e: &Entity| -> Vec<PointId> {
            match *e {
                Entity::Line { a, b } => vec![a, b],
                Entity::Circle { center, .. } => vec![center],
                Entity::Point { p } => vec![p],
                Entity::Arc { center, a, b } => vec![center, a, b],
                Entity::Spline { s } => self.splines.get(s).cloned().unwrap_or_default(),
            }
        };
        let mut ents: Vec<EntityId> = Vec::new();
        let mut pts: Vec<PointId> = Vec::new();
        match start {
            Sel::Entity(e) if e < self.entities.len() => ents.push(e),
            Sel::Point(p) if p < self.points.len() => pts.push(p),
            _ => return Vec::new(),
        }
        // Grow until nothing new is reached. (The origin joins nothing: lots of
        // unrelated geometry is tied to it.)
        loop {
            let before = (ents.len(), pts.len());
            for &e in &ents.clone() {
                for p in ends(&self.entities[e]) {
                    if !pts.contains(&p) {
                        pts.push(p);
                    }
                }
            }
            for (e, ent) in self.entities.iter().enumerate() {
                if !ents.contains(&e) && ends(ent).iter().any(|p| *p != Self::ORIGIN && pts.contains(p)) {
                    ents.push(e);
                }
            }
            if (ents.len(), pts.len()) == before {
                break;
            }
        }
        ents.sort_unstable();
        pts.sort_unstable();
        ents.into_iter().map(Sel::Entity).chain(pts.into_iter().filter(|&p| p != Self::ORIGIN).map(Sel::Point)).collect()
    }

    /// Every point lying inside the box, line ends included (the origin
    /// excepted: it can't be moved or deleted, so selecting it is no use).
    pub fn points_in_rect(&self, a: DVec2, b: DVec2) -> Vec<Sel> {
        let (lo, hi) = (a.min(b), a.max(b));
        (0..self.points.len())
            .filter(|&p| p != Self::ORIGIN)
            .filter(|&p| (lo.x..=hi.x).contains(&self.points[p].x) && (lo.y..=hi.y).contains(&self.points[p].y))
            .map(Sel::Point)
            .collect()
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
        // A spline loses a deleted fit point (and goes when fewer than two are left).
        for list in &mut self.splines {
            list.retain(|&p| !dead_pts[p]);
        }
        for (i, e) in self.entities.iter().enumerate() {
            let uses_dead = match *e {
                Entity::Line { a, b } => dead_pts[a] || dead_pts[b],
                Entity::Circle { center, .. } => dead_pts[center],
                Entity::Point { p } => dead_pts[p],
                Entity::Arc { center, a, b } => dead_pts[center] || dead_pts[a] || dead_pts[b],
                Entity::Spline { s } => self.splines.get(s).is_none_or(|l| l.len() < 2),
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
        let old_construction = std::mem::take(&mut self.construction);
        self.construction = (0..dead_ents.len())
            .filter(|&i| !dead_ents[i])
            .map(|i| old_construction.get(i).copied().unwrap_or(false))
            .collect();
        while self.construction.last() == Some(&false) {
            self.construction.pop();
        }

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
                Entity::Spline { s } => {
                    for &p in self.splines.get(s).into_iter().flatten() {
                        used[p] = true;
                    }
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
                Entity::Spline { .. } => {}
            }
        }
        // Splines: keep the lists still used (renumbered), their points renumbered.
        let mut spline_map = vec![usize::MAX; self.splines.len()];
        let mut lists = Vec::new();
        for e in &mut self.entities {
            if let Entity::Spline { s } = e {
                if let Some(list) = self.splines.get(*s) {
                    spline_map[*s] = lists.len();
                    lists.push(list.iter().map(|&p| pt_map[p]).collect::<Vec<_>>());
                    *s = spline_map[*s];
                }
            }
        }
        self.splines = lists;
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
                Offset(a, b, d) => Offset(ent_map[a], ent_map[b], d),
                Coincident(p, q) => Coincident(pt_map[p], pt_map[q]),
                Collinear(a, b) => Collinear(ent_map[a], ent_map[b]),
                Concentric(a, b) => Concentric(ent_map[a], ent_map[b]),
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

// ---------- Inference (lining up with existing geometry) ----------

impl Sketch {
    /// Fusion's inference while drawing: `p` lines up vertically or
    /// horizontally (within `tol`) with the middle of a line or the centre
    /// of a circle / arc. Returns `p` moved onto the line-up(s), and the
    /// points it lines up with: (straight above/below, straight beside).
    ///
    /// Only from inside: a line's middle counts when `p` is inside a closed
    /// shape that line is part of, a centre when `p` is inside its circle.
    pub fn line_up(&self, p: DVec2, tol: f64) -> (DVec2, Option<DVec2>, Option<DVec2>) {
        let around: Vec<crate::profile::Region> = crate::profile::find_regions(self).into_iter().filter(|r| r.contains(p)).collect();
        let on_outline = |m: DVec2| {
            around.iter().any(|r| {
                r.loops().iter().any(|l| (0..l.len()).any(|k| dist_to_segment(m, l[k], l[(k + 1) % l.len()]) < 1e-6))
            })
        };
        let refs: Vec<DVec2> = self
            .entities
            .iter()
            .enumerate()
            .filter(|&(e, _)| !self.is_construction(e))
            .filter_map(|(e, ent)| match *ent {
                Entity::Line { a, b } => Some(self.points[a].midpoint(self.points[b])).filter(|&m| on_outline(m)),
                Entity::Circle { center, radius } => Some(self.points[center]).filter(|c| c.distance(p) < radius),
                Entity::Arc { center, .. } => self.arc_geometry(e).and_then(|(_, r, _, _)| Some(self.points[center]).filter(|c| c.distance(p) < r)),
                Entity::Point { .. } | Entity::Spline { .. } => None,
            })
            .filter(|r| r.distance(p) > tol) // not when already on it
            .collect();
        let best = |d: &dyn Fn(DVec2) -> f64| refs.iter().copied().filter(|&r| d(r) <= tol).min_by(|a, b| d(*a).total_cmp(&d(*b)));
        let above = best(&|r: DVec2| (r.x - p.x).abs());
        let beside = best(&|r: DVec2| (r.y - p.y).abs());
        let q = DVec2::new(above.map_or(p.x, |r| r.x), beside.map_or(p.y, |r| r.y));
        (q, above, beside)
    }
}

// ---------- Perpendicular guides from line middles ----------

impl Sketch {
    /// The line whose middle `p` is on (within `tol`), for acquiring it.
    pub fn middle_at(&self, p: DVec2, tol: f64) -> Option<EntityId> {
        self.entities
            .iter()
            .enumerate()
            .filter_map(|(e, ent)| match *ent {
                Entity::Line { a, b } => Some((e, self.points[a].midpoint(self.points[b]).distance(p))),
                _ => None,
            })
            .filter(|&(_, d)| d <= tol)
            .min_by(|x, y| x.1.total_cmp(&y.1))
            .map(|(e, _)| e)
    }

    /// The guide through line `e`'s middle, square to the line: (middle, direction).
    pub fn middle_guide(&self, e: EntityId) -> Option<(DVec2, DVec2)> {
        let Some(&Entity::Line { a, b }) = self.entities.get(e) else { return None };
        let (a, b) = (self.points[a], self.points[b]);
        let dir = (b - a).try_normalize()?;
        Some((a.midpoint(b), dir.perp()))
    }

    /// Fusion's inference from line middles the cursor rested on: `p` snapped
    /// onto the perpendicular through an acquired middle when within `tol`
    /// of it — onto where two of them cross when near both. Returns the
    /// point and the middles it lines up with; None when no guide is near.
    pub fn on_middle_guides(&self, acquired: &[EntityId], p: DVec2, tol: f64) -> Option<(DVec2, Vec<DVec2>)> {
        let guides: Vec<(DVec2, DVec2)> = acquired.iter().filter_map(|&e| self.middle_guide(e)).collect();
        let off = |(m, d): (DVec2, DVec2)| (p - m).perp_dot(d).abs();
        // Two guides crossing near the cursor: right onto the crossing.
        for (i, &(m1, d1)) in guides.iter().enumerate() {
            for &(m2, d2) in &guides[i + 1..] {
                let den = d1.perp_dot(d2);
                if den.abs() < 1e-9 {
                    continue;
                }
                let x = m1 + d1 * ((m2 - m1).perp_dot(d2) / den);
                if x.distance(p) <= tol * 1.5 {
                    return Some((x, vec![m1, m2]));
                }
            }
        }
        let (m, d) = guides.into_iter().filter(|&g| off(g) <= tol && g.0.distance(p) > tol).min_by(|a, b| off(*a).total_cmp(&off(*b)))?;
        Some((m + d * (p - m).dot(d), vec![m]))
    }
}

// ---------- Selection details ----------

/// What a figure in the selection details measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Length,
    Area,
}

impl Sketch {
    /// How far `p` is from entity `e` (its curve; a point entity by its point).
    fn distance_to(&self, e: EntityId, p: DVec2) -> f64 {
        match *self.entities.get(e).expect("valid entity") {
            Entity::Line { a, b } => dist_to_segment(p, self.points[a], self.points[b]),
            Entity::Circle { center, radius } => (p.distance(self.points[center]) - radius).abs(),
            Entity::Point { p: q } => p.distance(self.points[q]),
            Entity::Spline { s } => crate::spline::distance(&self.spline_fit(s), p),
            Entity::Arc { .. } => {
                let Some((c, r, a0, sweep)) = self.arc_geometry(e) else { return f64::INFINITY };
                let ang = (p - c).y.atan2((p - c).x);
                if (ang - a0).rem_euclid(std::f64::consts::TAU) <= sweep + 1e-9 {
                    (p.distance(c) - r).abs()
                } else {
                    let ends = [c + DVec2::from_angle(a0) * r, c + DVec2::from_angle(a0 + sweep) * r];
                    ends[0].distance(p).min(ends[1].distance(p))
                }
            }
        }
    }

    /// Figures about a selection for the corner readout (mm, mm²): a line's
    /// length; a circle's diameter, circumference and area; an arc's radius
    /// and length; several curves' total length, and — when they close a
    /// shape — its area and perimeter; a point's position or two points' distance.
    pub fn selection_info(&self, sel: &[Sel]) -> Vec<(&'static str, f64, Amount)> {
        use Amount::*;
        let ents: Vec<EntityId> = sel.iter().filter_map(|s| if let Sel::Entity(e) = *s { Some(e) } else { None }).filter(|&e| e < self.entities.len()).collect();
        let pts: Vec<PointId> = sel.iter().filter_map(|s| if let Sel::Point(p) = *s { Some(p) } else { None }).filter(|&p| p < self.points.len()).collect();
        let length = |e: EntityId| match self.entities[e] {
            Entity::Line { a, b } => self.points[a].distance(self.points[b]),
            Entity::Circle { radius, .. } => std::f64::consts::TAU * radius,
            Entity::Arc { .. } => self.arc_geometry(e).map_or(0.0, |(_, r, _, sweep)| r * sweep),
            Entity::Point { .. } => 0.0,
            Entity::Spline { s } => self.spline_curve(s).windows(2).map(|w| w[0].distance(w[1])).sum(),
        };
        let mut out = Vec::new();
        match (ents.as_slice(), pts.as_slice()) {
            ([], [p]) => {
                let q = self.points[*p];
                out.extend([("X", q.x, Length), ("Y", q.y, Length)]);
            }
            ([], [p, q]) => out.push(("Distance", self.points[*p].distance(self.points[*q]), Length)),
            ([e], _) => match self.entities[*e] {
                Entity::Line { .. } => out.push(("Length", length(*e), Length)),
                Entity::Circle { radius, .. } => out.extend([
                    ("Diameter", 2.0 * radius, Length),
                    ("Circumference", length(*e), Length),
                    ("Area", std::f64::consts::PI * radius * radius, Area),
                ]),
                Entity::Arc { .. } => {
                    if let Some((_, r, _, _)) = self.arc_geometry(*e) {
                        out.extend([("Radius", r, Length), ("Length", length(*e), Length)]);
                    }
                }
                Entity::Spline { .. } => out.push(("Length", length(*e), Length)),
                Entity::Point { p } => {
                    let q = self.points[p];
                    out.extend([("X", q.x, Length), ("Y", q.y, Length)]);
                }
            },
            (many, _) if many.len() > 1 => {
                out.push(("Total length", many.iter().map(|&e| length(e)).sum(), Length));
                // A closed shape: a profile whose outline lies all on the selection.
                let tol = 1e-6;
                let closed = crate::profile::find_regions(self)
                    .into_iter()
                    .map(|r| crate::profile::Region { holes: Vec::new(), hole_curves: Vec::new(), ..r })
                    .filter(|r| {
                        // The middle of every edge of the outline (on the arc, for arcs).
                        let n = r.outer.len();
                        (0..n).all(|i| {
                            let mid = (r.outer[i] + r.outer[(i + 1) % n]) * 0.5;
                            let mid = match r.outer_curves.get(i) {
                                Some(&crate::profile::Curve::Arc { center, radius }) => center + (mid - center).normalize_or_zero() * radius,
                                _ => mid,
                            };
                            many.iter().any(|&e| self.distance_to(e, mid) < tol)
                        })
                    })
                    .max_by(|a, b| a.exact_area().total_cmp(&b.exact_area()));
                if let Some(r) = closed {
                    out.extend([("Area", r.exact_area(), Area), ("Perimeter", r.loop_length(), Length)]);
                }
            }
            _ => {}
        }
        out
    }
}

// ---------- Constraint actions ----------

/// Constraint buttons, in toolbar order.
pub const CONSTRAINT_ACTIONS: [&str; 12] = [
    "Dimension", "Coincident", "Collinear", "Concentric", "Horizontal", "Vertical", "Parallel", "Perpendicular", "Equal", "Tangent", "Midpoint", "Fix",
];

impl Sketch {
    /// Keep the constraints `keep` says yes to (and their label positions
    /// and driven flags).
    pub fn retain_constraints(&mut self, mut keep: impl FnMut(&Constraint) -> bool) {
        let shifts = std::mem::take(&mut self.label_shift);
        let driven = std::mem::take(&mut self.driven);
        let units = std::mem::take(&mut self.dim_units);
        let exprs = std::mem::take(&mut self.dim_exprs);
        let (mut kept, mut kept_driven, mut kept_units, mut kept_exprs) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut i = 0;
        self.constraints.retain(|c| {
            let k = keep(c);
            if k {
                kept.push(shifts.get(i).copied().unwrap_or(DVec2::ZERO));
                kept_driven.push(driven.get(i).copied().unwrap_or(false));
                kept_units.push(units.get(i).copied().flatten());
                kept_exprs.push(exprs.get(i).cloned().flatten());
            }
            i += 1;
            k
        });
        while kept_units.last() == Some(&None) {
            kept_units.pop();
        }
        self.dim_units = kept_units;
        while kept_exprs.last() == Some(&None) {
            kept_exprs.pop();
        }
        self.dim_exprs = kept_exprs;
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
    #[cfg(test)]
    pub fn remove_constraint(&mut self, ci: usize) {
        if ci < self.constraints.len() {
            self.constraints.remove(ci);
            if ci < self.label_shift.len() {
                self.label_shift.remove(ci);
            }
            if ci < self.dim_units.len() {
                self.dim_units.remove(ci);
            }
            if ci < self.dim_exprs.len() {
                self.dim_exprs.remove(ci);
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

    pub fn is_construction(&self, e: EntityId) -> bool {
        self.construction.get(e).copied().unwrap_or(false)
    }

    pub fn set_construction(&mut self, e: EntityId, on: bool) {
        if self.construction.len() <= e {
            if !on {
                return;
            }
            self.construction.resize(e + 1, false);
        }
        self.construction[e] = on;
        while self.construction.last() == Some(&false) {
            self.construction.pop();
        }
    }

    /// Fusion's X on a selection: if everything selected is construction make
    /// it normal again, otherwise make it all construction. Points selected
    /// alone have no such state; the entities of a selected point are left alone.
    pub fn toggle_construction(&mut self, sel: &[Sel]) {
        let ents: Vec<EntityId> = sel
            .iter()
            .filter_map(|s| match *s {
                Sel::Entity(e) if e < self.entities.len() && !matches!(self.entities[e], Entity::Point { .. }) => Some(e),
                _ => None,
            })
            .collect();
        let all = !ents.is_empty() && ents.iter().all(|&e| self.is_construction(e));
        for e in ents {
            self.set_construction(e, !all);
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
            Constraint::Offset(a, b, _) => self.offset_between(a, b),
            _ => None,
        }
    }

    /// How far an offset copy `b` is from `a` now: from the middle of a copied
    /// line to the original's line, or the difference in radius.
    pub fn offset_between(&self, a: EntityId, b: EntityId) -> Option<f64> {
        self.signed_offset(a, b).map(f64::abs)
    }

    /// `offset_between` with its side (see `Constraint::Offset`).
    pub fn signed_offset(&self, a: EntityId, b: EntityId) -> Option<f64> {
        match (self.entities.get(a)?, self.entities.get(b)?) {
            (&Entity::Line { a: p, b: q }, &Entity::Line { a: r, b: s }) => {
                let (p, q) = (self.points[p], self.points[q]);
                let d = q - p;
                let mid = (self.points[r] + self.points[s]) * 0.5;
                Some(d.perp_dot(mid - p) / d.length().max(1e-12))
            }
            _ => Some(self.round_radius(b)? - self.round_radius(a)?),
        }
    }

    /// A circle's or an arc's radius.
    pub fn round_radius(&self, e: EntityId) -> Option<f64> {
        match *self.entities.get(e)? {
            Entity::Circle { radius, .. } => Some(radius),
            Entity::Arc { .. } => self.arc_geometry(e).map(|(_, r, _, _)| r),
            _ => None,
        }
    }

    /// The Offset constraints made by the same offset as constraint `ci` (their
    /// copies joined end to end), lowest first; just `ci` for anything else.
    /// They share one value, and only the first shows a dimension.
    pub fn offset_group(&self, ci: usize) -> Vec<usize> {
        let Some(&Constraint::Offset(_, first, _)) = self.constraints.get(ci) else { return vec![ci] };
        let ends = |e: EntityId| match self.entities.get(e) {
            Some(&Entity::Line { a, b }) | Some(&Entity::Arc { a, b, .. }) => vec![a, b],
            _ => Vec::new(),
        };
        let offsets: Vec<(usize, EntityId)> =
            self.constraints.iter().enumerate().filter_map(|(k, c)| if let Constraint::Offset(_, b, _) = *c { Some((k, b)) } else { None }).collect();
        let mut group = vec![first];
        let mut grew = true;
        while grew {
            grew = false;
            for &(_, e) in &offsets {
                if !group.contains(&e) && group.iter().any(|&g| ends(g).iter().any(|p| ends(e).contains(p))) {
                    group.push(e);
                    grew = true;
                }
            }
        }
        offsets.into_iter().filter(|(_, e)| group.contains(e)).map(|(k, _)| k).collect()
    }

    /// Where a dimension's label was dragged to (sketch units from default).
    pub fn label_shift(&self, ci: usize) -> DVec2 {
        self.label_shift.get(ci).copied().unwrap_or(DVec2::ZERO)
    }

    /// The unit dimension `ci` was typed in, if not the design's.
    pub fn dim_unit(&self, ci: usize) -> Option<crate::units::Unit> {
        self.dim_units.get(ci).copied().flatten()
    }

    /// The expression dimension `ci` was typed as, if any.
    pub fn dim_expr(&self, ci: usize) -> Option<&str> {
        self.dim_exprs.get(ci).and_then(|e| e.as_deref())
    }

    pub fn set_dim_expr(&mut self, ci: usize, text: Option<String>) {
        if self.dim_exprs.len() <= ci {
            self.dim_exprs.resize(ci + 1, None);
        }
        self.dim_exprs[ci] = text;
        while self.dim_exprs.last() == Some(&None) {
            self.dim_exprs.pop();
        }
    }

    pub fn set_dim_unit(&mut self, ci: usize, unit: Option<crate::units::Unit>) {
        if self.dim_units.len() <= ci {
            self.dim_units.resize(ci + 1, None);
        }
        self.dim_units[ci] = unit;
        while self.dim_units.last() == Some(&None) {
            self.dim_units.pop();
        }
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
            && self.plane_ref == other.plane_ref
            && self.points == other.points
            && self.entities == other.entities
            && self.constraints == other.constraints
            && self.driven == other.driven
            && self.construction == other.construction
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
                // A spline's ends join things; its inside fit points are its own.
                Entity::Spline { s } => {
                    if let (Some(&a), Some(&b)) = (self.splines.get(s).and_then(|l| l.first()), self.splines.get(s).and_then(|l| l.last())) {
                        n[a] += 1;
                        n[b] += 1;
                    }
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
                Entity::Spline { s } if self.splines.get(s).is_some_and(|l| l.first() == Some(&p) || l.last() == Some(&p)) => return None,
                Entity::Spline { .. } => {}
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
    /// corner into distances to that sharp. `t` is the distance along each of the
    /// two lines (in `corner_lines` order). Returns (line1, line2, new end 1,
    /// new end 2, the constraints to add).
    fn cut_corner(&mut self, p: PointId, t: (f64, f64)) -> Result<(EntityId, EntityId, PointId, PointId, Vec<Constraint>), String> {
        let (l1, q1, l2, q2) = self.corner_lines(p).ok_or("pick a corner where exactly two lines meet")?;
        let corner = self.points[p];
        let (d1, d2) = (self.points[q1] - corner, self.points[q2] - corner);
        if t.0 >= d1.length() - MIN_SIZE || t.1 >= d2.length() - MIN_SIZE {
            return Err("too large for these lines".into());
        }
        let (t1, t2) = (corner + d1.normalize() * t.0, corner + d2.normalize() * t.1);
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
        let (l1, l2, n1, n2, mut out) = trial.cut_corner(p, (r / half.tan(), r / half.tan()))?;
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

    /// Bevel the corner at `p` taking `d1` off one line and `d2` off the
    /// other. `first` says which line `d1` is for (else the lower-numbered one).
    pub fn chamfer_corner2(&mut self, p: PointId, d1: f64, d2: f64, first: Option<EntityId>) -> Result<Vec<Constraint>, String> {
        if !(d1 > 0.0 && d2 > 0.0) {
            return Err("the distances must be greater than zero".into());
        }
        let (l1, ..) = self.corner_lines(p).ok_or("pick a corner where exactly two lines meet")?;
        let t = if first.is_some_and(|f| f != l1) { (d2, d1) } else { (d1, d2) };
        let mut trial = self.clone();
        let (_, _, n1, n2, mut out) = trial.cut_corner(p, t)?;
        trial.entities.push(Entity::Line { a: n1, b: n2 });
        out.extend([Constraint::Distance(n1, p, t.0), Constraint::Distance(n2, p, t.1)]);
        *self = trial;
        Ok(out)
    }

    /// What a fillet (`d1` = radius) or chamfer (`d1`, `d2` along the lines,
    /// `first` as in `chamfer_corner2`) at corner `p` would look like, for the
    /// preview and the drag arrow. None if it can't be made.
    pub fn corner_cut(&self, p: PointId, fillet: bool, d1: f64, d2: f64, first: Option<EntityId>) -> Option<CornerCut> {
        let (l1, q1, _, q2) = self.corner_lines(p)?;
        let corner = *self.points.get(p)?;
        let (u1, u2) = ((self.points[q1] - corner).try_normalize()?, (self.points[q2] - corner).try_normalize()?);
        let bisector = (u1 + u2).try_normalize()?;
        let (len1, len2) = (self.points[q1].distance(corner), self.points[q2].distance(corner));
        if fillet {
            let half = u1.dot(u2).clamp(-1.0, 1.0).acos() / 2.0;
            if !(d1 > 0.0) || half < 1e-6 || std::f64::consts::FRAC_PI_2 - half < 1e-6 {
                return None;
            }
            let t = d1 / half.tan();
            if t >= len1 - MIN_SIZE || t >= len2 - MIN_SIZE {
                return None;
            }
            let (a, b) = (corner + u1 * t, corner + u2 * t);
            let center = corner + bisector * (d1 / half.sin());
            let (ea, eb) = (a - center, b - center);
            let (start, end) = if ea.perp_dot(eb) > 0.0 { (ea, eb) } else { (eb, ea) };
            let a0 = start.y.atan2(start.x);
            let sweep = (end.y.atan2(end.x) - a0).rem_euclid(std::f64::consts::TAU);
            Some(CornerCut { a, b, arc: Some((center, d1, a0, sweep)) })
        } else {
            if !(d1 > 0.0 && d2 > 0.0) {
                return None;
            }
            let (t1, t2) = if first.is_some_and(|f| f != l1) { (d2, d1) } else { (d1, d2) };
            if t1 >= len1 - MIN_SIZE || t2 >= len2 - MIN_SIZE {
                return None;
            }
            Some(CornerCut { a: corner + u1 * t1, b: corner + u2 * t2, arc: None })
        }
    }

    /// Where line `e` is cut by other geometry: (parameter t along the line,
    /// the spot, the cutting entity, an existing point of it to join to).
    /// Touching at the line's own ends doesn't count.
    fn line_cuts(&self, e: EntityId) -> Vec<(f64, DVec2, EntityId, Option<PointId>)> {
        let Some(&Entity::Line { a, b }) = self.entities.get(e) else { return Vec::new() };
        let (pa, pb) = (self.points[a], self.points[b]);
        let d = pb - pa;
        let len = d.length();
        if len < 1e-9 {
            return Vec::new();
        }
        let tol = 1e-7;
        let (t_lo, t_hi) = (tol / len, 1.0 - tol / len);
        let mut cuts = Vec::new();
        let near = |p: PointId, x: DVec2| (self.points[p].distance(x) < 1e-6).then_some(p);
        for (i, ent) in self.entities.iter().enumerate() {
            if i == e {
                continue;
            }
            match *ent {
                Entity::Line { a: c, b: d2 } => {
                    let (pc, pd) = (self.points[c], self.points[d2]);
                    let f = pd - pc;
                    let denom = d.perp_dot(f);
                    if denom.abs() < 1e-12 * len * f.length().max(1e-12) {
                        continue; // parallel
                    }
                    let t = (pc - pa).perp_dot(f) / denom;
                    let u = (pc - pa).perp_dot(d) / denom;
                    let su = tol / f.length().max(1e-12);
                    if t > t_lo && t < t_hi && u >= -su && u <= 1.0 + su {
                        let x = pa + d * t;
                        cuts.push((t, x, i, near(c, x).or(near(d2, x))));
                    }
                }
                Entity::Circle { center, radius } => {
                    for t in line_circle(pa, d, self.points[center], radius) {
                        if t > t_lo && t < t_hi {
                            cuts.push((t, pa + d * t, i, None));
                        }
                    }
                }
                Entity::Arc { a: ea, b: eb, .. } => {
                    let Some((c, r, a0, sweep)) = self.arc_geometry(i) else { continue };
                    for t in line_circle(pa, d, c, r) {
                        let x = pa + d * t;
                        let ang = (x - c).y.atan2((x - c).x);
                        let on = (ang - a0).rem_euclid(std::f64::consts::TAU) <= sweep + 1e-9;
                        if t > t_lo && t < t_hi && on {
                            cuts.push((t, x, i, near(ea, x).or(near(eb, x))));
                        }
                    }
                }
                // A spline cuts where its curve crosses (the new end isn't tied to it).
                Entity::Spline { s } => {
                    let ends = self.splines.get(s).map_or(Vec::new(), |l| [l.first(), l.last()].into_iter().flatten().copied().collect());
                    for w in self.spline_curve(s).windows(2) {
                        let f = w[1] - w[0];
                        let denom = d.perp_dot(f);
                        if denom.abs() < 1e-12 * len * f.length().max(1e-12) {
                            continue;
                        }
                        let t = (w[0] - pa).perp_dot(f) / denom;
                        let u = (w[0] - pa).perp_dot(d) / denom;
                        if t > t_lo && t < t_hi && (0.0..=1.0).contains(&u) {
                            let x = pa + d * t;
                            cuts.push((t, x, i, ends.iter().copied().find_map(|q| near(q, x))));
                        }
                    }
                }
                Entity::Point { .. } => {}
            }
        }
        cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
        cuts.dedup_by(|x, y| (x.0 - y.0).abs() * len < 1e-7);
        cuts
    }

    /// Trim line `e` at the click `at`: the stretch of the line under the
    /// click, between the nearest cuts by other geometry, is removed. An end
    /// piece just shortens the line (it ends exactly at the cut, joined to the
    /// other line when it ends there, else tied onto it); a middle piece
    /// splits the line in two; a line nothing crosses is deleted.
    /// Returns constraints for the caller to add through the solver.
    /// The stretch of line `e` that Trim would take away for a click at `at`
    /// (for showing it before the click).
    pub fn trim_preview(&self, e: EntityId, at: DVec2) -> Option<(DVec2, DVec2)> {
        let Some(&Entity::Line { a, b }) = self.entities.get(e) else { return None };
        let (pa, pb) = (self.points[a], self.points[b]);
        let d = pb - pa;
        let tc = ((at - pa).dot(d) / d.length_squared().max(1e-30)).clamp(0.0, 1.0);
        let cuts = self.line_cuts(e);
        let lo = cuts.iter().rev().find(|c| c.0 <= tc).map_or(pa, |c| c.1);
        let hi = cuts.iter().find(|c| c.0 > tc).map_or(pb, |c| c.1);
        Some((lo, hi))
    }

    pub fn trim_line(&mut self, e: EntityId, at: DVec2) -> Result<Vec<Constraint>, String> {
        let Some(&Entity::Line { a, b }) = self.entities.get(e) else {
            return Err("Trim works on lines".into());
        };
        let (pa, pb) = (self.points[a], self.points[b]);
        let d = pb - pa;
        let tc = ((at - pa).dot(d) / d.length_squared().max(1e-30)).clamp(0.0, 1.0);
        let cuts = self.line_cuts(e);
        let lo = cuts.iter().rev().find(|c| c.0 <= tc).copied();
        let hi = cuts.iter().find(|c| c.0 > tc).copied();
        if lo.is_none() && hi.is_none() {
            self.delete(&[Sel::Entity(e)]);
            return Ok(Vec::new());
        }
        let mut trial = self.clone();
        // New points (to be tied to what cuts there) are found again by
        // position after unused points are cleaned up.
        let mut ties: Vec<(DVec2, EntityId)> = Vec::new();
        let mut cut_point = |sk: &mut Sketch, c: (f64, DVec2, EntityId, Option<PointId>)| match c.3 {
            Some(p) => p,
            None => {
                sk.points.push(c.1);
                ties.push((c.1, c.2));
                sk.points.len() - 1
            }
        };
        // Its length is about to change: drop what pins the old one.
        trial.retain_constraints(|k| match *k {
            Constraint::Length(l, _) => l != e,
            Constraint::Equal(x, y) => x != e && y != e,
            _ => true,
        });
        let mut copy_to_new: Vec<Constraint> = Vec::new();
        match (lo, hi) {
            (Some(l), None) => {
                let p = cut_point(&mut trial, l);
                trial.entities[e] = Entity::Line { a, b: p };
            }
            (None, Some(h)) => {
                let p = cut_point(&mut trial, h);
                trial.entities[e] = Entity::Line { a: p, b };
            }
            (Some(l), Some(h)) => {
                let (pl, ph) = (cut_point(&mut trial, l), cut_point(&mut trial, h));
                trial.entities[e] = Entity::Line { a, b: pl };
                trial.entities.push(Entity::Line { a: ph, b });
                let n = trial.entities.len() - 1;
                trial.set_construction(n, trial.is_construction(e));
                for k in &trial.constraints {
                    match *k {
                        Constraint::Horizontal(x) if x == e => copy_to_new.push(Constraint::Horizontal(n)),
                        Constraint::Vertical(x) if x == e => copy_to_new.push(Constraint::Vertical(n)),
                        Constraint::Parallel(x, y) if x == e => copy_to_new.push(Constraint::Parallel(n, y)),
                        Constraint::Parallel(x, y) if y == e => copy_to_new.push(Constraint::Parallel(x, n)),
                        Constraint::Perpendicular(x, y) if x == e => copy_to_new.push(Constraint::Perpendicular(n, y)),
                        Constraint::Perpendicular(x, y) if y == e => copy_to_new.push(Constraint::Perpendicular(x, n)),
                        _ => {}
                    }
                }
            }
            (None, None) => unreachable!("handled above"),
        }
        trial.delete(&[]); // the old end point, if nothing else uses it
        let mut out = copy_to_new;
        for (x, other) in ties {
            let Some(p) = trial.points.iter().position(|q| *q == x) else { continue };
            match trial.entities.get(other) {
                Some(Entity::Line { .. }) => out.push(Constraint::OnLine(p, other)),
                Some(Entity::Circle { .. } | Entity::Arc { .. }) => out.push(Constraint::OnCircle(p, other)),
                _ => {} // (nothing holds a point on a spline yet)
            }
        }
        *self = trial;
        Ok(out)
    }

    /// A circle or arc as (centre, radius, start angle, sweep); a circle starts
    /// at angle 0 and goes all the way round.
    fn round_shape(&self, e: EntityId) -> Option<(DVec2, f64, f64, f64)> {
        match *self.entities.get(e)? {
            Entity::Circle { center, radius } => Some((self.points[center], radius, 0.0, std::f64::consts::TAU)),
            Entity::Arc { .. } => self.arc_geometry(e),
            _ => None,
        }
    }

    /// Where other geometry crosses circle / arc `e`: (angle from its start,
    /// the point, what crosses it, an existing point of that there). Sorted;
    /// an arc's own ends don't count.
    fn round_cuts(&self, e: EntityId) -> Vec<(f64, DVec2, EntityId, Option<PointId>)> {
        let Some((c, r, a0, sweep)) = self.round_shape(e) else { return Vec::new() };
        let whole = matches!(self.entities[e], Entity::Circle { .. });
        let rel = |x: DVec2| ((x - c).y.atan2((x - c).x) - a0).rem_euclid(std::f64::consts::TAU);
        let eps = 1e-7 / r.max(1e-9);
        let near = |p: PointId, x: DVec2| (self.points[p].distance(x) < 1e-6).then_some(p);
        let mut cuts = Vec::new();
        let mut offer = |x: DVec2, other: EntityId, join: Option<PointId>| {
            let t = rel(x);
            if whole || (t > eps && t < sweep - eps) {
                cuts.push((t, x, other, join));
            }
        };
        for (i, ent) in self.entities.iter().enumerate() {
            if i == e {
                continue;
            }
            match *ent {
                Entity::Line { a, b } => {
                    let (pa, pb) = (self.points[a], self.points[b]);
                    let d = pb - pa;
                    let st = 1e-7 / d.length().max(1e-12);
                    for t in line_circle(pa, d, c, r) {
                        if t >= -st && t <= 1.0 + st {
                            let x = pa + d * t;
                            offer(x, i, near(a, x).or(near(b, x)));
                        }
                    }
                }
                Entity::Circle { center, radius } => {
                    for x in circle_circle(c, r, self.points[center], radius) {
                        offer(x, i, None);
                    }
                }
                Entity::Spline { .. } => {} // (curves cut by splines: not yet)
                Entity::Arc { a, b, .. } => {
                    let Some((c2, r2, b0, sw2)) = self.arc_geometry(i) else { continue };
                    for x in circle_circle(c, r, c2, r2) {
                        let on = ((x - c2).y.atan2((x - c2).x) - b0).rem_euclid(std::f64::consts::TAU) <= sw2 + 1e-9;
                        if on {
                            offer(x, i, near(a, x).or(near(b, x)));
                        }
                    }
                }
                Entity::Point { .. } => {}
            }
        }
        cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
        cuts.dedup_by(|x, y| (x.0 - y.0).abs() * r < 1e-7);
        cuts
    }

    /// The stretch of a circle or arc Trim takes away for a click at `at`:
    /// (centre, radius, start angle, sweep) — the whole thing when nothing cuts it.
    fn round_trim_span(&self, e: EntityId, at: DVec2) -> Option<(DVec2, f64, f64, f64)> {
        let (c, r, a0, sweep) = self.round_shape(e)?;
        let whole = matches!(self.entities[e], Entity::Circle { .. });
        let t = ((at - c).y.atan2((at - c).x) - a0).rem_euclid(std::f64::consts::TAU).min(sweep);
        let cuts = self.round_cuts(e);
        if whole {
            if cuts.len() < 2 {
                return Some((c, r, 0.0, std::f64::consts::TAU));
            }
            let lo = cuts.iter().rev().find(|k| k.0 <= t).unwrap_or(cuts.last().expect("two cuts")).0;
            let hi = cuts.iter().find(|k| k.0 > t).unwrap_or(&cuts[0]).0;
            return Some((c, r, lo, (hi - lo).rem_euclid(std::f64::consts::TAU)));
        }
        let lo = cuts.iter().rev().find(|k| k.0 <= t).map_or(0.0, |k| k.0);
        let hi = cuts.iter().find(|k| k.0 > t).map_or(sweep, |k| k.0);
        Some((c, r, a0 + lo, hi - lo))
    }

    /// Trim a circle or arc at the click `at`, as Trim does a line: the
    /// stretch under the click, between the nearest cuts, goes. A circle cut
    /// in two or more places becomes the arc that's left (a Diameter becomes
    /// a Radius); an arc end piece shortens it, a middle piece splits it in
    /// two (the second kept equal to the first); nothing crossing deletes it.
    /// New ends are tied onto what cut them. Constraints for the caller to add.
    pub fn trim_round(&mut self, e: EntityId, at: DVec2) -> Result<Vec<Constraint>, String> {
        let Some((c, _, a0, sweep)) = self.round_shape(e) else { return Err("Trim works on lines, circles and arcs".into()) };
        let whole = matches!(self.entities[e], Entity::Circle { .. });
        let center = match self.entities[e] {
            Entity::Circle { center, .. } | Entity::Arc { center, .. } => center,
            _ => unreachable!("round"),
        };
        let cuts = self.round_cuts(e);
        let t = ((at - c).y.atan2((at - c).x) - a0).rem_euclid(std::f64::consts::TAU).min(sweep);
        let lo = cuts.iter().rev().find(|k| k.0 <= t).copied();
        let hi = cuts.iter().find(|k| k.0 > t).copied();
        if (whole && cuts.len() < 2) || (!whole && lo.is_none() && hi.is_none()) {
            self.delete(&[Sel::Entity(e)]);
            return Ok(Vec::new());
        }
        let mut trial = self.clone();
        let mut ties: Vec<(DVec2, EntityId)> = Vec::new();
        let mut cut_point = |sk: &mut Sketch, k: (f64, DVec2, EntityId, Option<PointId>)| match k.3 {
            Some(p) => p,
            None => {
                sk.points.push(k.1);
                ties.push((k.1, k.2));
                sk.points.len() - 1
            }
        };
        let mut extra: Vec<Constraint> = Vec::new();
        if whole {
            // The part left runs from the cut after the click round to the cut before it.
            let lo = lo.unwrap_or(*cuts.last().expect("two cuts"));
            let hi = hi.unwrap_or(cuts[0]);
            let (pl, ph) = (cut_point(&mut trial, lo), cut_point(&mut trial, hi));
            trial.entities[e] = Entity::Arc { center, a: ph, b: pl };
            // An arc takes a radius, not a diameter.
            for k in trial.constraints.iter_mut() {
                if let Constraint::Diameter(x, d) = *k {
                    if x == e {
                        *k = Constraint::Radius(e, d / 2.0);
                    }
                }
            }
        } else {
            let Entity::Arc { a, b, .. } = self.entities[e] else { unreachable!("arc") };
            match (lo, hi) {
                (Some(l), None) => {
                    let p = cut_point(&mut trial, l);
                    trial.entities[e] = Entity::Arc { center, a, b: p };
                }
                (None, Some(h)) => {
                    let p = cut_point(&mut trial, h);
                    trial.entities[e] = Entity::Arc { center, a: p, b };
                }
                (Some(l), Some(h)) => {
                    let (pl, ph) = (cut_point(&mut trial, l), cut_point(&mut trial, h));
                    trial.entities[e] = Entity::Arc { center, a, b: pl };
                    trial.entities.push(Entity::Arc { center, a: ph, b });
                    let n = trial.entities.len() - 1;
                    trial.set_construction(n, trial.is_construction(e));
                    extra.push(Constraint::Equal(n, e));
                }
                (None, None) => unreachable!("handled above"),
            }
        }
        trial.delete(&[]); // old end points nothing uses now
        let mut out = extra;
        for (x, other) in ties {
            let Some(p) = trial.points.iter().position(|q| *q == x) else { continue };
            match trial.entities.get(other) {
                Some(Entity::Line { .. }) => out.push(Constraint::OnLine(p, other)),
                Some(Entity::Circle { .. } | Entity::Arc { .. }) => out.push(Constraint::OnCircle(p, other)),
                _ => {} // (nothing holds a point on a spline yet)
            }
        }
        *self = trial;
        Ok(out)
    }

    /// Trim whatever `e` is (line, circle or arc) at the click `at`.
    pub fn trim(&mut self, e: EntityId, at: DVec2) -> Result<Vec<Constraint>, String> {
        if self.is_line(e) { self.trim_line(e, at) } else { self.trim_round(e, at) }
    }

    /// What Trim would take away for a click at `at` (for showing it first).
    pub fn trim_piece(&self, e: EntityId, at: DVec2) -> Option<TrimPiece> {
        if self.is_line(e) {
            self.trim_preview(e, at).map(|(a, b)| TrimPiece::Line(a, b))
        } else {
            self.round_trim_span(e, at).map(|(c, r, a0, sweep)| TrimPiece::Arc(c, r, a0, sweep))
        }
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
            (Offset(a, b, _), Offset(c, d, _)) => (a, b) == (c, d),
            (Angle(a, b, _), Angle(c, d, _)) | (Distance(a, b, _), Distance(c, d, _)) => {
                (a, b) == (c, d) || (a, b) == (d, c)
            }
            (Parallel(a, b), Parallel(c, d))
            | (Perpendicular(a, b), Perpendicular(c, d))
            | (Equal(a, b), Equal(c, d))
            | (Coincident(a, b), Coincident(c, d))
            | (Collinear(a, b), Collinear(c, d))
            | (Concentric(a, b), Concentric(c, d)) => (a, b) == (c, d) || (a, b) == (d, c),
            (x, c) => x == c,
        })
    }

    /// Which constraint actions apply to a selection and what each would add.
    pub fn constraint_options(&self, sel: &[Sel]) -> Vec<(&'static str, Vec<Constraint>)> {
        use Constraint::*;
        let (mut lines, mut circles, mut points, mut axes) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for s in sel {
            match *s {
                Sel::Point(p) => points.push(p),
                Sel::Entity(e) if self.is_line(e) => lines.push(e),
                Sel::Entity(e) if self.is_round(e) => circles.push(e),
                Sel::Entity(_) => {}
                Sel::Axis(k) => axes.push(k),
            }
        }
        let mut out: Vec<(&'static str, Vec<Constraint>)> = Vec::new();
        // With one of the sketch's axes: a point onto it, a line along it,
        // parallel or square to it.
        if let [k] = axes[..] {
            match (lines.as_slice(), circles.as_slice(), points.as_slice()) {
                ([], [], [p]) if *p != Self::ORIGIN => out.push(("Coincident", vec![OnAxis(*p, k)])),
                ([l], [], []) => {
                    if let Some(&Entity::Line { a, b }) = self.entities.get(*l) {
                        out.push(("Collinear", vec![OnAxis(a, k), OnAxis(b, k)]));
                    }
                    let (along, across) = if k == 0 { (Horizontal(*l), Vertical(*l)) } else { (Vertical(*l), Horizontal(*l)) };
                    out.push(("Parallel", vec![along]));
                    out.push(("Perpendicular", vec![across]));
                }
                ([], [c], []) => {
                    if let Some(&Entity::Circle { center, .. } | &Entity::Arc { center, .. }) = self.entities.get(*c) {
                        out.push(("Coincident", vec![OnAxis(center, k)]));
                    }
                }
                _ => {}
            }
            for (_, cs) in &mut out {
                cs.retain(|c| !self.has_similar(c));
            }
            out.retain(|(_, cs)| !cs.is_empty());
            return out;
        }
        if !axes.is_empty() {
            return out;
        }
        let only_lines = !lines.is_empty() && circles.is_empty() && points.is_empty();
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
                out.push(("Collinear", vec![Collinear(*a, *b)]));
            }
            ([], [a, b], []) => {
                out.push(("Equal", vec![Equal(*a, *b)]));
                out.push(("Concentric", vec![Concentric(*a, *b)]));
            }
            // A point onto a circle or arc.
            ([], [c], [p]) => out.push(("Coincident", vec![OnCircle(*p, *c)])),
            ([l], [c], []) => out.push(("Tangent", vec![Tangent(*l, *c)])),
            // A point and a line: the point sits at the line's middle.
            ([l], [], [p]) => {
                if let Some(&Entity::Line { a, b }) = self.entities.get(*l) {
                    if *p != a && *p != b {
                        out.push(("Midpoint", vec![Midpoint(*p, a, b)]));
                        out.push(("Coincident", vec![OnLine(*p, *l)]));
                    }
                }
            }
            // Two points: the distance between them.
            ([], [], [p, q]) if p != q => {
                out.push(("Dimension", vec![Distance(*p, *q, self.points[*p].distance(self.points[*q]))]));
                out.push(("Coincident", vec![Coincident(*p, *q)]));
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
    Trim,
    /// Run a line on to the next geometry in its way.
    Extend,
    /// Copy a run of lines / arcs (or a circle) to one side.
    Offset,
    /// Centre-to-centre slot: two centres, then the width.
    Slot,
    /// Copy the geometry that was selected across a clicked line.
    Mirror,
    /// Click a body edge to bring it into the sketch.
    Project,
    PolygonCircumscribed,
    PolygonInscribed,
    PolygonEdge,
    /// Three lines it touches.
    Circle3Tangent,
    /// Fit-point spline: click its points; Enter (or clicking the last again) finishes.
    Spline,
    /// Pick geometry for a constraint ("Dimension", "Parallel", …): the
    /// constraint is added as soon as the picks are enough for it.
    Constrain(&'static str),
}

impl Tool {
    /// The arc tools, in the order of the toolbar's arc menu.
    pub const ARCS: [Tool; 2] = [Tool::Arc3Point, Tool::ArcCenter];

    /// The polygon tools, in the order of the toolbar's polygon menu.
    pub const POLYGONS: [Tool; 3] = [Tool::PolygonCircumscribed, Tool::PolygonInscribed, Tool::PolygonEdge];

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
            Tool::Spline => "Spline",
            Tool::Circle => "Center Diameter Circle",
            Tool::Circle2Point => "2-Point Circle",
            Tool::Circle3Point => "3-Point Circle",
            Tool::Circle2Tangent => "2-Tangent Circle",
            Tool::Circle3Tangent => "3-Tangent Circle",
            Tool::Arc3Point => "3-Point Arc",
            Tool::ArcCenter => "Center Point Arc",
            Tool::SketchFillet => "Fillet",
            Tool::SketchChamfer => "Chamfer",
            Tool::Trim => "Trim",
            Tool::Extend => "Extend",
            Tool::Offset => "Offset",
            Tool::Slot => "Slot",
            Tool::Mirror => "Mirror",
            Tool::Project => "Project",
            Tool::PolygonCircumscribed => "Circumscribed Polygon",
            Tool::PolygonInscribed => "Inscribed Polygon",
            Tool::PolygonEdge => "Edge Polygon",
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
    /// Construction mode (Fusion's X while drawing): new geometry is construction.
    pub construction: bool,
    /// Polygon tools: the number of sides (0 = not set yet, which means 6).
    pub polygon_sides: usize,
    /// Spline tool: the fit points clicked so far.
    pub spline: Vec<Anchor>,
}

/// A slot: the two end-cap centres and half the width.
#[derive(Clone, Copy, Debug)]
pub struct SlotShape {
    pub a: DVec2,
    pub b: DVec2,
    pub half: f64,
}

impl Editor {
    /// The slot the clicks so far make, with the width set by `here`.
    pub fn slot_shape(&self, sk: &Sketch, here: DVec2) -> Option<SlotShape> {
        let (a, b) = (sk.pos(self.start?), sk.pos(self.second?));
        let d = (b - a).try_normalize()?;
        let half = d.perp_dot(here - a).abs();
        (half >= MIN_SIZE / 2.0).then_some(SlotShape { a, b, half })
    }

    pub const MAX_SIDES: usize = 64;

    /// Sides for the polygon tools, between 3 and 64 (Fusion starts at 6).
    pub fn sides(&self) -> usize {
        if self.polygon_sides < 3 { 6 } else { self.polygon_sides.min(Self::MAX_SIDES) }
    }

    pub fn set_tool(&mut self, tool: Tool) {
        self.tool = tool;
        self.cancel();
    }

    pub fn cancel(&mut self) {
        self.start = None;
        self.second = None;
        self.tangent_lines.clear();
        self.spline.clear();
    }

    /// Is something half-made (a line started, circle points or lines picked)?
    pub fn busy(&self) -> bool {
        self.start.is_some() || self.second.is_some() || !self.tangent_lines.is_empty() || !self.spline.is_empty()
    }

    /// The spline being drawn, through its points so far and `here` (the cursor).
    pub fn spline_preview(&self, sk: &Sketch, here: DVec2) -> Vec<DVec2> {
        let mut pts: Vec<DVec2> = self.spline.iter().filter(|a| sk.is_valid_anchor(**a)).map(|&a| sk.pos(a)).collect();
        if pts.last().is_none_or(|l| l.distance(here) > MIN_SIZE) {
            pts.push(here);
        }
        crate::spline::polyline(&pts)
    }

    /// Finish the spline being drawn: its points become sketch points (tied
    /// onto lines / axes they were dropped on) and the curve goes through
    /// them. Needs two points; returns the constraints to add.
    pub fn finish_spline(&mut self, sk: &mut Sketch) -> Vec<Constraint> {
        self.make_spline(sk, false)
    }

    /// `finish_spline`; `closed` joins it back to its first point (smoothly).
    fn make_spline(&mut self, sk: &mut Sketch, closed: bool) -> Vec<Constraint> {
        let anchors: Vec<Anchor> = std::mem::take(&mut self.spline).into_iter().filter(|a| sk.is_valid_anchor(*a)).collect();
        let mut ties = Vec::new();
        let mut pts: Vec<PointId> = Vec::new();
        for a in anchors {
            if pts.last().is_some_and(|&l| sk.points[l].distance(sk.pos(a)) < MIN_SIZE) {
                continue; // the same spot twice
            }
            let p = sk.resolve(a);
            match a {
                Anchor::OnLine(e, _) => ties.push(Constraint::OnLine(p, e)),
                Anchor::OnAxis(axis, _) => ties.push(Constraint::OnAxis(p, axis)),
                _ => {}
            }
            pts.push(p);
        }
        if pts.len() < if closed { 3 } else { 2 } {
            sk.delete(&[]); // points made for nothing
            return Vec::new();
        }
        if closed {
            pts.push(pts[0]);
        }
        sk.splines.push(pts);
        sk.entities.push(Entity::Spline { s: sk.splines.len() - 1 });
        if self.construction {
            sk.set_construction(sk.entities.len() - 1, true);
        }
        ties
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

    /// The regular polygon a polygon tool would make with the cursor at `at`.
    pub fn polygon_preview(&self, sk: &Sketch, at: DVec2) -> Option<PolygonShape> {
        use std::f64::consts::{PI, TAU};
        let pos = |a: Option<Anchor>| a.filter(|&a| sk.is_valid_anchor(a)).map(|a| sk.pos(a));
        let p = pos(self.start)?;
        let n = self.sides();
        let ring = |center: DVec2, radius: f64, first: f64, step: f64| -> Vec<DVec2> {
            (0..n).map(|k| center + DVec2::from_angle(first + step * k as f64) * radius).collect()
        };
        let angle = |v: DVec2| v.y.atan2(v.x);
        let shape = match self.tool {
            Tool::PolygonInscribed => {
                let r = p.distance(at);
                PolygonShape { center: p, radius: r, verts: ring(p, r, angle(at - p), TAU / n as f64) }
            }
            Tool::PolygonCircumscribed => {
                let apothem = p.distance(at);
                let r = apothem / (PI / n as f64).cos();
                PolygonShape { center: p, radius: r, verts: ring(p, r, angle(at - p) - PI / n as f64, TAU / n as f64) }
            }
            Tool::PolygonEdge => {
                let q = pos(self.second)?;
                let side = q - p;
                let s = side.length();
                if s < MIN_SIZE {
                    return None;
                }
                let mid = (p + q) * 0.5;
                let left = side.perp() / s;
                // Built on whichever side of the edge the cursor is.
                let (nrm, step) = if left.dot(at - mid) < 0.0 { (-left, -TAU / n as f64) } else { (left, TAU / n as f64) };
                let center = mid + nrm * (s / (2.0 * (PI / n as f64).tan()));
                let r = s / (2.0 * (PI / n as f64).sin());
                PolygonShape { center, radius: r, verts: ring(center, r, angle(p - center), step) }
            }
            _ => return None,
        };
        (shape.radius >= MIN_SIZE).then_some(shape)
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
        let before = sk.entities.len();
        let out = self.click_tool(sk, at);
        if self.construction {
            for e in before..sk.entities.len() {
                sk.set_construction(e, true);
            }
        }
        out
    }

    fn click_tool(&mut self, sk: &mut Sketch, at: Anchor) -> Vec<Constraint> {
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
                | Tool::Trim
                | Tool::Extend
                | Tool::Offset
                | Tool::Mirror
                | Tool::Project
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
                // Right at a line's middle: the point is its midpoint, not just on it.
                Anchor::OnLine(e, q) => match sk.entities.get(e) {
                    Some(&Entity::Line { a: la, b: lb }) if sk.points[la].midpoint(sk.points[lb]).distance(q) < 1e-9 * (1.0 + sk.points[la].distance(sk.points[lb])) && p != la && p != lb => {
                        ties.push(Constraint::Midpoint(p, la, lb))
                    }
                    _ => ties.push(Constraint::OnLine(p, e)),
                },
                Anchor::OnAxis(axis, _) => ties.push(Constraint::OnAxis(p, axis)),
                _ => {}
            }
            p
        };
        // Spline: each click is a fit point; the last one clicked again finishes.
        if self.tool == Tool::Spline {
            if self.spline.last().is_some_and(|&l| sk.is_valid_anchor(l) && sk.pos(l).distance(sk.pos(at)) < MIN_SIZE) {
                return self.finish_spline(sk);
            }
            // Back on the first point (with three or more placed): a closed shape, done.
            let back_to_start = self.spline.first().is_some_and(|&f| sk.is_valid_anchor(f) && (f == at || sk.pos(f).distance(sk.pos(at)) < MIN_SIZE));
            if self.spline.len() >= 3 && back_to_start {
                return self.make_spline(sk, true);
            }
            self.spline.push(at);
            return Vec::new();
        }
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
            Tool::PolygonEdge if self.second.is_none() => {
                if size >= MIN_SIZE {
                    self.second = Some(at); // the edge is set; the next click picks the side
                }
                Vec::new()
            }
            Tool::PolygonCircumscribed | Tool::PolygonInscribed | Tool::PolygonEdge => {
                let second = self.second.take();
                self.start = None;
                let probe = Editor { tool: self.tool, start: Some(start), second, polygon_sides: self.polygon_sides, ..Default::default() };
                let Some(shape) = probe.polygon_preview(sk, sk.pos(at)) else { return Vec::new() };
                let n = shape.verts.len();
                let inscribed = self.tool != Tool::PolygonCircumscribed;
                // Centre point of its own (a clicked centre is kept as it is).
                let center = if self.tool == Tool::PolygonEdge {
                    sk.resolve(Anchor::Free(shape.center))
                } else {
                    resolve(sk, start)
                };
                let used = sk.entities.iter().any(|e| match *e {
                    Entity::Point { p } => p == center,
                    Entity::Line { a, b } => a == center || b == center,
                    Entity::Circle { center: c, .. } => c == center,
                    Entity::Arc { center: c, a, b } => c == center || a == center || b == center,
                    Entity::Spline { s } => sk.splines.get(s).is_some_and(|l| l.contains(&center)),
                });
                if !used {
                    sk.entities.push(Entity::Point { p: center });
                }
                // Corners: the clicked ones are reused.
                let corners: Vec<PointId> = shape
                    .verts
                    .iter()
                    .enumerate()
                    .map(|(k, &v)| match (self.tool, k) {
                        (Tool::PolygonEdge, 0) => resolve(sk, start),
                        (Tool::PolygonEdge, 1) => resolve(sk, second.expect("edge set")),
                        (Tool::PolygonInscribed, 0) => resolve(sk, at),
                        _ => sk.resolve(Anchor::Free(v)),
                    })
                    .collect();
                // The reference circle (construction): corners on it, or edges tangent to it.
                let circle_r = if inscribed { shape.radius } else { shape.radius * (std::f64::consts::PI / n as f64).cos() };
                sk.entities.push(Entity::Circle { center, radius: circle_r });
                let circle = sk.entities.len() - 1;
                sk.set_construction(circle, true);
                let first = sk.entities.len();
                for k in 0..n {
                    sk.entities.push(Entity::Line { a: corners[k], b: corners[(k + 1) % n] });
                }
                // Corners on a circle plus equal sides make it regular. For circumscribed
                // polygons that circle is a second, hidden one; the size dimension is on
                // the circle the edges touch, which one edge is tied to (tying them all
                // would only repeat what the rest already says).
                let on_circle = if inscribed {
                    circle
                } else {
                    sk.entities.push(Entity::Circle { center, radius: shape.radius });
                    let outer = sk.entities.len() - 1;
                    sk.set_construction(outer, true);
                    outer
                };
                let mut out: Vec<Constraint> = (0..n).map(|k| Constraint::OnCircle(corners[k], on_circle)).collect();
                out.extend((1..n).map(|k| Constraint::Equal(first, first + k)));
                if !inscribed {
                    out.push(Constraint::Tangent(first, circle));
                }
                // Its size is a dimension on the reference circle (edit it like any other).
                out.push(Constraint::Diameter(circle, 2.0 * circle_r));
                out.extend(ties);
                out
            }
            Tool::Slot if self.second.is_none() => {
                if size >= MIN_SIZE {
                    self.second = Some(at); // the centres are set; the next click gives the width
                }
                Vec::new()
            }
            Tool::Slot => {
                let second = self.second.take().expect("checked above");
                self.start = None;
                let probe = Editor { tool: self.tool, start: Some(start), second: Some(second), ..Default::default() };
                let Some(shape) = probe.slot_shape(sk, sk.pos(at)) else { return Vec::new() };
                let (ca, cb) = (resolve(sk, start), resolve(sk, second));
                let n = (shape.b - shape.a).normalize().perp() * shape.half;
                let [a_up, b_up, b_dn, a_dn] = [shape.a + n, shape.b + n, shape.b - n, shape.a - n].map(|q| sk.resolve(Anchor::Free(q)));
                let first = sk.entities.len();
                sk.entities.push(Entity::Line { a: a_up, b: b_up });
                sk.entities.push(Entity::Line { a: b_dn, b: a_dn });
                // Counter-clockwise: round B from its lower to its upper side (through the far end), round A the other way.
                sk.entities.push(Entity::Arc { center: cb, a: b_dn, b: b_up });
                sk.entities.push(Entity::Arc { center: ca, a: a_up, b: a_dn });
                // The line through the two centres, as construction.
                let spine = sk.entities.len();
                sk.entities.push(Entity::Line { a: ca, b: cb });
                sk.set_construction(spine, true);
                let mut out = vec![
                    Constraint::Tangent(first, first + 2),
                    Constraint::Tangent(first + 1, first + 2),
                    Constraint::Tangent(first, first + 3),
                    Constraint::Tangent(first + 1, first + 3),
                    Constraint::Equal(first + 2, first + 3),
                    Constraint::Radius(first + 2, shape.half),
                ];
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
                        Entity::Spline { s } => sk.splines.get(s).is_some_and(|l| l.contains(&center)),
                    })
                {
                    sk.entities.push(Entity::Point { p: center });
                }
                let corner = c.map(|q| sk.resolve(Anchor::Free(q)));
                let first = sk.entities.len();
                for k in 0..4 {
                    sk.entities.push(Entity::Line { a: corner[k], b: corner[(k + 1) % 4] });
                }
                // Construction diagonals through the centre, as in Fusion.
                for (u, v) in [(0, 2), (1, 3)] {
                    sk.set_construction(sk.entities.len(), true);
                    sk.entities.push(Entity::Line { a: corner[u], b: corner[v] });
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
            Tool::Circle2Tangent | Tool::Circle3Tangent | Tool::Constrain(_) | Tool::Point | Tool::Spline => Vec::new(),
            Tool::SketchFillet | Tool::SketchChamfer | Tool::Trim | Tool::Extend | Tool::Offset | Tool::Mirror | Tool::Project => Vec::new(),
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
        // Inch designs: inch fractions, a major line every inch.
        let g = GridSpec::for_units(100.0, crate::units::Unit::In);
        assert!(close(g.step, 0.25 * 25.4) && g.major_every == 4, "{g:?}");
        assert!(close(GridSpec::for_units(600.0, crate::units::Unit::In).step, 25.4));
        assert_eq!(GridSpec::for_units(300.0, crate::units::Unit::Mm), GridSpec::for_distance(300.0));
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
        assert_eq!(labels(&two), vec!["Dimension", "Parallel", "Perpendicular", "Equal", "Collinear", "Horizontal", "Vertical"]);
        assert!(matches!(two[0].1[0], Constraint::Angle(0, 1, _)), "dimensioning two lines gives their angle");
        assert_eq!(two[5].1, vec![Constraint::Horizontal(0), Constraint::Horizontal(1)]);

        let circ = sk.constraint_options(&[Sel::Entity(2)]);
        assert_eq!(circ[0].1, vec![Constraint::Diameter(2, 10.0)]);

        let tan = sk.constraint_options(&[Sel::Entity(1), Sel::Entity(2)]);
        assert_eq!(labels(&tan), vec!["Tangent"]);

        let pts = sk.constraint_options(&[Sel::Point(0), Sel::Point(2)]);
        assert_eq!(labels(&pts), vec!["Dimension", "Coincident", "Fix"], "two points: their distance, together, or fix them");
        assert!(matches!(pts[0].1[0], Constraint::Distance(0, 2, _)));
        assert_eq!(pts[2].1, vec![Constraint::Fix(2)], "origin is already fixed");

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

#[cfg(test)]
mod construction_tests {
    use super::*;

    fn free(x: f64, y: f64) -> Anchor {
        Anchor::Free(DVec2::new(x, y))
    }

    #[test]
    fn perpendicular_guides_through_line_middles_cross_at_the_centre() {
        // A slanted square: the perpendiculars through two neighbouring
        // sides' middles cross at its centre.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        let c = [DVec2::new(10.0, 0.0), DVec2::new(20.0, 10.0), DVec2::new(10.0, 20.0), DVec2::new(0.0, 10.0)];
        for q in c {
            ed.click(&mut sk, Anchor::Free(q));
        }
        ed.click(&mut sk, Anchor::Point(0));
        let side = |m: DVec2| sk.middle_at(m, 0.1).expect("a side's middle");
        let (s1, s2) = (side(DVec2::new(15.0, 5.0)), side(DVec2::new(15.0, 15.0)));
        assert_eq!(sk.middle_at(DVec2::new(12.0, 3.0), 0.1), None, "not at a middle");
        // One guide: near it, the cursor goes onto it.
        let (q, from) = sk.on_middle_guides(&[s1], DVec2::new(10.3, 10.0), 0.5).expect("near the guide");
        assert!(q.distance(DVec2::new(10.15, 9.85)) < 1e-9, "{q}");
        assert_eq!(from, vec![DVec2::new(15.0, 5.0)]);
        assert!(sk.on_middle_guides(&[s1], DVec2::new(14.0, 10.0), 0.5).is_none(), "far from it");
        // Two: near their crossing, right onto the centre.
        let (q, from) = sk.on_middle_guides(&[s1, s2], DVec2::new(10.4, 9.7), 0.5).expect("near the crossing");
        assert!(q.distance(DVec2::new(10.0, 10.0)) < 1e-9, "{q}");
        assert_eq!(from.len(), 2);
    }

    #[test]
    fn a_point_and_a_line_offer_a_midpoint_constraint() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(0.0, 0.0), DVec2::new(10.0, 0.0), DVec2::new(3.0, 4.0)]);
        sk.entities.push(Entity::Line { a: 1, b: 2 });
        let opts = sk.constraint_options(&[Sel::Entity(0), Sel::Point(3)]);
        assert!(opts.contains(&("Midpoint", vec![Constraint::Midpoint(3, 1, 2)])), "{opts:?}");
        // Its own end point is no midpoint candidate.
        assert!(sk.constraint_options(&[Sel::Entity(0), Sel::Point(1)]).iter().all(|(n, _)| *n != "Midpoint"));
        // Solved, the point sits halfway between the line's ends (which are free to move too).
        let mut t = sk.clone();
        crate::solver::add_constraints(&mut t, &[Constraint::Midpoint(3, 1, 2)]).unwrap();
        assert!(t.points[3].distance((t.points[1] + t.points[2]) / 2.0) < 1e-6, "{:?}", t.points);
    }

    #[test]
    fn a_point_dropped_on_a_lines_middle_gets_a_midpoint_constraint() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(10.0, 0.0)));
        ed.cancel();
        // A second line from the first one's middle: Midpoint, not OnLine.
        ed.click(&mut sk, Anchor::OnLine(0, DVec2::new(5.0, 0.0)));
        let ties = ed.click(&mut sk, Anchor::Free(DVec2::new(5.0, 8.0)));
        assert!(ties.iter().any(|c| matches!(c, Constraint::Midpoint(..))), "{ties:?}");
        assert!(!ties.iter().any(|c| matches!(c, Constraint::OnLine(..))));
        ed.cancel();
        // Elsewhere along the line it's still OnLine.
        ed.click(&mut sk, Anchor::OnLine(0, DVec2::new(3.0, 0.0)));
        let ties = ed.click(&mut sk, Anchor::Free(DVec2::new(3.0, 8.0)));
        assert!(ties.iter().any(|c| matches!(c, Constraint::OnLine(..))));
    }

    #[test]
    fn quarter_turns_lock_only_when_close() {
        let c = DVec2::new(10.0, 5.0);
        let near = |deg: f64, r: f64| c + DVec2::from_angle(deg.to_radians()) * r;
        for (deg, want) in [(1.5, 0.0), (88.0, 90.0), (-2.0, 0.0), (181.0, 180.0), (272.5, 270.0), (359.0, 360.0)] {
            let got = snap_quarter_turn(c, near(deg, 7.0), 3.0);
            assert!(got.distance(near(want, 7.0)) < 1e-9, "{deg}: {got:?}");
        }
        // Far from a quarter turn: untouched.
        let free = near(40.0, 7.0);
        assert_eq!(snap_quarter_turn(c, free, 3.0), free);
        assert_eq!(snap_quarter_turn(c, c, 3.0), c);
    }

    #[test]
    fn connected_follows_shared_points_only() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        // A path 1-2-3 (two lines sharing point 2) and a separate line 4-5.
        sk.points.extend([DVec2::new(0.0, 5.0), DVec2::new(5.0, 5.0), DVec2::new(5.0, 9.0), DVec2::new(20.0, 20.0), DVec2::new(30.0, 20.0)]);
        sk.entities.push(Entity::Line { a: 1, b: 2 });
        sk.entities.push(Entity::Line { a: 2, b: 3 });
        sk.entities.push(Entity::Line { a: 4, b: 5 });
        let chain = sk.connected(Sel::Entity(0));
        assert_eq!(chain, vec![Sel::Entity(0), Sel::Entity(1), Sel::Point(1), Sel::Point(2), Sel::Point(3)]);
        assert_eq!(sk.connected(Sel::Point(3)), chain, "from a point too");
        assert_eq!(sk.connected(Sel::Entity(2)), vec![Sel::Entity(2), Sel::Point(4), Sel::Point(5)]);
        // Things tied to the origin don't all become one chain.
        sk.entities.push(Entity::Line { a: 0, b: 1 });
        sk.entities.push(Entity::Line { a: 0, b: 4 });
        let c = sk.connected(Sel::Entity(0));
        assert!(!c.contains(&Sel::Entity(2)) && !c.contains(&Sel::Entity(4)), "{c:?}");
    }

    #[test]
    fn construction_lines_are_dashed_data_that_follow_deletes() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for (a, b) in [((0.0, 0.0), (10.0, 0.0)), ((0.0, 5.0), (10.0, 5.0)), ((0.0, 9.0), (10.0, 9.0))] {
            ed.cancel();
            ed.click(&mut sk, free(a.0, a.1));
            ed.click(&mut sk, free(b.0, b.1));
        }
        sk.toggle_construction(&[Sel::Entity(1)]);
        assert!(sk.is_construction(1) && !sk.is_construction(0) && !sk.is_construction(2));
        sk.delete(&[Sel::Entity(0)]);
        assert!(sk.is_construction(0) && !sk.is_construction(1), "flag moved with its line");
        sk.toggle_construction(&[Sel::Entity(0), Sel::Entity(1)]);
        assert!(sk.is_construction(0) && sk.is_construction(1), "mixed selection → all construction");
        sk.toggle_construction(&[Sel::Entity(0), Sel::Entity(1)]);
        assert!(sk.construction.is_empty(), "all normal again, nothing stored");
    }

    #[test]
    fn construction_mode_marks_new_geometry_and_regions_ignore_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(20.0, 10.0));
        assert_eq!(crate::profile::find_regions(&sk).len(), 1);
        ed.construction = true;
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, free(30.0, 0.0));
        ed.click(&mut sk, free(50.0, 10.0));
        assert_eq!((0..4).filter(|&e| sk.is_construction(e)).count(), 0);
        assert_eq!((4..8).filter(|&e| sk.is_construction(e)).count(), 4);
        assert_eq!(crate::profile::find_regions(&sk).len(), 1, "construction rectangle isn't a profile");
    }

    #[test]
    fn center_rectangle_gets_construction_diagonals_that_dont_split_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::RectangleCenter);
        ed.click(&mut sk, free(0.0, 0.0));
        ed.click(&mut sk, free(10.0, 5.0));
        let diagonals: Vec<_> = (0..sk.entities.len()).filter(|&e| sk.is_construction(e)).collect();
        assert_eq!(diagonals.len(), 2);
        let regions = crate::profile::find_regions(&sk);
        assert_eq!(regions.len(), 1);
        assert!((regions[0].area() - 200.0).abs() < 1e-9);
    }
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    fn seg(sk: &mut Sketch, a: (f64, f64), b: (f64, f64)) -> EntityId {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(sk, Anchor::Free(DVec2::new(a.0, a.1)));
        ed.click(sk, Anchor::Free(DVec2::new(b.0, b.1)));
        sk.entities.len() - 1
    }

    fn ends(sk: &Sketch, e: EntityId) -> (DVec2, DVec2) {
        let Entity::Line { a, b } = sk.entities[e] else { panic!("not a line") };
        (sk.points[a], sk.points[b])
    }

    #[test]
    fn trimming_an_overhang_ends_the_line_at_the_crossing_line() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = seg(&mut sk, (0.0, 0.0), (30.0, 0.0));
        let m = seg(&mut sk, (10.0, -5.0), (10.0, 5.0));
        let cs = sk.trim_line(l, DVec2::new(20.0, 0.0)).unwrap();
        let (a, b) = ends(&sk, l);
        assert_eq!((a, b), (DVec2::new(0.0, 0.0), DVec2::new(10.0, 0.0)));
        // The new end is tied onto the crossing line.
        let Entity::Line { b: end, .. } = sk.entities[l] else { unreachable!() };
        assert_eq!(cs, vec![Constraint::OnLine(end, m)]);
        assert!(crate::solver::add_constraints(&mut sk, &cs).is_ok());
        assert_eq!(sk.points.len(), 5, "the far end point went away: origin + 4 left");
    }

    #[test]
    fn trimming_where_another_line_ends_joins_the_two_lines() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = seg(&mut sk, (0.0, 0.0), (30.0, 0.0));
        let m = seg(&mut sk, (10.0, 0.0), (10.0, 10.0));
        let cs = sk.trim_line(l, DVec2::new(25.0, 0.0)).unwrap();
        assert!(cs.is_empty(), "no tie needed, they share the point");
        let (Entity::Line { b: lb, .. }, Entity::Line { a: ma, .. }) = (sk.entities[l], sk.entities[m]) else { panic!() };
        assert_eq!(lb, ma, "both lines end at the same point");
        assert_eq!(ends(&sk, l).1, DVec2::new(10.0, 0.0));
    }

    #[test]
    fn trimming_between_two_cuts_splits_the_line_and_keeps_horizontal() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = seg(&mut sk, (0.0, 0.0), (30.0, 0.0));
        seg(&mut sk, (10.0, -5.0), (10.0, 5.0));
        seg(&mut sk, (20.0, -5.0), (20.0, 5.0));
        sk.constraints.push(Constraint::Horizontal(l));
        sk.constraints.push(Constraint::Length(l, 30.0));
        let cs = sk.trim_line(l, DVec2::new(15.0, 0.0)).unwrap();
        assert_eq!(ends(&sk, l), (DVec2::ZERO, DVec2::new(10.0, 0.0)));
        let n = sk.entities.len() - 1;
        assert_eq!(ends(&sk, n), (DVec2::new(20.0, 0.0), DVec2::new(30.0, 0.0)));
        assert!(cs.contains(&Constraint::Horizontal(n)), "{cs:?}");
        assert!(!sk.constraints.iter().any(|c| matches!(c, Constraint::Length(..))), "old length dropped");
    }

    #[test]
    fn a_dimensions_own_unit_stays_with_it() {
        use crate::units::Unit;
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = seg(&mut sk, (0.0, 0.0), (25.4, 0.0));
        let m = seg(&mut sk, (0.0, 5.0), (10.0, 5.0));
        sk.constraints.extend([Constraint::Horizontal(l), Constraint::Length(l, 25.4), Constraint::Length(m, 10.0)]);
        sk.set_dim_unit(1, Some(Unit::In));
        assert_eq!((sk.dim_unit(1), sk.dim_unit(2)), (Some(Unit::In), None));
        // Deleting an earlier constraint shifts it along with its dimension.
        sk.retain_constraints(|c| !matches!(c, Constraint::Horizontal(_)));
        assert_eq!((sk.dim_unit(0), sk.dim_unit(1)), (Some(Unit::In), None));
        sk.set_dim_unit(0, None);
        assert!(sk.dim_units.is_empty(), "nothing stored when every dimension follows the design");
    }

    #[test]
    fn the_cursor_lines_up_with_middles_and_centres() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(40.0, 20.0)));
        // Near the middle of the rectangle: both middles (x 20 from top/bottom, y 10 from the sides).
        let (q, above, beside) = sk.line_up(DVec2::new(20.3, 9.8), 0.5);
        assert_eq!(q, DVec2::new(20.0, 10.0));
        assert!(above.is_some_and(|r| r.x == 20.0) && beside.is_some_and(|r| r.y == 10.0));
        // Lined up one way only.
        let (q, above, beside) = sk.line_up(DVec2::new(20.2, 5.0), 0.5);
        assert_eq!((q, beside), (DVec2::new(20.0, 5.0), None));
        assert!(above.is_some());
        // Outside the rectangle: no lining up with its sides' middles.
        assert_eq!(sk.line_up(DVec2::new(20.2, 30.0), 0.5), (DVec2::new(20.2, 30.0), None, None));
        assert_eq!(sk.line_up(DVec2::new(40.2, 50.0), 0.5), (DVec2::new(40.2, 50.0), None, None));
        // Nowhere near: unchanged.
        assert_eq!(sk.line_up(DVec2::new(13.0, 4.0), 0.5), (DVec2::new(13.0, 4.0), None, None));
    }

    #[test]
    fn selection_details_for_lines_circles_and_closed_shapes() {
        use std::f64::consts::PI;
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(40.0, 20.0)));
        let rect: Vec<Sel> = (0..4).map(Sel::Entity).collect();
        let info = sk.selection_info(&rect);
        let get = |info: &[(&str, f64, Amount)], k: &str| info.iter().find(|r| r.0 == k).map(|r| r.1);
        assert_eq!(get(&info, "Total length"), Some(120.0));
        assert!((get(&info, "Area").unwrap() - 800.0).abs() < 1e-9 && (get(&info, "Perimeter").unwrap() - 120.0).abs() < 1e-9);
        // Three sides: open, no area.
        assert_eq!(get(&sk.selection_info(&rect[..3]), "Area"), None);
        assert_eq!(get(&sk.selection_info(&rect[..1]), "Length"), Some(40.0));
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(100.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(105.0, 0.0)));
        let c = sk.selection_info(&[Sel::Entity(sk.entities.len() - 1)]);
        assert_eq!(c.iter().map(|r| r.0).collect::<Vec<_>>(), ["Diameter", "Circumference", "Area"]);
        assert!((c[2].1 - 25.0 * PI).abs() < 1e-9 && c[2].2 == Amount::Area);
        // A slot (lines and half circles): 20 × 6 plus a circle of radius 3.
        let mut slot = Sketch::new("S".into(), OriginPlane::XY);
        let mut se = Editor::default();
        se.set_tool(Tool::Slot);
        se.click(&mut slot, Anchor::Free(DVec2::new(0.0, 10.0)));
        se.click(&mut slot, Anchor::Free(DVec2::new(20.0, 10.0)));
        se.click(&mut slot, Anchor::Free(DVec2::new(5.0, 13.0)));
        let sides: Vec<Sel> = (0..4).map(Sel::Entity).collect();
        let a = get(&slot.selection_info(&sides), "Area").unwrap();
        assert!((a - (120.0 + 9.0 * PI)).abs() < 1e-9, "{a}");
        let d = sk.selection_info(&[Sel::Point(1), Sel::Point(3)]);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].0, "Distance");
        assert!((d[0].1 - sk.points[1].distance(sk.points[3])).abs() < 1e-12);
    }

    #[test]
    fn trim_preview_shows_the_stretch_a_click_removes() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = seg(&mut sk, (0.0, 0.0), (30.0, 0.0));
        seg(&mut sk, (10.0, -5.0), (10.0, 5.0));
        seg(&mut sk, (20.0, -5.0), (20.0, 5.0));
        assert_eq!(sk.trim_preview(l, DVec2::new(15.0, 1.0)), Some((DVec2::new(10.0, 0.0), DVec2::new(20.0, 0.0))));
        assert_eq!(sk.trim_preview(l, DVec2::new(25.0, 0.0)), Some((DVec2::new(20.0, 0.0), DVec2::new(30.0, 0.0))));
        // It's what trimming really takes: the kept pieces are the rest.
        let before = sk.clone();
        sk.trim_line(l, DVec2::new(25.0, 0.0)).unwrap();
        assert_eq!(ends(&sk, l), (DVec2::ZERO, DVec2::new(20.0, 0.0)));
        // Nothing crossing: the whole line.
        let mut lone = Sketch::new("S".into(), OriginPlane::XY);
        let l2 = seg(&mut lone, (0.0, 0.0), (8.0, 0.0));
        assert_eq!(lone.trim_preview(l2, DVec2::new(3.0, 0.0)), Some((DVec2::ZERO, DVec2::new(8.0, 0.0))));
        assert_eq!(before.trim_preview(99, DVec2::ZERO), None);
    }

    #[test]
    fn trimming_a_line_nothing_crosses_deletes_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = seg(&mut sk, (0.0, 0.0), (30.0, 0.0));
        seg(&mut sk, (0.0, 5.0), (30.0, 5.0));
        sk.trim_line(l, DVec2::new(5.0, 0.0)).unwrap();
        assert_eq!(sk.entities.len(), 1);
    }

    #[test]
    fn a_circle_cuts_a_line_too_and_the_end_goes_on_the_circle() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let l = seg(&mut sk, (-30.0, 0.0), (30.0, 0.0));
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(10.0, 0.0)));
        let circle = sk.entities.len() - 1;
        let cs = sk.trim_line(l, DVec2::new(20.0, 0.0)).unwrap();
        assert!((ends(&sk, l).1 - DVec2::new(10.0, 0.0)).length() < 1e-9);
        assert!(cs.iter().any(|c| matches!(c, Constraint::OnCircle(_, e) if *e == circle)), "{cs:?}");
    }

    #[test]
    fn a_circle_cut_twice_trims_to_the_arc_left() {
        use std::f64::consts::PI;
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(10.0, 0.0)));
        let c = sk.entities.len() - 1;
        sk.constraints.push(Constraint::Diameter(c, 20.0));
        seg(&mut sk, (-15.0, 0.0), (15.0, 0.0)); // across the middle
        // The preview: the upper half, from (10, 0) round to (-10, 0).
        let Some(TrimPiece::Arc(_, r, a0, sweep)) = sk.trim_piece(c, DVec2::new(0.0, 9.0)) else { panic!("an arc piece") };
        assert!((r - 10.0).abs() < 1e-9 && a0.abs() < 1e-9 && (sweep - PI).abs() < 1e-9, "{a0} {sweep}");
        let cs = sk.trim(c, DVec2::new(0.0, 9.0)).unwrap();
        // What's left: the lower half, an arc, tied onto the line at both ends.
        let (centre, r, a0, sweep) = sk.arc_geometry(c).expect("now an arc");
        assert!(centre.length() < 1e-9 && (r - 10.0).abs() < 1e-9 && (a0 - PI).abs() < 1e-9 && (sweep - PI).abs() < 1e-9, "{a0} {sweep}");
        assert!(sk.constraints.contains(&Constraint::Radius(c, 10.0)), "the diameter became a radius");
        assert_eq!(cs.iter().filter(|k| matches!(k, Constraint::OnLine(..))).count(), 2);
        // A circle nothing crosses is deleted.
        let mut lone = Sketch::new("S".into(), OriginPlane::XY);
        lone.points.push(DVec2::new(50.0, 50.0));
        lone.entities.push(Entity::Circle { center: 1, radius: 3.0 });
        lone.trim(0, DVec2::new(53.0, 50.0)).unwrap();
        assert!(lone.entities.is_empty());
    }

    #[test]
    fn an_arc_trims_at_an_end_or_splits_in_the_middle() {
        use std::f64::consts::PI;
        // A half circle over the top (radius 10), crossed by two vertical lines at x = ±5.
        let arc = || {
            let mut sk = Sketch::new("S".into(), OriginPlane::XY);
            sk.points.extend([DVec2::new(0.0, 0.0), DVec2::new(10.0, 0.0), DVec2::new(-10.0, 0.0)]);
            sk.entities.push(Entity::Arc { center: 1, a: 2, b: 3 });
            seg(&mut sk, (5.0, -1.0), (5.0, 20.0));
            seg(&mut sk, (-5.0, -1.0), (-5.0, 20.0));
            sk
        };
        // The right end piece (x > 5): the arc now starts at x = 5.
        let mut sk = arc();
        sk.trim(0, DVec2::new(9.0, 4.0)).unwrap();
        let (_, _, a0, sweep) = sk.arc_geometry(0).unwrap();
        assert!((a0 - PI / 3.0).abs() < 1e-9 && (sweep - 2.0 * PI / 3.0).abs() < 1e-9, "{a0} {sweep}");
        // The middle piece: two arcs, kept equal.
        let mut sk = arc();
        let cs = sk.trim(0, DVec2::new(0.0, 10.0)).unwrap();
        let arcs: Vec<usize> = (0..sk.entities.len()).filter(|&e| sk.is_arc(e)).collect();
        assert_eq!(arcs.len(), 2);
        let total: f64 = arcs.iter().map(|&e| sk.arc_geometry(e).unwrap().3).sum();
        assert!((total - 2.0 * PI / 3.0).abs() < 1e-9, "two 60° arcs left: {total}");
        assert!(cs.iter().any(|k| matches!(k, Constraint::Equal(..))));
    }

    #[test]
    fn the_spline_tool_draws_through_its_points_and_deleting_one_shortens_it() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Spline);
        for (x, y) in [(0.0, 0.0), (10.0, 6.0), (20.0, -3.0), (30.0, 2.0)] {
            assert!(ed.click(&mut sk, Anchor::Free(DVec2::new(x, y))).is_empty());
        }
        assert!(sk.entities.is_empty(), "not made until finished");
        // Clicking the last point again finishes it.
        ed.click(&mut sk, Anchor::Free(DVec2::new(30.0, 2.0)));
        assert_eq!(sk.entities, vec![Entity::Spline { s: 0 }]);
        assert_eq!(sk.splines[0].len(), 4);
        assert!(!ed.busy());
        let curve = sk.spline_curve(0);
        assert!(curve[0].distance(DVec2::ZERO) < 1e-9 && curve.last().unwrap().distance(DVec2::new(30.0, 2.0)) < 1e-9);
        crate::solver::solve(&mut sk.clone(), &[]).unwrap();
        // The second fit point deleted: three left, still a spline.
        let p = sk.splines[0][1];
        sk.delete(&[Sel::Point(p)]);
        assert_eq!(sk.splines.len(), 1);
        assert_eq!(sk.splines[0].len(), 3);
        assert!(sk.splines[0].iter().all(|&q| q < sk.points.len()));
        // Down to one point: the spline goes.
        let (a, b) = (sk.splines[0][0], sk.splines[0][1]);
        sk.delete(&[Sel::Point(a), Sel::Point(b)]);
        assert!(sk.entities.is_empty() && sk.splines.is_empty());
        // Enter-style finish with one point makes nothing.
        ed.click(&mut sk, Anchor::Free(DVec2::new(5.0, 5.0)));
        assert!(ed.finish_spline(&mut sk).is_empty() && sk.entities.is_empty());
    }

    #[test]
    fn trim_line_only_works_on_lines() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 0.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(10.0, 0.0)));
        assert!(sk.trim_line(0, DVec2::ZERO).is_err(), "(circles go through trim / trim_round)");
    }
}

#[cfg(test)]
mod polygon_tests {
    use super::*;
    use crate::solver;

    fn free(x: f64, y: f64) -> Anchor {
        Anchor::Free(DVec2::new(x, y))
    }

    /// Make a polygon the way the app does: clicks, then each constraint
    /// through the solver, skipping redundant ones.
    fn make(tool: Tool, sides: usize, clicks: &[Anchor]) -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor { polygon_sides: sides, ..Default::default() };
        ed.set_tool(tool);
        let mut cs = Vec::new();
        for &c in clicks {
            cs.extend(ed.click(&mut sk, c));
        }
        for c in cs {
            let mut t = sk.clone();
            if solver::add_constraints(&mut t, &[c]).is_ok() {
                sk = t;
            }
        }
        sk
    }

    fn side_lengths(sk: &Sketch) -> Vec<f64> {
        (0..sk.entities.len()).filter(|&e| sk.is_line(e)).map(|e| sk.line_length(e).unwrap()).collect()
    }

    fn regular(sk: &Sketch, n: usize) {
        let l = side_lengths(sk);
        assert_eq!(l.len(), n);
        assert!(l.iter().all(|x| (x - l[0]).abs() < 1e-6), "{l:?}");
        let circle = (0..sk.entities.len()).find(|&e| matches!(sk.entities[e], Entity::Circle { .. })).unwrap();
        assert!(sk.is_construction(circle), "the reference circle is construction");
    }

    #[test]
    fn inscribed_hexagon_has_corners_on_the_circle_and_three_degrees_of_freedom() {
        let mut sk = make(Tool::PolygonInscribed, 6, &[free(0.0, 0.0), free(10.0, 0.0)]);
        regular(&sk, 6);
        let Entity::Circle { center, radius } = sk.entities.iter().copied().find(|e| matches!(e, Entity::Circle { .. })).unwrap() else { panic!() };
        assert!((radius - 10.0).abs() < 1e-9);
        assert!(side_lengths(&sk).iter().all(|x| (x - 10.0).abs() < 1e-6), "hexagon side = radius");
        assert!(sk.points[center].length() < 1e-9);
        // centre (2) + size + rotation.
        assert_eq!(solver::solve(&mut sk, &[]).unwrap().dof, 3);
        // The size is a Diameter dimension on the reference circle; editing it keeps the polygon regular.
        let di = sk.constraints.iter().position(|c| matches!(c, Constraint::Diameter(..))).expect("the polygon's size dimension");
        solver::set_dimension_value(&mut sk, di, 40.0).unwrap();
        assert!(side_lengths(&sk).iter().all(|x| (x - 20.0).abs() < 1e-5), "{:?}", side_lengths(&sk));
    }

    #[test]
    fn circumscribed_polygon_has_edges_touching_the_circle() {
        let mut sk = make(Tool::PolygonCircumscribed, 5, &[free(0.0, 0.0), free(8.0, 0.0)]);
        regular(&sk, 5);
        assert_eq!(solver::solve(&mut sk, &[]).unwrap().dof, 3);
        let apothem = 8.0;
        for e in 0..sk.entities.len() {
            if sk.is_line(e) {
                assert!((sk.line_distance(e, DVec2::ZERO).unwrap() - apothem).abs() < 1e-6);
            }
        }
        // The edge midpoint was the click: the polygon has an edge centred at (8, 0).
        let mid_hit = (0..sk.entities.len()).filter(|&e| sk.is_line(e)).any(|e| {
            let Entity::Line { a, b } = sk.entities[e] else { unreachable!() };
            ((sk.points[a] + sk.points[b]) * 0.5).distance(DVec2::new(8.0, 0.0)) < 1e-6
        });
        assert!(mid_hit);
    }

    #[test]
    fn edge_polygon_uses_the_clicked_edge_and_builds_on_the_cursor_side() {
        for (side_y, expect_above) in [(5.0, true), (-5.0, false)] {
            let sk = make(Tool::PolygonEdge, 6, &[free(0.0, 0.0), free(10.0, 0.0), free(5.0, side_y)]);
            regular(&sk, 6);
            assert!(side_lengths(&sk).iter().all(|x| (x - 10.0).abs() < 1e-6));
            let ys: Vec<f64> = sk.points.iter().map(|p| p.y).collect();
            let far = ys.iter().cloned().fold(0.0f64, |m, y| if y.abs() > m.abs() { y } else { m });
            assert_eq!(far > 0.0, expect_above, "{ys:?}");
            assert!((far.abs() - 10.0 * 3f64.sqrt()).abs() < 1e-6, "hexagon with side 10 is 17.32 tall");
        }
    }

    #[test]
    fn every_polygon_is_rigid_up_to_centre_and_turn_and_survives_dragging_a_corner() {
        for tool in [Tool::PolygonInscribed, Tool::PolygonCircumscribed] {
            for n in [3, 4, 5, 6, 8, 12] {
                let mut sk = make(tool, n, &[free(20.0, 20.0), free(30.0, 20.0)]);
                assert_eq!(solver::solve(&mut sk, &[]).unwrap().dof, 3, "{tool:?} n = {n}");
                let corner = sk.entities.iter().find_map(|e| match *e { Entity::Line { a, .. } => Some(a), _ => None }).unwrap();
                for target in [DVec2::new(32.0, 24.0), DVec2::new(40.0, 40.0), DVec2::new(20.0, 20.0), DVec2::new(100.0, 0.0), DVec2::new(-5.0, 60.0)] {
                    let _ = solver::drag(&mut sk, corner, target);
                    let l = side_lengths(&sk);
                    assert!(l.iter().all(|x| (x - l[0]).abs() < 1e-5), "{tool:?} n = {n} after {target:?}: {l:?}");
                    assert!(l[0] > 1.0, "{tool:?} n = {n} collapsed after {target:?}: {l:?}");
                }
            }
        }
    }

    #[test]
    fn side_count_is_clamped_and_defaults_to_six() {
        let mut ed = Editor::default();
        assert_eq!(ed.sides(), 6);
        ed.polygon_sides = 2;
        assert_eq!(ed.sides(), 6);
        ed.polygon_sides = 500;
        assert_eq!(ed.sides(), 64);
        ed.polygon_sides = 3;
        assert_eq!(ed.sides(), 3);
    }

    #[test]
    fn a_triangle_and_a_many_sided_polygon_both_solve() {
        for n in [3, 12, 40] {
            let mut sk = make(Tool::PolygonInscribed, n, &[free(1.0, 2.0), free(11.0, 2.0)]);
            regular(&sk, n);
            assert_eq!(solver::solve(&mut sk, &[]).unwrap().dof, 3, "n = {n}");
        }
    }

    #[test]
    fn polygon_makes_a_single_extrudable_region() {
        let sk = make(Tool::PolygonInscribed, 6, &[free(0.0, 0.0), free(10.0, 0.0)]);
        let regions = crate::profile::find_regions(&sk);
        assert_eq!(regions.len(), 1, "the construction circle doesn't make a second profile");
        let want = 3.0 * 3f64.sqrt() / 2.0 * 100.0;
        assert!((regions[0].area() - want).abs() < 1e-6, "{}", regions[0].area());
    }
}

#[cfg(test)]
mod corner_tests {
    use super::*;

    /// An L: (0,0)–(30,0) and (0,0)–(0,30) meeting at point `corner`.
    fn ell() -> (Sketch, PointId, EntityId, EntityId) {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for (x, y) in [(30.0, 0.0), (0.0, 0.0), (0.0, 30.0)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(x, y)));
        }
        // Point 0 is the sketch origin, a separate point: find the one the lines share.
        let shared = (1..sk.points.len()).find(|&p| sk.corner_lines(p).is_some()).unwrap();
        (sk, shared, 0, 1)
    }

    #[test]
    fn chamfer_with_two_distances_cuts_each_line_by_its_own_amount() {
        let (mut sk, p, l_first, l_second) = ell();
        let cs = sk.chamfer_corner2(p, 2.0, 5.0, Some(l_first)).unwrap();
        // d1 = 2 on the first line (the x axis one), d2 = 5 on the second.
        let ends = |sk: &Sketch, l: EntityId| {
            let Entity::Line { a, b } = sk.entities[l] else { panic!() };
            [sk.points[a], sk.points[b]]
        };
        let on_first = ends(&sk, l_first).iter().any(|q| q.distance(DVec2::new(2.0, 0.0)) < 1e-9);
        let on_second = ends(&sk, l_second).iter().any(|q| q.distance(DVec2::new(0.0, 5.0)) < 1e-9);
        assert!(on_first && on_second);
        assert!(cs.iter().any(|c| matches!(c, Constraint::Distance(_, _, d) if (*d - 2.0).abs() < 1e-12)));
        assert!(cs.iter().any(|c| matches!(c, Constraint::Distance(_, _, d) if (*d - 5.0).abs() < 1e-12)));
        assert!(crate::solver::add_constraints(&mut sk, &cs).is_ok());
    }

    #[test]
    fn first_line_choice_swaps_which_distance_goes_where() {
        let (mut sk, p, l_first, l_second) = ell();
        sk.chamfer_corner2(p, 2.0, 5.0, Some(l_second)).unwrap();
        let Entity::Line { a, b } = sk.entities[l_first] else { panic!() };
        let end = [sk.points[a], sk.points[b]].into_iter().find(|q| q.x > 0.0 && q.x < 30.0).unwrap();
        assert!((end.x - 5.0).abs() < 1e-9, "the first line lost 5, the second 2: {end:?}");
    }

    #[test]
    fn equal_chamfer_is_unchanged_and_too_big_is_refused() {
        let (mut sk, p, ..) = ell();
        assert!(sk.clone().chamfer_corner2(p, 4.0, 40.0, None).is_err(), "second side is only 30 long");
        assert!(sk.clone().chamfer_corner2(p, 0.0, 4.0, None).is_err());
        let cs = sk.chamfer_corner2(p, 3.0, 3.0, None).unwrap();
        assert_eq!(cs.iter().filter(|c| matches!(c, Constraint::Distance(..))).count(), 2);
    }

    #[test]
    fn corner_cut_describes_the_preview() {
        let (sk, p, l1, _) = ell();
        let c = sk.corner_cut(p, false, 2.0, 5.0, Some(l1)).unwrap();
        assert!((c.a - DVec2::new(2.0, 0.0)).length() < 1e-9 && (c.b - DVec2::new(0.0, 5.0)).length() < 1e-9);
        let f = sk.corner_cut(p, true, 4.0, 0.0, None).unwrap();
        let (center, r, a0, sweep) = f.arc.unwrap();
        assert!((center - DVec2::new(4.0, 4.0)).length() < 1e-9 && (r - 4.0).abs() < 1e-12);
        assert!((sweep - std::f64::consts::FRAC_PI_2).abs() < 1e-9, "a right-angle corner rounds through 90°");
        // Counter-clockwise from the point on the vertical line, (0, 4) at 180°, round the corner.
        assert!((a0.abs() - std::f64::consts::PI).abs() < 1e-9, "{a0}");
        let mid = center + DVec2::from_angle(a0 + sweep / 2.0) * r;
        assert!(mid.x < 4.0 && mid.y < 4.0, "the arc bulges toward the corner: {mid:?}");
        assert!(sk.corner_cut(p, true, 40.0, 0.0, None).is_none(), "radius too big");
    }

    #[test]
    fn sizes_parse_one_or_two_numbers() {
        let parse_sizes_mm = |t: &str| parse_sizes(t, crate::units::Unit::Mm);
        // In an inch design a bare number is inches; a unit still wins.
        assert_eq!(parse_sizes("1, 2 mm", crate::units::Unit::In), Some((25.4, Some(2.0))));
        assert_eq!(parse_sizes_mm("5"), Some((5.0, None)));
        assert_eq!(parse_sizes_mm(" 2.5 mm "), Some((2.5, None)));
        assert_eq!(parse_sizes_mm("2, 5"), Some((2.0, Some(5.0))));
        assert_eq!(parse_sizes_mm("2 5"), Some((2.0, Some(5.0))));
        assert_eq!(parse_sizes_mm("2;5mm"), Some((2.0, Some(5.0))));
        assert_eq!(parse_sizes_mm("2,5,7"), None);
        assert_eq!(parse_sizes_mm("x"), None);
        assert_eq!(parse_sizes_mm(""), None);
    }
}

#[cfg(test)]
mod marquee_tests {
    use super::*;

    fn seg(sk: &mut Sketch, a: (f64, f64), b: (f64, f64)) {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(sk, Anchor::Free(DVec2::new(a.0, a.1)));
        ed.click(sk, Anchor::Free(DVec2::new(b.0, b.1)));
    }

    fn picture() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        seg(&mut sk, (1.0, 1.0), (4.0, 1.0)); // 0: well inside the box 0..5
        seg(&mut sk, (3.0, 3.0), (9.0, 3.0)); // 1: sticks out of it
        seg(&mut sk, (20.0, 20.0), (30.0, 20.0)); // 2: far away
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(2.0, 2.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(3.0, 2.0))); // 3: r = 1 at (2,2)
        ed.set_tool(Tool::Point);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.5, 4.5))); // 4: a lone point
        sk
    }

    #[test]
    fn window_selection_takes_only_what_is_completely_inside() {
        let sk = picture();
        let got = sk.select_in_rect(DVec2::new(0.0, 0.0), DVec2::new(5.0, 5.0), false);
        assert!(got.contains(&Sel::Entity(0)) && got.contains(&Sel::Entity(3)));
        assert!(!got.contains(&Sel::Entity(1)), "line 1 pokes out");
        assert!(!got.contains(&Sel::Entity(2)));
        assert!(got.iter().any(|s| matches!(s, Sel::Point(_))), "the lone point");
        // A line's own end points are never selected by themselves.
        assert_eq!(got.iter().filter(|s| matches!(s, Sel::Point(_))).count(), 1);
    }

    #[test]
    fn crossing_selection_takes_whatever_it_touches() {
        let sk = picture();
        let got = sk.select_in_rect(DVec2::new(0.0, 0.0), DVec2::new(5.0, 5.0), true);
        assert!(got.contains(&Sel::Entity(1)), "line 1 enters the box");
        assert!(!got.contains(&Sel::Entity(2)));
        // A thin box that only crosses a line's middle (no end point inside it).
        let thin = sk.select_in_rect(DVec2::new(5.9, 2.0), DVec2::new(6.1, 4.0), true);
        assert_eq!(thin, vec![Sel::Entity(1)]);
        // A circle: its rim crosses a box that doesn't contain its centre.
        let rim = sk.select_in_rect(DVec2::new(2.9, 1.5), DVec2::new(3.5, 2.5), true);
        assert!(rim.contains(&Sel::Entity(3)));
        // A box wholly inside the circle doesn't touch the rim.
        let hole = sk.select_in_rect(DVec2::new(1.8, 1.8), DVec2::new(2.2, 2.2), true);
        assert!(!hole.contains(&Sel::Entity(3)));
    }

    #[test]
    fn corners_can_be_dragged_in_either_direction() {
        let sk = picture();
        let a = sk.select_in_rect(DVec2::new(5.0, 5.0), DVec2::new(0.0, 0.0), false);
        let b = sk.select_in_rect(DVec2::new(0.0, 0.0), DVec2::new(5.0, 5.0), false);
        assert_eq!(a, b);
        assert!(sk.select_in_rect(DVec2::new(100.0, 100.0), DVec2::new(110.0, 110.0), true).is_empty());
        // Points in the box come along too, line ends included.
        let pts = sk.points_in_rect(DVec2::new(-1.0, -1.0), DVec2::new(5.0, 5.0));
        assert!(pts.contains(&Sel::Point(1)) && pts.contains(&Sel::Point(2)), "both ends of the line inside");
        assert!(pts.contains(&Sel::Point(3)) && !pts.contains(&Sel::Point(4)), "the sticking-out line's near end, not its far end");
        assert!(!pts.contains(&Sel::Point(Sketch::ORIGIN)));
        assert!(sk.points_in_rect(DVec2::new(100.0, 100.0), DVec2::new(110.0, 110.0)).is_empty());
    }

    #[test]
    fn a_slot_is_two_lines_and_two_half_circles_that_stay_tangent() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Slot);
        ed.click(&mut sk, Anchor::Free(DVec2::new(0.0, 10.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(20.0, 10.0)));
        let cs = ed.click(&mut sk, Anchor::Free(DVec2::new(5.0, 13.0)));
        // Two lines, two half circles, and the construction line through the centres (Fusion draws it too).
        assert_eq!(sk.entities.len(), 5);
        assert!(sk.is_construction(4) && matches!(sk.entities[4], Entity::Line { .. }));
        for c in cs {
            let mut t = sk.clone();
            if crate::solver::add_constraints(&mut t, &[c]).is_ok() {
                sk = t;
            }
        }
        // Half circles of radius 3 at the two centres, 180° each.
        for e in 2..4 {
            let (_, r, _, sweep) = sk.arc_geometry(e).unwrap();
            assert!((r - 3.0).abs() < 1e-9 && (sweep - std::f64::consts::PI).abs() < 1e-9, "arc {e}: r {r} sweep {sweep}");
        }
        let regions = crate::profile::find_regions(&sk);
        assert_eq!(regions.len(), 1);
        // The outline is the stadium: 20 × 6 plus the two caps (polygon approximation of the arcs).
        let exact = 20.0 * 6.0 + std::f64::consts::PI * 9.0;
        assert!((regions[0].area() - exact).abs() < 0.01 * exact);
        // Only the two centres are free (the width is a dimension).
        let solved = crate::solver::solve(&mut sk.clone(), &[]).unwrap();
        assert_eq!(solved.dof, 4);
    }
}
