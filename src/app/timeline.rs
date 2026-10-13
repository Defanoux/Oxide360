//! The timeline bar, renaming, and the Measure tool.

use crate::*;

impl OpenCad {
    /// Measure tool: what's under the cursor, clicks pick it, Esc leaves.
    /// Ctrl locks to snap points, Shift hides them (as in Fusion).
    pub(crate) fn measure_input(&mut self, view: &View, hover_px: Option<Vec2>, response: &egui::Response, aspect: f32, min: Vec2, size: Vec2) {
        if self.measure.is_none() {
            return;
        }
        let mods = response.ctx.input(|i| i.modifiers);
        let (hover, snaps) = match hover_px {
            Some(px) => {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                self.measure_pick_at(view, px, &ray, size.y, mods.ctrl, mods.shift)
            }
            None => (None, Vec::new()),
        };
        // Picks whose geometry changed in a rebuild go.
        let now: Vec<Option<measure::Geom>> = self.measure.as_ref().map_or(Vec::new(), |m| m.picks.iter().map(|p| self.measure_resolve(p.pick).map(|q| q.geom)).collect());
        let Some(m) = self.measure.as_mut() else { return };
        let mut k = 0;
        m.keep_valid(|_| {
            k += 1;
            now.get(k - 1).cloned().flatten()
        });
        m.hover = hover;
        m.snaps = snaps;
        if response.clicked() {
            if let Some(p) = m.hover.clone() {
                m.click(p);
            }
        }
        if response.ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.measure = None;
        }
    }

    /// Where a sketch's plane is, exactly: (origin, x axis, y axis).
    fn sketch_frame(&self, i: usize) -> (DVec3, DVec3, DVec3) {
        let sk = &self.sketches[i];
        let now = self.caches[Slot::Committed as usize]
            .as_ref()
            .filter(|c| c.sketches.get(i).is_some_and(|s| s.same_shape(sk)))
            .and_then(|c| c.model.sketch_frames.get(i).copied().flatten());
        match (sk.face, now) {
            (Some(_), Some(f)) => (f.origin, f.u, f.v),
            _ => {
                let p = sk.sketch_plane();
                (p.origin.as_dvec3(), p.u.as_dvec3(), p.v.as_dvec3())
            }
        }
    }

    /// A pick as it is in the current model and sketches.
    pub(crate) fn measure_resolve(&self, pick: measure::Pick) -> Option<measure::Picked> {
        use measure::{Geom, Pick, Picked, Samples};
        let model = &self.caches[Slot::Committed as usize].as_ref()?.model;
        let segments = |pts: &[DVec3]| Samples { segments: pts.windows(2).map(|w| [w[0], w[1]]).collect(), ..Default::default() };
        let (geom, samples) = match pick {
            Pick::Point(p) => return Some(Picked::point(p)),
            Pick::Edge { body, edge } => {
                let e = model.bodies.get(body)?.edges.get(edge)?;
                let pts: Vec<DVec3> = e.points.iter().map(|p| p.as_dvec3()).collect();
                (e.geom.clone(), segments(&pts))
            }
            Pick::Face { body, face } => {
                let b = model.bodies.get(body)?;
                (b.faces.get(face)?.clone(), Samples { triangles: b.face_tris(face).collect(), ..Default::default() })
            }
            Pick::Body { body } => {
                let b = model.bodies.get(body)?;
                let triangles = b.mesh.tris.iter().map(|t| t.map(|i| b.mesh.positions[i as usize].as_dvec3())).collect();
                (Geom::Body, Samples { triangles, ..Default::default() })
            }
            Pick::SketchPoint { sketch, point } => {
                let (o, u, v) = self.sketch_frame(sketch);
                let q = *self.sketches.get(sketch)?.points.get(point)?;
                return Some(Picked { pick, ..Picked::point(o + u * q.x + v * q.y) });
            }
            Pick::SketchCurve { sketch, entity } => {
                let sk = self.sketches.get(sketch)?;
                let (o, u, v) = self.sketch_frame(sketch);
                let w = |p: DVec2| o + u * p.x + v * p.y;
                let round = |c: DVec2, r: f64, a0: f64, sweep: f64| {
                    let pts: Vec<DVec3> = (0..=96).map(|k| w(c + DVec2::from_angle(a0 + sweep * k as f64 / 96.0) * r)).collect();
                    (Geom::Circle { center: w(c), normal: u.cross(v), radius: r, sweep }, segments(&pts))
                };
                match *sk.entities.get(entity)? {
                    sketch::Entity::Line { a, b } => {
                        let (a, b) = (w(sk.points[a]), w(sk.points[b]));
                        (Geom::Line { a, b }, segments(&[a, b]))
                    }
                    sketch::Entity::Circle { center, radius } => round(sk.points[center], radius, 0.0, std::f64::consts::TAU),
                    sketch::Entity::Arc { .. } => {
                        let (c, r, a0, sweep) = sk.arc_geometry(entity)?;
                        round(c, r, a0, sweep)
                    }
                    sketch::Entity::Point { p } => return Some(Picked { pick, ..Picked::point(w(sk.points[p])) }),
                }
            }
            Pick::Profile { sketch, region } => {
                let sk = self.sketches.get(sketch)?;
                let r = find_regions(sk).into_iter().nth(region)?;
                let (o, u, v) = self.sketch_frame(sketch);
                let w = |p: DVec2| o + u * p.x + v * p.y;
                let (verts, tris) = r.triangulate();
                let triangles = tris.iter().map(|t| t.map(|i| w(verts[i]))).collect();
                let geom = Geom::Plane { normal: u.cross(v), area: r.exact_area(), loop_length: r.loop_length(), center: r.round_center().map(w) };
                (geom, Samples { triangles, ..Default::default() })
            }
        };
        Some(Picked { pick, geom, samples })
    }

    /// Snap points on a pick (beyond its corners): an edge's middle, a circle's centre.
    fn measure_snaps(p: &measure::Picked) -> Vec<DVec3> {
        use measure::Geom;
        let mut out = Vec::new();
        if let (Some(first), Some(last)) = (p.samples.segments.first(), p.samples.segments.last()) {
            let pts: Vec<DVec3> = std::iter::once(first[0]).chain(p.samples.segments.iter().map(|s| s[1])).collect();
            // (A whole circle has no ends, and its "middle" would be anywhere.)
            if first[0].distance(last[1]) > 1e-9 {
                out.extend([first[0], last[1]]);
                out.extend(measure::middle(&pts));
            }
        }
        match p.geom {
            Geom::Circle { center, .. } => out.push(center),
            Geom::Plane { center: Some(c), .. } => out.push(c),
            _ => {}
        }
        out
    }

    /// What's under the cursor for Measure, and the snap points to show on it.
    /// With the Face/Edge/Vertex filter: corners and sketch points first, then
    /// edges and sketch curves, then faces and sketch profiles (nearest).
    fn measure_pick_at(&self, view: &View, px: Vec2, ray: &Ray, height_px: f32, ctrl: bool, shift: bool) -> (Option<measure::Picked>, Vec<DVec3>) {
        use measure::{Filter, Pick, Picked};
        let Some(m) = self.measure.as_ref() else { return (None, Vec::new()) };
        let Some(model) = self.caches[Slot::Committed as usize].as_ref().map(|c| &c.model) else { return (None, Vec::new()) };
        let visible = |b: &&kernel::Body| !self.hidden_bodies.contains(&b.id);
        let hit = model
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, b)| visible(b))
            .filter_map(|(i, b)| model::ray_hit_tri(&b.mesh, ray.origin, ray.dir).map(|(t, tri)| (t, tri, i)))
            .min_by(|x, y| x.0.total_cmp(&y.0));
        if m.filter == Filter::Body {
            return (hit.and_then(|(_, _, body)| self.measure_resolve(Pick::Body { body })), Vec::new());
        }
        // Things behind a body can't be picked.
        let slack = self.camera.distance * 0.01;
        let in_front = |p: Vec3| hit.is_none_or(|(h, ..)| (p - ray.origin).dot(ray.dir) <= h + slack);
        let mut point: Option<(f32, Pick)> = None;
        let mut curve: Option<(f32, Pick)> = None;
        for (bi, body) in model.bodies.iter().enumerate().filter(|(_, b)| visible(b)) {
            for (ei, e) in body.edges.iter().enumerate() {
                let (Some(&first), Some(&last)) = (e.points.first(), e.points.last()) else { continue };
                // A closed edge (a whole circle) has no real corner.
                if first.distance(last) > 1e-6 {
                    for p in [first, last] {
                        let Some(q) = view.project(p) else { continue };
                        let d = px.distance(q);
                        if d <= SNAP_PX && point.is_none_or(|(bd, _)| d < bd) && in_front(p) {
                            point = Some((d, Pick::Point(p.as_dvec3())));
                        }
                    }
                }
                for w in e.points.windows(2) {
                    let (Some(a), Some(b)) = (view.project(w[0]), view.project(w[1])) else { continue };
                    let d = dist_to_segment_px(px, a, b);
                    let t = ((px - a).dot(b - a) / (b - a).length_squared().max(1e-12)).clamp(0.0, 1.0);
                    if d <= SNAP_PX && curve.is_none_or(|(bd, _)| d < bd) && in_front(w[0].lerp(w[1], t)) {
                        curve = Some((d, Pick::Edge { body: bi, edge: ei }));
                    }
                }
            }
        }
        // Visible sketches: their points and curves (drawn over the bodies, so
        // they win), and their profiles.
        let mut profile: Option<(f32, Pick)> = None;
        for s in (0..self.sketches.len()).filter(|&s| self.sketch_visible(s)) {
            let sk = &self.sketches[s];
            let Some((t, p)) = self.plane_of(s).intersect_t(ray) else { continue };
            let world = self.plane_of(s).to_world(p);
            if !in_front(world) {
                continue;
            }
            let mm_per_px = (t * 2.0 * (camera::FOV_Y * 0.5).tan() / height_px.max(1.0)) as f64;
            match sk.hit(p, SNAP_PX as f64 * mm_per_px) {
                Some(Sel::Point(q)) if q != Sketch::ORIGIN => point = Some((0.0, Pick::SketchPoint { sketch: s, point: q })),
                Some(Sel::Entity(e)) => curve = Some((0.0, Pick::SketchCurve { sketch: s, entity: e })),
                _ => {}
            }
            // The smallest profile under the cursor on the nearest sketch.
            let regions = find_regions(sk);
            if let Some((ri, _)) = regions.iter().enumerate().filter(|(_, r)| r.contains(p)).min_by(|a, b| a.1.area().total_cmp(&b.1.area())) {
                if profile.is_none_or(|(bt, _)| t < bt) {
                    profile = Some((t, Pick::Profile { sketch: s, region: ri }));
                }
            }
        }
        let surface = match (profile, hit) {
            (Some((tp, pick)), Some((th, ..))) if tp <= th + slack => Some(pick),
            (Some((_, pick)), None) => Some(pick),
            (_, Some((_, tri, bi))) => model.bodies[bi].tri_face.get(tri).map(|&f| Pick::Face { body: bi, face: f as usize }),
            (None, None) => None,
        };
        let Some(pick) = point.or(curve).map(|(_, p)| p).or(surface) else { return (None, Vec::new()) };
        let Some(picked) = self.measure_resolve(pick) else { return (None, Vec::new()) };
        // Snap points on it (Shift hides them, Ctrl locks to the nearest).
        if shift || !self.measure_snap {
            return (Some(picked), Vec::new());
        }
        let snaps = Self::measure_snaps(&picked);
        let near = snaps
            .iter()
            .filter_map(|&q| view.project(q.as_vec3()).map(|s| (px.distance(s), q)))
            .filter(|&(d, _)| ctrl || d <= SNAP_PX)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        match near {
            Some((_, q)) if !matches!(picked.pick, Pick::Point(_) | Pick::SketchPoint { .. }) => (Some(Picked::point(q)), snaps),
            _ => (Some(picked), snaps),
        }
    }

    /// The floating Measure window, laid out like Fusion's: selection filter,
    /// Selection 1 / 2 with their readings, Results, then the settings. Click
    /// a value to copy it; hover one to see it in the view.
    pub(crate) fn measure_panel(&mut self, ui: &mut egui::Ui) {
        use measure::{Filter, Section, Show};
        let Some(m) = self.measure.as_mut() else { return };
        let (unit, unit2, decimals) = (self.measure_unit, self.measure_unit2, self.measure_precision);
        ui.strong("Measure");
        ui.horizontal(|ui| {
            ui.label("Selection filter");
            let before = m.filter;
            egui::ComboBox::from_id_salt("measure_filter")
                .selected_text(match m.filter {
                    Filter::FaceEdgeVertex => "Face/Edge/Vertex",
                    Filter::Body => "Body",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut m.filter, Filter::FaceEdgeVertex, "Face/Edge/Vertex");
                    ui.selectable_value(&mut m.filter, Filter::Body, "Body");
                });
            if m.filter != before {
                m.clear();
            }
        });
        let mut copy: Option<String> = None;
        let mut hovered: Option<Show> = None;
        let xyz = self.measure_xyz;
        let mut rows_ui = |ui: &mut egui::Ui, section: Section| {
            egui::Grid::new(("measure_rows", format!("{section:?}"))).num_columns(2).spacing([12.0, 2.0]).show(ui, |ui| {
                for r in m.rows.iter().filter(|r| r.section == section && (xyz || !matches!(r.show, Show::Delta(_)))) {
                    let l = ui.label(r.label);
                    let cell = ui.vertical(|ui| {
                        let main = unit.format(r.value, r.kind, false, decimals);
                        if ui.add(egui::Button::new(egui::RichText::new(&main).monospace()).frame(false)).on_hover_text("Click to copy").clicked() {
                            copy = Some(main);
                        }
                        if let Some(u2) = unit2 {
                            let other = u2.format(r.value, r.kind, true, decimals);
                            if ui.add(egui::Button::new(egui::RichText::new(&other).monospace().small().weak()).frame(false)).on_hover_text("Click to copy").clicked() {
                                copy = Some(other);
                            }
                        }
                    });
                    if l.hovered() || cell.response.contains_pointer() {
                        hovered = Some(r.show);
                    }
                    ui.end_row();
                }
            });
        };
        for i in 0..2 {
            ui.separator();
            match m.picks.get(i) {
                Some(p) => {
                    let name = p.name();
                    ui.horizontal(|ui| {
                        ui.label(format!("Selection {}:", i + 1));
                        ui.label(egui::RichText::new(name).color(COL_SELECT));
                    });
                    rows_ui(ui, Section::Selection(i));
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.label(format!("Selection {}:", i + 1));
                        ui.weak(if i == 0 || !m.picks.is_empty() { "click something" } else { "" });
                    });
                }
            }
        }
        if m.rows.iter().any(|r| r.section == Section::Results) {
            ui.separator();
            ui.label("Results");
            rows_ui(ui, Section::Results);
        }
        m.hover_row = hovered;
        if !m.rows.is_empty() {
            ui.weak("Click a value to copy it");
        }
        ui.separator();
        egui::Grid::new("measure_settings").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            ui.label("Precision");
            egui::ComboBox::from_id_salt("measure_precision").selected_text(format!("{:.*}", decimals, 0.0)).show_ui(ui, |ui| {
                for d in 0..=6 {
                    ui.selectable_value(&mut self.measure_precision, d, format!("{:.*}", d, 0.0));
                }
            });
            ui.end_row();
            ui.label("Units");
            egui::ComboBox::from_id_salt("measure_unit").selected_text(self.measure_unit.name()).show_ui(ui, |ui| {
                for u in measure::Unit::ALL {
                    ui.selectable_value(&mut self.measure_unit, u, u.name());
                }
            });
            ui.end_row();
            ui.label("Secondary units");
            egui::ComboBox::from_id_salt("measure_unit2").selected_text(self.measure_unit2.map_or("None", |u| u.name())).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.measure_unit2, None, "None");
                for u in measure::Unit::ALL {
                    ui.selectable_value(&mut self.measure_unit2, Some(u), u.name());
                }
            });
            ui.end_row();
            ui.label("XYZ Delta");
            egui::ComboBox::from_id_salt("measure_xyz").selected_text(if self.measure_xyz { "Global" } else { "None" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut self.measure_xyz, false, "None");
                ui.selectable_value(&mut self.measure_xyz, true, "Global");
            });
            ui.end_row();
        });
        ui.checkbox(&mut self.measure_snap, "Show snap points").on_hover_text("Edge middles and circle centres. Hold Ctrl to lock to them, Shift to hide them");
        ui.horizontal(|ui| {
            if ui.button("Clear selection").clicked() {
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
