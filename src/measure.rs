//! The Measure tool, as in Fusion's Measure reference: pick one or two things
//! (a corner, an edge, a face, a body, or sketch geometry) and read what each
//! is — position, length, radius, diameter, area, loop length, centre — and
//! how two sit to each other (minimum distance, angle, X/Y/Z delta between
//! corners). Values are in mm (mm², degrees) here; `Unit` turns them into
//! whatever the window shows.

use glam::DVec3;
use std::f64::consts::TAU;

/// Something that was clicked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    /// A corner or a snap point (an edge's middle, a circle's centre).
    Point(DVec3),
    /// `body`'s `edge`th pickable edge (`kernel::Body::edges`).
    Edge { body: usize, edge: usize },
    /// `body`'s `face`th face (`kernel::Body::faces`).
    Face { body: usize, face: usize },
    /// A whole body (the Body selection filter).
    Body { body: usize },
    /// A sketch's point, line / circle / arc, or closed profile.
    SketchPoint { sketch: usize, point: usize },
    SketchCurve { sketch: usize, entity: usize },
    Profile { sketch: usize, region: usize },
}

/// What can be picked (Fusion's Selection Filter; there are no components here).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    FaceEdgeVertex,
    Body,
}

/// What a pick is, exactly (from the kernel or the sketch, not a display mesh).
#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    Point(DVec3),
    Line { a: DVec3, b: DVec3 },
    /// A circle or an arc of one; `sweep` is 2π for a whole circle.
    Circle { center: DVec3, normal: DVec3, radius: f64, sweep: f64 },
    /// Any other curve, by its length.
    Curve { length: f64 },
    /// A flat face or a sketch profile; `center` when it's round.
    Plane { normal: DVec3, area: f64, loop_length: f64, center: Option<DVec3> },
    /// A cylindrical face (or part of one: a fillet).
    Cylinder { axis: DVec3, radius: f64, area: f64, loop_length: f64 },
    Surface { area: f64, loop_length: f64 },
    /// A whole body: its volume, surface area and centre of mass.
    Body { volume: f64, area: f64, center: DVec3 },
}

/// The pieces a pick is made of, for distances: corners, edge segments, face triangles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Samples {
    pub points: Vec<DVec3>,
    pub segments: Vec<[DVec3; 2]>,
    pub triangles: Vec<[DVec3; 3]>,
}

/// A pick with what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Picked {
    pub pick: Pick,
    pub geom: Geom,
    pub samples: Samples,
}

impl Picked {
    /// A corner or snap point.
    pub fn point(p: DVec3) -> Self {
        Picked { pick: Pick::Point(p), geom: Geom::Point(p), samples: Samples { points: vec![p], ..Default::default() } }
    }

    /// What the window calls it.
    pub fn name(&self) -> &'static str {
        let sketch = matches!(self.pick, Pick::SketchPoint { .. } | Pick::SketchCurve { .. });
        match (&self.geom, sketch) {
            (Geom::Point(_), true) => "Sketch point",
            (Geom::Point(_), false) => "Point",
            (Geom::Line { .. }, true) => "Sketch line",
            (Geom::Line { .. }, false) => "Edge (straight)",
            (Geom::Circle { sweep, .. }, true) if *sweep >= TAU - 1e-9 => "Sketch circle",
            (Geom::Circle { .. }, true) => "Sketch arc",
            (Geom::Circle { sweep, .. }, false) if *sweep >= TAU - 1e-9 => "Edge (circle)",
            (Geom::Circle { .. }, false) => "Edge (arc)",
            (Geom::Curve { .. }, _) => "Edge (curve)",
            (Geom::Plane { .. }, _) if matches!(self.pick, Pick::Profile { .. }) => "Profile",
            (Geom::Plane { .. }, _) => "Face (flat)",
            (Geom::Cylinder { .. }, _) => "Face (cylindrical)",
            (Geom::Surface { .. }, _) => "Face",
            (Geom::Body { .. }, _) => "Body",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Length,
    Area,
    Volume,
    Angle,
}

/// Which part of the window a row is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    /// Selection 1 or 2's own readings.
    Selection(usize),
    /// Between the two.
    Results,
}

/// What to light up in the view while the cursor is over a row (Fusion
/// highlights the measurement a result belongs to).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Pick(usize),
    Radius(usize),
    Diameter(usize),
    Centre(usize),
    Distance,
    /// The X (0), Y (1) or Z (2) leg between two corners.
    Delta(usize),
    Angle,
}

/// One line of the Measure window.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: &'static str,
    /// mm, mm² or degrees.
    pub value: f64,
    pub kind: Kind,
    pub section: Section,
    pub show: Show,
}

/// The Measure tool's state: up to two picks, and the readings for them.
#[derive(Clone, Debug, Default)]
pub struct Measure {
    pub picks: Vec<Picked>,
    /// What the cursor is over.
    pub hover: Option<Picked>,
    /// Snap points shown on what the cursor is over.
    pub snaps: Vec<DVec3>,
    pub rows: Vec<Row>,
    /// The closest points between two picks (drawn as a line).
    pub closest: Option<(DVec3, DVec3)>,
    /// The window row the cursor is over.
    pub hover_row: Option<Show>,
    pub filter: Filter,
}

impl Measure {
    /// A click: picking something already picked drops it; a third pick
    /// replaces the second (the first stays, to measure other things from).
    pub fn click(&mut self, p: Picked) {
        if let Some(i) = self.picks.iter().position(|q| q.pick == p.pick) {
            self.picks.remove(i);
        } else {
            self.picks.truncate(1);
            self.picks.push(p);
        }
        self.update();
    }

    /// Drop picks that aren't there any more (the model was rebuilt): `now`
    /// says what a pick is in the current model.
    pub fn keep_valid(&mut self, mut now: impl FnMut(&Pick) -> Option<Geom>) {
        let before = self.picks.len();
        self.picks.retain(|p| now(&p.pick).as_ref() == Some(&p.geom));
        if self.picks.len() != before {
            self.update();
        }
    }

    pub fn clear(&mut self) {
        self.picks.clear();
        self.update();
    }

    fn update(&mut self) {
        self.rows = self.picks.iter().enumerate().flat_map(|(i, p)| describe(&p.geom, i)).collect();
        self.closest = None;
        if let [a, b] = &self.picks[..] {
            let (rows, closest) = compare(a, b);
            self.rows.extend(rows);
            self.closest = closest;
        }
    }
}

/// What one pick measures (Fusion's fields for each kind of selection).
pub fn describe(g: &Geom, i: usize) -> Vec<Row> {
    use Kind::*;
    let row = |label, value, kind, show| Row { label, value, kind, section: Section::Selection(i), show };
    let centre = |c: DVec3| [row("Center X", c.x, Length, Show::Centre(i)), row("Center Y", c.y, Length, Show::Centre(i)), row("Center Z", c.z, Length, Show::Centre(i))];
    match *g {
        Geom::Point(p) => vec![row("X", p.x, Length, Show::Pick(i)), row("Y", p.y, Length, Show::Pick(i)), row("Z", p.z, Length, Show::Pick(i))],
        Geom::Line { a, b } => vec![row("Length", a.distance(b), Length, Show::Pick(i))],
        Geom::Circle { center, radius, sweep, .. } => {
            let whole = sweep >= TAU - 1e-9;
            let mut v = vec![
                row(if whole { "Circumference" } else { "Length" }, sweep.min(TAU) * radius, Length, Show::Pick(i)),
                row("Radius", radius, Length, Show::Radius(i)),
                row("Diameter", 2.0 * radius, Length, Show::Diameter(i)),
            ];
            v.extend(centre(center));
            v
        }
        Geom::Curve { length } => vec![row("Length", length, Length, Show::Pick(i))],
        Geom::Plane { area, loop_length, center, .. } => {
            let mut v = vec![row("Area", area, Area, Show::Pick(i)), row("Loop length", loop_length, Length, Show::Pick(i))];
            if let Some(c) = center {
                v.extend(centre(c));
            }
            v
        }
        Geom::Cylinder { radius, area, loop_length, .. } => vec![
            row("Radius", radius, Length, Show::Pick(i)),
            row("Diameter", 2.0 * radius, Length, Show::Pick(i)),
            row("Area", area, Area, Show::Pick(i)),
            row("Loop length", loop_length, Length, Show::Pick(i)),
        ],
        Geom::Surface { area, loop_length } => vec![row("Area", area, Area, Show::Pick(i)), row("Loop length", loop_length, Length, Show::Pick(i))],
        Geom::Body { volume, area, center } => vec![
            row("Volume", volume, Volume, Show::Pick(i)),
            row("Area", area, Area, Show::Pick(i)),
            row("Center of mass X", center.x, Length, Show::Pick(i)),
            row("Center of mass Y", center.y, Length, Show::Pick(i)),
            row("Center of mass Z", center.z, Length, Show::Pick(i)),
        ],
    }
}

/// The quick readings for what's selected in the model (bottom right, as
/// sketch selections show theirs): an edge's length (a circle's diameter and
/// circumference), a face's area (a cylinder's diameter too). Several: the
/// total length of the edges and area of the faces.
pub fn quick_stats(geoms: &[&Geom]) -> Vec<(&'static str, f64, Kind)> {
    use Kind::*;
    let length = |g: &Geom| match *g {
        Geom::Line { a, b } => Some(a.distance(b)),
        Geom::Circle { radius, sweep, .. } => Some(radius * sweep.min(TAU)),
        Geom::Curve { length } => Some(length),
        _ => None,
    };
    let area = |g: &Geom| match *g {
        Geom::Plane { area, .. } | Geom::Cylinder { area, .. } | Geom::Surface { area, .. } => Some(area),
        _ => None,
    };
    match geoms {
        [] => Vec::new(),
        [g] => match **g {
            Geom::Circle { radius, sweep, .. } if sweep >= TAU - 1e-9 => vec![("Diameter", 2.0 * radius, Length), ("Circumference", TAU * radius, Length)],
            Geom::Circle { radius, sweep, .. } => vec![("Radius", radius, Length), ("Length", radius * sweep, Length)],
            Geom::Cylinder { radius, area, .. } => vec![("Diameter", 2.0 * radius, Length), ("Area", area, Area)],
            ref g => length(g).map(|l| ("Length", l, Length)).into_iter().chain(area(g).map(|a| ("Area", a, Area))).collect(),
        },
        many => {
            let lengths: Vec<f64> = many.iter().filter_map(|g| length(g)).collect();
            let areas: Vec<f64> = many.iter().filter_map(|g| area(g)).collect();
            let mut out = Vec::new();
            if !lengths.is_empty() {
                out.push(("Total length", lengths.iter().sum(), Length));
            }
            if !areas.is_empty() {
                out.push(("Total area", areas.iter().sum(), Area));
            }
            out
        }
    }
}

/// The direction a pick has for angles: a straight edge runs one way, a flat
/// face faces one way, a cylinder or a circle has an axis.
fn direction(g: &Geom) -> Option<(DVec3, bool)> {
    // (unit vector, is it a normal rather than a line's direction)
    match *g {
        Geom::Line { a, b } => (b - a).try_normalize().map(|d| (d, false)),
        Geom::Plane { normal, .. } => normal.try_normalize().map(|n| (n, true)),
        Geom::Cylinder { axis, .. } => axis.try_normalize().map(|d| (d, false)),
        Geom::Circle { normal, .. } => normal.try_normalize().map(|n| (n, true)),
        _ => None,
    }
}

/// The angle between two picks in degrees (0–90): between two lines, two
/// planes, or a line and a plane.
pub fn angle(a: &Geom, b: &Geom) -> Option<f64> {
    let ((da, na), (db, nb)) = (direction(a)?, direction(b)?);
    let c = da.dot(db).abs().min(1.0);
    let between = c.acos().to_degrees();
    Some(if na == nb { between } else { 90.0 - between })
}

/// What two picks measure: the minimum distance between them, the angle when
/// both have a direction, and the X/Y/Z delta between two corners.
pub fn compare(a: &Picked, b: &Picked) -> (Vec<Row>, Option<(DVec3, DVec3)>) {
    use Kind::*;
    let row = |label, value, kind, show| Row { label, value, kind, section: Section::Results, show };
    let mut rows = Vec::new();
    let closest = closest(&a.samples, &b.samples);
    if let Some((p, q)) = closest {
        rows.push(row("Distance", p.distance(q), Length, Show::Distance));
    }
    if let Some(deg) = angle(&a.geom, &b.geom) {
        rows.push(row("Angle", deg, Angle, Show::Angle));
    }
    if let (Geom::Point(p), Geom::Point(q)) = (&a.geom, &b.geom) {
        let d = *q - *p;
        rows.extend([row("ΔX", d.x, Length, Show::Delta(0)), row("ΔY", d.y, Length, Show::Delta(1)), row("ΔZ", d.z, Length, Show::Delta(2))]);
    }
    // Two circles: how far apart their centres are too.
    if let (Geom::Circle { center: c1, .. }, Geom::Circle { center: c2, .. }) = (&a.geom, &b.geom) {
        rows.push(row("Center distance", c1.distance(*c2), Length, Show::Distance));
    }
    (rows, closest)
}

/// One piece of a pick, for distances.
#[derive(Clone, Copy)]
enum Prim {
    P(DVec3),
    S([DVec3; 2]),
    T([DVec3; 3]),
}

impl Prim {
    fn corners(&self) -> &[DVec3] {
        match self {
            Prim::P(p) => std::slice::from_ref(p),
            Prim::S(s) => s,
            Prim::T(t) => t,
        }
    }

    fn bounds(&self) -> (DVec3, DVec3) {
        let c = self.corners();
        c.iter().fold((c[0], c[0]), |(lo, hi), &p| (lo.min(p), hi.max(p)))
    }

    fn edges(&self) -> Vec<[DVec3; 2]> {
        match *self {
            Prim::P(_) => Vec::new(),
            Prim::S(s) => vec![s],
            Prim::T([a, b, c]) => vec![[a, b], [b, c], [c, a]],
        }
    }
}

/// The closest points of two pieces (assuming they don't cut through each
/// other — true of distinct faces and edges of solids).
fn prim_closest(a: Prim, b: Prim) -> (DVec3, DVec3) {
    let mut best: Option<(f64, DVec3, DVec3)> = None;
    let mut offer = |p: DVec3, q: DVec3| {
        let d = p.distance_squared(q);
        if best.is_none_or(|(bd, ..)| d < bd) {
            best = Some((d, p, q));
        }
    };
    let near = |p: DVec3, x: Prim| match x {
        Prim::P(q) => q,
        Prim::S(s) => point_segment(p, s),
        Prim::T(t) => point_triangle(p, t),
    };
    for &p in a.corners() {
        offer(p, near(p, b));
    }
    for &q in b.corners() {
        offer(near(q, a), q);
    }
    for s in a.edges() {
        for u in b.edges() {
            let (p, q) = segment_segment(s, u);
            offer(p, q);
        }
    }
    let (_, p, q) = best.expect("pieces have corners");
    (p, q)
}

/// The closest pair of points between two sets of pieces (None if either is
/// empty). Pairs whose boxes are already further apart than the best so far
/// are skipped, so two big bodies measure quickly.
pub fn closest(a: &Samples, b: &Samples) -> Option<(DVec3, DVec3)> {
    let prims = |s: &Samples| -> Vec<(Prim, (DVec3, DVec3))> {
        let v = s.points.iter().map(|&p| Prim::P(p)).chain(s.segments.iter().map(|&x| Prim::S(x))).chain(s.triangles.iter().map(|&t| Prim::T(t)));
        v.map(|p| (p, p.bounds())).collect()
    };
    let (pa, mut pb) = (prims(a), prims(b));
    // Sweep along X: B sorted by box centre, each piece of A looks outward
    // from its own X and stops once boxes are surely too far to beat the best.
    let cx = |(lo, hi): (DVec3, DVec3)| (lo.x + hi.x) * 0.5;
    pb.sort_by(|x, y| cx(x.1).total_cmp(&cx(y.1)));
    let half_b = pb.iter().map(|(_, (lo, hi))| (hi.x - lo.x) * 0.5).fold(0.0, f64::max);
    let gap = |(lo1, hi1): (DVec3, DVec3), (lo2, hi2): (DVec3, DVec3)| (lo2 - hi1).max(lo1 - hi2).max(DVec3::ZERO).length_squared();
    let mut best: Option<(f64, DVec3, DVec3)> = None;
    for &(x, bx) in &pa {
        let (c, half) = (cx(bx), (bx.1.x - bx.0.x) * 0.5);
        let mid = pb.partition_point(|y| cx(y.1) < c);
        let mut visit = |k: usize| -> bool {
            let (y, by) = pb[k];
            let reach = (cx(by) - c).abs() - half - half_b;
            if reach > 0.0 && best.is_some_and(|(bd, ..)| reach * reach >= bd) {
                return false; // everything further this way is further still
            }
            if best.is_none_or(|(bd, ..)| gap(bx, by) < bd) {
                let (p, q) = prim_closest(x, y);
                let d = p.distance_squared(q);
                if best.is_none_or(|(bd, ..)| d < bd) {
                    best = Some((d, p, q));
                }
            }
            true
        };
        for k in mid..pb.len() {
            if !visit(k) {
                break;
            }
        }
        for k in (0..mid).rev() {
            if !visit(k) {
                break;
            }
        }
    }
    best.map(|(_, p, q)| (p, q))
}

/// The point of segment `s` nearest `p`.
fn point_segment(p: DVec3, [a, b]: [DVec3; 2]) -> DVec3 {
    let ab = b - a;
    let l = ab.length_squared();
    if l <= 0.0 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / l).clamp(0.0, 1.0)
}

/// The point of triangle `t` nearest `p` (Ericson, Real-Time Collision Detection 5.1.5).
fn point_triangle(p: DVec3, [a, b, c]: [DVec3; 3]) -> DVec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = va + vb + vc;
    if denom.abs() < 1e-300 {
        return a;
    }
    a + ab * (vb / denom) + ac * (vc / denom)
}

/// The closest points of two segments (Ericson 5.1.9).
fn segment_segment([p1, q1]: [DVec3; 2], [p2, q2]: [DVec3; 2]) -> (DVec3, DVec3) {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.length_squared(), d2.length_squared(), d2.dot(r));
    let eps = 1e-24;
    let (s, t);
    if a <= eps && e <= eps {
        return (p1, p2);
    }
    if a <= eps {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= eps {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > eps * a * e { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// The circle through three points (centre, unit normal, radius); None if they're in a line.
pub fn circle_through(a: DVec3, b: DVec3, c: DVec3) -> Option<(DVec3, DVec3, f64)> {
    let (u, v) = (b - a, c - a);
    let w = u.cross(v);
    let w2 = w.length_squared();
    if w2 <= 1e-24 * u.length_squared() * v.length_squared() {
        return None;
    }
    let center = a + (w.cross(u) * v.length_squared() + v.cross(w) * u.length_squared()) / (2.0 * w2);
    Some((center, w / w2.sqrt(), center.distance(a)))
}

/// The circle (or arc) a run of points on a circle lies on: centre, normal
/// (the way the points go round), radius, and how far round they go.
pub fn circle_of(pts: &[DVec3]) -> Option<Geom> {
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let (center, normal, radius) = circle_through(pts[0], pts[n / 3], pts[2 * n / 3])?;
    let closed = pts[0].distance(pts[n - 1]) <= 1e-9 * radius.max(1.0);
    let sweep = if closed {
        TAU
    } else {
        let x = (pts[0] - center) / radius;
        let y = normal.cross(x);
        let e = pts[n - 1] - center;
        e.dot(y).atan2(e.dot(x)).rem_euclid(TAU)
    };
    Some(Geom::Circle { center, normal, radius, sweep })
}

pub use crate::units::Unit;

/// How the Measure window shows values.
impl Unit {
    /// A value (mm, mm² or degrees) as text in this unit with `decimals`
    /// places, e.g. "12.700 mm", "1.250 in²", "90.000°". `second` shows
    /// angles in radians (the second line under the degrees).
    pub fn format(self, value: f64, kind: Kind, second: bool, decimals: usize) -> String {
        let d = decimals;
        match kind {
            Kind::Length => format!("{:.d$} {}", value / self.mm(), self.name()),
            Kind::Area => format!("{:.d$} {}²", value / (self.mm() * self.mm()), self.name()),
            Kind::Volume => format!("{:.d$} {}³", value / self.mm().powi(3), self.name()),
            Kind::Angle if second => format!("{:.d$} rad", value.to_radians()),
            Kind::Angle => format!("{value:.d$}°"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn point(p: DVec3) -> Picked {
        Picked::point(p)
    }

    fn square(z: f64, face: usize) -> Picked {
        let [a, b, c, d] = [DVec3::new(0.0, 0.0, z), DVec3::new(10.0, 0.0, z), DVec3::new(10.0, 10.0, z), DVec3::new(0.0, 10.0, z)];
        Picked {
            pick: Pick::Face { body: 0, face },
            geom: Geom::Plane { normal: DVec3::Z, area: 100.0, loop_length: 40.0, center: None },
            samples: Samples { triangles: vec![[a, b, c], [a, c, d]], ..Default::default() },
        }
    }

    fn results(m: &Measure) -> Vec<(&'static str, f64)> {
        m.rows.iter().filter(|r| r.section == Section::Results).map(|r| (r.label, r.value)).collect()
    }

    #[test]
    fn two_points_show_both_positions_then_distance_and_deltas() {
        let mut m = Measure::default();
        m.click(point(DVec3::new(1.0, 2.0, 3.0)));
        assert_eq!(m.rows.iter().map(|r| r.value).collect::<Vec<_>>(), vec![1.0, 2.0, 3.0]);
        m.click(point(DVec3::new(4.0, 6.0, 3.0)));
        // Selection 1 and 2 each keep their own X/Y/Z, like Fusion.
        assert_eq!(m.rows.iter().filter(|r| r.section == Section::Selection(1)).count(), 3);
        assert_eq!(results(&m), [("Distance", 5.0), ("ΔX", 3.0), ("ΔY", 4.0), ("ΔZ", 0.0)]);
        // A third pick replaces the second; the first stays.
        m.click(point(DVec3::ZERO));
        assert_eq!(m.picks.len(), 2);
        assert_eq!(m.picks[0].geom, Geom::Point(DVec3::new(1.0, 2.0, 3.0)));
        assert_eq!(results(&m)[0].0, "Distance");
        assert!((results(&m)[0].1 - 14f64.sqrt()).abs() < 1e-12);
        // Clicking one again drops it.
        m.click(point(DVec3::ZERO));
        m.click(point(DVec3::new(1.0, 2.0, 3.0)));
        assert!(m.picks.is_empty() && m.rows.is_empty());
    }

    #[test]
    fn deltas_are_for_corners_only() {
        let mut m = Measure::default();
        m.click(point(DVec3::new(5.0, 5.0, 3.0)));
        m.click(square(0.0, 0));
        assert_eq!(results(&m), [("Distance", 3.0)]);
    }

    #[test]
    fn picks_that_changed_are_dropped() {
        let mut m = Measure::default();
        m.click(square(0.0, 0));
        m.click(square(5.0, 1));
        m.keep_valid(|p| matches!(p, Pick::Face { face: 0, .. }).then(|| square(0.0, 0).geom));
        assert_eq!(m.picks.len(), 1);
        assert_eq!(m.rows[0].label, "Area");
    }

    #[test]
    fn a_whole_circle_reads_circumference_radius_diameter_and_centre() {
        let pts: Vec<DVec3> = (0..=40).map(|i| {
            let a = TAU * i as f64 / 40.0;
            DVec3::new(3.0 + 5.0 * a.cos(), 5.0 * a.sin(), 7.0)
        }).collect();
        let g = circle_of(&pts).unwrap();
        let rows = describe(&g, 0);
        assert_eq!(rows.iter().map(|r| r.label).collect::<Vec<_>>(), ["Circumference", "Radius", "Diameter", "Center X", "Center Y", "Center Z"]);
        let v: Vec<f64> = rows.iter().map(|r| r.value).collect();
        assert!((v[0] - 10.0 * PI).abs() < 1e-9 && (v[1] - 5.0).abs() < 1e-9 && (v[2] - 10.0).abs() < 1e-9);
        assert!((v[3] - 3.0).abs() < 1e-9 && v[4].abs() < 1e-9 && (v[5] - 7.0).abs() < 1e-9);
        assert_eq!((rows[1].show, rows[2].show, rows[3].show), (Show::Radius(0), Show::Diameter(0), Show::Centre(0)));
    }

    #[test]
    fn an_arc_reads_its_length() {
        // A quarter, then three quarters, of a radius-2 circle.
        for sweep in [PI / 2.0, 1.5 * PI] {
            let pts: Vec<DVec3> = (0..=30).map(|i| {
                let a = sweep * i as f64 / 30.0;
                DVec3::new(2.0 * a.cos(), 2.0 * a.sin(), 0.0)
            }).collect();
            let rows = describe(&circle_of(&pts).unwrap(), 1);
            assert_eq!(rows[0].label, "Length");
            assert!((rows[0].value - 2.0 * sweep).abs() < 1e-9, "{rows:?}");
            assert!(rows.iter().all(|r| r.section == Section::Selection(1)));
        }
    }

    #[test]
    fn faces_read_area_and_loop_length_and_a_body_nothing() {
        let labels = |g: Geom| describe(&g, 0).iter().map(|r| r.label).collect::<Vec<_>>();
        assert_eq!(labels(square(0.0, 0).geom), ["Area", "Loop length"]);
        let disc = Geom::Plane { normal: DVec3::Z, area: PI, loop_length: TAU, center: Some(DVec3::ZERO) };
        assert_eq!(labels(disc), ["Area", "Loop length", "Center X", "Center Y", "Center Z"]);
        let cyl = Geom::Cylinder { axis: DVec3::Z, radius: 2.0, area: 1.0, loop_length: 1.0 };
        assert_eq!(labels(cyl), ["Radius", "Diameter", "Area", "Loop length"]);
        assert_eq!(labels(Geom::Body { volume: 1.0, area: 6.0, center: DVec3::ZERO }), ["Volume", "Area", "Center of mass X", "Center of mass Y", "Center of mass Z"]);
    }

    #[test]
    fn parallel_faces_are_their_gap_apart_at_zero_degrees() {
        let (rows, closest) = compare(&square(0.0, 0), &square(7.5, 1));
        assert_eq!((rows[0].label, rows[0].value), ("Distance", 7.5));
        let (p, q) = closest.unwrap();
        assert!((q - p - DVec3::new(0.0, 0.0, 7.5)).length() < 1e-12);
        assert_eq!(rows.iter().find(|r| r.label == "Angle").unwrap().value, 0.0);
    }

    #[test]
    fn angles_between_lines_planes_and_both() {
        let line = |d: DVec3| Geom::Line { a: DVec3::ZERO, b: d };
        let plane = |n: DVec3| Geom::Plane { normal: n, area: 1.0, loop_length: 4.0, center: None };
        assert!((angle(&line(DVec3::X), &line(DVec3::new(1.0, 1.0, 0.0))).unwrap() - 45.0).abs() < 1e-9);
        assert!((angle(&plane(DVec3::Z), &plane(DVec3::X)).unwrap() - 90.0).abs() < 1e-9);
        assert!((angle(&plane(DVec3::Z), &plane(-DVec3::Z)).unwrap()).abs() < 1e-9);
        // A line standing on a plane is at 90° to it; one lying in it at 0°.
        assert!((angle(&line(DVec3::Z), &plane(DVec3::Z)).unwrap() - 90.0).abs() < 1e-9);
        assert!(angle(&line(DVec3::X), &plane(DVec3::Z)).unwrap().abs() < 1e-9);
        assert_eq!(angle(&Geom::Point(DVec3::ZERO), &plane(DVec3::Z)), None);
    }

    #[test]
    fn closest_points_between_points_edges_and_faces() {
        // A corner above the middle of a face: straight down.
        let up = DVec3::new(5.0, 4.0, 3.0);
        let (p, q) = closest(&point(up).samples, &square(0.0, 0).samples).unwrap();
        assert_eq!((p, q), (up, DVec3::new(5.0, 4.0, 0.0)));
        // Crossing skew segments: the common perpendicular.
        let a = Samples { segments: vec![[DVec3::new(-1.0, 0.0, 0.0), DVec3::new(1.0, 0.0, 0.0)]], ..Default::default() };
        let b = Samples { segments: vec![[DVec3::new(0.0, -1.0, 2.0), DVec3::new(0.0, 1.0, 2.0)]], ..Default::default() };
        let (p, q) = closest(&a, &b).unwrap();
        assert!(p.distance(DVec3::ZERO) < 1e-12 && q.distance(DVec3::new(0.0, 0.0, 2.0)) < 1e-12);
        // An edge beside a face, off its side.
        let e = Samples { segments: vec![[DVec3::new(12.0, 0.0, 1.0), DVec3::new(12.0, 10.0, 1.0)]], ..Default::default() };
        let (p, q) = closest(&e, &square(0.0, 0).samples).unwrap();
        assert!((p.distance(q) - 5f64.sqrt()).abs() < 1e-12);
        assert!(closest(&Samples::default(), &a).is_none());
    }

    #[test]
    fn closest_between_big_meshes_is_quick_and_right() {
        // Two 60 × 60 grids of triangles (7200 each), 3 apart: the gap, fast.
        let grid = |z: f64| {
            let mut t = Vec::new();
            for i in 0..60 {
                for j in 0..60 {
                    let p = |a: usize, b: usize| DVec3::new(a as f64, b as f64, z);
                    t.push([p(i, j), p(i + 1, j), p(i + 1, j + 1)]);
                    t.push([p(i, j), p(i + 1, j + 1), p(i, j + 1)]);
                }
            }
            Samples { triangles: t, ..Default::default() }
        };
        let start = std::time::Instant::now();
        let (p, q) = closest(&grid(0.0), &grid(3.0)).unwrap();
        assert!((p.distance(q) - 3.0).abs() < 1e-12);
        assert!(start.elapsed().as_secs_f64() < 5.0, "{:?}", start.elapsed());
    }

    #[test]
    fn units_format_at_a_precision_and_copied_text_parses_back() {
        assert_eq!(Unit::Mm.format(12.7, Kind::Length, false, 3), "12.700 mm");
        assert_eq!(Unit::In.format(12.7, Kind::Length, false, 4), "0.5000 in");
        assert_eq!(Unit::In.format(12.7, Kind::Length, false, 1), "0.5 in");
        assert_eq!(Unit::Cm.format(250.0, Kind::Area, false, 2), "2.50 cm²");
        assert_eq!(Unit::Cm.format(1500.0, Kind::Volume, false, 2), "1.50 cm³");
        assert_eq!(Unit::Mm.format(1500.0, Kind::Volume, false, 0), "1500 mm³");
        assert_eq!(Unit::In.format(645.16, Kind::Area, false, 3), "1.000 in²");
        assert_eq!(Unit::Mm.format(90.0, Kind::Angle, false, 0), "90°");
        assert_eq!(Unit::Mm.format(180.0, Kind::Angle, true, 5), "3.14159 rad");
        // Pasted back, a copied value is right to the last digit shown.
        for u in Unit::ALL {
            for d in 0..=6 {
                let text = u.format(12.7, Kind::Length, false, d);
                let half_digit = 0.5 * 10f64.powi(-(d as i32)) * u.mm();
                assert!((crate::units::parse_length(&text, Unit::Mm).unwrap() - 12.7).abs() <= half_digit + 1e-12, "{text}");
            }
        }
        assert_eq!(crate::units::parse_angle("90.000°"), Some(90.0));
        // The radians line (copied) reads back as the same angle.
        assert!((crate::units::parse_angle(&Unit::Mm.format(1.0, Kind::Angle, true, 6)).unwrap() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn quick_stats_for_model_selections() {
        let circle = Geom::Circle { center: DVec3::ZERO, normal: DVec3::Z, radius: 60.0, sweep: TAU };
        let line = Geom::Line { a: DVec3::ZERO, b: DVec3::new(30.0, 40.0, 0.0) };
        let face = square(0.0, 0).geom;
        let cyl = Geom::Cylinder { axis: DVec3::Z, radius: 5.0, area: 314.0, loop_length: 0.0 };
        let names = |gs: &[&Geom]| quick_stats(gs).iter().map(|r| (r.0, (r.1 * 1e6).round() / 1e6)).collect::<Vec<_>>();
        assert_eq!(names(&[&circle]), [("Diameter", 120.0), ("Circumference", (TAU * 60.0 * 1e6).round() / 1e6)]);
        assert_eq!(names(&[&line]), [("Length", 50.0)]);
        assert_eq!(names(&[&face]), [("Area", 100.0)]);
        assert_eq!(names(&[&cyl]), [("Diameter", 10.0), ("Area", 314.0)]);
        assert_eq!(names(&[&line, &line, &face]), [("Total length", 100.0), ("Total area", 100.0)]);
    }
}
