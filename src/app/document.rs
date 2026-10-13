//! Documents, files, undo, the kernel cache, export.

use crate::*;

impl OpenCad {
    pub(crate) fn document(&self) -> Document {
        Document {
            sketches: self.sketches.clone(),
            extrudes: self.extrudes.clone(),
            edge_features: self.edge_features.clone(),
            revolves: self.revolves.clone(),
            copies: self.copies.clone(),
            axes: self.axes.clone(),
            planes: self.planes.clone(),
            removes: self.removes.clone(),
            units: self.units,
            version: doc::Version(self.version),
            timeline: self.timeline.clone(),
            suppressed: self.suppressed.clone(),
            view: self.view_state(),
            body_names: self.sorted_body_names(),
            body_colors: self.sorted_body_colors(),
        }
    }

    /// How many timeline entries are built: all of them unless the marker was
    /// dragged back.
    pub(crate) fn applied(&self) -> usize {
        self.marker.map_or(self.timeline.len(), |m| m.min(self.timeline.len()))
    }

    /// Roll the design to the first `n` timeline entries (Fusion's marker).
    /// Anything open that points at the part is closed first.
    pub(crate) fn set_marker(&mut self, n: usize) {
        let n = n.min(self.timeline.len());
        let new = (n < self.timeline.len()).then_some(n);
        if new == self.marker {
            return;
        }
        self.finish_sketch();
        self.cancel_extrude();
        self.cancel_revolve();
        self.cancel_edge_dialog();
        self.face_pick = false;
        self.float = None;
        self.dim_selection.clear();
        self.marker = new;
    }

    /// A new feature goes in at the marker (after everything built so far),
    /// and the marker moves past it.
    pub(crate) fn timeline_insert(&mut self, f: Feature) {
        self.marker = doc::insert_at_marker(&mut self.timeline, self.marker, f);
    }

    /// Take a feature out of the timeline, keeping the marker in place.
    pub(crate) fn timeline_remove(&mut self, f: Feature) {
        doc::forget_feature(&mut self.suppressed, f);
        self.marker = doc::remove_keeping_marker(&mut self.timeline, self.marker, f);
    }

    pub(crate) fn sorted_body_names(&self) -> Vec<(BodyId, String)> {
        let mut v: Vec<(BodyId, String)> = self.body_names.iter().map(|(&k, n)| (k, n.clone())).collect();
        v.sort();
        v
    }

    pub(crate) fn sorted_body_colors(&self) -> Vec<(BodyId, [u8; 3])> {
        let mut v: Vec<(BodyId, [u8; 3])> = self.body_colors.iter().map(|(&k, &c)| (k, c)).collect();
        v.sort();
        v
    }

    /// A body's colour: the one the user picked, else a default by the order
    /// bodies were made in.
    pub(crate) fn body_color(&self, id: BodyId) -> Color32 {
        // By how old the body is among the bodies there are now, so no two share a default.
        let at = |b: &BodyId| (self.timeline.iter().position(|f| *f == b.feature).unwrap_or(usize::MAX), b.piece);
        let rank = colors::rank(self.body_ids.iter().map(at), &at(&id));
        let c = self.body_colors.get(&id).copied().unwrap_or_else(|| colors::default_color(rank));
        Color32::from_rgb(c[0], c[1], c[2])
    }

    /// A body's name: the one the user gave it, else "Body1", "Body2"… by how
    /// new it is (the order the features that made the bodies come in the
    /// timeline, then piece number).
    pub(crate) fn body_name(&self, id: BodyId) -> String {
        if let Some(n) = self.body_names.get(&id) {
            return n.clone();
        }
        doc::default_body_name(&self.timeline, &self.body_ids, id)
    }

    /// Features were deleted: bodies they made are gone, later ones renumber.
    pub(crate) fn shift_body_keys(&mut self, revolve: bool, k: usize) {
        self.hidden_bodies = self.hidden_bodies.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        self.body_names = self.body_names.drain().filter_map(|(id, n)| Some((id.after_delete(revolve, k)?, n))).collect();
        self.body_colors = self.body_colors.drain().filter_map(|(id, c)| Some((id.after_delete(revolve, k)?, c))).collect();
        for c in &mut self.copies {
            c.bodies = c.bodies.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        }
        for r in &mut self.removes {
            r.bodies = r.bodies.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        }
        self.selected_bodies.clear();
        self.renaming = None;
        self.prune_removes();
    }

    /// A mirror / pattern was deleted: its copies are gone, later ones renumber.
    pub(crate) fn shift_copy_keys(&mut self, k: usize) {
        self.hidden_bodies = self.hidden_bodies.iter().filter_map(|id| id.after_copy_delete(k)).collect();
        self.body_names = self.body_names.drain().filter_map(|(id, n)| Some((id.after_copy_delete(k)?, n))).collect();
        self.body_colors = self.body_colors.drain().filter_map(|(id, c)| Some((id.after_copy_delete(k)?, c))).collect();
        for c in &mut self.copies {
            c.bodies = c.bodies.iter().filter_map(|id| id.after_copy_delete(k)).collect();
        }
        for r in &mut self.removes {
            r.bodies = r.bodies.iter().filter_map(|id| id.after_copy_delete(k)).collect();
        }
        self.selected_bodies.clear();
        self.renaming = None;
        self.prune_removes();
    }

    /// A Remove whose bodies were all deleted has nothing left to do: it goes.
    fn prune_removes(&mut self) {
        while let Some(k) = self.removes.iter().position(|r| r.bodies.is_empty()) {
            self.delete_remove(k);
        }
    }

    pub(crate) fn delete_remove(&mut self, k: usize) {
        if k >= self.removes.len() {
            return;
        }
        self.removes.remove(k);
        self.timeline_remove(Feature::Remove(k));
        for f in &mut self.timeline {
            if let Feature::Remove(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
    }

    /// Fusion's Remove: take the bodies out of the design from the marker on
    /// (a timeline feature; the features that made them stay).
    pub(crate) fn remove_bodies(&mut self, bodies: Vec<BodyId>) {
        if bodies.is_empty() {
            return;
        }
        let name = format!("Remove{}", self.removes.len() + 1);
        self.removes.push(model::RemoveFeature { name, bodies });
        self.timeline_insert(Feature::Remove(self.removes.len() - 1));
        self.selected_bodies.clear();
    }

    /// What's hidden and where Home is, for saving with the design.
    pub(crate) fn view_state(&self) -> ViewState {
        let mut hidden_ids: Vec<BodyId> = self.hidden_bodies.iter().copied().collect();
        hidden_ids.sort_unstable();
        let mut sketch_shown: Vec<(usize, bool)> = self.sketch_shown.iter().map(|(&i, &v)| (i, v)).collect();
        sketch_shown.sort_unstable();
        let h = self.home;
        ViewState {
            hidden_bodies: Vec::new(),
            hidden_ids,
            sketch_shown,
            marker: self.marker,
            home: Some(HomeView { yaw: h.yaw, pitch: h.pitch, target: h.target.to_array(), distance: h.distance }),
        }
    }

    /// Show the design the way it was saved (indices past the end are ignored).
    pub(crate) fn apply_view(&mut self, v: &ViewState) {
        self.hidden_bodies = v.hidden_ids.iter().copied().collect();
        self.sketch_shown = v.sketch_shown.iter().filter(|(i, _)| *i < self.sketches.len()).copied().collect();
        self.marker = v.marker.filter(|&n| n < self.timeline.len());
        self.home = match v.home {
            Some(h) => Pose { yaw: h.yaw, pitch: h.pitch, target: Vec3::from_array(h.target), distance: h.distance },
            None => OrbitCamera::default().pose(),
        };
    }

    /// Write the preferences when one changed (checked every frame; cheap).
    pub(crate) fn save_settings_if_changed(&mut self) {
        let now = self.current_settings();
        if now != self.settings_saved {
            if let Some(path) = settings::default_path() {
                let _ = settings::save(&path, &now); // a lost preference isn't worth an error box
            }
            self.settings_saved = now;
        }
    }

    /// The global settings as they are now.
    pub(crate) fn current_settings(&self) -> settings::Settings {
        settings::Settings {
            touchpad: self.touchpad,
            cube_axes: self.cube_axes,
            projection: self.camera.projection,
            measure_unit_own: self.measure_unit,
            measure_unit2: self.measure_unit2,
            measure_precision: self.measure_precision,
            measure_snap: self.measure_snap,
            measure_xyz: self.measure_xyz,
            default_units: self.default_units,
            touchpad_zoom_flip: self.touchpad_zoom_flip,
        }
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.sketches != self.saved.sketches
            || self.extrudes != self.saved.extrudes
            || self.edge_features != self.saved.edge_features
            || self.revolves != self.saved.revolves
            || self.copies != self.saved.copies
            || self.removes != self.saved.removes
            || self.units != self.saved.units
            || self.axes != self.saved.axes
            || self.planes != self.saved.planes
            || self.timeline != self.saved.timeline
    }

    pub(crate) fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| "Untitled".into())
    }

    /// Give the design a new name: a saved one's file is renamed (in the same
    /// folder); an unsaved one keeps it for its first save.
    pub(crate) fn rename_design(&mut self, new: &str) {
        let new = new.trim().trim_end_matches(".ocad").trim();
        if new.is_empty() || new.contains(['/', '\\']) {
            self.notify("A name can't be empty or contain / or \\".into());
            return;
        }
        match self.path.clone() {
            None => self.name = Some(new.to_string()),
            Some(old) => {
                let to = old.with_file_name(format!("{new}.{}", doc::EXTENSION));
                if to == old {
                    return;
                }
                if to.exists() {
                    self.alert = Some(format!("There's already a design called \"{new}\" in that folder."));
                    return;
                }
                match std::fs::rename(&old, &to) {
                    Ok(()) => self.path = Some(to),
                    Err(e) => self.alert = Some(format!("Couldn't rename the file: {e}")),
                }
            }
        }
    }

    /// Replace the whole design (New / Open) and reset every editing state.
    pub(crate) fn reset_with(&mut self, doc: Document, path: Option<PathBuf>) {
        self.active = None;
        self.editor = Editor::default();
        self.undo.clear();
        self.dof = None;
        self.message = None;
        self.drag = None;
        self.dialog = None;
        self.float = None;
        self.arrows.clear();
        self.arrow_drag = None;
        self.edge_dialog = None;
        self.sketches = doc.sketches.clone();
        self.extrudes = doc.extrudes.clone();
        self.edge_features = doc.edge_features.clone();
        self.revolves = doc.revolves.clone();
        self.copies = doc.copies.clone();
        self.removes = doc.removes.clone();
        self.units = doc.units;
        self.version = doc.version.0;
        self.name = None;
        self.selected_bodies.clear();
        self.axes = doc.axes.clone();
        self.planes = doc.planes.clone();
        self.plane_hidden.clear();
        self.plane_dialog = None;
        self.plane_pick = false;
        self.axis_hidden.clear();
        self.axis_pick = false;
        self.copy_dialog = None;
        self.revolve_dialog = None;
        self.timeline = doc.timeline.clone();
        self.suppressed = doc.suppressed.clone();
        self.renaming_feature = None;
        self.apply_view(&doc.view);
        self.body_names = doc.body_names.iter().cloned().collect();
        self.body_colors = doc.body_colors.iter().cloned().collect();
        self.renaming = None;
        self.history.reset(doc.clone());
        self.saved = doc;
        self.path = path;
    }

    // ---------- undo / redo (the whole design) ----------

    /// Record a finished change. Runs once per frame after the UI; waits while
    /// the mouse is held or a number is being typed, so a whole drag or a
    /// typed value becomes one step.
    pub(crate) fn record_history(&mut self, ctx: &egui::Context) {
        // (A label being carried on a touchpad is one step too: wait for the drop.)
        if ctx.input(|i| i.pointer.any_down()) || ctx.text_edit_focused() || self.label_carry {
            return;
        }
        self.history.record(self.document());
    }

    pub(crate) fn can_undo(&self) -> bool {
        self.editor.busy() || self.history.can_undo(&self.document())
    }

    /// Undo the last change, whatever it was: a sketch line, a constraint, an
    /// extrude, a fillet, a delete…
    pub(crate) fn undo(&mut self) {
        if self.editor.busy() {
            self.editor.cancel(); // first just drop a half-drawn line or circle
            return;
        }
        if let Some(prev) = self.history.undo(self.document()) {
            self.restore(prev);
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Some(next) = self.history.redo() {
            self.restore(next);
        }
    }

    /// Put a version of the design back, closing anything that points into
    /// the old one (dialogs, selections, drags).
    pub(crate) fn restore(&mut self, doc: Document) {
        let active = self.active.filter(|&i| i < doc.sketches.len());
        self.sketches = doc.sketches;
        self.extrudes = doc.extrudes;
        self.edge_features = doc.edge_features;
        self.revolves = doc.revolves;
        self.copies = doc.copies;
        self.removes = doc.removes;
        self.units = doc.units;
        self.selected_bodies.clear();
        self.axes = doc.axes;
        self.planes = doc.planes;
        self.plane_hidden.retain(|&k| k < self.planes.len());
        self.plane_dialog = None;
        self.plane_pick = false;
        self.axis_hidden.retain(|&k| k < self.axes.len());
        self.axis_pick = false;
        self.copy_dialog = None;
        self.timeline = doc.timeline;
        self.suppressed = doc.suppressed;
        self.renaming_feature = None;
        self.body_names = doc.body_names.into_iter().collect();
        self.body_colors = doc.body_colors.into_iter().collect();
        self.renaming = None;
        self.marker = self.marker.filter(|&n| n < self.timeline.len());
        self.dialog = None;
        self.revolve_dialog = None;
        self.edge_dialog = None;
        self.edge_hover = None;
        self.float = None;
        self.drag = None;
        self.arrow_drag = None;
        self.arrows.clear();
        self.face_pick = false;
        self.message = None;
        self.editor.selection.clear();
        self.editor.cancel();
        self.dim_selection.clear();
        self.undo.clear();
        self.active = active;
        self.dof = active.and_then(|i| solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof));
    }

    pub(crate) fn file_dialog(&self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        }
        d
    }

    pub(crate) fn save_to(&mut self, path: PathBuf) -> bool {
        // Every save is a new version (v1, v2… by the name).
        let mut doc = self.document();
        doc.version = doc::Version(self.version + 1);
        match doc::save(&path, &doc) {
            Ok(()) => {
                self.version += 1;
                self.saved = doc;
                self.path = Some(path);
                true
            }
            Err(e) => {
                self.alert = Some(e);
                false
            }
        }
    }

    pub(crate) fn save_as(&mut self) -> bool {
        let stem = self.file_name().trim_end_matches(".ocad").to_string();
        let picked = self
            .file_dialog()
            .set_title("Save design")
            .add_filter("opencad design", &[doc::EXTENSION])
            .set_file_name(format!("{stem}.{}", doc::EXTENSION))
            .save_file();
        match picked {
            Some(mut p) => {
                if p.extension().is_none_or(|e| e != doc::EXTENSION) {
                    p.set_extension(doc::EXTENSION);
                }
                self.save_to(p)
            }
            None => false, // cancelled
        }
    }

    pub(crate) fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    pub(crate) fn open_dialog(&mut self) {
        let picked = self
            .file_dialog()
            .set_title("Open design")
            .add_filter("opencad design", &[doc::EXTENSION])
            .pick_file();
        if let Some(p) = picked {
            match doc::load(&p) {
                Ok(d) => self.reset_with(d, Some(p)),
                Err(e) => self.alert = Some(format!("Couldn't open the design: {e}")),
            }
        }
    }

    /// Glide the camera to `to` (used by the view cube and Home).
    pub(crate) fn move_camera(&mut self, to: Pose, now: f64) {
        self.turn = Some(CameraTurn { from: self.camera.pose(), to, start: now, seconds: TURN_SECONDS });
    }

    /// The feature lists a slot is built from.
    pub(crate) fn inputs(&self, slot: Slot) -> (Vec<Extrude>, Vec<Revolve>, Vec<EdgeFeature>, Vec<CopyFeature>, Vec<AxisFeature>, Vec<PlaneFeature>, Vec<Feature>) {
        let (mut extrudes, mut revolves, mut edge_features, mut copies, mut timeline) =
            (self.extrudes.clone(), self.revolves.clone(), self.edge_features.clone(), self.copies.clone(), self.timeline.clone());
        let axes = self.axes.clone();
        let mut planes = self.planes.clone();
        timeline.truncate(self.applied()); // features after the marker aren't built
        timeline.retain(|f| !self.suppressed.contains(f));
        match slot {
            Slot::Committed => {}
            Slot::Preview => {
                if let (Some(ex), Some(d)) = (self.dialog_extrude(), self.dialog.as_ref()) {
                    match d.editing {
                        Some(k) => extrudes[k] = ex,
                        None => {
                            extrudes.push(ex);
                            timeline.push(Feature::Extrude(extrudes.len() - 1));
                        }
                    }
                }
                if let (Some(rv), Some(d)) = (self.dialog_revolve(), self.revolve_dialog.as_ref()) {
                    match d.editing {
                        Some(k) => revolves[k] = rv,
                        None => {
                            revolves.push(rv);
                            timeline.push(Feature::Revolve(revolves.len() - 1));
                        }
                    }
                }
                if let Some(d) = self.plane_dialog.as_ref() {
                    let p = d.feature(match d.editing {
                        Some(k) => self.planes[k].name.clone(),
                        None => format!("Plane{}", self.planes.len() + 1),
                    });
                    match d.editing {
                        Some(k) => planes[k] = p,
                        None => {
                            planes.push(p);
                            timeline.push(Feature::Plane(planes.len() - 1));
                        }
                    }
                }
                if let (Some(c), Some(d)) = (self.dialog_copy(), self.copy_dialog.as_ref()) {
                    match d.editing {
                        Some(k) => copies[k] = c,
                        None => {
                            copies.push(c);
                            timeline.push(Feature::Copy(copies.len() - 1));
                        }
                    }
                }
                if let (Some(f), Some(d)) = (self.dialog_edge_feature(), self.edge_dialog.as_ref()) {
                    match d.editing {
                        Some(k) => edge_features[k] = f,
                        None => {
                            edge_features.push(f);
                            timeline.push(Feature::Edge(edge_features.len() - 1));
                        }
                    }
                }
            }
            Slot::Base => {
                // Everything before the fillet being edited.
                if let Some(k) = self.edge_dialog.as_ref().and_then(|d| d.editing) {
                    if let Some(pos) = timeline.iter().position(|f| *f == Feature::Edge(k)) {
                        timeline.truncate(pos);
                    }
                }
            }
        }
        (extrudes, revolves, edge_features, copies, axes, planes, timeline)
    }

    /// Make sure the kernel model for `slot` matches the current design.
    /// Rebuilds only when its inputs changed.
    pub(crate) fn ensure_model(&mut self, slot: Slot) {
        let (extrudes, revolves, edge_features, copies, axes, planes, timeline) = self.inputs(slot);
        let fresh = self.caches[slot as usize].as_ref().is_some_and(|c| {
            c.sketches.len() == self.sketches.len()
                && c.sketches.iter().zip(&self.sketches).all(|(a, b)| a.same_shape(b))
                && c.extrudes == extrudes
                && c.revolves == revolves
                && c.edge_features == edge_features
                && c.copies == copies
                && c.axes == axes
                && c.planes == planes
                && c.removes == self.removes
                && c.timeline == timeline
        });
        if !fresh {
            let removes = self.removes.clone();
            let model = kernel::build_all(&self.sketches, &extrudes, &revolves, &edge_features, &copies, &axes, &planes, &removes, &timeline);
            self.caches[slot as usize] =
                Some(KernelCache { sketches: self.sketches.clone(), extrudes, revolves, edge_features, copies, axes, planes, removes, timeline, model });
        }
    }

    pub(crate) fn model(&self, slot: Slot) -> &kernel::Model {
        &self.caches[slot as usize].as_ref().expect("ensure_model runs first").model
    }

    pub(crate) fn export(&mut self, format: export::Format) {
        self.ensure_model(Slot::Committed);
        if self.model(Slot::Committed).bodies.is_empty() {
            self.alert = Some("Nothing to export yet — extrude a body first.".into());
            return;
        }
        let ext = format.extension();
        let stem = self.file_name().trim_end_matches(".ocad").to_string();
        let picked = self
            .file_dialog()
            .set_title(format!("Export {}", ext.to_uppercase()))
            .add_filter(format.label(), &[ext])
            .set_file_name(format!("{stem}.{ext}"))
            .save_file();
        let Some(mut p) = picked else { return };
        if p.extension().is_none_or(|e| e != ext) {
            p.set_extension(ext);
        }
        let model = self.model(Slot::Committed);
        let result = match format {
            export::Format::Step => kernel::write_step(&p, &model.bodies),
            _ => {
                let names: Vec<String> = model.bodies.iter().map(|b| self.body_name(b.id)).collect();
                let bodies: Vec<export::Body> = model
                    .bodies
                    .iter()
                    .zip(&names)
                    .map(|(b, name)| export::Body { name: name.clone(), mesh: &b.mesh })
                    .collect();
                export::write(&p, format, &bodies)
            }
        };
        if let Err(e) = result {
            self.alert = Some(e);
        }
    }

    /// New / Open / Quit go through here so unsaved work is never lost silently.
    pub(crate) fn request(&mut self, action: Pending, ctx: &egui::Context) {
        if self.is_dirty() {
            self.confirm = Some(action);
        } else {
            self.run(action, ctx);
        }
    }

    pub(crate) fn run(&mut self, action: Pending, ctx: &egui::Context) {
        match action {
            Pending::New => self.reset_with(Document { units: self.default_units, ..Document::default() }, None),
            Pending::Open => self.open_dialog(),
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::CloseTab => self.close_me = true,
        }
    }

    pub(crate) fn run_cmd(&mut self, cmd: Cmd, ctx: &egui::Context) {
        match cmd {
            Cmd::New => self.request(Pending::New, ctx),
            Cmd::Open => self.request(Pending::Open, ctx),
            Cmd::Save => {
                self.save();
            }
            Cmd::SaveAs => {
                self.save_as();
            }
            Cmd::Export(f) => self.export(f),
        }
    }

    /// "Save changes?" and error popups.
    pub(crate) fn modals(&mut self, ctx: &egui::Context) {
        if let Some(action) = self.confirm {
            let mut choice = None;
            let name = self.file_name();
            let _ = egui::Modal::new(egui::Id::new("confirm_save")).show(ctx, |ui| {
                ui.heading("Save changes?");
                ui.label(format!("\"{name}\" has unsaved changes."));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(0);
                    }
                    if ui.button("Don't Save").clicked() {
                        choice = Some(1);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(2);
                    }
                });
            });
            match choice {
                Some(0) => {
                    self.confirm = None;
                    if self.save() {
                        self.run(action, ctx);
                    }
                }
                Some(1) => {
                    self.confirm = None;
                    self.run(action, ctx);
                }
                Some(_) => self.confirm = None,
                None => {}
            }
        }
        if let Some(msg) = self.alert.clone() {
            let mut ok = false;
            let _ = egui::Modal::new(egui::Id::new("alert")).show(ctx, |ui| {
                ui.heading("opencad");
                ui.label(msg.as_str());
                ui.add_space(8.0);
                ok = ui.button("OK").clicked();
            });
            if ok {
                self.alert = None;
            }
        }
    }

    // ---------- sketch lifecycle ----------

}
