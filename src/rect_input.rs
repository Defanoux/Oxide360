//! Typed sizes while drawing a 2-point or centre rectangle (as in Fusion):
//! after the first click, typing fills the width box, Tab moves to the
//! height box. A typed size holds the rectangle at that size while the cursor
//! only picks which way it goes; the click places it, and each typed size
//! becomes a dimension. Nothing typed: the click sets the size, no dimensions.

use crate::sketch::Constraint;
use crate::units::{Unit, parse_length};
use glam::DVec2;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RectInput {
    /// Width, height as typed.
    pub text: [String; 2],
    /// The box being typed into (0 width, 1 height).
    pub field: usize,
    /// Focus `field` next frame.
    pub focus: bool,
}

impl RectInput {
    /// The first characters typed (in the view): into the current box.
    pub fn type_text(&mut self, typed: String) {
        self.text[self.field] = typed;
        self.focus = true;
    }

    /// Tab: the other box.
    pub fn tab(&mut self) {
        self.field = 1 - self.field;
        self.focus = true;
    }

    /// The sizes typed so far (a box that's empty or not a positive length: None).
    pub fn sizes(&self, unit: Unit) -> [Option<f64>; 2] {
        self.text.clone().map(|t| parse_length(&t, unit).filter(|v| *v > 0.0))
    }
}

/// Where the rectangle's far corner goes: each typed size fixed, on the side
/// of `start` the cursor is on. `centered`: `start` is the middle, so the
/// corner is half the size out.
pub fn sized_corner(start: DVec2, cursor: DVec2, centered: bool, sizes: [Option<f64>; 2]) -> DVec2 {
    let axis = |s: f64, c: f64, size: Option<f64>| match size {
        Some(v) => s + if c < s { -1.0 } else { 1.0 } * if centered { v / 2.0 } else { v },
        None => c,
    };
    DVec2::new(axis(start.x, cursor.x, sizes[0]), axis(start.y, cursor.y, sizes[1]))
}

/// The dimensions a just-made rectangle keeps: its first line is horizontal
/// (the width), the next vertical (the height).
pub fn size_dimensions(first_line: usize, sizes: [Option<f64>; 2]) -> Vec<Constraint> {
    let mut out = Vec::new();
    if let Some(w) = sizes[0] {
        out.push(Constraint::Length(first_line, w));
    }
    if let Some(h) = sizes[1] {
        out.push(Constraint::Length(first_line + 1, h));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, Entity, OriginPlane, Sketch, Tool};

    #[test]
    fn typed_sizes_hold_and_the_cursor_picks_the_side() {
        let s = DVec2::new(10.0, 10.0);
        // Both typed: 30 × 20, down and to the left of the start.
        assert_eq!(sized_corner(s, DVec2::new(2.0, 3.0), false, [Some(30.0), Some(20.0)]), DVec2::new(-20.0, -10.0));
        // Only the width: the height still follows the cursor.
        assert_eq!(sized_corner(s, DVec2::new(50.0, 14.0), false, [Some(30.0), None]), DVec2::new(40.0, 14.0));
        // Centre rectangle: half each way.
        assert_eq!(sized_corner(s, DVec2::new(11.0, 0.0), true, [Some(30.0), Some(20.0)]), DVec2::new(25.0, 0.0));
        // Nothing typed: just the cursor.
        assert_eq!(sized_corner(s, DVec2::new(1.0, 2.0), false, [None, None]), DVec2::new(1.0, 2.0));
    }

    #[test]
    fn tab_switches_boxes_and_units_parse() {
        let mut r = RectInput::default();
        r.type_text("25".into());
        assert_eq!(r.text, ["25".to_string(), String::new()]);
        assert_eq!((r.field, r.focus), (0, true));
        r.focus = false;
        r.tab();
        assert_eq!((r.field, r.focus), (1, true));
        r.text[1] = "1 in".into();
        assert_eq!(r.sizes(Unit::Mm), [Some(25.0), Some(25.4)]);
        r.text[0] = "-3".into();
        assert_eq!(r.sizes(Unit::Mm)[0], None, "a size must be positive");
        r.tab();
        assert_eq!(r.field, 0);
    }

    #[test]
    fn a_rectangle_placed_with_typed_sizes_keeps_them_as_dimensions() {
        for (tool, centered) in [(Tool::Rectangle, false), (Tool::RectangleCenter, true)] {
            let mut sk = Sketch::new("S".into(), OriginPlane::XY);
            let mut ed = Editor::default();
            ed.set_tool(tool);
            ed.click(&mut sk, Anchor::Free(DVec2::new(5.0, 5.0)));
            let sizes = [Some(40.0), Some(15.0)];
            let at = sized_corner(DVec2::new(5.0, 5.0), DVec2::new(-3.0, 9.0), centered, sizes);
            let first = sk.entities.len();
            let cs = ed.click(&mut sk, Anchor::Free(at));
            crate::solver::add_constraints(&mut sk, &cs).unwrap();
            let first = (first..sk.entities.len()).find(|&e| sk.is_line(e)).unwrap();
            crate::solver::add_constraints(&mut sk, &size_dimensions(first, sizes)).unwrap();
            let len = |e: usize| match sk.entities[e] {
                Entity::Line { a, b } => sk.points[a].distance(sk.points[b]),
                _ => 0.0,
            };
            assert!((len(first) - 40.0).abs() < 1e-9 && (len(first + 1) - 15.0).abs() < 1e-9, "{tool:?}");
            assert_eq!(sk.constraints.iter().filter(|c| matches!(c, Constraint::Length(..))).count(), 2);
        }
    }
}
