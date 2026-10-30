//! The Scale dialog (Fusion's, MODIFY): the bodies, the point they scale
//! about, and one factor or one for each of X, Y and Z.

use crate::copy_dialog::Action;
use crate::doc::BodyId;
use crate::model::{ScaleAt, ScaleFeature};
use eframe::egui;

/// Which field takes clicks in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Bodies,
    Point,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScaleDialog {
    pub editing: Option<usize>,
    pub scale: ScaleFeature,
    pub picking: Option<Field>,
}

impl ScaleDialog {
    /// A new scale; `selected`: bodies picked beforehand.
    pub fn new(name: String, selected: &[BodyId]) -> Self {
        let picking = selected.is_empty().then_some(Field::Bodies);
        ScaleDialog { editing: None, scale: ScaleFeature { name, bodies: selected.to_vec(), ..Default::default() }, picking }
    }

    pub fn from_feature(k: usize, f: &ScaleFeature) -> Self {
        ScaleDialog { editing: Some(k), scale: f.clone(), picking: None }
    }

    /// A body clicked: added, or dropped if it was picked.
    pub fn click_body(&mut self, id: BodyId) {
        match self.scale.bodies.iter().position(|b| *b == id) {
            Some(i) => {
                self.scale.bodies.remove(i);
            }
            None => self.scale.bodies.push(id),
        }
    }

    /// The point clicked (Point picking).
    pub fn set_point(&mut self, p: [f64; 3]) {
        self.scale.at = ScaleAt::Point(p);
        self.picking = if self.scale.bodies.is_empty() { Some(Field::Bodies) } else { None };
    }

    /// Switch between one factor and X/Y/Z, keeping the sizes it makes.
    pub fn set_uniform(&mut self, uniform: bool) {
        match (uniform, self.scale.xyz) {
            (true, Some(f)) => {
                self.scale.factor = f[0];
                self.scale.xyz = None;
            }
            (false, None) => self.scale.xyz = Some([self.scale.factor; 3]),
            _ => {}
        }
    }

    pub fn ready(&self) -> bool {
        !self.scale.bodies.is_empty() && self.scale.factors_ok()
    }

    /// The hint at the top of the view while picking.
    pub fn hint(&self) -> Option<&'static str> {
        match self.picking? {
            Field::Bodies => Some("Click the bodies to scale; click one again to drop it   (Esc when done)"),
            Field::Point => Some("Click a point on a body to scale about (corners and middles snap)"),
        }
    }

    /// The panel; `names`: every body's name.
    pub fn panel(&mut self, ui: &mut egui::Ui, names: &[(BodyId, String)], error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Scale" } else { "Scale" });
        let name_of = |id: &BodyId| names.iter().find(|(b, _)| b == id).map_or("(gone)".to_string(), |(_, n)| n.clone());
        egui::Grid::new("scale_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label("Entities");
            ui.horizontal(|ui| {
                let on = self.picking == Some(Field::Bodies);
                let text = match self.scale.bodies.len() {
                    0 if on => "Click bodies in the view…".to_string(),
                    0 => "Click to select".to_string(),
                    1 | 2 => self.scale.bodies.iter().map(name_of).collect::<Vec<_>>().join(", "),
                    n => format!("{n} bodies"),
                };
                let rich = if on { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                if ui.add(egui::Button::selectable(on, rich).min_size(egui::vec2(170.0, 0.0))).on_hover_text("The bodies to scale; click one again to drop it").clicked() {
                    self.picking = if on { None } else { Some(Field::Bodies) };
                }
                if !self.scale.bodies.is_empty() && ui.small_button("×").on_hover_text("Clear").clicked() {
                    self.scale.bodies.clear();
                    self.picking = Some(Field::Bodies);
                }
            });
            ui.end_row();
            ui.label("Point");
            ui.horizontal(|ui| {
                let picking = self.picking == Some(Field::Point);
                if ui.selectable_label(!picking && self.scale.at == ScaleAt::Origin, "Origin").on_hover_text("Scale about the design's origin").clicked() {
                    self.scale.at = ScaleAt::Origin;
                    self.picking = None;
                }
                if ui.selectable_label(!picking && self.scale.at == ScaleAt::BedCentre, "Bottom Centre").on_hover_text("Scale about the middle of the bodies' bottom, so a part on the bed stays on it").clicked() {
                    self.scale.at = ScaleAt::BedCentre;
                    self.picking = None;
                }
                let on = picking || matches!(self.scale.at, ScaleAt::Point(_));
                if ui.selectable_label(on, "Pick").on_hover_text("Click a point on a body").clicked() {
                    self.picking = Some(Field::Point);
                }
            });
            ui.end_row();
            ui.label("Scale Type");
            ui.horizontal(|ui| {
                let uniform = self.scale.xyz.is_none();
                if ui.selectable_label(uniform, "Uniform").clicked() {
                    self.set_uniform(true);
                }
                if ui.selectable_label(!uniform, "Non-Uniform").on_hover_text("A factor for each of X, Y and Z").clicked() {
                    self.set_uniform(false);
                }
            });
            ui.end_row();
            let factor = |ui: &mut egui::Ui, v: &mut f64| {
                ui.add(egui::DragValue::new(v).speed(0.01).range(0.001..=1000.0).max_decimals(4).update_while_editing(false));
            };
            match self.scale.xyz.as_mut() {
                None => {
                    ui.label("Scale Factor");
                    factor(ui, &mut self.scale.factor);
                    ui.end_row();
                }
                Some(f) => {
                    for (k, axis) in ["X Scale", "Y Scale", "Z Scale"].into_iter().enumerate() {
                        ui.label(axis);
                        factor(ui, &mut f[k]);
                        ui.end_row();
                    }
                }
            }
        });
        if let ScaleAt::Point(p) = self.scale.at {
            ui.weak(format!("About ({:.2}, {:.2}, {:.2}) mm", p[0], p[1], p[2]));
        }
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
    use crate::doc::Feature;

    fn body(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    #[test]
    fn bodies_point_and_type() {
        let mut d = ScaleDialog::new("Scale1".into(), &[]);
        assert_eq!(d.picking, Some(Field::Bodies));
        assert!(!d.ready());
        d.click_body(body(0));
        d.click_body(body(1));
        d.click_body(body(0));
        assert_eq!(d.scale.bodies, [body(1)]);
        assert!(d.ready(), "factor 1 about the origin to start");
        d.picking = Some(Field::Point);
        d.set_point([1.0, 2.0, 3.0]);
        assert_eq!((d.scale.at, d.picking), (ScaleAt::Point([1.0, 2.0, 3.0]), None));
        // Uniform 2 > non-uniform starts at 2, 2, 2; back takes X.
        d.scale.factor = 2.0;
        d.set_uniform(false);
        assert_eq!(d.scale.xyz, Some([2.0; 3]));
        d.scale.xyz = Some([3.0, 1.0, 1.0]);
        d.set_uniform(true);
        assert_eq!((d.scale.factor, d.scale.xyz), (3.0, None));
        // Picked beforehand: nothing to pick.
        assert_eq!(ScaleDialog::new("Scale2".into(), &[body(2)]).picking, None);
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = ScaleDialog::new("Scale1".into(), &[body(0)]);
        let names = vec![(body(0), "Body1".to_string())];
        crate::ui_shot::shot(&format!("{dir}/scale_dialog.png"), [420, 220], |ui| {
            d.panel(ui, &names, None);
        });
        d.set_uniform(false);
        crate::ui_shot::shot(&format!("{dir}/scale_dialog_xyz.png"), [420, 260], |ui| {
            d.panel(ui, &names, None);
        });
    }
}
