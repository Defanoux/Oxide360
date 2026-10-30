//! The Split Body dialog (Fusion's, MODIFY): the bodies to split, then the
//! splitting tool — an origin or construction plane, or a flat face (carried
//! on past its edges).

use crate::copy_dialog::Action;
use crate::doc::BodyId;
use crate::model::{ExtrudeTarget, SplitFeature};
use eframe::egui;

/// Which field takes clicks in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Bodies,
    Tool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplitDialog {
    pub editing: Option<usize>,
    pub split: SplitFeature,
    pub picking: Option<Field>,
}

impl SplitDialog {
    /// A new split; `selected`: bodies picked beforehand (then on to the tool).
    pub fn new(name: String, selected: &[BodyId]) -> Self {
        let picking = Some(if selected.is_empty() { Field::Bodies } else { Field::Tool });
        SplitDialog { editing: None, split: SplitFeature { name, bodies: selected.to_vec(), at: None }, picking }
    }

    pub fn from_feature(k: usize, f: &SplitFeature) -> Self {
        SplitDialog { editing: Some(k), split: f.clone(), picking: None }
    }

    /// A body clicked: added, or dropped if it was picked. The first one picked
    /// moves on to the splitting tool when there isn't one yet.
    pub fn click_body(&mut self, id: BodyId) {
        match self.split.bodies.iter().position(|b| *b == id) {
            Some(i) => {
                self.split.bodies.remove(i);
            }
            None => {
                self.split.bodies.push(id);
                if self.split.at.is_none() {
                    self.picking = Some(Field::Tool);
                }
            }
        }
    }

    /// The plane or face clicked.
    pub fn set_tool(&mut self, t: ExtrudeTarget) {
        self.split.at = Some(t);
        self.picking = if self.split.bodies.is_empty() { Some(Field::Bodies) } else { None };
    }

    pub fn ready(&self) -> bool {
        !self.split.bodies.is_empty() && self.split.at.is_some()
    }

    /// The hint at the top of the view while picking.
    pub fn hint(&self) -> Option<&'static str> {
        match self.picking? {
            Field::Bodies => Some("1 of 2: click the bodies to split; click one again to drop it   (Esc when done)"),
            Field::Tool => Some("2 of 2: click an origin plane, construction plane or flat face to split along"),
        }
    }

    /// The panel; `names`: every body's name; `tool_name`: what the picked plane is.
    pub fn panel(&mut self, ui: &mut egui::Ui, names: &[(BodyId, String)], tool_name: Option<String>, error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Split Body" } else { "Split Body" });
        let name_of = |id: &BodyId| names.iter().find(|(b, _)| b == id).map_or("(gone)".to_string(), |(_, n)| n.clone());
        egui::Grid::new("split_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for (field, label, tip) in [
                (Field::Bodies, "Body to Split", "The bodies cut in two; click one again to drop it"),
                (Field::Tool, "Splitting Tool", "An origin plane, construction plane or flat face (carried on past its edges)"),
            ] {
                ui.label(label);
                ui.horizontal(|ui| {
                    let on = self.picking == Some(field);
                    let chosen = match field {
                        Field::Bodies => match self.split.bodies.len() {
                            0 => None,
                            1 | 2 => Some(self.split.bodies.iter().map(name_of).collect::<Vec<_>>().join(", ")),
                            n => Some(format!("{n} bodies")),
                        },
                        Field::Tool => self.split.at.map(|_| tool_name.clone().unwrap_or_else(|| "a plane".into())),
                    };
                    let any = chosen.is_some();
                    let text = match (on, chosen) {
                        (_, Some(t)) => t,
                        (true, None) if field == Field::Bodies => "Click bodies in the view…".to_string(),
                        (true, None) => "Click a plane or face…".to_string(),
                        (false, None) => "Click to select".to_string(),
                    };
                    let rich = if on { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                    if ui.add(egui::Button::selectable(on, rich).min_size(egui::vec2(170.0, 0.0))).on_hover_text(tip).clicked() {
                        self.picking = if on { None } else { Some(field) };
                    }
                    if any && ui.small_button("×").on_hover_text("Clear").clicked() {
                        match field {
                            Field::Bodies => self.split.bodies.clear(),
                            Field::Tool => self.split.at = None,
                        }
                        self.picking = Some(field);
                    }
                });
                ui.end_row();
            }
        });
        ui.weak("The side the plane faces keeps the body; the rest become new bodies.");
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
    use crate::sketch::OriginPlane;

    fn body(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    #[test]
    fn bodies_then_the_tool() {
        let mut d = SplitDialog::new("Split1".into(), &[]);
        assert_eq!(d.picking, Some(Field::Bodies));
        d.click_body(body(0));
        assert_eq!(d.picking, Some(Field::Tool), "on to the tool after the first body");
        assert!(!d.ready());
        d.set_tool(ExtrudeTarget::Origin(OriginPlane::XY));
        assert!(d.ready() && d.picking.is_none());
        // More bodies, and one dropped again.
        d.picking = Some(Field::Bodies);
        d.click_body(body(1));
        d.click_body(body(0));
        assert_eq!(d.split.bodies, [body(1)]);
        assert_eq!(d.picking, Some(Field::Bodies), "a tool is there: stays on bodies");
        // Bodies picked beforehand: straight to the tool.
        let d = SplitDialog::new("Split2".into(), &[body(2)]);
        assert_eq!(d.picking, Some(Field::Tool));
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = SplitDialog::new("Split1".into(), &[body(0)]);
        let names = vec![(body(0), "Body1".to_string())];
        crate::ui_shot::shot(&format!("{dir}/split_dialog.png"), [400, 190], |ui| {
            d.panel(ui, &names, None, None);
        });
    }
}
