//! The Rectangular / Circular Pattern dialog inside a sketch: what's been set
//! so far, and the sketch it makes (shown live; Enter keeps it).

use crate::copy_dialog::Action;
use crate::pattern::{Layout, MAX_COPIES, pattern};
use crate::sketch::{Sel, Sketch};
use eframe::egui;
use glam::DVec2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Rectangular,
    Circular,
}

#[derive(Clone, Debug)]
pub struct SketchPattern {
    pub mode: Mode,
    /// The sketch as it was before the pattern; Cancel puts it back.
    pub base: Sketch,
    pub objects: Vec<Sel>,
    pub count: u32,
    pub spacing: f64,
    pub count2: u32,
    pub spacing2: f64,
    pub center: DVec2,
    pub picking_center: bool,
    pub count_round: u32,
    pub angle: f64,
    /// Extra distance of the copies from the centre, mm.
    pub radial: f64,
}

impl SketchPattern {
    pub fn new(mode: Mode, base: Sketch, objects: Vec<Sel>) -> Self {
        let size = base_size(&base, &objects).max(1.0);
        SketchPattern {
            mode,
            base,
            objects,
            count: 3,
            spacing: (size * 1.5 * 100.0).round() / 100.0,
            count2: 1,
            spacing2: (size * 1.5 * 100.0).round() / 100.0,
            center: DVec2::ZERO,
            picking_center: false,
            count_round: 4,
            angle: 360.0,
            radial: 0.0,
        }
    }

    pub fn layout(&self) -> Layout {
        match self.mode {
            Mode::Rectangular => Layout::Rectangular { count: self.count, spacing: self.spacing, count2: self.count2, spacing2: self.spacing2 },
            Mode::Circular => Layout::Circular { center: self.center, count: self.count_round, angle: self.angle, shift: self.out_dir().unwrap_or(DVec2::ZERO) * self.radial },
        }
    }

    /// The sketch with the copies in place (geometry only, for showing live).
    pub fn preview(&self) -> Result<Sketch, String> {
        let mut sk = self.base.clone();
        pattern(&mut sk, &self.objects, self.layout())?;
        Ok(sk)
    }

    /// The sketch to keep: the copies plus the constraints that carry over
    /// (any the solver refuses as redundant are left out).
    pub fn result(&self) -> Result<Sketch, String> {
        let mut sk = self.base.clone();
        let (cs, _) = pattern(&mut sk, &self.objects, self.layout())?;
        for c in cs {
            let mut trial = sk.clone();
            if crate::solver::add_constraints(&mut trial, &[c]).is_ok() {
                sk = trial;
            }
        }
        Ok(sk)
    }

    /// The middle of the selected geometry (where the arrows start).
    pub fn centroid(&self) -> DVec2 {
        selection_centre(&self.base, &self.objects)
    }

    /// Which way the selection lies from the centre.
    pub fn out_dir(&self) -> Option<DVec2> {
        (self.centroid() - self.center).try_normalize()
    }

    /// How far the selection reaches outward past its middle.
    pub fn reach(&self) -> f64 {
        let (Some(out), c) = (self.out_dir(), self.centroid()) else { return 0.0 };
        selection_points(&self.base, &self.objects).iter().map(|p| (*p - c).dot(out)).fold(0.0, f64::max)
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, error: Option<&str>) -> Action {
        ui.heading(match self.mode {
            Mode::Rectangular => "Rectangular Pattern",
            Mode::Circular => "Circular Pattern",
        });
        let count_box = |ui: &mut egui::Ui, n: &mut u32| {
            ui.horizontal(|ui| {
                ui.label("Quantity");
                ui.add(egui::DragValue::new(n).range(1..=MAX_COPIES).speed(0.2));
            });
        };
        match self.mode {
            Mode::Rectangular => {
                ui.strong("Along X");
                count_box(ui, &mut self.count);
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    ui.add(egui::DragValue::new(&mut self.spacing).speed(0.5).suffix(" mm"));
                });
                ui.strong("Along Y");
                count_box(ui, &mut self.count2);
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    ui.add(egui::DragValue::new(&mut self.spacing2).speed(0.5).suffix(" mm"));
                });
            }
            Mode::Circular => {
                ui.horizontal(|ui| {
                    ui.label("Centre");
                    let text = if self.picking_center {
                        "Click a point…".to_string()
                    } else {
                        format!("{:.1}, {:.1}", self.center.x, self.center.y)
                    };
                    if ui.selectable_label(self.picking_center, text).clicked() {
                        self.picking_center = !self.picking_center;
                    }
                    if ui.button("Origin").clicked() {
                        self.center = DVec2::ZERO;
                        self.picking_center = false;
                    }
                });
                count_box(ui, &mut self.count_round);
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    ui.add(egui::DragValue::new(&mut self.radial).speed(0.5).suffix(" mm"));
                });
                ui.horizontal(|ui| {
                    ui.label("Angle");
                    ui.add(egui::DragValue::new(&mut self.angle).speed(1.0).range(-360.0..=360.0).suffix("°"));
                    if ui.button("Full").clicked() {
                        self.angle = 360.0;
                    }
                });
            }
        }
        ui.weak("Drag the blue arrows, or type. Quantity counts the original.");
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        ui.strong("Press Enter to apply  ·  Esc to cancel");
        let mut action = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(error.is_none(), egui::Button::new("OK (Enter)")).clicked() {
                action = Action::Ok;
            }
            if ui.button("Cancel").clicked() {
                action = Action::Cancel;
            }
        });
        action
    }
}

/// Every point of the selected entities.
fn selection_points(sk: &Sketch, objects: &[Sel]) -> Vec<DVec2> {
    use crate::sketch::Entity;
    let mut pts = Vec::new();
    for s in objects {
        match *s {
            Sel::Point(p) => pts.extend(sk.points.get(p).copied()),
            Sel::Entity(e) => match sk.entities.get(e) {
                Some(&Entity::Line { a, b }) => pts.extend([sk.points[a], sk.points[b]]),
                Some(&Entity::Circle { center, radius }) => {
                    let c = sk.points[center];
                    pts.extend([c + DVec2::splat(radius), c - DVec2::splat(radius)]);
                }
                Some(&Entity::Point { p }) => pts.push(sk.points[p]),
                Some(&Entity::Arc { center, a, b }) => pts.extend([sk.points[center], sk.points[a], sk.points[b]]),
                None => {}
            },
        }
    }
    pts
}

fn selection_centre(sk: &Sketch, objects: &[Sel]) -> DVec2 {
    let pts = selection_points(sk, objects);
    if pts.is_empty() {
        return DVec2::ZERO;
    }
    let (lo, hi) = pts.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(l, h), p| (l.min(*p), h.max(*p)));
    (lo + hi) * 0.5
}

/// How big the selection is across (for a first spacing that clears it).
fn base_size(sk: &Sketch, objects: &[Sel]) -> f64 {
    let pts = selection_points(sk, objects);
    let (lo, hi) = pts.iter().fold((DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)), |(l, h), p| (l.min(*p), h.max(*p)));
    if pts.is_empty() { 0.0 } else { (hi - lo).max_element() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};

    fn square() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Rectangle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(2.0, 2.0)));
        for c in ed.click(&mut sk, Anchor::Free(DVec2::new(6.0, 6.0))) {
            crate::solver::add_constraints(&mut sk, &[c]).unwrap();
        }
        sk
    }

    #[test]
    fn preview_adds_geometry_and_result_adds_the_constraints_too() {
        let sk = square();
        let n = sk.entities.len();
        let sel: Vec<Sel> = (0..n).map(Sel::Entity).collect();
        let mut d = SketchPattern::new(Mode::Rectangular, sk.clone(), sel);
        assert!(d.spacing >= 4.0, "the first spacing clears the shape: {}", d.spacing);
        d.count = 2;
        d.count2 = 2;
        let live = d.preview().unwrap();
        assert_eq!(live.entities.len(), n * 4);
        assert_eq!(live.constraints.len(), sk.constraints.len(), "the live view is geometry only");
        let kept = d.result().unwrap();
        assert_eq!(kept.entities.len(), n * 4);
        assert!(kept.constraints.len() > sk.constraints.len(), "H/V and equal carried over");
        // The base is untouched, so Cancel can put it back.
        assert_eq!(d.base.entities.len(), n);
    }

    #[test]
    fn the_centre_of_the_selection_and_a_single_copy_error() {
        let sk = square();
        let n = sk.entities.len();
        let sel: Vec<Sel> = (0..n).map(Sel::Entity).collect();
        let mut d = SketchPattern::new(Mode::Circular, sk, sel);
        assert!(d.centroid().distance(DVec2::new(4.0, 4.0)) < 1e-9);
        d.count_round = 1;
        assert!(d.preview().is_err(), "one copy is nothing to show");
        d.count_round = 6;
        d.center = DVec2::new(4.0, 4.0);
        assert!(d.preview().is_ok());
    }
}
