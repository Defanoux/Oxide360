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
    /// How far it has moved the way the joint lets it (see `JointFeature::motion`).
    pub motion: f64,
    pub picking: Option<End>,
    /// Animating the motion: since when (seconds), and the motion to go back to after.
    pub playing: Option<(f64, f64)>,
}

/// Animate: a revolute joint goes round once every 4 s; a slider goes out
/// and back over `SLIDE_SPAN` mm in the same time.
pub const PLAY_SECONDS: f64 = 4.0;
pub const SLIDE_SPAN: f64 = 20.0;

/// The motion `t` seconds into an animation that started at `from`.
pub fn played(kind: JointKind, from: f64, t: f64) -> f64 {
    let k = (t / PLAY_SECONDS).fract();
    match kind {
        JointKind::Rigid => from,
        JointKind::Revolute => from + 360.0 * k,
        // Out and back: a triangle.
        JointKind::Slider => from + SLIDE_SPAN * (1.0 - (2.0 * k - 1.0).abs()),
    }
}

impl JointDialog {
    pub fn new(name: String) -> Self {
        JointDialog { editing: None, name, a: None, b: None, kind: JointKind::Rigid, angle: 0.0, offset: 0.0, flip: false, motion: 0.0, picking: Some(End::Moves), playing: None }
    }

    pub fn edit(k: usize, j: &JointFeature) -> Self {
        JointDialog { editing: Some(k), name: j.name.clone(), a: Some(j.a), b: Some(j.b), kind: j.kind, angle: j.angle, offset: j.offset, flip: j.flip, motion: j.motion, picking: None, playing: None }
    }

    /// The joint as set so far (once both ends are picked).
    pub fn feature(&self) -> Option<JointFeature> {
        Some(JointFeature { name: self.name.clone(), a: self.a?, b: self.b?, kind: self.kind, angle: self.angle, offset: self.offset, flip: self.flip, motion: self.motion })
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

    /// Start or stop animating; stopping goes back to where it was.
    pub fn toggle_play(&mut self, now: f64) {
        match self.playing.take() {
            Some((_, back)) => self.motion = back,
            None => self.playing = Some((now, self.motion)),
        }
    }

    /// Each frame while animating.
    pub fn advance(&mut self, now: f64) {
        if let Some((start, from)) = self.playing {
            self.motion = played(self.kind, from, now - start);
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>, now: f64) -> Action {
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
        // Position: where it's put, whatever the motion (as Fusion's Position tab).
        ui.separator();
        ui.strong("Position");
        egui::Grid::new("joint_position").num_columns(2).show(ui, |ui| {
            ui.label("Angle").on_hover_text("Turns it about the joint's axis where it's put");
            ui.add(egui::DragValue::new(&mut self.angle).speed(1.0).suffix("°"));
            ui.end_row();
            ui.label("Offset").on_hover_text("Lifts it along the joint's axis where it's put");
            ui.add(crate::units::length_drag(&mut self.offset, unit, 0.5));
            ui.end_row();
        });
        if ui.button("Flip").on_hover_text("Face the other way").clicked() {
            self.flip = !self.flip;
        }
        // Motion: how it may move afterwards (as Fusion's Motion tab).
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Motion");
            for k in JointKind::ALL {
                if ui.selectable_value(&mut self.kind, k, k.name()).changed() {
                    self.playing = None;
                    self.motion = 0.0;
                }
            }
        });
        match self.kind {
            JointKind::Rigid => {
                ui.weak("Locked together: it can't move.");
            }
            JointKind::Revolute | JointKind::Slider => {
                ui.horizontal(|ui| {
                    if self.kind == JointKind::Revolute {
                        ui.label("Rotation").on_hover_text("Turned about the joint's axis — the one way a revolute joint moves");
                        ui.add_enabled(self.playing.is_none(), egui::DragValue::new(&mut self.motion).speed(1.0).suffix("°"));
                    } else {
                        ui.label("Slide").on_hover_text("Slid along the joint's axis — the one way a slider moves");
                        ui.add_enabled(self.playing.is_none(), crate::units::length_drag(&mut self.motion, unit, 0.5));
                    }
                    let label = if self.playing.is_some() { "Stop" } else { "Animate" };
                    if ui.button(label).on_hover_text("Play the joint's motion").clicked() {
                        self.toggle_play(now);
                    }
                });
            }
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
    fn animating_turns_a_revolute_round_and_slides_a_slider_out_and_back() {
        assert_eq!(played(JointKind::Revolute, 10.0, 1.0), 10.0 + 90.0);
        assert_eq!(played(JointKind::Slider, 0.0, 1.0), SLIDE_SPAN * 0.5);
        assert_eq!(played(JointKind::Slider, 0.0, 2.0), SLIDE_SPAN, "furthest out half way");
        assert_eq!(played(JointKind::Slider, 0.0, 3.0), SLIDE_SPAN * 0.5, "coming back");
        assert_eq!(played(JointKind::Rigid, 5.0, 1.0), 5.0);
        let mut d = JointDialog::new("J".into());
        d.kind = JointKind::Revolute;
        d.motion = 30.0;
        d.toggle_play(100.0);
        d.advance(101.0);
        assert_eq!(d.motion, 120.0);
        d.toggle_play(101.5);
        assert_eq!((d.motion, d.playing), (30.0, None), "stopping goes back");
    }

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
