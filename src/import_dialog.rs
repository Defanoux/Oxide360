//! The Insert dialog (as Fusion's Insert Mesh / Insert CAD): where an
//! imported file's bodies go — moved and turned by numbers, centred on the
//! origin, set down on the XY plane, or dropped where you click.

use crate::copy_dialog::Action;
use crate::model::ImportFeature;
use eframe::egui;
use glam::DVec3;

pub struct ImportDialog {
    /// Which import is being edited (None: a new one, not in the design yet).
    pub editing: Option<usize>,
    pub feature: ImportFeature,
    /// Clicks in the view drop it there.
    pub picking: bool,
    /// The bodies' box as turned by `bounds_for` (before the move): what
    /// Centre, Ground and clicks place by. The app fills it.
    pub bounds: Option<(DVec3, DVec3)>,
    pub bounds_for: [f64; 3],
    /// The move arrows and squares: as last drawn (screen), the one under
    /// the mouse, and a drag under way.
    pub screen: Vec<(crate::gizmo::Handle, Vec<glam::Vec2>)>,
    pub hover: Option<crate::gizmo::Handle>,
    pub drag: Option<crate::gizmo::Drag>,
    /// Placing by clicking: the spot under the mouse, and the flat face it's
    /// on (body, plane) if any.
    pub spot: Option<DVec3>,
    pub spot_face: Option<(usize, crate::sketch::FacePlane)>,
    /// An OpenSCAD import: its parameters as shown (Customizer), a change
    /// waiting to be made (the app runs it), one under way, and what the
    /// script said.
    pub rows: Vec<ScadRow>,
    pub rerun: bool,
    pub busy: bool,
    pub messages: Vec<String>,
}

/// One OpenSCAD parameter in the dialog.
#[derive(Clone, Debug, PartialEq)]
pub struct ScadRow {
    pub name: String,
    pub text: String,
    /// As the script has it.
    pub default: String,
    /// The comment after it (what it's for).
    pub note: String,
    /// Choices listed in the comment ("assembly | shell_left | …") for a text value.
    pub options: Vec<String>,
    pub boolean: bool,
    /// Typed in and not run yet (runs when the box is left).
    pub edited: bool,
}

/// The parameters of `src`, with the values typed before.
pub fn scad_rows(src: &crate::model::ScadSource) -> Vec<ScadRow> {
    crate::scad::params(&src.text.0)
        .into_iter()
        .map(|p| {
            let default = match &p.value {
                crate::scad::Value::Str(s) => s.clone(),
                v => v.show(),
            };
            let text = src.values.iter().find(|(k, _)| *k == p.name).map_or_else(|| default.clone(), |(_, v)| v.clone());
            let options = match &p.value {
                crate::scad::Value::Str(_) if p.note.contains('|') => p.note.split('|').filter_map(|o| o.split_whitespace().next()).map(str::to_string).collect(),
                _ => Vec::new(),
            };
            ScadRow { boolean: matches!(p.value, crate::scad::Value::Bool(_)), name: p.name, text, default, note: p.note, options, edited: false }
        })
        .collect()
}

impl ImportDialog {
    pub fn new(feature: ImportFeature) -> Self {
        let rows = feature.scad.as_ref().map(scad_rows).unwrap_or_default();
        ImportDialog { editing: None, feature, picking: false, bounds: None, bounds_for: [f64::NAN; 3], screen: Vec::new(), hover: None, drag: None, spot: None, spot_face: None, rows, rerun: false, busy: false, messages: Vec::new() }
    }

    /// The parameter values that differ from the script's own.
    pub fn values(&self) -> Vec<(String, String)> {
        self.rows.iter().filter(|r| r.text.trim() != r.default).map(|r| (r.name.clone(), r.text.trim().to_string())).collect()
    }

    /// The OpenSCAD parameters (as its Customizer): a change is made when
    /// the box is left, a choice or tick at once.
    fn scad_panel(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong("Parameters");
            if self.busy {
                ui.spinner();
                ui.weak("Making it…");
            }
        });
        if self.rows.is_empty() {
            ui.weak("The script has no parameters (top-level name = value lines).");
        }
        let mut run = false;
        egui::ScrollArea::vertical().max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("scad_params").num_columns(2).striped(true).show(ui, |ui| {
                for r in &mut self.rows {
                    let label = ui.label(&r.name);
                    if !r.note.is_empty() {
                        label.on_hover_text(&r.note);
                    }
                    if r.boolean {
                        let mut on = r.text == "true";
                        if ui.checkbox(&mut on, "").changed() {
                            r.text = on.to_string();
                            run = true;
                        }
                    } else if !r.options.is_empty() {
                        let before = r.text.clone();
                        egui::ComboBox::from_id_salt(("scad_opt", &r.name)).selected_text(&r.text).show_ui(ui, |ui| {
                            for o in &r.options {
                                ui.selectable_value(&mut r.text, o.clone(), o);
                            }
                        });
                        if r.text != before {
                            run = true;
                        }
                    } else {
                        let resp = ui.add(egui::TextEdit::singleline(&mut r.text).desired_width(140.0));
                        if resp.changed() {
                            r.edited = true;
                        }
                        if resp.lost_focus() && r.edited {
                            r.edited = false;
                            run = true;
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if ui.add_enabled(self.rows.iter().any(|r| r.text.trim() != r.default), egui::Button::new("Reset to the script's values")).clicked() {
            for r in &mut self.rows {
                r.text = r.default.clone();
            }
            run = true;
        }
        for m in &self.messages {
            ui.weak(m);
        }
        if run {
            self.rerun = true;
        }
    }

    pub fn edit(k: usize, feature: &ImportFeature) -> Self {
        ImportDialog { editing: Some(k), ..Self::new(feature.clone()) }
    }

    /// Does `bounds` need working out again (new, or turned since)?
    pub fn bounds_stale(&self) -> bool {
        self.bounds.is_none() || self.bounds_for != self.feature.angles
    }

    /// Where the move arrows start: the middle of its box where it is now.
    pub fn gizmo_origin(&self) -> Option<DVec3> {
        self.bounds.map(|(lo, hi)| (lo + hi) * 0.5 + DVec3::from(self.feature.offset))
    }

    /// The middle of its box on the origin.
    pub fn centre(&mut self) {
        if let Some((lo, hi)) = self.bounds {
            self.feature.offset = (-(lo + hi) * 0.5).to_array();
        }
    }

    /// Its bottom on the XY plane (where it is across stays).
    pub fn ground(&mut self) {
        if let Some((lo, _)) = self.bounds {
            self.feature.offset[2] = -lo.z;
        }
    }

    /// The middle of its bottom at `p` (a clicked spot).
    pub fn place_at(&mut self, p: DVec3) {
        if let Some((lo, hi)) = self.bounds {
            let bottom = DVec3::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5, lo.z);
            self.feature.offset = (p - bottom).to_array();
        }
        self.picking = false;
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit) -> Action {
        ui.heading(if self.feature.scad.is_some() { "OpenSCAD" } else { "Insert" });
        ui.label(&self.feature.source);
        ui.horizontal(|ui| {
            if ui.selectable_label(self.picking, if self.picking { "Click where it goes…" } else { "Place by clicking" }).on_hover_text("Click a face of the model, or the ground, to put its bottom there").clicked() {
                self.picking = !self.picking;
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Centre").on_hover_text("Its middle on the origin").clicked() {
                self.centre();
            }
            if ui.button("Place on ground").on_hover_text("Its bottom on the XY plane").clicked() {
                self.ground();
            }
        });
        egui::Grid::new("import_place").num_columns(4).show(ui, |ui| {
            ui.label("Move");
            for v in &mut self.feature.offset {
                ui.add(crate::units::length_drag(v, unit, 0.5));
            }
            ui.end_row();
            ui.label("Turn");
            for a in &mut self.feature.angles {
                ui.add(egui::DragValue::new(a).speed(1.0).suffix("°"));
            }
            ui.end_row();
            ui.label("");
            for axis in ["X", "Y", "Z"] {
                ui.weak(axis);
            }
            ui.end_row();
        });
        if self.feature.scad.is_some() {
            self.scad_panel(ui);
        }
        let mut act = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.busy, egui::Button::new("OK")).clicked() {
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
    use crate::model::Blob;

    fn dialog() -> ImportDialog {
        let mut d = ImportDialog::new(ImportFeature { name: "part".into(), source: "part.step".into(), brep: Blob::new(String::new()), offset: [0.0; 3], angles: [0.0; 3], scad: None });
        // A part 10 × 20 × 30 sitting from (100, 0, -5).
        d.bounds = Some((DVec3::new(100.0, 0.0, -5.0), DVec3::new(110.0, 20.0, 25.0)));
        d.bounds_for = [0.0; 3];
        d
    }

    #[test]
    fn openscad_parameters_with_choices_from_their_comments() {
        let text = "part = \"assembly\";   // assembly | shell_left | back_right\nwall = 4; // side wall\nflip = false;\n";
        let src = crate::model::ScadSource { text: Blob::new(text.into()), values: vec![("wall".into(), "5".into())], ..Default::default() };
        let mut d = ImportDialog::new(ImportFeature { scad: Some(src), ..dialog().feature });
        assert_eq!(d.rows.len(), 3);
        assert_eq!(d.rows[0].options, vec!["assembly", "shell_left", "back_right"]);
        assert_eq!((d.rows[1].text.as_str(), d.rows[1].default.as_str(), d.rows[1].note.as_str()), ("5", "4", "side wall"));
        assert!(d.rows[2].boolean);
        assert_eq!(d.values(), vec![("wall".to_string(), "5".to_string())], "only what differs is kept");
        d.rows[0].text = "shell_left".into();
        assert_eq!(d.values().len(), 2);
    }

    #[test]
    fn centre_ground_and_click_place_it() {
        let mut d = dialog();
        assert!(!d.bounds_stale());
        d.centre();
        assert_eq!(d.feature.offset, [-105.0, -10.0, -10.0]);
        d.ground();
        assert_eq!(d.feature.offset, [-105.0, -10.0, 5.0], "bottom (−5) up to 0, across unchanged");
        d.picking = true;
        d.place_at(DVec3::new(0.0, 0.0, 12.0));
        assert_eq!(d.feature.offset, [-105.0, -10.0, 17.0], "bottom middle on the clicked spot");
        assert!(!d.picking);
        d.feature.angles[2] = 90.0;
        assert!(d.bounds_stale(), "turned: its box must be found again");
    }
}
