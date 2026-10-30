//! The Sketch Palette (Fusion's, at the view's right while a sketch is open):
//! what the sketch shows, grid snap, Slice, Look At and Finish Sketch.

use eframe::egui;

/// What the open sketch shows (all on, Slice off, as Fusion starts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub grid: bool,
    pub slice: bool,
    pub profile: bool,
    pub points: bool,
    pub dimensions: bool,
    pub constraints: bool,
    pub projected: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { grid: true, slice: false, profile: true, points: true, dimensions: true, constraints: true, projected: true }
    }
}

/// A button in the palette that the app acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Construction,
    LookAt,
    Finish,
}

/// The palette's rows; `construction`: construction mode is on.
pub fn panel(ui: &mut egui::Ui, o: &mut Options, snap: &mut bool, construction: bool) -> Action {
    let mut action = Action::None;
    egui::CollapsingHeader::new("Options").default_open(true).show(ui, |ui| {
        egui::Grid::new("sketch_palette").num_columns(2).spacing([24.0, 4.0]).show(ui, |ui| {
            ui.label("Construction");
            let mut on = construction;
            if ui.checkbox(&mut on, "").on_hover_text("New geometry is construction (X); with lines selected, switches them").clicked() {
                action = Action::Construction;
            }
            ui.end_row();
            ui.label("Look At");
            if ui.small_button("Look").on_hover_text("Look straight at the sketch plane").clicked() {
                action = Action::LookAt;
            }
            ui.end_row();
            let rows: [(&str, &mut bool, &str); 8] = [
                ("Sketch Grid", &mut o.grid, "Show the grid"),
                ("Snap", snap, "Snap to the grid (G)"),
                ("Slice", &mut o.slice, "Cut away the bodies in front of the sketch plane, to see and pick what's on it"),
                ("Show Profile", &mut o.profile, "Shade closed profiles"),
                ("Show Points", &mut o.points, "Show the points"),
                ("Show Dimensions", &mut o.dimensions, "Show the dimensions"),
                ("Show Constraints", &mut o.constraints, "Show the constraint glyphs"),
                ("Show Projected Geometries", &mut o.projected, "Show lines projected from bodies"),
            ];
            for (label, value, tip) in rows {
                ui.label(label);
                ui.checkbox(value, "").on_hover_text(tip);
                ui.end_row();
            }
            ui.label("3D Sketch");
            let mut off = false;
            ui.add_enabled(false, egui::Checkbox::new(&mut off, "")).on_disabled_hover_text("3D sketches aren't in Oxide360 yet");
            ui.end_row();
        });
    });
    ui.separator();
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Finish Sketch").clicked() {
                action = Action::Finish;
            }
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_like_fusion() {
        let o = Options::default();
        assert!(o.grid && o.profile && o.points && o.dimensions && o.constraints && o.projected);
        assert!(!o.slice);
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut o = Options::default();
        let mut snap = true;
        crate::ui_shot::shot(&format!("{dir}/sketch_palette.png"), [300, 360], |ui| {
            panel(ui, &mut o, &mut snap, false);
        });
    }
}
