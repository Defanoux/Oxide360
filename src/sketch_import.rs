//! Insert SVG / DXF into a sketch (Fusion's Insert SVG and Insert DXF): the
//! drawing's curves, in millimetres with Y up, become sketch lines, arcs,
//! circles, ellipses and splines. Lines, arcs and circles stay exact; SVG
//! curves are cubic Béziers, which a degree-3 control-point spline with four
//! points is exactly. Ends that meet share one sketch point, so closed
//! shapes make profiles.

use crate::sketch::{Entity, PointId, Sketch};
use glam::DVec2;
use kurbo::{Affine, BezPath, PathEl, Point, Shape as _};
use std::f64::consts::{PI, TAU};

/// One curve of the drawing (mm, Y up).
#[derive(Clone, Debug, PartialEq)]
pub enum Curve {
    Line(DVec2, DVec2),
    /// Counter-clockwise from angle `a0` (radians) through `sweep`.
    Arc { c: DVec2, r: f64, a0: f64, sweep: f64 },
    Circle { c: DVec2, r: f64 },
    /// A whole ellipse: semi-axis `major` (a vector from the centre), the other `ratio` times as long.
    Ellipse { c: DVec2, major: DVec2, ratio: f64 },
    /// A cubic Bézier: start, two pulls, end.
    Bezier([DVec2; 4]),
    /// A smooth curve through these points (closed if the last is the first).
    Fit(Vec<DVec2>),
}

/// What a file gave: its curves, and what was left out.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    pub curves: Vec<Curve>,
    /// "3 text elements" and so on, for the dialog.
    pub skipped: Vec<String>,
}

/// Where the curves go: scaled about the drawing's origin, turned, then moved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub offset: DVec2,
    /// Degrees, counter-clockwise.
    pub angle: f64,
    pub scale: f64,
}

impl Default for Placement {
    fn default() -> Self {
        Placement { offset: DVec2::ZERO, angle: 0.0, scale: 1.0 }
    }
}

impl Placement {
    pub fn apply(&self, p: DVec2) -> DVec2 {
        DVec2::from_angle(self.angle.to_radians()).rotate(p * self.scale) + self.offset
    }

    fn vector(&self, v: DVec2) -> DVec2 {
        DVec2::from_angle(self.angle.to_radians()).rotate(v * self.scale)
    }

    /// A curve placed (a similarity, so arcs stay arcs).
    pub fn curve(&self, c: &Curve) -> Curve {
        let a = self.angle.to_radians();
        match c {
            Curve::Line(p, q) => Curve::Line(self.apply(*p), self.apply(*q)),
            Curve::Arc { c, r, a0, sweep } => Curve::Arc { c: self.apply(*c), r: r * self.scale, a0: a0 + a, sweep: *sweep },
            Curve::Circle { c, r } => Curve::Circle { c: self.apply(*c), r: r * self.scale },
            Curve::Ellipse { c, major, ratio } => Curve::Ellipse { c: self.apply(*c), major: self.vector(*major), ratio: *ratio },
            Curve::Bezier(b) => Curve::Bezier(b.map(|p| self.apply(p))),
            Curve::Fit(pts) => Curve::Fit(pts.iter().map(|p| self.apply(*p)).collect()),
        }
    }
}

impl Curve {
    /// Points along it, for the preview.
    pub fn sample(&self) -> Vec<DVec2> {
        match *self {
            Curve::Line(p, q) => vec![p, q],
            Curve::Arc { c, r, a0, sweep } => {
                let n = ((sweep.abs() / TAU) * 64.0).ceil().max(4.0) as usize;
                (0..=n).map(|k| c + DVec2::from_angle(a0 + sweep * k as f64 / n as f64) * r).collect()
            }
            Curve::Circle { c, r } => (0..=64).map(|k| c + DVec2::from_angle(TAU * k as f64 / 64.0) * r).collect(),
            Curve::Ellipse { c, major, ratio } => {
                let minor = major.perp() * ratio;
                (0..=64).map(|k| {
                    let t = TAU * k as f64 / 64.0;
                    c + major * t.cos() + minor * t.sin()
                }).collect()
            }
            Curve::Bezier(b) => (0..=16).map(|k| bezier_at(&b, k as f64 / 16.0)).collect(),
            Curve::Fit(ref pts) => pts.clone(),
        }
    }
}

fn bezier_at(b: &[DVec2; 4], t: f64) -> DVec2 {
    let u = 1.0 - t;
    b[0] * (u * u * u) + b[1] * (3.0 * u * u * t) + b[2] * (3.0 * u * t * t) + b[3] * (t * t * t)
}

/// The drawing's size: (min, max) corners, or None when it's empty.
pub fn bounds(curves: &[Curve]) -> Option<(DVec2, DVec2)> {
    let pts: Vec<DVec2> = curves.iter().flat_map(|c| c.sample()).collect();
    let first = *pts.first()?;
    Some(pts.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))))
}

/// Add the curves to the sketch (already placed). Ends closer than `tol`
/// share one point. Returns the first new entity.
pub fn add_to_sketch(sk: &mut Sketch, curves: &[Curve]) -> usize {
    let first = sk.entities.len();
    let size = bounds(curves).map_or(1.0, |(lo, hi)| (hi - lo).length().max(1.0));
    let tol = size * 1e-7;
    let mut ends: Vec<PointId> = Vec::new();
    // A shared end point (found again, or made).
    let mut end = |sk: &mut Sketch, p: DVec2| -> PointId {
        if let Some(&id) = ends.iter().find(|&&id| sk.points[id].distance(p) <= tol) {
            return id;
        }
        sk.points.push(p);
        let id = sk.points.len() - 1;
        ends.push(id);
        id
    };
    let own = |sk: &mut Sketch, p: DVec2| -> PointId {
        sk.points.push(p);
        sk.points.len() - 1
    };
    for c in curves {
        match *c {
            Curve::Line(p, q) => {
                if p.distance(q) <= tol {
                    continue;
                }
                let (a, b) = (end(sk, p), end(sk, q));
                sk.entities.push(Entity::Line { a, b });
            }
            Curve::Arc { c, r, a0, sweep } => {
                if r <= tol || sweep.abs() <= 1e-9 {
                    continue;
                }
                if sweep.abs() >= TAU - 1e-9 {
                    let center = own(sk, c);
                    sk.entities.push(Entity::Circle { center, radius: r });
                    continue;
                }
                // Counter-clockwise from a to b.
                let (s, e) = if sweep > 0.0 { (a0, a0 + sweep) } else { (a0 + sweep, a0) };
                let center = own(sk, c);
                let a = end(sk, c + DVec2::from_angle(s) * r);
                let b = end(sk, c + DVec2::from_angle(e) * r);
                sk.entities.push(Entity::Arc { center, a, b });
            }
            Curve::Circle { c, r } => {
                if r > tol {
                    let center = own(sk, c);
                    sk.entities.push(Entity::Circle { center, radius: r });
                }
            }
            Curve::Ellipse { c, major, ratio } => {
                if major.length() > tol && ratio > 1e-9 {
                    let center = own(sk, c);
                    let major_p = own(sk, c + major);
                    let minor_p = own(sk, c + major.perp() * ratio);
                    sk.entities.push(Entity::Ellipse { center, major: major_p, minor: minor_p });
                }
            }
            Curve::Bezier(b) => {
                if b[0].distance(b[3]) <= tol && b[1].distance(b[0]) <= tol && b[2].distance(b[0]) <= tol {
                    continue;
                }
                // A straight Bézier is a line.
                let chord = b[3] - b[0];
                let straight = chord.length() > tol && [b[1], b[2]].iter().all(|p| (p - b[0]).perp_dot(chord).abs() <= tol * chord.length());
                if straight {
                    let (a, e) = (end(sk, b[0]), end(sk, b[3]));
                    sk.entities.push(Entity::Line { a, b: e });
                    continue;
                }
                let a = end(sk, b[0]);
                let p1 = own(sk, b[1]);
                let p2 = own(sk, b[2]);
                let e = end(sk, b[3]);
                sk.splines.push(vec![a, p1, p2, e]);
                sk.control_splines.push((sk.splines.len() - 1, 3));
                sk.entities.push(Entity::Spline { s: sk.splines.len() - 1 });
            }
            Curve::Fit(ref pts) => {
                let mut pts: Vec<DVec2> = pts.clone();
                pts.dedup_by(|a, b| a.distance(*b) <= tol);
                let closed = pts.len() >= 4 && pts[0].distance(pts[pts.len() - 1]) <= tol;
                if closed {
                    pts.pop();
                }
                if pts.len() < 2 {
                    continue;
                }
                let n = pts.len();
                let mut ids: Vec<PointId> = pts.iter().enumerate().map(|(k, &p)| if k == 0 || k == n - 1 { end(sk, p) } else { own(sk, p) }).collect();
                if closed {
                    ids.push(ids[0]);
                }
                sk.splines.push(ids);
                sk.entities.push(Entity::Spline { s: sk.splines.len() - 1 });
            }
        }
    }
    first
}

// ---------------------------------------------------------------- SVG

/// Millimetres in one of an SVG length's units ("10mm", "2in", "96" px).
fn svg_length(text: &str) -> Option<f64> {
    let t = text.trim();
    let split = t.find(|c: char| c.is_ascii_alphabetic() || c == '%').unwrap_or(t.len());
    let (num, unit) = t.split_at(split);
    let v: f64 = num.trim().parse().ok()?;
    let mm = match unit.trim() {
        "mm" => 1.0,
        "cm" => 10.0,
        "in" => 25.4,
        "pt" => 25.4 / 72.0,
        "pc" => 25.4 / 6.0,
        "px" | "" => 25.4 / 96.0,
        _ => return None, // % or em: no absolute size
    };
    Some(v * mm)
}

/// Numbers in an attribute (commas or spaces between).
fn numbers(text: &str) -> Vec<f64> {
    text.split(|c: char| c == ',' || c.is_whitespace()).filter(|s| !s.is_empty()).filter_map(|s| s.parse().ok()).collect()
}

/// An SVG `transform` attribute.
fn svg_transform(text: &str) -> Affine {
    let mut out = Affine::IDENTITY;
    for part in text.split(')') {
        let Some((name, args)) = part.split_once('(') else { continue };
        let v = numbers(args);
        let get = |i: usize, d: f64| v.get(i).copied().unwrap_or(d);
        let t = match name.trim().trim_start_matches(',').trim() {
            "matrix" if v.len() == 6 => Affine::new([v[0], v[1], v[2], v[3], v[4], v[5]]),
            "translate" => Affine::translate((get(0, 0.0), get(1, 0.0))),
            "scale" => Affine::scale_non_uniform(get(0, 1.0), get(1, get(0, 1.0))),
            "rotate" => {
                let (c, s) = (get(1, 0.0), get(2, 0.0));
                Affine::translate((c, s)) * Affine::rotate(get(0, 0.0).to_radians()) * Affine::translate((-c, -s))
            }
            "skewX" => Affine::skew(get(0, 0.0).to_radians().tan(), 0.0),
            "skewY" => Affine::skew(0.0, get(0, 0.0).to_radians().tan()),
            _ => Affine::IDENTITY,
        };
        out *= t;
    }
    out
}

/// Does this transform keep circles round (rotation, uniform scale, mirror)?
fn keeps_circles(t: &Affine) -> Option<f64> {
    let [a, b, c, d, _, _] = t.as_coeffs();
    let (s1, s2) = ((a * a + b * b).sqrt(), (c * c + d * d).sqrt());
    ((s1 - s2).abs() <= 1e-9 * s1.max(1.0) && (a * c + b * d).abs() <= 1e-9 * s1 * s1.max(1.0)).then_some(s1)
}

fn kp(p: Point) -> DVec2 {
    DVec2::new(p.x, p.y)
}

/// A path's pieces as curves (after `t`).
fn path_curves(path: &BezPath, t: Affine, out: &mut Vec<Curve>) {
    let path = t * path.clone();
    let (mut start, mut at) = (DVec2::ZERO, DVec2::ZERO);
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                start = kp(p);
                at = start;
            }
            PathEl::LineTo(p) => {
                out.push(Curve::Line(at, kp(p)));
                at = kp(p);
            }
            PathEl::QuadTo(q, p) => {
                // Raised to a cubic (exactly the same curve).
                let (q, p) = (kp(q), kp(p));
                out.push(Curve::Bezier([at, at + (q - at) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p]));
                at = p;
            }
            PathEl::CurveTo(a, b, p) => {
                out.push(Curve::Bezier([at, kp(a), kp(b), kp(p)]));
                at = kp(p);
            }
            PathEl::ClosePath => {
                if at.distance(start) > 1e-9 {
                    out.push(Curve::Line(at, start));
                }
                at = start;
            }
        }
    }
}

/// Read an SVG drawing: paths, rectangles, circles, ellipses, lines,
/// polylines and polygons, through groups and their transforms. Sizes come
/// from the root's width / height and viewBox (else 96 px to the inch); Y
/// is turned up.
pub fn parse_svg(text: &str) -> Result<Drawing, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut out = Drawing::default();
    let mut stack: Vec<Affine> = Vec::new();
    let mut skip_depth = 0usize; // inside <defs>, <clipPath>…: not drawn
    let mut texts = 0;
    let mut seen_root = false;
    loop {
        let ev = reader.read_event().map_err(|e| format!("not a readable SVG ({e})"))?;
        let (el, empty) = match &ev {
            Event::Start(e) => (e.clone(), false),
            Event::Empty(e) => (e.clone(), true),
            Event::End(_) => {
                if skip_depth > 0 {
                    skip_depth -= 1;
                } else {
                    stack.pop();
                }
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        let name = String::from_utf8_lossy(el.local_name().as_ref()).to_string();
        if skip_depth > 0 {
            if !empty {
                skip_depth += 1;
            }
            continue;
        }
        if matches!(name.as_str(), "defs" | "clipPath" | "mask" | "symbol" | "pattern" | "marker" | "metadata" | "title" | "desc" | "style") {
            if !empty {
                skip_depth = 1;
            }
            continue;
        }
        let mut attrs: Vec<(String, String)> = Vec::new();
        for a in el.attributes().flatten() {
            let k = String::from_utf8_lossy(a.key.local_name().as_ref()).to_string();
            let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).map(|v| v.to_string()).unwrap_or_default();
            attrs.push((k, v));
        }
        let attr = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let num = |k: &str| attr(k).and_then(|v| numbers(v).first().copied()).unwrap_or(0.0);
        let parent = stack.last().copied().unwrap_or(Affine::IDENTITY);
        let mut here = parent * attr("transform").map_or(Affine::IDENTITY, svg_transform);
        if name == "svg" && !seen_root {
            seen_root = true;
            // User units to mm, Y up.
            let vb = attr("viewBox").map(numbers).filter(|v| v.len() == 4 && v[2] > 0.0 && v[3] > 0.0);
            let (w, h) = (attr("width").and_then(svg_length), attr("height").and_then(svg_length));
            let px = 25.4 / 96.0;
            let (sx, sy, x0, y0) = match vb {
                Some(v) => (w.map_or(px, |w| w / v[2]), h.map_or(px, |h| h / v[3]), v[0], v[1]),
                None => (px, px, 0.0, 0.0),
            };
            here = Affine::scale_non_uniform(sx, -sy) * Affine::translate((-x0, -y0));
        }
        let hidden = attr("display") == Some("none") || attr("style").is_some_and(|s| s.replace(' ', "").contains("display:none"));
        match name.as_str() {
            _ if hidden => {
                if !empty {
                    skip_depth = 1;
                }
                continue;
            }
            "path" => {
                if let Some(d) = attr("d") {
                    let p = BezPath::from_svg(d).map_err(|e| format!("a path the reader can't follow ({e})"))?;
                    path_curves(&p, here, &mut out.curves);
                }
            }
            "rect" => {
                let (x, y, w, h) = (num("x"), num("y"), num("width"), num("height"));
                if w > 0.0 && h > 0.0 {
                    let r = attr("rx").or(attr("ry")).and_then(|v| numbers(v).first().copied()).unwrap_or(0.0).min(w / 2.0).min(h / 2.0);
                    match keeps_circles(&here) {
                        // Four lines and four exact quarter arcs.
                        Some(s) if r > 0.0 => {
                            let at = |px: f64, py: f64| kp(here * Point::new(px, py));
                            let corners = [(x + w - r, y + r), (x + w - r, y + h - r), (x + r, y + h - r), (x + r, y + r)];
                            let sides = [((x + r, y), (x + w - r, y)), ((x + w, y + r), (x + w, y + h - r)), ((x + w - r, y + h), (x + r, y + h)), ((x, y + h - r), (x, y + r))];
                            for k in 0..4 {
                                let ((ax, ay), (bx, by)) = sides[k];
                                out.curves.push(Curve::Line(at(ax, ay), at(bx, by)));
                                // The corner after this side, from its end to the next side's start.
                                let (cx, cy) = corners[k];
                                let (nx, ny) = sides[(k + 1) % 4].0;
                                let (c, p, q) = (at(cx, cy), at(bx, by), at(nx, ny));
                                let (a0, a1) = ((p - c).y.atan2((p - c).x), (q - c).y.atan2((q - c).x));
                                // The short way round (a quarter turn), whichever way the transform turned it.
                                let mut sweep = (a1 - a0).rem_euclid(TAU);
                                if sweep > PI {
                                    sweep -= TAU;
                                }
                                out.curves.push(Curve::Arc { c, r: r * s, a0, sweep });
                            }
                        }
                        _ => {
                            let path = if r > 0.0 { kurbo::RoundedRect::new(x, y, x + w, y + h, r).to_path(1e-9) } else { kurbo::Rect::new(x, y, x + w, y + h).to_path(1e-9) };
                            path_curves(&path, here, &mut out.curves);
                        }
                    }
                }
            }
            "circle" | "ellipse" => {
                let (cx, cy) = (num("cx"), num("cy"));
                let (rx, ry) = if name == "circle" { (num("r"), num("r")) } else { (num("rx"), num("ry")) };
                if rx > 0.0 && ry > 0.0 {
                    let c = kp(here * Point::new(cx, cy));
                    match keeps_circles(&here) {
                        Some(s) if (rx - ry).abs() <= 1e-12 * rx => out.curves.push(Curve::Circle { c, r: rx * s }),
                        Some(_) => {
                            let m = kp(here * Point::new(cx + rx, cy)) - c;
                            out.curves.push(Curve::Ellipse { c, major: m, ratio: ry / rx });
                        }
                        None => path_curves(&kurbo::Ellipse::new((cx, cy), (rx, ry), 0.0).to_path(1e-9), here, &mut out.curves),
                    }
                }
            }
            "line" => {
                let p = kp(here * Point::new(num("x1"), num("y1")));
                let q = kp(here * Point::new(num("x2"), num("y2")));
                out.curves.push(Curve::Line(p, q));
            }
            "polyline" | "polygon" => {
                let v = attr("points").map(numbers).unwrap_or_default();
                let mut pts: Vec<DVec2> = v.chunks_exact(2).map(|c| kp(here * Point::new(c[0], c[1]))).collect();
                if name == "polygon" && pts.len() > 2 {
                    pts.push(pts[0]);
                }
                out.curves.extend(pts.windows(2).map(|w| Curve::Line(w[0], w[1])));
            }
            "text" => texts += 1,
            _ => {}
        }
        if !empty {
            stack.push(here);
        }
    }
    if !seen_root {
        return Err("not an SVG drawing".into());
    }
    if texts > 0 {
        out.skipped.push(format!("{texts} text element{} (turn text into paths first)", if texts == 1 { "" } else { "s" }));
    }
    Ok(out)
}

// ---------------------------------------------------------------- DXF

/// Millimetres in the drawing's unit ($INSUNITS); unitless drawings are taken as mm.
fn dxf_unit(code: i64) -> f64 {
    match code {
        1 => 25.4,
        2 => 304.8,
        5 => 10.0,
        6 => 1000.0,
        8 => 25.4e-6,
        9 => 25.4e-3,
        10 => 914.4,
        _ => 1.0,
    }
}

/// The arc a polyline's bulge makes from p to q (bulge = tan(angle / 4), + counter-clockwise).
fn bulge_arc(p: DVec2, q: DVec2, bulge: f64) -> Curve {
    let theta = 4.0 * bulge.atan();
    let chord = q - p;
    let d = chord.length();
    let r = d / (2.0 * (theta / 2.0).sin()).abs();
    // The centre: from the chord's middle, along its normal.
    let h = (r * r - d * d / 4.0).max(0.0).sqrt();
    let n = chord.perp().normalize() * if (theta.abs() < PI) == (theta > 0.0) { 1.0 } else { -1.0 };
    let c = (p + q) * 0.5 + n * h;
    let a0 = (p - c).y.atan2((p - c).x);
    Curve::Arc { c, r, a0, sweep: theta }
}

/// A non-rational B-spline evaluated at t (de Boor), any knots.
fn deboor(ctrl: &[DVec2], knots: &[f64], degree: usize, t: f64) -> DVec2 {
    let n = ctrl.len();
    let p = degree.min(n - 1);
    let k = (p..n).rev().find(|&k| knots[k] <= t && (t < knots[k + 1] || k == n - 1)).unwrap_or(p);
    let mut d: Vec<DVec2> = (0..=p).map(|r| ctrl[k - p + r]).collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = k - p + j;
            let den = knots[i + p + 1 - r] - knots[i];
            let a = if den.abs() < 1e-15 { 0.0 } else { (t - knots[i]) / den };
            d[j] = d[j - 1].lerp(d[j], a);
        }
    }
    d[p]
}

/// Read a DXF drawing's model space: LINE, ARC, CIRCLE, LWPOLYLINE /
/// POLYLINE (with bulges), SPLINE, ELLIPSE. Blocks (INSERT) and text are
/// left out and listed.
pub fn parse_dxf(text: &str) -> Result<Drawing, String> {
    let lines: Vec<&str> = text.lines().map(|l| l.trim()).collect();
    if lines.len() < 2 || !lines.chunks(2).any(|p| p[0] == "0" && p.get(1) == Some(&"SECTION")) {
        return Err("not a DXF drawing (only text DXF files can be read)".into());
    }
    let pairs: Vec<(i64, &str)> = lines.chunks_exact(2).filter_map(|p| p[0].parse().ok().map(|c| (c, p[1]))).collect();
    let mut unit = 1.0;
    let mut out = Drawing::default();
    let mut skipped: std::collections::BTreeMap<String, usize> = Default::default();
    // Split into sections, then entities (each starts at a code-0 pair).
    let mut section = "";
    let mut i = 0;
    while i < pairs.len() {
        let (code, val) = pairs[i];
        if code == 0 && (val == "ENDSEC" || val == "EOF") {
            section = "";
            i += 1;
            continue;
        }
        if code == 0 && val == "SECTION" {
            section = pairs.get(i + 1).map_or("", |p| p.1);
            i += 2;
            continue;
        }
        if section == "HEADER" && code == 9 && val == "$INSUNITS" {
            if let Some(u) = pairs.get(i + 1).and_then(|p| p.1.parse().ok()) {
                unit = dxf_unit(u);
            }
        }
        if section != "ENTITIES" || code != 0 {
            i += 1;
            continue;
        }
        // One entity: its pairs up to the next code 0.
        let kind = val;
        let start = i + 1;
        let mut end = start;
        while end < pairs.len() && pairs[end].0 != 0 {
            end += 1;
        }
        let body = &pairs[start..end];
        let f = |c: i64| body.iter().find(|p| p.0 == c).and_then(|p| p.1.parse::<f64>().ok());
        let all = |c: i64| -> Vec<f64> { body.iter().filter(|p| p.0 == c).filter_map(|p| p.1.parse().ok()).collect() };
        let pt = |x: i64, y: i64| DVec2::new(f(x).unwrap_or(0.0), f(y).unwrap_or(0.0));
        match kind {
            "LINE" => out.curves.push(Curve::Line(pt(10, 20), pt(11, 21))),
            "CIRCLE" => out.curves.push(Curve::Circle { c: pt(10, 20), r: f(40).unwrap_or(0.0) }),
            "ARC" => {
                let (a0, a1) = (f(50).unwrap_or(0.0).to_radians(), f(51).unwrap_or(0.0).to_radians());
                let mut sweep = (a1 - a0).rem_euclid(TAU);
                if sweep < 1e-12 {
                    sweep = TAU;
                }
                out.curves.push(Curve::Arc { c: pt(10, 20), r: f(40).unwrap_or(0.0), a0, sweep });
            }
            "LWPOLYLINE" => {
                // Vertices in order; a bulge (42) belongs to the vertex before it.
                let mut verts: Vec<(DVec2, f64)> = Vec::new();
                for &(c, v) in body {
                    let v: f64 = v.parse().unwrap_or(0.0);
                    match c {
                        10 => verts.push((DVec2::new(v, 0.0), 0.0)),
                        20 => {
                            if let Some(last) = verts.last_mut() {
                                last.0.y = v;
                            }
                        }
                        42 => {
                            if let Some(last) = verts.last_mut() {
                                last.1 = v;
                            }
                        }
                        _ => {}
                    }
                }
                let closed = f(70).is_some_and(|fl| fl as i64 & 1 == 1);
                polyline(&verts, closed, &mut out.curves);
            }
            "POLYLINE" => {
                // VERTEX entities follow, up to SEQEND.
                let closed = f(70).is_some_and(|fl| fl as i64 & 1 == 1);
                let mut verts: Vec<(DVec2, f64)> = Vec::new();
                let mut j = end;
                while j < pairs.len() && !(pairs[j].0 == 0 && pairs[j].1 == "SEQEND") {
                    if pairs[j].0 == 0 && pairs[j].1 == "VERTEX" {
                        let mut k = j + 1;
                        let (mut p, mut b) = (DVec2::ZERO, 0.0);
                        while k < pairs.len() && pairs[k].0 != 0 {
                            let v: f64 = pairs[k].1.parse().unwrap_or(0.0);
                            match pairs[k].0 {
                                10 => p.x = v,
                                20 => p.y = v,
                                42 => b = v,
                                _ => {}
                            }
                            k += 1;
                        }
                        verts.push((p, b));
                        j = k;
                    } else {
                        j += 1;
                    }
                }
                polyline(&verts, closed, &mut out.curves);
                end = j;
            }
            "SPLINE" => {
                let degree = f(71).unwrap_or(3.0) as usize;
                let knots = all(40);
                let ctrl: Vec<DVec2> = all(10).into_iter().zip(all(20)).map(|(x, y)| DVec2::new(x, y)).collect();
                let fit: Vec<DVec2> = all(11).into_iter().zip(all(21)).map(|(x, y)| DVec2::new(x, y)).collect();
                if fit.len() >= 2 {
                    out.curves.push(Curve::Fit(fit));
                } else if ctrl.len() == 4 && degree == 3 && knots.len() == 8 && knots[..4].iter().all(|k| *k == knots[0]) && knots[4..].iter().all(|k| *k == knots[7]) {
                    // One Bézier piece: exact.
                    out.curves.push(Curve::Bezier([ctrl[0], ctrl[1], ctrl[2], ctrl[3]]));
                } else if ctrl.len() >= 2 && knots.len() == ctrl.len() + degree.min(ctrl.len() - 1) + 1 {
                    // Anything else: a smooth curve through points along it.
                    let p = degree.min(ctrl.len() - 1);
                    let (t0, t1) = (knots[p], knots[ctrl.len()]);
                    let n = (ctrl.len() * 4).max(8);
                    out.curves.push(Curve::Fit((0..=n).map(|k| deboor(&ctrl, &knots, p, t0 + (t1 - t0) * k as f64 / n as f64)).collect()));
                } else {
                    *skipped.entry("bad spline".into()).or_default() += 1;
                }
            }
            "ELLIPSE" => {
                let (c, major, ratio) = (pt(10, 20), pt(11, 21), f(40).unwrap_or(1.0));
                let (s, e) = (f(41).unwrap_or(0.0), f(42).unwrap_or(TAU));
                if ((e - s).rem_euclid(TAU)).abs() < 1e-9 || (e - s - TAU).abs() < 1e-9 {
                    out.curves.push(Curve::Ellipse { c, major, ratio });
                } else {
                    let sweep = (e - s).rem_euclid(TAU);
                    let minor = major.perp() * ratio;
                    let n = 32;
                    out.curves.push(Curve::Fit((0..=n).map(|k| {
                        let t = s + sweep * k as f64 / n as f64;
                        c + major * t.cos() + minor * t.sin()
                    }).collect()));
                }
            }
            "POINT" | "SEQEND" | "VERTEX" => {}
            other => *skipped.entry(other.to_string()).or_default() += 1,
        }
        i = end;
    }
    if unit != 1.0 {
        out.curves = out.curves.iter().map(|c| Placement { scale: unit, ..Default::default() }.curve(c)).collect();
    }
    out.skipped = skipped
        .into_iter()
        .map(|(k, n)| match k.as_str() {
            "INSERT" => format!("{n} block{} (explode them first)", if n == 1 { "" } else { "s" }),
            "TEXT" | "MTEXT" => format!("{n} text"),
            _ => format!("{n} {k}"),
        })
        .collect();
    Ok(out)
}

fn polyline(verts: &[(DVec2, f64)], closed: bool, out: &mut Vec<Curve>) {
    let n = verts.len();
    let segs = if closed { n } else { n.saturating_sub(1) };
    for k in 0..segs {
        let (p, b) = verts[k];
        let q = verts[(k + 1) % n].0;
        if p.distance(q) < 1e-12 {
            continue;
        }
        out.push(if b.abs() > 1e-12 { bulge_arc(p, q, b) } else { Curve::Line(p, q) });
    }
}

/// Read a file by its extension.
pub fn read(path: &std::path::Path) -> Result<Drawing, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("couldn't read it ({e})"))?;
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("svg") => parse_svg(&text),
        Some("dxf") => parse_dxf(&text),
        _ => Err("only .svg and .dxf files".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::OriginPlane;

    fn near(a: DVec2, b: DVec2) -> bool {
        a.distance(b) < 1e-9
    }

    #[test]
    fn svg_in_mm_with_y_up_and_transforms() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="50mm" viewBox="0 0 200 100">
            <defs><path d="M 0 0 L 999 999"/></defs>
            <g transform="translate(10,20)">
              <rect x="0" y="0" width="20" height="10"/>
              <circle cx="40" cy="0" r="4"/>
            </g>
            <path d="M 0 0 C 10 0 20 10 20 20 Z"/>
            <text>hi</text>
          </svg>"#;
        let d = parse_svg(svg).unwrap();
        // viewBox 200 wide in 100 mm: half a mm a unit; y = -0.5 * y.
        let lines: Vec<_> = d.curves.iter().filter_map(|c| if let Curve::Line(p, q) = c { Some((*p, *q)) } else { None }).collect();
        assert!(lines.iter().any(|(p, q)| near(*p, DVec2::new(5.0, -10.0)) && near(*q, DVec2::new(15.0, -10.0))), "{lines:?}");
        assert!(d.curves.iter().any(|c| matches!(c, Curve::Circle { c, r } if near(*c, DVec2::new(25.0, -10.0)) && (r - 2.0).abs() < 1e-12)));
        assert!(d.curves.iter().any(|c| matches!(c, Curve::Bezier(b) if near(b[1], DVec2::new(5.0, 0.0)) && near(b[3], DVec2::new(10.0, -10.0)))));
        assert_eq!(d.curves.len(), 4 + 1 + 2, "rect, circle, curve + closing line; nothing from <defs>");
        assert_eq!(d.skipped.len(), 1, "the text is listed");
        // A rounded rectangle: four lines and four quarter arcs, joined.
        let d = parse_svg(r#"<svg width="40mm" height="20mm" viewBox="0 0 40 20"><rect x="0" y="0" width="40" height="20" rx="5"/></svg>"#).unwrap();
        assert_eq!(d.curves.iter().filter(|c| matches!(c, Curve::Line(..))).count(), 4);
        assert!(d.curves.iter().filter(|c| matches!(c, Curve::Arc { r, sweep, .. } if (r - 5.0).abs() < 1e-12 && (sweep.abs() - PI / 2.0).abs() < 1e-9)).count() == 4, "{:?}", d.curves);
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        add_to_sketch(&mut sk, &d.curves);
        assert_eq!(crate::profile::find_regions(&sk).len(), 1);
        // No size: 96 px to the inch.
        let d = parse_svg(r#"<svg><line x1="0" y1="0" x2="96" y2="0"/></svg>"#).unwrap();
        assert!(matches!(d.curves[0], Curve::Line(_, q) if near(q, DVec2::new(25.4, 0.0))));
        assert!(parse_svg("<html></html>").is_err());
    }

    #[test]
    fn dxf_entities_units_and_bulges() {
        let dxf = "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n1\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n\
            0\nLINE\n8\n0\n10\n0\n20\n0\n11\n1\n21\n0\n\
            0\nARC\n10\n0\n20\n0\n40\n1\n50\n0\n51\n90\n\
            0\nLWPOLYLINE\n90\n2\n70\n0\n10\n0\n20\n0\n42\n1\n10\n2\n20\n0\n\
            0\nTEXT\n1\nhello\n\
            0\nENDSEC\n0\nEOF\n";
        let d = parse_dxf(dxf).unwrap();
        assert_eq!(d.curves.len(), 3);
        // Inches to mm.
        assert!(matches!(d.curves[0], Curve::Line(p, q) if near(p, DVec2::ZERO) && near(q, DVec2::new(25.4, 0.0))));
        assert!(matches!(d.curves[1], Curve::Arc { r, sweep, .. } if (r - 25.4).abs() < 1e-9 && (sweep - PI / 2.0).abs() < 1e-12));
        // Bulge 1: a half circle counter-clockwise from (0,0) to (2,0) in: centre (1,0), below the chord.
        match d.curves[2] {
            Curve::Arc { c, r, a0, sweep } => {
                assert!(near(c, DVec2::new(25.4, 0.0)) && (r - 25.4).abs() < 1e-9, "{c} {r}");
                assert!((sweep - PI).abs() < 1e-12);
                let mid = c + DVec2::from_angle(a0 + sweep / 2.0) * r;
                assert!(mid.y < 0.0, "counter-clockwise from left to right goes underneath: {mid}");
            }
            ref other => panic!("{other:?}"),
        }
        assert_eq!(d.skipped, vec!["1 text".to_string()]);
        assert!(parse_dxf("hello").is_err());
    }

    #[test]
    fn curves_join_in_the_sketch_and_make_a_profile() {
        // A 20 × 10 rectangle with a rounded right end (two lines, an arc, a line, a Bézier).
        let curves = vec![
            Curve::Line(DVec2::new(0.0, 0.0), DVec2::new(20.0, 0.0)),
            Curve::Arc { c: DVec2::new(20.0, 5.0), r: 5.0, a0: -PI / 2.0, sweep: PI },
            Curve::Line(DVec2::new(20.0, 10.0), DVec2::new(0.0, 10.0)),
            Curve::Bezier([DVec2::new(0.0, 10.0), DVec2::new(-4.0, 7.0), DVec2::new(-4.0, 3.0), DVec2::new(0.0, 0.0)]),
            Curve::Circle { c: DVec2::new(10.0, 5.0), r: 2.0 },
        ];
        let placed: Vec<Curve> = curves.iter().map(|c| Placement { offset: DVec2::new(100.0, 0.0), angle: 90.0, scale: 2.0 }.curve(c)).collect();
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let before = sk.points.len();
        let first = add_to_sketch(&mut sk, &placed);
        assert_eq!(sk.entities.len() - first, 5);
        // 4 corners shared + arc centre + 2 Bézier pulls + circle centre.
        assert_eq!(sk.points.len() - before, 4 + 1 + 2 + 1);
        assert_eq!(sk.control_splines, vec![(0, 3)]);
        // Placed: (20, 0) scaled 2, turned 90°, moved 100 > (100, 40).
        assert!(sk.points.iter().any(|p| near(*p, DVec2::new(100.0, 40.0))));
        let regions = crate::profile::find_regions(&sk);
        assert_eq!(regions.len(), 2, "the outline (with the circle as a hole) and the circle");
    }

    #[test]
    fn our_own_dxf_and_svg_come_back_the_same() {
        let curves = vec![
            Curve::Line(DVec2::new(0.0, 0.0), DVec2::new(30.0, 0.0)),
            Curve::Arc { c: DVec2::new(30.0, 10.0), r: 10.0, a0: -PI / 2.0, sweep: PI },
            Curve::Line(DVec2::new(30.0, 20.0), DVec2::new(0.0, 20.0)),
            Curve::Line(DVec2::new(0.0, 20.0), DVec2::new(0.0, 0.0)),
            Curve::Circle { c: DVec2::new(10.0, 10.0), r: 3.0 },
        ];
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        add_to_sketch(&mut sk, &curves);
        for d in [parse_dxf(&crate::sketch_export::dxf(&sk)).unwrap(), parse_svg(&crate::sketch_export::svg(&sk)).unwrap()] {
            let mut back = Sketch::new("B".into(), OriginPlane::XY);
            add_to_sketch(&mut back, &d.curves);
            assert_eq!(crate::profile::find_regions(&back).len(), 2, "{:?}", d.curves);
            let (lo, hi) = bounds(&d.curves).unwrap();
            // The SVG may sit elsewhere (its page); its size is the same.
            assert!(((hi - lo) - DVec2::new(40.0, 20.0)).length() < 1e-6, "{lo} {hi}");
        }
    }
}
