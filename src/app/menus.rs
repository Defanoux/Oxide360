//! The right-click marking menu and the S key's Design Shortcuts box (as
//! Fusion's): tools round the cursor, what's under it in a list below, and
//! every tool found by typing.

use crate::*;

/// What a slot round the marking menu does.
#[derive(Clone, Copy, PartialEq)]
enum Mark {
    Repeat,
    Undo,
    Redo,
    Delete,
    Tool(&'static str),
    FinishSketch,
}

/// A button with an icon before its words (`enabled` false: greyed, no clicks).
fn icon_button(ui: &mut egui::Ui, icon: Option<Icon>, text: &str, enabled: bool, at: Option<egui::Rect>) -> egui::Response {
    let font = egui::FontId::proportional(14.0);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, Color32::WHITE);
    let size = egui::vec2(galley.size().x + if icon.is_some() { 34.0 } else { 16.0 }, 28.0);
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, r) = match at {
        Some(a) => {
            let rect = egui::Rect::from_center_size(a.center(), size);
            (rect, ui.interact(rect, ui.id().with(("icon_button", text)), sense))
        }
        None => ui.allocate_exact_size(egui::vec2(size.x.max(ui.available_width().min(260.0)), size.y - 2.0), sense),
    };
    let fill = if r.hovered() && enabled { Color32::from_gray(70) } else if at.is_some() { Color32::from_gray(45) } else { Color32::TRANSPARENT };
    ui.painter().rect_filled(rect, 6.0, fill);
    let mut x = rect.left() + 8.0;
    if let Some(i) = icon {
        icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(x, rect.center().y - 10.0), egui::vec2(20.0, 20.0)), i, enabled);
        x += 26.0;
    }
    let ink = if enabled { Color32::from_gray(225) } else { Color32::from_gray(110) };
    ui.painter().galley_with_override_text_color(egui::pos2(x, rect.center().y - galley.size().y / 2.0), galley, ink);
    r
}

/// The icon a tool's toolbar button has (by its name), for the menus.
pub(crate) fn tool_icon(name: &str, sketching: bool) -> Option<Icon> {
    Some(match (name, sketching) {
        ("Create Sketch", _) => Icon::Sketch,
        ("Finish Sketch", _) => Icon::FinishSketch,
        ("Extrude", _) => Icon::Extrude,
        ("Revolve", _) => Icon::Revolve,
        ("Sweep", _) => Icon::Sweep,
        ("Loft", _) => Icon::Loft,
        ("Pipe", _) => Icon::Pipe,
        ("Coil", _) => Icon::Coil,
        ("Hole", _) => Icon::Hole,
        ("Thread", _) => Icon::Thread,
        ("Offset Plane", _) => Icon::Plane,
        ("Plane at Angle", _) => Icon::PlaneAngle,
        ("Midplane", _) => Icon::Midplane,
        ("Plane Through Three Points", _) => Icon::Plane3Points,
        ("Tangent Plane", _) => Icon::TangentPlane,
        ("Axis Along Edge", _) => Icon::Axis,
        ("Axis Through Cylinder", _) => Icon::AxisCylinder,
        ("Axis Through Two Points", _) => Icon::Axis2Points,
        ("Point", false) => Icon::ConstructPoint,
        ("Point", true) => Icon::Point,
        ("Mirror", _) => Icon::Mirror,
        ("Rectangular Pattern", _) => Icon::RectPattern,
        ("Circular Pattern", _) => Icon::CircPattern,
        ("Press Pull", _) => Icon::PressPull,
        ("Fillet", _) => Icon::Fillet,
        ("Chamfer", _) => Icon::Chamfer,
        ("Shell", _) => Icon::Shell,
        ("Move/Copy", _) => Icon::Move,
        ("Combine", _) => Icon::Combine,
        ("Split Body", _) => Icon::Split,
        ("Insert SVG / DXF", _) => Icon::Import,
        ("Scale", _) => Icon::Scale,
        ("Appearance", _) => Icon::Appearance,
        ("Change Parameters", _) => Icon::Parameters,
        ("Measure", _) => Icon::Measure,
        ("Undo", _) => Icon::Undo,
        ("Redo", _) => Icon::Redo,
        ("Delete", _) => Icon::Remove,
        ("Look At", _) => Icon::Select,
        ("Line", _) => Icon::Line,
        ("Rectangle", _) => Icon::Rectangle,
        ("Circle", _) => Icon::Circle,
        ("Arc", _) => Icon::Arc3Point,
        ("Polygon", _) => Icon::Polygon,
        ("Slot", _) => Icon::Slot,
        ("Spline", _) => Icon::Spline,
        ("Ellipse", _) => Icon::Ellipse,
        ("Text", _) => Icon::Text,
        ("Trim", _) => Icon::Trim,
        ("Break", _) => Icon::Break,
        ("Extend", _) => Icon::Extend,
        ("Offset", _) => Icon::Offset,
        ("Project", _) => Icon::Project,
        ("Construction", _) => Icon::Construction,
        ("Dimension", _) => Icon::Dimension,
        ("Coincident", _) => Icon::Coincident,
        ("Collinear", _) => Icon::Collinear,
        ("Concentric", _) => Icon::Concentric,
        ("Horizontal", _) => Icon::Horizontal,
        ("Vertical", _) => Icon::Vertical,
        ("Parallel", _) => Icon::Parallel,
        ("Perpendicular", _) => Icon::Perpendicular,
        ("Equal", _) => Icon::Equal,
        ("Tangent", _) => Icon::Tangent,
        ("Midpoint", _) => Icon::Midpoint,
        ("Symmetric", _) => Icon::Symmetric,
        ("Fix", _) => Icon::Fix,
        _ => return None,
    })
}

impl OpenCad {
    /// Start tool `name` of the mode we're in (and remember it for Repeat and the S box).
    pub(crate) fn run_tool(&mut self, name: &'static str) {
        shortcuts::used(&mut self.recent_tools, name);
        if self.active.is_some() {
            self.run_sketch_tool(name);
        } else {
            self.run_solid_tool(name);
        }
    }

    /// The right-click menu's list: what the click was on (a sketch's
    /// selection, a plane, a face).
    fn context_rows(&mut self, ui: &mut egui::Ui) {
        if let Some(i) = self.active {
            // (The sketch's axes can be picked for constraints but not deleted.)
            let has_sel = self.editor.selection.iter().any(|s| !matches!(s, Sel::Axis(_))) || !self.dim_selection.is_empty();
            if self.editor.tool == Tool::Select && has_sel {
                if ui.button("Delete").clicked() {
                    self.delete_selection();
                    ui.close();
                    return;
                }
            } else {
                if ui.button("Look at Sketch").on_hover_text("Look straight at the sketch plane").clicked() {
                    self.look_at(self.sketches[i].view_angles(), None);
                    ui.close();
                }
                if ui.button("Finish Sketch").clicked() {
                    self.finish_sketch();
                    ui.close();
                }
                return;
            }
            if self.editor.selection.iter().any(|s| matches!(s, Sel::Entity(_))) && ui.button("Construction").clicked() {
                self.toggle_construction();
                ui.close();
            }
            if let [ci] = self.dim_selection[..].iter().copied().filter(|&ci| self.sketches[i].constraints.get(ci).is_some_and(|c| c.value().is_some())).collect::<Vec<_>>()[..] {
                let driven = self.sketches.get(i).is_some_and(|sk| sk.is_driven(ci));
                if ui.button(if driven { "Make driving" } else { "Make reference (driven)" }).clicked() {
                    self.toggle_driven(ci);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Look at Sketch").on_hover_text("Look straight at the sketch plane").clicked() {
                self.look_at(self.sketches[i].view_angles(), None);
                ui.close();
            }
            if ui.button("Finish Sketch").clicked() {
                self.finish_sketch();
                ui.close();
            }
        } else if let Some(op) = self.ctx_plane {
            if ui.button(format!("Create Sketch on the {} plane", op.name())).clicked() {
                self.start_sketch(op);
                ui.close();
            }
            if ui.button("Look At").clicked() {
                self.look_at(op.view_angles(), None);
                ui.close();
            }
        } else if let Some(k) = self.ctx_cplane.filter(|&k| k < self.planes.len()) {
            if ui.button(format!("Create Sketch on {}", self.planes[k].name)).clicked() {
                self.start_sketch_on_plane(k);
                ui.close();
            }
            let built = self.model(Slot::Committed).planes.get(k).copied().flatten();
            if let Some(p) = built {
                if ui.button("Look At").clicked() {
                    self.look_at(camera::angles_for_dir(p.normal().as_vec3()), Some(p.origin.as_vec3()));
                    ui.close();
                }
            }
        } else if let Some((b, face)) = self.ctx_face {
            if ui.button("Create Sketch on this face").clicked() {
                self.start_sketch_on_face(face);
                ui.close();
            }
            if ui.button("Look At").on_hover_text("Look straight at this face").clicked() {
                // Centred on the face (its extent is in plane coordinates: p·u, p·v).
                let centre = self.model(Slot::Committed).bodies.get(b).map(|body| {
                    let [x0, y0, x1, y1] = model::face_extent(&body.mesh, &face);
                    let n = face.normal();
                    face.u * ((x0 + x1) / 2.0) + face.v * ((y0 + y1) / 2.0) + n * face.origin.dot(n)
                });
                self.look_at(camera::angles_for_dir(face.normal().as_vec3()), centre.or(Some(face.origin)).map(|c| c.as_vec3()));
                ui.close();
            }
        } else {
            ui.close();
        }
    }

    /// Fusion's marking menu: eight tools round where the right-click was,
    /// and a list under them. A click anywhere (or Esc) closes it.
    pub(crate) fn marking_menu(&mut self, ctx: &egui::Context) {
        let Some(at) = self.marking else { return };
        let sketching = self.active.is_some();
        // North, then clockwise.
        let slots: [Mark; 8] = if sketching {
            [Mark::Repeat, Mark::Redo, Mark::Tool("Dimension"), Mark::Tool("Circle"), Mark::FinishSketch, Mark::Tool("Line"), Mark::Delete, Mark::Undo]
        } else {
            [Mark::Repeat, Mark::Redo, Mark::Tool("Press Pull"), Mark::Tool("Hole"), Mark::Tool("Create Sketch"), Mark::Tool("Move/Copy"), Mark::Delete, Mark::Undo]
        };
        let can_delete = if sketching { !self.editor.selection.is_empty() || !self.dim_selection.is_empty() } else { !self.selected_bodies.is_empty() };
        let label = |m: Mark| -> String {
            match m {
                Mark::Repeat => self.last_tool.map_or_else(|| "Repeat".to_string(), |t| format!("Repeat {t}")),
                Mark::Undo => "Undo".into(),
                Mark::Redo => "Redo".into(),
                Mark::Delete => "Delete".into(),
                Mark::Tool(t) => t.into(),
                Mark::FinishSketch => "Finish Sketch".into(),
            }
        };
        let enabled = |m: Mark| match m {
            Mark::Repeat => self.last_tool.is_some(),
            Mark::Undo => self.can_undo(),
            Mark::Redo => self.history.depth().1 > 0,
            Mark::Delete => can_delete,
            _ => true,
        };
        let icon_of = |m: Mark| match m {
            Mark::Repeat => self.last_tool.and_then(|t| tool_icon(t, sketching)),
            Mark::Undo => Some(Icon::Undo),
            Mark::Redo => Some(Icon::Redo),
            Mark::Delete => Some(Icon::Remove),
            Mark::Tool(t) => tool_icon(t, sketching),
            Mark::FinishSketch => Some(Icon::FinishSketch),
        };
        let labels: Vec<(String, bool, Option<Icon>)> = slots.iter().map(|&m| (label(m), enabled(m), icon_of(m))).collect();
        let mut chosen: Option<Mark> = None;
        let radius = egui::vec2(150.0, 95.0);
        egui::Area::new(egui::Id::new("marking_menu")).order(egui::Order::Foreground).fixed_pos(at - radius - egui::vec2(80.0, 20.0)).show(ctx, |ui| {
            let (area, _) = ui.allocate_exact_size(radius * 2.0 + egui::vec2(160.0, 40.0), egui::Sense::hover());
            let centre = area.center();
            ui.painter().circle_stroke(centre, 9.0, Stroke::new(2.0, Color32::from_gray(200)));
            for (k, (text, on, icon)) in labels.iter().enumerate() {
                let a = std::f32::consts::TAU * k as f32 / 8.0;
                let spot = centre + egui::vec2(a.sin() * radius.x, -a.cos() * radius.y);
                let r = icon_button(ui, *icon, text, *on, Some(egui::Rect::from_center_size(spot, egui::Vec2::ZERO)));
                if r.clicked() && *on {
                    chosen = Some(slots[k]);
                }
            }
        });
        // The list under the bottom slot.
        let list_at = at + egui::vec2(-100.0, radius.y + 40.0);
        egui::Area::new(egui::Id::new("marking_list")).order(egui::Order::Foreground).fixed_pos(list_at).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(200.0);
                let before = ui.cursor().min.y;
                self.context_rows(ui);
                if ui.cursor().min.y > before + 1.0 {
                    ui.separator();
                }
                let quick: &[&'static str] = if sketching { &["Trim", "Offset", "Project", "Mirror", "Construction"] } else { &["Extrude", "Fillet", "Shell", "Offset Plane", "Combine"] };
                for &t in quick {
                    if icon_button(ui, tool_icon(t, sketching), t, true, None).clicked() {
                        chosen = Some(Mark::Tool(t));
                    }
                }
                if !sketching && icon_button(ui, Some(Icon::Measure), "Measure", true, None).clicked() {
                    self.measure = Some(measure::Measure::default());
                }
                ui.separator();
                if ui.button("Shortcuts…   S").on_hover_text("Find any tool by typing").clicked() {
                    self.shortcut_box = Some((at, String::new()));
                }
            });
        });
        match chosen {
            Some(Mark::Repeat) => {
                if let Some(t) = self.last_tool {
                    self.run_tool(t);
                }
            }
            Some(Mark::Undo) => self.undo(),
            Some(Mark::Redo) => self.redo(),
            Some(Mark::Delete) => {
                if sketching {
                    self.delete_selection();
                } else {
                    let bodies = self.selected_bodies.clone();
                    self.remove_bodies(bodies);
                }
            }
            Some(Mark::Tool(t)) => self.run_tool(t),
            Some(Mark::FinishSketch) => {
                self.finish_sketch();
            }
            None => {}
        }
        // Any click (it acted already) or Esc closes it.
        // (A right-click elsewhere moves it there instead: see the viewport.)
        if ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary) || i.key_pressed(egui::Key::Escape)) {
            self.marking = None;
        }
    }

    /// The S key's Design Shortcuts box: the tools used lately, and a search
    /// over every tool of this mode. Enter starts the first match.
    pub(crate) fn shortcut_box(&mut self, ctx: &egui::Context) {
        let Some((at, mut typed)) = self.shortcut_box.clone() else { return };
        let sketching = self.active.is_some();
        let tools = if sketching { shortcuts::SKETCH } else { shortcuts::SOLID };
        let mut chosen: Option<&'static str> = None;
        let mut close = false;
        let recent: Vec<&'static str> = self.recent_tools.iter().copied().filter(|t| tools.contains(t)).collect();
        let r = egui::Area::new(egui::Id::new("shortcut_box")).order(egui::Order::Foreground).fixed_pos(at + egui::vec2(-110.0, -20.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(220.0);
                ui.strong("Design Shortcuts");
                // The tools used lately, as icons (as Fusion's pinned row).
                if !recent.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for t in &recent {
                            if let Some(i) = tool_icon(t, sketching) {
                                if icons::button(ui, i, t, true, false).clicked() {
                                    chosen = Some(t);
                                }
                            }
                        }
                    });
                }
                let edit = ui.add(egui::TextEdit::singleline(&mut typed).hint_text("Search").desired_width(f32::INFINITY));
                edit.request_focus();
                let hits = shortcuts::search(tools, &typed);
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (k, t) in hits.iter().enumerate() {
                        let first = k == 0 && !typed.is_empty();
                        if first {
                            let r = ui.cursor();
                            ui.painter().rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(ui.available_width(), 28.0)), 6.0, Color32::from_rgb(30, 80, 120));
                        }
                        if icon_button(ui, tool_icon(t, sketching), t, true, None).clicked() {
                            chosen = Some(t);
                        }
                    }
                });
                let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                if enter {
                    chosen = hits.first().copied();
                }
                close = esc;
            });
        });
        // A click outside closes it.
        if ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary)) && !r.response.contains_pointer() && chosen.is_none() {
            close = true;
        }
        if let Some(t) = chosen {
            self.shortcut_box = None;
            self.run_tool(t);
        } else if close {
            self.shortcut_box = None;
        } else {
            self.shortcut_box = Some((at, typed));
        }
    }
}
