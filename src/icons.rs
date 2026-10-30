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
    Extend,
    Measure,
    Offset,
    Slot,
    Mirror,
    Midpoint,
    Symmetric,
    ArcTangent,
    Ellipse,
    Text,
    Section,
    Midplane,
    Plane3Points,
    TangentPlane,
    AxisCylinder,
    Axis2Points,
    ConstructPoint,
    Sweep,
    Loft,
    Pipe,
    Coil,
    Import,
    Joint,
    Appearance,
    Split,
    Component,
    Interference,
    Break,
    ControlSpline,
    Plane,
    PlaneAngle,
    Hole,
    Thread,
    Shell,
    PressPull,
    Combine,
    Move,
    Parameters,
    HoleSimple,
    HoleCounterbore,
    HoleCountersink,
    TapSimple,
    TapClearance,
    TapTapped,
    TapTaper,
    DrillFlat,
    DrillAngle,
    ThreadFull,
    ThreadPartial,
    ExtentDistance,
    ExtentTo,
    ExtentAll,
    Flip,
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
    Remove,
    Coincident,
    Collinear,
    Concentric,
    Folder,
    Gear,
    Part,
    Spline,
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
            poly(&[(0.5, 0.7), (0.85, 0.55), (0.85, 0.77), (0.5, 0.92)], tint(Color32::from_rgb(60, 125, 205)));
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
        Icon::Measure => {
            // Two points with a dimension line and end ticks.
            line((0.15, 0.7), (0.85, 0.3), Stroke::new(thin.width * 1.2, orange));
            line((0.1, 0.55), (0.2, 0.85), thin);
            line((0.8, 0.15), (0.9, 0.45), thin);
            dot(0.15, 0.7, ink);
            dot(0.85, 0.3, ink);
        }
        Icon::Extend => {
            // A short line running on (dashed) to a wall.
            line((0.1, 0.5), (0.45, 0.5), Stroke::new(thin.width * 1.2, orange));
            line((0.45, 0.5), (0.78, 0.5), Stroke::new(thin.width, Color32::from_gray(120)));
            line((0.78, 0.15), (0.78, 0.85), thin);
            line((0.66, 0.4), (0.78, 0.5), thin);
            line((0.66, 0.6), (0.78, 0.5), thin);
        }
        Icon::Offset => {
            // A shape and a bigger copy round it.
            outline(&[(0.3, 0.3), (0.7, 0.3), (0.7, 0.7), (0.3, 0.7)], Stroke::new(thin.width * 1.2, orange));
            outline(&[(0.12, 0.12), (0.88, 0.12), (0.88, 0.88), (0.12, 0.88)], Stroke::new(thin.width, ink));
        }
        Icon::Slot => {
            // A stadium: two lines and two half circles.
            let s = Stroke::new(thin.width * 1.2, orange);
            line((0.3, 0.32), (0.7, 0.32), s);
            line((0.3, 0.68), (0.7, 0.68), s);
            let arc = |cx: f32, from: f32| {
                let pts: Vec<(f32, f32)> = (0..=8)
                    .map(|k| {
                        let a = from + std::f32::consts::PI * k as f32 / 8.0;
                        (cx + 0.18 * a.cos(), 0.5 + 0.18 * a.sin())
                    })
                    .collect();
                for w in pts.windows(2) {
                    line(w[0], w[1], s);
                }
            };
            arc(0.7, -std::f32::consts::FRAC_PI_2);
            arc(0.3, std::f32::consts::FRAC_PI_2);
            dot(0.3, 0.5, ink);
            dot(0.7, 0.5, ink);
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
        Icon::PlaneAngle => {
            // A flat square, a line along its back edge, and a second square tilted up off that line.
            let (a, b, c, d) = ((0.1, 0.85), (0.6, 0.85), (0.9, 0.6), (0.4, 0.6));
            for (p, q) in [(a, b), (b, c), (c, d), (d, a)] {
                line(p, q, thin);
            }
            let (e, f) = ((0.4, 0.15), (0.9, 0.1));
            for (p, q) in [(d, e), (e, f), (f, c)] {
                line(p, q, Stroke::new(thin.width * 1.2, orange));
            }
            line((0.3, 0.6), (1.0, 0.6), Stroke::new(thin.width * 1.4, ink));
        }
        Icon::HoleSimple | Icon::HoleCounterbore | Icon::HoleCountersink | Icon::TapSimple | Icon::TapClearance | Icon::TapTapped | Icon::TapTaper | Icon::DrillFlat | Icon::DrillAngle | Icon::ThreadFull | Icon::ThreadPartial => {
            // A hole seen cut in half: the part in grey, the hole's outline in orange.
            let top = match icon {
                Icon::HoleCounterbore => 1,
                Icon::HoleCountersink => 2,
                _ => 0,
            };
            let taper = if icon == Icon::TapTaper { 0.05 } else { 0.0 };
            let angled = icon != Icon::DrillFlat;
            let (r, y0, y1) = (0.13, 0.12, 0.72);
            // Block sides.
            line((0.08, y0), (0.08, 0.92), thin);
            line((0.92, y0), (0.92, 0.92), thin);
            line((0.08, 0.92), (0.92, 0.92), thin);
            let s = Stroke::new(thin.width * 1.2, orange);
            let mut left: Vec<(f32, f32)> = Vec::new();
            match top {
                1 => left.extend([(0.08, y0), (0.5 - 0.27, y0), (0.5 - 0.27, 0.32), (0.5 - r, 0.32)]),
                2 => left.extend([(0.08, y0), (0.5 - 0.3, y0), (0.5 - r - taper, 0.3)]),
                _ => left.extend([(0.08, y0), (0.5 - r - taper, y0)]),
            }
            left.push((0.5 - r + taper, y1));
            left.push((0.5, if angled { 0.86 } else { y1 }));
            for w in left.windows(2) {
                line(w[0], w[1], s);
                line((1.0 - w[0].0, w[0].1), (1.0 - w[1].0, w[1].1), s);
            }
            if !angled {
                line((0.5 - r, y1), (0.5 + r, y1), s);
            }
            // Thread: ticks inside the walls (on the top half only for a partial thread).
            let threaded = matches!(icon, Icon::TapTapped | Icon::TapTaper | Icon::ThreadFull | Icon::ThreadPartial);
            if threaded {
                let from = if top == 0 { y0 + 0.05 } else { 0.36 };
                let to = if icon == Icon::ThreadPartial { 0.45 } else { y1 - 0.03 };
                let mut y = from;
                while y <= to {
                    let k = (y - y0) / (y1 - y0) * 2.0 * taper;
                    line((0.5 - r - taper + k, y), (0.5 - r + 0.07 - taper + k, y + 0.03), Stroke::new(thin.width * 0.8, ink));
                    line((0.5 + r + taper - k, y), (0.5 + r - 0.07 + taper - k, y + 0.03), Stroke::new(thin.width * 0.8, ink));
                    y += 0.07;
                }
            }
            if icon == Icon::TapClearance {
                // The screw, with room round it.
                let screw = Stroke::new(thin.width * 0.8, ink);
                line((0.42, 0.04), (0.42, 0.66), screw);
                line((0.58, 0.04), (0.58, 0.66), screw);
            }
        }
        Icon::ExtentDistance => {
            line((0.2, 0.15), (0.8, 0.15), thin);
            line((0.2, 0.85), (0.8, 0.85), thin);
            let s = Stroke::new(thin.width * 1.2, orange);
            line((0.5, 0.22), (0.5, 0.78), s);
            arrow_head((0.5, 0.2), (0.5, 0.5), orange);
            arrow_head((0.5, 0.8), (0.5, 0.5), orange);
        }
        Icon::ExtentTo => {
            line((0.15, 0.8), (0.85, 0.8), Stroke::new(thin.width * 1.4, orange));
            line((0.5, 0.12), (0.5, 0.68), thin);
            arrow_head((0.5, 0.76), (0.5, 0.5), ink);
        }
        Icon::ExtentAll => {
            outline(&[(0.2, 0.35), (0.8, 0.35), (0.8, 0.65), (0.2, 0.65)], thin);
            line((0.5, 0.05), (0.5, 0.82), Stroke::new(thin.width * 1.2, orange));
            arrow_head((0.5, 0.95), (0.5, 0.6), orange);
        }
        Icon::Flip => {
            let s = Stroke::new(thin.width * 1.2, ink);
            line((0.35, 0.2), (0.35, 0.8), s);
            arrow_head((0.35, 0.12), (0.35, 0.5), ink);
            line((0.65, 0.2), (0.65, 0.8), s);
            arrow_head((0.65, 0.88), (0.65, 0.5), ink);
        }
        Icon::Shell => {
            // An open box seen from above at an angle: thin walls, hollow inside.
            let outer = [(0.12, 0.38), (0.5, 0.2), (0.88, 0.38), (0.88, 0.72), (0.5, 0.9), (0.12, 0.72)];
            outline(&outer, thin);
            let s = Stroke::new(thin.width * 1.2, orange);
            let inner = [(0.24, 0.4), (0.5, 0.29), (0.76, 0.4), (0.5, 0.52)];
            outline(&inner, s);
            line((0.5, 0.52), (0.5, 0.78), Stroke::new(thin.width * 0.8, ink));
        }
        Icon::PressPull => {
            // A block whose top face (orange) is pulled: arrows up and down off it.
            poly(&[(0.04, 0.5), (0.38, 0.34), (0.72, 0.5), (0.38, 0.66)], tint(Color32::from_rgb(235, 150, 60)));
            poly(&[(0.04, 0.5), (0.38, 0.66), (0.38, 0.96), (0.04, 0.8)], blue_dark);
            poly(&[(0.38, 0.66), (0.72, 0.5), (0.72, 0.8), (0.38, 0.96)], tint(Color32::from_rgb(60, 125, 205)));
            let s = Stroke::new(thin.width * 1.3, ink);
            line((0.87, 0.14), (0.87, 0.6), s);
            arrow_head((0.87, 0.04), (0.87, 0.28), ink);
            arrow_head((0.87, 0.7), (0.87, 0.46), ink);
        }
        Icon::Move => {
            // A block with a four-way arrow over it.
            poly(&[(0.28, 0.42), (0.72, 0.42), (0.72, 0.86), (0.28, 0.86)], tint(Color32::from_rgb(60, 125, 205)));
            let s = Stroke::new(thin.width * 1.3, ink);
            line((0.5, 0.12), (0.5, 0.88), s);
            line((0.12, 0.5), (0.88, 0.5), s);
            arrow_head((0.5, 0.04), (0.5, 0.24), ink);
            arrow_head((0.5, 0.96), (0.5, 0.76), ink);
            arrow_head((0.04, 0.5), (0.24, 0.5), ink);
            arrow_head((0.96, 0.5), (0.76, 0.5), ink);
        }
        Icon::Parameters => {
            // "fx" in italics over a table line, Fusion's Change Parameters.
            painter.text(p(0.5, 0.42), egui::Align2::CENTER_CENTER, "fx", egui::FontId::proportional(w * 0.62), orange);
            line((0.12, 0.84), (0.88, 0.84), thin);
        }
        Icon::Split => {
            // A block in two halves pulled a little apart, the cutting plane between.
            poly(&[(0.08, 0.3), (0.44, 0.3), (0.36, 0.9), (0.08, 0.9)], blue);
            poly(&[(0.58, 0.3), (0.92, 0.3), (0.92, 0.9), (0.5, 0.9)], blue_dark);
            line((0.56, 0.08), (0.4, 0.98), Stroke::new(thin.width * 1.4, orange));
        }
        Icon::Combine => {
            // Two overlapping blocks: the target (blue) and a tool (orange outline) merging into it.
            poly(&[(0.06, 0.38), (0.6, 0.38), (0.6, 0.94), (0.06, 0.94)], tint(Color32::from_rgb(60, 125, 205)));
            poly(&[(0.4, 0.06), (0.94, 0.06), (0.94, 0.6), (0.4, 0.6)], tint(Color32::from_rgba_unmultiplied(235, 150, 60, 150)));
            outline(&[(0.4, 0.06), (0.94, 0.06), (0.94, 0.6), (0.4, 0.6)], Stroke::new(thin.width * 1.2, tint(Color32::from_rgb(235, 150, 60))));
        }
        Icon::Thread => {
            // A bolt shank with slanted thread lines.
            line((0.3, 0.1), (0.3, 0.9), thin);
            line((0.7, 0.1), (0.7, 0.9), thin);
            let t = Stroke::new(thin.width * 1.2, orange);
            for k in 0..5 {
                let y = 0.2 + 0.15 * k as f32;
                line((0.3, y + 0.06), (0.7, y - 0.06), t);
            }
        }
        Icon::Hole => {
            // A block seen from the front with a drilled hole: two walls and a pointed tip.
            for (p, q) in [((0.1, 0.25), (0.9, 0.25)), ((0.1, 0.25), (0.1, 0.9)), ((0.9, 0.25), (0.9, 0.9)), ((0.1, 0.9), (0.9, 0.9))] {
                line(p, q, thin);
            }
            let wall = Stroke::new(thin.width * 1.3, orange);
            line((0.38, 0.25), (0.38, 0.62), wall);
            line((0.62, 0.25), (0.62, 0.62), wall);
            line((0.38, 0.62), (0.5, 0.74), wall);
            line((0.62, 0.62), (0.5, 0.74), wall);
            line((0.5, 0.05), (0.5, 0.2), thin);
        }
        Icon::Symmetric => {
            // A dashed mirror line between two matching slanted strokes.
            let dash = Stroke::new(thin.width, ink);
            for k in 0..4 {
                let y = 0.08 + k as f32 * 0.24;
                line((0.5, y), (0.5, y + 0.14), dash);
            }
            let s = Stroke::new(thin.width * 1.4, orange);
            line((0.12, 0.8), (0.36, 0.28), s);
            line((0.88, 0.8), (0.64, 0.28), s);
            dot(0.36, 0.28, orange);
            dot(0.64, 0.28, orange);
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
        Icon::ControlSpline => {
            // A curve pulled toward a dashed frame of points.
            let frame = [(0.08, 0.85), (0.3, 0.15), (0.7, 0.15), (0.92, 0.85)];
            for w in frame.windows(2) {
                line(w[0], w[1], Stroke::new(thin.width * 0.6, ink));
            }
            for &(x, y) in &frame {
                dot(x, y, ink);
            }
            let pts: Vec<Pos2> = (0..=24).map(|k| {
                let t = k as f32 / 24.0;
                let u = 1.0 - t;
                let x = u * u * u * 0.08 + 3.0 * u * u * t * 0.3 + 3.0 * u * t * t * 0.7 + t * t * t * 0.92;
                let y = u * u * u * 0.85 + 3.0 * u * u * t * 0.15 + 3.0 * u * t * t * 0.15 + t * t * t * 0.85;
                p(x, y)
            }).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(thin.width * 1.3, orange)));
        }
        Icon::Break => {
            // A line crossed by another, split at the crossing (a small gap, two dots).
            line((0.5, 0.08), (0.5, 0.92), thin);
            let s = Stroke::new(thin.width * 1.4, orange);
            line((0.06, 0.5), (0.34, 0.5), s);
            line((0.66, 0.5), (0.94, 0.5), s);
            dot(0.34, 0.5, orange);
            dot(0.66, 0.5, orange);
        }
        Icon::Ellipse => {
            // A wide ellipse with its centre.
            let pts: Vec<Pos2> = (0..=32).map(|k| {
                let t = std::f32::consts::TAU * k as f32 / 32.0;
                p(0.5 + 0.42 * t.cos(), 0.5 + 0.26 * t.sin())
            }).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(thin.width * 1.2, orange)));
            dot(0.5, 0.5, ink);
        }
        Icon::Midplane => {
            // Two plates with an orange plane halfway between.
            let s = Stroke::new(thin.width * 1.1, ink);
            outline(&[(0.1, 0.2), (0.6, 0.06), (0.9, 0.2), (0.4, 0.34)], s);
            outline(&[(0.1, 0.8), (0.6, 0.66), (0.9, 0.8), (0.4, 0.94)], s);
            poly(&[(0.06, 0.5), (0.6, 0.35), (0.94, 0.5), (0.4, 0.65)], orange);
        }
        Icon::Plane3Points => {
            // A plane through three dots.
            outline(&[(0.08, 0.6), (0.55, 0.3), (0.92, 0.5), (0.45, 0.8)], Stroke::new(thin.width * 1.2, orange));
            for (x, y) in [(0.2, 0.58), (0.58, 0.4), (0.62, 0.68)] {
                dot(x, y, ink);
            }
        }
        Icon::TangentPlane => {
            // A cylinder end-on with a plane touching its side.
            arc(0.38, 0.55, 0.28, 0.0, std::f32::consts::TAU, Stroke::new(thin.width * 1.2, ink));
            line((0.66, 0.1), (0.66, 0.95), Stroke::new(thin.width * 1.6, orange));
            dot(0.66, 0.55, orange);
        }
        Icon::AxisCylinder => {
            // A cylinder with an orange axis through it.
            outline(&[(0.25, 0.25), (0.75, 0.25), (0.75, 0.8), (0.25, 0.8)], Stroke::new(thin.width * 1.1, ink));
            arc(0.5, 0.25, 0.25, std::f32::consts::PI, std::f32::consts::TAU, Stroke::new(thin.width * 1.1, ink));
            line((0.5, 0.05), (0.5, 0.97), Stroke::new(thin.width * 1.5, orange));
        }
        Icon::Sweep => {
            // A profile (orange) carried along a curved path into a bent tube.
            arc(0.2, 0.85, 0.62, -std::f32::consts::FRAC_PI_2, 0.0, Stroke::new(thin.width * 3.2, blue));
            arc(0.2, 0.85, 0.62, -std::f32::consts::FRAC_PI_2, 0.0, Stroke::new(thin.width * 0.8, ink));
            poly(&[(0.12, 0.14), (0.3, 0.14), (0.3, 0.32), (0.12, 0.32)], orange);
        }
        Icon::Pipe => {
            // A bent tube with its round end showing.
            arc(0.2, 0.85, 0.6, -std::f32::consts::FRAC_PI_2, 0.0, Stroke::new(thin.width * 4.0, blue));
            arc(0.2, 0.25, 0.1, 0.0, std::f32::consts::TAU, Stroke::new(thin.width * 1.3, orange));
        }
        Icon::Interference => {
            // Two blocks overlapping, the overlap in red.
            poly(&[(0.1, 0.3), (0.6, 0.3), (0.6, 0.75), (0.1, 0.75)], blue_dark);
            poly(&[(0.4, 0.15), (0.9, 0.15), (0.9, 0.6), (0.4, 0.6)], blue);
            poly(&[(0.4, 0.3), (0.6, 0.3), (0.6, 0.6), (0.4, 0.6)], if enabled { Color32::from_rgb(230, 70, 70) } else { ink });
        }
        Icon::Component => {
            // A box drawn as an outline cube: a part of its own.
            let s = Stroke::new(thin.width * 1.2, ink);
            outline(&[(0.2, 0.35), (0.55, 0.2), (0.85, 0.35), (0.85, 0.75), (0.5, 0.9), (0.2, 0.75)], s);
            line((0.2, 0.35), (0.5, 0.5), s);
            line((0.85, 0.35), (0.5, 0.5), s);
            line((0.5, 0.5), (0.5, 0.9), s);
            poly(&[(0.2, 0.35), (0.55, 0.2), (0.85, 0.35), (0.5, 0.5)], blue);
        }
        Icon::Appearance => {
            // Three paint balls: metal, red plastic, wood, each with a highlight.
            for (x, y, c) in [(0.3, 0.32, Color32::from_rgb(175, 180, 188)), (0.7, 0.36, Color32::from_rgb(205, 45, 45)), (0.5, 0.7, Color32::from_rgb(190, 145, 95))] {
                painter.circle_filled(p(x, y), w * 0.2, tint(c));
                painter.circle_filled(p(x - 0.07, y - 0.07), w * 0.05, tint(Color32::from_gray(245)));
            }
        }
        Icon::Joint => {
            // Two blocks meeting at a pin (orange).
            poly(&[(0.1, 0.55), (0.48, 0.55), (0.48, 0.9), (0.1, 0.9)], blue_dark);
            poly(&[(0.52, 0.1), (0.9, 0.1), (0.9, 0.45), (0.52, 0.45)], blue);
            arc(0.5, 0.5, 0.12, 0.0, std::f32::consts::TAU, Stroke::new(thin.width * 1.5, orange));
            dot(0.5, 0.5, orange);
        }
        Icon::Import => {
            // A blue box with an arrow coming down into it.
            poly(&[(0.2, 0.55), (0.8, 0.55), (0.8, 0.9), (0.2, 0.9)], blue);
            let s = Stroke::new(thin.width * 1.4, orange);
            line((0.5, 0.1), (0.5, 0.48), s);
            line((0.34, 0.33), (0.5, 0.49), s);
            line((0.66, 0.33), (0.5, 0.49), s);
        }
        Icon::Coil => {
            // A spring seen from the side.
            let s = Stroke::new(thin.width * 1.4, orange);
            for k in 0..4 {
                let y = 0.15 + 0.2 * k as f32;
                line((0.2, y), (0.8, y + 0.1), s);
                if k < 3 {
                    line((0.8, y + 0.1), (0.2, y + 0.2), Stroke::new(thin.width, ink));
                }
            }
        }
        Icon::Loft => {
            // A square below blending into a circle above.
            arc(0.5, 0.2, 0.18, 0.0, std::f32::consts::TAU, Stroke::new(thin.width * 1.3, orange));
            outline(&[(0.2, 0.7), (0.8, 0.7), (0.8, 0.92), (0.2, 0.92)], Stroke::new(thin.width * 1.3, orange));
            line((0.32, 0.2), (0.2, 0.7), thin);
            line((0.68, 0.2), (0.8, 0.7), thin);
        }
        Icon::ConstructPoint => {
            // An orange point with a little cross through it.
            let s = Stroke::new(thin.width * 1.1, ink);
            line((0.2, 0.5), (0.8, 0.5), s);
            line((0.5, 0.2), (0.5, 0.8), s);
            dot(0.5, 0.5, orange);
            arc(0.5, 0.5, 0.14, 0.0, std::f32::consts::TAU, Stroke::new(thin.width * 1.4, orange));
        }
        Icon::Axis2Points => {
            // An orange line through two dots.
            line((0.08, 0.9), (0.92, 0.1), Stroke::new(thin.width * 1.5, orange));
            dot(0.3, 0.69, ink);
            dot(0.7, 0.31, ink);
        }
        Icon::Section => {
            // A block cut part way: its cut face (orange) facing out, and the cutting plane's edge.
            poly(&[(0.12, 0.42), (0.5, 0.24), (0.5, 0.62), (0.12, 0.8)], blue_dark);
            poly(&[(0.12, 0.42), (0.5, 0.24), (0.66, 0.32), (0.28, 0.5)], blue);
            poly(&[(0.28, 0.5), (0.66, 0.32), (0.66, 0.7), (0.28, 0.88)], orange);
            outline(&[(0.6, 0.06), (0.94, 0.22), (0.94, 0.86), (0.6, 0.7)], Stroke::new(thin.width, ink));
        }
        Icon::Text => {
            // An outlined capital A on a baseline, its start point marked.
            let s = Stroke::new(thin.width * 1.2, orange);
            outline(&[(0.14, 0.86), (0.42, 0.12), (0.58, 0.12), (0.86, 0.86), (0.7, 0.86), (0.62, 0.64), (0.38, 0.64), (0.3, 0.86)], s);
            outline(&[(0.44, 0.5), (0.5, 0.32), (0.56, 0.5)], s);
            dot(0.14, 0.86, ink);
        }
        Icon::ArcTangent => {
            // A line running into an arc that leaves it smoothly.
            line((0.08, 0.8), (0.45, 0.8), Stroke::new(thin.width * 1.2, ink));
            arc(0.45, 0.45, 0.35, std::f32::consts::FRAC_PI_2, 0.0, Stroke::new(thin.width * 1.2, orange));
            dot(0.45, 0.8, ink);
            dot(0.8, 0.45, ink);
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
            poly(&[(0.24, 0.5), (0.76, 0.5), (0.76, 0.88), (0.24, 0.88)], tint(Color32::from_rgb(230, 80, 80)));
        }
        Icon::Remove => {
            // A body (box) with a red cross over it.
            poly(&[(0.18, 0.4), (0.62, 0.4), (0.62, 0.84), (0.18, 0.84)], blue_dark);
            poly(&[(0.18, 0.4), (0.38, 0.22), (0.82, 0.22), (0.62, 0.4)], blue);
            poly(&[(0.62, 0.4), (0.82, 0.22), (0.82, 0.66), (0.62, 0.84)], blue_dark);
            let red = Stroke::new(thin.width * 1.6, if enabled { Color32::from_rgb(230, 80, 80) } else { ink });
            line((0.5, 0.5), (0.92, 0.92), red);
            line((0.92, 0.5), (0.5, 0.92), red);
        }
        Icon::Coincident => {
            // Two lines whose ends meet at one (orange) point.
            line((0.12, 0.82), (0.5, 0.45), Stroke::new(thin.width * 1.2, ink));
            line((0.5, 0.45), (0.9, 0.62), Stroke::new(thin.width * 1.2, ink));
            painter.circle_filled(p(0.5, 0.45), w * 0.1, orange);
        }
        Icon::Collinear => {
            // Two segments on one straight line, with a gap.
            line((0.1, 0.85), (0.42, 0.53), Stroke::new(thin.width * 1.4, ink));
            line((0.58, 0.42), (0.9, 0.12), Stroke::new(thin.width * 1.4, orange));
            dot(0.1, 0.85, ink);
            dot(0.9, 0.12, orange);
        }
        Icon::Concentric => {
            // Two circles round one centre.
            painter.circle_stroke(p(0.5, 0.5), w * 0.38, Stroke::new(thin.width * 1.2, ink));
            painter.circle_stroke(p(0.5, 0.5), w * 0.2, Stroke::new(thin.width * 1.2, orange));
            dot(0.5, 0.5, ink);
        }
        Icon::Spline => {
            // A wavy curve through three fit points.
            let pts: Vec<Pos2> = (0..=24)
                .map(|i| {
                    let t = i as f32 / 24.0;
                    p(0.1 + 0.8 * t, 0.5 - 0.28 * (t * std::f32::consts::TAU * 0.85).sin())
                })
                .collect();
            painter.add(egui::Shape::line(pts, Stroke::new(thin.width * 1.3, orange)));
            for (x, y) in [(0.1, 0.5), (0.48, 0.52), (0.9, 0.62)] {
                dot(x, y, ink);
            }
        }
        Icon::Folder => {
            poly(&[(0.08, 0.28), (0.4, 0.28), (0.48, 0.38), (0.92, 0.38), (0.92, 0.84), (0.08, 0.84)], Color32::from_rgb(150, 150, 150));
            poly(&[(0.08, 0.44), (0.92, 0.44), (0.92, 0.84), (0.08, 0.84)], Color32::from_rgb(185, 185, 185));
        }
        Icon::Gear => {
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                line((0.5 + 0.22 * a.cos(), 0.5 + 0.22 * a.sin()), (0.5 + 0.4 * a.cos(), 0.5 + 0.4 * a.sin()), Stroke::new(w * 0.12, ink));
            }
            painter.circle_filled(p(0.5, 0.5), w * 0.27, ink);
            painter.circle_filled(p(0.5, 0.5), w * 0.11, Color32::from_gray(45));
        }
        Icon::Part => {
            // A single part: one solid box.
            poly(&[(0.15, 0.38), (0.6, 0.38), (0.6, 0.86), (0.15, 0.86)], blue_dark);
            poly(&[(0.15, 0.38), (0.38, 0.16), (0.84, 0.16), (0.6, 0.38)], blue);
            poly(&[(0.6, 0.38), (0.84, 0.16), (0.84, 0.64), (0.6, 0.86)], blue_dark);
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

/// A small icon in a row of the browser (no click).
pub fn small(ui: &mut egui::Ui, icon: Icon) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
    paint(ui.painter(), rect, icon, true);
}

/// The browser's visibility eye: open when shown, shut (greyed, struck
/// through) when hidden. Click to switch.
pub fn eye(ui: &mut egui::Ui, shown: bool, tooltip: &str) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), egui::Sense::click());
    let painter = ui.painter();
    let ink = if r.hovered() { Color32::from_gray(245) } else if shown { Color32::from_gray(200) } else { Color32::from_gray(105) };
    let c = rect.center();
    let (w, h) = (rect.width() * 0.42, rect.height() * 0.26);
    // The almond outline: two arcs.
    let arc = |up: f32| -> Vec<Pos2> {
        (0..=16)
            .map(|i| {
                let t = i as f32 / 16.0 * 2.0 - 1.0;
                Pos2::new(c.x + t * w, c.y + up * h * (1.0 - t * t))
            })
            .collect()
    };
    let s = Stroke::new(1.3, ink);
    painter.add(egui::Shape::line(arc(-1.0), s));
    painter.add(egui::Shape::line(arc(1.0), s));
    if shown {
        painter.circle_filled(c, h * 0.8, ink);
    } else {
        painter.line_segment([Pos2::new(c.x - w, c.y + h * 1.3), Pos2::new(c.x + w, c.y - h * 1.3)], s);
    }
    r.on_hover_text(tooltip)
}

/// A square icon button with a tooltip. `selected` draws it pressed in.
/// A small icon choice in a dialog row (like Fusion's), named `label` for
/// screen readers and tests; `tooltip` says what it does.
pub fn option(ui: &mut egui::Ui, icon: Icon, label: &str, tooltip: &str, enabled: bool, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let label = label.to_string();
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label.clone()));
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
    paint(ui.painter(), rect.shrink(3.0), icon, enabled);
    response.on_hover_text(tooltip)
}

pub fn button(ui: &mut egui::Ui, icon: Icon, tooltip: &str, enabled: bool, selected: bool) -> egui::Response {
    button_sized(ui, icon, tooltip, enabled, selected, 34.0)
}

/// `button` at another size (the Sketch Palette's Construction is a row tall).
pub fn button_sized(ui: &mut egui::Ui, icon: Icon, tooltip: &str, enabled: bool, selected: bool, side: f32) -> egui::Response {
    let size = egui::vec2(side, side);
    // (Drag too: toolbar groups let tools be dragged into another order.)
    let sense = if enabled { egui::Sense::click_and_drag() } else { egui::Sense::hover() };
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
    paint(ui.painter(), rect.shrink(side * 5.0 / 34.0), icon, enabled);
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
    let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 36.0), egui::Sense::click_and_drag());
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

#[cfg(test)]
mod shot_tests {
    use super::*;

    /// A row of MODIFY icons, big, to look at (in the scratch dir).
    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        crate::ui_shot::shot(&format!("{dir}/icons_modify.png"), [720, 80], |ui| {
            ui.horizontal(|ui| {
                for icon in [Icon::PressPull, Icon::Fillet, Icon::Chamfer, Icon::Shell, Icon::Combine, Icon::Move, Icon::Parameters, Icon::ArcTangent, Icon::Symmetric, Icon::Ellipse, Icon::Break, Icon::ControlSpline, Icon::Text, Icon::Section, Icon::Midplane, Icon::Plane3Points, Icon::TangentPlane, Icon::AxisCylinder, Icon::Axis2Points, Icon::ConstructPoint, Icon::Sweep, Icon::Loft, Icon::Pipe, Icon::Coil, Icon::Import, Icon::Interference, Icon::Component, Icon::Joint, Icon::Appearance, Icon::Split] {
                    button(ui, icon, "", true, false);
                }
            });
        });
    }
}
