//! The timeline bar, renaming, and the Measure tool.

use crate::*;

impl OpenCad {
    /// Measure tool: what's under the cursor (corner, then edge, then face),
    /// clicks pick it, Esc leaves.
    pub(crate) fn measure_input(&mut self, view: &View, hover_px: Option<Vec2>, response: &egui::Response, aspect: f32, min: Vec2, size: Vec2) {
        if self.measure.is_none() {
            return;
        }
        let hover = hover_px.and_then(|px| {
            let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
            self.measure_pick_at(view, px, &ray)
        });
        let Some(m) = self.measure.as_mut() else { return };
        m.hover = hover;
        if response.clicked() {
            if let Some(p) = m.hover.clone() {
                m.click(p);
            }
        }
        if response.ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.measure = None;
        }
    }

    /// The corner, edge or face of a visible body under the cursor, in that
    /// order of preference (like Fusion: corners and edges win near them).
    fn measure_pick_at(&self, view: &View, px: Vec2, ray: &Ray) -> Option<measure::Picked> {
        use measure::{Geom, Pick, Picked, Samples};
        let model = &self.caches[Slot::Committed as usize].as_ref()?.model;
        let visible = |b: &&kernel::Body| !self.hidden_bodies.contains(&b.id);
        let hit = model
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| visible(b))
            .filter_map(|(i, b)| model::ray_hit_tri(&b.mesh, ray.origin, ray.dir).map(|(t, tri)| (t, tri, i)))
            .min_by(|x, y| x.0.total_cmp(&y.0));
        // Things behind a body can't be picked.
        let slack = self.camera.distance * 0.01;
        let in_front = |p: Vec3| hit.is_none_or(|(h, ..)| (p - ray.origin).dot(ray.dir) <= h + slack);
        let mut corner: Option<(f32, Vec3)> = None;
        let mut edge: Option<(f32, usize, usize)> = None;
        for (bi, body) in model.bodies.iter().enumerate().filter(|(_, b)| visible(b)) {
            for (ei, e) in body.edges.iter().enumerate() {
                let (Some(&first), Some(&last)) = (e.points.first(), e.points.last()) else { continue };
                // A closed edge (a whole circle) has no real corner.
                if first.distance(last) > 1e-6 {
                    for p in [first, last] {
                        let Some(q) = view.project(p) else { continue };
                        let d = px.distance(q);
                        if d <= SNAP_PX && corner.is_none_or(|(bd, _)| d < bd) && in_front(p) {
                            corner = Some((d, p));
                        }
                    }
                }
                for w in e.points.windows(2) {
                    let (Some(a), Some(b)) = (view.project(w[0]), view.project(w[1])) else { continue };
                    let ab = b - a;
                    let t = if ab.length_squared() > 0.0 { ((px - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                    let d = px.distance(a + ab * t);
                    if d <= SNAP_PX && edge.is_none_or(|(bd, ..)| d < bd) && in_front(w[0].lerp(w[1], t)) {
                        edge = Some((d, bi, ei));
                    }
                }
            }
        }
        if let Some((_, p)) = corner {
            let p = p.as_dvec3();
            return Some(Picked { pick: Pick::Point(p), geom: Geom::Point(p), samples: Samples { points: vec![p], ..Default::default() } });
        }
        if let Some((_, bi, ei)) = edge {
            let e = &model.bodies[bi].edges[ei];
            let pts: Vec<DVec3> = e.points.iter().map(|p| p.as_dvec3()).collect();
            let segments = pts.windows(2).map(|w| [w[0], w[1]]).collect();
            return Some(Picked { pick: Pick::Edge { body: bi, edge: ei }, geom: e.geom.clone(), samples: Samples { segments, ..Default::default() } });
        }
        let (_, tri, bi) = hit?;
        let body = &model.bodies[bi];
        let face = *body.tri_face.get(tri)? as usize;
        let geom = body.faces.get(face)?.clone();
        Some(Picked { pick: Pick::Face { body: bi, face }, geom, samples: Samples { triangles: body.face_tris(face).collect(), ..Default::default() } })
    }

    /// The floating Measure window: what's picked, the readings (click one to
    /// copy it), and the units.
    pub(crate) fn measure_panel(&mut self, ui: &mut egui::Ui) {
        let Some(m) = self.measure.as_ref() else { return };
        let (unit, unit2) = (self.measure_unit, self.measure_unit2);
        ui.strong("Measure");
        let name = |p: &measure::Picked| match &p.geom {
            measure::Geom::Point(_) => "Corner",
            measure::Geom::Line { .. } => "Edge (straight)",
            measure::Geom::Circle { sweep, .. } if *sweep >= std::f64::consts::TAU - 1e-9 => "Edge (circle)",
            measure::Geom::Circle { .. } => "Edge (arc)",
            measure::Geom::Curve { .. } => "Edge (curve)",
            measure::Geom::Plane { .. } => "Face (flat)",
            measure::Geom::Cylinder { .. } => "Face (cylinder)",
            measure::Geom::Surface { .. } => "Face",
        };
        for (i, label) in ["Selection 1", "Selection 2"].into_iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(format!("{label}:"));
                match m.picks.get(i) {
                    Some(p) => ui.label(egui::RichText::new(name(p)).color(COL_SELECT)),
                    None => ui.weak("click a corner, edge or face"),
                };
            });
        }
        let mut copy: Option<String> = None;
        if !m.rows.is_empty() {
            ui.separator();
            egui::Grid::new("measure_rows").num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                for r in &m.rows {
                    ui.label(r.label);
                    ui.vertical(|ui| {
                        let main = unit.format(r.value, r.kind, false);
                        if ui.add(egui::Button::new(egui::RichText::new(&main).monospace()).frame(false)).on_hover_text("Click to copy").clicked() {
                            copy = Some(main);
                        }
                        if let Some(u2) = unit2 {
                            let other = u2.format(r.value, r.kind, true);
                            if ui.add(egui::Button::new(egui::RichText::new(&other).monospace().small().weak()).frame(false)).on_hover_text("Click to copy").clicked() {
                                copy = Some(other);
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            ui.weak("Click a value to copy it");
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Units");
            egui::ComboBox::from_id_salt("measure_unit").width(56.0).selected_text(self.measure_unit.name()).show_ui(ui, |ui| {
                for u in measure::Unit::ALL {
                    ui.selectable_value(&mut self.measure_unit, u, u.name());
                }
            });
            ui.label("2nd");
            egui::ComboBox::from_id_salt("measure_unit2").width(56.0).selected_text(self.measure_unit2.map_or("none", |u| u.name())).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.measure_unit2, None, "none");
                for u in measure::Unit::ALL {
                    ui.selectable_value(&mut self.measure_unit2, Some(u), u.name());
                }
            });
        });
        ui.horizontal(|ui| {
            if ui.button("Clear").clicked() {
                if let Some(m) = self.measure.as_mut() {
                    m.clear();
                }
            }
            if ui.button("Close").clicked() {
                self.measure = None;
            }
        });
        if let Some(text) = copy {
            ui.ctx().copy_text(text.clone());
            self.notify(format!("Copied {text}"));
        }
    }

    pub(crate) fn feature_name_mut(&mut self, f: Feature) -> Option<&mut String> {
        Some(match f {
            Feature::Sketch(i) => &mut self.sketches.get_mut(i)?.name,
            Feature::Extrude(k) => &mut self.extrudes.get_mut(k)?.name,
            Feature::Edge(k) => &mut self.edge_features.get_mut(k)?.name,
            Feature::Revolve(k) => &mut self.revolves.get_mut(k)?.name,
            Feature::Copy(k) => &mut self.copies.get_mut(k)?.name,
            Feature::Axis(k) => &mut self.axes.get_mut(k)?.name,
            Feature::Plane(k) => &mut self.planes.get_mut(k)?.name,
        })
    }

    /// The little box that renames a timeline feature (Enter keeps, Esc drops).
    pub(crate) fn rename_feature_window(&mut self, ctx: &egui::Context) {
        let Some((f, text)) = self.renaming_feature.as_mut() else { return };
        let f = *f;
        let mut done: Option<bool> = None;
        egui::Window::new("Rename").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -70.0)).show(ctx, |ui| {
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(180.0));
            if !r.has_focus() && !r.lost_focus() {
                r.request_focus();
            }
            let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
            if enter {
                done = Some(true);
            } else if esc {
                done = Some(false);
            }
        });
        if let Some(keep) = done {
            let new = self.renaming_feature.take().map(|(_, t)| t.trim().to_string()).unwrap_or_default();
            if keep && !new.is_empty() {
                if let Some(n) = self.feature_name_mut(f) {
                    *n = new;
                }
            }
        }
    }

    /// Move a timeline feature one place; refused (with the reason) if that
    /// makes something fail that built before.
    pub(crate) fn move_feature(&mut self, from: usize, gap: usize) {
        let Some(new) = doc::moved(&self.timeline, from, gap) else { return };
        self.ensure_model(Slot::Committed);
        let before = self.model(Slot::Committed).error_count();
        let (extrudes, revolves, edge_features, copies, axes, planes, _) = self.inputs(Slot::Committed);
        let mut tl = new.clone();
        tl.truncate(self.applied());
        tl.retain(|f| !self.suppressed.contains(f));
        let after = kernel::build_with(&self.sketches, &extrudes, &revolves, &edge_features, &copies, &axes, &planes, &tl).error_count();
        if after > before {
            self.notify("Can't move it there: something in the timeline would stop building".into());
            return;
        }
        self.timeline = new;
    }

    pub(crate) fn timeline_bar(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let mut delete = None;
        let mut rename: Option<Feature> = None;
        let mut toggle_suppress: Option<Feature> = None;
        let mut shift: Option<(usize, usize)> = None;
        let mut goto: Option<usize> = None;
        let len = self.timeline.len();
        if self.marker.is_some_and(|m| m >= len) {
            self.marker = None;
        }
        let applied = self.applied();
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 36.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.label("Timeline:");
            // Step the marker (Fusion's rollback bar): features after it stay
            // in the timeline but aren't built.
            if ui.add_sized([28.0, 36.0], egui::Button::new("⏮")).on_hover_text("Roll back to the start").clicked() {
                goto = Some(0);
            }
            if ui.add_sized([28.0, 36.0], egui::Button::new("◀")).on_hover_text("Step back one feature").clicked() {
                goto = Some(applied.saturating_sub(1));
            }
            if ui.add_sized([28.0, 36.0], egui::Button::new("▶")).on_hover_text("Step forward one feature").clicked() {
                goto = Some(applied + 1);
            }
            if ui.add_sized([28.0, 36.0], egui::Button::new("⏭")).on_hover_text("Roll forward to the end").clicked() {
                goto = Some(len);
            }
            if self.timeline.is_empty() {
                ui.weak("(empty)");
            }
            let mut rects: Vec<egui::Rect> = Vec::new();
            for (idx, f) in self.timeline.iter().enumerate() {
                let future = idx >= applied; // rolled back past this one
                let (name, selected) = match *f {
                    Feature::Sketch(i) => (self.sketches[i].name.clone(), self.active == Some(i)),
                    Feature::Extrude(k) => (
                        self.extrudes[k].name.clone(),
                        self.dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                    Feature::Edge(k) => (
                        self.edge_features[k].name.clone(),
                        self.edge_dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                    Feature::Revolve(k) => (
                        self.revolves[k].name.clone(),
                        self.revolve_dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                    Feature::Axis(k) => (self.axes[k].name.clone(), false),
                    Feature::Plane(k) => (
                        self.planes[k].name.clone(),
                        self.plane_dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                    Feature::Copy(k) => (
                        self.copies[k].name.clone(),
                        self.copy_dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                };
                let error = match *f {
                    Feature::Extrude(k) => self.feature_errors.get(k).cloned().flatten(),
                    Feature::Edge(k) => self.edge_feature_errors.get(k).cloned().flatten(),
                    Feature::Revolve(k) => self.revolve_errors.get(k).cloned().flatten(),
                    Feature::Copy(k) => self.copy_errors.get(k).cloned().flatten(),
                    Feature::Axis(k) => self.axis_errors.get(k).cloned().flatten(),
                    Feature::Plane(k) => self.plane_errors.get(k).cloned().flatten(),
                    Feature::Sketch(k) => self.sketch_errors.get(k).cloned().flatten(),
                };
                let icon = match *f {
                    Feature::Sketch(_) => Icon::Sketch,
                    Feature::Extrude(_) => Icon::Extrude,
                    Feature::Revolve(_) => Icon::Revolve,
                    Feature::Axis(_) => Icon::Axis,
                    Feature::Plane(_) => Icon::Plane,
                    Feature::Copy(k) => match self.copies[k].kind {
                        model::CopyKind::Mirror(_) => Icon::Mirror,
                        model::CopyKind::Rectangular { .. } => Icon::RectPattern,
                        model::CopyKind::Circular { .. } => Icon::CircPattern,
                    },
                    Feature::Edge(k) => match self.edge_features[k].kind {
                        EdgeKind::Fillet => Icon::Fillet,
                        EdgeKind::Chamfer => Icon::Chamfer,
                    },
                };
                // Underlined in the colour of the body this feature worked on.
                let underline = self.caches[Slot::Committed as usize]
                    .as_ref()
                    .and_then(|c| c.model.touched.get(f))
                    .map(|&id| self.body_color(id));
                let off = self.suppressed.contains(f);
                let tip = match (&error, future, off) {
                    (_, true, _) => format!("{name} — rolled back; click to roll forward to here"),
                    (_, _, true) => format!("{name} — suppressed (right-click to unsuppress)"),
                    (Some(e), _, _) => format!("{name}: {e}"),
                    (None, _, _) => name.clone(),
                };
                let r = icons::timeline_button(ui, icon, &tip, selected && !future, underline, future || off, error.is_some() && !future && !off);
                rects.push(r.rect);
                if r.drag_started() {
                    self.timeline_drag = Some(idx);
                }
                if r.clicked() {
                    if future {
                        goto = Some(idx + 1);
                    } else {
                        action = Some(*f);
                    }
                }
                r.context_menu(|ui| {
                    if ui.button("Rename…").clicked() {
                        rename = Some(*f);
                        ui.close();
                    }
                    if ui.button(if off { "Unsuppress" } else { "Suppress" }).clicked() {
                        toggle_suppress = Some(*f);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        delete = Some(*f);
                        ui.close();
                    }
                });
            }
            // Dragging a feature along the timeline: a bar shows where it will land.
            if let (Some(from), Some(p)) = (self.timeline_drag, ui.input(|i| i.pointer.interact_pos())) {
                let gap = rects.iter().filter(|r| r.center().x < p.x).count();
                if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
                    let x = if gap == 0 { first.left() - 3.0 } else if gap >= rects.len() { last.right() + 3.0 } else { (rects[gap - 1].right() + rects[gap].left()) / 2.0 };
                    ui.painter().line_segment([egui::pos2(x, first.top()), egui::pos2(x, first.bottom())], Stroke::new(3.0, COL_ARROW_HOT));
                }
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                if ui.input(|i| i.pointer.any_released()) {
                    self.timeline_drag = None;
                    shift = Some((from, gap));
                }
            }
            // The marker: a bar between built and rolled-back features, draggable.
            if let Some(first) = rects.first().copied() {
                let gap_x = |g: usize| if g == 0 { first.left() - 3.0 } else { rects[g - 1].right() + 3.0 };
                let x = gap_x(applied);
                let (top, bottom) = (first.top() - 2.0, first.bottom() + 2.0);
                let handle = egui::Rect::from_min_max(egui::pos2(x - 6.0, top), egui::pos2(x + 6.0, bottom));
                let resp = ui.interact(handle, egui::Id::new("timeline_marker"), egui::Sense::drag());
                let colour = if resp.dragged() || resp.hovered() { COL_ARROW_HOT } else { COL_ARROW };
                let painter = ui.painter();
                painter.line_segment([egui::pos2(x, top), egui::pos2(x, bottom)], Stroke::new(3.0, colour));
                painter.add(egui::Shape::convex_polygon(
                    vec![egui::pos2(x - 5.0, top), egui::pos2(x + 5.0, top), egui::pos2(x, top + 7.0)],
                    colour,
                    Stroke::NONE,
                ));
                resp.clone().on_hover_text("Drag to roll the design back (and forward) along the timeline");
                if resp.dragged() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        goto = (0..=len).min_by(|&a, &b| (gap_x(a) - p.x).abs().total_cmp(&(gap_x(b) - p.x).abs()));
                    }
                }
            }
        });
        if let Some(n) = goto {
            self.set_marker(n);
        }
        if let Some(f) = rename {
            let text = self.feature_name_mut(f).map(|n| n.clone()).unwrap_or_default();
            self.renaming_feature = Some((f, text));
        }
        if let Some(f) = toggle_suppress {
            match self.suppressed.iter().position(|x| *x == f) {
                Some(p) => {
                    self.suppressed.remove(p);
                }
                None => self.suppressed.push(f),
            }
        }
        if let Some((from, gap)) = shift {
            self.move_feature(from, gap);
        }
        self.rename_feature_window(ui.ctx());
        match delete {
            Some(Feature::Sketch(i)) => self.delete_sketch(i),
            Some(Feature::Extrude(k)) => self.delete_extrude(k),
            Some(Feature::Edge(k)) => self.delete_edge_feature(k),
            Some(Feature::Revolve(k)) => self.delete_revolve(k),
            Some(Feature::Copy(k)) => self.delete_copy(k),
            Some(Feature::Axis(k)) => self.delete_axis(k),
            Some(Feature::Plane(k)) => self.delete_plane(k),
            None => {}
        }
        if delete.is_some() {
            return;
        }
        match action {
            Some(Feature::Sketch(i)) => self.edit_sketch(i),
            Some(Feature::Extrude(k)) => self.open_extrude(Some(k)),
            Some(Feature::Revolve(k)) => self.open_revolve(Some(k)),
            Some(Feature::Axis(_)) => {}
            Some(Feature::Plane(k)) => {
                self.cancel_all_dialogs();
                self.plane_dialog = Some(PlaneDialog::from_feature(k, &self.planes[k]));
            }
            Some(Feature::Copy(k)) => {
                let mode = match self.copies[k].kind {
                    model::CopyKind::Mirror(_) => CopyMode::Mirror,
                    model::CopyKind::Rectangular { .. } => CopyMode::Rectangular,
                    model::CopyKind::Circular { .. } => CopyMode::Circular,
                };
                self.open_copy(mode, Some(k));
            }
            Some(Feature::Edge(k)) => {
                let kind = self.edge_features[k].kind;
                self.open_edge_dialog(Some(k), kind);
            }
            None => {}
        }
    }}
