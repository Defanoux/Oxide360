//! The Section Analysis dialog (Inspect → Section Analysis, as in Fusion):
//! click a plane or flat face, set how far along it, flip which half stays.
//! The cut shows as you go; OK keeps it in the browser's Analysis folder.

use crate::copy_dialog::Action;
use crate::model::{ExtrudeTarget, SectionAnalysis};

pub struct SectionDialog {
    /// Which section is being edited (None: a new one).
    pub editing: Option<usize>,
    pub at: Option<ExtrudeTarget>,
    pub offset: f64,
    pub flip: bool,
    /// Waiting for a click on a plane or face.
    pub picking: bool,
}

impl SectionDialog {
    pub fn new() -> Self {
        SectionDialog { editing: None, at: None, offset: 0.0, flip: false, picking: true }
    }

    pub fn edit(k: usize, s: &SectionAnalysis) -> Self {
        SectionDialog { editing: Some(k), at: Some(s.at), offset: s.offset, flip: s.flip, picking: false }
    }

    /// The section as set so far (once a plane is picked).
    pub fn section(&self, name: String) -> Option<SectionAnalysis> {
        Some(SectionAnalysis { name, at: self.at?, offset: self.offset, flip: self.flip, shown: true })
    }

    /// `plane_name`: what to call the picked plane or face.
    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, plane_name: Option<String>, error: Option<&str>) -> Action {
        ui.heading("Section Analysis");
        ui.horizontal(|ui| {
            ui.label("Plane");
            let text = if self.picking { "Click a plane or face…".to_string() } else { plane_name.unwrap_or_else(|| "none".into()) };
            if ui.selectable_label(self.picking, text).on_hover_text("Click, then pick an origin plane, construction plane or flat face in the view").clicked() {
                self.picking = !self.picking;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Distance");
            ui.add(crate::units::length_drag(&mut self.offset, unit, 0.5));
            if ui.button("Flip").on_hover_text("Keep the other half").clicked() {
                self.flip = !self.flip;
            }
        });
        ui.weak("Shows the model cut; nothing in the design changes.");
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 80), e);
        }
        let mut act = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.at.is_some(), egui::Button::new("OK")).clicked() {
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
    use crate::sketch::OriginPlane;

    #[test]
    fn nothing_to_keep_until_a_plane_is_picked() {
        let mut d = SectionDialog::new();
        assert!(d.picking && d.section("S".into()).is_none());
        d.at = Some(ExtrudeTarget::Origin(OriginPlane::XZ));
        d.offset = 4.0;
        d.flip = true;
        let s = d.section("Section1".into()).unwrap();
        assert_eq!((s.offset, s.flip, s.shown), (4.0, true, true));
        let again = SectionDialog::edit(0, &s);
        assert_eq!((again.editing, again.at, again.picking), (Some(0), Some(ExtrudeTarget::Origin(OriginPlane::XZ)), false));
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = SectionDialog::new();
        crate::ui_shot::shot(&format!("{dir}/section_dialog.png"), [280, 170], |ui| {
            d.panel(ui, crate::units::Unit::Mm, None, None);
        });
    }
}
