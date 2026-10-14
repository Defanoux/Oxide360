//! The Thread dialog (Fusion's): cylindrical faces to thread, cosmetic or
//! modeled, full length or an offset and length, and the thread from the
//! library (type, size, designation, class, direction). The size starts as
//! the one that fits the first face picked; Remember Size keeps the choice
//! for the next Thread.

use crate::copy_dialog::Action;
use crate::model::{CylinderRef, ThreadFeature, ThreadSpec};
use crate::threads::{self, Standard, Thread};
use eframe::egui;

#[derive(Clone, Debug, PartialEq)]
pub struct ThreadDialog {
    pub editing: Option<usize>,
    pub thread: ThreadFeature,
    /// The Faces field takes clicks in the view.
    pub picking: bool,
    /// Remember Size: the next Thread starts with this one's thread.
    pub remember: bool,
    /// The faces picked so far are holes (internal threads).
    pub internal: bool,
}

impl ThreadDialog {
    /// A new Thread; `last`: the remembered size, if any.
    pub fn new(name: String, last: Option<ThreadSpec>) -> Self {
        let remember = last.is_some();
        let spec = last.unwrap_or_else(|| ThreadSpec { class: "6g".into(), ..Default::default() });
        let thread = ThreadFeature { name, faces: Vec::new(), spec, full_length: true, offset: 0.0, length: 10.0 };
        ThreadDialog { editing: None, thread, picking: true, remember, internal: false }
    }

    pub fn from_feature(k: usize, t: &ThreadFeature, internal: bool) -> Self {
        ThreadDialog { editing: Some(k), thread: t.clone(), picking: false, remember: false, internal }
    }

    /// Click on a face: added (the first one sets the size to fit it, unless
    /// a size is remembered), or dropped if it's already picked.
    pub fn toggle(&mut self, face: CylinderRef, internal: bool) {
        let same = |c: &CylinderRef| c.axis.dot(face.axis).abs() > 1.0 - 1e-9 && (c.radius - face.radius).abs() < 1e-6 && {
            let d = c.axis_point - face.axis_point;
            (d - face.axis * d.dot(face.axis)).length() < 1e-6
        };
        if let Some(i) = self.thread.faces.iter().position(same) {
            self.thread.faces.remove(i);
            return;
        }
        if self.thread.faces.is_empty() {
            self.internal = internal;
            if !self.remember {
                self.fit(face.radius * 2.0);
            }
        }
        self.thread.faces.push(face);
    }

    /// The thread that fits a cylinder of this diameter.
    pub fn fit(&mut self, diameter: f64) {
        if let Some(t) = threads::nearest(self.thread.spec.standard, diameter, self.internal) {
            self.thread.spec.designation = t.designation.to_string();
            self.thread.spec.class = t.standard.classes(self.internal)[0].to_string();
        }
    }

    pub fn panel(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, error: Option<&str>) -> Action {
        ui.heading(if self.editing.is_some() { "Edit Thread" } else { "Thread" });
        let internal = self.internal;
        let mut standard = self.thread.spec.standard;
        let mut size = self.thread.spec.thread().map_or(String::new(), |t| t.size.to_string());
        egui::Grid::new("thread_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            ui.label("Faces");
            let n = self.thread.faces.len();
            let text = if self.picking {
                "Click cylindrical faces…".to_string()
            } else if n == 0 {
                "Click to select".to_string()
            } else {
                format!("{n} selected")
            };
            let rich = if self.picking { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
            if ui
                .add(egui::Button::selectable(self.picking, rich).min_size(egui::vec2(170.0, 0.0)))
                .on_hover_text("Click a shaft or the inside of a hole; click one again to drop it")
                .clicked()
            {
                self.picking = !self.picking;
            }
            ui.end_row();
            let t = &mut self.thread;
            ui.label("Modeled");
            ui.checkbox(&mut t.spec.modeled, "").on_hover_text("Cut the thread into the part (for 3D printing); off, it's only drawn");
            ui.end_row();
            ui.label("Full Length");
            ui.checkbox(&mut t.full_length, "");
            ui.end_row();
            if !t.full_length {
                ui.label("Offset");
                ui.add(crate::units::length_drag(&mut t.offset, unit, 0.1).range(0.0..=f64::MAX));
                ui.end_row();
                ui.label("Length");
                ui.add(crate::units::length_drag(&mut t.length, unit, 0.1).range(0.001..=f64::MAX));
                ui.end_row();
            }
            ui.label("Thread Type");
            egui::ComboBox::from_id_salt("thread_standard").selected_text(standard.name()).width(190.0).show_ui(ui, |ui| {
                // Straight threads only (pipe threads are tapered: a Taper Tapped hole).
                for s in [Standard::IsoMetric, Standard::Unified] {
                    ui.selectable_value(&mut standard, s, s.name());
                }
            });
            ui.end_row();
            ui.label("Size");
            egui::ComboBox::from_id_salt("thread_size").selected_text(size.as_str()).show_ui(ui, |ui| {
                for s in threads::sizes(t.spec.standard) {
                    ui.selectable_value(&mut size, s.to_string(), s);
                }
            });
            ui.end_row();
            ui.label("Designation");
            let current = t.spec.thread().map_or("", |th| th.size);
            egui::ComboBox::from_id_salt("thread_designation").selected_text(t.spec.designation.as_str()).show_ui(ui, |ui| {
                for d in threads::designations(t.spec.standard, current) {
                    ui.selectable_value(&mut t.spec.designation, d.to_string(), d);
                }
            });
            ui.end_row();
            ui.label("Class");
            egui::ComboBox::from_id_salt("thread_class").selected_text(t.spec.class.as_str()).show_ui(ui, |ui| {
                for c in t.spec.standard.classes(internal) {
                    ui.selectable_value(&mut t.spec.class, c.to_string(), *c);
                }
            });
            ui.end_row();
            ui.label("Direction");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut t.spec.right_hand, true, "Right hand");
                ui.selectable_value(&mut t.spec.right_hand, false, "Left hand");
            });
            ui.end_row();
            ui.label("Remember Size");
            ui.checkbox(&mut self.remember, "").on_hover_text("Start the next Thread with this thread");
            ui.end_row();
        });
        let spec = &mut self.thread.spec;
        if standard != spec.standard {
            spec.standard = standard;
            spec.class = standard.classes(internal)[0].to_string();
            match self.thread.faces.first() {
                Some(f) => {
                    let d = f.radius * 2.0;
                    self.fit(d);
                }
                None => spec.designation = Thread::of_size(standard, if standard == Standard::IsoMetric { "M6" } else { "1/4" }).map_or(String::new(), |t| t.designation.into()),
            }
        } else if spec.thread().is_some_and(|t| t.size != size) {
            if let Some(t) = Thread::of_size(spec.standard, &size) {
                spec.designation = t.designation.into();
            }
        }
        if let Some(e) = error {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        ui.separator();
        let mut action = Action::None;
        ui.horizontal(|ui| {
            if ui.add_enabled(error.is_none() && !self.thread.faces.is_empty(), egui::Button::new("OK")).clicked() {
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
    use glam::DVec3;

    fn shaft(r: f64) -> CylinderRef {
        CylinderRef { axis_point: DVec3::ZERO, axis: DVec3::Z, radius: r, at: DVec3::new(r, 0.0, 1.0) }
    }

    #[test]
    fn the_first_face_sets_the_size_and_a_second_click_drops_it() {
        let mut d = ThreadDialog::new("Thread1".into(), None);
        d.toggle(shaft(4.0), false);
        assert_eq!((d.thread.spec.designation.as_str(), d.thread.spec.class.as_str()), ("M8x1.25", "6g"));
        d.toggle(shaft(4.0), false);
        assert!(d.thread.faces.is_empty());
        // A hole Ø5 taps M6, internal class.
        let mut d = ThreadDialog::new("Thread1".into(), None);
        d.toggle(shaft(2.5), true);
        assert_eq!((d.thread.spec.designation.as_str(), d.thread.spec.class.as_str()), ("M6x1", "6H"));
        // A remembered size isn't replaced by the fit.
        let last = ThreadSpec { designation: "M3x0.5".into(), ..Default::default() };
        let mut d = ThreadDialog::new("Thread2".into(), Some(last));
        d.toggle(shaft(4.0), false);
        assert_eq!(d.thread.spec.designation, "M3x0.5");
    }

    #[test]
    fn the_span_runs_from_the_offset() {
        let mut t = ThreadDialog::new("T".into(), None).thread;
        assert_eq!(t.span(0.0, 20.0).unwrap(), (0.0, 20.0));
        (t.full_length, t.offset, t.length) = (false, 2.0, 5.0);
        assert_eq!(t.span(0.0, 20.0).unwrap(), (2.0, 7.0));
        t.length = 50.0;
        assert_eq!(t.span(0.0, 20.0).unwrap(), (2.0, 20.0), "no further than the face");
        t.offset = 25.0;
        assert!(t.span(0.0, 20.0).is_err());
    }
}
