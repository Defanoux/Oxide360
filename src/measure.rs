//! The Measure tool, like Fusion's: pick one or two things on the model (a
//! corner, an edge, a face) and read what they are — length, radius,
//! circumference, area — and how they sit to each other (minimum distance,
//! angle). Values are in mm (mm², degrees) here; `Unit` turns them into
//! whatever the window shows.

use glam::DVec3;
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// Something on a body that was clicked.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pick {
    /// A corner (an end of an edge).
    Point(DVec3),
    /// `body`'s `edge`th pickable edge (`kernel::Body::edges`).
    Edge { body: usize, edge: usize },
    /// `body`'s `face`th face (`kernel::Body::faces`).
    Face { body: usize, face: usize },
}

/// What an edge or face is, exactly (from the kernel, not the display mesh).
#[derive(Clone, Debug, PartialEq)]
pub enum Geom {
    Point(DVec3),
    Line { a: DVec3, b: DVec3 },
    /// A circle or an arc of one; `sweep` is 2π for a whole circle.
    Circle { center: DVec3, normal: DVec3, radius: f64, sweep: f64 },
    /// Any other curve, by its length.
    Curve { length: f64 },
    Plane { normal: DVec3, area: f64, perimeter: f64 },
    Cylinder { axis: DVec3, radius: f64, area: f64 },
    Surface { area: f64 },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Length,
    Area,
    Angle,
}

/// One line of the Measure window.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub label: &'static str,
    /// mm, mm² or degrees.
    pub value: f64,
    pub kind: Kind,
}

fn row(label: &'static str, value: f64, kind: Kind) -> Row {
    Row { label, value, kind }
}

/// The Measure tool's state: up to two picks, and the readings for them.
#[derive(Clone, Debug, Default)]
pub struct Measure {
    pub picks: Vec<Picked>,
    /// What the cursor is over.
    pub hover: Option<Picked>,
    pub rows: Vec<Row>,
    /// The closest points between two picks (drawn as a line).
    pub closest: Option<(DVec3, DVec3)>,
}

impl Measure {
    /// A click: picking something already picked drops it; a third pick starts over.
    pub fn click(&mut self, p: Picked) {
        if let Some(i) = self.picks.iter().position(|q| q.pick == p.pick) {
            self.picks.remove(i);
        } else {
            if self.picks.len() >= 2 {
                self.picks.clear();
            }
            self.picks.push(p);
        }
        self.update();
    }

    /// Drop picks that aren't there any more (the model was rebuilt): `now`
    /// says what a pick is in the current model.
    pub fn keep_valid(&mut self, now: impl Fn(&Pick) -> Option<Geom>) {
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
        (self.rows, self.closest) = match &self.picks[..] {
            [a] => (describe(&a.geom), None),
            [a, b] => compare(a, b),
            _ => (Vec::new(), None),
        };
    }
}

/// What one pick measures.
pub fn describe(g: &Geom) -> Vec<Row> {
    use Kind::*;
    match *g {
        Geom::Point(p) => vec![row("X", p.x, Length), row("Y", p.y, Length), row("Z", p.z, Length)],
        Geom::Line { a, b } => vec![row("Length", a.distance(b), Length)],
        Geom::Circle { radius, sweep, .. } if sweep >= TAU - 1e-9 => {
            vec![row("Circumference", TAU * radius, Length), row("Diameter", 2.0 * radius, Length), row("Radius", radius, Length)]
        }
        Geom::Circle { radius, sweep, .. } => vec![
            row("Arc length", sweep * radius, Length),
            row("Diameter", 2.0 * radius, Length),
            row("Radius", radius, Length),
            row("Arc angle", sweep.to_degrees(), Angle),
        ],
        Geom::Curve { length } => vec![row("Length", length, Length)],
        Geom::Plane { area, perimeter, .. } => vec![row("Area", area, Area), row("Perimeter", perimeter, Length)],
        Geom::Cylinder { radius, area, .. } => vec![row("Area", area, Area), row("Diameter", 2.0 * radius, Length), row("Radius", radius, Length)],
        Geom::Surface { area } => vec![row("Area", area, Area)],
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

/// What two picks measure: the minimum distance between them, its X/Y/Z
/// parts, and the angle when both have a direction.
pub fn compare(a: &Picked, b: &Picked) -> (Vec<Row>, Option<(DVec3, DVec3)>) {
    use Kind::*;
    let mut rows = Vec::new();
    let closest = closest(&a.samples, &b.samples);
    if let Some((p, q)) = closest {
        let d = q - p;
        rows.extend([row("Distance", p.distance(q), Length), row("ΔX", d.x, Length), row("ΔY", d.y, Length), row("ΔZ", d.z, Length)]);
    }
    if let Some(deg) = angle(&a.geom, &b.geom) {
        rows.push(row("Angle", deg, Angle));
    }
    // Two circles / cylinders: how far apart their centres are too.
    if let (Geom::Circle { center: c1, .. }, Geom::Circle { center: c2, .. }) = (&a.geom, &b.geom) {
        rows.push(row("Centre distance", c1.distance(*c2), Length));
    }
    (rows, closest)
}

/// The closest pair of points between two sets of pieces (None if either is empty).
pub fn closest(a: &Samples, b: &Samples) -> Option<(DVec3, DVec3)> {
    let mut best: Option<(f64, DVec3, DVec3)> = None;
    let mut offer = |p: DVec3, q: DVec3| {
        let d = p.distance_squared(q);
        if best.is_none_or(|(bd, ..)| d < bd) {
            best = Some((d, p, q));
        }
    };
    // Every piece as points, segments and triangles; compare all kinds.
    let segs = |s: &Samples| {
        let mut v = s.segments.clone();
        for t in &s.triangles {
            v.extend([[t[0], t[1]], [t[1], t[2]], [t[2], t[0]]]);
        }
        v
    };
    let (sa, sb) = (segs(a), segs(b));
    for &p in &a.points {
        for &q in &b.points {
            offer(p, q);
        }
        for s in &sb {
            offer(p, point_segment(p, *s));
        }
        for t in &b.triangles {
            offer(p, point_triangle(p, *t));
        }
    }
    for &q in &b.points {
        for s in &sa {
            offer(point_segment(q, *s), q);
        }
        for t in &a.triangles {
            offer(point_triangle(q, *t), q);
        }
    }
    for s in &sa {
        for u in &sb {
            let (p, q) = segment_segment(*s, *u);
            offer(p, q);
        }
    }
    // A segment end against a face (the other way round is covered above for corners).
    for s in &sa {
        for t in &b.triangles {
            for p in *s {
                offer(p, point_triangle(p, *t));
            }
        }
    }
    for u in &sb {
        for t in &a.triangles {
            for q in *u {
                offer(point_triangle(q, *t), q);
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

/// Units the Measure window can show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Mm,
    Cm,
    M,
    In,
    Ft,
}

impl Unit {
    pub const ALL: [Unit; 5] = [Unit::Mm, Unit::Cm, Unit::M, Unit::In, Unit::Ft];

    pub fn name(self) -> &'static str {
        match self {
            Unit::Mm => "mm",
            Unit::Cm => "cm",
            Unit::M => "m",
            Unit::In => "in",
            Unit::Ft => "ft",
        }
    }

    /// How many millimetres one of these is.
    pub fn mm(self) -> f64 {
        match self {
            Unit::Mm => 1.0,
            Unit::Cm => 10.0,
            Unit::M => 1000.0,
            Unit::In => 25.4,
            Unit::Ft => 304.8,
        }
    }

    fn decimals(self) -> usize {
        match self {
            Unit::Mm => 3,
            Unit::Cm | Unit::In => 4,
            Unit::M | Unit::Ft => 6,
        }
    }

    /// A value (mm, mm² or degrees) as text in this unit, e.g. "12.700 mm",
    /// "1.2500 in²", "90.000°". `second` shows angles in radians (the second
    /// line under the degrees).
    pub fn format(self, value: f64, kind: Kind, second: bool) -> String {
        let d = self.decimals();
        match kind {
            Kind::Length => format!("{:.d$} {}", value / self.mm(), self.name()),
            Kind::Area => format!("{:.d$} {}²", value / (self.mm() * self.mm()), self.name()),
            Kind::Angle if second => format!("{:.5} rad", value.to_radians()),
            Kind::Angle => format!("{value:.3}°"),
        }
    }
}

/// A typed or pasted number with an optional unit, in mm (lengths) or
/// degrees (angles): "12.7", "12.7 mm", "0.5 in", "0.5\"", "3 cm", "90°",
/// "1.5708 rad". So a value copied from the Measure window pastes anywhere.
pub fn parse_value(text: &str) -> Option<f64> {
    let t = text.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E'))).unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let v: f64 = num.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    let scale = match unit.trim() {
        "" | "mm" | "°" | "deg" => 1.0,
        "cm" => 10.0,
        "m" => 1000.0,
        "in" | "\"" => 25.4,
        "ft" | "'" => 304.8,
        "rad" => 180.0 / std::f64::consts::PI,
        _ => return None,
    };
    Some(v * scale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    fn point(p: DVec3) -> Picked {
        Picked { pick: Pick::Point(p), geom: Geom::Point(p), samples: Samples { points: vec![p], ..Default::default() } }
    }

    fn square(z: f64, face: usize) -> Picked {
        let [a, b, c, d] = [DVec3::new(0.0, 0.0, z), DVec3::new(10.0, 0.0, z), DVec3::new(10.0, 10.0, z), DVec3::new(0.0, 10.0, z)];
        Picked {
            pick: Pick::Face { body: 0, face },
            geom: Geom::Plane { normal: DVec3::Z, area: 100.0, perimeter: 40.0 },
            samples: Samples { triangles: vec![[a, b, c], [a, c, d]], ..Default::default() },
        }
    }

    #[test]
    fn two_points_give_distance_and_deltas_and_a_third_starts_over() {
        let mut m = Measure::default();
        m.click(point(DVec3::new(1.0, 2.0, 3.0)));
        assert_eq!(m.rows.iter().map(|r| r.value).collect::<Vec<_>>(), vec![1.0, 2.0, 3.0]);
        m.click(point(DVec3::new(4.0, 6.0, 3.0)));
        assert_eq!(m.rows[0], row("Distance", 5.0, Kind::Length));
        assert_eq!((m.rows[1].value, m.rows[2].value, m.rows[3].value), (3.0, 4.0, 0.0));
        m.click(point(DVec3::ZERO));
        assert_eq!(m.picks.len(), 1);
        // Clicking it again drops it.
        m.click(point(DVec3::ZERO));
        assert!(m.picks.is_empty() && m.rows.is_empty());
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
    fn a_whole_circle_reads_circumference_diameter_radius() {
        let pts: Vec<DVec3> = (0..=40).map(|i| {
            let a = TAU * i as f64 / 40.0;
            DVec3::new(3.0 + 5.0 * a.cos(), 5.0 * a.sin(), 7.0)
        }).collect();
        let g = circle_of(&pts).unwrap();
        let rows = describe(&g);
        assert_eq!(rows.iter().map(|r| r.label).collect::<Vec<_>>(), ["Circumference", "Diameter", "Radius"]);
        assert!((rows[0].value - 10.0 * PI).abs() < 1e-9);
        assert!((rows[1].value - 10.0).abs() < 1e-9 && (rows[2].value - 5.0).abs() < 1e-9);
        let Geom::Circle { center, normal, .. } = g else { unreachable!() };
        assert!(center.distance(DVec3::new(3.0, 0.0, 7.0)) < 1e-9 && normal.z.abs() > 1.0 - 1e-12);
    }

    #[test]
    fn an_arc_reads_its_length_and_angle() {
        // A quarter, then three quarters, of a radius-2 circle.
        for (end, sweep) in [(PI / 2.0, PI / 2.0), (1.5 * PI, 1.5 * PI)] {
            let pts: Vec<DVec3> = (0..=30).map(|i| {
                let a = end * i as f64 / 30.0;
                DVec3::new(2.0 * a.cos(), 2.0 * a.sin(), 0.0)
            }).collect();
            let rows = describe(&circle_of(&pts).unwrap());
            assert_eq!(rows[0].label, "Arc length");
            assert!((rows[0].value - 2.0 * sweep).abs() < 1e-9, "{rows:?}");
            assert!((rows[3].value - sweep.to_degrees()).abs() < 1e-7);
        }
    }

    #[test]
    fn parallel_faces_are_their_gap_apart_at_zero_degrees() {
        let (rows, closest) = compare(&square(0.0, 0), &square(7.5, 1));
        assert_eq!(rows[0], row("Distance", 7.5, Kind::Length));
        let (p, q) = closest.unwrap();
        assert!((q - p - DVec3::new(0.0, 0.0, 7.5)).length() < 1e-12);
        assert_eq!(rows.iter().find(|r| r.label == "Angle").unwrap().value, 0.0);
    }

    #[test]
    fn angles_between_lines_planes_and_both() {
        let line = |d: DVec3| Geom::Line { a: DVec3::ZERO, b: d };
        let plane = |n: DVec3| Geom::Plane { normal: n, area: 1.0, perimeter: 4.0 };
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
    fn units_format_and_copied_text_parses_back() {
        assert_eq!(Unit::Mm.format(12.7, Kind::Length, false), "12.700 mm");
        assert_eq!(Unit::In.format(12.7, Kind::Length, false), "0.5000 in");
        assert_eq!(Unit::Cm.format(250.0, Kind::Area, false), "2.5000 cm²");
        assert_eq!(Unit::In.format(645.16, Kind::Area, false), "1.0000 in²");
        assert_eq!(Unit::Mm.format(90.0, Kind::Angle, false), "90.000°");
        assert_eq!(Unit::Mm.format(180.0, Kind::Angle, true), "3.14159 rad");
        // Pasted back, a copied value is right to the last digit shown.
        for u in Unit::ALL {
            let text = u.format(12.7, Kind::Length, false);
            let half_digit = 0.5 * 10f64.powi(-(u.decimals() as i32)) * u.mm();
            assert!((parse_value(&text).unwrap() - 12.7).abs() <= half_digit, "{text}");
        }
        assert_eq!(parse_value("90.000°"), Some(90.0));
        assert_eq!(parse_value(" 3 "), Some(3.0));
        assert_eq!(parse_value("-2.5mm"), Some(-2.5));
        assert_eq!(parse_value("1\""), Some(25.4));
        assert_eq!(parse_value("1e1"), Some(10.0));
        assert_eq!(parse_value("2 parsecs"), None);
        assert_eq!(parse_value(""), None);
    }
}
