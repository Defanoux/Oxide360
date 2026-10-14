//! The Offset Plane dialog: a plane a set distance from an origin plane or a
//! body face. Also Plane at Angle: a plane through a line, turned about it.

use crate::copy_dialog::Action;
use crate::model::{Hinge, PlaneAngle, PlaneFeature, angle_frame};
use crate::sketch::FacePlane;
use eframe::egui;
use glam::DVec3;

/// Plane at Angle: what it turns about, and that line as it is now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AngleSetup {
    pub hinge: Hinge,
    /// A point on the line (the plane's origin), its direction, the 0° normal.
    pub line: (DVec3, DVec3, DVec3),
    pub degrees: f64,
    /// The Line field is waiting for a click on another line.
    pub picking_line: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlaneDialog {
    pub editing: Option<usize>,
    pub base: FacePlane,
    pub on_origin: bool,
    pub distance: f64,
    pub anchor: [f64; 2],
    pub extent: [f64; 4],
    /// Plane at Angle (else Offset Plane).
    pub angle: Option<AngleSetup>,
}

impl PlaneDialog {
    /// Starts 0 mm from what was picked.
    pub fn new(base: FacePlane, on_origin: bool, anchor: [f64; 2], extent: [f64; 4]) -> Self {
        PlaneDialog { editing: None, base, on_origin, distance: 0.0, anchor, extent, angle: None }
    }

    /// Plane at Angle through a line (`o`, `d`), starting at 0° (facing `n0`),
    /// drawn `size` across.
    pub fn at_angle(hinge: Hinge, o: DVec3, d: DVec3, n0: DVec3, size: f64) -> Self {
        let h = size / 2.0;
        let angle = Some(AngleSetup { hinge, line: (o, d, n0), degrees: 0.0, picking_line: false });
        let mut dialog = PlaneDialog { editing: None, base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: false, distance: 0.0, anchor: [0.0; 2], extent: [-h, -h, h, h], angle };
        dialog.base = dialog.frame();
        dialog
    }

    /// Turn about another line instead (picked wrong), keeping the angle.
    pub fn set_line(&mut self, hinge: Hinge, o: DVec3, d: DVec3, n0: DVec3, size: f64) {
        if let Some(a) = self.angle.as_mut() {
            (a.hinge, a.line, a.picking_line) = (hinge, (o, d, n0), false);
            let h = size / 2.0;
            self.extent = [-h, -h, h, h];
            self.base = self.frame();
        }
    }

    /// Picking a new line in the view?
    pub fn picking_line(&self) -> bool {
        self.angle.is_some_and(|a| a.picking_line)
    }

    /// Edit a plane; `built` is where it is now (a Plane at Angle's line is
    /// found from it).
    pub fn from_feature(k: usize, f: &PlaneFeature, built: Option<FacePlane>) -> Self {
        let angle = f.angle.map(|a| {
            let now = built.unwrap_or(f.base);
            let (d, n) = (now.u, now.normal());
            // Turn the normal back to 0°.
            let back = (-a.degrees).to_radians();
            AngleSetup { hinge: a.hinge, line: (now.origin, d, n * back.cos() + d.cross(n) * back.sin()), degrees: a.degrees, picking_line: false }
        });
        PlaneDialog { editing: Some(k), base: f.base, on_origin: f.on_origin, distance: f.offset, anchor: f.anchor, extent: f.extent, angle }
    }

    pub fn feature(&self, name: String) -> PlaneFeature {
        let angle = self.angle.map(|a| PlaneAngle { hinge: a.hinge, degrees: a.degrees });
        let base = if self.angle.is_some() { self.frame() } else { self.base };
        PlaneFeature { name, base, on_origin: self.on_origin, offset: self.distance, anchor: self.anchor, extent: self.extent, angle }
    }

    /// Where the plane would be right now: the base moved along its normal,
    /// or turned about its line.
    pub fn frame(&self) -> FacePlane {
        if let Some(a) = self.angle {
            let (o, d, n0) = a.line;
            if let Some(f) = angle_frame(o, d, n0, a.degrees) {
                return f;
            }
        }
        FacePlane { origin: self.base.origin + self.base.normal() * self.distance, follow: None, ..self.base }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>) -> Action {
        if let Some(a) = self.angle.as_mut() {
            ui.heading(if self.editing.is_some() { "Edit Plane at Angle" } else { "Plane at Angle" });
            ui.horizontal(|ui| {
                ui.label("Line");
                // Like Fusion's selection field: click it, then click another line.
                let name = if a.picking_line {
                    "Click a line, edge or axis…".to_string()
                } else {
                    match a.hinge {
                        Hinge::World(w) => format!("{} axis", w.name()),
                        Hinge::Axis(k) => format!("Axis{}", k + 1),
                        Hinge::Edge(_) => "a body edge".into(),
                        Hinge::SketchLine { .. } => "a sketch line".into(),
                    }
                };
                let text = if a.picking_line { egui::RichText::new(name).strong() } else { egui::RichText::new(name) };
                if ui.add(egui::Button::selectable(a.picking_line, text).min_size(egui::vec2(150.0, 0.0))).on_hover_text("Click, then click another line to turn the plane about").clicked() {
                    a.picking_line = !a.picking_line;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Angle");
                ui.add(egui::DragValue::new(&mut a.degrees).speed(1.0).range(-360.0..=360.0).suffix("°"));
                if ui.button("Flip").clicked() {
                    a.degrees = -a.degrees;
                }
            });
            ui.weak("Drag the blue arrow round, or type.");
        } else {
            ui.heading(if self.editing.is_some() { "Edit Offset Plane" } else { "Offset Plane" });
            ui.horizontal(|ui| {
                ui.label("Distance");
                ui.add(crate::units::length_drag(&mut self.distance, unit, 0.5));
                if ui.button("Flip").clicked() {
                    self.distance = -self.distance;
                }
            });
            ui.weak("Drag the blue arrow, or type. Negative goes the other way.");
        }
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
        assert_eq!(PlaneDialog::from_feature(2, &f, None).feature("Plane1".into()), f);
        assert_eq!(PlaneDialog::from_feature(2, &f, None).editing, Some(2));
    }

    #[test]
    fn a_plane_at_angle_saves_its_line_and_angle_and_edits_back() {
        use crate::model::WorldAxis;
        let o = DVec3::new(0.0, 0.0, 10.0);
        let mut d = PlaneDialog::at_angle(Hinge::World(WorldAxis::X), o, DVec3::X, DVec3::Z, 40.0);
        assert!(d.frame().normal().distance(DVec3::Z) < 1e-12, "0° lies in the reference plane");
        d.angle.as_mut().unwrap().degrees = 30.0;
        let f = d.feature("Plane1".into());
        assert_eq!(f.angle, Some(PlaneAngle { hinge: Hinge::World(WorldAxis::X), degrees: 30.0 }));
        assert!(f.base.normal().distance(d.frame().normal()) < 1e-12, "base remembers the plane as made");
        // Editing it again, from where the kernel built it, finds the same line and 0°.
        let back = PlaneDialog::from_feature(0, &f, Some(d.frame()));
        let (bo, bd, bn) = back.angle.unwrap().line;
        assert!(bo.distance(o) < 1e-12 && bd.distance(DVec3::X) < 1e-12 && bn.distance(DVec3::Z) < 1e-12, "{bn}");
        assert!(back.frame().normal().distance(d.frame().normal()) < 1e-12);
        assert_eq!(back.feature("Plane1".into()), f);
    }

    #[test]
    fn picking_another_line_keeps_the_angle() {
        use crate::model::WorldAxis;
        let mut d = PlaneDialog::at_angle(Hinge::World(WorldAxis::X), DVec3::ZERO, DVec3::X, DVec3::Z, 40.0);
        d.angle.as_mut().unwrap().degrees = 90.0;
        d.angle.as_mut().unwrap().picking_line = true;
        assert!(d.picking_line());
        d.set_line(Hinge::World(WorldAxis::Y), DVec3::ZERO, DVec3::Y, DVec3::Z, 20.0);
        assert!(!d.picking_line());
        let a = d.angle.unwrap();
        assert_eq!((a.hinge, a.degrees), (Hinge::World(WorldAxis::Y), 90.0));
        // 90° about Y from XY stands up as the YZ plane.
        assert!(d.frame().normal().cross(DVec3::X).length() < 1e-12, "{}", d.frame().normal());
        assert_eq!(d.extent, [-10.0, -10.0, 10.0, 10.0]);
        assert_eq!(d.feature("P".into()).angle.unwrap().hinge, Hinge::World(WorldAxis::Y));
    }
}
