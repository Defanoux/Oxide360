//! The Insert dialog (as Fusion's Insert Mesh / Insert CAD): where an
//! imported file's bodies go — moved and turned by numbers, centred on the
//! origin, set down on the XY plane, or dropped where you click.

use crate::copy_dialog::Action;
use crate::model::ImportFeature;
use eframe::egui;
use glam::DVec3;

pub struct ImportDialog {
    /// Which import is being edited (None: a new one, not in the design yet).
    pub editing: Option<usize>,
    pub feature: ImportFeature,
    /// Clicks in the view drop it there.
    pub picking: bool,
    /// The bodies' box as turned by `bounds_for` (before the move): what
    /// Centre, Ground and clicks place by. The app fills it.
    pub bounds: Option<(DVec3, DVec3)>,
    pub bounds_for: [f64; 3],
}

impl ImportDialog {
    pub fn new(feature: ImportFeature) -> Self {
        ImportDialog { editing: None, feature, picking: false, bounds: None, bounds_for: [f64::NAN; 3] }
    }

    pub fn edit(k: usize, feature: &ImportFeature) -> Self {
        ImportDialog { editing: Some(k), ..Self::new(feature.clone()) }
    }

    /// Does `bounds` need working out again (new, or turned since)?
    pub fn bounds_stale(&self) -> bool {
        self.bounds.is_none() || self.bounds_for != self.feature.angles
    }

    /// The middle of its box on the origin.
    pub fn centre(&mut self) {
        if let Some((lo, hi)) = self.bounds {
            self.feature.offset = (-(lo + hi) * 0.5).to_array();
        }
    }

    /// Its bottom on the XY plane (where it is across stays).
    pub fn ground(&mut self) {
        if let Some((lo, _)) = self.bounds {
            self.feature.offset[2] = -lo.z;
        }
    }

    /// The middle of its bottom at `p` (a clicked spot).
    pub fn place_at(&mut self, p: DVec3) {
        if let Some((lo, hi)) = self.bounds {
            let bottom = DVec3::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, lo.z);
            self.feature.offset = (p - bottom).to_array();
        }
        self.picking = false;
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit) -> Action {
        ui.heading("Insert");
        ui.label(&self.feature.source);
        ui.horizontal(|ui| {
            if ui.selectable_label(self.picking, if self.picking { "Click where it goes…" } else { "Place by clicking" }).on_hover_text("Click a face of the model, or the ground, to put its bottom there").clicked() {
                self.picking = !self.picking;
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Centre").on_hover_text("Its middle on the origin").clicked() {
                self.centre();
            }
            if ui.button("Place on ground").on_hover_text("Its bottom on the XY plane").clicked() {
                self.ground();
            }
        });
        egui::Grid::new("import_place").num_columns(4).show(ui, |ui| {
            ui.label("Move");
            for v in &mut self.feature.offset {
                ui.add(crate::units::length_drag(v, unit, 0.5));
            }
            ui.end_row();
            ui.label("Turn");
            for a in &mut self.feature.angles {
                ui.add(egui::DragValue::new(a).speed(1.0).suffix("°"));
            }
            ui.end_row();
            ui.label("");
            for axis in ["X", "Y", "Z"] {
                ui.weak(axis);
            }
            ui.end_row();
        });
        let mut act = Action::None;
        ui.horizontal(|ui| {
            if ui.button("OK").clicked() {
                act = Action::Ok;
            }
            if ui.button("Cancel").clicked() {
                act = Action::Cancel;
            }
        });
        act
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Blob;

    fn dialog() -> ImportDialog {
        let mut d = ImportDialog::new(ImportFeature { name: "part".into(), source: "part.step".into(), brep: Blob::new(String::new()), offset: [0.0; 3], angles: [0.0; 3] });
        // A part 10 × 20 × 30 sitting from (100, 0, -5).
        d.bounds = Some((DVec3::new(100.0, 0.0, -5.0), DVec3::new(110.0, 20.0, 25.0)));
        d.bounds_for = [0.0; 3];
        d
    }

    #[test]
    fn centre_ground_and_click_place_it() {
        let mut d = dialog();
        assert!(!d.bounds_stale());
        d.centre();
        assert_eq!(d.feature.offset, [-105.0, -10.0, -10.0]);
        d.ground();
        assert_eq!(d.feature.offset, [-105.0, -10.0, 5.0], "bottom (−5) up to 0, across unchanged");
        d.picking = true;
        d.place_at(DVec3::new(0.0, 0.0, 12.0));
        assert_eq!(d.feature.offset, [-105.0, -10.0, 17.0], "bottom middle on the clicked spot");
        assert!(!d.picking);
        d.feature.angles[2] = 90.0;
        assert!(d.bounds_stale(), "turned: its box must be found again");
    }
}
