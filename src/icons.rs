//! Toolbar icons, drawn as vector shapes (no image files), in the spirit of
//! Fusion 360's ribbon: grey line work with blue for solids and orange for
//! sketch geometry.

use eframe::egui;
use egui::{Color32, Pos2, Rect, Stroke};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    File,
    Save,
    Undo,
    Redo,
    Sketch,
    FinishSketch,
    Extrude,
    Revolve,
    Fillet,
    Chamfer,
    Construction,
    Trim,
    Mirror,
    Midpoint,
    Plane,
    Project,
    Axis,
    RectPattern,
    CircPattern,
    Polygon,
    Select,
    Line,
    Rectangle,
    Rectangle3Point,
    RectangleCenter,
    Point,
    Circle,
    Circle2Point,
    Circle3Point,
    Circle2Tangent,
    Circle3Tangent,
    Arc3Point,
    ArcCenter,
    Dimension,
    Horizontal,
    Vertical,
    Parallel,
    Perpendicular,
    Equal,
    Tangent,
    Fix,
    Grid,
}

const BLUE: Color32 = Color32::from_rgb(70, 145, 230);
const BLUE_DARK: Color32 = Color32::from_rgb(45, 100, 170);
const ORANGE: Color32 = Color32::from_rgb(240, 160, 50);
const GREEN: Color32 = Color32::from_rgb(90, 200, 110);

/// Draw `icon` inside `rect`. `ink` is the line colour (grey when disabled).
pub fn paint(painter: &egui::Painter, rect: Rect, icon: Icon, enabled: bool) {
    let ink = if enabled { Color32::from_gray(220) } else { Color32::from_gray(95) };
    let tint = |c: Color32| if enabled { c } else { Color32::from_gray(95) };
    let (blue, blue_dark, orange, green) = (tint(BLUE), tint(BLUE_DARK), tint(ORANGE), tint(GREEN));
    let w = rect.width().min(rect.height());
    let o = rect.center() - egui::vec2(w, w) * 0.5;
    // Points in a 0..1 box.
    let p = |x: f32, y: f32| Pos2::new(o.x + x * w, o.y + y * w);
    let thin = Stroke::new((w * 0.06).max(1.2), ink);
    let line = |a: (f32, f32), b: (f32, f32), s: Stroke| {
        painter.line_segment([p(a.0, a.1), p(b.0, b.1)], s);
    };
    let poly = |pts: &[(f32, f32)], fill: Color32| {
        painter.add(egui::Shape::convex_polygon(pts.iter().map(|&(x, y)| p(x, y)).collect(), fill, Stroke::NONE));
    };
    let outline = |pts: &[(f32, f32)], s: Stroke| {
        let mut v: Vec<Pos2> = pts.iter().map(|&(x, y)| p(x, y)).collect();
        v.push(v[0]);
        painter.add(egui::Shape::line(v, s));
    };
    // Arc from a0 to a1 (radians, screen y down) around (cx, cy).
    let arc = |cx: f32, cy: f32, r: f32, a0: f32, a1: f32, s: Stroke| {
        let pts: Vec<Pos2> = (0..=24)
            .map(|i| {
                let a = a0 + (a1 - a0) * i as f32 / 24.0;
                p(cx + r * a.cos(), cy + r * a.sin())
            })
            .collect();
        painter.add(egui::Shape::line(pts, s));
    };
    let dot = |x: f32, y: f32, c: Color32| {
        painter.circle_filled(p(x, y), w * 0.07, c);
    };
    let arrow_head = |tip: (f32, f32), toward: (f32, f32), c: Color32| {
        let (dx, dy) = (tip.0 - toward.0, tip.1 - toward.1);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let (ux, uy) = (dx / len, dy / len);
        let (s, b) = (0.16, 0.11);
        poly(
            &[tip, (tip.0 - ux * s - uy * b, tip.1 - uy * s + ux * b), (tip.0 - ux * s + uy * b, tip.1 - uy * s - ux * b)],
            c,
        );
    };

    match icon {
        Icon::File => {
            outline(&[(0.25, 0.1), (0.6, 0.1), (0.78, 0.28), (0.78, 0.9), (0.25, 0.9)], thin);
            line((0.6, 0.1), (0.6, 0.28), thin);
            line((0.6, 0.28), (0.78, 0.28), thin);
            line((0.35, 0.5), (0.68, 0.5), Stroke::new(thin.width * 0.8, blue));
            line((0.35, 0.65), (0.68, 0.65), Stroke::new(thin.width * 0.8, blue));
        }
        Icon::Save => {
            poly(&[(0.15, 0.15), (0.75, 0.15), (0.85, 0.25), (0.85, 0.85), (0.15, 0.85)], blue_dark);
            poly(&[(0.3, 0.15), (0.68, 0.15), (0.68, 0.4), (0.3, 0.4)], Color32::from_gray(230));
            poly(&[(0.27, 0.55), (0.73, 0.55), (0.73, 0.85), (0.27, 0.85)], Color32::from_gray(245));
            line((0.58, 0.2), (0.58, 0.35), Stroke::new(thin.width, blue_dark));
        }
        Icon::Undo | Icon::Redo => {
            let s = Stroke::new(thin.width * 1.4, ink);
            // A hook: down-turned arc with the arrow on the left (undo) or right (redo).
            arc(0.5, 0.62, 0.28, std::f32::consts::PI, 2.0 * std::f32::consts::PI, s);
            let (tip, from) = if icon == Icon::Undo { ((0.22, 0.75), (0.22, 0.6)) } else { ((0.78, 0.75), (0.78, 0.6)) };
            line(from, tip, s);
            arrow_head((tip.0, tip.1 + 0.06), (tip.0, tip.1 - 0.1), ink);
        }
        Icon::Sketch => {
            outline(&[(0.12, 0.3), (0.72, 0.3), (0.72, 0.88), (0.12, 0.88)], thin);
            // pencil
            poly(&[(0.55, 0.62), (0.88, 0.18), (0.98, 0.26), (0.65, 0.7)], orange);
            poly(&[(0.55, 0.62), (0.65, 0.7), (0.5, 0.76)], ink);
        }
        Icon::FinishSketch => {
            let s = Stroke::new(w * 0.14, green);
            line((0.18, 0.52), (0.42, 0.76), s);
            line((0.42, 0.76), (0.85, 0.25), s);
        }
        Icon::Extrude => {
            // An isometric block with an up arrow.
            poly(&[(0.15, 0.55), (0.5, 0.4), (0.85, 0.55), (0.5, 0.7)], blue);
            poly(&[(0.15, 0.55), (0.5, 0.7), (0.5, 0.92), (0.15, 0.77)], blue_dark);
            poly(&[(0.5, 0.7), (0.85, 0.55), (0.85, 0.77), (0.5, 0.92)], Color32::from_rgb(60, 125, 205));
            line((0.5, 0.52), (0.5, 0.14), Stroke::new(thin.width * 1.2, ink));
            arrow_head((0.5, 0.06), (0.5, 0.3), ink);
        }
        Icon::Revolve => {
            // A half-profile beside an axis, with a turning arrow.
            line((0.5, 0.06), (0.5, 0.94), Stroke::new(thin.width, ink));
            poly(&[(0.56, 0.3), (0.82, 0.3), (0.82, 0.78), (0.56, 0.78)], blue);
            let s = Stroke::new(thin.width * 1.2, orange);
            arc(0.5, 0.2, 0.3, 0.2, 2.8, s);
            arrow_head((0.2, 0.26), (0.3, 0.38), orange);
        }
        Icon::Fillet => {
            // A block whose top-right corner is rounded (still convex).
            let mut pts = vec![(0.15, 0.85), (0.15, 0.15), (0.5, 0.15)];
            for i in 1..=8 {
                let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_2 * i as f32 / 8.0;
                pts.push((0.5 + 0.35 * a.cos(), 0.5 + 0.35 * a.sin()));
            }
            pts.push((0.85, 0.85));
            poly(&pts, blue);
            arc(0.5, 0.5, 0.35, -std::f32::consts::FRAC_PI_2, 0.0, Stroke::new(thin.width * 1.4, orange));
            line((0.15, 0.85), (0.85, 0.85), thin);
        }
        Icon::Chamfer => {
            poly(&[(0.15, 0.85), (0.15, 0.15), (0.5, 0.15), (0.85, 0.5), (0.85, 0.85)], blue);
            line((0.5, 0.15), (0.85, 0.5), Stroke::new(thin.width * 1.4, orange));
            line((0.15, 0.85), (0.85, 0.85), thin);
        }
        Icon::Polygon => {
            let pts: Vec<(f32, f32)> = (0..6)
                .map(|k| {
                    let a = std::f32::consts::TAU * k as f32 / 6.0 - std::f32::consts::FRAC_PI_2;
                    (0.5 + 0.36 * a.cos(), 0.5 + 0.36 * a.sin())
                })
                .collect();
            outline(&pts, Stroke::new(thin.width * 1.2, orange));
            dot(0.5, 0.5, ink);
        }
        Icon::Trim => {
            // A line crossing another, the overhang struck out.
            line((0.1, 0.5), (0.62, 0.5), Stroke::new(thin.width * 1.2, orange));
            line((0.62, 0.5), (0.9, 0.5), Stroke::new(thin.width, Color32::from_gray(120)));
            line((0.62, 0.15), (0.62, 0.85), thin);
            line((0.74, 0.3), (0.86, 0.7), Stroke::new(thin.width * 1.2, ink));
            line((0.86, 0.3), (0.74, 0.7), Stroke::new(thin.width * 1.2, ink));
        }
        Icon::Mirror => {
            // A shape and its reflection either side of a dashed axis.
            let dash = Stroke::new(thin.width, Color32::from_gray(140));
            for k in 0..4 {
                let y = 0.1 + 0.22 * k as f32;
                line((0.5, y), (0.5, y + 0.12), dash);
            }
            let s = Stroke::new(thin.width * 1.2, orange);
            line((0.12, 0.7), (0.4, 0.7), s);
            line((0.4, 0.7), (0.4, 0.3), s);
            line((0.4, 0.3), (0.12, 0.7), s);
            line((0.88, 0.7), (0.6, 0.7), Stroke::new(thin.width * 1.2, ink));
            line((0.6, 0.7), (0.6, 0.3), Stroke::new(thin.width * 1.2, ink));
            line((0.6, 0.3), (0.88, 0.7), Stroke::new(thin.width * 1.2, ink));
        }
        Icon::Axis => {
            // A long line with a small cross-mark at each end and a dot on it.
            line((0.15, 0.85), (0.85, 0.15), Stroke::new(thin.width * 1.3, orange));
            line((0.08, 0.72), (0.28, 0.92), thin);
            line((0.72, 0.08), (0.92, 0.28), thin);
            dot(0.5, 0.5, ink);
        }
        Icon::Project => {
            // A line above, an arrow dropping onto its shadow below.
            line((0.15, 0.25), (0.85, 0.25), Stroke::new(thin.width * 1.2, ink));
            line((0.5, 0.3), (0.5, 0.68), thin);
            line((0.4, 0.58), (0.5, 0.7), thin);
            line((0.6, 0.58), (0.5, 0.7), thin);
            line((0.15, 0.82), (0.85, 0.82), Stroke::new(thin.width * 1.4, orange));
        }
        Icon::Plane => {
            // A skewed square with an arrow rising off it.
            let (a, b, c, d) = ((0.15, 0.7), (0.65, 0.7), (0.85, 0.45), (0.35, 0.45));
            for (p, q) in [(a, b), (b, c), (c, d), (d, a)] {
                line(p, q, Stroke::new(thin.width * 1.2, orange));
            }
            line((0.5, 0.55), (0.5, 0.12), thin);
            line((0.42, 0.22), (0.5, 0.1), thin);
            line((0.58, 0.22), (0.5, 0.1), thin);
        }
        Icon::Midpoint => {
            // A line with a dot at its middle and two small ticks.
            line((0.1, 0.65), (0.9, 0.65), Stroke::new(thin.width * 1.2, ink));
            dot(0.5, 0.65, orange);
            line((0.3, 0.45), (0.3, 0.85), thin);
            line((0.7, 0.45), (0.7, 0.85), thin);
            line((0.4, 0.25), (0.6, 0.25), Stroke::new(thin.width, orange));
        }
        Icon::RectPattern => {
            // A grid of small squares, the first one the original.
            for r in 0..2 {
                for c in 0..3 {
                    let (x, y) = (0.12 + 0.27 * c as f32, 0.2 + 0.36 * r as f32);
                    let col = if r == 0 && c == 0 { orange } else { ink };
                    for (a, b) in [((x, y), (x + 0.18, y)), ((x + 0.18, y), (x + 0.18, y + 0.24)), ((x + 0.18, y + 0.24), (x, y + 0.24)), ((x, y + 0.24), (x, y))] {
                        line(a, b, Stroke::new(thin.width * 1.2, col));
                    }
                }
            }
        }
        Icon::CircPattern => {
            // Small squares round a ring.
            for k in 0..5 {
                let a = std::f32::consts::TAU * k as f32 / 5.0 - 1.2;
                let (cx, cy) = (0.5 + 0.32 * a.cos(), 0.5 + 0.32 * a.sin());
                let col = if k == 0 { orange } else { ink };
                for (p, q) in [((-1.0, -1.0), (1.0, -1.0)), ((1.0, -1.0), (1.0, 1.0)), ((1.0, 1.0), (-1.0, 1.0)), ((-1.0, 1.0), (-1.0, -1.0))] {
                    line((cx + p.0 * 0.07, cy + p.1 * 0.07), (cx + q.0 * 0.07, cy + q.1 * 0.07), Stroke::new(thin.width * 1.2, col));
                }
            }
        }
        Icon::Construction => {
            // A dashed line with its end dots.
            let s = Stroke::new(thin.width * 1.2, orange);
            for k in 0..3 {
                let t = |u: f32| (0.15 + 0.7 * u, 0.82 - 0.64 * u);
                line(t(k as f32 / 3.0 + 0.02), t(k as f32 / 3.0 + 0.2), s);
            }
            dot(0.15, 0.82, ink);
            dot(0.85, 0.18, ink);
        }
        Icon::Select => {
            poly(&[(0.25, 0.1), (0.25, 0.78), (0.42, 0.62), (0.6, 0.62)], ink);
            line((0.4, 0.6), (0.55, 0.88), Stroke::new(thin.width * 1.5, ink));
        }
        Icon::Line => {
            line((0.15, 0.82), (0.85, 0.18), Stroke::new(thin.width * 1.2, orange));
            dot(0.15, 0.82, ink);
            dot(0.85, 0.18, ink);
        }
        Icon::Rectangle => {
            outline(&[(0.15, 0.25), (0.85, 0.25), (0.85, 0.75), (0.15, 0.75)], Stroke::new(thin.width * 1.2, orange));
            dot(0.15, 0.25, ink);
            dot(0.85, 0.75, ink);
        }
        Icon::Rectangle3Point => {
            // A tilted rectangle, three corner dots.
            let (a, b, c, d) = ((0.1, 0.62), (0.62, 0.18), (0.9, 0.5), (0.38, 0.94));
            outline(&[a, b, c, d], Stroke::new(thin.width * 1.2, orange));
            dot(a.0, a.1, ink);
            dot(b.0, b.1, ink);
            dot(c.0, c.1, ink);
        }
        Icon::RectangleCenter => {
            outline(&[(0.12, 0.25), (0.88, 0.25), (0.88, 0.75), (0.12, 0.75)], Stroke::new(thin.width * 1.2, orange));
            dot(0.5, 0.5, ink);
            dot(0.88, 0.75, ink);
            line((0.5, 0.5), (0.88, 0.75), Stroke::new(thin.width * 0.7, ink));
        }
        Icon::Point => {
            let s = Stroke::new(thin.width, ink);
            line((0.5, 0.15), (0.5, 0.35), s);
            line((0.5, 0.65), (0.5, 0.85), s);
            line((0.15, 0.5), (0.35, 0.5), s);
            line((0.65, 0.5), (0.85, 0.5), s);
            painter.circle_filled(p(0.5, 0.5), w * 0.11, orange);
        }
        Icon::Circle => {
            painter.circle_stroke(p(0.5, 0.5), w * 0.36, Stroke::new(thin.width * 1.2, orange));
            dot(0.5, 0.5, ink);
        }
        Icon::Circle2Point => {
            painter.circle_stroke(p(0.5, 0.5), w * 0.36, Stroke::new(thin.width * 1.2, orange));
            line((0.14, 0.5), (0.86, 0.5), Stroke::new(thin.width * 0.8, ink));
            dot(0.14, 0.5, ink);
            dot(0.86, 0.5, ink);
        }
        Icon::Circle3Point => {
            painter.circle_stroke(p(0.5, 0.5), w * 0.36, Stroke::new(thin.width * 1.2, orange));
            for a in [-1.9f32, -0.2, 1.9] {
                dot(0.5 + 0.36 * a.cos(), 0.5 + 0.36 * a.sin(), ink);
            }
        }
        Icon::Circle2Tangent => {
            // A corner with a circle sitting in it, touching both sides.
            let s = Stroke::new(thin.width * 1.2, ink);
            line((0.1, 0.9), (0.1, 0.1), s);
            line((0.1, 0.9), (0.9, 0.9), s);
            let r = 0.27;
            painter.circle_stroke(p(0.1 + r, 0.9 - r), w * r - thin.width * 0.6, Stroke::new(thin.width * 1.2, orange));
        }
        Icon::Circle3Tangent => {
            // A triangle with its incircle.
            let s = Stroke::new(thin.width * 1.2, ink);
            let (a, b, c) = ((0.5, 0.08), (0.06, 0.88), (0.94, 0.88));
            line(a, b, s);
            line(b, c, s);
            line(c, a, s);
            // incircle of that triangle
            let la = ((b.0 - c.0) as f32).hypot(b.1 - c.1);
            let lb = ((a.0 - c.0) as f32).hypot(a.1 - c.1);
            let lc = ((a.0 - b.0) as f32).hypot(a.1 - b.1);
            let per = la + lb + lc;
            let cx = (a.0 * la + b.0 * lb + c.0 * lc) / per;
            let cy = (a.1 * la + b.1 * lb + c.1 * lc) / per;
            let area = ((b.0 - a.0) * (c.1 - a.1) - (c.0 - a.0) * (b.1 - a.1)).abs() * 0.5;
            let r = 2.0 * area / per;
            painter.circle_stroke(p(cx, cy), w * r - thin.width * 0.6, Stroke::new(thin.width * 1.2, orange));
        }
        Icon::Arc3Point => {
            arc(0.5, 0.62, 0.38, std::f32::consts::PI, 2.0 * std::f32::consts::PI, Stroke::new(thin.width * 1.2, orange));
            dot(0.12, 0.62, ink);
            dot(0.5, 0.24, ink);
            dot(0.88, 0.62, ink);
        }
        Icon::ArcCenter => {
            arc(0.3, 0.7, 0.55, -std::f32::consts::FRAC_PI_2, 0.0, Stroke::new(thin.width * 1.2, orange));
            line((0.3, 0.7), (0.85, 0.7), Stroke::new(thin.width * 0.7, ink));
            dot(0.3, 0.7, ink);
            dot(0.85, 0.7, ink);
            dot(0.3, 0.15, ink);
        }
        Icon::Dimension => {
            line((0.12, 0.25), (0.12, 0.8), thin);
            line((0.88, 0.25), (0.88, 0.8), thin);
            line((0.2, 0.55), (0.8, 0.55), thin);
            arrow_head((0.14, 0.55), (0.4, 0.55), ink);
            arrow_head((0.86, 0.55), (0.6, 0.55), ink);
        }
        Icon::Horizontal => {
            line((0.12, 0.5), (0.88, 0.5), Stroke::new(thin.width * 1.4, orange));
            line((0.3, 0.35), (0.3, 0.65), thin);
            line((0.7, 0.35), (0.7, 0.65), thin);
        }
        Icon::Vertical => {
            line((0.5, 0.12), (0.5, 0.88), Stroke::new(thin.width * 1.4, orange));
            line((0.35, 0.3), (0.65, 0.3), thin);
            line((0.35, 0.7), (0.65, 0.7), thin);
        }
        Icon::Parallel => {
            let s = Stroke::new(thin.width * 1.3, orange);
            line((0.2, 0.85), (0.6, 0.15), s);
            line((0.42, 0.85), (0.82, 0.15), s);
        }
        Icon::Perpendicular => {
            let s = Stroke::new(thin.width * 1.3, orange);
            line((0.15, 0.82), (0.85, 0.82), s);
            line((0.5, 0.82), (0.5, 0.15), s);
            outline(&[(0.5, 0.82), (0.62, 0.82), (0.62, 0.7), (0.5, 0.7)], thin);
        }
        Icon::Equal => {
            let s = Stroke::new(thin.width * 1.5, orange);
            line((0.2, 0.38), (0.8, 0.38), s);
            line((0.2, 0.62), (0.8, 0.62), s);
        }
        Icon::Tangent => {
            // The line touches the top of the circle at exactly one point:
            // the circle's outer edge (radius + half its stroke) meets the line.
            let s = Stroke::new(thin.width * 1.2, orange);
            let r = w * 0.3;
            let top = 0.3 * w + s.width * 0.5 + r; // centre so the outer edge sits on y = 0.3
            painter.circle_stroke(Pos2::new(o.x + 0.5 * w, o.y + top), r, s);
            line((0.08, 0.3), (0.92, 0.3), Stroke::new(thin.width * 1.2, ink));
        }
        Icon::Fix => {
            arc(0.5, 0.42, 0.18, std::f32::consts::PI, 2.0 * std::f32::consts::PI, Stroke::new(thin.width * 1.4, ink));
            line((0.32, 0.42), (0.32, 0.5), Stroke::new(thin.width * 1.4, ink));
            line((0.68, 0.42), (0.68, 0.5), Stroke::new(thin.width * 1.4, ink));
            poly(&[(0.24, 0.5), (0.76, 0.5), (0.76, 0.88), (0.24, 0.88)], Color32::from_rgb(230, 80, 80));
        }
        Icon::Grid => {
            for t in [0.2, 0.5, 0.8] {
                line((t, 0.12), (t, 0.88), thin);
                line((0.12, t), (0.88, t), thin);
            }
            dot(0.5, 0.5, blue);
        }
    }
}

/// A square icon button with a tooltip. `selected` draws it pressed in.
pub fn button(ui: &mut egui::Ui, icon: Icon, tooltip: &str, enabled: bool, selected: bool) -> egui::Response {
    let size = egui::vec2(34.0, 34.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let bg = if selected {
        Some(Color32::from_rgb(40, 80, 130))
    } else if enabled && response.hovered() {
        Some(Color32::from_gray(60))
    } else {
        None
    };
    if let Some(c) = bg {
        ui.painter().rect_filled(rect, 4.0, c);
    }
    paint(ui.painter(), rect.shrink(5.0), icon, enabled);
    response.on_hover_text(tooltip)
}

/// A narrow ▾ button that opens a menu next to an icon button.
pub fn dropdown(ui: &mut egui::Ui, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 34.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 3.0, Color32::from_gray(60));
    }
    let c = rect.center();
    let ink = Color32::from_gray(200);
    ui.painter().add(egui::Shape::convex_polygon(
        vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)],
        ink,
        Stroke::NONE,
    ));
    response.on_hover_text(tooltip)
}

/// A timeline entry: the feature's icon with a coloured bar under it (the
/// colour of the body it worked on). `dim` greys it out (rolled back);
/// `warn` marks a feature that failed.
pub fn timeline_button(
    ui: &mut egui::Ui,
    icon: Icon,
    tooltip: &str,
    selected: bool,
    underline: Option<Color32>,
    dim: bool,
    warn: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 36.0), egui::Sense::click());
    let body = Rect::from_min_size(rect.min, egui::vec2(32.0, 30.0));
    let bg = if selected {
        Some(Color32::from_rgb(40, 80, 130))
    } else if response.hovered() {
        Some(Color32::from_gray(60))
    } else {
        None
    };
    if let Some(c) = bg {
        ui.painter().rect_filled(body, 4.0, c);
    }
    paint(ui.painter(), body.shrink(4.0), icon, !dim);
    if let Some(c) = underline {
        let c = if dim { c.gamma_multiply(0.35) } else { c };
        ui.painter().rect_filled(Rect::from_min_size(rect.min + egui::vec2(3.0, 31.0), egui::vec2(26.0, 3.5)), 1.5, c);
    }
    if warn {
        ui.painter().circle_filled(body.right_top() + egui::vec2(-4.0, 4.0), 4.0, Color32::from_rgb(230, 80, 80));
    }
    response.on_hover_text(tooltip)
}
