//! The Sweep and Loft dialogs (Fusion's Create → Sweep / Loft). Sweep: a
//! sketch's profiles and a path of sketch curves. Loft: one profile from
//! each of two or more sketches, in the order clicked.

use crate::copy_dialog::Action;
use crate::model::{Operation, SweepFeature, SweepKind};

pub struct SweepDialog {
    /// Which sweep / loft is being edited (None: a new one).
    pub editing: Option<usize>,
    pub loft: bool,
    /// Sweep: the profiles' sketch and which of its regions.
    pub sketch: Option<usize>,
    pub profiles: Vec<usize>,
    /// Sweep: the path's sketch and its curves (entities).
    pub path_sketch: Option<usize>,
    pub path: Vec<usize>,
    /// Sweep: clicks pick the path (else profiles).
    pub picking_path: bool,
    /// Loft: (sketch, region) in order.
    pub sections: Vec<(usize, usize)>,
    pub ruled: bool,
    pub op: Operation,
}

impl SweepDialog {
    /// A new sweep (`loft`: a loft); joining the part if there is one, as in Fusion.
    pub fn new(loft: bool, has_bodies: bool) -> Self {
        SweepDialog {
            editing: None,
            loft,
            sketch: None,
            profiles: Vec::new(),
            path_sketch: None,
            path: Vec::new(),
            picking_path: false,
            sections: Vec::new(),
            ruled: false,
            op: if has_bodies { Operation::Join } else { Operation::NewBody },
        }
    }

    pub fn edit(k: usize, f: &SweepFeature) -> Self {
        let mut d = Self::new(matches!(f.kind, SweepKind::Loft { .. }), true);
        d.editing = Some(k);
        d.op = f.op;
        match &f.kind {
            SweepKind::Path { sketch, profiles, path_sketch, path } => {
                (d.sketch, d.profiles, d.path_sketch, d.path) = (Some(*sketch), profiles.clone(), Some(*path_sketch), path.clone());
            }
            SweepKind::Loft { sections, ruled } => (d.sections, d.ruled) = (sections.clone(), *ruled),
        }
        d
    }

    /// The feature as set so far (None until there's enough to build).
    pub fn feature(&self, name: String) -> Option<SweepFeature> {
        let kind = if self.loft {
            if self.sections.len() < 2 {
                return None;
            }
            SweepKind::Loft { sections: self.sections.clone(), ruled: self.ruled }
        } else {
            if self.profiles.is_empty() || self.path.is_empty() {
                return None;
            }
            SweepKind::Path { sketch: self.sketch?, profiles: self.profiles.clone(), path_sketch: self.path_sketch?, path: self.path.clone() }
        };
        Some(SweepFeature { name, kind, op: self.op })
    }

    /// A profile clicked (`sketch`, `region`): a sweep toggles it (one
    /// sketch's profiles); a loft adds it as the next section, or drops it
    /// if it's already one.
    pub fn click_profile(&mut self, sketch: usize, region: usize) {
        if self.loft {
            match self.sections.iter().position(|&s| s == (sketch, region)) {
                Some(k) => {
                    self.sections.remove(k);
                }
                // (One section per sketch: another region of the same sketch replaces it.)
                None => match self.sections.iter().position(|s| s.0 == sketch) {
                    Some(k) => self.sections[k] = (sketch, region),
                    None => self.sections.push((sketch, region)),
                },
            }
        } else if self.sketch != Some(sketch) {
            (self.sketch, self.profiles) = (Some(sketch), vec![region]);
        } else if let Some(k) = self.profiles.iter().position(|&r| r == region) {
            self.profiles.remove(k);
        } else {
            self.profiles.push(region);
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, error: Option<&str>) -> Action {
        ui.heading(if self.loft { "Loft" } else { "Sweep" });
        if self.loft {
            ui.label(format!("Profiles: {} (click one on each sketch, in order)", self.sections.len()));
            if !self.sections.is_empty() && ui.small_button("Clear").clicked() {
                self.sections.clear();
            }
            ui.checkbox(&mut self.ruled, "Straight between profiles");
        } else {
            ui.horizontal(|ui| {
                ui.label("Profile");
                let text = if self.profiles.is_empty() { "Click a profile…".to_string() } else { format!("{} selected", self.profiles.len()) };
                if ui.selectable_label(!self.picking_path, text).clicked() {
                    self.picking_path = false;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Path");
                let text = if self.path.is_empty() { "Click a sketch curve…".to_string() } else { format!("{} curves", self.path.len()) };
                if ui.selectable_label(self.picking_path, text).on_hover_text("Click a line, arc or spline; the curves joined to it come too").clicked() {
                    self.picking_path = true;
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("Operation");
            for op in [Operation::Join, Operation::Cut, Operation::NewBody, Operation::Intersect] {
                ui.selectable_value(&mut self.op, op, op.name());
            }
        });
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 80), e);
        }
        let mut act = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.feature(String::new()).is_some(), egui::Button::new("OK")).clicked() {
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

    #[test]
    fn a_sweep_needs_a_profile_and_a_path_and_profiles_come_from_one_sketch() {
        let mut d = SweepDialog::new(false, false);
        assert_eq!(d.op, Operation::NewBody, "nothing to join yet");
        d.click_profile(0, 1);
        d.click_profile(0, 2);
        assert_eq!(d.profiles, vec![1, 2]);
        d.click_profile(0, 1);
        assert_eq!(d.profiles, vec![2], "clicked again: dropped");
        d.click_profile(3, 0);
        assert_eq!((d.sketch, d.profiles.clone()), (Some(3), vec![0]), "another sketch starts over");
        assert!(d.feature("S".into()).is_none());
        (d.path_sketch, d.path) = (Some(1), vec![4, 5]);
        assert!(matches!(d.feature("S".into()).unwrap().kind, SweepKind::Path { sketch: 3, .. }));
    }

    #[test]
    fn a_loft_takes_one_profile_per_sketch_in_order() {
        let mut d = SweepDialog::new(true, true);
        assert_eq!(d.op, Operation::Join);
        d.click_profile(2, 0);
        assert!(d.feature("L".into()).is_none(), "one profile isn't a loft");
        d.click_profile(0, 1);
        d.click_profile(2, 3); // same sketch: replaces its section
        assert_eq!(d.sections, vec![(2, 3), (0, 1)]);
        let f = d.feature("L".into()).unwrap();
        assert_eq!(SweepDialog::edit(0, &f).sections, vec![(2, 3), (0, 1)]);
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = SweepDialog::new(false, true);
        crate::ui_shot::shot(&format!("{dir}/sweep_dialog.png"), [340, 170], |ui| {
            d.panel(ui, None);
        });
    }
}
