//! The Move/Copy dialog (Fusion's, M): the bodies, the move type (Free Move:
//! X/Y/Z distances and angles; Rotate: about an axis), and Create Copy.

use crate::copy_dialog::Action;
use crate::doc::BodyId;
use crate::model::{MoveFeature, MoveKind, WorldAxis};
use eframe::egui;

#[derive(Clone, Debug, PartialEq)]
pub struct MoveDialog {
    pub editing: Option<usize>,
    pub feature: MoveFeature,
    /// The Bodies field takes clicks in the view.
    pub picking: bool,
}

impl MoveDialog {
    /// A new Move; `selected`: bodies picked in the browser beforehand.
    pub fn new(name: String, selected: &[BodyId]) -> Self {
        let feature = MoveFeature { name, bodies: selected.to_vec(), ..Default::default() };
        MoveDialog { editing: None, picking: selected.is_empty(), feature }
    }

    pub fn from_feature(k: usize, f: &MoveFeature) -> Self {
        MoveDialog { editing: Some(k), feature: f.clone(), picking: false }
    }

    pub fn toggle_body(&mut self, id: BodyId) {
        match self.feature.bodies.iter().position(|b| *b == id) {
            Some(i) => {
                self.feature.bodies.remove(i);
            }
            None => self.feature.bodies.push(id),
        }
    }

    /// Switch the move type, starting the new one at nothing.
    pub fn set_rotate(&mut self, rotate: bool) {
        let now = matches!(self.feature.kind, MoveKind::Rotate { .. });
        if now != rotate {
            self.feature.kind = if rotate { MoveKind::Rotate { axis: WorldAxis::Z, custom: None, angle: 0.0 } } else { MoveKind::Free { offset: [0.0; 3], angles: [0.0; 3] } };
        }
    }

    /// The panel; `names`: every body's name, `axes`: the construction axes' names.
    pub fn panel(&mut self, ui: &mut egui::Ui, names: &[(BodyId, String)], axes: &[String], unit: crate::units::Unit, error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Move/Copy" } else { "Move/Copy" });
        let name_of = |id: &BodyId| names.iter().find(|(b, _)| b == id).map_or("(gone)".to_string(), |(_, n)| n.clone());
        let mut rotate = matches!(self.feature.kind, MoveKind::Rotate { .. });
        egui::Grid::new("move_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Bodies");
            ui.horizontal(|ui| {
                let f = &mut self.feature;
                let text = match (self.picking, f.bodies.len()) {
                    (_, 1 | 2) => f.bodies.iter().map(name_of).collect::<Vec<_>>().join(", "),
                    (true, 0) => "Click bodies in the view…".to_string(),
                    (false, 0) => "Click to select".to_string(),
                    (_, n) => format!("{n} bodies"),
                };
                let rich = if self.picking { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                if ui.add(egui::Button::selectable(self.picking, rich).min_size(egui::vec2(170.0, 0.0))).on_hover_text("Click bodies in the view or the browser; click one again to drop it").clicked() {
                    self.picking = !self.picking;
                }
                if !f.bodies.is_empty() && ui.small_button("×").on_hover_text("Clear").clicked() {
                    f.bodies.clear();
                    self.picking = true;
                }
            });
            ui.end_row();
            ui.label("Move Type");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut rotate, false, "Free Move").on_hover_text("Drag the arrows and rings, or type distances and angles");
                ui.selectable_value(&mut rotate, true, "Rotate").on_hover_text("Turn about an axis");
            });
            ui.end_row();
        });
        self.set_rotate(rotate);
        let f = &mut self.feature;
        egui::Grid::new("move_values").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            let angle = |ui: &mut egui::Ui, v: &mut f64| ui.add(egui::DragValue::new(v).speed(0.5).range(-360.0..=360.0).suffix("°"));
            match &mut f.kind {
                MoveKind::Free { offset, angles } => {
                    for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                        ui.label(format!("{axis} Distance"));
                        ui.add(crate::units::length_drag(&mut offset[i], unit, 0.1));
                        ui.end_row();
                    }
                    for (i, axis) in ["X", "Y", "Z"].iter().enumerate() {
                        ui.label(format!("{axis} Angle"));
                        angle(ui, &mut angles[i]);
                        ui.end_row();
                    }
                }
                MoveKind::Rotate { axis, custom, angle: a } => {
                    ui.label("Axis");
                    ui.horizontal(|ui| {
                        for w in WorldAxis::ALL {
                            if ui.selectable_label(custom.is_none() && *axis == w, w.name()).clicked() {
                                (*axis, *custom) = (w, None);
                            }
                        }
                        if !axes.is_empty() {
                            let shown = custom.and_then(|k| axes.get(k)).map_or("Axis…", |s| s.as_str());
                            egui::ComboBox::from_id_salt("move_axis").selected_text(shown).show_ui(ui, |ui| {
                                for (k, name) in axes.iter().enumerate() {
                                    ui.selectable_value(custom, Some(k), name);
                                }
                            });
                        }
                    });
                    ui.end_row();
                    ui.label("Angle");
                    angle(ui, a);
                    ui.end_row();
                }
            }
            ui.label("Create Copy");
            ui.checkbox(&mut f.copy, "").on_hover_text("Leave the bodies where they are and move copies of them");
            ui.end_row();
        });
        if matches!(f.kind, MoveKind::Free { .. }) {
            ui.weak("Drag the arrows to move, the rings to turn (about the bodies' middle).");
        }
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        let mut action = Action::None;
        let ready = !self.feature.bodies.is_empty();
        ui.horizontal(|ui| {
            if ui.add_enabled(error.is_none() && ready, egui::Button::new("OK")).clicked() {
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
    use crate::doc::Feature;

    fn body(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let names = vec![(body(0), "Body1".to_string())];
        let mut d = MoveDialog::new("Move1".into(), &[body(0)]);
        crate::ui_shot::shot(&format!("{dir}/move_free.png"), [420, 330], |ui| {
            d.panel(ui, &names, &[], crate::units::Unit::Mm, None);
        });
        let mut d = MoveDialog::new("Move1".into(), &[body(0)]);
        d.set_rotate(true);
        crate::ui_shot::shot(&format!("{dir}/move_rotate.png"), [420, 260], |ui| {
            d.panel(ui, &names, &["Axis1".to_string()], crate::units::Unit::Mm, None);
        });
    }

    #[test]
    fn bodies_toggle_and_the_type_switches() {
        let mut d = MoveDialog::new("Move1".into(), &[]);
        assert!(d.picking, "nothing selected first: waits for bodies");
        d.toggle_body(body(0));
        d.toggle_body(body(1));
        d.toggle_body(body(0));
        assert_eq!(d.feature.bodies, vec![body(1)]);
        assert!(!MoveDialog::new("Move1".into(), &[body(2)]).picking, "bodies selected beforehand");
        d.feature.kind = MoveKind::Free { offset: [1.0, 2.0, 3.0], angles: [0.0; 3] };
        d.set_rotate(false);
        assert_eq!(d.feature.kind, MoveKind::Free { offset: [1.0, 2.0, 3.0], angles: [0.0; 3] }, "the same type keeps its values");
        d.set_rotate(true);
        assert!(matches!(d.feature.kind, MoveKind::Rotate { angle, .. } if angle == 0.0));
    }
}
