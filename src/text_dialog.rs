//! The Text dialog (Fusion's): what a sketch text says, its height, font,
//! angle, bold and italic. The text in the sketch changes as you type; OK
//! keeps it, Cancel puts the sketch back as it was.

use crate::copy_dialog::Action;
use crate::sketch::{EntityId, Sketch, SketchText};

pub struct TextDialog {
    /// The text entity being edited.
    pub entity: EntityId,
    /// The sketch before the dialog opened (Cancel restores it).
    pub before: Sketch,
    /// True when the text was just placed (Cancel removes it).
    pub new: bool,
    pub text: SketchText,
    /// Typed into the font list to narrow it.
    font_filter: String,
    /// Put the cursor in the text box when it opens.
    focus: bool,
}

impl TextDialog {
    pub fn new(sk: &Sketch, entity: EntityId, new: bool) -> Option<Self> {
        let crate::sketch::Entity::Text { t, .. } = *sk.entities.get(entity)? else { return None };
        Some(TextDialog { entity, before: sk.clone(), new, text: sk.texts.get(t)?.clone(), font_filter: String::new(), focus: true })
    }

    /// Can OK be pressed? (Text with nothing to show is refused, as in Fusion.)
    pub fn problem(&self) -> Option<&'static str> {
        if self.text.text.trim().is_empty() {
            Some("Type some text")
        } else if !(self.text.height.is_finite() && self.text.height > 0.0) {
            Some("Height must be more than zero")
        } else {
            None
        }
    }

    /// Fonts whose names contain what's typed in the filter (any case).
    pub fn matching_fonts<'a>(fonts: &'a [String], filter: &str) -> Vec<&'a String> {
        let f = filter.to_lowercase();
        fonts.iter().filter(|name| name.to_lowercase().contains(&f)).collect()
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit) -> Action {
        ui.heading("Text");
        ui.label("Text");
        let edit = ui.add(egui::TextEdit::multiline(&mut self.text.text).desired_rows(2).desired_width(220.0));
        if std::mem::take(&mut self.focus) {
            edit.request_focus();
            // Selected, so typing replaces the placeholder.
            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), edit.id) {
                let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(self.text.text.chars().count()));
                state.cursor.set_char_range(Some(all));
                state.store(ui.ctx(), edit.id);
            }
        }
        egui::Grid::new("text_dialog").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Height");
            ui.add(crate::units::length_drag(&mut self.text.height, unit, 0.1).range(0.01..=f64::MAX));
            ui.end_row();
            ui.label("Angle");
            ui.add(egui::DragValue::new(&mut self.text.angle).speed(1.0).range(-360.0..=360.0).suffix("°"));
            ui.end_row();
            ui.label("Font");
            egui::ComboBox::from_id_salt("text_font").selected_text(self.text.font.as_str()).width(170.0).height(320.0).show_ui(ui, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.font_filter).hint_text("Search fonts").desired_width(160.0));
                for name in Self::matching_fonts(crate::text::families(), &self.font_filter) {
                    if ui.selectable_label(*name == self.text.font, name.as_str()).clicked() {
                        self.text.font = name.clone();
                    }
                }
            });
            ui.end_row();
            ui.label("Style");
            ui.horizontal(|ui| {
                ui.toggle_value(&mut self.text.bold, egui::RichText::new("B").strong()).on_hover_text("Bold");
                ui.toggle_value(&mut self.text.italic, egui::RichText::new("I").italics()).on_hover_text("Italic");
            });
            ui.end_row();
        });
        ui.weak("Drag its point to move it. Its letters are profiles to extrude.");
        if let Some(p) = self.problem() {
            ui.colored_label(egui::Color32::from_rgb(230, 80, 80), p);
        }
        let mut act = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(self.problem().is_none(), egui::Button::new("OK")).clicked() {
                act = Action::Ok;
            }
            if ui.button("Cancel").clicked() {
                act = Action::Cancel;
            }
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            act = Action::Cancel;
        }
        act
    }

    /// The sketch with the dialog's values applied to the text.
    pub fn apply(&self, sk: &mut Sketch) {
        if let Some(st) = sk.text_mut(self.entity) {
            *st = self.text.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, OriginPlane};
    use glam::DVec2;

    fn placed() -> (Sketch, EntityId) {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let st = SketchText { text: "Text".into(), height: 5.0, font: crate::text::UBUNTU.into(), bold: false, italic: false, angle: 0.0 };
        let e = sk.add_text(Anchor::Free(DVec2::new(1.0, 2.0)), st);
        (sk, e)
    }

    #[test]
    fn edits_apply_to_the_text_and_empty_text_is_refused() {
        let (mut sk, e) = placed();
        let mut d = TextDialog::new(&sk, e, true).unwrap();
        d.text.text = "Hello".into();
        d.text.height = 8.0;
        d.apply(&mut sk);
        assert_eq!(sk.texts[0].text, "Hello");
        assert_eq!(sk.texts[0].height, 8.0);
        assert!(d.problem().is_none());
        d.text.text = "  ".into();
        assert!(d.problem().is_some());
        // Cancel's snapshot is the sketch as it was.
        assert_eq!(d.before.texts[0].text, "Text");
    }

    #[test]
    fn the_font_list_narrows_as_you_type() {
        let fonts: Vec<String> = ["DejaVu Sans", "Liberation Sans", "Liberation Serif", "Hack"].map(String::from).to_vec();
        assert_eq!(TextDialog::matching_fonts(&fonts, "lib").len(), 2);
        assert_eq!(TextDialog::matching_fonts(&fonts, "SANS").len(), 2);
        assert_eq!(TextDialog::matching_fonts(&fonts, "").len(), 4);
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let (sk, e) = placed();
        let mut d = TextDialog::new(&sk, e, true).unwrap();
        crate::ui_shot::shot(&format!("{dir}/text_dialog.png"), [280, 300], |ui| {
            d.panel(ui, crate::units::Unit::Mm);
        });
    }
}
