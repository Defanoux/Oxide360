//! The Insert SVG / DXF dialog (sketch mode): the file's curves, placed in
//! the open sketch — moved, turned and scaled — before OK.

use crate::copy_dialog::Action;
use crate::sketch_import::{Curve, Drawing, Placement, bounds};
use crate::units::Unit;
use eframe::egui;
use glam::DVec2;

#[derive(Clone, Debug, PartialEq)]
pub struct InsertDrawingDialog {
    /// The file's name, for the panel.
    pub name: String,
    pub drawing: Drawing,
    pub place: Placement,
}

impl InsertDrawingDialog {
    pub fn new(name: String, drawing: Drawing) -> Self {
        InsertDrawingDialog { name, drawing, place: Placement::default() }
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
