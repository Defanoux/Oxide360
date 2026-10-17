//! A sketch saved as a 2-D drawing (right-click a sketch → Save As DXF /
//! SVG, as in Fusion), in the sketch's own coordinates, millimetres.
//! Construction geometry is left out. Lines, circles and arcs stay exact;
//! splines, ellipses and text outlines go as polylines.

use crate::sketch::{Entity, Sketch};
use glam::DVec2;
use std::fmt::Write;

/// One piece of drawing.
#[derive(Clone, Debug, PartialEq)]
enum Piece {
    Line(DVec2, DVec2),
    Circle(DVec2, f64),
    /// Centre, radius, start angle and sweep (radians, counter-clockwise).
    Arc(DVec2, f64, f64, f64),
    /// Points along a curve, and whether it closes.
    Poly(Vec<DVec2>, bool),
    Point(DVec2),
}

fn pieces(sk: &Sketch) -> Vec<Piece> {
    let mut out = Vec::new();
    for (e, ent) in sk.entities.iter().enumerate() {
        if sk.is_construction(e) {
            continue;
        }
        match *ent {
            Entity::Line { a, b } => out.push(Piece::Line(sk.points[a], sk.points[b])),
            Entity::Circle { center, radius } => out.push(Piece::Circle(sk.points[center], radius)),
            Entity::Arc { .. } => {
                if let Some((c, r, a0, sweep)) = sk.arc_geometry(e) {
                    out.push(Piece::Arc(c, r, a0, sweep));
                }
            }
            Entity::Point { p } => out.push(Piece::Point(sk.points[p])),
            Entity::Spline { .. } => out.push(Piece::Poly(sk.entity_polyline(e), false)),
            Entity::Ellipse { .. } => out.push(Piece::Poly(sk.entity_polyline(e), true)),
            Entity::Text { .. } => out.extend(sk.text_contours(e).into_iter().map(|c| Piece::Poly(c.pts, true))),
        }
    }
    out
}

/// The sketch as an AutoCAD R12 DXF (readable by about everything: laser
/// cutters, Inkscape, LibreCAD, Fusion).
pub fn dxf(sk: &Sketch) -> String {
    let mut s = String::new();
    let mut g = |code: i32, value: String| {
        let _ = writeln!(s, "{code}\n{value}");
    };
    g(0, "SECTION".into());
    g(2, "HEADER".into());
    g(9, "$ACADVER".into());
    g(1, "AC1009".into());
    g(9, "$INSUNITS".into());
    g(70, "4".into()); // millimetres
    g(0, "ENDSEC".into());
    g(0, "SECTION".into());
    g(2, "ENTITIES".into());
    let f = |x: f64| format!("{x:.6}");
    for p in pieces(sk) {
        match p {
            Piece::Line(a, b) => {
                g(0, "LINE".into());
                g(8, "0".into());
                for (code, v) in [(10, a.x), (20, a.y), (30, 0.0), (11, b.x), (21, b.y), (31, 0.0)] {
                    g(code, f(v));
                }
            }
            Piece::Circle(c, r) => {
                g(0, "CIRCLE".into());
                g(8, "0".into());
                for (code, v) in [(10, c.x), (20, c.y), (30, 0.0), (40, r)] {
                    g(code, f(v));
                }
            }
            Piece::Arc(c, r, a0, sweep) => {
                g(0, "ARC".into());
                g(8, "0".into());
                let (start, end) = (a0.to_degrees().rem_euclid(360.0), (a0 + sweep).to_degrees().rem_euclid(360.0));
                for (code, v) in [(10, c.x), (20, c.y), (30, 0.0), (40, r), (50, start), (51, end)] {
                    g(code, f(v));
                }
            }
            Piece::Poly(pts, closed) => {
                if pts.len() < 2 {
                    continue;
                }
                g(0, "POLYLINE".into());
                g(8, "0".into());
                g(66, "1".into());
                g(70, if closed { "1" } else { "0" }.into());
                for q in pts {
                    g(0, "VERTEX".into());
                    g(8, "0".into());
                    g(10, f(q.x));
                    g(20, f(q.y));
                    g(30, f(0.0));
                }
                g(0, "SEQEND".into());
            }
            Piece::Point(q) => {
                g(0, "POINT".into());
                g(8, "0".into());
                for (code, v) in [(10, q.x), (20, q.y), (30, 0.0)] {
                    g(code, f(v));
                }
            }
        }
    }
    g(0, "ENDSEC".into());
    g(0, "EOF".into());
    s
}

/// The sketch as an SVG, 1 unit = 1 mm, y up as in the sketch.
pub fn svg(sk: &Sketch) -> String {
    let all = pieces(sk);
    // The drawing's extent.
    let mut lo = DVec2::splat(f64::MAX);
    let mut hi = DVec2::splat(f64::MIN);
    let mut grow = |p: DVec2| {
        lo = lo.min(p);
        hi = hi.max(p);
    };
    for p in &all {
        match p {
            Piece::Line(a, b) => {
                grow(*a);
                grow(*b);
            }
            Piece::Circle(c, r) => {
                grow(*c - DVec2::splat(*r));
                grow(*c + DVec2::splat(*r));
            }
            Piece::Arc(c, r, a0, sweep) => (0..=64).for_each(|k| grow(*c + DVec2::from_angle(a0 + sweep * k as f64 / 64.0) * *r)),
            Piece::Poly(pts, _) => pts.iter().for_each(|q| grow(*q)),
            Piece::Point(q) => grow(*q),
        }
    }
    if lo.x > hi.x {
        (lo, hi) = (DVec2::ZERO, DVec2::splat(1.0));
    }
    let pad = ((hi - lo).max_element() * 0.02).max(1.0);
    let (lo, hi) = (lo - DVec2::splat(pad), hi + DVec2::splat(pad));
    let size = hi - lo;
    // Flipped so y is up: a point (x, y) is drawn at (x, −y).
    let p = |q: DVec2| format!("{:.4} {:.4}", q.x, -q.y);
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w:.4}mm" height="{h:.4}mm" viewBox="{x:.4} {y:.4} {w:.4} {h:.4}">"#,
        w = size.x,
        h = size.y,
        x = lo.x,
        y = -hi.y
    );
    let _ = writeln!(s, r#"<g fill="none" stroke="black" stroke-width="0.25" stroke-linejoin="round" stroke-linecap="round">"#);
    for piece in &all {
        match piece {
            Piece::Line(a, b) => {
                let _ = writeln!(s, r#"<path d="M {} L {}"/>"#, p(*a), p(*b));
            }
            Piece::Circle(c, r) => {
                let _ = writeln!(s, r#"<circle cx="{:.4}" cy="{:.4}" r="{:.4}"/>"#, c.x, -c.y, r);
            }
            Piece::Arc(c, r, a0, sweep) => {
                let (a, b) = (*c + DVec2::from_angle(*a0) * *r, *c + DVec2::from_angle(a0 + sweep) * *r);
                // Counter-clockwise in the sketch is counter-clockwise on the page too,
                // which (y down) is SVG's negative direction: sweep flag 0.
                let large = if *sweep > std::f64::consts::PI { 1 } else { 0 };
                let _ = writeln!(s, r#"<path d="M {} A {r:.4} {r:.4} 0 {large} 0 {}"/>"#, p(a), p(b));
            }
            Piece::Poly(pts, closed) if pts.len() >= 2 => {
                let mut d = format!("M {}", p(pts[0]));
                for q in &pts[1..] {
                    let _ = write!(d, " L {}", p(*q));
                }
                if *closed {
                    d.push_str(" Z");
                }
                let _ = writeln!(s, r#"<path d="{d}"/>"#);
            }
            Piece::Poly(..) => {}
            Piece::Point(q) => {
                let _ = writeln!(s, r#"<circle cx="{:.4}" cy="{:.4}" r="0.3" fill="black" stroke="none"/>"#, q.x, -q.y);
            }
        }
    }
    s.push_str("</g>\n</svg>\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::OriginPlane;

    /// A 40 × 20 rectangle's bottom edge, a circle r 5 at (10, 10), a quarter
    /// arc r 8 round (30, 10) from 0° to 90°, and a construction line.
    fn sample() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let pt = |sk: &mut Sketch, x: f64, y: f64| {
            sk.points.push(DVec2::new(x, y));
            sk.points.len() - 1
        };
        let (a, b) = (pt(&mut sk, 0.0, 0.0), pt(&mut sk, 40.0, 0.0));
        sk.entities.push(Entity::Line { a, b });
        let c = pt(&mut sk, 10.0, 10.0);
        sk.entities.push(Entity::Circle { center: c, radius: 5.0 });
        let (ac, a0, a1) = (pt(&mut sk, 30.0, 10.0), pt(&mut sk, 38.0, 10.0), pt(&mut sk, 30.0, 18.0));
        sk.entities.push(Entity::Arc { center: ac, a: a0, b: a1 });
        let (p, q) = (pt(&mut sk, 0.0, 20.0), pt(&mut sk, 40.0, 20.0));
        sk.entities.push(Entity::Line { a: p, b: q });
        sk.construction = vec![false, false, false, true];
        sk
    }

    /// (group code, value) pairs.
    fn groups(dxf: &str) -> Vec<(i32, String)> {
        let lines: Vec<&str> = dxf.lines().collect();
        lines.chunks(2).map(|c| (c[0].trim().parse().unwrap(), c[1].trim().to_string())).collect()
    }

    fn after(g: &[(i32, String)], entity: &str, code: i32) -> f64 {
        let at = g.iter().position(|(c, v)| *c == 0 && v == entity).unwrap_or_else(|| panic!("no {entity}"));
        g[at + 1..].iter().find(|(c, _)| *c == code).unwrap().1.parse().unwrap()
    }

    #[test]
    fn dxf_keeps_lines_circles_and_arcs_exact_and_leaves_construction_out() {
        let d = dxf(&sample());
        let g = groups(&d);
        assert_eq!(g.first(), Some(&(0, "SECTION".to_string())));
        assert_eq!(g.last(), Some(&(0, "EOF".to_string())));
        assert_eq!(g.iter().filter(|(c, v)| *c == 0 && v == "LINE").count(), 1, "the construction line is left out");
        assert_eq!(after(&g, "LINE", 11), 40.0);
        assert_eq!((after(&g, "CIRCLE", 10), after(&g, "CIRCLE", 20), after(&g, "CIRCLE", 40)), (10.0, 10.0, 5.0));
        assert_eq!((after(&g, "ARC", 40), after(&g, "ARC", 50), after(&g, "ARC", 51)), (8.0, 0.0, 90.0));
    }

    #[test]
    fn svg_is_in_millimetres_with_y_up() {
        let s = svg(&sample());
        assert!(s.starts_with("<svg") && s.trim_end().ends_with("</svg>"));
        assert!(s.contains(r#"<circle cx="10.0000" cy="-10.0000" r="5.0000"/>"#), "{s}");
        // The quarter arc: from (38, 10) to (30, 18), short way, counter-clockwise.
        assert!(s.contains(r#"<path d="M 38.0000 -10.0000 A 8.0000 8.0000 0 0 0 30.0000 -18.0000"/>"#), "{s}");
        assert_eq!(s.matches("<path").count(), 2, "line and arc; the construction line is left out");
        // 40 wide plus a 1 mm margin each side.
        assert!(s.contains(r#"width="42.0000mm""#), "{s}");
    }

    #[test]
    fn text_goes_as_closed_outlines() {
        let mut sk = Sketch::new("T".into(), OriginPlane::XY);
        sk.add_text(crate::sketch::Anchor::Free(DVec2::ZERO), crate::sketch::SketchText { text: "O".into(), height: 10.0, font: String::new(), bold: false, italic: false, angle: 0.0 });
        let g = groups(&dxf(&sk));
        // An O is two loops: outside and inside.
        assert_eq!(g.iter().filter(|(c, v)| *c == 0 && v == "POLYLINE").count(), 2);
        assert!(svg(&sk).matches(" Z\"").count() == 2);
    }

    #[test]
    #[ignore]
    fn write_samples() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut sk = sample();
        sk.add_text(crate::sketch::Anchor::Free(DVec2::new(2.0, 22.0)), crate::sketch::SketchText { text: "Hi".into(), height: 8.0, font: String::new(), bold: false, italic: false, angle: 0.0 });
        std::fs::write(format!("{dir}/sample.dxf"), dxf(&sk)).unwrap();
        std::fs::write(format!("{dir}/sample.svg"), svg(&sk)).unwrap();
    }
}
