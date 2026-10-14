//! The Shell dialog (Fusion's): faces to open and/or bodies to hollow closed,
//! the direction (inside, outside, both), the wall thicknesses and the type
//! (sharp or rounded offset corners).

use crate::copy_dialog::Action;
use crate::doc::BodyId;
use crate::model::{PickedFace, ShellDirection, ShellFeature};
use eframe::egui;

/// Which selection field takes clicks in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Faces,
    Bodies,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShellDialog {
    pub editing: Option<usize>,
    pub shell: ShellFeature,
    pub picking: Option<Field>,
}

impl ShellDialog {
    pub fn new(name: String) -> Self {
        ShellDialog { editing: None, shell: ShellFeature { name, ..Default::default() }, picking: Some(Field::Faces) }
    }

    pub fn from_feature(k: usize, s: &ShellFeature) -> Self {
        ShellDialog { editing: Some(k), shell: s.clone(), picking: None }
    }

    /// Click on a face: opened, or closed again if it was picked.
    pub fn toggle_face(&mut self, p: PickedFace) {
        let same = |q: &PickedFace| {
            q.face.normal().dot(p.face.normal()) > 0.9999
                && (q.face.origin - p.face.origin).length() < 1e-6
                && (q.anchor[0] - p.anchor[0]).hypot(q.anchor[1] - p.anchor[1]) < 1e-6
        };
        match self.shell.faces.iter().position(same) {
            Some(i) => {
                self.shell.faces.remove(i);
            }
            None => self.shell.faces.push(p),
        }
    }

    /// The same face again (by plane), for "click it again to drop it" on a
    /// face picked at another spot.
    pub fn picked_on(&self, p: &PickedFace) -> Option<usize> {
        self.shell.faces.iter().position(|q| q.face.normal().dot(p.face.normal()) > 0.9999 && (q.face.origin - p.face.origin).length() < 1e-6)
    }

    pub fn toggle_body(&mut self, id: BodyId) {
        match self.shell.bodies.iter().position(|b| *b == id) {
            Some(i) => {
                self.shell.bodies.remove(i);
            }
            None => self.shell.bodies.push(id),
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Shell" } else { "Shell" });
        let s = &mut self.shell;
        egui::Grid::new("shell_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for (field, label, what, tip) in [
                (Field::Faces, "Faces", s.faces.len(), "Click flat faces to take away (the opening); click one again to keep it"),
                (Field::Bodies, "Bodies", s.bodies.len(), "Click bodies to hollow with no opening"),
            ] {
                ui.label(label);
                let on = self.picking == Some(field);
                let text = match (on, what) {
                    (true, _) => format!("Click {}…", label.to_lowercase()),
                    (false, 0) => "Click to select".to_string(),
                    (false, n) => format!("{n} selected"),
                };
                let rich = if on { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                if ui.add(egui::Button::selectable(on, rich).min_size(egui::vec2(170.0, 0.0))).on_hover_text(tip).clicked() {
                    self.picking = if on { None } else { Some(field) };
                }
                ui.end_row();
            }
            ui.label("Direction");
            ui.horizontal(|ui| {
                for (d, name) in [(ShellDirection::Inside, "Inside"), (ShellDirection::Outside, "Outside"), (ShellDirection::Both, "Both")] {
                    ui.selectable_value(&mut s.direction, d, name);
                }
            });
            ui.end_row();
            if s.direction != ShellDirection::Outside {
                ui.label("Inside Thickness");
                ui.add(crate::units::length_drag(&mut s.inside, unit, 0.05).range(0.0..=f64::MAX));
                ui.end_row();
            }
            if s.direction != ShellDirection::Inside {
                ui.label("Outside Thickness");
                ui.add(crate::units::length_drag(&mut s.outside, unit, 0.05).range(0.0..=f64::MAX));
                ui.end_row();
            }
            ui.label("Type");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut s.rounded, false, "Sharp Offset").on_hover_text("Sharp offset corners");
                ui.selectable_value(&mut s.rounded, true, "Rounded Offset").on_hover_text("Rounded offset corners");
            });
            ui.end_row();
        });
        ui.weak("Drag the blue arrow to set the wall.");
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        let mut action = Action::None;
        let ready = !self.shell.faces.is_empty() || !self.shell.bodies.is_empty();
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
    use crate::sketch::FacePlane;
    use glam::DVec3;

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = ShellDialog::new("Shell1".into());
        d.shell.direction = ShellDirection::Both;
        crate::ui_shot::shot(&format!("{dir}/shell.png"), [400, 330], |ui| {
            d.panel(ui, crate::units::Unit::Mm, None);
        });
    }

    #[test]
    fn faces_and_bodies_toggle() {
        let mut d = ShellDialog::new("Shell1".into());
        assert_eq!(d.picking, Some(Field::Faces), "a new shell waits for faces");
        let top = PickedFace { face: FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0)), anchor: [1.0, 2.0] };
        d.toggle_face(top);
        assert_eq!(d.picked_on(&PickedFace { anchor: [9.0, 9.0], ..top }), Some(0), "the same face picked elsewhere");
        d.toggle_face(top);
        assert!(d.shell.faces.is_empty());
        let body = BodyId { feature: Feature::Extrude(0), piece: 0 };
        d.toggle_body(body);
        d.toggle_body(body);
        assert!(d.shell.bodies.is_empty());
        assert_eq!(d.shell.walls().unwrap(), (2.0, 0.0));
        d.shell.direction = ShellDirection::Both;
        assert_eq!(d.shell.walls().unwrap(), (2.0, 2.0));
    }
}
