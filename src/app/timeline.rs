//! The timeline bar, renaming, and the Measure tool.

use crate::*;

impl OpenCad {
    /// Measure tool: the point under the cursor (snapped), clicks, Esc to leave.
    pub(crate) fn measure_input(&mut self, hover_px: Option<Vec2>, response: &egui::Response, aspect: f32, min: Vec2, size: Vec2) {
        let Some((m, hover)) = self.measure.as_mut() else { return };
        *hover = None;
        let model = &self.caches[Slot::Committed as usize].as_ref().expect("model built").model;
        if let Some(px) = hover_px {
            let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
            let hit = model
                .bodies
                .iter()
                .filter(|b| !self.hidden_bodies.contains(&b.id))
                .filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir))
                .reduce(f32::min);
            if let Some(t) = hit {
                let p = (ray.origin + ray.dir * t).as_dvec3();
                let corners = model.bodies.iter().filter(|b| !self.hidden_bodies.contains(&b.id)).flat_map(|b| {
                    b.edges.iter().flat_map(|e| [e.points.first().copied(), e.points.last().copied(), Some(e.at.mid.as_vec3())].into_iter().flatten().map(|v| v.as_dvec3()))
                });
                let tol = f64::from(self.camera.distance) * 0.03;
                *hover = Some(measure::snap(p, corners, tol));
            }
        }
        if response.clicked() {
            if let Some(p) = *hover {
                m.click(p);
            }
        }
        if response.ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.measure = None;
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
    pub(crate) fn move_feature(&mut self, at: usize, later: bool) {
        let Some(new) = doc::swapped(&self.timeline, at, later) else { return };
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
        let mut shift: Option<(usize, bool)> = None;
        let mut goto: Option<usize> = None;
        let len = self.timeline.len();
        if self.marker.is_some_and(|m| m >= len) {
            self.marker = None;
        }
        let applied = self.applied();
        ui.horizontal(|ui| {
            ui.label("Timeline:");
            // Step the marker (Fusion's rollback bar): features after it stay
            // in the timeline but aren't built.
            if ui.small_button("⏮").on_hover_text("Roll back to the start").clicked() {
                goto = Some(0);
            }
            if ui.small_button("◀").on_hover_text("Step back one feature").clicked() {
                goto = Some(applied.saturating_sub(1));
            }
            if ui.small_button("▶").on_hover_text("Step forward one feature").clicked() {
                goto = Some(applied + 1);
            }
            if ui.small_button("⏭").on_hover_text("Roll forward to the end").clicked() {
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
                    if ui.add_enabled(idx > 0, egui::Button::new("Move earlier")).clicked() {
                        shift = Some((idx, false));
                        ui.close();
                    }
                    if ui.add_enabled(idx + 1 < len, egui::Button::new("Move later")).clicked() {
                        shift = Some((idx, true));
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        delete = Some(*f);
                        ui.close();
                    }
                });
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
        if let Some((at, later)) = shift {
            self.move_feature(at, later);
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
    }

}
