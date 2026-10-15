//! The Offset Face dialog (Fusion's; also what Press Pull opens on a flat
//! face): the faces to move and how far, out along their normals.

use crate::copy_dialog::Action;
use crate::model::{OffsetFaceFeature, PickedFace};
use eframe::egui;

#[derive(Clone, Debug, PartialEq)]
pub struct OffsetDialog {
    pub editing: Option<usize>,
    pub offset: OffsetFaceFeature,
    /// The Faces field takes clicks in the view.
    pub picking: bool,
    /// Opened as Press Pull (Q): a click on a sketch profile starts an
    /// Extrude, on a fillet edits that fillet.
    pub press_pull: bool,
}

impl OffsetDialog {
    pub fn new(name: String, press_pull: bool) -> Self {
        OffsetDialog { editing: None, offset: OffsetFaceFeature { name, ..Default::default() }, picking: true, press_pull }
    }

    pub fn from_feature(k: usize, f: &OffsetFaceFeature) -> Self {
        OffsetDialog { editing: Some(k), offset: f.clone(), picking: false, press_pull: false }
    }

    /// The picked face in the same plane as `p` (picked anywhere on it).
    pub fn picked_on(&self, p: &PickedFace) -> Option<usize> {
        self.offset.faces.iter().position(|q| q.face.normal().dot(p.face.normal()) > 0.9999 && (q.face.origin - p.face.origin).length() < 1e-6)
    }

    /// Click on a face: added, or dropped if it's picked already.
    pub fn toggle_face(&mut self, p: PickedFace) {
        match self.picked_on(&p) {
            Some(i) => {
                self.offset.faces.remove(i);
            }
            None => self.offset.faces.push(p),
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>) -> Action {
        let title = match (self.editing.is_some(), self.press_pull) {
            (true, _) => "Edit Offset Face",
            (false, true) => "Press Pull",
            (false, false) => "Offset Face",
        };
        ui.heading(title);
        let o = &mut self.offset;
        egui::Grid::new("offset_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Faces");
            ui.horizontal(|ui| {
                let n = o.faces.len();
                let text = match (self.picking, n) {
                    (true, _) => "Click faces…".to_string(),
                    (false, 0) => "Click to select".to_string(),
                    (false, n) => format!("{n} selected"),
                };
                let rich = if self.picking { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                if ui.add(egui::Button::selectable(self.picking, rich).min_size(egui::vec2(170.0, 0.0))).on_hover_text("Click flat faces to move; click one again to drop it").clicked() {
                    self.picking = !self.picking;
                }
                if n > 0 && ui.small_button("×").on_hover_text("Clear the faces").clicked() {
                    o.faces.clear();
                    self.picking = true;
                }
            });
            ui.end_row();
            ui.label("Distance");
            ui.add(crate::units::length_drag(&mut o.distance, unit, 0.1));
            ui.end_row();
        });
        ui.weak(if self.press_pull && self.editing.is_none() {
            "Drag the arrow or type a distance (out is +). A fillet opens its edit; a sketch profile, Extrude."
        } else {
            "Drag the arrow or type a distance (out is +)."
        });
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        let mut action = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(error.is_none() && !self.offset.faces.is_empty(), egui::Button::new("OK")).clicked() {
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
    use crate::sketch::FacePlane;
    use glam::DVec3;

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = OffsetDialog::new("Offset Face1".into(), true);
        d.offset.faces.push(PickedFace { face: FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0)), anchor: [0.0; 2] });
        d.offset.distance = 5.0;
        d.picking = false;
        crate::ui_shot::shot(&format!("{dir}/offset.png"), [420, 220], |ui| {
            d.panel(ui, crate::units::Unit::Mm, None);
        });
    }

    #[test]
    fn faces_toggle_by_plane() {
        let mut d = OffsetDialog::new("Offset Face1".into(), false);
        assert!(d.picking, "a new one waits for faces");
        let top = PickedFace { face: FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0)), anchor: [1.0, 2.0] };
        d.toggle_face(top);
        d.toggle_face(PickedFace { anchor: [9.0, 9.0], ..top });
        assert!(d.offset.faces.is_empty(), "the same face, clicked elsewhere, drops it");
        d.toggle_face(top);
        let bottom = PickedFace { face: FacePlane::from_face(DVec3::NEG_Z, DVec3::ZERO), anchor: [1.0, 2.0] };
        d.toggle_face(bottom);
        assert_eq!(d.offset.faces.len(), 2);
    }
}
