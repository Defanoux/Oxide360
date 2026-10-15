//! The Combine dialog (Fusion's): a target body, tool bodies, the operation
//! (join, cut, intersect) and Keep Tools.

use crate::copy_dialog::Action;
use crate::doc::BodyId;
use crate::model::{CombineFeature, Operation};
use eframe::egui;

/// Which body field takes clicks in the view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Target,
    Tools,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CombineDialog {
    pub editing: Option<usize>,
    pub combine: CombineFeature,
    pub picking: Option<Field>,
}

impl CombineDialog {
    /// A new Combine; `selected`: bodies picked in the browser beforehand
    /// (the first is the target, the rest tools).
    pub fn new(name: String, selected: &[BodyId]) -> Self {
        let mut d = CombineDialog { editing: None, combine: CombineFeature { name, ..Default::default() }, picking: Some(Field::Target) };
        for &id in selected {
            d.click(id);
        }
        d
    }

    pub fn from_feature(k: usize, c: &CombineFeature) -> Self {
        CombineDialog { editing: Some(k), combine: c.clone(), picking: None }
    }

    /// A body clicked in the view: the target (then on to the tools, as
    /// Fusion does), or a tool added / dropped.
    pub fn click(&mut self, id: BodyId) {
        let c = &mut self.combine;
        match self.picking {
            Some(Field::Target) => {
                c.target = Some(id);
                c.tools.retain(|t| *t != id);
                self.picking = Some(Field::Tools);
            }
            Some(Field::Tools) if c.target == Some(id) => {}
            Some(Field::Tools) => match c.tools.iter().position(|t| *t == id) {
                Some(i) => {
                    c.tools.remove(i);
                }
                None => c.tools.push(id),
            },
            None => {}
        }
    }

    pub fn ready(&self) -> bool {
        self.combine.target.is_some() && !self.combine.tools.is_empty()
    }

    /// The panel; `names`: every body's name.
    pub fn panel(&mut self, ui: &mut egui::Ui, names: &[(BodyId, String)], error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Combine" } else { "Combine" });
        let name_of = |id: &BodyId| names.iter().find(|(b, _)| b == id).map_or("(gone)".to_string(), |(_, n)| n.clone());
        let c = &mut self.combine;
        egui::Grid::new("combine_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            for (field, label, tip) in [
                (Field::Target, "Target Body", "The body that's kept and changed"),
                (Field::Tools, "Tool Bodies", "The bodies joined to it, cut from it or intersected with it; click one again to drop it"),
            ] {
                ui.label(label);
                ui.horizontal(|ui| {
                    let on = self.picking == Some(field);
                    let (text, any) = match field {
                        Field::Target => (c.target.as_ref().map(name_of), c.target.is_some()),
                        Field::Tools => (
                            match c.tools.len() {
                                0 => None,
                                1 | 2 => Some(c.tools.iter().map(name_of).collect::<Vec<_>>().join(", ")),
                                n => Some(format!("{n} bodies")),
                            },
                            !c.tools.is_empty(),
                        ),
                    };
                    let text = match (on, text) {
                        (_, Some(t)) => t,
                        (true, None) => "Click bodies in the view…".to_string(),
                        (false, None) => "Click to select".to_string(),
                    };
                    let rich = if on { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
                    if ui.add(egui::Button::selectable(on, rich).min_size(egui::vec2(170.0, 0.0))).on_hover_text(tip).clicked() {
                        self.picking = if on { None } else { Some(field) };
                    }
                    if any && ui.small_button("×").on_hover_text("Clear").clicked() {
                        match field {
                            Field::Target => c.target = None,
                            Field::Tools => c.tools.clear(),
                        }
                        self.picking = Some(field);
                    }
                });
                ui.end_row();
            }
            ui.label("Operation");
            ui.horizontal(|ui| {
                for op in [Operation::Join, Operation::Cut, Operation::Intersect] {
                    ui.selectable_value(&mut c.op, op, op.name());
                }
            });
            ui.end_row();
            ui.label("Keep Tools");
            ui.checkbox(&mut c.keep_tools, "").on_hover_text("Keep the tool bodies as they were (else they're used up)");
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
    use crate::doc::Feature;

    fn body(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = CombineDialog::new("Combine1".into(), &[body(0), body(1)]);
        d.combine.op = Operation::Cut;
        let names = vec![(body(0), "Body1".to_string()), (body(1), "Body2".to_string())];
        crate::ui_shot::shot(&format!("{dir}/combine.png"), [420, 220], |ui| {
            d.panel(ui, &names, None);
        });
    }

    #[test]
    fn the_first_click_is_the_target_then_tools_toggle() {
        let mut d = CombineDialog::new("Combine1".into(), &[]);
        assert_eq!(d.picking, Some(Field::Target));
        d.click(body(0));
        assert_eq!((d.combine.target, d.picking), (Some(body(0)), Some(Field::Tools)));
        d.click(body(1));
        d.click(body(2));
        d.click(body(1));
        d.click(body(0));
        assert_eq!(d.combine.tools, vec![body(2)], "a second click drops a tool; the target isn't a tool");
        assert!(d.ready());
        // Picking the target again takes it out of the tools.
        d.picking = Some(Field::Target);
        d.click(body(2));
        assert_eq!((d.combine.target, d.combine.tools.len()), (Some(body(2)), 0));
        // Bodies selected beforehand: the first the target, the rest tools.
        let d = CombineDialog::new("Combine1".into(), &[body(3), body(4)]);
        assert_eq!((d.combine.target, d.combine.tools.clone()), (Some(body(3)), vec![body(4)]));
    }
}
