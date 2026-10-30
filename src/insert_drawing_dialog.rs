//! The Insert SVG / DXF dialog (sketch mode): the file's curves, placed in
//! the open sketch — moved, turned and scaled — before OK.

use crate::copy_dialog::Action;
use crate::sketch_import::{Curve, Drawing, Placement, bounds};
use crate::units::Unit;
use eframe::egui;
use glam::{DVec2, Vec2};

/// A move handle in the view: an arrow along the sketch's X or Y, or the
/// box in the middle (both at once).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grab {
    X,
    Y,
    Both,
}

/// Half the middle box's size (screen px).
pub const BOX_PX: f32 = 8.0;

#[derive(Clone, Debug, PartialEq)]
pub struct InsertDrawingDialog {
    /// The file's name, for the panel.
    pub name: String,
    pub drawing: Drawing,
    pub place: Placement,
    /// The handles as last drawn (screen): the middle, the X and Y arrow tips.
    pub screen: Option<[Vec2; 3]>,
    pub hover: Option<Grab>,
    /// A drag under way: the handle, the offset and the mouse (sketch mm) when it started.
    pub drag: Option<(Grab, DVec2, DVec2)>,
}

impl InsertDrawingDialog {
    pub fn new(name: String, drawing: Drawing) -> Self {
        InsertDrawingDialog { name, drawing, place: Placement::default(), screen: None, hover: None, drag: None }
    }

    /// Where the handles sit: the middle of the curves as placed (sketch mm).
    pub fn middle(&self) -> DVec2 {
        bounds(&self.placed()).map_or(self.place.offset, |(lo, hi)| (lo + hi) * 0.5)
    }

    /// The handle under `p` (screen), from where they were drawn. The box
    /// wins over the arrows' starts; arrows are picked along their length.
    pub fn pick(&self, p: Vec2) -> Option<Grab> {
        let [m, x, y] = self.screen?;
        if (p - m).abs().max_element() <= BOX_PX {
            return Some(Grab::Both);
        }
        let near = |a: Vec2, b: Vec2| {
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
            (a + ab * t).distance(p)
        };
        [(Grab::X, near(m, x)), (Grab::Y, near(m, y))].into_iter().filter(|(_, d)| *d < 9.0).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(g, _)| g)
    }

    /// Start dragging what's under `p` (screen) with the mouse at `at` (sketch mm).
    pub fn start_drag(&mut self, p: Vec2, at: DVec2) -> bool {
        self.drag = self.pick(p).map(|g| (g, self.place.offset, at));
        self.drag.is_some()
    }

    /// The mouse now at `at` (sketch mm): the drawing follows, along the arrow's axis only.
    pub fn drag_to(&mut self, at: DVec2) {
        if let Some((g, offset, from)) = self.drag {
            let d = at - from;
            self.place.offset = offset
                + match g {
                    Grab::X => DVec2::new(d.x, 0.0),
                    Grab::Y => DVec2::new(0.0, d.y),
                    Grab::Both => d,
                };
        }
    }

    /// The curves where they'd go.
    pub fn placed(&self) -> Vec<Curve> {
        self.drawing.curves.iter().map(|c| self.place.curve(c)).collect()
    }

    /// Move it so its middle is on the sketch's origin.
    pub fn centre(&mut self) {
        if let Some((lo, hi)) = bounds(&self.placed()) {
            self.place.offset -= (lo + hi) * 0.5;
        }
    }

    /// Move it so its bottom-left corner is on the sketch's origin.
    pub fn corner(&mut self) {
        if let Some((lo, _)) = bounds(&self.placed()) {
            self.place.offset -= lo;
        }
    }

    pub fn ready(&self) -> bool {
        !self.drawing.curves.is_empty() && self.place.scale.is_finite() && self.place.scale > 0.0
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: Unit) -> Action {
        ui.heading("Insert SVG / DXF");
        ui.label(&self.name);
        let size = bounds(&self.placed()).map(|(lo, hi)| hi - lo).unwrap_or(DVec2::ZERO);
        ui.weak(format!("{} curves · {} × {}", self.drawing.curves.len(), unit.show(size.x), unit.show(size.y)));
        for s in &self.drawing.skipped {
            ui.colored_label(egui::Color32::from_rgb(230, 170, 90), format!("Left out: {s}"));
        }
        egui::Grid::new("insert_drawing_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("X");
            ui.add(crate::units::length_drag(&mut self.place.offset.x, unit, 0.1).update_while_editing(false));
            ui.end_row();
            ui.label("Y");
            ui.add(crate::units::length_drag(&mut self.place.offset.y, unit, 0.1).update_while_editing(false));
            ui.end_row();
            ui.label("Angle");
            ui.add(egui::DragValue::new(&mut self.place.angle).speed(0.5).suffix("°").update_while_editing(false));
            ui.end_row();
            ui.label("Scale");
            ui.add(egui::DragValue::new(&mut self.place.scale).speed(0.01).range(0.0001..=10000.0).max_decimals(4).update_while_editing(false));
            ui.end_row();
        });
        ui.horizontal(|ui| {
            if ui.button("Centre on Origin").clicked() {
                self.centre();
            }
            if ui.button("Corner on Origin").on_hover_text("Its bottom-left corner on the sketch's origin").clicked() {
                self.corner();
            }
        });
        ui.separator();
        let mut action = Action::None;
        let ready = self.ready();
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, egui::Button::new("OK")).clicked() {
                action = Action::Ok;
            }
            if ui.button("Cancel").clicked() {
                action = Action::Cancel;
            }
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centre_and_corner_on_the_origin() {
        let drawing = Drawing { curves: vec![Curve::Line(DVec2::new(10.0, 10.0), DVec2::new(30.0, 20.0))], skipped: Vec::new() };
        let mut d = InsertDrawingDialog::new("a.svg".into(), drawing);
        d.place.scale = 2.0;
        d.centre();
        let (lo, hi) = bounds(&d.placed()).unwrap();
        assert!((lo + hi).length() < 1e-9, "{lo} {hi}");
        d.corner();
        let (lo, _) = bounds(&d.placed()).unwrap();
        assert!(lo.length() < 1e-9, "{lo}");
        assert!(d.ready());
        d.place.scale = 0.0;
        assert!(!d.ready());
    }

    #[test]
    fn arrows_move_one_way_and_the_box_both() {
        let drawing = Drawing { curves: vec![Curve::Line(DVec2::new(0.0, 0.0), DVec2::new(20.0, 10.0))], skipped: Vec::new() };
        let mut d = InsertDrawingDialog::new("a.dxf".into(), drawing);
        assert_eq!(d.middle(), DVec2::new(10.0, 5.0));
        // Drawn: middle at (100, 100) px, X tip to the right, Y tip up.
        d.screen = Some([Vec2::new(100.0, 100.0), Vec2::new(160.0, 100.0), Vec2::new(100.0, 40.0)]);
        assert_eq!(d.pick(Vec2::new(103.0, 98.0)), Some(Grab::Both));
        assert_eq!(d.pick(Vec2::new(140.0, 104.0)), Some(Grab::X));
        assert_eq!(d.pick(Vec2::new(97.0, 60.0)), Some(Grab::Y));
        assert_eq!(d.pick(Vec2::new(150.0, 50.0)), None);
        // The X arrow: only X follows.
        assert!(d.start_drag(Vec2::new(140.0, 100.0), DVec2::new(15.0, 5.0)));
        d.drag_to(DVec2::new(22.0, 9.0));
        assert_eq!(d.place.offset, DVec2::new(7.0, 0.0));
        // The box: both.
        assert!(d.start_drag(Vec2::new(100.0, 100.0), DVec2::new(0.0, 0.0)));
        d.drag_to(DVec2::new(-1.0, 3.0));
        assert_eq!(d.place.offset, DVec2::new(6.0, 3.0));
        assert!(!d.start_drag(Vec2::new(300.0, 300.0), DVec2::ZERO));
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let drawing = Drawing { curves: vec![Curve::Line(DVec2::ZERO, DVec2::new(30.0, 20.0))], skipped: vec!["2 text elements (turn text into paths first)".into()] };
        let mut d = InsertDrawingDialog::new("bracket.svg".into(), drawing);
        crate::ui_shot::shot(&format!("{dir}/insert_drawing.png"), [380, 300], |ui| {
            d.panel(ui, Unit::Mm);
        });
    }
}
