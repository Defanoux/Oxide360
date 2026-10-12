//! The Offset Plane dialog: a plane a set distance from an origin plane or a
//! body face.

use crate::copy_dialog::Action;
use crate::model::PlaneFeature;
use crate::sketch::FacePlane;
use eframe::egui;

#[derive(Clone, Debug, PartialEq)]
pub struct PlaneDialog {
    pub editing: Option<usize>,
    pub base: FacePlane,
    pub on_origin: bool,
    pub distance: f64,
    pub anchor: [f64; 2],
    pub extent: [f64; 4],
}

impl PlaneDialog {
    /// Starts 0 mm from what was picked.
    pub fn new(base: FacePlane, on_origin: bool, anchor: [f64; 2], extent: [f64; 4]) -> Self {
        PlaneDialog { editing: None, base, on_origin, distance: 0.0, anchor, extent }
    }

    pub fn from_feature(k: usize, f: &PlaneFeature) -> Self {
        PlaneDialog { editing: Some(k), base: f.base, on_origin: f.on_origin, distance: f.offset, anchor: f.anchor, extent: f.extent }
    }

    pub fn feature(&self, name: String) -> PlaneFeature {
        PlaneFeature { name, base: self.base, on_origin: self.on_origin, offset: self.distance, anchor: self.anchor, extent: self.extent }
    }

    /// Where the plane would be right now: the base moved along its normal.
    pub fn frame(&self) -> FacePlane {
        FacePlane { origin: self.base.origin + self.base.normal() * self.distance, follow: None, ..self.base }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Offset Plane" } else { "Offset Plane" });
        ui.horizontal(|ui| {
            ui.label("Distance");
            ui.add(egui::DragValue::new(&mut self.distance).speed(0.5).suffix(" mm"));
            if ui.button("Flip").clicked() {
                self.distance = -self.distance;
            }
        });
        ui.weak("Drag the blue arrow, or type. Negative goes the other way.");
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        let mut action = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(error.is_none() && self.distance.is_finite(), egui::Button::new("OK")).clicked() {
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
    use glam::DVec3;

    #[test]
    fn the_plane_sits_the_distance_along_the_normal_and_round_trips() {
        let xy = FacePlane::from_face(DVec3::Z, DVec3::ZERO);
        let mut d = PlaneDialog::new(xy, true, [0.0, 0.0], [0.0; 4]);
        assert_eq!(d.distance, 0.0, "starts on what was picked");
        d.distance = -4.5;
        assert!((d.frame().origin - DVec3::new(0.0, 0.0, -4.5)).length() < 1e-12);
        let f = d.feature("Plane1".into());
        assert_eq!(PlaneDialog::from_feature(2, &f).feature("Plane1".into()), f);
        assert_eq!(PlaneDialog::from_feature(2, &f).editing, Some(2));
    }
}
