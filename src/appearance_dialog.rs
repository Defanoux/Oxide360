//! The Appearance dialog (Fusion's, A): pick an appearance from the library
//! (or the ones already in the design), then click bodies — or, with Apply To:
//! Faces, single faces — to give it to them.
//! It stays open until Close, like Fusion's.

use crate::appearance::{self, Appearance, Category};
use eframe::egui::{self, Color32, Sense, Stroke};

/// What clicking a body does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice {
    Give(&'static Appearance),
    /// Take the appearance off (back to the body's own colour).
    Remove,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AppearanceDialog {
    pub current: Option<Choice>,
    pub search: String,
    /// Apply To: Faces (else whole bodies).
    pub faces: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    /// An appearance (or Remove) was picked in the list: give it to the
    /// bodies selected beforehand, if any.
    Picked(Choice),
    Close,
}

/// The library entries matching what's typed (any part of the name or the
/// category, ignoring case), by category.
pub fn matching(search: &str) -> Vec<(Category, Vec<&'static Appearance>)> {
    let t = search.trim().to_lowercase();
    Category::ALL
        .into_iter()
        .map(|c| {
            let hit = |a: &&Appearance| t.is_empty() || a.name.to_lowercase().contains(&t) || c.name().to_lowercase().contains(&t);
            (c, appearance::in_category(c).filter(hit).collect::<Vec<_>>())
        })
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// A little ball in the appearance's colour: a highlight as glossy as it is,
/// a checker behind glass.
pub fn swatch(ui: &mut egui::Ui, a: &Appearance, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    let r = size * 0.45;
    if a.opacity < 1.0 {
        let s = size / 4.0;
        for i in 0..4 {
            for j in 0..4 {
                let g = if (i + j) % 2 == 0 { 200 } else { 120 };
                let cell = egui::Rect::from_min_size(rect.min + egui::vec2(i as f32 * s, j as f32 * s), egui::vec2(s, s));
                p.rect_filled(cell.intersect(egui::Rect::from_center_size(c, egui::vec2(r * 1.5, r * 1.5))), 0.0, Color32::from_gray(g));
            }
        }
    }
    let [cr, cg, cb] = a.color;
    let alpha = (a.opacity * 255.0) as u8;
    p.circle_filled(c, r, Color32::from_rgba_unmultiplied(cr, cg, cb, alpha));
    // Shading: darker toward the lower right, brighter for metals at the top.
    let shade = Color32::from_rgba_unmultiplied(0, 0, 0, 70);
    p.circle_filled(c + egui::vec2(r * 0.35, r * 0.35), r * 0.6, shade);
    let spot = 0.12 + 0.3 * (1.0 - a.gloss);
    let shine = (60.0 + 180.0 * a.gloss.max(a.metal * 0.6)) as u8;
    p.circle_filled(c + egui::vec2(-r * 0.35, -r * 0.4), r * spot, Color32::from_rgba_unmultiplied(255, 255, 255, shine));
    p.circle_stroke(c, r, Stroke::new(1.0, Color32::from_gray(30)));
    resp
}

impl AppearanceDialog {
    /// What to do next, in words.
    pub fn hint(&self, bodies_picked: usize) -> String {
        let what = if self.faces { "faces" } else { "bodies" };
        match (self.current, bodies_picked) {
            (None, n) if n > 0 && !self.faces => format!("Pick an appearance for the {n} selected {}.", if n == 1 { "body" } else { "bodies" }),
            (None, _) => format!("Pick an appearance, then click {what} in the view."),
            (Some(Choice::Give(a)), _) => format!("Click {what} to make them {}.", a.name),
            (Some(Choice::Remove), _) => format!("Click {what} to take their appearance off."),
        }
    }

    /// `in_design`: the appearances bodies have now; `bodies_picked`: how
    /// many bodies were selected when it opened (a pick goes to them).
    pub fn panel(&mut self, ui: &mut egui::Ui, in_design: &[&'static Appearance], bodies_picked: usize) -> Action {
        let mut action = Action::None;
        ui.heading("Appearance");
        ui.horizontal(|ui| {
            ui.label("Apply To");
            ui.selectable_value(&mut self.faces, false, "Bodies").on_hover_text("A click gives the whole body the appearance");
            ui.selectable_value(&mut self.faces, true, "Faces").on_hover_text("A click gives one face its own appearance, over its body's");
        });
        ui.label(self.hint(bodies_picked));
        ui.separator();
        let mut row = |ui: &mut egui::Ui, choice: Choice, current: &mut Option<Choice>| {
            let on = *current == Some(choice);
            ui.horizontal(|ui| {
                match choice {
                    Choice::Give(a) => {
                        swatch(ui, a, 20.0);
                    }
                    Choice::Remove => {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
                        ui.painter().circle_stroke(rect.center(), 9.0, Stroke::new(1.0, Color32::from_gray(150)));
                        ui.painter().line_segment([rect.center() + egui::vec2(-6.0, 6.0), rect.center() + egui::vec2(6.0, -6.0)], Stroke::new(1.0, Color32::from_gray(150)));
                    }
                }
                let text = match choice {
                    Choice::Give(a) => a.name,
                    Choice::Remove => "None (the body's own colour)",
                };
                if ui.add(egui::Button::selectable(on, text)).clicked() {
                    *current = Some(choice);
                    action = Action::Picked(choice);
                }
            });
        };
        ui.label(egui::RichText::new("In This Design").strong());
        row(ui, Choice::Remove, &mut self.current);
        for &a in in_design {
            row(ui, Choice::Give(a), &mut self.current);
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Library").strong());
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search").desired_width(150.0));
        });
        let found = matching(&self.search);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            if found.is_empty() {
                ui.weak("Nothing matches");
            }
            for (cat, list) in &found {
                let open = !self.search.trim().is_empty();
                egui::CollapsingHeader::new(cat.name()).id_salt(("appearance_cat", cat.name())).default_open(false).open(open.then_some(true)).show(ui, |ui| {
                    for &a in list {
                        row(ui, Choice::Give(a), &mut self.current);
                    }
                });
            }
        });
        ui.separator();
        if ui.button("Close").clicked() {
            action = Action::Close;
        }
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_finds_by_name_or_category() {
        let all = matching("");
        assert_eq!(all.len(), Category::ALL.len());
        let brass = matching("BRASS");
        assert_eq!(brass.len(), 1);
        assert_eq!(brass[0].1[0].name, "Brass - Polished");
        let glass = matching("glass");
        assert!(glass.iter().any(|(c, v)| *c == Category::Glass && v.len() == appearance::in_category(Category::Glass).count()), "a category name finds all of it");
        assert!(matching("zzz").is_empty());
    }

    #[test]
    fn the_hint_says_bodies_or_faces() {
        let mut d = AppearanceDialog { current: appearance::by_name("Oak").map(Choice::Give), ..Default::default() };
        assert_eq!(d.hint(0), "Click bodies to make them Oak.");
        d.faces = true;
        assert_eq!(d.hint(3), "Click faces to make them Oak.");
        d.current = None;
        assert_eq!(d.hint(3), "Pick an appearance, then click faces in the view.", "selected bodies only take a pick in Bodies");
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = AppearanceDialog { current: appearance::by_name("Oak").map(Choice::Give), search: String::new(), faces: true };
        let used = [appearance::by_name("Oak").unwrap(), appearance::by_name("Glass - Clear").unwrap()];
        crate::ui_shot::shot(&format!("{dir}/appearance.png"), [340, 420], |ui| {
            d.panel(ui, &used, 0);
        });
        let mut d = AppearanceDialog { current: None, search: "polished".into(), faces: false };
        crate::ui_shot::shot(&format!("{dir}/appearance_search.png"), [340, 420], |ui| {
            d.panel(ui, &[], 2);
        });
    }
}
