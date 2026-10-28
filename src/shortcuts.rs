//! The tools by name, for the S key's Design Shortcuts box and the
//! right-click marking menu (as Fusion's): every tool a mode has, and
//! finding them by what's typed.

/// Solid-mode tools (the names `run_solid_tool` takes).
pub const SOLID: &[&str] = &[
    "Create Sketch", "Extrude", "Revolve", "Sweep", "Loft", "Pipe", "Coil", "Hole", "Thread",
    "Offset Plane", "Plane at Angle", "Midplane", "Plane Through Three Points", "Tangent Plane", "Axis Along Edge", "Axis Through Cylinder", "Axis Through Two Points", "Point",
    "Mirror", "Rectangular Pattern", "Circular Pattern",
    "Press Pull", "Fillet", "Chamfer", "Shell", "Move/Copy", "Combine", "Appearance", "Change Parameters",
];

/// Sketch-mode tools (the names `run_sketch_tool` takes).
pub const SKETCH: &[&str] = &[
    "Line", "Rectangle", "Circle", "Arc", "Polygon", "Point", "Slot", "Spline", "Ellipse", "Text",
    "Fillet", "Chamfer", "Trim", "Break", "Extend", "Offset", "Rectangular Pattern", "Circular Pattern", "Mirror", "Project", "Construction", "Change Parameters",
    "Dimension", "Coincident", "Collinear", "Concentric", "Horizontal", "Vertical", "Parallel", "Perpendicular", "Equal", "Tangent", "Midpoint", "Symmetric", "Fix",
];

/// The tools in `tools` matching `typed` (ignoring case): names starting
/// with it first, then those with a word starting with it, then those
/// containing it anywhere; each group in list order. Nothing typed: all.
pub fn search(tools: &[&'static str], typed: &str) -> Vec<&'static str> {
    let t = typed.trim().to_lowercase();
    if t.is_empty() {
        return tools.to_vec();
    }
    let rank = |name: &str| {
        let n = name.to_lowercase();
        if n.starts_with(&t) {
            Some(0)
        } else if n.split([' ', '/']).any(|w| w.starts_with(&t)) {
            Some(1)
        } else if n.contains(&t) {
            Some(2)
        } else {
            None
        }
    };
    let mut hits: Vec<(usize, usize, &'static str)> = tools.iter().enumerate().filter_map(|(k, n)| rank(n).map(|r| (r, k, *n))).collect();
    hits.sort();
    hits.into_iter().map(|(_, _, n)| n).collect()
}

/// `name` was just used: first in the box's recent row, once, six at most.
pub fn used(recent: &mut Vec<&'static str>, name: &'static str) {
    recent.retain(|n| *n != name);
    recent.insert(0, name);
    recent.truncate(6);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_finds_tools_by_start_then_word_then_anywhere() {
        assert_eq!(search(SOLID, "")[..3], ["Create Sketch", "Extrude", "Revolve"]);
        assert_eq!(search(SOLID, "fil"), ["Fillet"]);
        // "pl": names starting with it, then a word starting with it.
        let pl = search(SOLID, "pl");
        assert_eq!(pl[0], "Plane at Angle");
        assert!(pl.contains(&"Offset Plane") && pl.contains(&"Midplane"));
        assert!(pl.iter().position(|n| *n == "Offset Plane") < pl.iter().position(|n| *n == "Midplane"), "a word's start before the middle of one: {pl:?}");
        assert_eq!(search(SOLID, "COPY"), ["Move/Copy"]);
        assert!(search(SKETCH, "xyz").is_empty());
        // Every sketch constraint can be found.
        for c in crate::sketch::CONSTRAINT_ACTIONS {
            assert!(SKETCH.contains(&c), "{c}");
        }
    }

    #[test]
    fn the_recent_row_keeps_the_latest_six() {
        let mut r = Vec::new();
        for n in ["Line", "Circle", "Trim", "Offset", "Fillet", "Project", "Text"] {
            used(&mut r, n);
        }
        used(&mut r, "Trim");
        assert_eq!(r, ["Trim", "Text", "Project", "Fillet", "Offset", "Circle"]);
    }
}
