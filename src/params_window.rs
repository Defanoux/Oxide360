//! Fusion's Change Parameters window: the design's user parameters as a
//! table (name, unit, expression, value, comment), added, edited, renamed
//! (what uses the old name follows) and deleted.

use crate::expr::{self, ParamUnit, UserParameter};
use eframe::egui;

/// Something the window did that the rest of the design must follow.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    Renamed { old: String, new: String },
}

/// The window's own state: names being typed (applied when the box is left).
#[derive(Clone, Debug, Default)]
pub struct ParamsWindow {
    pub open: bool,
    names: Vec<String>,
    pub error: Option<String>,
}

/// Rename parameter `i` to `new` in `params`, and in every expression that
/// uses it. Refused (with the reason) for a bad or taken name.
pub fn rename(params: &mut [UserParameter], i: usize, new: &str) -> Result<Option<Change>, String> {
    let new = new.trim();
    let old = params[i].name.clone();
    if new == old {
        return Ok(None);
    }
    if !expr::good_name(new) {
        return Err(format!("\"{new}\" can't be a name: start with a letter, then letters, digits or _ (not a unit or function)"));
    }
    if params.iter().any(|p| p.name == new) {
        return Err(format!("there's already a parameter called {new}"));
    }
    params[i].name = new.to_string();
    for p in params.iter_mut() {
        p.expression = expr::rename(&p.expression, &old, new);
    }
    Ok(Some(Change::Renamed { old, new: new.to_string() }))
}

/// A name not taken yet: "param1", "param2", …
pub fn fresh_name(params: &[UserParameter]) -> String {
    (1..).map(|k| format!("param{k}")).find(|n| params.iter().all(|p| &p.name != n)).expect("some number is free")
}

impl ParamsWindow {
    /// The window (when open). `used_by(name)`: where the design uses a
    /// parameter, to say why one can't be deleted.
    pub fn show(&mut self, ctx: &egui::Context, params: &mut Vec<UserParameter>, used_by: &dyn Fn(&str) -> Vec<String>) -> Vec<Change> {
        let mut changes = Vec::new();
        if !self.open {
            return changes;
        }
        if self.names.len() != params.len() {
            self.names = params.iter().map(|p| p.name.clone()).collect();
        }
        let mut open = self.open;
        egui::Window::new("Parameters").open(&mut open).resizable(true).default_width(820.0).show(ctx, |ui| {
            changes = self.table(ui, params, used_by);
        });
        self.open = open;
        changes
    }

    pub fn table(&mut self, ui: &mut egui::Ui, params: &mut Vec<UserParameter>, used_by: &dyn Fn(&str) -> Vec<String>) -> Vec<Change> {
        let mut changes = Vec::new();
        if self.names.len() != params.len() {
            self.names = params.iter().map(|p| p.name.clone()).collect();
        }
        let values = expr::evaluate(params);
        let mut delete = None;
        egui::Grid::new("params_grid").num_columns(6).spacing([8.0, 6.0]).striped(true).show(ui, |ui| {
            for h in ["Name", "Unit", "Expression", "Value", "Comment", ""] {
                ui.strong(h);
            }
            ui.end_row();
            for i in 0..params.len() {
                let name_box = ui.add_sized([110.0, 20.0], egui::TextEdit::singleline(&mut self.names[i]));
                if name_box.lost_focus() {
                    match rename(params, i, &self.names[i]) {
                        Ok(c) => {
                            changes.extend(c);
                            self.error = None;
                        }
                        Err(e) => {
                            self.error = Some(e);
                            self.names[i] = params[i].name.clone();
                        }
                    }
                }
                let p = &mut params[i];
                egui::ComboBox::from_id_salt(("param_unit", i)).selected_text(p.unit.name()).width(70.0).show_ui(ui, |ui| {
                    for u in ParamUnit::ALL {
                        ui.selectable_value(&mut p.unit, u, u.name());
                    }
                });
                ui.add_sized([180.0, 20.0], egui::TextEdit::singleline(&mut p.expression).hint_text("e.g. width * 2"));
                match &values[i] {
                    Ok(v) => ui.label(p.unit.show(*v)),
                    Err(e) => ui.colored_label(egui::Color32::from_rgb(230, 120, 90), "error").on_hover_text(e.as_str()),
                };
                ui.add_sized([150.0, 20.0], egui::TextEdit::singleline(&mut p.comment));
                let users = used_by(&p.name);
                let others: Vec<&str> = params.iter().enumerate().filter(|&(j, q)| j != i && expr::uses(&q.expression, &params[i].name)).map(|(_, q)| q.name.as_str()).collect();
                let blocked = !users.is_empty() || !others.is_empty();
                let tip = if blocked {
                    format!("Used by {} — change those first", users.iter().map(String::as_str).chain(others).collect::<Vec<_>>().join(", "))
                } else {
                    "Delete this parameter".to_string()
                };
                if ui.add_enabled(!blocked, egui::Button::new("×").small()).on_hover_text(tip.as_str()).on_disabled_hover_text(tip.as_str()).clicked() {
                    delete = Some(i);
                }
                ui.end_row();
            }
        });
        if let Some(i) = delete {
            params.remove(i);
            self.names.remove(i);
        }
        if ui.button("+ Add parameter").clicked() {
            let name = fresh_name(params);
            params.push(UserParameter { name: name.clone(), unit: ParamUnit::Mm, expression: "10".into(), comment: String::new() });
            self.names.push(name);
        }
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e.as_str());
        }
        ui.weak("Type a parameter's name into any length or angle box (\"width * 2 + 5\"); changing it here changes the design.");
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, expression: &str) -> UserParameter {
        UserParameter { name: name.into(), unit: ParamUnit::Mm, expression: expression.into(), comment: String::new() }
    }

    #[test]
    fn renaming_follows_through_and_bad_names_are_refused() {
        let mut params = vec![p("width", "40"), p("depth", "width / 2"), p("widths", "3")];
        assert_eq!(rename(&mut params, 0, " span "), Ok(Some(Change::Renamed { old: "width".into(), new: "span".into() })));
        assert_eq!(params[1].expression, "span / 2");
        assert_eq!(params[2].name, "widths", "only whole names");
        assert!(rename(&mut params, 0, "depth").is_err(), "taken");
        assert!(rename(&mut params, 0, "2x").is_err());
        assert!(rename(&mut params, 0, "mm").is_err(), "a unit");
        assert_eq!(rename(&mut params, 0, "span"), Ok(None));
        assert_eq!(fresh_name(&[p("param1", "1")]), "param2");
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut w = ParamsWindow { open: true, ..Default::default() };
        let mut params = vec![p("width", "40"), p("depth", "width / 2"), UserParameter { name: "tilt".into(), unit: ParamUnit::Deg, expression: "atan(depth / width)".into(), comment: "lid slope".into() }, p("bad", "nope + 1")];
        crate::ui_shot::shot(&format!("{dir}/params.png"), [800, 260], |ui| {
            w.table(ui, &mut params, &|n| if n == "width" { vec!["Extrude1".into()] } else { Vec::new() });
        });
    }
}
