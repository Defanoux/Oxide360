//! The Sweep, Loft, Pipe and Coil dialogs (Fusion's Create menu). Sweep: a
//! sketch's profiles and a path of sketch curves. Loft: one profile from
//! each of two or more sketches, in the order clicked. Pipe: a round section
//! along a path. Coil: a helix of a section round a plane's normal.

use crate::copy_dialog::Action;
use crate::model::{CoilSection, ExtrudeTarget, Operation, SweepFeature, SweepKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Sweep,
    Loft,
    Pipe,
    Coil,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Sweep => "Sweep",
            Mode::Loft => "Loft",
            Mode::Pipe => "Pipe",
            Mode::Coil => "Coil",
        }
    }

    pub fn of(kind: &SweepKind) -> Mode {
        match kind {
            SweepKind::Path { .. } => Mode::Sweep,
            SweepKind::Loft { .. } => Mode::Loft,
            SweepKind::Pipe { .. } => Mode::Pipe,
            SweepKind::Coil { .. } => Mode::Coil,
        }
    }
}

pub struct SweepDialog {
    /// Which one is being edited (None: a new one).
    pub editing: Option<usize>,
    pub mode: Mode,
    /// Sweep: the profiles' sketch and which of its regions.
    pub sketch: Option<usize>,
    pub profiles: Vec<usize>,
    /// Sweep, Pipe: the path's sketch and its curves (entities).
    pub path_sketch: Option<usize>,
    pub path: Vec<usize>,
    /// Sweep: clicks pick the path (else profiles). Pipe: always the path.
    pub picking_path: bool,
    /// Loft: (sketch, region) in order.
    pub sections: Vec<(usize, usize)>,
    pub ruled: bool,
    /// Pipe: the section's diameter and (0: solid) wall thickness.
    pub diameter: f64,
    pub thickness: f64,
    /// Coil: the plane it stands on (picked by a click), its centre there, size and turns.
    pub at: Option<ExtrudeTarget>,
    pub picking_plane: bool,
    pub centre: [f64; 2],
    pub coil_diameter: f64,
    pub turns: f64,
    pub height: f64,
    pub section: CoilSection,
    pub size: f64,
    pub op: Operation,
    /// What the preview shows: `feature()` as it was the last time nothing
    /// was being dragged or typed (see `settle`).
    pub shown: Option<SweepFeature>,
}

impl SweepDialog {
    /// A new one; joining the part if there is one, as in Fusion.
    pub fn new(mode: Mode, has_bodies: bool) -> Self {
        SweepDialog {
            editing: None,
            mode,
            sketch: None,
            profiles: Vec::new(),
            path_sketch: None,
            path: Vec::new(),
            picking_path: mode == Mode::Pipe,
            sections: Vec::new(),
            ruled: false,
            diameter: 5.0,
            thickness: 0.0,
            at: None,
            picking_plane: mode == Mode::Coil,
            centre: [0.0, 0.0],
            coil_diameter: 20.0,
            turns: 5.0,
            height: 30.0,
            section: CoilSection::Circle,
            size: 2.0,
            op: if has_bodies && mode != Mode::Coil { Operation::Join } else { Operation::NewBody },
            shown: None,
        }
    }

    pub fn edit(k: usize, f: &SweepFeature) -> Self {
        let mut d = Self::new(Mode::of(&f.kind), true);
        d.editing = Some(k);
        (d.op, d.picking_path, d.picking_plane) = (f.op, false, false);
        match &f.kind {
            SweepKind::Path { sketch, profiles, path_sketch, path } => {
                (d.sketch, d.profiles, d.path_sketch, d.path) = (Some(*sketch), profiles.clone(), Some(*path_sketch), path.clone());
            }
            SweepKind::Loft { sections, ruled } => (d.sections, d.ruled) = (sections.clone(), *ruled),
            SweepKind::Pipe { path_sketch, path, diameter, thickness } => {
                (d.path_sketch, d.path, d.diameter, d.thickness) = (Some(*path_sketch), path.clone(), *diameter, *thickness);
            }
            SweepKind::Coil { at, centre, diameter, turns, height, section, size } => {
                (d.at, d.centre, d.coil_diameter, d.turns, d.height, d.section, d.size) = (Some(*at), *centre, *diameter, *turns, *height, *section, *size);
            }
        }
        d
    }

    /// Bring the preview up to date, unless a value is still being dragged
    /// or typed (`busy`): rebuilding on every step is too slow for a coil.
    pub fn settle(&mut self, busy: bool) {
        if !busy {
            self.shown = self.feature(String::new());
        }
    }

    /// The feature as set so far (None until there's enough to build).
    pub fn feature(&self, name: String) -> Option<SweepFeature> {
        let kind = match self.mode {
            Mode::Loft if self.sections.len() >= 2 => SweepKind::Loft { sections: self.sections.clone(), ruled: self.ruled },
            Mode::Sweep if !self.profiles.is_empty() && !self.path.is_empty() => {
                SweepKind::Path { sketch: self.sketch?, profiles: self.profiles.clone(), path_sketch: self.path_sketch?, path: self.path.clone() }
            }
            Mode::Pipe if !self.path.is_empty() => SweepKind::Pipe { path_sketch: self.path_sketch?, path: self.path.clone(), diameter: self.diameter, thickness: self.thickness },
            Mode::Coil => SweepKind::Coil { at: self.at?, centre: self.centre, diameter: self.coil_diameter, turns: self.turns, height: self.height, section: self.section, size: self.size },
            _ => return None,
        };
        Some(SweepFeature { name, kind, op: self.op })
    }

    /// A profile clicked (`sketch`, `region`): a sweep toggles it (one
    /// sketch's profiles); a loft adds it as the next section, or drops it
    /// if it's already one.
    pub fn click_profile(&mut self, sketch: usize, region: usize) {
        if self.mode == Mode::Loft {
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

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>) -> Action {
        ui.heading(self.mode.name());
        let path_row = |ui: &mut egui::Ui, d: &mut SweepDialog| {
            ui.horizontal(|ui| {
                ui.label("Path");
                let text = if d.path.is_empty() { "Click a sketch curve…".to_string() } else { format!("{} curves", d.path.len()) };
                if ui.selectable_label(d.picking_path, text).on_hover_text("Click a line, arc or spline; the curves joined to it come too").clicked() {
                    d.picking_path = true;
                }
            });
        };
        match self.mode {
            Mode::Loft => {
                ui.label(format!("Profiles: {} (click one on each sketch, in order)", self.sections.len()));
                if !self.sections.is_empty() && ui.small_button("Clear").clicked() {
                    self.sections.clear();
                }
                ui.checkbox(&mut self.ruled, "Straight between profiles");
            }
            Mode::Sweep => {
                ui.horizontal(|ui| {
                    ui.label("Profile");
                    let text = if self.profiles.is_empty() { "Click a profile…".to_string() } else { format!("{} selected", self.profiles.len()) };
                    if ui.selectable_label(!self.picking_path, text).clicked() {
                        self.picking_path = false;
                    }
                });
                path_row(ui, self);
            }
            Mode::Pipe => {
                path_row(ui, self);
                egui::Grid::new("pipe_dialog").num_columns(2).show(ui, |ui| {
                    ui.label("Diameter");
                    ui.add(crate::units::length_drag(&mut self.diameter, unit, 0.1).range(0.01..=f64::MAX));
                    ui.end_row();
                    ui.label("Wall (0: solid)");
                    ui.add(crate::units::length_drag(&mut self.thickness, unit, 0.1).range(0.0..=f64::MAX));
                    ui.end_row();
                });
            }
            Mode::Coil => {
                ui.horizontal(|ui| {
                    ui.label("Plane");
                    let text = if self.picking_plane { "Click a plane or face…" } else if self.at.is_some() { "picked" } else { "none" };
                    if ui.selectable_label(self.picking_plane, text).on_hover_text("Click an origin plane, construction plane or flat face (a face: where you click is the centre)").clicked() {
                        self.picking_plane = true;
                    }
                });
                egui::Grid::new("coil_dialog").num_columns(2).show(ui, |ui| {
                    ui.label("Centre");
                    ui.horizontal(|ui| {
                        ui.add(crate::units::length_drag(&mut self.centre[0], unit, 0.5));
                        ui.add(crate::units::length_drag(&mut self.centre[1], unit, 0.5));
                    });
                    ui.end_row();
                    ui.label("Diameter");
                    ui.add(crate::units::length_drag(&mut self.coil_diameter, unit, 0.5).range(0.01..=f64::MAX));
                    ui.end_row();
                    ui.label("Revolutions");
                    ui.add(egui::DragValue::new(&mut self.turns).speed(0.1).range(0.1..=500.0));
                    ui.end_row();
                    ui.label("Height");
                    ui.add(crate::units::length_drag(&mut self.height, unit, 0.5));
                    ui.end_row();
                    ui.label("Section");
                    egui::ComboBox::from_id_salt("coil_section").selected_text(self.section.name()).show_ui(ui, |ui| {
                        for s in CoilSection::ALL {
                            ui.selectable_value(&mut self.section, s, s.name());
                        }
                    });
                    ui.end_row();
                    ui.label("Section size");
                    ui.add(crate::units::length_drag(&mut self.size, unit, 0.1).range(0.01..=f64::MAX));
                    ui.end_row();
                });
            }
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
    use crate::sketch::OriginPlane;

    #[test]
    fn the_preview_waits_until_dragging_or_typing_stops() {
        let mut d = SweepDialog::new(Mode::Coil, false);
        d.at = Some(ExtrudeTarget::Origin(crate::sketch::OriginPlane::XY));
        d.settle(false);
        assert!(matches!(d.shown.as_ref().map(|f| &f.kind), Some(SweepKind::Coil { turns, .. }) if *turns == 5.0));
        d.turns = 8.0;
        d.settle(true);
        assert!(matches!(d.shown.as_ref().map(|f| &f.kind), Some(SweepKind::Coil { turns, .. }) if *turns == 5.0), "not while busy");
        d.settle(false);
        assert!(matches!(d.shown.as_ref().map(|f| &f.kind), Some(SweepKind::Coil { turns, .. }) if *turns == 8.0));
    }

    #[test]
    fn a_sweep_needs_a_profile_and_a_path_and_profiles_come_from_one_sketch() {
        let mut d = SweepDialog::new(Mode::Sweep, false);
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
        let mut d = SweepDialog::new(Mode::Loft, true);
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
    fn pipe_waits_for_a_path_and_coil_for_a_plane() {
        let mut p = SweepDialog::new(Mode::Pipe, false);
        assert!(p.picking_path && p.feature("P".into()).is_none());
        (p.path_sketch, p.path) = (Some(0), vec![0]);
        assert!(matches!(p.feature("P".into()).unwrap().kind, SweepKind::Pipe { diameter: 5.0, thickness: 0.0, .. }));
        let mut c = SweepDialog::new(Mode::Coil, true);
        assert_eq!(c.op, Operation::NewBody, "a coil is its own body by default");
        assert!(c.picking_plane && c.feature("C".into()).is_none());
        c.at = Some(ExtrudeTarget::Origin(OriginPlane::XY));
        let f = c.feature("C".into()).unwrap();
        assert_eq!(Mode::of(&f.kind), Mode::Coil);
        assert_eq!(SweepDialog::edit(0, &f).turns, 5.0);
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        for (name, mode) in [("sweep_dialog", Mode::Sweep), ("coil_dialog", Mode::Coil), ("pipe_dialog", Mode::Pipe)] {
            let mut d = SweepDialog::new(mode, true);
            crate::ui_shot::shot(&format!("{dir}/{name}.png"), [360, 330], |ui| {
                d.panel(ui, crate::units::Unit::Mm, None);
            });
        }
    }
}
