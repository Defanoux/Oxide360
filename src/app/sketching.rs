//! Starting, editing and finishing sketches; sketch tools' actions.

use crate::*;

impl OpenCad {
    /// The Create Sketch tool (N): a plane that's already selected is used at
    /// once, otherwise click one (or a flat face) in the view.
    pub(crate) fn new_sketch_tool(&mut self) {
        if let Some(op) = self.selected_plane.take() {
            self.selected_cplane = None;
            self.cancel_all_dialogs();
            self.start_sketch(op);
        } else if let Some(k) = self.selected_cplane.take().filter(|&k| k < self.planes.len()) {
            self.cancel_all_dialogs();
            self.start_sketch_on_plane(k);
        } else {
            let was = self.face_pick && !self.plane_pick;
            self.cancel_all_dialogs();
            self.face_pick = !was;
        }
    }

    pub(crate) fn start_sketch(&mut self, plane: OriginPlane) {
        self.dialog = None;
        self.finish_sketch();
        let name = format!("Sketch{}", self.sketches.len() + 1);
        self.sketches.push(Sketch::new(name, plane));
        let i = self.sketches.len() - 1;
        self.timeline_insert(Feature::Sketch(i));
        self.enter_sketch(i);
    }

    /// The plane sketch `i` is on right now. A sketch on a face follows that
    /// face: the committed build says where the face is (the sketch itself
    /// keeps the plane it was picked on, so tracking always starts from there).
    pub(crate) fn plane_of(&self, i: usize) -> sketch::Plane {
        let sk = &self.sketches[i];
        let now = self.caches[Slot::Committed as usize]
            .as_ref()
            .filter(|c| c.sketches.get(i).is_some_and(|s| s.same_shape(sk)))
            .and_then(|c| c.model.sketch_frames.get(i).copied().flatten());
        match (sk.face, now) {
            (Some(_), Some(face)) => face.plane(),
            _ => sk.sketch_plane(),
        }
    }

    /// Is a sketch drawn? By default a sketch used by a feature hides itself
    /// (like Fusion); the browser's checkbox overrides that either way.
    pub(crate) fn sketch_visible(&self, i: usize) -> bool {
        let used = self.extrudes.iter().any(|e| e.sketch == i) || self.revolves.iter().any(|r| r.sketch == i);
        // A sketch the marker is still before doesn't exist yet.
        let built = self.timeline.iter().position(|f| *f == Feature::Sketch(i)).is_none_or(|p| p < self.applied());
        built && self.sketch_shown.get(&i).copied().unwrap_or(!used)
    }

    /// Delete a sketch and the extrudes built from it.
    pub(crate) fn delete_sketch(&mut self, i: usize) {
        if i >= self.sketches.len() {
            return;
        }
        let on_sketch = |p: &model::HolePlace| matches!(*p, model::HolePlace::SketchPoint { sketch, .. } | model::HolePlace::SketchCircle { sketch, .. } if sketch == i);
        if self.holes.iter().any(|h| h.places.iter().any(on_sketch)) {
            self.notify("A hole is placed on that sketch — delete the hole first".into());
            return;
        }
        for h in &mut self.holes {
            for p in &mut h.places {
                if let model::HolePlace::SketchPoint { sketch, .. } | model::HolePlace::SketchCircle { sketch, .. } = p {
                    if *sketch > i {
                        *sketch -= 1;
                    }
                }
            }
        }
        if self.planes.iter().any(|p| p.angle.is_some_and(|a| a.hinge.uses_sketch(i))) {
            self.notify("A plane at angle turns about a line of that sketch — delete the plane first".into());
            return;
        }
        for p in &mut self.planes {
            if let Some(a) = p.angle.as_mut() {
                a.hinge.sketch_deleted(i);
            }
        }
        // Leave sketch mode without finish_sketch (it may drop an empty sketch
        // and shift the indices under us).
        self.active = None;
        self.editor = Editor::default();
        self.undo.clear();
        self.drag = None;
        self.float = None;
        self.cancel_extrude();
        self.cancel_edge_dialog();
        let mut uses: Vec<usize> = (0..self.extrudes.len()).filter(|&k| self.extrudes[k].sketch == i).collect();
        uses.sort_unstable_by(|a, b| b.cmp(a)); // highest first, so indices stay valid
        for k in uses {
            self.delete_extrude(k);
        }
        let mut spun: Vec<usize> = (0..self.revolves.len()).filter(|&k| self.revolves[k].sketch == i).collect();
        spun.sort_unstable_by(|a, b| b.cmp(a));
        for k in spun {
            self.delete_revolve(k);
        }
        self.sketches.remove(i);
        self.timeline_remove(Feature::Sketch(i));
        for f in &mut self.timeline {
            if let Feature::Sketch(k) = f {
                if *k > i {
                    *k -= 1;
                }
            }
        }
        for e in &mut self.extrudes {
            if e.sketch > i {
                e.sketch -= 1;
            }
        }
        for r in &mut self.revolves {
            if r.sketch > i {
                r.sketch -= 1;
            }
        }
        self.sketch_shown =
            self.sketch_shown.drain().filter(|&(k, _)| k != i).map(|(k, v)| (if k > i { k - 1 } else { k }, v)).collect();
    }

    /// Start a new sketch on a flat face of a body.
    pub(crate) fn start_sketch_on_face(&mut self, face: FacePlane) {
        self.dialog = None;
        self.finish_sketch();
        let name = format!("Sketch{}", self.sketches.len() + 1);
        self.sketches.push(Sketch::on_face(name, face));
        let i = self.sketches.len() - 1;
        self.timeline_insert(Feature::Sketch(i));
        self.enter_sketch(i);
        // Centre the view on the face, like the origin-plane sketches.
        if let Some(p) = self.pending_turn.as_mut() {
            p.target = face.origin.as_vec3();
        }
    }

    /// Look At (Fusion's): glide round to look straight at a plane, from
    /// (yaw, pitch) in degrees, centred on `target` if given.
    pub(crate) fn look_at(&mut self, (yaw, pitch): (f32, f32), target: Option<Vec3>) {
        let mut to = Pose { yaw: yaw.to_radians(), pitch: pitch.to_radians(), ..self.camera.pose() };
        if let Some(t) = target {
            to.target = t;
        }
        self.pending_turn = Some(to);
        self.turn = None;
    }

    pub(crate) fn enter_sketch(&mut self, i: usize) {
        self.sketch_pattern = None;
        self.sp_applied = None;
        self.revolve_dialog = None;
        self.copy_dialog = None;
        self.dim_selection.clear();
        self.face_pick = false;
        self.face_hover = None;
        self.dialog = None;
        self.edge_dialog = None;
        self.float = None;
        self.active = Some(i);
        self.editor = Editor::default();
        self.undo.clear();
        self.message = None;
        self.drag = None;
        self.dof = solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof);
        // Turn to face the plane slowly (kinder than snapping): the viewport animates it.
        let (yaw, pitch) = self.sketches[i].view_angles();
        self.pending_turn = Some(Pose { yaw: yaw.to_radians(), pitch: pitch.to_radians(), ..self.camera.pose() });
        self.turn = None;
    }

    /// Leave sketch mode. An empty sketch nothing depends on is discarded.
    /// Returns the index of a removed sketch so callers can fix up indices.
    pub(crate) fn finish_sketch(&mut self) -> Option<usize> {
        self.cancel_sketch_pattern();
        let i = self.active.take()?;
        self.editor = Editor::default();
        self.undo.clear();
        self.message = None;
        self.drag = None;
        self.float = None;
        let used = self.extrudes.iter().any(|e| e.sketch == i) || self.revolves.iter().any(|r| r.sketch == i);
        if !self.sketches[i].entities.is_empty() || used {
            return None;
        }
        self.sketches.remove(i);
        self.timeline_remove(Feature::Sketch(i));
        for f in &mut self.timeline {
            if let Feature::Sketch(k) = f {
                if *k > i {
                    *k -= 1;
                }
            }
        }
        for e in &mut self.extrudes {
            if e.sketch > i {
                e.sketch -= 1;
            }
        }
        for r in &mut self.revolves {
            if r.sketch > i {
                r.sketch -= 1;
            }
        }
        Some(i)
    }

    pub(crate) fn edit_sketch(&mut self, mut i: usize) {
        if self.active == Some(i) {
            return;
        }
        if let Some(removed) = self.finish_sketch() {
            if removed < i {
                i -= 1;
            }
        }
        self.enter_sketch(i);
    }

    pub(crate) fn report(&mut self, r: Result<Option<Solved>, SolveError>) {
        match r {
            Ok(Some(s)) => {
                self.dof = Some(s.dof);
                self.message = None;
            }
            Ok(None) => {}
            Err(e) => {
                let text = format!("Can't do that: {e}");
                self.notify(text.clone());
                self.message = Some(text);
            }
        }
    }

    /// Show a short notice in the viewport's bottom-left corner.
    pub(crate) fn notify(&mut self, text: String) {
        self.toast = Some((text, f64::NAN)); // timestamped when drawn
    }

    // ---------- sketch editing ----------

    /// Apply the constraint action `label` to the current selection.
    pub(crate) fn constrain(&mut self, label: &str) {
        let Some(i) = self.active else { return };
        let opts = self.sketches[i].constraint_options(&self.editor.selection);
        let Some((_, cs)) = opts.into_iter().find(|(l, _)| *l == label) else { return };
        let first_new = self.sketches[i].constraints.len();
        let mut trial = self.sketches[i].clone();
        let mut driven = false;
        let r = match (label, cs.as_slice()) {
            // A dimension that would over-constrain becomes a reference one.
            ("Dimension", &[c]) => solver::add_dimension(&mut trial, c).map(|(s, d)| {
                driven = d;
                Some(s)
            }),
            // Several at once (both ends of a line onto an axis, H on several
            // lines): one at a time, skipping any already implied — it works
            // if at least one takes.
            _ if cs.len() > 1 => {
                let mut result = Err(solver::SolveError::Redundant);
                for c in &cs {
                    let mut t = trial.clone();
                    match solver::add_constraints(&mut t, std::slice::from_ref(c)) {
                        Ok(s) => {
                            trial = t;
                            result = Ok(Some(s));
                        }
                        Err(e) if result.is_err() => result = Err(e),
                        Err(_) => {}
                    }
                }
                result
            }
            _ => solver::add_constraints(&mut trial, &cs).map(Some),
        };
        let ok = r.is_ok();
        if ok {
            self.undo.push(std::mem::replace(&mut self.sketches[i], trial));
        }
        self.report(r);
        if ok && driven {
            self.notify("Already fixed by other constraints — added as a reference (driven) dimension".into());
        }
        if ok {
            self.editor.selection.clear();
            if driven {
                return;
            }
            if cs.first().is_some_and(|c| c.value().is_some()) {
                // Open the number box on the new dimension, ready to type.
                let text = self.value_text(i, first_new);
                self.float = Some(FloatEdit { target: EditTarget::Dimension(first_new), text, focus: true, keep_text: false });
            }
        }
    }

    /// A constraint button or key. With suitable geometry selected it's added
    /// right away; otherwise the constraint becomes the tool and you pick the
    /// geometry next (like Fusion).
    pub(crate) fn constraint_button(&mut self, label: &'static str) {
        let Some(i) = self.active else { return };
        let ready = self.sketches[i]
            .constraint_options(&self.editor.selection)
            .iter()
            .any(|(l, cs)| *l == label && !cs.is_empty());
        if ready {
            self.constrain(label);
        } else {
            self.editor.selection.clear();
            self.editor.set_tool(Tool::Constrain(label));
        }
    }

    /// How many picks a constraint needs before it can be added.
    pub(crate) fn picks_needed(label: &str) -> usize {
        match label {
            "Horizontal" | "Vertical" | "Fix" => 1,
            "Symmetric" => 3,
            _ => 2,
        }
    }

    /// A pick with a constraint tool (not Dimension): add it to the picks and
    /// apply the constraint as soon as they're enough. Picks that can't work
    /// together start over from the new one.
    pub(crate) fn constraint_pick(&mut self, label: &'static str, sel: Sel) {
        let Some(i) = self.active else { return };
        if self.editor.selection.contains(&sel) {
            self.editor.selection.retain(|s| *s != sel);
            return;
        }
        self.editor.selection.push(sel);
        let applies = |app: &Self| {
            app.sketches[i].constraint_options(&app.editor.selection).iter().any(|(l, cs)| *l == label && !cs.is_empty())
        };
        if !applies(self) && self.editor.selection.len() >= Self::picks_needed(label) {
            self.editor.selection = vec![sel];
        }
        if applies(self) {
            self.constrain(label);
            // Symmetric is done once (back to Select); the quick ones stay for the next.
            self.editor.set_tool(if label == "Symmetric" { Tool::Select } else { Tool::Constrain(label) });
        }
    }

    /// Dimension tool: what the current picks would dimension.
    pub(crate) fn dimension_candidate(&self) -> Option<Constraint> {
        let i = self.active?;
        self.sketches[i]
            .constraint_options(&self.editor.selection)
            .into_iter()
            .find(|(l, _)| *l == "Dimension")
            .and_then(|(_, cs)| cs.first().copied())
    }

    /// Dimension tool: add the dimension with its label where it was placed,
    /// then open its value for typing (`typed`: what was already typed).
    pub(crate) fn place_dimension(&mut self, c: Constraint, shift: DVec2, typed: Option<String>) {
        let Some(i) = self.active else { return };
        let mut trial = self.sketches[i].clone();
        match solver::add_dimension(&mut trial, c) {
            Ok((s, driven)) => {
                let ci = trial.constraints.len() - 1;
                trial.set_label_shift(ci, shift);
                self.undo.push(std::mem::replace(&mut self.sketches[i], trial));
                self.dof = Some(s.dof);
                self.message = None;
                if driven {
                    self.notify("Already fixed by other constraints — added as a reference (driven) dimension".into());
                    self.editor.selection.clear();
                    return;
                }
                let keep_text = typed.is_some();
                let text = typed.unwrap_or_else(|| self.value_text(i, ci));
                self.float = Some(FloatEdit { target: EditTarget::Dimension(ci), text, focus: true, keep_text });
            }
            Err(e) => self.report(Err(e)),
        }
        self.editor.selection.clear();
    }

    /// Where to put a size box so it doesn't cover the drag arrow: behind the
    /// arrow's start, on the side away from where it points. `fallback` when
    /// there is no arrow.
    pub(crate) fn off_arrow(&self, fallback: egui::Pos2) -> egui::Pos2 {
        match self.arrows.iter().flatten().next().copied() {
            Some((base, tip)) => {
                let dir = (tip - base).normalize_or_zero();
                to_pos(base - dir * 80.0) - egui::vec2(55.0, 14.0)
            }
            None => fallback,
        }
    }

    /// The fillet / chamfer the open size box would make right now.
    pub(crate) fn corner_preview(&self, i: usize, p: PointId) -> Option<sketch::CornerCut> {
        let text = &self.float.as_ref()?.text;
        let (d1, d2) = sketch::parse_sizes(text, self.units).unwrap_or((self.corner_size, None));
        let fillet = self.editor.tool == Tool::SketchFillet;
        self.sketches.get(i)?.corner_cut(p, fillet, d1, d2.unwrap_or(d1), self.corner_first)
    }

    /// Sketch fillet / chamfer: round or bevel the corner at point `p`.
    /// A chamfer can take two distances (`size2`); a fillet only has a radius.
    pub(crate) fn finish_corner(&mut self, p: PointId, size: f64, size2: Option<f64>) {
        let Some(i) = self.active else { return };
        let fillet = self.editor.tool == Tool::SketchFillet;
        let first = self.corner_first;
        let mut why = None;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let mut trial = sk.clone();
            let made = if fillet {
                trial.fillet_corner(p, size)
            } else {
                trial.chamfer_corner2(p, size, size2.unwrap_or(size), first)
            };
            match made {
                Ok(cs) => match solver::add_constraints(&mut trial, &cs) {
                    Ok(_) => *sk = trial,
                    Err(e) => why = Some(format!("{e}")),
                },
                Err(e) => why = Some(e),
            }
        });
        match why {
            Some(e) => self.notify(format!("Can't {}: {e}", if fillet { "fillet" } else { "chamfer" })),
            None => {
                self.corner_size = size;
                self.report(r);
            }
        }
    }

    /// Typed width for the slot being drawn (the third click, at that width).
    /// Finish the spline being drawn (Enter / Esc), with the points so far.
    pub(crate) fn finish_spline(&mut self) {
        let Some(i) = self.active else { return };
        let ed = &mut self.editor;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            for c in ed.finish_spline(sk) {
                let mut t = sk.clone();
                if solver::add_constraints(&mut t, &[c]).is_ok() {
                    *sk = t;
                }
            }
        });
        self.report(r);
    }

    pub(crate) fn finish_slot_typed(&mut self, width: f64) {
        let (Some(i), Some(a), Some(b)) = (self.active, self.editor.start, self.editor.second) else { return };
        let sk = &self.sketches[i];
        if !(width > 0.0) || !sk.is_valid_anchor(a) || !sk.is_valid_anchor(b) {
            self.notify("The width must be greater than zero".into());
            return;
        }
        let (pa, pb) = (sk.pos(a), sk.pos(b));
        let Some(dir) = (pb - pa).try_normalize() else { return };
        let at = Anchor::Free(pa + dir.perp() * (width / 2.0));
        let ed = &mut self.editor;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            for c in ed.click(sk, at) {
                let mut t = sk.clone();
                if solver::add_constraints(&mut t, &[c]).is_ok() {
                    *sk = t;
                }
            }
        });
        self.report(r);
    }

    /// Typed distance for the offset being placed (on the side the cursor is).
    pub(crate) fn finish_offset_typed(&mut self, distance: f64) {
        let (Some(Sel::Entity(e)), Some(at)) = (self.editor.selection.first().copied(), self.sketch_cursor) else { return };
        if !(distance > 0.0) {
            self.notify("The distance must be greater than zero".into());
            return;
        }
        self.sketch_op("offset", move |sk| sk.offset_constrained(e, at, Some(distance)));
    }

    /// Centre circle: a diameter was typed after placing the centre. Make the
    /// circle that size, with a diameter dimension holding it.
    /// A size typed as an expression made a dimension of value `v`: link the
    /// newest such dimension to the text, so it follows the parameters.
    pub(crate) fn link_new_dimension(&mut self, v: f64, text: &str) {
        let Some(sk) = self.active.and_then(|i| self.sketches.get_mut(i)) else { return };
        if !units::is_expression(text) {
            return;
        }
        if let Some(ci) = (0..sk.constraints.len()).rev().find(|&ci| sk.dim_expr(ci).is_none() && sk.constraints[ci].value().is_some_and(|d| (d - v).abs() < 1e-9)) {
            sk.set_dim_expr(ci, Some(text.trim().to_string()));
        }
    }

    pub(crate) fn finish_circle_typed(&mut self, d: f64) {
        let (Some(i), Some(start)) = (self.active, self.editor.start) else { return };
        if !(d > 0.0) || !self.sketches[i].is_valid_anchor(start) {
            self.notify("The diameter must be greater than zero".into());
            return;
        }
        let at = Anchor::Free(self.sketches[i].pos(start) + DVec2::new(d / 2.0, 0.0));
        let ed = &mut self.editor;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let before = sk.entities.len();
            let mut add = vec![];
            add.extend(ed.click(sk, at));
            if sk.entities.len() > before {
                add.push(Constraint::Diameter(before, d));
            }
            for c in add {
                let mut t = sk.clone();
                if solver::add_constraints(&mut t, &[c]).is_ok() {
                    *sk = t;
                }
            }
        });
        self.report(r);
    }

    /// Fusion's X: with geometry selected, switch it between normal and
    /// construction; with nothing selected, switch construction mode for what
    /// you draw next.
    pub(crate) fn toggle_construction(&mut self) {
        let Some(i) = self.active else { return };
        let sel = self.editor.selection.clone();
        if sel.iter().any(|s| matches!(s, Sel::Entity(_))) {
            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| sk.toggle_construction(&sel));
            self.report(r);
        } else {
            self.editor.construction = !self.editor.construction;
            self.notify(if self.editor.construction { "Construction mode on: new geometry is construction" } else { "Construction mode off" }.into());
        }
    }

    /// Project tool: bring the body edge `r` into sketch `i`.
    pub(crate) fn project_edge_into(&mut self, i: usize, r: EdgeRef) {
        let Some(points) = self
            .model(Slot::Committed)
            .bodies
            .iter()
            .flat_map(|b| &b.edges)
            .find(|e| e.at.mid.distance(r.mid) < 1e-6)
            .map(|e| e.points.iter().map(|p| p.as_dvec3()).collect::<Vec<_>>())
        else {
            return;
        };
        let plane = self.plane_of(i);
        let projected = project::project_edge(plane.origin.as_dvec3(), plane.u.as_dvec3(), plane.v.as_dvec3(), &points);
        let projected = match projected {
            Ok(p) => p,
            Err(e) => return self.notify(format!("Can't project: {e}")),
        };
        let mut why = None;
        let res = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let mut trial = sk.clone();
            match trial.add_projected(projected) {
                Ok(fixes) => {
                    for c in fixes {
                        let mut t = trial.clone();
                        if solver::add_constraints(&mut t, &[c]).is_ok() {
                            trial = t;
                        }
                    }
                    *sk = trial;
                }
                Err(e) => why = Some(e),
            }
        });
        match why {
            Some(e) => self.notify(e),
            None => self.report(res),
        }
    }

    /// Project tool, clicked on a body: every edge of it that can be projected.
    pub(crate) fn project_body_into(&mut self, i: usize, body: usize) {
        let Some(edges) = self.model(Slot::Committed).bodies.get(body).map(|b| {
            b.edges.iter().map(|e| e.points.iter().map(|p| p.as_dvec3()).collect::<Vec<_>>()).collect::<Vec<_>>()
        }) else {
            return;
        };
        let plane = self.plane_of(i);
        let (o, u, v) = (plane.origin.as_dvec3(), plane.u.as_dvec3(), plane.v.as_dvec3());
        let mut added = 0;
        let res = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            added = project::project_all(sk, o, u, v, &edges);
        });
        self.notify(if added == 0 { "Nothing new to project from that body".into() } else { format!("Projected {added} edges") });
        self.report(res);
    }

    /// The Mirror button: the current selection is what gets copied; next click picks the line.
    pub(crate) fn start_mirror(&mut self) {
        let sel = self.editor.selection.clone();
        if sel.is_empty() {
            self.notify("Select the geometry to mirror first (click or drag a box), then press Mirror".into());
            return;
        }
        self.mirror_objects = sel;
        self.editor.set_tool(Tool::Mirror);
        self.notify("Click the mirror line (or an origin axis)".into());
    }

    pub(crate) fn mirror_across(&mut self, line: pattern::MirrorLine) {
        let Some(i) = self.active else { return };
        let objects = std::mem::take(&mut self.mirror_objects);
        let mut why = None;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let mut trial = sk.clone();
            match pattern::mirror(&mut trial, &objects, line) {
                Ok((cs, _)) => {
                    for c in cs {
                        let mut t = trial.clone();
                        if solver::add_constraints(&mut t, &[c]).is_ok() {
                            trial = t;
                        }
                    }
                    *sk = trial;
                }
                Err(e) => why = Some(e),
            }
        });
        self.editor.selection.clear();
        self.editor.set_tool(Tool::Select);
        match why {
            Some(e) => self.notify(format!("Can't mirror: {e}")),
            None => self.report(r),
        }
    }

    /// Trim tool: cut line `e` back at the click `p`.
    pub(crate) fn trim_at(&mut self, e: usize, p: DVec2) {
        self.sketch_op("trim", move |sk| sk.trim(e, p));
    }

    /// Extend tool: run line `e` on from the end nearest `p`.
    pub(crate) fn extend_at(&mut self, e: usize, p: DVec2) {
        self.sketch_op("extend", move |sk| sk.extend_line(e, p));
    }

    /// Offset tool: copy the run `e` belongs to over to where `p` is.
    pub(crate) fn offset_at(&mut self, e: usize, p: DVec2) {
        self.sketch_op("offset", move |sk| sk.offset_constrained(e, p, None));
        self.editor.selection.clear();
    }

    /// One edit that can fail with a message and hands back constraints to add (one at a time:
    /// a tie that turns out redundant is just skipped).
    pub(crate) fn sketch_op(&mut self, what: &str, op: impl FnOnce(&mut Sketch) -> Result<Vec<Constraint>, String>) {
        let Some(i) = self.active else { return };
        let mut why = None;
        self.dim_selection.clear();
        self.editor.selection.clear();
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let mut trial = sk.clone();
            match op(&mut trial) {
                Ok(cs) => {
                    for c in cs {
                        let mut t = trial.clone();
                        if solver::add_constraints(&mut t, &[c]).is_ok() {
                            trial = t;
                        }
                    }
                    *sk = trial;
                }
                Err(e) => why = Some(e),
            }
        });
        match why {
            Some(e) => self.notify(format!("Can't {what}: {e}")),
            None => {
                self.report(r);
                // Trim and Extend stay on (used many times in a row, as in
                // Fusion); Offset makes a whole new run, so it's done once used.
                if what == "offset" {
                    self.editor.set_tool(Tool::Select);
                }
            }
        }
    }

    /// Switch dimension `ci` between driving and driven (reference).
    pub(crate) fn toggle_driven(&mut self, ci: usize) {
        let Some(i) = self.active else { return };
        self.float = None;
        let was_driven = self.sketches[i].is_driven(ci);
        let mut why = None;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            if was_driven {
                if let Err(e) = solver::make_driving(sk, ci) {
                    why = Some(format!("{e}"));
                }
            } else {
                solver::make_driven(sk, ci);
            }
        });
        match why {
            Some(e) => self.notify(format!("Can't make it drive: {e}")),
            None => {
                self.notify(if was_driven { "Dimension now drives" } else { "Dimension is now reference only" }.into());
                self.report(r);
            }
        }
    }

    /// The design's parameters for the boxes this frame (tabs share the
    /// reader); when they changed, the dimensions using them follow. And
    /// the Change Parameters window, when open.
    pub(crate) fn follow_parameters(&mut self, ctx: &egui::Context) {
        let sketches = &self.sketches;
        let feature_users: Vec<(String, String)> = self.feature_exprs.iter().map(|e| (self.feature_label(e.feature), e.text.clone())).collect();
        let used_by = |name: &str| -> Vec<String> {
            let mut users: Vec<String> = sketches.iter().filter(|sk| sk.dim_exprs.iter().flatten().any(|e| expr::uses(e, name))).map(|sk| sk.name.clone()).collect();
            users.extend(feature_users.iter().filter(|(_, t)| expr::uses(t, name)).map(|(n, _)| n.clone()));
            users
        };
        for change in self.params_window.show(ctx, &mut self.parameters, &used_by) {
            let params_window::Change::Renamed { old, new } = change;
            for sk in &mut self.sketches {
                for e in sk.dim_exprs.iter_mut().flatten() {
                    *e = expr::rename(e, &old, &new);
                }
            }
            for e in &mut self.feature_exprs {
                e.text = expr::rename(&e.text, &old, &new);
            }
        }
        // (Half-typed in the window: wait until the box is left.)
        let typing = self.params_window.open && ctx.text_edit_focused();
        expr::set_scope(self.params_seen.as_deref().filter(|_| typing).unwrap_or(&self.parameters));
        if typing || self.params_seen.as_ref() == Some(&self.parameters) {
            return;
        }
        let unit = self.units;
        let mut failed = Vec::new();
        for sk in &mut self.sketches {
            for (ci, why) in solver::follow_parameters(sk, unit) {
                failed.push(format!("{} dimension {}: {why}", sk.name, ci + 1));
            }
        }
        for e in self.feature_exprs.clone() {
            let v = expr::eval(&e.text, e.field.kind(), unit);
            let now = self.feature_value(e.feature, e.field);
            let r = match (v, now) {
                (Ok(v), Some(now)) if (v - now).abs() > 1e-9 => self.set_feature_value(e.feature, e.field, v),
                (Err(why), _) => Err(why),
                _ => Ok(()),
            };
            if let Err(why) = r {
                failed.push(format!("{}: {why}", self.feature_label(e.feature)));
            }
        }
        if !failed.is_empty() {
            self.notify(format!("Some dimensions can't follow the parameters — {}", failed.join("; ")));
        }
        self.params_seen = Some(self.parameters.clone());
    }

    pub(crate) fn set_dimension(&mut self, ci: usize, v: f64) {
        let Some(i) = self.active else { return };
        if self.sketches[i].is_driven(ci) {
            self.message = Some("That's a reference dimension — right-click it to make it drive".into());
            return;
        }
        let is_angle = matches!(self.sketches[i].constraints.get(ci), Some(Constraint::Angle(..)));
        if is_angle && !(0.0..180.0).contains(&v) {
            self.message = Some("Angles go from 0° up to (not including) 180°".into());
            return;
        }
        if !is_angle && v <= 0.0 {
            self.message = Some("Dimensions must be greater than zero".into());
            return;
        }
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            // (Keeps a chamfer's virtual sharp where it is; see the solver.)
            if solver::set_dimension_value(sk, ci, v).is_err() {
                // Couldn't solve: leave the new value in so the usual error shows.
                for k in sk.offset_group(ci) {
                    if let Some(c) = sk.constraints.get_mut(k) {
                        c.set_value(v);
                    }
                }
            }
        });
        self.report(r);
    }

    pub(crate) fn delete_selection(&mut self) {
        let Some(i) = self.active else { return };
        let sel = std::mem::take(&mut self.editor.selection);
        let dims = std::mem::take(&mut self.dim_selection);
        self.float = None; // constraint indices may shift
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            // Selected dimensions first (by index, before anything renumbers).
            let mut k = 0;
            sk.retain_constraints(|_| {
                let keep = !dims.contains(&k);
                k += 1;
                keep
            });
            sk.delete(&sel);
        });
        self.report(r);
    }

    pub(crate) fn undo_last(&mut self) {
        self.undo();
    }

    // ---------- extrude ----------

}
