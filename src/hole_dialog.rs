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
        use crate::icons::{Icon, option};
        ui.heading(if self.editing.is_some() { "Edit Hole" } else { "Hole" });
        let mut new_tap = None;
        egui::Grid::new("hole_grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            let h = &mut self.hole;
            ui.label("Position");
            ui.horizontal(|ui| {
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
                if !h.places.is_empty() && ui.small_button("×").on_hover_text("Clear the places").clicked() {
                    h.places.clear();
                    self.picking = true;
                }
            });
            ui.end_row();

            ui.label("Extents");
            ui.horizontal(|ui| {
                for (e, icon, name, tip) in [
                    (HoleExtent::Distance, Icon::ExtentDistance, "Distance", "Distance: as deep as set"),
                    (HoleExtent::To, Icon::ExtentTo, "To", "To: up to a plane or face"),
                    (HoleExtent::All, Icon::ExtentAll, "All", "All: right through"),
                ] {
                    if option(ui, icon, name, tip, true, h.extent == e).clicked() {
                        h.extent = e;
                        self.picking_target = e == HoleExtent::To && h.target.is_none();
                        if self.picking_target {
                            self.picking = false;
                        }
                    }
                }
                ui.add_space(12.0);
                if option(ui, Icon::Flip, "Flip Direction", "Flip Direction: drill the other way", true, h.flip).clicked() {
                    h.flip = !h.flip;
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
                for (k, icon, name) in [(HoleKind::Simple, Icon::HoleSimple, "Simple"), (HoleKind::Counterbore, Icon::HoleCounterbore, "Counterbore"), (HoleKind::Countersink, Icon::HoleCountersink, "Countersink")] {
                    // Fusion: taper tapped holes are simple or countersunk only.
                    let allowed = !(k == HoleKind::Counterbore && h.tap == TapType::TaperTapped);
                    if option(ui, icon, name, name, allowed, h.kind == k).clicked() {
                        h.kind = k;
                    }
                }
            });
            ui.end_row();

            ui.label("Hole Tap Type");
            ui.horizontal(|ui| {
                for (t, icon, name, tip) in [
                    (TapType::Simple, Icon::TapSimple, "Simple tap", "Simple: the diameter you set"),
                    (TapType::Clearance, Icon::TapClearance, "Clearance", "Clearance: room for a screw of a size"),
                    (TapType::Tapped, Icon::TapTapped, "Tapped", "Tapped: drilled and threaded for a screw"),
                    (TapType::TaperTapped, Icon::TapTaper, "Taper Tapped", "Taper Tapped: a tapered pipe thread (NPT)"),
                ] {
                    if option(ui, icon, name, tip, true, h.tap == t).clicked() && h.tap != t {
                        new_tap = Some(t);
                    }
                }
            });
            ui.end_row();

            if matches!(h.tap, TapType::Tapped | TapType::TaperTapped) {
                ui.label("Thread Offset");
                ui.horizontal(|ui| {
                    if option(ui, Icon::ThreadFull, "Full", "Threaded the whole depth", true, h.thread_full).clicked() {
                        h.thread_full = true;
                    }
                    if option(ui, Icon::ThreadPartial, "Thread length", "Threaded only so far in", true, !h.thread_full).clicked() {
                        h.thread_full = false;
                    }
                });
                ui.end_row();
            }

            ui.label("Drill Point");
            ui.horizontal(|ui| {
                if option(ui, Icon::DrillFlat, "Flat", "Flat bottom", true, h.tip_angle.is_none()).clicked() {
                    h.tip_angle = None;
                }
                if option(ui, Icon::DrillAngle, "Angle", "Drill point (angle)", true, h.tip_angle.is_some()).clicked() && h.tip_angle.is_none() {
                    h.tip_angle = Some(118.0);
                }
            });
            ui.end_row();
        });
        if let Some(t) = new_tap {
            self.set_tap(t);
        }
        ui.add_space(4.0);
        self.diagram(ui, unit, circles);
        if self.hole.tap != TapType::Simple {
            ui.add_space(4.0);
            self.thread_rows(ui);
        }
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

    /// Fusion's picture of the hole: its section drawn to suit the settings,
    /// each size's box beside the part it sets (depth, diameter, tip,
    /// counterbore, countersink, thread length).
    fn diagram(&mut self, ui: &mut egui::Ui, unit: crate::units::Unit, circles: usize) {
        use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};
        let (rect, _) = ui.allocate_exact_size(vec2(300.0, 222.0), egui::Sense::hover());
        let h = &mut self.hole;
        let painter = ui.painter_at(rect);
        let (l, t) = (rect.left(), rect.top());
        let cx = l + 80.0;
        let top = t + 16.0;
        let bottom = t + 140.0;
        let angled = h.tip_angle.is_some();
        let tip = if angled { bottom + 22.0 } else { bottom };
        let taper = if h.tap == TapType::TaperTapped { 5.0 } else { 0.0 };
        let (rb, rc, rs) = (15.0, 32.0, 34.0); // bore, counterbore, countersink half widths
        let step = top + 42.0; // where a counterbore or countersink meets the bore
        let wall = |y: f32| rb + taper * (1.0 - (y - top) / (bottom - top)) * 2.0 - taper;
        let part = Color32::from_gray(150);
        let edge = Stroke::new(1.5, Color32::from_gray(210));
        let dim = Stroke::new(1.0, Color32::from_rgb(225, 85, 85));
        let leader = Stroke::new(1.0, Color32::from_gray(110));
        // The part round the hole.
        let (x0, x1) = (l + 14.0, l + 146.0);
        painter.line_segment([pos2(x0, top), pos2(x0, tip + 18.0)], Stroke::new(1.0, part));
        painter.line_segment([pos2(x1, top), pos2(x1, tip + 18.0)], Stroke::new(1.0, part));
        // The hole's outline (left side, mirrored).
        let mut side: Vec<Pos2> = vec![pos2(x0, top)];
        match h.kind {
            HoleKind::Simple => side.push(pos2(cx - wall(top), top)),
            HoleKind::Counterbore => side.extend([pos2(cx - rc, top), pos2(cx - rc, step), pos2(cx - wall(step), step)]),
            HoleKind::Countersink => side.extend([pos2(cx - rs, top), pos2(cx - wall(step - 18.0), step - 18.0)]),
        }
        side.push(pos2(cx - wall(bottom), bottom));
        side.push(pos2(cx, tip));
        for w in side.windows(2) {
            painter.line_segment([w[0], w[1]], edge);
            painter.line_segment([pos2(2.0 * cx - w[0].x, w[0].y), pos2(2.0 * cx - w[1].x, w[1].y)], edge);
        }
        if !angled {
            painter.line_segment([pos2(cx - wall(bottom), bottom), pos2(cx + wall(bottom), bottom)], edge);
        }
        // Thread: ticks on the walls as far as it goes.
        let threaded = matches!(h.tap, TapType::Tapped | TapType::TaperTapped);
        let from = if h.kind == HoleKind::Simple { top + 4.0 } else { step + 4.0 };
        let thread_end = if h.thread_full || h.depth <= 0.0 { bottom - 4.0 } else { (from + (bottom - from) * (h.thread_length / h.depth).clamp(0.05, 1.0) as f32).min(bottom - 4.0) };
        if threaded {
            let mut y = from;
            while y <= thread_end {
                let x = wall(y);
                let tick = Stroke::new(1.0, Color32::from_rgb(220, 120, 60));
                painter.line_segment([pos2(cx - x, y), pos2(cx - x + 6.0, y + 3.0)], tick);
                painter.line_segment([pos2(cx + x, y), pos2(cx + x - 6.0, y + 3.0)], tick);
                y += 7.0;
            }
        }
        // Boxes in a column on the right, each with a leader from its part.
        let bx = l + 182.0;
        let place = |y: f32| Rect::from_min_size(pos2(bx, y - 10.0), vec2(112.0, 20.0));
        let lead = |from: Pos2, y: f32| {
            painter.line_segment([from, pos2(bx - 4.0, y)], leader);
        };
        let tip_text = |ui: &mut egui::Ui, r: Rect, text: &str| {
            ui.put(r, egui::Label::new(egui::RichText::new(text).weak()));
        };
        match h.kind {
            HoleKind::Counterbore => {
                painter.line_segment([pos2(cx - rc, top - 6.0), pos2(cx + rc, top - 6.0)], dim);
                lead(pos2(cx + rc, top - 6.0), top - 4.0);
                ui.put(place(top - 4.0), crate::units::length_drag(&mut h.cbore_diameter, unit, 0.05).range(0.001..=f64::MAX)).on_hover_text("Counterbore diameter");
                painter.line_segment([pos2(cx + rc + 6.0, top), pos2(cx + rc + 6.0, step)], dim);
                lead(pos2(cx + rc + 6.0, step), step);
                ui.put(place(step), crate::units::length_drag(&mut h.cbore_depth, unit, 0.05).range(0.001..=f64::MAX)).on_hover_text("Counterbore depth");
            }
            HoleKind::Countersink => {
                painter.line_segment([pos2(cx - rs, top - 6.0), pos2(cx + rs, top - 6.0)], dim);
                lead(pos2(cx + rs, top - 6.0), top - 4.0);
                ui.put(place(top - 4.0), crate::units::length_drag(&mut h.csink_diameter, unit, 0.05).range(0.001..=f64::MAX)).on_hover_text("Countersink diameter");
                lead(pos2(cx + (rs + rb) / 2.0, step - 26.0), step);
                ui.put(place(step), egui::DragValue::new(&mut h.csink_angle).speed(1.0).range(1.0..=179.0).suffix("°")).on_hover_text("Countersink angle");
            }
            HoleKind::Simple => {}
        }
        // Depth: a dimension down the right of the hole, clear of the head's.
        let mid = (top + bottom) / 2.0 + 6.0;
        let dx = cx + rs + 14.0;
        painter.line_segment([pos2(dx, top), pos2(dx, bottom)], dim);
        for y in [top, bottom] {
            painter.line_segment([pos2(dx - 4.0, y), pos2(dx + 4.0, y)], dim);
        }
        lead(pos2(dx, mid), mid);
        match h.extent {
            HoleExtent::Distance => {
                ui.put(place(mid), crate::units::length_drag(&mut h.depth, unit, 0.2).range(0.001..=f64::MAX)).on_hover_text("Depth (to the end of the full diameter)");
            }
            HoleExtent::To => tip_text(ui, place(mid), "up to the object"),
            HoleExtent::All => tip_text(ui, place(mid), "right through"),
        }
        if threaded && !h.thread_full {
            lead(pos2(cx + wall(thread_end) - 4.0, thread_end), mid + 26.0);
            ui.put(place(mid + 26.0), crate::units::length_drag(&mut h.thread_length, unit, 0.2).range(0.001..=f64::MAX)).on_hover_text("Thread length");
        }
        if let Some(a) = h.tip_angle.as_mut() {
            lead(pos2(cx + 6.0, tip - 6.0), tip);
            ui.put(place(tip), egui::DragValue::new(a).speed(1.0).range(1.0..=179.0).suffix("°")).on_hover_text("Tip angle");
        }
        // Diameter: across the bottom.
        let dy = tip + 14.0;
        painter.line_segment([pos2(cx - wall(bottom), dy), pos2(cx + wall(bottom), dy)], dim);
        let dy = dy.max(if angled { tip + 26.0 } else { 0.0 }) - 6.0;
        lead(pos2(cx + wall(bottom), tip + 14.0), dy + 6.0);
        match h.tap {
            TapType::Simple => {
                ui.put(place(dy + 6.0), crate::units::length_drag(&mut h.diameter, unit, 0.05).range(0.001..=f64::MAX))
                    .on_hover_text(if circles > 0 { "Diameter (holes on circles take the circle's)" } else { "Diameter" });
            }
            _ => {
                let d = h.bore(None).map_or("—".into(), |d| format!("{} {}", unit.plain(d), unit.name()));
                ui.put(place(dy + 6.0), egui::Label::new(egui::RichText::new(d).weak()))
                    .on_hover_text(if h.tap == TapType::Clearance { "Clearance for the screw size" } else { "Tap drill (minor diameter)" });
            }
        }
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
                // Fusion models Tapped threads only; taper tapped ones are shown.
                if tap == crate::model::TapType::Tapped {
                    ui.label("Modeled");
                    ui.checkbox(&mut t.modeled, "").on_hover_text("Cut the thread into the part (for 3D printing); off, it's only drawn");
                    ui.end_row();
                }
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

    /// Run the panel for a frame (with `events`), returning where each labelled widget is.
    fn frame(ctx: &egui::Context, d: &mut HoleDialog, events: Vec<egui::Event>) -> Vec<(String, egui::Rect)> {
        let input = egui::RawInput { events, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 900.0))), ..Default::default() };
        let out = ctx.run_ui(input, |ui| {
            d.panel(ui, crate::units::Unit::Mm, None, &[], 0);
        });
        let mut out = out;
        out.textures_delta.clear();
        let tree = out.platform_output.accesskit_update.take().expect("accesskit on");
        tree.nodes
            .iter()
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                Some((n.label().or(n.value())?.to_string(), egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))))
            })
            .collect()
    }

    fn click(ctx: &egui::Context, d: &mut HoleDialog, label: &str) {
        let widgets = frame(ctx, d, vec![]);
        let r = widgets.iter().find(|(l, _)| l == label).unwrap_or_else(|| panic!("no {label} in {widgets:?}")).1;
        let at = r.center();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(ctx, d, vec![egui::Event::PointerMoved(at)]);
        frame(ctx, d, vec![press(true)]);
        frame(ctx, d, vec![press(false)]);
    }

    #[test]
    fn clicking_the_tap_type_shows_the_thread_choices() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut d = HoleDialog::new("Hole1".into());
        click(&ctx, &mut d, "Clearance");
        assert_eq!(d.hole.tap, TapType::Clearance);
        let shown: Vec<String> = frame(&ctx, &mut d, vec![]).into_iter().map(|(l, _)| l).collect();
        assert!(shown.iter().any(|l| l == "Fit") && shown.iter().any(|l| l.contains("M6")), "{shown:?}");
        click(&ctx, &mut d, "Counterbore");
        assert_eq!((d.hole.kind, d.hole.cbore_diameter), (HoleKind::Counterbore, 11.0));
    }

    /// Pictures of the dialog in a few setups (look at them in the scratch dir).
    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let setups: [(&str, fn(&mut HoleDialog)); 4] = [
            ("simple", |_| {}),
            ("cbore_clearance", |d| {
                d.hole.kind = HoleKind::Counterbore;
                d.set_tap(TapType::Clearance);
            }),
            ("csink_tapped_partial", |d| {
                d.hole.kind = HoleKind::Countersink;
                d.set_tap(TapType::Tapped);
                d.hole.thread_full = false;
            }),
            ("taper_flat", |d| {
                d.set_tap(TapType::TaperTapped);
                d.hole.tip_angle = None;
            }),
        ];
        for (name, setup) in setups {
            let mut d = HoleDialog::new("Hole1".into());
            d.hole.places.push(HolePlace::SketchPoint { sketch: 0, point: 1 });
            d.picking = false;
            setup(&mut d);
            crate::ui_shot::shot(&format!("{dir}/hole_{name}.png"), [380, 760], |ui| {
                d.panel(ui, crate::units::Unit::Mm, None, &[], 0);
            });
        }
    }

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
