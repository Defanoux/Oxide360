//! The Hole dialog (Fusion's): where the holes go, how far, what kind
//! (simple / counterbore / countersink), the tap type (simple, clearance,
//! tapped, taper tapped) with its thread, the drill point and the sizes.

use crate::copy_dialog::Action;
use crate::model::{ExtrudeTarget, HoleExtent, HoleFeature, HoleKind, HolePlace, TapType};
use crate::threads::{self, Fit, Standard, Thread};
use eframe::egui;

#[derive(Clone, Debug, PartialEq)]
pub struct HoleDialog {
    pub editing: Option<usize>,
    pub hole: HoleFeature,
    /// The Position field takes clicks in the view.
    pub picking: bool,
    /// The Extents "To" field takes a click on a plane or face.
    pub picking_target: bool,
}

impl HoleDialog {
    /// A new hole, waiting for its places.
    pub fn new(name: String) -> Self {
        HoleDialog { editing: None, hole: HoleFeature { name, ..Default::default() }, picking: true, picking_target: false }
    }

    pub fn from_feature(k: usize, h: &HoleFeature) -> Self {
        HoleDialog { editing: Some(k), hole: h.clone(), picking: false, picking_target: false }
    }

    /// Click on a place: added, or taken away if it's already there.
    pub fn toggle(&mut self, place: HolePlace) {
        match self.hole.places.iter().position(|p| same_place(p, &place)) {
            Some(i) => {
                self.hole.places.remove(i);
            }
            None => self.hole.places.push(place),
        }
    }

    /// Choose a tap type: the thread list fits it (pipe threads only for
    /// taper tapped, which can't have a counterbore).
    pub fn set_tap(&mut self, tap: TapType) {
        self.hole.tap = tap;
        let want_npt = tap == TapType::TaperTapped;
        if want_npt != (self.hole.thread.standard == Standard::Npt) {
            self.set_standard(if want_npt { Standard::Npt } else { Standard::IsoMetric });
        }
        if want_npt && self.hole.kind == HoleKind::Counterbore {
            self.hole.kind = HoleKind::Simple;
        }
        self.fit_heads();
    }

    pub fn set_standard(&mut self, standard: Standard) {
        let t = &mut self.hole.thread;
        t.standard = standard;
        // A middling size to start with (M6, 1/4, 1/4 NPT).
        let size = match standard {
            Standard::IsoMetric => "M6",
            Standard::Unified | Standard::Npt => "1/4",
        };
        t.designation = Thread::of_size(standard, size).map_or(String::new(), |th| th.designation.to_string());
        t.class = standard.classes(true)[0].to_string();
        self.hole.csink_angle = standard.countersink_angle();
        self.fit_heads();
    }

    pub fn set_size(&mut self, size: &str) {
        if let Some(th) = Thread::of_size(self.hole.thread.standard, size) {
            self.hole.thread.designation = th.designation.to_string();
            self.fit_heads();
        }
    }

    /// The counterbore / countersink follow the screw size (clearance or tapped).
    fn fit_heads(&mut self) {
        if self.hole.tap == TapType::Simple {
            return;
        }
        if let Some(th) = self.hole.thread.thread() {
            (self.hole.cbore_diameter, self.hole.cbore_depth) = th.counterbore();
            self.hole.csink_diameter = th.countersink();
        }
    }

    pub fn size(&self) -> &'static str {
        self.hole.thread.thread().map_or("", |t| t.size)
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>, plane_names: &[String], circles: usize) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Hole" } else { "Hole" });
        let h = &mut self.hole;
        egui::Grid::new("hole_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Position");
            let text = if self.picking {
                "Click points, circles or faces…".to_string()
            } else if h.places.is_empty() {
                "Click to select".to_string()
            } else {
                format!("{} selected", h.places.len())
            };
            let rich = if self.picking { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
            if ui
                .add(egui::Button::selectable(self.picking, rich).min_size(egui::vec2(170.0, 0.0)))
                .on_hover_text("Click sketch points or circles, or a flat face (its centre snaps). Click one again to drop it.")
                .clicked()
            {
                self.picking = !self.picking;
                self.picking_target = false;
            }
            ui.end_row();

            ui.label("Extents");
            ui.horizontal(|ui| {
                for (e, name) in [(HoleExtent::Distance, "Distance"), (HoleExtent::To, "To"), (HoleExtent::All, "All")] {
                    if ui.selectable_label(h.extent == e, name).clicked() {
                        h.extent = e;
                        self.picking_target = e == HoleExtent::To && h.target.is_none();
                        if self.picking_target {
                            self.picking = false;
                        }
                    }
                }
            });
            ui.end_row();
            if h.extent == HoleExtent::To {
                ui.label("Object");
                let name = match (self.picking_target, h.target) {
                    (true, _) => "Click a plane or flat face…".to_string(),
                    (_, Some(ExtrudeTarget::Origin(op))) => format!("{} plane", op.name()),
                    (_, Some(ExtrudeTarget::Plane(k))) => plane_names.get(k).cloned().unwrap_or_else(|| "a plane".into()),
                    (_, Some(ExtrudeTarget::Face { .. })) => "a body face".into(),
                    (_, None) => "Click to select".into(),
                };
                if ui.add(egui::Button::selectable(self.picking_target, name).min_size(egui::vec2(170.0, 0.0))).clicked() {
                    self.picking_target = !self.picking_target;
                    if self.picking_target {
                        self.picking = false;
                    }
                }
                ui.end_row();
            }

            ui.label("Hole Type");
            ui.horizontal(|ui| {
                for (k, name) in [(HoleKind::Simple, "Simple"), (HoleKind::Counterbore, "Counterbore"), (HoleKind::Countersink, "Countersink")] {
                    // Fusion: taper tapped holes are simple or countersunk only.
                    let allowed = !(k == HoleKind::Counterbore && h.tap == TapType::TaperTapped);
                    if ui.add_enabled(allowed, egui::Button::selectable(h.kind == k, name)).clicked() {
                        h.kind = k;
                    }
                }
            });
            ui.end_row();
        });
        // The tap type changes the thread list, so it's chosen outside the borrow above.
        let mut new_tap = None;
        ui.horizontal(|ui| {
            ui.label("Tap Type");
            for (t, name) in [(TapType::Simple, "Simple"), (TapType::Clearance, "Clearance"), (TapType::Tapped, "Tapped"), (TapType::TaperTapped, "Taper Tapped")] {
                if ui.selectable_label(self.hole.tap == t, name).clicked() && self.hole.tap != t {
                    new_tap = Some(t);
                }
            }
        });
        if let Some(t) = new_tap {
            self.set_tap(t);
        }
        if self.hole.tap != TapType::Simple {
            self.thread_rows(ui);
        }

        let h = &mut self.hole;
        egui::Grid::new("hole_sizes").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Drill Point");
            ui.horizontal(|ui| {
                if ui.selectable_label(h.tip_angle.is_none(), "Flat").clicked() {
                    h.tip_angle = None;
                }
                if ui.selectable_label(h.tip_angle.is_some(), "Angle").clicked() && h.tip_angle.is_none() {
                    h.tip_angle = Some(118.0);
                }
            });
            ui.end_row();
            if h.extent == HoleExtent::Distance {
                ui.label("Depth");
                ui.add(crate::units::length_drag(&mut h.depth, unit, 0.2).range(0.001..=f64::MAX));
                ui.end_row();
            }
            match h.tap {
                TapType::Simple => {
                    ui.label("Diameter");
                    ui.add(crate::units::length_drag(&mut h.diameter, unit, 0.05).range(0.001..=f64::MAX));
                    ui.end_row();
                    if circles > 0 {
                        ui.label("");
                        ui.weak("Holes on circles take the circle's size");
                        ui.end_row();
                    }
                }
                _ => {
                    ui.label("Diameter");
                    let d = h.bore(None).map_or("—".into(), |d| format!("{} {}", unit.plain(d), unit.name()));
                    ui.label(d).on_hover_text(if h.tap == TapType::Clearance { "Clearance for the screw size" } else { "Tap drill (minor diameter); the thread is cut beyond it" });
                    ui.end_row();
                }
            }
            if let Some(a) = h.tip_angle.as_mut() {
                ui.label("Tip Angle");
                ui.add(egui::DragValue::new(a).speed(1.0).range(1.0..=179.0).suffix("°"));
                ui.end_row();
            }
            match h.kind {
                HoleKind::Simple => {}
                HoleKind::Counterbore => {
                    ui.label("Counterbore Diameter");
                    ui.add(crate::units::length_drag(&mut h.cbore_diameter, unit, 0.05).range(0.001..=f64::MAX));
                    ui.end_row();
                    ui.label("Counterbore Depth");
                    ui.add(crate::units::length_drag(&mut h.cbore_depth, unit, 0.05).range(0.001..=f64::MAX));
                    ui.end_row();
                }
                HoleKind::Countersink => {
                    ui.label("Countersink Diameter");
                    ui.add(crate::units::length_drag(&mut h.csink_diameter, unit, 0.05).range(0.001..=f64::MAX));
                    ui.end_row();
                    ui.label("Countersink Angle");
                    ui.add(egui::DragValue::new(&mut h.csink_angle).speed(1.0).range(1.0..=179.0).suffix("°"));
                    ui.end_row();
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("Flip Direction").clicked() {
                self.hole.flip = !self.hole.flip;
            }
        });
        ui.weak("Drag the blue arrow to set the depth.");
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        let mut action = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(error.is_none() && !self.hole.places.is_empty(), egui::Button::new("OK")).clicked() {
                action = Action::Ok;
            }
            if ui.button("Cancel").clicked() {
                action = Action::Cancel;
            }
        });
        action
    }

    /// Thread Type, Size, Designation, Class / Fit, Direction, Modeled.
    fn thread_rows(&mut self, ui: &mut egui::Ui) {
        let tap = self.hole.tap;
        let mut standard = self.hole.thread.standard;
        let mut size = self.size().to_string();
        egui::Grid::new("hole_thread").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Thread Type");
            let choices: Vec<Standard> = Standard::ALL.into_iter().filter(|s| (*s == Standard::Npt) == (tap == TapType::TaperTapped)).collect();
            egui::ComboBox::from_id_salt("hole_standard").selected_text(standard.name()).width(190.0).show_ui(ui, |ui| {
                for s in choices {
                    ui.selectable_value(&mut standard, s, s.name());
                }
            });
            ui.end_row();
            ui.label("Size");
            egui::ComboBox::from_id_salt("hole_size").selected_text(size.as_str()).show_ui(ui, |ui| {
                for s in threads::sizes(self.hole.thread.standard) {
                    ui.selectable_value(&mut size, s.to_string(), s);
                }
            });
            ui.end_row();
            let t = &mut self.hole.thread;
            if tap != TapType::Clearance {
                ui.label("Designation");
                let current_size = t.thread().map_or("", |th| th.size);
                egui::ComboBox::from_id_salt("hole_designation").selected_text(t.designation.as_str()).show_ui(ui, |ui| {
                    for d in threads::designations(t.standard, current_size) {
                        ui.selectable_value(&mut t.designation, d.to_string(), d);
                    }
                });
                ui.end_row();
                ui.label("Class");
                egui::ComboBox::from_id_salt("hole_class").selected_text(t.class.as_str()).show_ui(ui, |ui| {
                    for c in t.standard.classes(true) {
                        ui.selectable_value(&mut t.class, c.to_string(), *c);
                    }
                });
                ui.end_row();
                ui.label("Direction");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut t.right_hand, true, "Right hand");
                    ui.selectable_value(&mut t.right_hand, false, "Left hand");
                });
                ui.end_row();
                ui.label("Modeled");
                ui.checkbox(&mut t.modeled, "").on_hover_text("Cut the thread into the part (for 3D printing); off, it's only drawn");
                ui.end_row();
            } else {
                ui.label("Fit");
                ui.horizontal(|ui| {
                    for f in Fit::ALL {
                        ui.selectable_value(&mut t.fit, f, f.name());
                    }
                });
                ui.end_row();
            }
        });
        if standard != self.hole.thread.standard {
            self.set_standard(standard);
        } else if size != self.size() {
            self.set_size(&size);
        }
    }
}

/// The same place (a face place counts as the same when picked at the same spot).
fn same_place(a: &HolePlace, b: &HolePlace) -> bool {
    match (a, b) {
        (HolePlace::Face { face: f, anchor: x, centre: c }, HolePlace::Face { face: g, anchor: y, centre: d }) => {
            c == d && f.normal().dot(g.normal()) > 0.9999 && (f.origin - g.origin).length() < 1e-6 && ((x[0] - y[0]).hypot(x[1] - y[1]) < 1e-6 || *c)
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_toggle_and_the_tap_type_picks_its_threads() {
        let mut d = HoleDialog::new("Hole1".into());
        assert!(d.picking, "a new hole waits for its places");
        d.toggle(HolePlace::SketchPoint { sketch: 0, point: 3 });
        d.toggle(HolePlace::SketchCircle { sketch: 0, entity: 5 });
        d.toggle(HolePlace::SketchPoint { sketch: 0, point: 3 });
        assert_eq!(d.hole.places, vec![HolePlace::SketchCircle { sketch: 0, entity: 5 }]);
        // Clearance M6: the counterbore fits a socket head.
        d.hole.kind = HoleKind::Counterbore;
        d.set_tap(TapType::Clearance);
        assert_eq!((d.hole.cbore_diameter, d.hole.cbore_depth), (11.0, 6.4));
        d.set_size("M8");
        assert_eq!(d.hole.thread.designation, "M8x1.25");
        assert_eq!(d.hole.bore(None).unwrap(), 9.0);
        // Taper tapped: pipe threads, and no counterbore.
        d.set_tap(TapType::TaperTapped);
        assert_eq!(d.hole.thread.standard, Standard::Npt);
        assert_eq!(d.hole.kind, HoleKind::Simple);
        // Back to tapped: an ordinary thread again.
        d.set_tap(TapType::Tapped);
        assert_eq!(d.hole.thread.standard, Standard::IsoMetric);
        d.set_standard(Standard::Unified);
        assert_eq!((d.hole.thread.designation.as_str(), d.hole.thread.class.as_str(), d.hole.csink_angle), ("1/4-20 UNC", "2B", 82.0));
    }
}
