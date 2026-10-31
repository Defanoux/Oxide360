//! The Emboss dialog (Fusion's, CREATE): sketch profiles (text too), the face
//! they go on — flat, or a shaft or hole they wrap round — the depth, and
//! whether they're raised (Emboss) or cut in (Deboss).

use crate::copy_dialog::Action;
use crate::model::{EmbossFace, EmbossFeature};
use crate::units::Unit;
use eframe::egui;

/// Which field takes clicks in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Profiles,
    Face,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmbossDialog {
    pub editing: Option<usize>,
    pub emboss: EmbossFeature,
    /// The profiles' sketch, once one is picked.
    pub sketch: Option<usize>,
    pub picking: Option<Field>,
}

impl EmbossDialog {
    pub fn new(name: String) -> Self {
        EmbossDialog { editing: None, emboss: EmbossFeature { name, ..Default::default() }, sketch: None, picking: Some(Field::Profiles) }
    }

    pub fn from_feature(k: usize, f: &EmbossFeature) -> Self {
        EmbossDialog { editing: Some(k), emboss: f.clone(), sketch: Some(f.sketch), picking: None }
    }

    /// The face clicked; then on to the profiles if there are none yet.
    pub fn set_face(&mut self, f: EmbossFace) {
        self.emboss.face = Some(f);
        self.picking = if self.emboss.profiles.is_empty() { Some(Field::Profiles) } else { None };
    }

    /// The feature as it stands (its sketch filled in), when there's enough to build.
    pub fn feature(&self) -> Option<EmbossFeature> {
        let sketch = self.sketch?;
        (!self.emboss.profiles.is_empty() && self.emboss.face.is_some() && self.emboss.depth > 0.0).then(|| EmbossFeature { sketch, ..self.emboss.clone() })
    }

    pub fn ready(&self) -> bool {
        self.feature().is_some()
    }

    /// The hint at the top of the view while picking.
    pub fn hint(&self) -> Option<&'static str> {
        match self.picking? {
            Field::Profiles => Some("1 of 2: click sketch profiles or text; click one again to drop it"),
            Field::Face => Some("2 of 2: click the face to emboss on — a flat face, a shaft or the inside of a hole"),
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: Unit, error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Emboss" } else { "Emboss" });
        egui::Grid::new("emboss_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for (field, label, tip) in [
                (Field::Profiles, "Profiles", "Sketch profiles or text; click one again to drop it"),
                (Field::Face, "Face", "A flat face (the sketch must be parallel to it), or a shaft or hole (the sketch on a plane along its axis) to wrap round"),
            ] {
                ui.label(label);
                ui.horizontal(|ui| {
                    let on = self.picking == Some(field);
                    let chosen = match field {
                        Field::Profiles => match self.emboss.profiles.len() {
                            0 => None,
                            1 => Some("1 profile".to_string()),
                            n => Some(format!("{n} profiles")),
                        },
                        Field::Face => self.emboss.face.map(|f| match f {
                            EmbossFace::Flat { .. } => "Flat face".to_string(),
                            EmbossFace::Round(c) if c.internal => format!("Hole Ø{}", unit.show(2.0 * c.radius)),
                            EmbossFace::Round(c) => format!("Shaft Ø{}", unit.show(2.0 * c.radius)),
                        }),
                    };
                    let any = chosen.is_some();
                    let text = match (on, chosen) {
                        (_, Some(t)) => t,
                        (true, None) if field == Field::Profiles => "Click profiles in the view…".to_string(),
                        (true, None) => "Click a face…".to_string(),
                        (false, None) => "Click to select".to_string(),
                    };
                    let rich = if on { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                    if ui.add(egui::Button::selectable(on, rich).min_size(egui::vec2(150.0, 0.0))).on_hover_text(tip).clicked() {
                        self.picking = if on { None } else { Some(field) };
                    }
                    if any && ui.small_button("×").on_hover_text("Clear").clicked() {
                        match field {
                            Field::Profiles => {
                                self.emboss.profiles.clear();
                                self.sketch = None;
                            }
                            Field::Face => self.emboss.face = None,
                        }
                        self.picking = Some(field);
                    }
                });
                ui.end_row();
            }
            ui.label("Type");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.emboss.deboss, false, "Emboss").on_hover_text("Raised from the face");
                ui.selectable_value(&mut self.emboss.deboss, true, "Deboss").on_hover_text("Cut into the face");
            });
            ui.end_row();
            ui.label("Depth");
            ui.add(crate::units::length_drag(&mut self.emboss.depth, unit, 0.05).range(0.001..=f64::MAX).update_while_editing(false));
            ui.end_row();
        });
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        let mut action = Action::None;
        let ready = self.ready();
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
    use crate::model::CylinderRef;
    use glam::DVec3;

    fn shaft() -> EmbossFace {
        EmbossFace::Round(CylinderRef { axis_point: DVec3::ZERO, axis: DVec3::Z, radius: 5.0, at: DVec3::new(0.0, -5.0, 2.0), internal: false, length: 10.0 })
    }

    #[test]
    fn profiles_then_face_makes_it_ready() {
        let mut d = EmbossDialog::new("Emboss1".into());
        assert_eq!(d.picking, Some(Field::Profiles));
        assert!(!d.ready());
        (d.sketch, d.emboss.profiles) = (Some(2), vec![0, 3]);
        d.picking = Some(Field::Face);
        d.set_face(shaft());
        assert_eq!(d.picking, None, "nothing left to pick");
        let f = d.feature().unwrap();
        assert_eq!((f.sketch, f.profiles.clone()), (2, vec![0, 3]));
        // Editing comes back as it was.
        let e = EmbossDialog::from_feature(4, &f);
        assert_eq!((e.editing, e.sketch, e.emboss), (Some(4), Some(2), f));
    }

    #[test]
    fn a_face_first_goes_on_to_the_profiles() {
        let mut d = EmbossDialog::new("Emboss1".into());
        d.picking = Some(Field::Face);
        d.set_face(shaft());
        assert_eq!(d.picking, Some(Field::Profiles));
        assert!(!d.ready());
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = EmbossDialog::new("Emboss1".into());
        (d.sketch, d.emboss.profiles) = (Some(0), vec![0, 1, 2]);
        d.set_face(shaft());
        crate::ui_shot::shot(&format!("{dir}/emboss.png"), [360, 240], |ui| {
            d.panel(ui, Unit::Mm, None);
        });
    }
}

#[cfg(test)]
mod demo {
    use crate::doc::{Document, Feature};
    use crate::model::{Extrude, Operation};
    use crate::sketch::{Anchor, Editor, OriginPlane, Sketch, SketchText, Tool};
    use glam::DVec2;

    /// A design to try Emboss on by hand: a shaft and text on the front plane.
    #[test]
    #[ignore]
    fn demo_file() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut base = Sketch::new("Sketch1".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut base, Anchor::Free(DVec2::ZERO));
        ed.click(&mut base, Anchor::Free(DVec2::new(15.0, 0.0)));
        let mut words = Sketch::new("Sketch2".into(), OriginPlane::XZ);
        let st = SketchText { text: "OXIDE".into(), height: 8.0, font: crate::text::HACK.into(), bold: false, italic: false, angle: 0.0 };
        words.add_text(Anchor::Free(DVec2::new(-16.0, 16.0)), st);
        let mut doc = Document::default();
        doc.sketches = vec![base, words];
        doc.extrudes = vec![Extrude { op: Operation::NewBody, name: "Extrude1".into(), sketch: 0, profiles: vec![0], distance: 40.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None }];
        doc.timeline = vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1)];
        crate::doc::save(std::path::Path::new(&format!("{dir}/emboss_demo.ox360")), &doc).unwrap();
    }
}
