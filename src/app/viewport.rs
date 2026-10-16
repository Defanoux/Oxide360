//! The 3D / sketch viewport: input and drawing.

use crate::*;

impl OpenCad {
    pub(crate) fn viewport(&mut self, ui: &mut egui::Ui) {
        // Press Pull clicked a fillet or a profile last frame: open its tool now.
        match self.press_pull_go.take() {
            Some(PressPullHover::Fillet(k)) => self.open_edge_dialog(Some(k), EdgeKind::Fillet),
            Some(PressPullHover::Profile(s, ri)) => {
                self.open_extrude(None);
                if let Some(d) = self.dialog.as_mut() {
                    (d.sketch, d.profiles) = (Some(s), vec![ri]);
                }
            }
            _ => {}
        }
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        self.view_rect = rect;
        let sketching = self.active.is_some();

        crate::mark("view cube: sits on top of the viewport");
        // --- view cube: sits on top of the viewport, so it gets clicks first ---
        let cube_center = rect.right_top() + egui::vec2(-CUBE_MARGIN, CUBE_TOP);
        let cube_rect = egui::Rect::from_center_size(cube_center, egui::vec2(CUBE_MARGIN * 1.8, CUBE_MARGIN * 1.8));
        let cube_resp = ui.interact(cube_rect, ui.id().with("view_cube"), egui::Sense::click());
        let cells = cube_cells();
        let now = ui.input(|inp| inp.time);
        let cube_hover = cube_resp
            .hover_pos()
            .and_then(|p| self.camera.cube_hit(&cells, (to_vec(p) - to_vec(cube_center)) / CUBE_SCALE))
            .map(|i| cells[i].dir);
        if cube_resp.clicked() {
            if let Some(dir) = cube_hover {
                // Already looking straight at it? Then flip to the opposite side.
                let dir = cube_click_target(-self.camera.basis().0, dir);
                let (yaw, pitch) = angles_for_dir(dir);
                let to = Pose { yaw: yaw.to_radians(), pitch: pitch.to_radians(), roll: 0.0, ..self.camera.pose() };
                self.move_camera(to, now);
            }
        }
        // A sketch was just started: glide round to face its plane.
        if let Some(to) = self.pending_turn.take() {
            self.turn = Some(CameraTurn { from: self.camera.pose(), to, start: now, seconds: SKETCH_TURN_SECONDS });
        }
        if let Some(t) = &self.turn {
            let k = ((now - t.start) / t.seconds).clamp(0.0, 1.0) as f32;
            let e = k * k * (3.0 - 2.0 * k); // ease in and out
            self.camera.set_pose(Pose::lerp(t.from, t.to, e));
            if k >= 1.0 {
                self.turn = None;
            } else {
                ui.ctx().request_repaint();
            }
        }
        let typing = ui.ctx().text_edit_focused();
        let press_px = ui.input(|i| i.pointer.press_origin()).map(to_vec);
        let hover_px = response.hover_pos().map(to_vec);
        let near_arrow = |p: Option<Vec2>, arrows: &[Option<(Vec2, Vec2)>]| -> Option<usize> {
            let p = p?;
            arrows.iter().position(|a| a.is_some_and(|(a, b)| dist_to_segment_px(p, a, b) < 10.0))
        };
        // Grabbing a drag arrow (decided before the camera, so it doesn't orbit).
        let grabbed = if response.drag_started_by(egui::PointerButton::Primary) { near_arrow(press_px, &self.arrows) } else { None };
        let arrow_active = grabbed.is_some() || self.arrow_drag.is_some();

        let min = to_vec(rect.min);
        let size = Vec2::new(rect.width().max(1.0), rect.height().max(1.0));
        let aspect = size.x / size.y;

        crate::mark("camera input");
        // --- camera input ---
        let shift = ui.input(|i| i.modifiers.shift);
        let d = response.drag_delta();
        let middle = response.dragged_by(egui::PointerButton::Middle);
        if (!sketching && !arrow_active && response.dragged_by(egui::PointerButton::Primary)) || (shift && middle) {
            self.turn = None;
            self.camera.orbit(d.x, d.y);
        } else if response.dragged_by(egui::PointerButton::Secondary) || middle {
            self.camera.pan(d.x, d.y);
        }
        if response.hovered() {
            // Zoom toward whatever is under the cursor.
            let at = hover_px.map_or(Vec2::ZERO, |p| ndc_from_pixel(p, min, size));
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if pinch != 1.0 {
                // Pinch on a touchpad, or Ctrl+scroll. On a touchpad, fingers
                // up zoom in (unless flipped back in Settings).
                let z = pinch.ln() / 0.002;
                let z = if self.touchpad && self.touchpad_zoom_flip { -z } else { z };
                self.camera.zoom_at(z, aspect, at);
            }
            if scroll != egui::Vec2::ZERO {
                if self.touchpad {
                    // Two fingers: pan; with Shift: orbit.
                    self.turn = None;
                    if shift {
                        self.camera.orbit(scroll.x, scroll.y);
                    } else {
                        self.camera.pan(scroll.x, scroll.y);
                    }
                } else if scroll.y != 0.0 {
                    self.camera.zoom_at(scroll.y, aspect, at);
                }
            }
        }
        let painter = ui.painter_at(rect);
        let view = View { painter: &painter, vp: self.camera.view_proj(aspect), min, size, trace: None };
        let grid = GridSpec::for_units(self.camera.distance, self.units);
        let dplane = self.active.map_or(OriginPlane::XY.plane(), |i| self.plane_of(i));
        let mm_per_px = (self.camera.distance * 2.0 * (FOV_Y * 0.5).tan() / size.y) as f64;
        let regions: Vec<Vec<Region>> = self.sketches.iter().map(find_regions).collect();

        // Polygon tools ask for the number of sides after the first click.
        if Tool::POLYGONS.contains(&self.editor.tool) && self.editor.start.is_some() {
            if !self.poly_asked && self.float.is_none() {
                self.poly_asked = true;
                self.float = Some(FloatEdit {
                    target: EditTarget::PolygonSides,
                    text: self.editor.sides().to_string(),
                    focus: true,
                    keep_text: false,
                });
            }
        } else {
            self.poly_asked = false;
            if self.float.as_ref().is_some_and(|f| f.target == EditTarget::PolygonSides) {
                self.float = None;
            }
        }
        // The corner size box only lives while its tool does.
        if matches!(self.float.as_ref().map(|f| f.target), Some(EditTarget::Corner(_)))
            && !(matches!(self.editor.tool, Tool::SketchFillet | Tool::SketchChamfer) && self.active.is_some())
        {
            self.float = None;
        }
        crate::mark("arrow drag (extrude distance, fillet /");
        // --- arrow drag (extrude distance, fillet / chamfer size, revolve angle, pattern arrows) ---
        let handles = self.current_handles(&regions);
        if let Some(k) = grabbed.filter(|&k| k < handles.len()) {
            if let Some(c) = press_px.and_then(|px| self.sample_handle(&handles[k], px, aspect, min, size)) {
                self.arrow_drag = Some((k, c - handles[k].value())); // keep the grab point under the cursor
                self.arrow_turn = Some((c, c));
            }
        }
        if let (Some((k, offset)), Some(px)) = (self.arrow_drag, hover_px) {
            if let (Some(h), true) = (handles.get(k).copied(), response.dragged_by(egui::PointerButton::Primary)) {
                if let Some(c) = self.sample_handle(&h, px, aspect, min, size) {
                    let mut v = match (h, self.arrow_turn) {
                        (Handle::Turn { .. }, Some((acc, raw))) => {
                            let acc = acc + handles::angle_step(raw, c);
                            self.arrow_turn = Some((acc, c));
                            acc - offset
                        }
                        _ => c - offset,
                    };
                    if self.grid_snap {
                        let step = if matches!(h, Handle::Turn { .. }) { 1.0 } else { grid.step / 10.0 };
                        v = (v / step).round() * step;
                    }
                    self.set_handle_value(h.target(), v);
                }
            }
        }
        if self.arrow_drag.is_some() && response.drag_stopped() {
            self.arrow_drag = None;
            self.arrow_turn = None;
            self.focus_distance();
        }

        // Part-way through a shape that takes a typed size, a letter starts a
        // parameter name in its box (as Fusion does), not a tool.
        let ed = &self.editor;
        let mid_shape = self.active.is_some()
            && self.float.is_none()
            && ((matches!(ed.tool, Tool::Rectangle | Tool::RectangleCenter | Tool::Circle | Tool::Line) && ed.start.is_some())
                || (ed.tool == Tool::Slot && ed.second.is_some())
                || (ed.tool == Tool::Offset && ed.selection.len() == 1));
        let name_typed = mid_shape && typed_number(ui, !self.parameters.is_empty()).is_some_and(|t| t.starts_with(|c: char| c.is_alphabetic() || c == '_'));
        // Tool keys (Settings → Keybinds); none while a key is being set.
        let pressed = |a: keys::Action| {
            !name_typed
                && self.capturing_key.is_none()
                && self.keymap.key(a).is_some_and(|k| ui.input(|i| i.key_pressed(k) && !i.modifiers.command && !i.modifiers.alt))
        };
        use keys::Action as A;
        let (key_l, key_r, key_c, key_g, key_d, key_x, key_p) =
            (pressed(A::Line), pressed(A::Rectangle), pressed(A::Circle), pressed(A::GridSnap), pressed(A::Dimension), pressed(A::Construction), pressed(A::Project));
        let (key_trim, key_extend, key_offset) = (pressed(A::Trim), pressed(A::Extend), pressed(A::Offset));
        let (key_n, key_e, key_f, key_i) = (pressed(A::NewSketch), pressed(A::Extrude), pressed(A::Fillet), pressed(A::Measure));
        let (key_h, key_q, key_m) = (pressed(A::Hole), pressed(A::PressPull), pressed(A::Move));
        let (esc, del, undo) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::Z) && i.modifiers.command,
            )
        });

        // Esc in the 3D view lets go of bodies selected in the browser.
        if esc && !typing && self.active.is_none() {
            self.selected_bodies.clear();
        }

        crate::mark("pattern axis: click one of the axes in");
        // --- pattern axis: click one of the axes in the view ---
        self.copy_axis_hover = None;
        if self.copy_dialog.as_ref().is_some_and(|d| d.picking_axis) {
            self.ensure_model(Slot::Committed);
            let reach = origin::size_for(self.camera.distance);
            let mut found: Vec<(AxisChoice, Vec3, Vec3)> = model::WorldAxis::ALL
                .into_iter()
                .map(|a| (AxisChoice::World(a), Vec3::ZERO, a.dir().as_vec3() * reach))
                .collect();
            for (k, a) in self.model(Slot::Committed).axes.iter().enumerate() {
                if let Some((o, d)) = a {
                    found.push((AxisChoice::Custom(k), (*o - *d * reach as f64 * 1.5).as_vec3(), (*o + *d * reach as f64 * 1.5).as_vec3()));
                }
            }
            if let Some(px) = hover_px {
                self.copy_axis_hover = found
                    .iter()
                    .filter_map(|&(c, a, b)| {
                        let d = dist_to_segment_px(px, view.project(a)?, view.project(b)?);
                        (d < 9.0).then_some((c, d))
                    })
                    .min_by(|x, y| x.1.total_cmp(&y.1))
                    .map(|(c, _)| c);
            }
            if let (true, Some(c), Some(d)) = (response.clicked(), self.copy_axis_hover, self.copy_dialog.as_mut()) {
                match c {
                    AxisChoice::World(a) => (d.axis, d.custom) = (a, None),
                    AxisChoice::Custom(k) => d.custom = Some(k),
                }
                d.picking_axis = false;
            }
            if esc && !typing {
                if let Some(d) = self.copy_dialog.as_mut() {
                    d.picking_axis = false;
                }
            }
        }

        crate::mark("create sketch / offset plane: pick an ");
        // --- create sketch / offset plane: pick an origin plane, a construction plane or a flat face ---
        self.face_hover = None;
        self.origin_hover = None;
        self.cplane_hover = None;
        if self.face_pick {
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let found = self.pick_face(&ray);
                self.face_pick_point = found.map(|(t, ..)| (ray.origin + ray.dir * t).as_dvec3());
                // The planes count too; whichever is nearer wins, and a body's
                // face wins a tie (a plane lying in a face is covered by it).
                // (An offset plane is measured from origin planes and faces only.)
                let psize = origin::size_for(self.camera.distance);
                let origin_hit = origin::pick(&ray, psize, |_| true);
                let custom_hit = if self.plane_pick { None } else { self.pick_cplane(&ray, psize) };
                let plane = match (origin_hit, custom_hit) {
                    (Some((op, to)), Some((_, tc))) if to <= tc => Some((Some(op), None, to)),
                    (_, Some((k, tc))) => Some((None, Some(k), tc)),
                    (Some((op, to)), None) => Some((Some(op), None, to)),
                    (None, None) => None,
                };
                match (plane, found) {
                    (Some((op, k, tp)), f) if f.is_none_or(|(tf, ..)| tp < tf - origin::tie_tolerance(tf)) => {
                        self.origin_hover = op;
                        self.cplane_hover = k;
                    }
                    (_, f) => self.face_hover = f.map(|(_, b, face)| (b, face)),
                }
            }
            if response.clicked() {
                if self.plane_pick {
                    // Offset Plane: measure from what was clicked.
                    let base = match (self.origin_hover, self.face_hover) {
                        (Some(op), _) => {
                            let p = op.plane();
                            Some((sketch::FacePlane { origin: DVec3::ZERO, u: p.u.as_dvec3(), v: p.v.as_dvec3(), follow: None }, true, [0.0; 2], [0.0; 4]))
                        }
                        (None, Some((b, face))) => {
                            let q = self.face_pick_point.unwrap_or(face.origin);
                            let extent = self
                                .model(Slot::Committed)
                                .bodies
                                .get(b)
                                .map_or([0.0; 4], |body| model::face_extent(&body.mesh, &face));
                            Some((sketch::FacePlane { follow: None, ..face }, false, [q.dot(face.u), q.dot(face.v)], extent))
                        }
                        _ => None,
                    };
                    if let Some((base, on_origin, anchor, extent)) = base {
                        self.cancel_all_dialogs();
                        self.open_plane_dialog(PlaneDialog::new(base, on_origin, anchor, extent));
                    }
                } else if let Some(i) = self.redefine.take() {
                    // Redefine Sketch Plane: the sketch moves onto what was clicked
                    // (on a face, centred where it was clicked).
                    let mut new = self.sketches[i].clone();
                    let mut tied_note = false;
                    let picked = if let Some(op) = self.origin_hover {
                        new.redefine_plane(&Sketch::new(String::new(), op));
                        true
                    } else if let Some(k) = self.cplane_hover {
                        let frame = self.caches[Slot::Committed as usize].as_ref().and_then(|c| c.model.planes.get(k).copied().flatten());
                        if let Some(f) = frame {
                            new.redefine_plane(&Sketch::on_plane(String::new(), f, k));
                        }
                        frame.is_some()
                    } else if let Some((_, face)) = self.face_hover {
                        tied_note = !new.redefine_onto_face_at(face, self.face_pick_point.unwrap_or(face.origin));
                        true
                    } else {
                        false
                    };
                    if picked {
                        if self.redefine_sketch_plane(i, new) && tied_note {
                            self.notify("It's tied to the sketch's origin or axes, so it keeps its coordinates on the new face (edit the sketch to move it)".into());
                        }
                    } else {
                        self.redefine = Some(i); // missed: still waiting
                    }
                } else if let Some(op) = self.origin_hover {
                    self.start_sketch(op);
                } else if let Some(k) = self.cplane_hover {
                    self.start_sketch_on_plane(k);
                } else if let Some((_, face)) = self.face_hover {
                    self.start_sketch_on_face(face);
                }
            }
            if esc && !typing {
                self.face_pick = false;
                self.plane_pick = false;
                self.redefine = None;
            }
        }

        crate::mark("Mirror / patterns: the active selectio");
        // --- Mirror / patterns: the active selection field takes clicks in the
        // view: bodies (click again to drop one), or a mirror plane ---
        self.copy_body_hover = None;
        let field = self.copy_dialog.as_ref().filter(|d| !d.picking_axis).and_then(|d| d.picking).filter(|_| self.active.is_none() && !self.face_pick);
        if let Some(field) = field {
            self.ensure_model(Slot::Committed);
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                match field {
                    copy_dialog::Field::Plane => {
                        let psize = origin::size_for(self.camera.distance);
                        let origin_hit = origin::pick(&ray, psize, |_| true).map(|(op, t)| (Some(op), None, t));
                        let custom_hit = self.pick_cplane(&ray, psize).map(|(k, t)| (None, Some(k), t));
                        if let Some((op, k, _)) = [origin_hit, custom_hit].into_iter().flatten().min_by(|a, b| a.2.total_cmp(&b.2)) {
                            self.origin_hover = op;
                            self.cplane_hover = k;
                        }
                    }
                    copy_dialog::Field::Bodies => {
                        let model = self.model(Slot::Committed);
                        self.copy_body_hover = model
                            .bodies
                            .iter()
                            .filter(|b| !self.hidden_bodies.contains(&b.id))
                            .filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir).map(|t| (b.id, t)))
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(id, _)| id);
                    }
                }
            }
            if response.clicked() {
                let (op, k, body) = (self.origin_hover, self.cplane_hover, self.copy_body_hover);
                if let Some(d) = self.copy_dialog.as_mut() {
                    if let Some(op) = op {
                        (d.plane, d.mirror_custom, d.picking, d.plane_set) = (op, None, None, true);
                    } else if let Some(k) = k {
                        (d.mirror_custom, d.picking, d.plane_set) = (Some(k), None, true);
                    } else if let Some(id) = body {
                        if d.bodies.contains(&id) {
                            d.bodies.retain(|b| *b != id);
                        } else {
                            d.bodies.push(id);
                        }
                    }
                }
            }
            if esc && !typing {
                if let Some(d) = self.copy_dialog.as_mut() {
                    d.picking = None;
                }
            }
        }

        crate::mark("the 3D view: hover origin planes and f");
        // --- the 3D view: hover origin planes and faces; click selects a plane, right-click offers a sketch ---
        if !self.face_pick && self.active.is_none() {
            self.marquee = None;
            self.measure_input(&view, hover_px, &response, aspect, min, size);
            let free_view = self.measure.is_none()
                && self.dialog.is_none()
                && self.edge_dialog.is_none()
                && self.revolve_dialog.is_none()
                && self.copy_dialog.is_none()
                && self.plane_dialog.is_none()
                && self.hole_dialog.is_none()
                && self.thread_dialog.is_none()
                && self.shell_dialog.is_none()
                && self.offset_dialog.is_none()
                && self.combine_dialog.is_none()
                && self.move_dialog.is_none();
            if let (Some(px), true) = (hover_px, free_view) {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let face = self.pick_face(&ray);
                // A plane counts only where nothing of a body is in front of it
                // (or in it: a face lying in the plane covers it).
                let plane = origin::pick(&ray, origin::size_for(self.camera.distance), |p| self.origin_shown.plane(p)).filter(|&(_, t_plane)| {
                    let t_body = self
                        .model(Slot::Committed)
                        .bodies
                        .iter()
                        .filter(|b| !self.hidden_bodies.contains(&b.id))
                        .filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir))
                        .fold(f32::MAX, f32::min);
                    t_plane < t_body - origin::tie_tolerance(t_body.min(t_plane))
                });
                // A construction plane, the same way (nothing of a body in front of it).
                let psize = origin::size_for(self.camera.distance);
                let custom = self.pick_cplane(&ray, psize).filter(|&(_, t_plane)| {
                    let t_body = self
                        .model(Slot::Committed)
                        .bodies
                        .iter()
                        .filter(|b| !self.hidden_bodies.contains(&b.id))
                        .filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir))
                        .fold(f32::MAX, f32::min);
                    t_plane < t_body - origin::tie_tolerance(t_body.min(t_plane))
                });
                match (plane, custom) {
                    (Some((op, to)), Some((_, tc))) if to <= tc => self.origin_hover = Some(op),
                    (_, Some((k, _))) => self.cplane_hover = Some(k),
                    (Some((op, _)), None) => self.origin_hover = Some(op),
                    (None, None) => self.face_hover = face.map(|(_, b, f)| (b, f)),
                }
                if response.clicked() {
                    self.selected_plane = self.origin_hover;
                    self.selected_cplane = self.cplane_hover;
                }
                if response.secondary_clicked() {
                    self.ctx_plane = self.origin_hover;
                    self.ctx_cplane = self.cplane_hover;
                    self.ctx_face = self.face_hover;
                    if self.ctx_plane.is_some() {
                        self.selected_plane = self.ctx_plane;
                    }
                }
            }
        } else if self.active.is_some() {
            self.selected_plane = None;
            self.selected_cplane = None;
        }

        crate::mark("Hole: click sketch points, sketch circ");
        // --- Hole: click sketch points, sketch circles or faces (the face's
        // middle snaps) to add places; click one again to drop it ---
        self.hole_hover = None;
        let picking_holes = self.hole_dialog.as_ref().is_some_and(|d| d.picking) && self.active.is_none();
        if picking_holes {
            self.ensure_model(Slot::Committed);
            if let Some(px) = hover_px {
                self.hole_hover = self.hole_place_under(&view, px, aspect, min, size);
            }
            if let (true, Some((place, _))) = (response.clicked(), self.hole_hover) {
                if let Some(d) = self.hole_dialog.as_mut() {
                    d.toggle(place);
                }
                // Drilled into the body: flip a sketch hole that would drill away from it.
                self.aim_hole();
            }
            if esc && !typing {
                if let Some(d) = self.hole_dialog.as_mut() {
                    d.picking = false;
                }
            }
        } else if self.hole_dialog.as_ref().is_some_and(|d| !d.picking_target) && esc && !typing {
            self.cancel_hole(); // Esc ends the command, as in Fusion
        }

        crate::mark("Shell: click faces to open or bodies");
        // --- Shell: the Faces field takes flat faces, the Bodies field bodies ---
        self.shell_hover = None;
        if let (Some(field), true) = (self.shell_dialog.as_ref().and_then(|d| d.picking), self.active.is_none()) {
            self.ensure_model(Slot::Committed);
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                self.shell_hover = match field {
                    shell_dialog::Field::Faces => self.pick_face(&ray).map(|(t, _, face)| {
                        let q = (ray.origin + ray.dir * t).as_dvec3();
                        let face = sketch::FacePlane { follow: None, ..face };
                        (Some(model::PickedFace { face, anchor: [q.dot(face.u), q.dot(face.v)] }), None)
                    }),
                    shell_dialog::Field::Bodies => {
                        let model = self.model(Slot::Committed);
                        model
                            .bodies
                            .iter()
                            .filter(|b| !self.hidden_bodies.contains(&b.id))
                            .filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir).map(|t| (b.id, t)))
                            .min_by(|a, b| a.1.total_cmp(&b.1))
                            .map(|(id, _)| (None, Some(id)))
                    }
                };
            }
            if let (true, Some((face, body)), Some(d)) = (response.clicked(), self.shell_hover, self.shell_dialog.as_mut()) {
                if let Some(f) = face {
                    // The same face clicked again (anywhere on it) is kept again.
                    match d.picked_on(&f) {
                        Some(i) => {
                            d.shell.faces.remove(i);
                        }
                        None => d.toggle_face(f),
                    }
                }
                if let Some(id) = body {
                    d.toggle_body(id);
                }
            }
            if esc && !typing {
                if let Some(d) = self.shell_dialog.as_mut() {
                    d.picking = None;
                }
            }
        } else if self.shell_dialog.is_some() && esc && !typing {
            self.cancel_shell(); // Esc ends the command, as in Fusion
        }

        crate::mark("Move/Copy: click bodies");
        // --- Move/Copy: click the bodies to move ---
        self.move_hover = None;
        if self.move_dialog.as_ref().is_some_and(|d| d.picking) && self.active.is_none() {
            self.ensure_model(Slot::Committed);
            if let Some(px) = hover_px.filter(|_| near_arrow(hover_px, &self.arrows).is_none()) {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let model = self.model(Slot::Committed);
                self.move_hover = model
                    .bodies
                    .iter()
                    .filter(|b| !self.hidden_bodies.contains(&b.id))
                    .filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir).map(|t| (b.id, t)))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(id, _)| id);
            }
            if let (true, Some(id), Some(d)) = (response.clicked(), self.move_hover, self.move_dialog.as_mut()) {
                d.toggle_body(id);
            }
            if esc && !typing {
                if let Some(d) = self.move_dialog.as_mut() {
                    d.picking = false;
                }
            }
        } else if self.move_dialog.is_some() && esc && !typing {
            self.cancel_move();
        }

        crate::mark("Combine: click the target, then tool bodies");
        // --- Combine: click the target body, then the tool bodies ---
        self.combine_hover = None;
        if self.combine_dialog.as_ref().is_some_and(|d| d.picking.is_some()) && self.active.is_none() {
            self.ensure_model(Slot::Committed);
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let model = self.model(Slot::Committed);
                self.combine_hover = model
                    .bodies
                    .iter()
                    .filter(|b| !self.hidden_bodies.contains(&b.id))
                    .filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir).map(|t| (b.id, t)))
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(id, _)| id);
            }
            if let (true, Some(id), Some(d)) = (response.clicked(), self.combine_hover, self.combine_dialog.as_mut()) {
                d.click(id);
            }
            if esc && !typing {
                if let Some(d) = self.combine_dialog.as_mut() {
                    d.picking = None;
                }
            }
        } else if self.combine_dialog.is_some() && esc && !typing {
            self.cancel_combine();
        }

        crate::mark("Offset Face / Press Pull: click faces");
        // --- Offset Face / Press Pull: click flat faces to move; Press Pull also
        // takes a fillet (edit it) or a sketch profile (extrude it) ---
        self.offset_hover = None;
        if self.offset_dialog.as_ref().is_some_and(|d| d.picking) && self.active.is_none() {
            self.ensure_model(Slot::Committed);
            let (press_pull, nothing_yet) = self.offset_dialog.as_ref().map_or((false, true), |d| (d.press_pull && d.editing.is_none(), d.offset.faces.is_empty()));
            if let Some(px) = hover_px.filter(|_| near_arrow(hover_px, &self.arrows).is_none()) {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let face = self.pick_face(&ray).map(|(t, _, face)| {
                    let q = (ray.origin + ray.dir * t).as_dvec3();
                    let face = sketch::FacePlane { follow: None, ..face };
                    (t, PressPullHover::Face(model::PickedFace { face, anchor: [q.dot(face.u), q.dot(face.v)] }))
                });
                let mut best = face;
                if press_pull && nothing_yet {
                    // A rounded face a Fillet made, nearer than any flat face.
                    let model = self.model(Slot::Committed);
                    let hit = model
                        .bodies
                        .iter()
                        .filter(|b| !self.hidden_bodies.contains(&b.id))
                        .filter_map(|b| model::ray_hit_tri(&b.mesh, ray.origin, ray.dir).map(|(t, tri)| (t, tri, b)))
                        .min_by(|x, y| x.0.total_cmp(&y.0));
                    if let Some((t, tri, body)) = hit {
                        let f = body.tri_face.get(tri).copied().unwrap_or(u32::MAX) as usize;
                        if best.is_none_or(|(bt, _)| t < bt - 1e-4) {
                            best = kernel::fillet_for_face(&self.edge_features, body, f).map(|k| (t, PressPullHover::Fillet(k)));
                        }
                    }
                    // A sketch profile in front of (or on) the face.
                    for s in (0..self.sketches.len()).filter(|&s| self.sketch_visible(s)) {
                        let Some((t, p)) = self.plane_of(s).intersect_t(&ray) else { continue };
                        let Some(ri) = regions.get(s).and_then(|rs| rs.iter().enumerate().filter(|(_, r)| r.contains(p)).min_by(|a, b| a.1.area().total_cmp(&b.1.area())).map(|(ri, _)| ri)) else { continue };
                        if best.is_none_or(|(bt, _)| t <= bt + 1e-3) {
                            best = Some((t, PressPullHover::Profile(s, ri)));
                        }
                    }
                }
                self.offset_hover = best.map(|(_, h)| h);
            }
            if response.clicked() {
                match self.offset_hover {
                    Some(PressPullHover::Face(f)) => {
                        if let Some(d) = self.offset_dialog.as_mut() {
                            d.toggle_face(f);
                        }
                        self.focus_distance();
                    }
                    // Switch tools next frame, so this click isn't taken again
                    // by the tool it opens (a profile picked, then dropped).
                    other @ Some(_) => {
                        self.press_pull_go = other;
                        ui.ctx().request_repaint();
                    }
                    None => {}
                }
            }
            if esc && !typing {
                if let Some(d) = self.offset_dialog.as_mut() {
                    d.picking = false;
                }
            }
        } else if self.offset_dialog.is_some() && esc && !typing {
            self.cancel_offset();
        }

        crate::mark("Thread: click cylindrical faces (shaft");
        // --- Thread: click cylindrical faces (shafts or holes) ---
        self.thread_hover = None;
        if self.thread_dialog.as_ref().is_some_and(|d| d.picking) && self.active.is_none() {
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                self.thread_hover = self.cylinder_under(&ray);
            }
            if let (true, Some((c, at)), Some(d)) = (response.clicked(), self.thread_hover, self.thread_dialog.as_mut()) {
                d.toggle(c.as_ref(at), c.internal);
            }
            if esc && !typing {
                if let Some(d) = self.thread_dialog.as_mut() {
                    d.picking = false;
                }
            }
        } else if self.thread_dialog.as_ref().is_some_and(|d| !d.picking_any()) && esc && !typing {
            self.cancel_thread(); // Esc ends the command, as in Fusion
        }

        crate::mark("Extrude To Object / Hole To: click the");
        // --- Extrude To Object / Hole To: click the plane or flat face it runs up to ---
        let picking_target = self.dialog.as_ref().is_some_and(|d| d.picking_target) || self.hole_dialog.as_ref().is_some_and(|d| d.picking_target);
        if picking_target {
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let found = self.pick_face(&ray);
                self.face_pick_point = found.map(|(t, ..)| (ray.origin + ray.dir * t).as_dvec3());
                // As in Create Sketch: the nearer wins, a face wins a tie.
                let psize = origin::size_for(self.camera.distance);
                let origin_hit = origin::pick(&ray, psize, |_| true).map(|(op, t)| (Some(op), None, t));
                let custom_hit = self.pick_cplane(&ray, psize).map(|(k, t)| (None, Some(k), t));
                let plane = [origin_hit, custom_hit].into_iter().flatten().min_by(|a, b| a.2.total_cmp(&b.2));
                match (plane, found) {
                    (Some((op, k, tp)), f) if f.is_none_or(|(tf, ..)| tp < tf - origin::tie_tolerance(tf)) => {
                        self.origin_hover = op;
                        self.cplane_hover = k;
                    }
                    (_, f) => self.face_hover = f.map(|(_, b, face)| (b, face)),
                }
            }
            if response.clicked() {
                let target = match (self.origin_hover, self.cplane_hover, self.face_hover) {
                    (Some(op), ..) => Some(model::ExtrudeTarget::Origin(op)),
                    (_, Some(k), _) => Some(model::ExtrudeTarget::Plane(k)),
                    (.., Some((_, face))) => {
                        // Kept as picked; the kernel finds the face again every build.
                        let q = self.face_pick_point.unwrap_or(face.origin);
                        Some(model::ExtrudeTarget::Face { face: sketch::FacePlane { follow: None, ..face }, anchor: [q.dot(face.u), q.dot(face.v)] })
                    }
                    _ => None,
                };
                if let (Some(t), Some(d)) = (target, self.dialog.as_mut()) {
                    (d.target, d.picking_target) = (Some(t), false);
                }
                if let (Some(t), Some(d)) = (target, self.hole_dialog.as_mut()) {
                    (d.hole.target, d.picking_target) = (Some(t), false);
                }
            }
            if esc && !typing {
                if let Some(d) = self.dialog.as_mut() {
                    d.picking_target = false;
                }
                if let Some(d) = self.hole_dialog.as_mut() {
                    d.picking_target = false;
                }
            }
        }

        crate::mark("extrude / revolve input: pick profiles");
        // --- extrude / revolve input: pick profiles (and a revolve axis) ---
        let mut hover_profile: Option<(usize, usize)> = None;
        self.axis_hover = None;
        let picking_axis = self.revolve_dialog.as_ref().is_some_and(|d| d.picking_axis);
        if picking_axis {
            // A line in the profile's sketch (or any sketch, before a profile is picked).
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let want = self.revolve_dialog.as_ref().and_then(|d| d.sketch);
                let tol = SNAP_PX as f64 * mm_per_px;
                for s in 0..self.sketches.len() {
                    if want.is_some_and(|w| w != s) {
                        continue;
                    }
                    let Some(p) = self.plane_of(s).intersect(&ray) else { continue };
                    let sk = &self.sketches[s];
                    let line = sk.entities.iter().enumerate().find(|(_, e)| match **e {
                        Entity::Line { a, b } => sketch::dist_to_segment(p, sk.points[a], sk.points[b]) <= tol,
                        _ => false,
                    });
                    if let Some((e, _)) = line {
                        self.axis_hover = Some((s, e));
                        break;
                    }
                }
            }
            if response.clicked() {
                if let (Some((s, e)), Some(d)) = (self.axis_hover, self.revolve_dialog.as_mut()) {
                    if d.sketch.is_none() {
                        d.sketch = Some(s);
                    }
                    d.axis = RevolveAxis::Line(e);
                    d.picking_axis = false;
                }
                self.focus_distance();
            }
            if esc && !typing {
                if let Some(d) = self.revolve_dialog.as_mut() {
                    d.picking_axis = false;
                }
            }
        } else if picking_target {
            // The object it runs up to is being picked (above).
        } else if self.profile_pick().is_some() {
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let mut best: Option<(f32, f64)> = None;
                for s in (0..self.sketches.len()).filter(|&s| self.pickable_sketch(s)) {
                    let Some((t, p)) = self.plane_of(s).intersect_t(&ray) else { continue };
                    for (ri, r) in regions.get(s).into_iter().flatten().enumerate() {
                        if !r.contains(p) {
                            continue;
                        }
                        let area = r.area();
                        let better = match best {
                            None => true,
                            Some((bt, ba)) => t < bt - 1e-3 || ((t - bt).abs() <= 1e-3 && area < ba),
                        };
                        if better {
                            best = Some((t, area));
                            hover_profile = Some((s, ri));
                        }
                    }
                }
            }
            if response.clicked() && near_arrow(hover_px, &self.arrows).is_none() {
                self.focus_distance();
                if let (Some((s, ri)), Some((sketch, profiles))) = (hover_profile, self.profile_pick_mut()) {
                    if *sketch != Some(s) {
                        *sketch = Some(s);
                        *profiles = vec![ri];
                    } else if let Some(pos) = profiles.iter().position(|&p| p == ri) {
                        profiles.remove(pos);
                    } else {
                        profiles.push(ri);
                    }
                }
            }
            if esc && !typing {
                self.cancel_extrude();
                self.cancel_revolve();
            }
        } else if self.axis_pick {
            // --- construction axis: click a straight edge ---
            self.ensure_model(Slot::Committed);
            self.edge_hover = None;
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                self.edge_hover = self.edge_under(&view, Slot::Committed, px, &ray, true);
            }
            if let (true, Some(r)) = (response.clicked(), self.edge_hover) {
                self.add_axis(r);
                self.edge_hover = None;
            }
            if esc && !typing {
                self.axis_pick = false;
            }
        } else if self.angle_pick || self.plane_dialog.as_ref().is_some_and(|d| d.picking_line()) {
            // --- Plane at Angle: click a sketch line, an axis or a straight edge ---
            self.ensure_model(Slot::Committed);
            self.edge_hover = None;
            self.hinge_hover = None;
            let reach = origin::size_for(self.camera.distance) as f64;
            if let Some(px) = hover_px {
                // Lines and axes by how near they pass on screen.
                let mut lines: Vec<(model::Hinge, DVec3, DVec3, DVec3)> = model::WorldAxis::ALL
                    .into_iter()
                    .map(|a| (model::Hinge::World(a), -a.dir() * reach, a.dir() * reach, model::Hinge::world_normal(a)))
                    .collect();
                for (k, a) in self.model(Slot::Committed).axes.iter().enumerate() {
                    if let (Some((o, d)), false) = (a, self.axis_hidden.contains(&k)) {
                        let n0 = self.axes.get(k).map_or(DVec3::Z, |af| af.edge.faces[0]);
                        lines.push((model::Hinge::Axis(k), *o - *d * reach * 1.5, *o + *d * reach * 1.5, n0));
                    }
                }
                for s in (0..self.sketches.len()).filter(|&s| self.sketch_visible(s)) {
                    let (sk, plane) = (&self.sketches[s], self.plane_of(s));
                    for (e, ent) in sk.entities.iter().enumerate() {
                        if let Entity::Line { a, b } = *ent {
                            let (p, q) = (plane.to_world(sk.points[a]).as_dvec3(), plane.to_world(sk.points[b]).as_dvec3());
                            lines.push((model::Hinge::SketchLine { sketch: s, line: e }, p, q, plane.normal().as_dvec3()));
                        }
                    }
                }
                self.hinge_hover = lines
                    .into_iter()
                    .filter_map(|l| {
                        let d = dist_to_segment_px(px, view.project(l.1.as_vec3())?, view.project(l.2.as_vec3())?);
                        (d < SNAP_PX).then_some((l, d))
                    })
                    .min_by(|x, y| x.1.total_cmp(&y.1))
                    .map(|(l, _)| l);
                if self.hinge_hover.is_none() {
                    let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                    self.edge_hover = self.edge_under(&view, Slot::Committed, px, &ray, true);
                    if let Some(r) = self.edge_hover {
                        let ends = self.model(Slot::Committed).bodies.iter().flat_map(|b| &b.edges).find(|e| e.at.mid.distance(r.mid) < 1e-6).and_then(|e| {
                            Some((e.points.first()?.as_dvec3(), e.points.last()?.as_dvec3()))
                        });
                        if let Some((p, q)) = ends {
                            self.hinge_hover = Some((model::Hinge::Edge(r), p, q, r.faces[0]));
                        }
                    }
                }
            }
            if let (true, Some((hinge, p, q))) = (response.clicked(), self.hinge_hover.map(|h| (h.0, h.1, h.2))) {
                let n0 = self.hinge_hover.map_or(DVec3::Z, |h| h.3);
                // The plane's origin where the kernel puts it: an origin axis
                // through the origin, anything else at its middle.
                let o = if matches!(hinge, model::Hinge::World(_)) { DVec3::ZERO } else { (p + q) * 0.5 };
                let size = p.distance(q).max(reach * 0.5);
                match self.plane_dialog.as_mut().filter(|d| d.picking_line()) {
                    // The dialog's Line field: swap the line, keep the angle.
                    Some(d) => {
                        d.set_line(hinge, o, q - p, n0, size);
                        self.set_distance_text();
                    }
                    None => {
                        self.cancel_all_dialogs();
                        self.open_plane_dialog(PlaneDialog::at_angle(hinge, o, q - p, n0, size));
                    }
                }
                self.hinge_hover = None;
            }
            if esc && !typing {
                self.angle_pick = false;
                self.hinge_hover = None;
                if let Some(a) = self.plane_dialog.as_mut().and_then(|d| d.angle.as_mut()) {
                    a.picking_line = false;
                }
            }
        } else if self.edge_dialog.is_some() {
            // --- fillet / chamfer input: pick edges on the model before this feature ---
            self.ensure_model(Slot::Base);
            self.edge_hover = None;
            self.edge_group_hover.clear();
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                self.edge_hover = self.edge_under(&view, Slot::Base, px, &ray, false);
                let chain = self.edge_dialog.as_ref().is_some_and(|d| d.tangent_chain);
                let base = self.model(Slot::Base);
                // An edge (with its tangent chain), or else a face's edges (Fusion).
                let group: Vec<EdgeRef> = match self.edge_hover {
                    Some(r) => base
                        .bodies
                        .iter()
                        .find_map(|b| {
                            let i = b.edges.iter().position(|e| e.at.mid.distance(r.mid) < 1e-6)?;
                            Some(if chain { b.tangent_chain(i).into_iter().map(|j| b.edges[j].at).collect() } else { vec![r] })
                        })
                        .unwrap_or_else(|| vec![r]),
                    None => base
                        .bodies
                        .iter()
                        .filter(|b| !self.hidden_bodies.contains(&b.id))
                        .filter_map(|b| model::ray_hit_tri(&b.mesh, ray.origin, ray.dir).map(|(t, tri)| (t, tri, b)))
                        .min_by(|x, y| x.0.total_cmp(&y.0))
                        .map(|(_, tri, b)| b.face_edges(b.tri_face.get(tri).copied().unwrap_or(0) as usize).into_iter().map(|j| b.edges[j].at).collect())
                        .unwrap_or_default(),
                };
                self.edge_group_hover = group;
            }
            if response.clicked() && !self.edge_group_hover.is_empty() {
                // Remember if an edge lies on the far end of an extrude/revolve.
                let caps = &self.model(Slot::Base).caps;
                let group: Vec<EdgeRef> = self
                    .edge_group_hover
                    .iter()
                    .map(|&r| EdgeRef { follow: kernel::follow_for_edge(caps, r.mid, r.dir, FOLLOW_TOL), ..r })
                    .collect();
                if let Some(d) = self.edge_dialog.as_mut() {
                    let has = |r: &EdgeRef| d.edges.iter().any(|e| e.mid.distance(r.mid) < 1e-6);
                    // Clicking picked edges again drops them; else the group is added.
                    if group.iter().all(has) {
                        d.edges.retain(|e| !group.iter().any(|r| r.mid.distance(e.mid) < 1e-6));
                    } else {
                        let new: Vec<EdgeRef> = group.into_iter().filter(|r| !has(r)).collect();
                        d.edges.extend(new);
                    }
                }
                self.focus_distance();
            }
            if esc && !typing {
                self.cancel_edge_dialog();
            }
        } else if !sketching && key_e && !typing && regions.iter().any(|r| !r.is_empty()) {
            self.open_extrude(None);
        } else if !sketching && key_n && !typing {
            self.new_sketch_tool();
        } else if !sketching && key_f && !typing && self.body_count > 0 {
            self.open_edge_dialog(None, EdgeKind::Fillet);
        } else if !sketching && key_h && !typing && self.body_count > 0 {
            self.open_hole(None);
        } else if !sketching && key_m && !typing && self.body_count > 0 {
            self.open_move(None);
        } else if !sketching && key_q && !typing {
            self.open_offset(None, true);
        } else if !sketching && key_i && !typing {
            self.measure = if self.measure.is_some() { None } else { Some(measure::Measure::default()) };
        }

        crate::mark("sketch input");
        // --- sketch input ---
        self.refresh_sketch_pattern();
        let mut cursor: Option<Anchor> = None; // drawing target (snapped)
        let mut hovered: Option<Sel> = None; // select-tool hover
        // Measuring inside a sketch: clicks go to Measure, not the sketch tools.
        if self.active.is_some() && self.measure.is_some() {
            self.ensure_model(Slot::Committed);
            self.measure_input(&view, hover_px, &response, aspect, min, size);
        }
        if let Some(i) = self.active.filter(|_| self.measure.is_none()) {
            let plane = self.plane_of(i);
            let raw = |px: Vec2| plane.intersect(&self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size)));
            let raw_hover = hover_px.and_then(raw);
            self.sketch_cursor = raw_hover.or(self.sketch_cursor);
            let raw_press = ui.input(|i| i.pointer.press_origin()).map(to_vec).and_then(raw);
            // Over a dimension: its label — or near one of its lines, when no
            // sketch geometry is under the cursor (a line behind a dimension
            // line is picked, not the dimension).
            let geometry_under = raw_hover.and_then(|p| self.sketches[i].hit(p, SNAP_PX as f64 * mm_per_px)).filter(|h| !matches!(h, Sel::Axis(_)));
            let over_text = hover_px.and_then(|px| self.dim_labels.iter().find(|(_, r, _)| r.contains(to_pos(px))).map(|(ci, ..)| *ci));
            let over_label = over_text.or_else(|| {
                let px = hover_px.filter(|_| geometry_under.is_none())?;
                self.dim_labels.iter().find(|(_, _, segs)| segs.iter().any(|s| dist_to_segment_px(px, s[0], s[1]) < 5.0)).map(|(ci, ..)| *ci)
            });
            self.dim_hover = over_label;

            {
                let sk = &self.sketches[i];
                let tool = self.editor.tool;
                // Tools that pick existing geometry get a hover target…
                if matches!(
                    tool,
                    Tool::Select
                        | Tool::Constrain(_)
                        | Tool::Circle2Tangent
                        | Tool::Circle3Tangent
                        | Tool::SketchFillet
                        | Tool::SketchChamfer
                        | Tool::Trim
                        | Tool::Break
                        | Tool::Extend
                        | Tool::Offset
                        | Tool::Mirror
                ) && over_label.is_none()
                {
                    hovered = raw_hover.and_then(|p| sk.hit(p, SNAP_PX as f64 * mm_per_px));
                    if tool.tangent_lines() > 0 {
                        hovered = hovered.filter(|h| matches!(h, Sel::Entity(e) if sk.is_line(*e)));
                    }
                    if tool == Tool::Mirror {
                        hovered = hovered.filter(|h| matches!(h, Sel::Entity(e) if sk.is_line(*e)));
                    }
                    let spline = |e: usize| matches!(sk.entities.get(e), Some(Entity::Spline { .. }));
                    if tool == Tool::Extend {
                        hovered = hovered.filter(|h| matches!(h, Sel::Entity(e) if sk.is_line(*e) || spline(*e)));
                    }
                    // Trim and Break take lines, circles, arcs and splines.
                    if matches!(tool, Tool::Trim | Tool::Break) {
                        hovered = hovered.filter(|h| matches!(h, Sel::Entity(e) if sk.is_line(*e) || sk.is_round(*e) || spline(*e)));
                    }
                    // Only Select and the constraint tools pick the sketch's axes.
                    if !matches!(tool, Tool::Select | Tool::Constrain(_)) {
                        hovered = hovered.filter(|h| !matches!(h, Sel::Axis(_)));
                    }
                    if tool == Tool::Offset {
                        hovered = hovered.filter(|h| matches!(h, Sel::Entity(e) if !matches!(sk.entities.get(*e), Some(Entity::Point { .. }))));
                    }
                    if matches!(tool, Tool::SketchFillet | Tool::SketchChamfer) {
                        // Corners where exactly two lines meet, or lines to pick in pairs.
                        hovered = hovered.filter(|h| match h {
                            Sel::Point(p) => sk.corner_lines(*p).is_some(),
                            // Or a line: pick two lines instead of their corner.
                            Sel::Entity(e) => sk.is_line(*e),
                            Sel::Axis(_) => false,
                        });
                    }
                }
                // Trim / Extend: show what a click on the hovered line would do.
                self.cut_preview = match (tool, hovered, raw_hover) {
                    (Tool::Trim | Tool::Break, Some(Sel::Entity(e)), Some(p)) => sk.trim_piece(e, p).map(|piece| (tool, piece)),
                    (Tool::Extend, Some(Sel::Entity(e)), Some(p)) => sk.extend_preview(e, p).map(|(a, b)| (tool, sketch::TrimPiece::Line(a, b))),
                    _ => None,
                };
                // Offset: the copy follows the cursor until the second click.
                self.offset_preview = None;
                if let (Tool::Offset, [Sel::Entity(e)], Some(p)) = (tool, self.editor.selection.as_slice(), raw_hover) {
                    let mut trial = sk.clone();
                    if trial.offset_chain(*e, p, None).is_ok() {
                        self.offset_preview = Some(trial);
                    }
                }
                // …drawing tools a snapped cursor.
                self.line_up = None;
                self.mid_guide = None;
                self.break_marks = match (tool, hovered, raw_hover) {
                    (Tool::Break, Some(Sel::Entity(e)), Some(p)) => sk.break_points(e, p),
                    _ => Vec::new(),
                };
                if matches!(tool, Tool::Select | Tool::Constrain(_) | Tool::SketchFillet | Tool::SketchChamfer | Tool::Trim | Tool::Break | Tool::Extend | Tool::Offset | Tool::Mirror | Tool::Project) {
                } else if let Some(px) = hover_px {
                    let nearest = sk
                        .points
                        .iter()
                        .enumerate()
                        .filter_map(|(id, &p)| Some((id, view.project(plane.to_world(p))?.distance(px))))
                        .filter(|&(_, d)| d < SNAP_PX)
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    // Near a line (not at a point)? The new point will stay on it.
                    let on_line = raw_hover.and_then(|p| {
                        let tol = SNAP_PX as f64 * mm_per_px;
                        sk.entities
                            .iter()
                            .enumerate()
                            .filter_map(|(e, ent)| match *ent {
                                Entity::Line { a, b } => {
                                    let (a, b) = (sk.points[a], sk.points[b]);
                                    let ab = b - a;
                                    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-30)).clamp(0.0, 1.0);
                                    let q = a + ab * t;
                                    let d = q.distance(p);
                                    // Near the middle: snap right onto it (the new point becomes its midpoint).
                                    let mid = a.midpoint(b);
                                    let q = if q.distance(mid) <= tol * 1.5 { mid } else { q };
                                    (d <= tol).then_some((e, q, d))
                                }
                                _ => None,
                            })
                            .min_by(|x, y| x.2.total_cmp(&y.2))
                    });
                    // Centre circle, edge near where it would touch a line: snap
                    // there exactly, so the circle is made tangent to it.
                    let tangent_spot = match (tool, self.editor.start, on_line) {
                        (Tool::Circle, Some(start), Some((e, q, _))) if sk.is_valid_anchor(start) => sk
                            .line_foot(e, sk.pos(start))
                            .filter(|f| f.distance(q) <= 1.5 * SNAP_PX as f64 * mm_per_px)
                            .map(|f| Anchor::OnLine(e, f)),
                        _ => None,
                    };
                    let arc3_third = tool == Tool::Arc3Point && self.editor.second.is_some();
                    cursor = match (nearest, on_line) {
                        (Some((id, _)), _) => Some(Anchor::Point(id)),
                        _ if tangent_spot.is_some() => tangent_spot,
                        // Tangent circles sit where the cursor really is, never on the grid.
                        _ if tool.tangent_lines() > 0 => raw_hover.map(Anchor::Free),
                        // 3-point arc, third click: the raw cursor (never the grid),
                        // with the centre snaps below.
                        _ if arc3_third => raw_hover.map(Anchor::Free),
                        (None, Some((e, q, _))) => Some(Anchor::OnLine(e, q)),
                        (None, None) => raw_hover.map(|p| {
                            // Near an origin axis: locked onto it (nearer axis wins).
                            let tol = SNAP_PX as f64 * mm_per_px;
                            match (p.y.abs() <= tol, p.x.abs() <= tol) {
                                (true, false) => Anchor::OnAxis(0, DVec2::new(if self.grid_snap { grid.snap(p).x } else { p.x }, 0.0)),
                                (false, true) => Anchor::OnAxis(1, DVec2::new(0.0, if self.grid_snap { grid.snap(p).y } else { p.y })),
                                (true, true) if p.y.abs() <= p.x.abs() => Anchor::OnAxis(0, DVec2::new(p.x, 0.0)),
                                (true, true) => Anchor::OnAxis(1, DVec2::new(0.0, p.y)),
                                _ => Anchor::Free(if self.grid_snap { grid.snap(p) } else { p }),
                            }
                        }),
                    };
                    // Polygons: the direction from the centre locks to 0°/90°/180°/270°
                    // when close, unless Ctrl is held.
                    if let (true, Some(start), Some(Anchor::Free(p))) = (
                        matches!(tool, Tool::PolygonCircumscribed | Tool::PolygonInscribed | Tool::PolygonEdge)
                            && self.editor.second.is_none()
                            && !ui.input(|inp| inp.modifiers.ctrl),
                        self.editor.start.filter(|a| sk.is_valid_anchor(*a)),
                        cursor,
                    ) {
                        cursor = Some(Anchor::Free(sketch::snap_quarter_turn(sk.pos(start), p, 3.0)));
                    }
                    if let (true, Some(Anchor::Free(p))) = (arc3_third, cursor) {
                        let g = self.grid_snap.then_some(grid);
                        cursor = Some(Anchor::Free(self.editor.arc3_snap(sk, p, g, tol_px(mm_per_px)).0));
                    }
                    // Line with a typed length: the end stays that far from the start,
                    // toward the cursor (onto horizontal/vertical when close); the
                    // click sets the angle.
                    if self.editor.tool != Tool::Line || self.editor.start.is_none() {
                        self.line_length = None;
                    }
                    if let (Some(l), Some(start), Some(c)) = (self.line_length, self.editor.start.filter(|a| sk.is_valid_anchor(*a)), cursor) {
                        let a = sk.pos(start);
                        if let Some(mut dir) = (sk.pos(c) - a).try_normalize() {
                            let deg = dir.y.atan2(dir.x).to_degrees().rem_euclid(90.0);
                            if deg <= sketch::ALIGN_SNAP_DEG || deg >= 90.0 - sketch::ALIGN_SNAP_DEG {
                                dir = if dir.x.abs() >= dir.y.abs() { DVec2::new(dir.x.signum(), 0.0) } else { DVec2::new(0.0, dir.y.signum()) };
                            }
                            cursor = Some(Anchor::Free(a + dir * l));
                        }
                    }
                    // Rectangle with typed sizes: the far corner stays that size from
                    // the start, on the cursor's side of it.
                    let sizing = matches!(self.editor.tool, Tool::Rectangle | Tool::RectangleCenter) && self.editor.start.is_some_and(|a| sk.is_valid_anchor(a));
                    self.rect_boxes = None; // set again where the preview is drawn
                    if !sizing {
                        self.rect_input = None;
                        self.rect_place = false;
                    }
                    if let (Some(r), Some(start), Some(c)) = (&self.rect_input, self.editor.start.filter(|_| sizing), cursor) {
                        let sizes = r.sizes(self.units);
                        if sizes.iter().any(|s| s.is_some()) {
                            let centered = self.editor.tool == Tool::RectangleCenter;
                            cursor = Some(Anchor::Free(rect_input::sized_corner(sk.pos(start), sk.pos(c), centered, sizes)));
                        }
                    }
                    // Spline: near its first point (three or more placed), the cursor
                    // snaps onto it — a click there closes the shape.
                    if let (Tool::Spline, Some(&first), Some(px)) = (tool, self.editor.spline.first(), hover_px) {
                        let near = sk.is_valid_anchor(first)
                            && self.editor.spline.len() >= 3
                            && view.project(plane.to_world(sk.pos(first))).is_some_and(|q| q.distance(px) < SNAP_PX);
                        if near {
                            cursor = Some(first);
                        }
                    }
                    let closing_spline = tool == Tool::Spline && self.editor.spline.len() >= 3 && cursor == self.editor.spline.first().copied();
                    // Line middles the cursor rests on are acquired (the two newest),
                    // as in Fusion; the perpendicular through each then guides it.
                    let tol = SNAP_PX as f64 * mm_per_px;
                    if self.acquired.0 != Some(i) || self.acquired.1.iter().any(|&e| !sk.is_line(e)) {
                        self.acquired = (Some(i), Vec::new());
                    }
                    if let Some(e) = raw_hover.and_then(|p| sk.middle_at(p, tol)) {
                        let got = &mut self.acquired.1;
                        if !got.contains(&e) {
                            got.push(e);
                            if got.len() > 2 {
                                got.remove(0);
                            }
                        }
                    }
                    let inferring = !(tool.tangent_lines() > 0 || arc3_third || self.line_length.is_some() || closing_spline);
                    if let (true, Some(Anchor::Free(_)), Some(raw)) = (inferring, cursor, raw_hover) {
                        if let Some((q, from)) = sk.on_middle_guides(&self.acquired.1, raw, tol) {
                            cursor = Some(Anchor::Free(q));
                            self.mid_guide = Some((q, from));
                        }
                    }
                    // Inference: lined up with a line's middle or a circle's centre
                    // (dashed lines show it; both at once lands where they cross).
                    if let (true, None, Some(Anchor::Free(g)), Some(raw)) = (inferring, &self.mid_guide, cursor, raw_hover) {
                        let (q, above, beside) = sk.line_up(raw, SNAP_PX as f64 * mm_per_px);
                        if above.is_some() || beside.is_some() {
                            let q = DVec2::new(if above.is_some() { q.x } else { g.x }, if beside.is_some() { q.y } else { g.y });
                            cursor = Some(Anchor::Free(q));
                            self.line_up = Some((above, beside, q));
                        }
                    }
                }
            }

            // Ctrl+G flips grid snapping at any moment, even with a number box open.
            if ui.input(|i| i.key_pressed(egui::Key::G) && i.modifiers.command) {
                self.grid_snap = !self.grid_snap;
                self.notify(if self.grid_snap { "Grid snap on" } else { "Grid snap off" }.into());
            }
            if !typing {
                if key_l {
                    self.editor.set_tool(Tool::Line);
                }
                if key_r {
                    self.editor.set_tool(self.rect_tool);
                }
                if key_c {
                    self.editor.set_tool(self.circle_tool);
                }
                if key_g {
                    self.grid_snap = !self.grid_snap;
                }
                if key_x {
                    self.toggle_construction();
                }
                if key_trim {
                    self.editor.set_tool(Tool::Trim);
                }
                if key_extend {
                    self.editor.set_tool(Tool::Extend);
                }
                if key_offset {
                    self.editor.set_tool(Tool::Offset);
                }
                if key_p && self.body_count > 0 && self.sketch_pattern.is_none() {
                    self.editor.set_tool(Tool::Project);
                }
                if Tool::POLYGONS.contains(&self.editor.tool) {
                    let step = ui.input(|i| i.key_pressed(egui::Key::ArrowUp) as i32 - i.key_pressed(egui::Key::ArrowDown) as i32);
                    if step != 0 {
                        let n = (self.editor.sides() as i32 + step).clamp(3, Editor::MAX_SIDES as i32);
                        self.editor.polygon_sides = n as usize;
                    }
                }
                if key_d {
                    self.constraint_button("Dimension");
                }
                if del {
                    self.delete_selection();
                }
                if esc {
                    if matches!(self.editor.tool, Tool::Spline | Tool::ControlSpline) && self.editor.spline.len() >= 2 {
                        self.finish_spline(); // Esc keeps the spline drawn so far
                    } else if self.editor.busy() {
                        self.editor.cancel();
                    } else if matches!(self.editor.tool, Tool::Constrain(_)) && !self.editor.selection.is_empty() {
                        self.editor.selection.clear(); // drop the picks, stay in the tool
                    } else if self.editor.tool != Tool::Select {
                        self.editor.set_tool(Tool::Select);
                    } else {
                        self.editor.selection.clear();
                    }
                }
                if undo && self.sketch_pattern.is_none() {
                    self.undo_last();
                }
            }
            if response.secondary_clicked() {
                if self.editor.tool == Tool::Select {
                    // The Select tool shows a menu for what's under the cursor
                    // (see the context menu at the end of the viewport).
                    match (self.dim_hover.filter(|&ci| ci < self.sketches[i].constraints.len()), hovered) {
                        (Some(ci), _) => {
                            if !self.dim_selection.contains(&ci) {
                                self.dim_selection = vec![ci];
                                self.editor.selection.clear();
                            }
                        }
                        (None, Some(sel)) => {
                            if !self.editor.selection.contains(&sel) {
                                self.editor.selection = vec![sel];
                                self.dim_selection.clear();
                            }
                        }
                        (None, None) => {}
                    }
                } else {
                    match self.dim_hover.filter(|&ci| ci < self.sketches[i].constraints.len()) {
                        // Right-click a dimension: driving ⇄ driven (glyphs have no value).
                        Some(ci) if self.sketches[i].constraints[ci].value().is_some() => self.toggle_driven(ci),
                        Some(_) => {}
                        None => self.editor.cancel(),
                    }
                }
            }

            // Undo or delete above may have removed points that the cursor, the
            // tool or a drag still refer to. Drop those references before using them.
            {
                let sk = &self.sketches[i];
                if cursor.is_some_and(|a| !sk.is_valid_anchor(a)) {
                    cursor = None;
                }
                if self.editor.start.is_some_and(|a| !sk.is_valid_anchor(a)) {
                    self.editor.start = None;
                }
                if hovered.is_some_and(|s| !sk.is_valid_sel(s)) {
                    hovered = None;
                }
                if self.drag.is_some_and(|p| p >= sk.points.len()) {
                    self.drag = None;
                }
            }

            // Project: the body edge under the cursor.
            self.project_hover = None;
            self.project_body_hover = None;
            if let (Tool::Project, Some(px)) = (self.editor.tool, hover_px) {
                self.ensure_model(Slot::Committed);
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                self.project_hover = self.edge_under(&view, Slot::Committed, px, &ray, false);
                if self.project_hover.is_none() {
                    // Not on an edge: on a body, which takes all of its edges.
                    self.project_body_hover = self
                        .model(Slot::Committed)
                        .bodies
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| !self.hidden_bodies.contains(&b.id))
                        .filter_map(|(k, b)| model::ray_hit(&b.mesh, ray.origin, ray.dir).map(|t| (k, t)))
                        .min_by(|a, b| a.1.total_cmp(&b.1))
                        .map(|(k, _)| k);
                }
            }

            // A pattern dialog is open: Enter applies, Esc cancels, and (circular) a click on a point sets the centre.
            if self.sketch_pattern.is_some() {
                let enter = ui.input(|inp| inp.key_pressed(egui::Key::Enter));
                if enter && !typing {
                    self.commit_sketch_pattern();
                } else if esc && !typing {
                    self.cancel_sketch_pattern();
                } else if let Some(d) = self.sketch_pattern.as_mut() {
                    if d.picking_center && response.clicked() {
                        if let Some(Sel::Point(p)) = raw_hover.and_then(|h| self.sketches[i].hit(h, SNAP_PX as f64 * mm_per_px)) {
                            d.center = self.sketches[i].points[p];
                            d.picking_center = false;
                        }
                    }
                }
            }
            match self.editor.tool {
                _ if self.sketch_pattern.is_some() => {}
                Tool::Select => {
                    // A selected spline's tangent handles: the end under the cursor.
                    let handle_end = |app: &Self, q: DVec2| -> Option<(sketch::PointId, f64, usize)> {
                        let sk = &app.sketches[i];
                        let tol = SNAP_PX as f64 * mm_per_px;
                        sk.shown_handles(&app.editor.selection).into_iter().find_map(|(s, p, fwd, _)| {
                            let back = sk.points[p] * 2.0 - fwd;
                            if fwd.distance(q) <= tol {
                                Some((p, 1.0, s))
                            } else if back.distance(q) <= tol {
                                Some((p, -1.0, s))
                            } else {
                                None
                            }
                        })
                    };
                    if response.double_clicked() {
                        // Double-click a handle's end: back to the curve's own way.
                        if let Some((p, _, _)) = raw_hover.and_then(|q| handle_end(self, q)) {
                            self.undo.push(self.sketches[i].clone());
                            self.sketches[i].set_spline_tangent(p, None);
                        }
                    }
                    if response.drag_started_by(egui::PointerButton::Primary) {
                        if let Some(h) = raw_press.and_then(|q| handle_end(self, q)) {
                            self.undo.push(self.sketches[i].clone());
                            self.handle_drag = Some(h);
                        }
                    }
                    if let (Some((p, sign, s)), Some(now)) = (self.handle_drag, raw_hover) {
                        if response.dragged_by(egui::PointerButton::Primary) {
                            let sk = &mut self.sketches[i];
                            let base = sk.handle_base(s).max(1e-9);
                            sk.set_spline_tangent(p, Some((now - sk.points[p]) * sign / base));
                        }
                    }
                    if self.handle_drag.is_some() && response.drag_stopped() {
                        self.handle_drag = None;
                    }
                    if self.handle_drag.is_some() {
                        // (The handle has the drag.)
                    } else if response.drag_started_by(egui::PointerButton::Primary) {
                        let sk = &self.sketches[i];
                        if let (Some(ci), Some(q)) = (over_label.filter(|&ci| sk.constraints.get(ci).is_some_and(|c| c.value().is_some())), raw_press) {
                            // Grab a dimension's label to move it out of the way.
                            self.label_drag = Some((ci, q, sk.label_shift(ci)));
                        } else if let Some(Sel::Point(p)) = raw_press.and_then(|q| sk.hit(q, SNAP_PX as f64 * mm_per_px)) {
                            if !sk.is_fixed(p) {
                                self.undo.push(sk.clone());
                                self.drag = Some(p);
                            }
                        } else if let (Some(Sel::Entity(e)), Some(q)) = (raw_press.and_then(|q| sk.hit(q, SNAP_PX as f64 * mm_per_px)), raw_press) {
                            // Dragging a line moves it with the cursor, keeping its constraints.
                            if let Some(&Entity::Line { a, b }) = sk.entities.get(e) {
                                if sk.points[a].distance(sk.points[b]) > 0.0 {
                                    self.undo.push(sk.clone());
                                    self.drag_line = Some((e, q, [sk.points[a], sk.points[b]]));
                                }
                            }
                        } else if let Some(q) = raw_press.filter(|q| sk.hit(*q, SNAP_PX as f64 * mm_per_px).is_none()) {
                            // Dragging from empty space: drag-select.
                            self.marquee = Some((q, q));
                        }
                    }
                    if let (Some((start, _)), Some(now)) = (self.marquee, raw_hover) {
                        if response.dragged_by(egui::PointerButton::Primary) {
                            self.marquee = Some((start, now));
                        }
                    }
                    if response.drag_stopped() {
                        if let Some((start, end)) = self.marquee.take() {
                            // Whatever the box touches, and every point in it; Shift adds.
                            let mut picked = self.sketches[i].select_in_rect(start, end, true);
                            picked.extend(self.sketches[i].points_in_rect(start, end));
                            if !ui.input(|inp| inp.modifiers.shift) {
                                self.editor.selection.clear();
                                self.dim_selection.clear();
                            }
                            for s in picked {
                                if !self.editor.selection.contains(&s) {
                                    self.editor.selection.push(s);
                                }
                            }
                        }
                    }
                    if let (Some((ci, start, before)), Some(now)) = (self.label_drag, raw_hover) {
                        if response.dragged_by(egui::PointerButton::Primary) {
                            self.sketches[i].set_label_shift(ci, before + (now - start));
                        }
                    }
                    if self.label_drag.is_some() && response.drag_stopped() {
                        self.label_drag = None;
                    }
                    if let (Some(p), Some(target)) = (self.drag, raw_hover) {
                        if response.dragged_by(egui::PointerButton::Primary) {
                            let target = if self.grid_snap { grid.snap(target) } else { target };
                            // Follows the cursor as far as the constraints allow.
                            let _ = solver::drag(&mut self.sketches[i], p, target);
                        }
                    }
                    if let (Some((e, start, from)), Some(now)) = (self.drag_line, raw_hover) {
                        if response.dragged_by(egui::PointerButton::Primary) && e < self.sketches[i].entities.len() {
                            let mut shift = now - start;
                            if self.grid_snap {
                                // The line's first end lands on the grid (the grid follows the zoom).
                                shift = grid.snap(from[0] + shift) - from[0];
                            }
                            let _ = solver::drag_line(&mut self.sketches[i], e, from, shift);
                        }
                    }
                    if self.drag_line.is_some() && response.drag_stopped() {
                        self.drag_line = None;
                        self.dof = solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof);
                        if self.undo.last() == Some(&self.sketches[i]) {
                            self.undo.pop();
                        }
                    }
                    if self.drag.is_some() && response.drag_stopped() {
                        self.drag = None;
                        self.dof = solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof);
                        if self.undo.last() == Some(&self.sketches[i]) {
                            self.undo.pop(); // drag didn't move anything
                        }
                    }
                    // Clicking dimensions: one click selects (Delete removes it),
                    // a double-click edits its value. On a touchpad (no easy
                    // dragging) a click on an already-selected dimension picks
                    // its label up; it follows the cursor until the next click.
                    let edit_value = |app: &mut Self, ci: usize| {
                        if app.sketches[i].is_driven(ci) {
                            app.notify("Reference dimension: right-click it to make it drive".into());
                            return;
                        }
                        if app.sketches[i].constraints.get(ci).is_some_and(|c| c.value().is_some()) {
                            app.float = Some(FloatEdit {
                                target: EditTarget::Dimension(ci),
                                text: app.value_text(i, ci),
                                focus: true,
                                keep_text: false,
                            });
                        }
                    };
                    if response.double_clicked() && over_label.is_some() {
                        // (Put back a label the first click of a touchpad
                        // double-click picked up.)
                        if let (true, Some((ci, _, before))) = (self.label_carry, self.label_drag) {
                            self.sketches[i].set_label_shift(ci, before);
                        }
                        self.label_carry = false;
                        self.label_drag = None;
                        if let Some(ci) = over_label {
                            edit_value(self, ci);
                        }
                    } else if response.double_clicked() && hovered.is_some() && !self.label_carry {
                        // Double-click: the whole chain joined to what's under the cursor.
                        if let Some(h) = hovered {
                            self.editor.selection = self.sketches[i].connected(h);
                            self.dim_selection.clear();
                        }
                    } else if self.label_carry {
                        if let (Some((ci, start, before)), Some(now)) = (self.label_drag, raw_hover) {
                            self.sketches[i].set_label_shift(ci, before + (now - start));
                        }
                        if esc {
                            if let Some((ci, _, before)) = self.label_drag {
                                self.sketches[i].set_label_shift(ci, before);
                            }
                        }
                        if response.clicked() || esc {
                            self.label_carry = false;
                            self.label_drag = None;
                        }
                    } else if response.clicked() {
                        match over_label {
                            Some(ci)
                                if self.touchpad
                                    && self.dim_selection.contains(&ci)
                                    && self.sketches[i].constraints.get(ci).is_some_and(|c| c.value().is_some()) =>
                            {
                                if let Some(p) = raw_hover {
                                    self.label_drag = Some((ci, p, self.sketches[i].label_shift(ci)));
                                    self.label_carry = true;
                                }
                            }
                            Some(ci) => {
                                if let Some(k) = self.dim_selection.iter().position(|&c| c == ci) {
                                    self.dim_selection.remove(k);
                                } else {
                                    self.dim_selection.push(ci);
                                }
                            }
                            None => {
                                if hovered.is_none() {
                                    self.dim_selection.clear();
                                }
                                self.editor.select_click(hovered);
                            }
                        }
                    }
                }
                Tool::Constrain(label) => {
                    if label == "Dimension" {
                        // Picks so far → what would be dimensioned, with its label
                        // following the cursor until a click places it.
                        self.dim_preview = None;
                        if let (Some(c), Some(p)) = (self.dimension_candidate(), raw_hover) {
                            let sk = &self.sketches[i];
                            let hidden_painter = painter.with_clip_rect(egui::Rect::NOTHING);
                            let hidden = View { painter: &hidden_painter, ..view };
                            let centroid = sketch_centroid(&view, sk, &plane, to_vec(rect.center()));
                            let at_default = draw_dimension(&hidden, sk, &plane, &c, false, DVec2::ZERO, centroid, COL_DIM, self.units, false)
                                .and_then(|r| {
                                    let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(to_vec(r.center()), min, size));
                                    plane.intersect(&ray)
                                });
                            if let Some(d0) = at_default {
                                self.dim_preview = Some((c, p - d0));
                            }
                        }
                        if response.clicked() {
                            match (hovered, self.dim_preview) {
                                (Some(sel), _) => {
                                    // Another pick: a second line (angle) or point (distance).
                                    if !self.editor.selection.contains(&sel) {
                                        self.editor.selection.push(sel);
                                        if self.editor.selection.len() > 2 || self.dimension_candidate().is_none() && self.editor.selection.len() == 2 {
                                            self.editor.selection = vec![sel];
                                        }
                                    }
                                }
                                // An existing dimension's number: edit it (the tool stays on).
                                (None, _) if over_label.is_some() => {
                                    let sk = &self.sketches[i];
                                    if let Some(ci) = over_text.filter(|&ci| !sk.is_driven(ci) && sk.constraints.get(ci).is_some_and(|c| c.value().is_some())) {
                                        self.editor.selection.clear();
                                        self.float = Some(FloatEdit { target: EditTarget::Dimension(ci), text: self.value_text(i, ci), focus: true, keep_text: false });
                                    }
                                }
                                (None, Some((c, shift))) => self.place_dimension(c, shift, None),
                                (None, None) => self.editor.selection.clear(),
                            }
                        }
                        // Or just start typing the value: placed where the label is now.
                        if !typing && self.float.is_none() {
                            let typed: String = ui.input(|inp| {
                                inp.events
                                    .iter()
                                    .filter_map(|e| match e {
                                        egui::Event::Text(t) => Some(t.clone()),
                                        _ => None,
                                    })
                                    .collect()
                            });
                            if typed.chars().next().is_some_and(|ch| ch.is_ascii_digit() || ch == '.') {
                                let place = self.dim_preview.or_else(|| self.dimension_candidate().map(|c| (c, DVec2::ZERO)));
                                if let Some((c, shift)) = place {
                                    self.place_dimension(c, shift, Some(typed));
                                }
                            }
                        }
                    } else if response.clicked() {
                        if let Some(sel) = hovered {
                            self.constraint_pick(label, sel);
                        } else {
                            self.editor.selection.clear();
                        }
                    }
                }
                Tool::Project => {
                    if let Some(r) = self.project_hover.filter(|_| response.clicked()) {
                        self.project_edge_into(i, r);
                    } else if let Some(b) = self.project_body_hover.filter(|_| response.clicked()) {
                        self.project_body_into(i, b);
                    }
                }
                Tool::Mirror => {
                    if response.clicked() {
                        let tol = SNAP_PX as f64 * mm_per_px;
                        match (hovered, raw_hover) {
                            (Some(Sel::Entity(e)), _) => self.mirror_across(pattern::MirrorLine::Entity(e)),
                            // The origin axes work as mirror lines too.
                            (_, Some(p)) if p.y.abs() <= tol || p.x.abs() <= tol => {
                                self.mirror_across(pattern::MirrorLine::Axis(if p.y.abs() <= p.x.abs() { 0 } else { 1 }))
                            }
                            _ => self.notify("Click a line or an origin axis to mirror across".into()),
                        }
                    }
                }
                Tool::Trim => {
                    // Click the stretch of a line to remove.
                    if response.clicked() {
                        match (hovered, raw_hover) {
                            (Some(Sel::Entity(e)), Some(p)) => self.trim_at(e, p),
                            _ => self.notify("Click a line, circle or arc to trim it back to the nearest crossings".into()),
                        }
                    }
                }
                Tool::Break => {
                    if response.clicked() {
                        match (hovered, raw_hover) {
                            (Some(Sel::Entity(e)), Some(p)) => self.break_curve_at(e, p),
                            _ => self.notify("Click a line, circle, arc or spline where something crosses it".into()),
                        }
                    }
                }
                Tool::Extend => {
                    if response.clicked() {
                        match (hovered, raw_hover) {
                            (Some(Sel::Entity(e)), Some(p)) => self.extend_at(e, p),
                            _ => self.notify("Click a line near the end you want to extend".into()),
                        }
                    }
                }
                Tool::Offset => {
                    // Click the geometry, then click where the copy should go (the cursor shows it) — or type how far.
                    if self.editor.selection.len() == 1 && !typing && self.float.is_none() {
                        if let Some(typed) = typed_number(ui, !self.parameters.is_empty()) {
                            self.float = Some(FloatEdit { target: EditTarget::NewCircle, text: typed, focus: true, keep_text: true });
                        }
                    }
                    if response.clicked() {
                        match (self.editor.selection.first().copied(), hovered, raw_hover) {
                            (Some(Sel::Entity(e)), None, Some(p)) => self.offset_at(e, p),
                            (Some(Sel::Entity(e)), Some(Sel::Entity(h)), Some(p)) if h != e && !self.sketches[i].connected(Sel::Entity(e)).contains(&Sel::Entity(h)) => {
                                let _ = p;
                                self.editor.selection = vec![Sel::Entity(h)];
                            }
                            (Some(Sel::Entity(e)), Some(_), Some(p)) => self.offset_at(e, p),
                            (_, Some(Sel::Entity(h)), _) => self.editor.selection = vec![Sel::Entity(h)],
                            _ => self.notify("Click a line, arc or circle to offset".into()),
                        }
                    }
                }
                Tool::SketchFillet | Tool::SketchChamfer => {
                    // Click a corner, then type the size (Enter applies).
                    if response.clicked() && near_arrow(hover_px, &self.arrows).is_none() {
                        match hovered {
                            Some(Sel::Point(p)) => {
                                self.corner_first = None;
                                self.float = Some(FloatEdit {
                                    target: EditTarget::Corner(p),
                                    text: self.units.plain(self.corner_size),
                                    focus: true,
                                    keep_text: false,
                                });
                            }
                            Some(Sel::Entity(e)) => {
                                // Two lines that meet at a corner work like the corner.
                                let first = match self.editor.selection.as_slice() {
                                    [Sel::Entity(f)] if *f != e => Some(*f),
                                    _ => None,
                                };
                                match first.map(|f| self.sketches[i].corner_between(f, e)) {
                                    Some(Some(p)) => {
                                        self.editor.selection.clear();
                                        self.corner_first = first; // a chamfer's first distance is on the first line picked
                                        self.float = Some(FloatEdit {
                                            target: EditTarget::Corner(p),
                                            text: self.units.plain(self.corner_size),
                                            focus: true,
                                            keep_text: false,
                                        });
                                    }
                                    Some(None) => {
                                        self.editor.selection = vec![Sel::Entity(e)];
                                        self.notify("Those two lines don't meet in a corner of just two lines".into());
                                    }
                                    None => self.editor.selection = vec![Sel::Entity(e)],
                                }
                            }
                            _ => {
                                self.editor.selection.clear();
                                self.float = None;
                                self.notify("Click a corner, or two lines that meet".into());
                            }
                        }
                    }
                }
                Tool::Circle2Tangent | Tool::Circle3Tangent => {
                    if response.clicked() {
                        let lines_needed = self.editor.tool.tangent_lines();
                        if let Some(Sel::Entity(e)) = hovered {
                            self.editor.pick_tangent_line(&self.sketches[i], e);
                        }
                        let ready = self.editor.tangent_lines.len() == lines_needed;
                        // Once the lines are picked, the preview follows the cursor
                        // (which side / region it sits in); a click away from the
                        // lines places it.
                        let go = ready && hovered.is_none();
                        if let (true, Some(p)) = (go, raw_hover) {
                            let ed = &mut self.editor;
                            let mut why = None;
                            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| match ed.tangent_click(sk, p) {
                                Ok(cs) => {
                                    for c in cs {
                                        let mut t = sk.clone();
                                        if solver::add_constraints(&mut t, &[c]).is_ok() {
                                            *sk = t;
                                        }
                                    }
                                }
                                Err(e) => why = Some(e),
                            });
                            match why {
                                Some(e) => self.notify(format!("Can't make the circle: {e}")),
                                None => self.report(r),
                            }
                        }
                    }
                }
                Tool::Line
                | Tool::Rectangle
                | Tool::Rectangle3Point
                | Tool::RectangleCenter
                | Tool::Point
                | Tool::Circle
                | Tool::Circle2Point
                | Tool::Circle3Point
                | Tool::Arc3Point
                | Tool::ArcCenter
                | Tool::ArcTangent
                | Tool::Ellipse
                | Tool::Slot
                | Tool::PolygonCircumscribed
                | Tool::PolygonInscribed
                | Tool::PolygonEdge
                | Tool::Spline
                | Tool::ControlSpline => {
                    // Spline: Enter finishes it (as Esc does, keeping the points so far).
                    if matches!(self.editor.tool, Tool::Spline | Tool::ControlSpline) && !typing && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.finish_spline();
                    }
                    // The ✓ by the last point finishes the spline (Fusion's).
                    let drawing_spline = matches!(self.editor.tool, Tool::Spline | Tool::ControlSpline) && self.editor.spline.len() >= 2;
                    let on_check = drawing_spline && self.spline_check.zip(hover_px).is_some_and(|(r, px)| r.contains(to_pos(px)));
                    if on_check && response.clicked() && matches!(self.editor.tool, Tool::Spline | Tool::ControlSpline) {
                        self.finish_spline();
                        self.spline_check = None;
                    }
                    // Control Point Spline: ↑/↓ switch its degree between 3 and 5.
                    if self.editor.tool == Tool::ControlSpline && !typing {
                        let (up, down) = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown)));
                        if up {
                            self.editor.cp_degree = 5;
                        }
                        if down {
                            self.editor.cp_degree = 3;
                        }
                    }
                    // After the centre (circle), the first point (line) or the two centres (slot),
                    // typing a number sets the diameter / length / width.
                    let wants_number = (matches!(self.editor.tool, Tool::Circle | Tool::Line) && self.editor.start.is_some())
                        || (self.editor.tool == Tool::Slot && self.editor.second.is_some());
                    // Rectangle: typing (or Tab) after the first corner fills the width
                    // box (Tab: the height box).
                    let rect_sizing = matches!(self.editor.tool, Tool::Rectangle | Tool::RectangleCenter) && self.editor.start.is_some();
                    if rect_sizing && !typing && self.float.is_none() {
                        if let Some(typed) = typed_number(ui, !self.parameters.is_empty()) {
                            self.rect_input.get_or_insert_with(Default::default).type_text(typed);
                        } else if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab)) {
                            self.rect_input.get_or_insert_with(Default::default).tab();
                        }
                    }
                    if wants_number && !typing && self.float.is_none() {
                        if let Some(typed) = typed_number(ui, !self.parameters.is_empty()) {
                            if self.editor.tool == Tool::Line {
                                // The line takes the length from the first digit typed.
                                self.line_length = units::parse_length(&typed, self.units).filter(|v| *v > 0.0);
                            }
                            self.float = Some(FloatEdit { target: EditTarget::NewCircle, text: typed, focus: true, keep_text: true });
                        }
                    }
                    // Line: drag off the end just placed and it's a tangent arc
                    // instead (as in Fusion); lines carry on from the arc's end.
                    if self.editor.tool == Tool::Line && response.drag_started_by(egui::PointerButton::Primary) {
                        let sk = &self.sketches[i];
                        if let (Some(Anchor::Point(p)), Some(q)) = (self.editor.start, raw_press) {
                            if sk.tangent_from(p).is_some() && sk.points[p].distance(q) <= SNAP_PX as f64 * mm_per_px {
                                self.line_arc = Some(p);
                            }
                        }
                    }
                    if self.editor.tool != Tool::Line {
                        self.line_arc = None;
                    }
                    if let (Some(p), true) = (self.line_arc, response.drag_stopped()) {
                        self.line_arc = None;
                        if let Some(at) = cursor {
                            let mut arc_ed = Editor { tool: Tool::ArcTangent, start: Some(Anchor::Point(p)), construction: self.editor.construction, ..Default::default() };
                            let before = self.sketches[i].entities.len();
                            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
                                for c in arc_ed.click(sk, at) {
                                    let mut t = sk.clone();
                                    if solver::add_constraints(&mut t, &[c]).is_ok() {
                                        *sk = t;
                                    }
                                }
                            });
                            self.report(r);
                            let sk = &self.sketches[i];
                            if let Some(&Entity::Arc { a, b, .. }) = sk.entities.get(before) {
                                self.editor.start = Some(Anchor::Point(if a == p { b } else { a }));
                            }
                        }
                    }
                    if (response.clicked() && !on_check) || std::mem::take(&mut self.rect_place) {
                        if let Some(at) = cursor {
                            let ed = &mut self.editor;
                            let length = self.line_length.filter(|_| ed.tool == Tool::Line && ed.start.is_some());
                            let rect_typed = self.rect_input.take().filter(|_| matches!(ed.tool, Tool::Rectangle | Tool::RectangleCenter) && ed.start.is_some());
                            let rect_sizes = rect_typed.as_ref().map(|r| r.sizes(self.units));
                            let mut rect_first = None; // the rectangle's first line (its width)
                            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
                                let before = sk.entities.len();
                                // A line drawn nearly horizontal/vertical gets that constraint,
                                // unless it would over-constrain the sketch (then it's skipped).
                                for c in ed.click(sk, at) {
                                    let mut t = sk.clone();
                                    if solver::add_constraints(&mut t, &[c]).is_ok() {
                                        *sk = t;
                                    }
                                }
                                // Typed rectangle sizes: kept as its width / height dimensions.
                                if let (Some(sizes), Some(first)) = (rect_sizes, (before..sk.entities.len()).find(|&e| sk.is_line(e))) {
                                    rect_first = Some(first);
                                    for c in rect_input::size_dimensions(first, sizes) {
                                        let mut t = sk.clone();
                                        if solver::add_constraints(&mut t, &[c]).is_ok() {
                                            *sk = t;
                                        }
                                    }
                                }
                                // A typed length: the new line keeps it as a dimension.
                                if let (Some(l), true) = (length, sk.entities.len() > before && sk.is_line(sk.entities.len() - 1)) {
                                    let mut t = sk.clone();
                                    if solver::add_constraints(&mut t, &[Constraint::Length(sk.entities.len() - 1, l)]).is_ok() {
                                        *sk = t;
                                    }
                                }
                            });
                            self.report(r);
                            // Sizes typed as expressions: their dimensions follow the parameters.
                            if let (Some(l), Some(t)) = (length, self.line_length_text.take()) {
                                self.link_new_dimension(l, &t);
                            }
                            if let (Some(r), Some(first), Some(sk)) = (rect_typed, rect_first, self.sketches.get_mut(i)) {
                                // Width on the first line, height on the next.
                                for k in 0..2 {
                                    let ci = sk.constraints.iter().position(|c| matches!(c, Constraint::Length(e, _) if *e == first + k));
                                    if let (Some(ci), true) = (ci, units::is_expression(&r.text[k])) {
                                        sk.set_dim_expr(ci, Some(r.text[k].trim().to_string()));
                                    }
                                }
                            }
                            self.line_length = None;
                        }
                    }
                }
            }
        }

        // Input above can add or remove sketches (a click on a face starts a new
        // one; finishing an empty sketch drops it), so refresh everything that
        // was computed per sketch before drawing with it.
        let regions: Vec<Vec<Region>> = self.sketches.iter().map(find_regions).collect();

        crate::mark("draw: background, grid, axes");
        // --- draw: background, grid, axes ---
        painter.rect_filled(rect, 0.0, Color32::from_gray(38));
        let rel = self.camera.target - dplane.origin;
        let centre = DVec2::new(rel.dot(dplane.u) as f64, rel.dot(dplane.v) as f64);
        let extent = (GRID_HALF_LINES as f64 * grid.step + centre.abs().max_element()) as f32;
        // Grid and axes as coloured world segments. With the GPU they're drawn
        // with depth, so a part hides the grid under it from above but not from
        // below; the CPU fallback just paints them first.
        // The grid belongs to sketches (as in Fusion); the 3D view shows the
        // origin instead: short axes and three small planes.
        let sketching = self.active.is_some();
        let origin_size = origin::size_for(self.camera.distance);
        // Create Sketch shows the whole origin, whatever the browser says.
        // Picking an axis or a mirror plane shows the whole origin too.
        let picking_axis = self.copy_dialog.as_ref().is_some_and(|d| d.picking_axis || d.mode == CopyMode::Mirror)
            || self.dialog.as_ref().is_some_and(|d| d.picking_target || d.extent == model::Extent::ToObject)
            || self.angle_pick
            || self.plane_dialog.as_ref().is_some_and(|d| d.picking_line());
        let shown = if self.face_pick || picking_axis { origin::Shown::default() } else { self.origin_shown };
        let show_origin = !sketching && (shown.all || self.face_pick || picking_axis);
        let mirror_plane = self.copy_dialog.as_ref().filter(|d| d.mode == CopyMode::Mirror && d.plane_set).map(|d| (d.mirror_custom, d.plane));
        let extrude_target = self.dialog.as_ref().filter(|d| d.extent == model::Extent::ToObject).and_then(|d| d.target);
        let mut grid_segments: Vec<(Vec3, Vec3, [f32; 4])> = if sketching {
            sketch::grid_lines(&dplane, centre, grid, GRID_HALF_LINES)
                .into_iter()
                .map(|(a, b, major)| {
                    let g = if major { 85.0 / 255.0 } else { 60.0 / 255.0 };
                    (a, b, [g, g, g, 1.0])
                })
                .collect()
        } else {
            Vec::new()
        };
        let axes: Vec<(Vec3, Vec3, Vec3)> = if sketching {
            vec![(dplane.u * -extent, dplane.u * extent, dplane.u), (dplane.v * -extent, dplane.v * extent, dplane.v)]
        } else if show_origin {
            let reach = origin_size * 1.0;
            [Vec3::X, Vec3::Y, Vec3::Z].into_iter().enumerate().filter(|&(k, _)| shown.axis(k)).map(|(_, a)| (Vec3::ZERO, a * reach, a)).collect()
        } else {
            Vec::new()
        };
        for &(a, b, axis) in &axes {
            let c = rgb(axis_color(axis));
            grid_segments.push((a, b, [c[0], c[1], c[2], 1.0]));
        }
        // Construction axes: a long dashed-looking line through the picked edge.
        if !sketching {
            self.ensure_model(Slot::Committed);
            let reach = origin_size * 1.5;
            let hot = [1.0, 0.85, 0.3, 1.0];
            for (k, a) in self.model(Slot::Committed).axes.iter().enumerate() {
                let (Some((o, d)), false) = (a, self.axis_hidden.contains(&k)) else { continue };
                grid_segments.push(((o - d * reach as f64).as_vec3(), (o + d * reach as f64).as_vec3(), hot));
            }
        }
        if !self.gpu {
            if sketching {
                view.grid(&dplane, centre, grid);
            }
            for &(a, b, axis) in &axes {
                view.line(a, b, Stroke::new(2.0, axis_color(axis)));
            }
        }

        // The sketch's axes light up when hovered or picked (for constraints).
        if let Some(i) = self.active {
            let plane = self.plane_of(i);
            let ext = extent as f64;
            for k in 0..2u8 {
                let sel = Sel::Axis(k);
                let col = if self.editor.selection.contains(&sel) {
                    COL_SELECT
                } else if hovered == Some(sel) {
                    COL_HOVER
                } else {
                    continue;
                };
                let dir = if k == 0 { DVec2::X } else { DVec2::Y };
                view.line(plane.to_world(-dir * ext), plane.to_world(dir * ext), Stroke::new(3.0, col));
            }
        }

        crate::mark("draw: the origin planes (3D view)");
        // --- draw: the origin planes (3D view) ---
        let mut origin_glass: Vec<([Vec3; 3], [f32; 4])> = Vec::new();
        let mut origin_outlines: Vec<(Vec3, Vec3, [f32; 4])> = Vec::new();
        if show_origin {
            const GLASS: [f32; 3] = [1.0, 0.667, 0.235];
            let mut glass: Vec<([Vec3; 3], [f32; 4])> = Vec::new();
            let mut outlines: Vec<(Vec3, Vec3, [f32; 4])> = Vec::new();
            for op in OriginPlane::ALL.into_iter().filter(|&p| shown.plane(p)) {
                let hot = self.origin_hover == Some(op);
                let chosen = self.selected_plane == Some(op) || mirror_plane == Some((None, op)) || extrude_target == Some(model::ExtrudeTarget::Origin(op));
                let alpha = if hot { 150 } else if chosen { 120 } else if self.face_pick { 80 } else { 55 };
                let p = op.plane();
                let s = origin_size;
                let (lo, hi) = origin::span(s);
                let q = [p.u * lo + p.v * lo, p.u * hi + p.v * lo, p.u * hi + p.v * hi, p.u * lo + p.v * hi];
                if self.gpu {
                    let c = [GLASS[0], GLASS[1], GLASS[2], alpha as f32 / 255.0];
                    glass.push(([q[0], q[1], q[2]], c));
                    glass.push(([q[0], q[2], q[3]], c));
                    let lc = [1.0, 0.745, 0.353, if hot || chosen { 1.0 } else { 0.55 }];
                    for k in 0..4 {
                        outlines.push((q[k], q[(k + 1) % 4], lc));
                    }
                } else {
                    let fill = Color32::from_rgba_unmultiplied(255, 170, 60, alpha);
                    let edge = Stroke::new(
                        if hot || chosen { 2.0 } else { 1.0 },
                        Color32::from_rgba_unmultiplied(255, 190, 90, if hot || chosen { 255 } else { 140 }),
                    );
                    let pts: Vec<Option<Vec2>> = q.iter().map(|&v| view.project(v)).collect();
                    if let [Some(a), Some(b), Some(c), Some(d)] = pts[..] {
                        let mut mesh = egui::Mesh::default();
                        for v in [a, b, c, d] {
                            mesh.colored_vertex(to_pos(v), fill);
                        }
                        mesh.add_triangle(0, 1, 2);
                        mesh.add_triangle(0, 2, 3);
                        painter.add(egui::Shape::mesh(mesh));
                        for (v, w) in [(a, b), (b, c), (c, d), (d, a)] {
                            painter.line_segment([to_pos(v), to_pos(w)], edge);
                        }
                    }
                }
            }
            origin_glass = glass;
            origin_outlines = outlines;
        }

        crate::mark("draw: construction planes (and the one");
        // --- draw: construction planes (and the one being set up) ---
        if !sketching {
            self.ensure_model(Slot::Committed);
            let editing = self.plane_dialog.as_ref().and_then(|d| d.editing);
            let mut items: Vec<(sketch::FacePlane, bool, f32, [f64; 4])> = Vec::new();
            for k in 0..self.planes.len() {
                if self.plane_hidden.contains(&k) || editing == Some(k) {
                    continue;
                }
                if let Some(f) = self.model(Slot::Committed).planes.get(k).copied().flatten() {
                    let alpha = if self.cplane_hover == Some(k) {
                        150.0
                    } else if mirror_plane.is_some_and(|(c, _)| c == Some(k)) || extrude_target == Some(model::ExtrudeTarget::Plane(k)) {
                        120.0
                    } else if self.face_pick {
                        80.0
                    } else {
                        55.0
                    };
                    items.push((f, self.planes[k].on_origin, alpha / 255.0, self.planes[k].extent));
                }
            }
            if let Some(d) = &self.plane_dialog {
                items.push((d.frame(), d.on_origin, 120.0 / 255.0, d.extent));
            }
            for (f, on_origin, alpha, extent) in items {
                let (lo, hi) = self.plane_rect(extent, &f, on_origin, origin_size);
                let at = |x: f64, y: f64| (f.origin + f.u * x + f.v * y).as_vec3();
                let q = [at(lo.x, lo.y), at(hi.x, lo.y), at(hi.x, hi.y), at(lo.x, hi.y)];
                if self.gpu {
                    // Orange like the origin planes.
                    let hot = alpha >= 0.45;
                    let c = [1.0, 0.667, 0.235, alpha];
                    origin_glass.push(([q[0], q[1], q[2]], c));
                    origin_glass.push(([q[0], q[2], q[3]], c));
                    for k in 0..4 {
                        origin_outlines.push((q[k], q[(k + 1) % 4], [1.0, 0.745, 0.353, if hot { 1.0 } else { 0.55 }]));
                    }
                } else {
                    let pts: Vec<Option<Vec2>> = q.iter().map(|&v| view.project(v)).collect();
                    if let [Some(a), Some(b), Some(c), Some(d)] = pts[..] {
                        painter.add(egui::Shape::convex_polygon(
                            vec![to_pos(a), to_pos(b), to_pos(c), to_pos(d)],
                            Color32::from_rgba_unmultiplied(255, 170, 60, (alpha * 255.0) as u8),
                            Stroke::new(if alpha >= 0.45 { 2.0 } else { 1.0 }, Color32::from_rgba_unmultiplied(255, 190, 90, if alpha >= 0.45 { 255 } else { 140 })),
                        ));
                    }
                }
            }
        }

        crate::mark("draw: bodies (from the kernel; with an");
        // --- draw: bodies (from the kernel; with an open dialog previewed) ---
        let previewing = self.dialog.is_some() || self.edge_dialog.is_some() || self.revolve_dialog.is_some() || self.copy_dialog.is_some() || self.plane_dialog.is_some() || self.hole_dialog.is_some() || self.thread_dialog.is_some() || self.shell_dialog.is_some() || self.offset_dialog.is_some() || self.combine_dialog.is_some() || self.move_dialog.is_some();
        // The committed design always drives the timeline errors and body list.
        self.ensure_model(Slot::Committed);
        self.adopt_linked_sketches();
        self.feature_errors = self.model(Slot::Committed).errors.clone();
        self.edge_feature_errors = self.model(Slot::Committed).edge_errors.clone();
        self.sketch_errors = self.model(Slot::Committed).sketch_errors.clone();
        self.revolve_errors = self.model(Slot::Committed).revolve_errors.clone();
        self.copy_errors = self.model(Slot::Committed).copy_errors.clone();
        self.remove_errors = self.model(Slot::Committed).remove_errors.clone();
        self.axis_errors = self.model(Slot::Committed).axis_errors.clone();
        self.plane_errors = self.model(Slot::Committed).plane_errors.clone();
        self.hole_errors = self.model(Slot::Committed).hole_errors.clone();
        self.thread_errors = self.model(Slot::Committed).thread_errors.clone();
        self.shell_errors = self.model(Slot::Committed).shell_errors.clone();
        self.offset_face_errors = self.model(Slot::Committed).offset_face_errors.clone();
        self.combine_errors = self.model(Slot::Committed).combine_errors.clone();
        self.move_errors = self.model(Slot::Committed).move_errors.clone();
        self.body_count = self.model(Slot::Committed).bodies.len();
        // With a body in the scene the origin gets out of the way (Create Sketch
        // and the browser's Origin box bring it back).
        if self.body_count > 0 && self.prev_body_count == 0 {
            self.origin_shown.all = false;
        }
        self.prev_body_count = self.body_count;
        self.body_ids = self.model(Slot::Committed).bodies.iter().map(|b| b.id).collect();
        let slot = if previewing { Slot::Preview } else { Slot::Committed };
        self.ensure_model(slot);
        // The preview's first build still running: show the design as it is meanwhile.
        let slot = if self.ready(slot) { slot } else { Slot::Committed };
        let model = &self.caches[slot as usize].as_ref().expect("built above").model;
        self.dialog_error = if let Some(d) = &self.shell_dialog {
            let k = d.editing.unwrap_or(self.shells.len());
            if d.shell.faces.is_empty() && d.shell.bodies.is_empty() { None } else { model.shell_errors.get(k).cloned().flatten() }
        } else if let Some(d) = &self.move_dialog {
            let k = d.editing.unwrap_or(self.moves.len());
            if d.feature.bodies.is_empty() { None } else { model.move_errors.get(k).cloned().flatten() }
        } else if let Some(d) = &self.combine_dialog {
            let k = d.editing.unwrap_or(self.combines.len());
            if d.ready() { model.combine_errors.get(k).cloned().flatten() } else { None }
        } else if let Some(d) = &self.offset_dialog {
            let k = d.editing.unwrap_or(self.offset_faces.len());
            if d.offset.faces.is_empty() { None } else { model.offset_face_errors.get(k).cloned().flatten() }
        } else if let Some(d) = &self.thread_dialog {
            let k = d.editing.unwrap_or(self.thread_features.len());
            if d.thread.faces.is_empty() { None } else { model.thread_errors.get(k).cloned().flatten() }
        } else if let Some(d) = &self.hole_dialog {
            let k = d.editing.unwrap_or(self.holes.len());
            if d.hole.places.is_empty() { None } else { model.hole_errors.get(k).cloned().flatten() }
        } else if let Some(d) = &self.dialog {
            let k = d.editing.unwrap_or(self.extrudes.len());
            model.errors.get(k).cloned().flatten()
        } else if let Some(d) = &self.edge_dialog {
            let k = d.editing.unwrap_or(self.edge_features.len());
            model.edge_errors.get(k).cloned().flatten()
        } else if let Some(d) = &self.revolve_dialog {
            let k = d.editing.unwrap_or(self.revolves.len());
            model.revolve_errors.get(k).cloned().flatten()
        } else if let Some(d) = &self.copy_dialog {
            let k = d.editing.unwrap_or(self.copies.len());
            model.copy_errors.get(k).cloned().flatten()
        } else if let Some(d) = &self.plane_dialog {
            let k = d.editing.unwrap_or(self.planes.len());
            model.plane_errors.get(k).cloned().flatten()
        } else {
            None
        };
        let body_color = if previewing { COL_BODY_PREVIEW } else { COL_BODY };

        let hidden: HashSet<usize> = if previewing {
            HashSet::new()
        } else {
            model.bodies.iter().enumerate().filter(|(_, b)| self.hidden_bodies.contains(&b.id)).map(|(k, _)| k).collect()
        };
        let meshes: Vec<(&Mesh, Color32)> = model
            .bodies
            .iter()
            .enumerate()
            .filter(|(k, _)| !hidden.contains(k))
            .map(|(_, b)| {
                let c = if previewing { body_color } else { self.body_color(b.id) };
                // Bodies selected in the browser (or picked for a mirror / pattern)
                // are tinted with the selection colour; the one under the cursor lighter.
                let picked = self.copy_dialog.as_ref().is_some_and(|d| d.bodies.contains(&b.id)) || self.shell_dialog.as_ref().is_some_and(|d| d.shell.bodies.contains(&b.id))
                    || self.combine_dialog.as_ref().is_some_and(|d| d.combine.target == Some(b.id) || d.combine.tools.contains(&b.id))
                    || self.move_dialog.as_ref().is_some_and(|d| d.feature.bodies.contains(&b.id));
                let hovered = self.copy_body_hover == Some(b.id) || self.shell_hover.and_then(|h| h.1) == Some(b.id) || self.combine_hover == Some(b.id) || self.move_hover == Some(b.id);
                let c = if hovered && !picked {
                    let mix = |x: u8, y: u8| ((x as u16 * 3 + y as u16) / 4) as u8;
                    Color32::from_rgb(mix(c.r(), COL_HOVER.r()), mix(c.g(), COL_HOVER.g()), mix(c.b(), COL_HOVER.b()))
                } else if self.selected_bodies.contains(&b.id) || picked {
                    let mix = |x: u8, y: u8| ((x as u16 + y as u16) / 2) as u8;
                    Color32::from_rgb(mix(c.r(), COL_SELECT.r()), mix(c.g(), COL_SELECT.g()), mix(c.b(), COL_SELECT.b()))
                } else {
                    c
                };
                (&b.mesh, c)
            })
            .collect();
        if self.gpu {
            let list: Vec<(&Mesh, [f32; 3])> = meshes.iter().map(|(m, c)| (*m, rgb(*c))).collect();
            let mut scene = gpu::Scene::build(&list, &self.camera, aspect);
            scene.add_lines(&grid_segments, &self.camera);
            scene.add_lines(&origin_outlines, &self.camera);
            // Shown threads on the bodies: hidden where the part is in front.
            if self.hole_dialog.is_none() && self.thread_dialog.is_none() {
                scene.add_lines(&self.thread_helices(slot), &self.camera);
            }
            scene.add_glass(&origin_glass, &self.camera);
            painter.add(egui::Shape::Callback(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                SceneCallback { scene, size_points: [rect.width(), rect.height()] },
            )));
        } else {
            view.solids(&meshes, &self.camera);
        }

        crate::mark("draw: threads that are only shown, whi");
        // --- draw: threads that are only shown, while a Hole or Thread window
        // is open: on top (the rest of the time they're drawn with the bodies) ---
        if !self.gpu || self.hole_dialog.is_some() || self.thread_dialog.is_some() {
            let stroke = Stroke::new(1.2, Color32::from_rgb(230, 190, 90));
            for (a, b, _) in self.thread_helices(slot) {
                view.line(a, b, stroke);
            }
        }

        crate::mark("draw: the cylinder a Thread would go o");
        // --- draw: the cylinder a Thread would go on, and those picked ---
        if let Some(d) = &self.thread_dialog {
            let ring = |c: &model::CylinderRef, len: f64, stroke: Stroke| {
                let u = model::zero_normal(c.axis, DVec3::Z);
                let v = c.axis.cross(u);
                for z in [0.0, len] {
                    let pts: Vec<Vec3> = (0..=48)
                        .map(|k| {
                            let a = k as f64 / 48.0 * std::f64::consts::TAU;
                            (c.axis_point + c.axis * z + (u * a.cos() + v * a.sin()) * c.radius).as_vec3()
                        })
                        .collect();
                    for w in pts.windows(2) {
                        view.line(w[0], w[1], stroke);
                    }
                }
            };
            if let Some((c, at)) = self.thread_hover {
                ring(&c.as_ref(at), c.length, Stroke::new(3.0, COL_HOVER));
            }
            for f in &d.thread.faces {
                ring(f, f.length, Stroke::new(2.5, COL_SELECT));
            }
            if d.picking {
                painter.text(
                    rect.center_top() + egui::vec2(0.0, 40.0),
                    egui::Align2::CENTER_TOP,
                    "Click a shaft or the inside of a hole to thread it   (Esc when done)",
                    egui::FontId::proportional(15.0),
                    Color32::from_gray(220),
                );
            }
        }

        crate::mark("draw: hole places (where a click could");
        // --- draw: hole places (where a click could go, the one under the
        // cursor, and those picked) ---
        if self.hole_dialog.as_ref().is_some_and(|d| d.picking) {
            for p in self.hole_spots() {
                if let Some(q) = view.project(p.as_vec3()) {
                    painter.circle_stroke(to_pos(q), 3.5, Stroke::new(1.5, Color32::from_rgba_unmultiplied(235, 235, 235, 200)));
                }
            }
        }
        if let Some(d) = &self.hole_dialog {
            for place in &d.hole.places {
                if let Some(q) = self.hole_place_world(place).and_then(|(p, _)| view.project(p.as_vec3())) {
                    painter.circle_stroke(to_pos(q), 5.0, Stroke::new(2.0, COL_SELECT));
                }
            }
            if let Some((place, at)) = self.hole_hover {
                if let Some(q) = view.project(at.as_vec3()) {
                    let picked = d.hole.places.contains(&place);
                    painter.circle_stroke(to_pos(q), 7.0, Stroke::new(2.0, if picked { COL_SELECT } else { COL_HOVER }));
                    let label = match place {
                        model::HolePlace::SketchCircle { .. } => "circle (its size)",
                        model::HolePlace::SketchPoint { .. } => "point",
                        model::HolePlace::Face { centre: true, .. } => "centre of face",
                        model::HolePlace::Face { .. } => "on face",
                    };
                    painter.text(to_pos(q) + egui::vec2(10.0, -10.0), egui::Align2::LEFT_BOTTOM, label, egui::FontId::proportional(13.0), Color32::from_gray(225));
                }
            }
            if d.picking {
                painter.text(
                    rect.center_top() + egui::vec2(0.0, 40.0),
                    egui::Align2::CENTER_TOP,
                    "Click sketch points, circles or faces for the holes (click again to drop one)   (Esc when done)",
                    egui::FontId::proportional(15.0),
                    Color32::from_gray(220),
                );
            }
        }

        crate::mark("draw: faces a Shell opens");
        // --- draw: faces a Shell opens (and the one under the cursor) ---
        if let Some(d) = &self.shell_dialog {
            let model = self.model(Slot::Committed);
            let hover = self.shell_hover.and_then(|h| h.0);
            let faces = d.shell.faces.iter().map(|f| (f, Color32::from_rgba_unmultiplied(230, 90, 70, 120))).chain(hover.iter().map(|f| (f, profile_fill(110))));
            for (f, fill) in faces {
                let at = f.face.origin + f.face.u * f.anchor[0] + f.face.v * f.anchor[1];
                for body in &model.bodies {
                    // Only the face the click was on (not every face in that plane).
                    let Some(k) = body.face_at(&f.face, at) else { continue };
                    let mut overlay = egui::Mesh::default();
                    for [a, b, c] in body.face_tris(k) {
                        if let (Some(pa), Some(pb), Some(pc)) = (view.project(a.as_vec3()), view.project(b.as_vec3()), view.project(c.as_vec3())) {
                            let base = overlay.vertices.len() as u32;
                            for q in [pa, pb, pc] {
                                overlay.colored_vertex(to_pos(q), fill);
                            }
                            overlay.add_triangle(base, base + 1, base + 2);
                        }
                    }
                    painter.add(egui::Shape::mesh(overlay));
                }
            }
            if let Some(field) = d.picking {
                painter.text(
                    rect.center_top() + egui::vec2(0.0, 40.0),
                    egui::Align2::CENTER_TOP,
                    match field {
                        shell_dialog::Field::Faces => "Click the faces to take away (the opening); click one again to keep it   (Esc when done)",
                        shell_dialog::Field::Bodies => "Click bodies to hollow with no opening   (Esc when done)",
                    },
                    egui::FontId::proportional(15.0),
                    Color32::from_gray(220),
                );
            }
        }

        if self.move_dialog.as_ref().is_some_and(|d| d.picking) {
            painter.text(rect.center_top() + egui::vec2(0.0, 40.0), egui::Align2::CENTER_TOP, "Click the bodies to move; click one again to drop it   (Esc when done)", egui::FontId::proportional(15.0), Color32::from_gray(220));
        }
        if let Some(field) = self.combine_dialog.as_ref().and_then(|d| d.picking) {
            let hint = match field {
                combine_dialog::Field::Target => "Click the target body (the one kept)   (Esc when done)",
                combine_dialog::Field::Tools => "Click the tool bodies; click one again to drop it   (Esc when done)",
            };
            painter.text(rect.center_top() + egui::vec2(0.0, 40.0), egui::Align2::CENTER_TOP, hint, egui::FontId::proportional(15.0), Color32::from_gray(220));
        }

        crate::mark("draw: faces Offset Face moves");
        // --- draw: faces Offset Face moves (and what a click would take) ---
        if let Some(d) = &self.offset_dialog {
            let model = self.model(Slot::Committed);
            let mut overlay = egui::Mesh::default();
            let mut paint_tris = |tris: &mut dyn Iterator<Item = [DVec3; 3]>, fill: Color32| {
                for [a, b, c] in tris {
                    if let (Some(pa), Some(pb), Some(pc)) = (view.project(a.as_vec3()), view.project(b.as_vec3()), view.project(c.as_vec3())) {
                        let base = overlay.vertices.len() as u32;
                        for q in [pa, pb, pc] {
                            overlay.colored_vertex(to_pos(q), fill);
                        }
                        overlay.add_triangle(base, base + 1, base + 2);
                    }
                }
            };
            let hover_face = match self.offset_hover {
                Some(PressPullHover::Face(f)) => Some(f),
                _ => None,
            };
            let faces = d.offset.faces.iter().map(|f| (f, Color32::from_rgba_unmultiplied(70, 140, 230, 120))).chain(hover_face.iter().map(|f| (f, profile_fill(110))));
            for (f, fill) in faces {
                let at = f.face.origin + f.face.u * f.anchor[0] + f.face.v * f.anchor[1];
                for body in &model.bodies {
                    if let Some(k) = body.face_at(&f.face, at) {
                        paint_tris(&mut body.face_tris(k), fill);
                    }
                }
            }
            if let Some(PressPullHover::Fillet(k)) = self.offset_hover {
                // The rounded faces of that fillet: every cylinder face of its radius.
                if let Some(ef) = self.edge_features.get(k) {
                    for body in &model.bodies {
                        for f in 0..body.faces.len() {
                            if kernel::fillet_for_face(std::slice::from_ref(ef), body, f).is_some() {
                                paint_tris(&mut body.face_tris(f), profile_fill(110));
                            }
                        }
                    }
                }
            }
            painter.add(egui::Shape::mesh(overlay));
            if d.picking {
                let hint = if d.press_pull && d.editing.is_none() && d.offset.faces.is_empty() {
                    "Click a face to move it, a fillet to change it, or a sketch profile to extrude   (Esc when done)"
                } else {
                    "Click the faces to move; click one again to drop it   (Esc when done)"
                };
                painter.text(rect.center_top() + egui::vec2(0.0, 40.0), egui::Align2::CENTER_TOP, hint, egui::FontId::proportional(15.0), Color32::from_gray(220));
            }
        }

        crate::mark("draw: the face a sketch would start on");
        // --- draw: the face a sketch would start on ---
        if let Some((b, face)) = self.face_hover {
            let model = self.model(Slot::Committed);
            if let Some(body) = model.bodies.get(b) {
                let n = face.normal();
                let fill = profile_fill(110);
                let mut overlay = egui::Mesh::default();
                for (t, tri) in body.mesh.tris.iter().enumerate() {
                    let [a, bb, c] = tri.map(|i| body.mesh.positions[i as usize]);
                    let on_face = body.mesh.normal(t).as_dvec3().dot(n) > 0.9999
                        && n.dot(a.as_dvec3() - face.origin).abs() < self.camera.distance as f64 * 1e-3;
                    if let (true, Some(pa), Some(pb), Some(pc)) = (on_face, view.project(a), view.project(bb), view.project(c)) {
                        let base = overlay.vertices.len() as u32;
                        for q in [pa, pb, pc] {
                            overlay.colored_vertex(to_pos(q), fill);
                        }
                        overlay.add_triangle(base, base + 1, base + 2);
                    }
                }
                painter.add(egui::Shape::mesh(overlay));
            }
        }
        if self.face_pick {
            painter.text(
                rect.center_top() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_TOP,
                if self.plane_pick { "Click an origin plane or a flat face to measure the offset from   (Esc to cancel)" } else { "Click a plane or a flat face to sketch on   (Esc to cancel)" },
                egui::FontId::proportional(15.0),
                Color32::from_gray(220),
            );
        }

        crate::mark("draw: the origin's centre dot");
        // --- draw: the origin's centre dot ---
        if show_origin {
            if let Some(o) = view.project(Vec3::ZERO) {
                painter.circle_filled(to_pos(o), 4.0, Color32::from_gray(235));
                painter.circle_stroke(to_pos(o), 4.0, Stroke::new(1.0, Color32::from_gray(60)));
            }
        }

        crate::mark("draw: the body the Project tool would ");
        // --- draw: the body the Project tool would bring in whole ---
        if let (Some(b), true) = (self.project_body_hover, self.active.is_some()) {
            if let Some(body) = self.model(Slot::Committed).bodies.get(b) {
                for e in &body.edges {
                    for w in e.points.windows(2) {
                        view.line(w[0], w[1], Stroke::new(2.0, COL_HOVER));
                    }
                }
            }
        }

        crate::mark("draw: the body edge the Project tool w");
        // --- draw: the body edge the Project tool would bring in ---
        if let (Some(h), true) = (self.project_hover, self.active.is_some()) {
            for body in &self.model(Slot::Committed).bodies {
                for e in body.edges.iter().filter(|e| e.at.mid.distance(h.mid) < 1e-6) {
                    for w in e.points.windows(2) {
                        view.line(w[0], w[1], Stroke::new(3.0, COL_HOVER));
                    }
                }
            }
        }

        crate::mark("draw: the axis a pattern would turn ab");
        // --- draw: the axis a pattern would turn about ---
        if let Some(d) = self.copy_dialog.as_ref().filter(|d| d.picking_axis) {
            let reach = origin_size;
            let ends = match (self.copy_axis_hover, d.custom) {
                (Some(AxisChoice::World(a)), _) => Some((Vec3::ZERO, a.dir().as_vec3() * reach)),
                (Some(AxisChoice::Custom(k)), _) => self.model(Slot::Committed).axes.get(k).copied().flatten().map(|(o, dir)| {
                    ((o - dir * reach as f64 * 1.5).as_vec3(), (o + dir * reach as f64 * 1.5).as_vec3())
                }),
                _ => None,
            };
            if let Some((a, b)) = ends {
                view.line(a, b, Stroke::new(4.0, COL_HOVER));
            }
            painter.text(
                rect.center_top() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_TOP,
                "Click an axis to turn the pattern about   (Esc to cancel)",
                egui::FontId::proportional(15.0),
                Color32::from_gray(220),
            );
        }

        crate::mark("draw: the line a Plane at Angle would ");
        // --- draw: the line a Plane at Angle would turn about ---
        if self.angle_pick || self.plane_dialog.as_ref().is_some_and(|d| d.picking_line()) {
            if let Some((_, p, q, _)) = self.hinge_hover {
                view.line(p.as_vec3(), q.as_vec3(), Stroke::new(4.0, COL_HOVER));
            }
            painter.text(
                rect.center_top() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_TOP,
                "Click a sketch line, edge or axis to turn the plane about   (Esc to cancel)",
                egui::FontId::proportional(15.0),
                Color32::from_gray(220),
            );
        }

        crate::mark("draw: the edge a construction axis wou");
        // --- draw: the edge a construction axis would go along ---
        if self.axis_pick {
            if let Some(h) = self.edge_hover {
                for body in &self.model(Slot::Committed).bodies {
                    for e in body.edges.iter().filter(|e| e.at.mid.distance(h.mid) < 1e-6) {
                        for w in e.points.windows(2) {
                            view.line(w[0], w[1], Stroke::new(3.0, COL_HOVER));
                        }
                    }
                }
            }
            painter.text(
                rect.center_top() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_TOP,
                "Click a straight edge to put an axis along it   (Esc to cancel)",
                egui::FontId::proportional(15.0),
                Color32::from_gray(220),
            );
        }

        crate::mark("draw: picked and hovered edges (fillet");
        // --- draw: picked and hovered edges (fillet / chamfer) ---
        if self.edge_dialog.is_some() {
            self.ensure_model(Slot::Base); // the dialog may have opened this frame
        }
        if let Some(d) = &self.edge_dialog {
            let base = self.model(Slot::Base);
            for body in &base.bodies {
                for e in &body.edges {
                    let selected = d.edges.iter().any(|r| r.mid.distance(e.at.mid) < 1e-6);
                    let hovered = self.edge_group_hover.iter().any(|r| r.mid.distance(e.at.mid) < 1e-6);
                    if selected || hovered {
                        let stroke = Stroke::new(3.0, if selected { COL_SELECT } else { COL_HOVER });
                        for w in e.points.windows(2) {
                            view.line(w[0], w[1], stroke);
                        }
                    }
                }
            }
        }


        crate::mark("draw: the drag arrow");
        // --- draw: the drag arrow ---
        let prev_arrows = std::mem::take(&mut self.arrows);
        for (k, h) in self.current_handles(&regions).into_iter().enumerate() {
            let len = ARROW_PX * mm_per_px as f32;
            let (base, dir, value) = match h {
                Handle::Line { line, value, .. } => (line.origin + line.dir * value, line.dir, value),
                Handle::Turn { turn, value, .. } => (turn.position(value), turn.direction(value), value),
            };
            let sign = if value < 0.0 { -1.0 } else { 1.0 };
            let (base, tip) = (base.as_vec3(), base.as_vec3() + dir.as_vec3() * (sign * len));
            let hot = self.arrow_drag.is_some_and(|(d, _)| d == k) || near_arrow(hover_px, &prev_arrows) == Some(k);
            // Move/Copy's arrows and rings in the axis colours (X red, Y green, Z blue).
            let axis_colour = match h.target() {
                HandleTarget::MoveX | HandleTarget::MoveRX => Some(Color32::from_rgb(225, 80, 70)),
                HandleTarget::MoveY | HandleTarget::MoveRY => Some(Color32::from_rgb(90, 190, 90)),
                HandleTarget::MoveZ | HandleTarget::MoveRZ => Some(Color32::from_rgb(80, 130, 235)),
                _ => None,
            };
            let colour = match (hot, axis_colour) {
                (true, _) => COL_ARROW_HOT,
                (false, Some(c)) => c,
                (false, None) => COL_ARROW,
            };
            self.arrows.push(view.arrow3d(base, tip, colour));
        }

        crate::mark("draw: the drag-select rectangle");
        // --- draw: the drag-select rectangle ---
        if let (Some((a, b)), Some(i)) = (self.marquee, self.active) {
            let plane = self.plane_of(i);
            let corners = [a, DVec2::new(b.x, a.y), b, DVec2::new(a.x, b.y)];
            let stroke = Stroke::new(1.5, Color32::from_rgb(120, 220, 140));
            for k in 0..4 {
                view.dashed(plane.to_world(corners[k]), plane.to_world(corners[(k + 1) % 4]), stroke);
            }
        }

        crate::mark("draw: the fillet / chamfer the size bo");
        // --- draw: the fillet / chamfer the size box would make ---
        if let (Some(EditTarget::Corner(p)), Some(i)) = (self.float.as_ref().map(|f| f.target), self.active) {
            if let Some(cut) = self.corner_preview(i, p) {
                let plane = self.plane_of(i);
                let stroke = Stroke::new(2.0, COL_PREVIEW);
                match cut.arc {
                    Some((c, r, a0, sweep)) => view.arc(&plane, c, r, a0, sweep, stroke),
                    None => view.line(plane.to_world(cut.a), plane.to_world(cut.b), stroke),
                }
            }
        }

        crate::mark("draw: profile fills");
        // --- draw: profile fills ---
        if let Some((pick_sketch, pick_profiles)) = self.profile_pick() {
            for s in (0..self.sketches.len()).filter(|&s| self.pickable_sketch(s)) {
                let plane = self.plane_of(s);
                for (ri, r) in regions.get(s).into_iter().flatten().enumerate() {
                    let selected = pick_sketch == Some(s) && pick_profiles.contains(&ri);
                    let alpha = if selected {
                        140
                    } else if hover_profile == Some((s, ri)) {
                        90
                    } else {
                        35
                    };
                    view.fill_region(&plane, r, profile_fill(alpha));
                }
            }
        } else if self.offset_dialog.as_ref().is_some_and(|d| d.picking && d.press_pull && d.editing.is_none() && d.offset.faces.is_empty()) {
            // Press Pull also takes profiles: show them, the one under the cursor stronger.
            for s in (0..self.sketches.len()).filter(|&s| self.sketch_visible(s)) {
                let plane = self.plane_of(s);
                for (ri, r) in regions.get(s).into_iter().flatten().enumerate() {
                    let alpha = if self.offset_hover == Some(PressPullHover::Profile(s, ri)) { 90 } else { 35 };
                    view.fill_region(&plane, r, profile_fill(alpha));
                }
            }
        } else if let Some(i) = self.active {
            let plane = self.plane_of(i);
            for r in regions.get(i).into_iter().flatten() {
                view.fill_region(&plane, r, profile_fill(28)); // shows which loops are closed
            }
        }

        crate::mark("draw: the revolve axis (and a line it ");
        // --- draw: the revolve axis (and a line it could be) ---
        if let Some(d) = &self.revolve_dialog {
            let axis_line = |s: usize, which: RevolveAxis| self.revolve_axis_line(s, which, 1.0);
            if let Some((a, b)) = d.sketch.and_then(|s| axis_line(s, d.axis)) {
                view.line(a, b, Stroke::new(2.0, COL_SELECT));
            }
            if let Some((s, e)) = self.axis_hover {
                if let Some((a, b)) = axis_line(s, RevolveAxis::Line(e)) {
                    view.line(a, b, Stroke::new(2.0, COL_HOVER));
                }
            }
        }

        crate::mark("draw: sketches");
        // --- draw: sketches ---
        self.dim_labels.clear();
        for (j, sk) in self.sketches.iter().enumerate() {
            let plane = self.plane_of(j);
            let active = self.active == Some(j);
            // Like Fusion: a sketch used by a feature hides itself. Shown: while
            // editing it, when its visibility is on, or when it's the one an
            // extrude being edited uses. Hidden sketches can't be picked from.
            // Sketch lines draw over bodies, so a visible sketch shows through
            // the part it's on.
            if !active && !self.pickable_sketch(j) {
                continue;
            }
            let base = if !active {
                COL_INACTIVE
            } else if self.dof == Some(0) {
                COL_FULL
            } else {
                COL_UNDER
            };
            let lost = self.caches[Slot::Committed as usize].as_ref().and_then(|c| c.model.projection_lost.get(j)).cloned().unwrap_or_default();
            let colour_of = |s: Sel| {
                if !active {
                    base
                } else if self.editor.selection.contains(&s)
                    || matches!(s, Sel::Entity(e) if self.editor.tangent_lines.contains(&e))
                {
                    COL_SELECT
                } else if hovered == Some(s) {
                    COL_HOVER
                } else {
                    base
                }
            };
            for (e, ent) in sk.entities.iter().enumerate() {
                let mut col = colour_of(Sel::Entity(e));
                // Linked to an edge that's gone: red, as in Fusion; else projected: purple.
                if col == base && lost.contains(&e) {
                    col = COL_FIXED;
                } else if col == base && sk.is_projected(e) {
                    col = COL_PROJECTED;
                }
                let stroke = Stroke::new(if active { 2.0 } else { 1.5 }, col);
                let dashed = sk.is_construction(e);
                match *ent {
                    Entity::Line { a, b } if dashed => {
                        view.dashed(plane.to_world(sk.points[a]), plane.to_world(sk.points[b]), stroke)
                    }
                    Entity::Line { a, b } => view.line(plane.to_world(sk.points[a]), plane.to_world(sk.points[b]), stroke),
                    Entity::Circle { center, radius } if dashed => view.dashed_circle(&plane, sk.points[center], radius, stroke),
                    Entity::Circle { center, radius } => view.circle(&plane, sk.points[center], radius, stroke),
                    Entity::Point { .. } => {} // drawn with the points
                    Entity::Arc { .. } => {
                        if let Some((c, r, a0, sweep)) = sk.arc_geometry(e) {
                            if dashed {
                                view.dashed_arc(&plane, c, r, a0, sweep, stroke);
                            } else {
                                view.arc(&plane, c, r, a0, sweep, stroke);
                            }
                        }
                    }
                    Entity::Spline { s } => {
                        for w in sk.spline_curve(s).windows(2) {
                            if dashed {
                                view.dashed(plane.to_world(w[0]), plane.to_world(w[1]), stroke);
                            } else {
                                view.line(plane.to_world(w[0]), plane.to_world(w[1]), stroke);
                            }
                        }
                    }
                    Entity::Ellipse { .. } => {
                        let curve = sk.ellipse_curve(e);
                        for (k, w) in curve.windows(2).enumerate() {
                            // Dashed: every other piece.
                            if !dashed || k % 2 == 0 {
                                view.line(plane.to_world(w[0]), plane.to_world(w[1]), stroke);
                            }
                        }
                    }
                }
            }
            if active {
                // Control Point Splines show their control frame, dashed.
                for &(s, _) in &sk.control_splines {
                    let pts = sk.spline_fit(s);
                    for w in pts.windows(2) {
                        view.dashed(plane.to_world(w[0]), plane.to_world(w[1]), Stroke::new(1.0, Color32::from_rgb(120, 200, 255)));
                    }
                }
            }
            if active && self.editor.tool == Tool::Select {
                // A selected spline's tangent handles (Fusion's): a line through each
                // fit point; its ends hollow while the curve takes its own way, filled once set.
                let handle_col = Color32::from_rgb(120, 200, 255);
                {
                    for (_, p, fwd, set) in sk.shown_handles(&self.editor.selection) {
                        let back = sk.points[p] * 2.0 - fwd;
                        let stroke = Stroke::new(1.2, handle_col);
                        view.line(plane.to_world(back), plane.to_world(fwd), stroke);
                        for end in [back, fwd] {
                            if let Some(px) = view.project(plane.to_world(end)) {
                                if set {
                                    painter.circle_filled(to_pos(px), 4.0, handle_col);
                                } else {
                                    painter.circle_stroke(to_pos(px), 4.0, stroke);
                                }
                            }
                        }
                    }
                }
            }
            if active && self.acquired.0 == Some(j) && !matches!(self.editor.tool, Tool::Select | Tool::Constrain(_)) {
                // Acquired middles: a small ring; the guide through one, dashed to the cursor.
                for &e in &self.acquired.1 {
                    if let Some((m, _)) = sk.middle_guide(e) {
                        if let Some(px) = view.project(plane.to_world(m)) {
                            painter.circle_stroke(to_pos(px), 3.5, Stroke::new(1.5, COL_SNAP));
                        }
                    }
                }
                if let Some((q, from)) = &self.mid_guide {
                    for &m in from {
                        view.dashed(plane.to_world(m), plane.to_world(*q), Stroke::new(1.5, COL_SNAP));
                    }
                }
            }
            if let (true, Some((above, beside, q))) = (active, self.line_up) {
                for r in [above, beside].into_iter().flatten() {
                    view.dashed(plane.to_world(r), plane.to_world(q), Stroke::new(1.5, COL_SNAP));
                    if let Some(px) = view.project(plane.to_world(r)) {
                        painter.circle_stroke(to_pos(px), 4.0, Stroke::new(1.5, COL_SNAP));
                    }
                }
            }
            if active {
                // Break: the stretch the click picks out, and dots where it splits.
                if self.editor.tool == Tool::Break {
                    let stroke = Stroke::new(4.0, COL_SELECT);
                    match &self.cut_preview {
                        Some((_, sketch::TrimPiece::Line(a, b))) => view.line(plane.to_world(*a), plane.to_world(*b), stroke),
                        Some((_, sketch::TrimPiece::Arc(c, r, a0, sweep))) => view.arc(&plane, *c, *r, *a0, *sweep, stroke),
                        Some((_, sketch::TrimPiece::Curve(pts))) => {
                            for w in pts.windows(2) {
                                view.line(plane.to_world(w[0]), plane.to_world(w[1]), stroke);
                            }
                        }
                        None => {}
                    }
                    for m in &self.break_marks {
                        if let Some(px) = view.project(plane.to_world(*m)) {
                            painter.circle_filled(to_pos(px), 5.0, COL_SNAP);
                        }
                    }
                }
                match &self.cut_preview {
                    Some((Tool::Trim, sketch::TrimPiece::Curve(pts))) => {
                        for w in pts.windows(2) {
                            view.line(plane.to_world(w[0]), plane.to_world(w[1]), Stroke::new(4.0, Color32::from_rgb(230, 70, 70)));
                        }
                    }
                    _ => {}
                }
                match self.cut_preview.clone().filter(|(t, _)| *t != Tool::Break) {
                    // The stretch Trim takes away: red over the line or curve.
                    Some((Tool::Trim, sketch::TrimPiece::Line(a, b))) => view.line(plane.to_world(a), plane.to_world(b), Stroke::new(4.0, Color32::from_rgb(230, 70, 70))),
                    Some((Tool::Trim, sketch::TrimPiece::Arc(c, r, a0, sweep))) => view.arc(&plane, c, r, a0, sweep, Stroke::new(4.0, Color32::from_rgb(230, 70, 70))),
                    // The piece Extend adds: dashed, to a dot where the end goes.
                    Some((_, sketch::TrimPiece::Line(a, b))) => {
                        view.dashed(plane.to_world(a), plane.to_world(b), Stroke::new(2.0, COL_PREVIEW));
                        if let Some(px) = view.project(plane.to_world(b)) {
                            painter.circle_filled(to_pos(px), 4.0, COL_PREVIEW);
                        }
                    }
                    _ => {}
                }
                if let Some(p) = &self.offset_preview {
                    let stroke = Stroke::new(2.0, COL_PREVIEW);
                    for e in sk.entities.len()..p.entities.len() {
                        match p.entities[e] {
                            Entity::Line { a, b } => view.line(plane.to_world(p.points[a]), plane.to_world(p.points[b]), stroke),
                            Entity::Circle { center, radius } => view.circle(&plane, p.points[center], radius, stroke),
                            Entity::Arc { .. } => {
                                if let Some((c, r, a0, sweep)) = p.arc_geometry(e) {
                                    view.arc(&plane, c, r, a0, sweep, stroke);
                                }
                            }
                            Entity::Spline { s } => {
                                for w in p.spline_curve(s).windows(2) {
                                    view.line(plane.to_world(w[0]), plane.to_world(w[1]), stroke);
                                }
                            }
                            Entity::Point { .. } | Entity::Ellipse { .. } => {}
                        }
                    }
                }
            }
            if !active {
                continue;
            }
            let projected: Vec<Option<Vec2>> = sk.points.iter().map(|&p| view.project(plane.to_world(p))).collect();
            let uses = sk.point_uses();
            for (id, px) in projected.iter().enumerate() {
                if let Some(px) = *px {
                    let sel = Sel::Point(id);
                    let col = if self.editor.selection.contains(&sel) || hovered == Some(sel) {
                        colour_of(sel)
                    } else if id == Sketch::ORIGIN {
                        COL_ORIGIN
                    } else if sk.is_fixed(id) {
                        COL_FIXED
                    } else {
                        COL_POINT
                    };
                    if uses[id] >= 2 && id != Sketch::ORIGIN {
                        // A joint: lines that share this point stay connected.
                        painter.circle_filled(to_pos(px), 4.5, col);
                        painter.circle_filled(to_pos(px), 1.8, Color32::from_gray(20));
                    } else {
                        painter.circle_filled(to_pos(px), 3.5, col);
                    }
                }
            }

            // Dimensions sit outside the shape, other glyphs inside it.
            let visible: Vec<Vec2> = projected.iter().skip(1).flatten().copied().collect();
            let centroid = if visible.is_empty() {
                to_vec(rect.center())
            } else {
                visible.iter().copied().sum::<Vec2>() / visible.len() as f32
            };
            // Constraint glyphs, by what they sit beside: (constraint, text).
            // Each is its own label, so it can be clicked (selected) and deleted.
            let mut glyphs: Vec<Vec<(usize, &str)>> = vec![Vec::new(); sk.entities.len()];
            let mut point_glyphs: Vec<Vec<(usize, &str)>> = vec![Vec::new(); sk.points.len()];
            for (ci, c) in sk.constraints.iter().enumerate() {
                let label = match *c {
                    // An offset's pieces share one dimension, on the first.
                    Constraint::Offset(..) if sk.offset_group(ci).first() != Some(&ci) => None,
                    Constraint::Length(..)
                    | Constraint::Distance(..)
                    | Constraint::Diameter(..)
                    | Constraint::Radius(..)
                    | Constraint::Offset(..)
                    | Constraint::Angle(..) => {
                        // Drawn with its lines recorded, so they can be clicked.
                        let colour = if self.dim_selection.contains(&ci) {
                            COL_SELECT
                        } else if self.dim_hover == Some(ci) {
                            COL_HOVER
                        } else if sk.is_driven(ci) {
                            COL_DRIVEN
                        } else if sk.dim_expr(ci).is_some_and(|e| {
                            let kind = if matches!(c, Constraint::Angle(..)) { expr::Kind::Angle } else { expr::Kind::Length };
                            expr::eval(e, kind, sk.dim_unit(ci).unwrap_or(self.units)).is_err()
                        }) {
                            // Its expression no longer works (a parameter gone): red, as Fusion shows it.
                            Color32::from_rgb(230, 80, 70)
                        } else {
                            COL_DIM
                        };
                        let trace = std::cell::RefCell::new(Vec::new());
                        let traced = View { trace: Some(&trace), ..view };
                        let r = draw_dimension(&traced, sk, &plane, c, sk.is_driven(ci), sk.label_shift(ci), centroid, colour, sk.dim_unit(ci).unwrap_or(self.units), sk.dim_expr(ci).is_some());
                        r.map(|r| (r, trace.into_inner()))
                    }
                    // Glyphs about a point sit at the point.
                    // Symmetry: at both mirrored points.
                    Constraint::Symmetric(p, q, _) | Constraint::SymmetricAxis(p, q, _) => {
                        for x in [p, q] {
                            if let Some(g) = point_glyphs.get_mut(x) {
                                g.push((ci, c.glyph()));
                            }
                        }
                        None
                    }
                    Constraint::Fix(p) | Constraint::Coincident(p, _) | Constraint::OnAxis(p, _) | Constraint::Midpoint(p, ..) => {
                        if let Some(g) = point_glyphs.get_mut(p) {
                            g.push((ci, c.glyph()));
                        }
                        None
                    }
                    _ => {
                        for e in c.entities() {
                            glyphs[e].push((ci, c.glyph()));
                        }
                        None
                    }
                };
                if let Some((r, segs)) = label {
                    self.dim_labels.push((ci, r, segs));
                }
            }
            // The dimension being placed (Dimension tool), following the cursor.
            if let (Tool::Constrain("Dimension"), Some((c, shift))) = (self.editor.tool, self.dim_preview) {
                if !self.editor.selection.is_empty() {
                    draw_dimension(&view, sk, &plane, &c, false, shift, centroid, COL_DIM.gamma_multiply(0.7), self.units, false);
                }
            }
            let glyph_colour = |ci: usize| {
                if self.dim_selection.contains(&ci) {
                    COL_SELECT
                } else if self.dim_hover == Some(ci) {
                    COL_HOVER
                } else if matches!(sk.constraints[ci], Constraint::Fix(_)) {
                    COL_FIXED
                } else {
                    COL_GLYPH
                }
            };
            let mut glyph_rects: Vec<(usize, egui::Rect)> = Vec::new();
            let mut place = |items: &[(usize, &str)], at: Vec2, dir: Vec2, gap: f32| {
                let texts: Vec<(String, Color32)> = items.iter().map(|&(ci, t)| (t.to_string(), glyph_colour(ci))).collect();
                for (r, &(ci, _)) in view.texts_beside(at, dir, gap, &texts).into_iter().zip(items) {
                    glyph_rects.push((ci, r));
                }
            };
            for (p, g) in point_glyphs.iter().enumerate() {
                if let (false, Some(px)) = (g.is_empty(), projected[p]) {
                    place(g, px, Vec2::new(0.0, -1.0), 10.0);
                }
            }
            for (e, g) in glyphs.iter().enumerate() {
                if g.is_empty() {
                    continue;
                }
                match sk.entities[e] {
                    Entity::Line { a, b } => {
                        if let (Some(pa), Some(pb)) = (projected[a], projected[b]) {
                            let mid = (pa + pb) * 0.5;
                            let mut n = (pb - pa).normalize_or_zero().perp();
                            if (centroid - mid).dot(n) < 0.0 {
                                n = -n;
                            }
                            place(g, mid, n, 8.0);
                        }
                    }
                    Entity::Circle { center, radius } => {
                        let at = sk.points[center] + DVec2::new(-radius, radius) * std::f64::consts::FRAC_1_SQRT_2;
                        if let (Some(pc), Some(pa)) = (projected[center], view.project(plane.to_world(at))) {
                            place(g, pa, (pa - pc).normalize_or_zero(), 6.0);
                        }
                    }
                    Entity::Point { p } => {
                        if let Some(px) = projected[p] {
                            place(g, px, Vec2::new(0.7, -0.7), 8.0);
                        }
                    }
                    Entity::Arc { center, .. } => {
                        if let Some((c, r, a0, sweep)) = sk.arc_geometry(e) {
                            let mid = c + DVec2::from_angle(a0 + sweep * 0.5) * r;
                            if let (Some(pc), Some(pm)) = (projected[center], view.project(plane.to_world(mid))) {
                                place(g, pm, (pm - pc).normalize_or_zero(), 6.0);
                            }
                        }
                    }
                    Entity::Ellipse { .. } => {
                        let curve = sk.ellipse_curve(e);
                        if let Some(px) = curve.get(curve.len() / 8).and_then(|&m| view.project(plane.to_world(m))) {
                            place(g, px, Vec2::new(0.7, -0.7), 8.0);
                        }
                    }
                    // Beside the middle of the curve.
                    Entity::Spline { s } => {
                        let curve = sk.spline_curve(s);
                        if let Some(px) = curve.get(curve.len() / 2).and_then(|&m| view.project(plane.to_world(m))) {
                            place(g, px, Vec2::new(0.0, -1.0), 8.0);
                        }
                    }
                }
            }
            // Glyphs are picked like dimension labels (but not dragged or edited).
            self.dim_labels.extend(glyph_rects.into_iter().map(|(ci, r)| (ci, r.expand(2.0), Vec::new())));
            // Over a glyph: what it is and what clicking does.
            if let (Some(ci), Some(px)) = (self.dim_hover, hover_px) {
                if let Some(c) = sk.constraints.get(ci).filter(|c| c.value().is_none()) {
                    let tip = format!("{} constraint. Click it to select it, then press Delete to remove it.", c.name());
                    view.label(px + Vec2::new(14.0, 18.0), egui::Align2::LEFT_TOP, tip, Color32::from_gray(225));
                }
            }
        }

        crate::mark("draw: measure (over the sketches, so a");
        // --- draw: measure (over the sketches, so a picked sketch line shows) ---
        if let Some(m) = &self.measure {
            use measure::{Geom, Pick, Show};
            let model = &self.model(Slot::Committed);
            let show = |p: &measure::Picked, col: Color32, width: f32| match p.pick {
                Pick::Point(_) | Pick::SketchPoint { .. } => {
                    if let Geom::Point(q) = &p.geom {
                        if let Some(px) = view.project(q.as_vec3()) {
                            painter.circle_filled(to_pos(px), width + 2.0, col);
                        }
                    }
                }
                Pick::Edge { .. } | Pick::SketchCurve { .. } => {
                    for s in &p.samples.segments {
                        view.line(s[0].as_vec3(), s[1].as_vec3(), Stroke::new(width, col));
                    }
                }
                Pick::Face { body, face } => {
                    if let Some(b) = model.bodies.get(body) {
                        view.fill_tris(b.face_tris(face).map(|t| t.map(|q| q.as_vec3())), col.gamma_multiply(0.35));
                    }
                }
                Pick::Body { .. } | Pick::Profile { .. } => {
                    view.fill_tris(p.samples.triangles.iter().map(|t| t.map(|q| q.as_vec3())), col.gamma_multiply(0.3));
                }
            };
            let hot = m.hover_row;
            for (i, p) in m.picks.iter().enumerate() {
                let lit = matches!(hot, Some(Show::Pick(k)) if k == i) || hot == Some(Show::Angle);
                show(p, if lit { COL_HOVER } else { COL_SELECT }, if lit { 5.0 } else { 3.5 });
            }
            if let Some(h) = m.hover.as_ref().filter(|h| !m.picks.iter().any(|p| p.pick == h.pick)) {
                show(h, COL_HOVER, 3.5);
            }
            for q in &m.snaps {
                if let Some(px) = view.project(q.as_vec3()) {
                    painter.circle_stroke(to_pos(px), 4.0, Stroke::new(1.5, COL_SNAP));
                }
            }
            // A circle's radius, diameter or centre, while its row is hovered.
            if let Some(Show::Radius(i) | Show::Diameter(i) | Show::Centre(i)) = hot {
                if let Some(Geom::Circle { center, normal, radius, .. }) = m.picks.get(i).map(|p| &p.geom) {
                    let out = normal.any_orthonormal_vector() * *radius;
                    let stroke = Stroke::new(2.5, COL_HOVER);
                    match hot {
                        Some(Show::Radius(_)) => view.line(center.as_vec3(), (*center + out).as_vec3(), stroke),
                        Some(Show::Diameter(_)) => view.line((*center - out).as_vec3(), (*center + out).as_vec3(), stroke),
                        _ => {}
                    }
                    if let Some(px) = view.project(center.as_vec3()) {
                        painter.circle_filled(to_pos(px), 4.0, COL_HOVER);
                    }
                }
                if let Some(Geom::Plane { center: Some(c), .. }) = m.picks.get(i).map(|p| &p.geom) {
                    if let Some(px) = view.project(c.as_vec3()) {
                        painter.circle_filled(to_pos(px), 4.0, COL_HOVER);
                    }
                }
            }
            if let Some((a, b)) = m.closest {
                let lit = hot == Some(Show::Distance);
                view.dashed(a.as_vec3(), b.as_vec3(), Stroke::new(if lit { 3.5 } else { 2.0 }, if lit { COL_HOVER } else { COL_PREVIEW }));
                for q in [a, b] {
                    if let Some(px) = view.project(q.as_vec3()) {
                        painter.circle_filled(to_pos(px), 3.5, COL_PREVIEW);
                    }
                }
            }
            // X, Y and Z legs between two corners (XYZ Delta).
            if let (true, [p, q]) = (self.measure_xyz, &m.picks[..]) {
                if let (Geom::Point(a), Geom::Point(b)) = (&p.geom, &q.geom) {
                    let corners = [*a, DVec3::new(b.x, a.y, a.z), DVec3::new(b.x, b.y, a.z), *b];
                    for k in 0..3 {
                        let lit = hot == Some(Show::Delta(k));
                        let col = axis_color(Vec3::AXES[k]);
                        view.line(corners[k].as_vec3(), corners[k + 1].as_vec3(), Stroke::new(if lit { 4.0 } else { 1.5 }, col));
                    }
                }
            }
            painter.text(
                rect.center_top() + egui::vec2(0.0, 40.0),
                egui::Align2::CENTER_TOP,
                "Click one or two things   (Ctrl: lock to snap points, Shift: hide them, Esc: leave)",
                egui::FontId::proportional(13.0),
                COL_PREVIEW,
            );
        }

        crate::mark("right-click menu: a sketch's selection");
        // --- right-click menu: a sketch's selection, or an origin plane ---
        response.context_menu(|ui| {
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
        });

        crate::mark("floating editor position");
        // --- floating editor position ---
        self.float_anchor = match self.float.as_ref().map(|f| f.target) {
            Some(EditTarget::Dimension(ci)) => {
                self.dim_labels.iter().find(|(c, ..)| *c == ci).map(|(_, r, _)| r.center() - egui::vec2(52.0, 14.0))
            }
            Some(EditTarget::Corner(p)) => self.active.and_then(|i| {
                let at = *self.sketches[i].points.get(p)?;
                view.project(self.plane_of(i).to_world(at)).map(|q| self.off_arrow(to_pos(q) + egui::vec2(14.0, 10.0)))
            }),
            Some(EditTarget::PolygonSides) => self.active.and_then(|i| {
                let sk = &self.sketches[i];
                let start = self.editor.start.filter(|s| sk.is_valid_anchor(*s))?;
                view.project(self.plane_of(i).to_world(sk.pos(start))).map(|p| to_pos(p) + egui::vec2(16.0, 12.0))
            }),
            Some(EditTarget::NewCircle) => self.active.and_then(|i| {
                let sk = &self.sketches[i];
                let at = match (self.editor.tool, self.editor.selection.first()) {
                    (Tool::Offset, Some(&Sel::Entity(e))) => sk.entity_points(e).first().map(|&p| sk.points[p])?,
                    (Tool::Slot, _) => sk.pos(self.editor.second.filter(|s| sk.is_valid_anchor(*s))?),
                    _ => sk.pos(self.editor.start.filter(|s| sk.is_valid_anchor(*s))?),
                };
                view.project(self.plane_of(i).to_world(at)).map(|p| to_pos(p) + egui::vec2(16.0, 12.0))
            }),
            Some(EditTarget::RevolveAngle) => self.revolve_dialog.as_ref().and_then(|d| {
                let s = d.sketch?;
                let r = regions.get(s)?.get(*d.profiles.first()?)?;
                view.project(self.plane_of(s).to_world(r.centroid())).map(|p| self.off_arrow(to_pos(p) + egui::vec2(16.0, -16.0)))
            }),
            Some(EditTarget::EdgeSize) => self.edge_dialog.as_ref().and_then(|d| {
                // Next to the first picked edge.
                let r = d.edges.first()?;
                view.project(r.mid.as_vec3()).map(|p| self.off_arrow(to_pos(p) + egui::vec2(18.0, -22.0)))
            }),
            Some(EditTarget::PlaneAngle) => self.plane_dialog.as_ref().and_then(|d| {
                // By the round arrow, which sits on the plane's far side from its line.
                let reach = (d.extent[2] - d.extent[0]).abs().max(1.0) * 0.5;
                let f = d.frame();
                view.project((f.origin + f.v * reach).as_vec3()).map(|p| self.off_arrow(to_pos(p) + egui::vec2(16.0, -16.0)))
            }),
            Some(EditTarget::OffsetDistance) => self.offset_dialog.as_ref().and_then(|d| {
                // By the arrow's tip.
                let p = d.offset.faces.first()?;
                let tip = p.face.origin + p.face.u * p.anchor[0] + p.face.v * p.anchor[1] + p.face.normal() * d.offset.distance;
                view.project(tip.as_vec3()).map(|q| self.off_arrow(to_pos(q) + egui::vec2(16.0, -16.0)))
            }),
            Some(EditTarget::PlaneOffset) => self.plane_dialog.as_ref().and_then(|d| {
                // By the arrow's tip.
                let (lo, hi) = self.plane_rect(d.extent, &d.base, d.on_origin, origin::size_for(self.camera.distance));
                let c = (lo + hi) * 0.5;
                let tip = d.base.origin + d.base.u * c.x + d.base.v * c.y + d.base.normal() * d.distance;
                view.project(tip.as_vec3()).map(|p| self.off_arrow(to_pos(p) + egui::vec2(16.0, -16.0)))
            }),
            Some(EditTarget::ExtrudeDistance) => self.dialog.as_ref().filter(|d| d.extent != model::Extent::ToObject).and_then(|d| {
                let s = d.sketch?;
                let r = regions.get(s)?.get(*d.profiles.first()?)?;
                let plane = self.plane_of(s);
                let top = plane.to_world(r.centroid()) + plane.normal() * d.distance as f32;
                view.project(top).map(|p| to_pos(p) + egui::vec2(16.0, -16.0))
            }),
            None => None,
        };

        crate::mark("draw: snap marker, tool preview, reado");
        // --- draw: snap marker, tool preview, readouts ---
        let mut status = String::new();
        let u = self.units; // readouts in the design's units
        if let Some(i) = self.active {
            let sk = &self.sketches[i];
            let plane = self.plane_of(i);
            if let Some(at) = cursor {
                let here = sk.pos(at);
                if let Some(px) = view.project(plane.to_world(here)) {
                    match at {
                        Anchor::Point(_) => {
                            painter.circle_stroke(to_pos(px), 7.0, Stroke::new(2.0, COL_SNAP));
                        }
                        Anchor::Free(_) if self.grid_snap => {
                            painter.circle_filled(to_pos(px), 3.0, COL_PREVIEW);
                        }
                        Anchor::Free(_) => {}
                        Anchor::OnLine(..) | Anchor::OnAxis(..) => {
                            // A small diamond: "this point will stay on that line".
                            let c = to_pos(px);
                            if let Anchor::OnLine(e, q) = at {
                                if let Some(&Entity::Line { a, b }) = sk.entities.get(e) {
                                    if sk.points[a].midpoint(sk.points[b]).distance(q) < 1e-9 {
                                        view.text_beside(px, Vec2::new(1.0, -1.0), 14.0, "Midpoint".into(), COL_SNAP);
                                    }
                                }
                            }
                            painter.add(egui::Shape::closed_line(
                                vec![c + egui::vec2(0.0, -6.0), c + egui::vec2(6.0, 0.0), c + egui::vec2(0.0, 6.0), c + egui::vec2(-6.0, 0.0)],
                                Stroke::new(2.0, COL_SNAP),
                            ));
                        }
                    }
                }
                status = format!("x {}  y {}", u.show(here.x), u.show(here.y));
                // The spline being drawn: through its points and the cursor.
                if matches!(self.editor.tool, Tool::Spline | Tool::ControlSpline) && !self.editor.spline.is_empty() {
                    let curve = self.editor.spline_preview(sk, here);
                    for w in curve.windows(2) {
                        view.line(plane.to_world(w[0]), plane.to_world(w[1]), Stroke::new(1.5, COL_PREVIEW));
                    }
                    let placed: Vec<DVec2> = self.editor.spline.iter().filter(|a| sk.is_valid_anchor(**a)).map(|a| sk.pos(*a)).collect();
                    for q in &placed {
                        if let Some(px) = view.project(plane.to_world(*q)) {
                            painter.circle_filled(to_pos(px), 3.5, COL_PREVIEW);
                        }
                    }
                    // A ✓ just above and to the left of the last point: click it to finish.
                    self.spline_check = None;
                    if let Some(px) = placed.last().filter(|_| placed.len() >= 2).and_then(|q| view.project(plane.to_world(*q))) {
                        let c = to_pos(px) + egui::vec2(-18.0, -18.0);
                        let r = egui::Rect::from_center_size(c, egui::vec2(18.0, 18.0));
                        let hot = hover_px.is_some_and(|h| r.contains(to_pos(h)));
                        painter.circle_filled(c, 9.0, if hot { Color32::from_rgb(90, 200, 110) } else { Color32::from_rgb(60, 160, 80) });
                        let s = Stroke::new(2.0, Color32::WHITE);
                        painter.line_segment([c + egui::vec2(-4.5, 0.0), c + egui::vec2(-1.5, 3.5)], s);
                        painter.line_segment([c + egui::vec2(-1.5, 3.5), c + egui::vec2(4.5, -3.5)], s);
                        self.spline_check = Some(r);
                    }
                    let control = self.editor.tool == Tool::ControlSpline;
                    if control {
                        // The control frame, dashed, on to the cursor.
                        for w in placed.iter().copied().chain(std::iter::once(here)).collect::<Vec<_>>().windows(2) {
                            view.dashed(plane.to_world(w[0]), plane.to_world(w[1]), Stroke::new(1.0, COL_PREVIEW));
                        }
                    }
                    let closing = !control && self.editor.spline.len() >= 3 && cursor == self.editor.spline.first().copied();
                    if closing {
                        if let Some(px) = view.project(plane.to_world(here)) {
                            painter.circle_stroke(to_pos(px), 8.0, Stroke::new(2.0, COL_SNAP));
                        }
                    }
                    status = if closing {
                        "click to close the spline".to_string()
                    } else if control {
                        format!("{} control point{} · degree {} (Up/Down arrow keys) — click more, Enter (or the last point again) to finish", self.editor.spline.len(), if self.editor.spline.len() == 1 { "" } else { "s" }, self.editor.degree())
                    } else {
                        format!(
                            "{} point{} — click more, the first point to close it, or Enter (or the last point again) to finish",
                            self.editor.spline.len(),
                            if self.editor.spline.len() == 1 { "" } else { "s" }
                        )
                    };
                }
                if let Some(start) = self.editor.start {
                    let from = sk.pos(start);
                    let preview = Stroke::new(1.5, COL_PREVIEW);
                    let label = match self.editor.tool {
                        // Dragging off a line's end: the tangent arc it would make.
                        Tool::Line if self.line_arc.is_some() => {
                            let p = self.line_arc.expect("checked");
                            match sk.tangent_from(p).and_then(|(dir, _)| sketch::tangent_arc(sk.points[p], dir, here)) {
                                Some(a) => {
                                    view.arc(&plane, a.center, a.radius, a.start, a.sweep, preview);
                                    format!("R {}   {:.1}°   (let go to place the arc)", u.show(a.radius), a.sweep.to_degrees())
                                }
                                None => "drag sideways for an arc".to_string(),
                            }
                        }
                        Tool::Rectangle | Tool::Rectangle3Point | Tool::RectangleCenter => {
                            match self.editor.rectangle_preview(sk, here) {
                                Some(c) => {
                                    for k in 0..4 {
                                        view.line(plane.to_world(c[k]), plane.to_world(c[(k + 1) % 4]), preview);
                                    }
                                    let sizes = [c[0].distance(c[1]), c[1].distance(c[2])];
                                    if matches!(self.editor.tool, Tool::Rectangle | Tool::RectangleCenter) {
                                        // The width box by a horizontal side, the height box by a
                                        // vertical one, both just outside the rectangle.
                                        let mid = (c[0] + c[2]) * 0.5;
                                        let place = |a: DVec2, b: DVec2| {
                                            let m = (a + b) * 0.5;
                                            let (pm, pc) = (view.project(plane.to_world(m))?, view.project(plane.to_world(mid))?);
                                            let out = (pm - pc).try_normalize().unwrap_or(Vec2::Y);
                                            Some(to_pos(pm + out * 22.0))
                                        };
                                        self.rect_boxes = place(c[0], c[1]).zip(place(c[1], c[2])).map(|(w, h)| ([w, h], sizes));
                                        format!("{} × {}   (type the width, Tab for the height)", u.show(sizes[0]), u.show(sizes[1]))
                                    } else {
                                        format!("{} × {}", u.show(sizes[0]), u.show(sizes[1]))
                                    }
                                }
                                None => {
                                    // 3-point, first side: just that side so far.
                                    view.line(plane.to_world(from), plane.to_world(here), preview);
                                    u.show(from.distance(here))
                                }
                            }
                        }
                        Tool::PolygonCircumscribed | Tool::PolygonInscribed | Tool::PolygonEdge => {
                            match self.editor.polygon_preview(sk, here) {
                                Some(p) => {
                                    let n = p.verts.len();
                                    for k in 0..n {
                                        view.line(plane.to_world(p.verts[k]), plane.to_world(p.verts[(k + 1) % n]), preview);
                                    }
                                    format!("{n} sides   side {}   (Up/Down arrow keys change the sides)", u.show(p.verts[0].distance(p.verts[1])))
                                }
                                None => {
                                    // Edge polygon, first edge so far.
                                    view.line(plane.to_world(from), plane.to_world(here), preview);
                                    u.show(from.distance(here))
                                }
                            }
                        }
                        Tool::Circle => {
                            let r = from.distance(here);
                            view.circle(&plane, from, r, preview);
                            let tangent = matches!(at, Anchor::OnLine(e, q) if sk.line_foot(e, from).is_some_and(|f| f.distance(q) < 1e-9 * (1.0 + r)));
                            format!("Ø {}{}   (or type the diameter)", u.show(2.0 * r), if tangent { "  tangent" } else { "" })
                        }
                        Tool::Ellipse => match self.editor.ellipse_preview(sk, here) {
                            Some((c, ax, bx)) => {
                                let pts: Vec<DVec2> = (0..=96).map(|k| {
                                    let t = std::f64::consts::TAU * k as f64 / 96.0;
                                    c + ax * t.cos() + bx * t.sin()
                                }).collect();
                                for w in pts.windows(2) {
                                    view.line(plane.to_world(w[0]), plane.to_world(w[1]), preview);
                                }
                                view.dashed(plane.to_world(c), plane.to_world(c + ax), preview);
                                view.dashed(plane.to_world(c), plane.to_world(c + bx), preview);
                                format!("{} × {}", u.show(2.0 * ax.length().max(bx.length())), u.show(2.0 * ax.length().min(bx.length())))
                            }
                            None => {
                                // The first axis so far, from the centre.
                                view.dashed(plane.to_world(from), plane.to_world(here), preview);
                                format!("{} (half the axis)", u.show(from.distance(here)))
                            }
                        },
                        Tool::Arc3Point | Tool::ArcCenter | Tool::ArcTangent => match self.editor.arc_preview(sk, here) {
                            Some(a) => {
                                view.arc(&plane, a.center, a.radius, a.start, a.sweep, preview);
                                // Centre snapped? Mark it (idempotent on the snapped point).
                                let g = self.grid_snap.then_some(grid);
                                let hint = match self.editor.arc3_snap(sk, here, g, tol_px(mm_per_px)).1 {
                                    sketch::Arc3Snap::HalfCircle => Some(("½", a.center)),
                                    sketch::Arc3Snap::GridCenter(c) => Some(("grid", c)),
                                    sketch::Arc3Snap::None => None,
                                };
                                if let Some((label, c)) = hint {
                                    if let Some(px) = view.project(plane.to_world(c)) {
                                        painter.circle_stroke(to_pos(px), 5.0, Stroke::new(2.0, COL_SNAP));
                                        painter.text(
                                            to_pos(px) + egui::vec2(9.0, -9.0),
                                            egui::Align2::LEFT_BOTTOM,
                                            label,
                                            egui::FontId::proportional(13.0),
                                            COL_SNAP,
                                        );
                                    }
                                }
                                format!("R {}   {:.1}°", u.show(a.radius), a.sweep.to_degrees())
                            }
                            None => {
                                // First two clicks: the chord (3-point) or radius (centre).
                                view.line(plane.to_world(from), plane.to_world(here), preview);
                                u.show(from.distance(here))
                            }
                        },
                        Tool::Slot if self.editor.second.is_none() => {
                            // The line through the centres, from the first to the cursor (becomes construction).
                            view.dashed(plane.to_world(from), plane.to_world(here), preview);
                            if let Some(px) = view.project(plane.to_world(from)) {
                                painter.circle_filled(to_pos(px), 4.5, COL_PREVIEW);
                            }
                            format!("{} between centres", u.show(from.distance(here)))
                        }
                        Tool::Slot => match self.editor.slot_shape(sk, here) {
                            Some(sh) => {
                                view.dashed(plane.to_world(sh.a), plane.to_world(sh.b), preview);
                                for c in [sh.a, sh.b] {
                                    if let Some(px) = view.project(plane.to_world(c)) {
                                        painter.circle_filled(to_pos(px), 4.5, COL_PREVIEW);
                                    }
                                }
                                let dir = (sh.b - sh.a).normalize();
                                let n = dir.perp() * sh.half;
                                view.line(plane.to_world(sh.a + n), plane.to_world(sh.b + n), preview);
                                view.line(plane.to_world(sh.a - n), plane.to_world(sh.b - n), preview);
                                let turn = dir.y.atan2(dir.x);
                                view.arc(&plane, sh.b, sh.half, turn - std::f64::consts::FRAC_PI_2, std::f64::consts::PI, preview);
                                view.arc(&plane, sh.a, sh.half, turn + std::f64::consts::FRAC_PI_2, std::f64::consts::PI, preview);
                                format!("width {}   (or type it)   length {}", u.show(2.0 * sh.half), u.show(sh.a.distance(sh.b)))
                            }
                            None => {
                                if let Some(b) = self.editor.second {
                                    view.line(plane.to_world(from), plane.to_world(sk.pos(b)), preview);
                                }
                                String::new()
                            }
                        },
                        Tool::Circle2Point | Tool::Circle3Point => match self.editor.circle_preview(sk, here) {
                            Some((c, r)) => {
                                view.circle(&plane, c, r, preview);
                                format!("Ø {}", u.show(2.0 * r))
                            }
                            None => {
                                // 3-point, first two clicks: just the chord so far.
                                view.line(plane.to_world(from), plane.to_world(here), preview);
                                String::new()
                            }
                        },
                        _ => {
                            // Show the line exactly where the click will put it,
                            // including the horizontal/vertical snap.
                            let (end, align) = self.editor.line_end(sk, at);
                            let to = sk.pos(end);
                            view.line(plane.to_world(from), plane.to_world(to), preview);
                            let tag = match align {
                                Some(sketch::Align::Horizontal) => "  H",
                                Some(sketch::Align::Vertical) => "  V",
                                None => "",
                            };
                            format!("{}{tag}", u.show(from.distance(to)))
                        }
                    };
                    if let Some(px) = response.hover_pos() {
                        painter.text(
                            px + egui::vec2(14.0, -14.0),
                            egui::Align2::LEFT_BOTTOM,
                            label,
                            egui::FontId::monospace(13.0),
                            COL_PREVIEW,
                        );
                    }
                }
            }
            // Tangent circles: once the lines are picked, show the circle.
            if self.editor.tool.tangent_lines() > 0 {
                let left = self.editor.tool.tangent_lines() - self.editor.tangent_lines.len();
                let hint = if left > 0 {
                    format!("pick {left} more line{}", if left == 1 { "" } else { "s" })
                } else {
                    let at = cursor.map(|a| sk.pos(a)).unwrap_or_default();
                    match self.editor.tangent_circle(sk, at) {
                        Ok((c, r)) => {
                            view.circle(&plane, c, r, Stroke::new(1.5, COL_PREVIEW));
                            format!("Ø {} — click to place", u.show(2.0 * r))
                        }
                        Err(e) => e,
                    }
                };
                if let Some(px) = response.hover_pos() {
                    painter.text(px + egui::vec2(14.0, -14.0), egui::Align2::LEFT_BOTTOM, hint, egui::FontId::monospace(13.0), COL_PREVIEW);
                }
            }
            status = format!(
                "{}  {}   grid {} mm{}",
                sk.plane_name(),
                status,
                (grid.step * 1000.0).round() / 1000.0,
                if self.grid_snap { " (snap)" } else { "" }
            );
        }

        crate::mark("notification (bottom-left, fades after");
        // --- notification (bottom-left, fades after a few seconds) ---
        // What's selected in the sketch measures, bottom right.
        if let Some(i) = self.active {
            let info = self.sketches[i].selection_info(&self.editor.selection);
            if !info.is_empty() {
                let text = info
                    .iter()
                    .map(|&(label, v, amount)| match amount {
                        sketch::Amount::Length => format!("{label}  {}", self.units.show(v)),
                        sketch::Amount::Area => format!("{label}  {}", self.units.show_area(v)),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let galley = painter.layout_no_wrap(text, egui::FontId::monospace(13.0), Color32::from_gray(225));
                let at = rect.right_bottom() - egui::vec2(16.0, 34.0) - galley.size();
                painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand(8.0), 6.0, Color32::from_rgba_unmultiplied(28, 28, 28, 225));
                painter.galley(at, galley, Color32::WHITE);
            }
        }

        // Sketch status, top left (Fusion has no constraint list; constraints
        // are picked on the canvas by their glyphs).
        // Top centre: the sketch (name, how constrained) and the tool in use.
        {
            let mut parts: Vec<(String, Color32)> = Vec::new();
            if let Some(i) = self.active {
                parts.push((self.sketches[i].name.clone(), Color32::from_gray(225)));
                parts.push(match self.dof {
                    Some(0) => ("fully constrained".to_string(), Color32::from_rgb(120, 220, 120)),
                    Some(n) => (format!("{n} degree{} of freedom left", if n == 1 { "" } else { "s" }), COL_UNDER),
                    None => ("not solvable".to_string(), COL_FIXED),
                });
            }
            if let Some(tool) = self.tool_name() {
                parts.push((format!("Tool: {tool}"), COL_PREVIEW));
            }
            if !parts.is_empty() {
                let mut job = egui::text::LayoutJob::default();
                for (k, (t, c)) in parts.iter().enumerate() {
                    if k > 0 {
                        job.append("   ·   ", 0.0, egui::TextFormat::simple(egui::FontId::proportional(14.0), Color32::from_gray(120)));
                    }
                    job.append(t, 0.0, egui::TextFormat::simple(egui::FontId::proportional(14.0), *c));
                }
                let galley = painter.layout_job(job);
                let at = rect.center_top() + egui::vec2(-galley.size().x / 2.0, 8.0);
                painter.rect_filled(egui::Rect::from_min_size(at, galley.size()).expand(6.0), 6.0, Color32::from_rgba_unmultiplied(28, 28, 28, 210));
                painter.galley(at, galley, Color32::WHITE);
            }
            if let (Some(msg), Some(_)) = (&self.message, self.active) {
                painter.text(rect.center_top() + egui::vec2(0.0, 58.0), egui::Align2::CENTER_TOP, msg.as_str(), egui::FontId::proportional(13.0), COL_FIXED);
            } else if let (Tool::ArcTangent, None, Some(_)) = (self.editor.tool, self.editor.start, self.active) {
                painter.text(rect.center_top() + egui::vec2(0.0, 58.0), egui::Align2::CENTER_TOP, "Click the end of a line or arc to start from", egui::FontId::proportional(15.0), Color32::from_gray(220));
            } else if let (Tool::Constrain("Symmetric"), Some(i)) = (self.editor.tool, self.active) {
                // What to pick next, in order.
                let step = self.sketches[i].symmetric_step(&self.editor.selection);
                painter.text(rect.center_top() + egui::vec2(0.0, 58.0), egui::Align2::CENTER_TOP, step, egui::FontId::proportional(15.0), Color32::from_gray(220));
            }
        }

        let mut expired = false;
        if let Some((text, since)) = &mut self.toast {
            if since.is_nan() {
                *since = now;
            }
            let age = now - *since;
            if age > 5.0 {
                expired = true;
            } else {
                let alpha = ((5.0 - age).min(1.0) * 235.0) as u8;
                let galley = painter.layout(text.clone(), egui::FontId::proportional(14.0), Color32::from_rgba_unmultiplied(255, 225, 200, alpha), 420.0);
                let at = rect.left_bottom() + egui::vec2(14.0, -40.0 - galley.size().y);
                let bg = egui::Rect::from_min_size(at, galley.size()).expand(8.0);
                painter.rect_filled(bg, 6.0, Color32::from_rgba_unmultiplied(70, 30, 30, alpha));
                painter.galley(at, galley, Color32::WHITE);
                ui.ctx().request_repaint();
            }
        }
        if expired {
            self.toast = None;
        }

        view.view_cube(&self.camera, &cells, to_vec(cube_center), CUBE_SCALE, cube_hover, self.cube_axes);

        // The browser floats over the view's left side, from the top down to the
        // status line (drawn here, on top of the model, so clicks between its
        // rows still reach the view).
        let strip = egui::Rect::from_min_max(rect.left_top() + egui::vec2(8.0, 8.0), egui::pos2(rect.left() + 300.0, rect.bottom() - 30.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(strip).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            if self.browser_open {
                self.browser(ui);
            } else if ui.button("» Browser").on_hover_text("Show the browser").clicked() {
                self.browser_open = true;
            }
        });

        // View cube controls, placed after everything else so they draw on top:
        // Home at the top-left, the view options menu at the bottom-right.
        let icon = Color32::from_gray(215);
        let home_rect = egui::Rect::from_center_size(cube_center + egui::vec2(-CUBE_MARGIN * 0.82, -CUBE_MARGIN * 0.82), egui::vec2(26.0, 24.0));
        let home = ui.put(home_rect, egui::Button::new("")).on_hover_text("Home view");
        View::house_icon(ui.painter(), home_rect.shrink2(egui::vec2(7.0, 6.0)), icon);
        if home.clicked() {
            self.move_camera(self.home, now);
        }
        // Looking straight at a face: arrows to roll the view 45° at a time (as in Fusion).
        if self.camera.face_on() {
            // Round a point a little up and left of the cube's centre, clear of it.
            let (around, radius) = (cube_center + egui::vec2(-8.0, -10.0), CUBE_SCALE * 1.85);
            for (clockwise, from, to, tip) in [(false, -48f32, -88f32, "Rotate view 45° counter-clockwise"), (true, -42f32, -2f32, "Rotate view 45° clockwise")] {
                let (from, to) = (from.to_radians(), to.to_radians());
                let mid = (from + to) / 2.0;
                let spot = egui::Rect::from_center_size(around + egui::vec2(mid.cos(), mid.sin()) * radius, egui::vec2(40.0, 40.0));
                let r = ui.interact(spot, ui.id().with(("roll", clockwise)), egui::Sense::click()).on_hover_text(tip);
                let col = if r.hovered() { COL_ARROW_HOT } else { icon };
                View::roll_arrow(ui.painter(), around, radius, from, to, col);
                if r.clicked() {
                    let to = self.camera.rolled(clockwise);
                    self.move_camera(to, now);
                }
            }
        }
        let menu_rect = egui::Rect::from_center_size(cube_center + egui::vec2(CUBE_MARGIN * 0.82, CUBE_MARGIN * 0.82), egui::vec2(26.0, 24.0));
        let menu = ui.put(menu_rect, egui::Button::new("")).on_hover_text("View options");
        View::menu_icon(ui.painter(), menu_rect, icon);
        egui::Popup::menu(&menu).show(|ui| {
            if ui.button("Go Home").clicked() {
                self.move_camera(self.home, now);
                ui.close();
            }
            if ui.button("Set current view as Home").clicked() {
                self.home = self.camera.pose();
                ui.close();
            }
            if ui.button("Reset Home").clicked() {
                self.home = OrbitCamera::default().pose();
                ui.close();
            }
            ui.separator();
            ui.checkbox(&mut self.cube_axes, "Show XYZ axes on cube");
            ui.separator();
            ui.weak("Camera");
            for p in Projection::ALL {
                if ui.radio(self.camera.projection == p, p.name()).clicked() {
                    self.camera.projection = p;
                    ui.close();
                }
            }
        });

        painter.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "{} {}  dist {}   {}",
                if self.gpu { "gpu" } else { "cpu" },
                if self.camera.is_ortho() { "ortho" } else { "persp" },
                self.units.show(self.camera.distance as f64),
                status
            ),
            egui::FontId::monospace(12.0),
            Color32::from_gray(160),
        );
    }
}
