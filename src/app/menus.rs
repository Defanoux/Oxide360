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
        let labels: Vec<(String, bool)> = slots.iter().map(|&m| (label(m), enabled(m))).collect();
        let mut chosen: Option<Mark> = None;
        let radius = egui::vec2(150.0, 95.0);
        egui::Area::new(egui::Id::new("marking_menu")).order(egui::Order::Foreground).fixed_pos(at - radius - egui::vec2(80.0, 20.0)).show(ctx, |ui| {
            let (area, _) = ui.allocate_exact_size(radius * 2.0 + egui::vec2(160.0, 40.0), egui::Sense::hover());
            let centre = area.center();
            ui.painter().circle_stroke(centre, 9.0, Stroke::new(2.0, Color32::from_gray(200)));
            for (k, (text, on)) in labels.iter().enumerate() {
                let a = std::f32::consts::TAU * k as f32 / 8.0;
                let spot = centre + egui::vec2(a.sin() * radius.x, -a.cos() * radius.y);
                let size = egui::vec2(text.len() as f32 * 7.5 + 24.0, 28.0);
                let r = ui.put(egui::Rect::from_center_size(spot, size), egui::Button::new(text.as_str()).fill(Color32::from_gray(45)).corner_radius(6.0).sense(if *on { egui::Sense::click() } else { egui::Sense::hover() }));
                if !on {
                    ui.painter().rect_filled(r.rect, 6.0, Color32::from_black_alpha(110));
                }
                if r.clicked() && *on {
                    chosen = Some(slots[k]);
                }
            }
        });
        // The list under the bottom slot.
        let list_at = at + egui::vec2(-90.0, radius.y + 24.0);
        egui::Area::new(egui::Id::new("marking_list")).order(egui::Order::Foreground).fixed_pos(list_at).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(180.0);
                self.context_rows(ui);
                ui.separator();
                let quick: &[&'static str] = if sketching { &["Trim", "Offset", "Project", "Mirror", "Construction"] } else { &["Extrude", "Fillet", "Shell", "Offset Plane", "Combine"] };
                for &t in quick {
                    if ui.button(t).clicked() {
                        chosen = Some(Mark::Tool(t));
                    }
                }
                if !sketching && ui.button("Measure").clicked() {
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
        let tools = if self.active.is_some() { shortcuts::SKETCH } else { shortcuts::SOLID };
        let mut chosen: Option<&'static str> = None;
        let mut close = false;
        let recent: Vec<&'static str> = self.recent_tools.iter().copied().filter(|t| tools.contains(t)).collect();
        let r = egui::Area::new(egui::Id::new("shortcut_box")).order(egui::Order::Foreground).fixed_pos(at + egui::vec2(-110.0, -20.0)).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(220.0);
                ui.strong("Design Shortcuts");
                if !recent.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for t in &recent {
                            if ui.small_button(*t).clicked() {
                                chosen = Some(t);
                            }
                        }
                    });
                }
                let edit = ui.add(egui::TextEdit::singleline(&mut typed).hint_text("Search").desired_width(f32::INFINITY));
                edit.request_focus();
                let hits = shortcuts::search(tools, &typed);
                egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                    for (k, t) in hits.iter().enumerate() {
                        let r = ui.selectable_label(k == 0 && !typed.is_empty(), *t);
                        if r.clicked() {
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
