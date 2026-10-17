//! The Joint dialog (Fusion's ASSEMBLE → Joint): click where the part that
//! moves attaches, then where it goes on another part; choose the motion
//! (rigid, revolute, slider), the angle and offset, flip if it faces wrong.

use crate::copy_dialog::Action;
use crate::model::{JointFeature, JointKind, JointSnap};
use eframe::egui;

/// Which end takes the next click.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The part that moves (Fusion's Component 1).
    Moves,
    /// Where it goes (Component 2).
    Onto,
}

pub struct JointDialog {
    pub editing: Option<usize>,
    pub name: String,
    pub a: Option<JointSnap>,
    pub b: Option<JointSnap>,
    pub kind: JointKind,
    pub angle: f64,
    pub offset: f64,
    pub flip: bool,
    pub picking: Option<End>,
}

impl JointDialog {
    pub fn new(name: String) -> Self {
        JointDialog { editing: None, name, a: None, b: None, kind: JointKind::Rigid, angle: 0.0, offset: 0.0, flip: false, picking: Some(End::Moves) }
    }

    pub fn edit(k: usize, j: &JointFeature) -> Self {
        JointDialog { editing: Some(k), name: j.name.clone(), a: Some(j.a), b: Some(j.b), kind: j.kind, angle: j.angle, offset: j.offset, flip: j.flip, picking: None }
    }

    /// The joint as set so far (once both ends are picked).
    pub fn feature(&self) -> Option<JointFeature> {
        Some(JointFeature { name: self.name.clone(), a: self.a?, b: self.b?, kind: self.kind, angle: self.angle, offset: self.offset, flip: self.flip })
    }

    /// A click in the view picked `s` for the end being picked; then on to
    /// the other end if it isn't set yet.
    pub fn pick(&mut self, s: JointSnap) {
        match self.picking {
            Some(End::Moves) => {
                self.a = Some(s);
                self.picking = if self.b.is_none() { Some(End::Onto) } else { None };
            }
            Some(End::Onto) => {
                self.b = Some(s);
                self.picking = None;
            }
            None => {}
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>) -> Action {
        ui.heading("Joint");
        let mut field = |ui: &mut egui::Ui, label: &str, end: End, set: bool, tip: &str| {
            ui.horizontal(|ui| {
                ui.label(label);
                let on = self.picking == Some(end);
                let text = if on { "Click a point…" } else if set { "picked" } else { "none" };
                if ui.selectable_label(on, text).on_hover_text(tip).clicked() {
                    self.picking = if on { None } else { Some(end) };
                }
            });
        };
        field(ui, "Moves", End::Moves, self.a.is_some(), "The part that moves: click a round edge (its centre), a face, or a corner on it");
        field(ui, "Onto", End::Onto, self.b.is_some(), "Where it goes: a round edge, face or corner on another part");
        ui.horizontal(|ui| {
            ui.label("Motion");
            for k in JointKind::ALL {
                ui.selectable_value(&mut self.kind, k, k.name());
            }
        });
        egui::Grid::new("joint_values").num_columns(2).show(ui, |ui| {
            // (Same labels whatever the motion, so the boxes don't jump about.)
            ui.label("Angle").on_hover_text("Turns it about the joint's axis (a revolute joint's motion)");
            ui.add(egui::DragValue::new(&mut self.angle).speed(1.0).suffix("°"));
            ui.end_row();
            ui.label("Offset").on_hover_text("Slides it along the joint's axis (a slider joint's motion)");
            ui.add(crate::units::length_drag(&mut self.offset, unit, 0.5));
            ui.end_row();
        });
        if ui.button("Flip").on_hover_text("Face the other way").clicked() {
            self.flip = !self.flip;
        }
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 80), e);
        }
        let mut act = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.feature().is_some(), egui::Button::new("OK")).clicked() {
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
    use glam::DVec3;

    #[test]
    fn clicks_fill_the_moving_end_then_the_other() {
        let mut d = JointDialog::new("Joint1".into());
        assert!(d.feature().is_none());
        d.pick(JointSnap::Corner(DVec3::X));
        assert_eq!((d.a, d.picking), (Some(JointSnap::Corner(DVec3::X)), Some(End::Onto)));
        d.pick(JointSnap::Corner(DVec3::Y));
        assert_eq!(d.picking, None);
        let j = d.feature().expect("both ends");
        assert_eq!((j.a, j.b, j.kind), (JointSnap::Corner(DVec3::X), JointSnap::Corner(DVec3::Y), JointKind::Rigid));
        // Re-picking the moving end leaves the other as it was.
        d.picking = Some(End::Moves);
        d.pick(JointSnap::Corner(DVec3::Z));
        assert_eq!((d.a, d.b, d.picking), (Some(JointSnap::Corner(DVec3::Z)), Some(JointSnap::Corner(DVec3::Y)), None));
    }
}
