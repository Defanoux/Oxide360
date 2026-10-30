//! The Print Check window (INSPECT): which printer, the overhang limit, and
//! what the check found — does it fit, does it need support, and the best
//! faces to lay it on (Lay Flat turns it onto one, as a Move in the timeline).

use crate::print3d::{Orientation, PRINTERS, Printer};
use eframe::egui;
use glam::DVec3;

/// What the check found for the bodies shown.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Findings {
    /// The bodies' size (X, Y, Z), when there are any.
    pub size: Option<DVec3>,
    /// How much too big for the printer, if it is.
    pub too_big: Option<DVec3>,
    /// Overhang area needing support as it stands (mm²).
    pub support: f64,
    /// The best ways to lay it, best first.
    pub best: Vec<Orientation>,
}

/// What a click in the window asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    None,
    Close,
    /// Lay it on suggestion k.
    LayFlat(usize),
}

/// The window. `printer` / `custom` / `limit` are the saved preferences.
pub fn panel(ui: &mut egui::Ui, printer: &mut String, custom: &mut [f64; 3], limit: &mut f64, found: &Findings) -> Action {
    let mut action = Action::None;
    ui.heading("Print Check");
    egui::Grid::new("print_check_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("Printer");
        egui::ComboBox::from_id_salt("printer").selected_text(printer.as_str()).width(190.0).show_ui(ui, |ui| {
            for p in PRINTERS {
                ui.selectable_value(printer, p.name.to_string(), p.name);
            }
            ui.selectable_value(printer, "Custom".to_string(), "Custom");
        });
        ui.end_row();
        if !PRINTERS.iter().any(|p| p.name == printer) {
            ui.label("Build volume");
            ui.horizontal(|ui| {
                for v in custom.iter_mut() {
                    ui.add(egui::DragValue::new(v).speed(1.0).range(10.0..=2000.0).suffix(" mm").update_while_editing(false));
                }
            });
            ui.end_row();
        }
        ui.label("Overhang limit");
        ui.add(egui::DragValue::new(limit).speed(0.5).range(10.0..=85.0).suffix("°").update_while_editing(false))
            .on_hover_text("How far a face may lean out from upright before it needs support (45° suits most printers)");
        ui.end_row();
    });
    ui.separator();
    let warn = egui::Color32::from_rgb(230, 120, 90);
    let good = egui::Color32::from_rgb(120, 200, 120);
    let p: Printer = crate::print3d::printer(printer, *custom);
    match found.size {
        None => {
            ui.label("No bodies shown.");
        }
        Some(size) => {
            let text = format!("{:.1} × {:.1} × {:.1} mm on a {:.0} × {:.0} × {:.0} bed", size.x, size.y, size.z, p.size[0], p.size[1], p.size[2]);
            match found.too_big {
                None => ui.colored_label(good, format!("Fits: {text}")),
                Some(over) => {
                    let axes: Vec<String> = [("X", over.x), ("Y", over.y), ("Z", over.z)].iter().filter(|(_, v)| *v > 0.0).map(|(a, v)| format!("{v:.1} mm in {a}")).collect();
                    ui.colored_label(warn, format!("Too big by {}: {text}", axes.join(", ")))
                }
            };
            if found.support > 0.5 {
                ui.colored_label(warn, format!("Needs support as it lies: {:.0} mm² of overhang (shaded)", found.support));
            } else {
                ui.colored_label(good, "Prints without support as it lies");
            }
            if !found.best.is_empty() {
                ui.add_space(4.0);
                ui.strong("Best faces to lay it on");
                egui::Grid::new("print_best").num_columns(3).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (k, o) in found.best.iter().enumerate() {
                        ui.label(format!("{}.", k + 1));
                        let support = if o.support > 0.5 { format!("{:.0} mm² support", o.support) } else { "no support".into() };
                        ui.label(format!("{support} · {:.0} mm² on the bed · {:.1} mm tall", o.contact, o.height));
                        if o.down.z < -0.9999 {
                            ui.weak("as it lies");
                        } else if ui.button("Lay Flat").on_hover_text("Turn it onto this face, on the bed and centred (a Move in the timeline)").clicked() {
                            action = Action::LayFlat(k);
                        }
                        ui.end_row();
                    }
                });
            }
        }
    }
    ui.separator();
    if ui.button("Close").clicked() {
        action = Action::Close;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let (mut printer, mut custom, mut limit) = ("Bambu Lab P1S".to_string(), [200.0; 3], 45.0);
        let found = Findings {
            size: Some(DVec3::new(60.0, 10.0, 50.0)),
            too_big: None,
            support: 340.0,
            best: vec![
                Orientation { down: DVec3::Y, contact: 1000.0, support: 0.0, height: 10.0 },
                Orientation { down: -DVec3::Z, contact: 600.0, support: 300.0, height: 50.0 },
            ],
        };
        crate::ui_shot::shot(&format!("{dir}/print_check.png"), [520, 330], |ui| {
            panel(ui, &mut printer, &mut custom, &mut limit, &found);
        });
    }
}
