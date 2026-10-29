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
            holes: self.holes.clone(),
            thread_features: self.thread_features.clone(),
            shells: self.shells.clone(),
            offset_faces: self.offset_faces.clone(),
            combines: self.combines.clone(),
            moves: self.moves.clone(),
            points: self.points.clone(),
            sweeps: self.sweeps.clone(),
            imports: self.imports.clone(),
            components: self.components.clone(),
            joints: self.joints.clone(),
            sections: self.sections.clone(),
            parameters: self.parameters.clone(),
            feature_exprs: self.feature_exprs.clone(),
            units: self.units,
            version: doc::Version(self.version),
            timeline: self.timeline.clone(),
            suppressed: self.suppressed.clone(),
            view: self.view_state(),
            body_names: self.sorted_body_names(),
            body_colors: self.sorted_body_colors(),
            body_materials: self.sorted_body_materials(),
            body_appearances: self.sorted_body_appearances(),
            face_appearances: self.face_appearances.clone(),
            drawing: self.drawing.clone(),
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
        doc::forget_feature_exprs(&mut self.feature_exprs, f);
        self.marker = doc::remove_keeping_marker(&mut self.timeline, self.marker, f);
    }

    pub(crate) fn sorted_body_names(&self) -> Vec<(BodyId, String)> {
        let mut v: Vec<(BodyId, String)> = self.body_names.iter().map(|(&k, n)| (k, n.clone())).collect();
        v.sort();
        v
    }

    pub(crate) fn sorted_body_materials(&self) -> Vec<(BodyId, String)> {
        let mut v: Vec<(BodyId, String)> = self.body_materials.iter().map(|(&k, m)| (k, m.clone())).collect();
        v.sort();
        v
    }

    /// A body's physical material (Steel unless one was chosen).
    pub(crate) fn body_material(&self, id: BodyId) -> materials::Material {
        materials::by_name(self.body_materials.get(&id).map_or(materials::DEFAULT, String::as_str))
    }

    pub(crate) fn sorted_body_appearances(&self) -> Vec<(BodyId, String)> {
        let mut v: Vec<(BodyId, String)> = self.body_appearances.iter().map(|(&k, a)| (k, a.clone())).collect();
        v.sort();
        v
    }

    /// A body's appearance, if it has one this version knows.
    pub(crate) fn body_appearance(&self, id: BodyId) -> Option<&'static appearance::Appearance> {
        self.body_appearances.get(&id).and_then(|n| appearance::by_name(n))
    }

    /// How a body's surface is shaded: its appearance's finish, else plain.
    pub(crate) fn body_finish(&self, id: BodyId) -> gpu::Finish {
        self.body_appearance(id).map_or(gpu::Finish::PLAIN, |a| gpu::Finish { metal: a.metal, gloss: a.gloss, opacity: a.opacity })
    }

    pub(crate) fn sorted_body_colors(&self) -> Vec<(BodyId, [u8; 3])> {
        let mut v: Vec<(BodyId, [u8; 3])> = self.body_colors.iter().map(|(&k, &c)| (k, c)).collect();
        v.sort();
        v
    }

    /// A body's colour: its appearance's, else the one the user picked, else
    /// a default by the order bodies were made in.
    pub(crate) fn body_color(&self, id: BodyId) -> Color32 {
        if let Some(a) = self.body_appearance(id) {
            return Color32::from_rgb(a.color[0], a.color[1], a.color[2]);
        }
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
        self.body_materials = self.body_materials.drain().filter_map(|(id, m)| Some((id.after_delete(revolve, k)?, m))).collect();
        self.body_appearances = self.body_appearances.drain().filter_map(|(id, a)| Some((id.after_delete(revolve, k)?, a))).collect();
        self.face_appearances = self.face_appearances.drain(..).filter_map(|f| Some(appearance::FaceAppearance { body: f.body.after_delete(revolve, k)?, ..f })).collect();
        assembly::remap(&mut self.components, |id| id.after_delete(revolve, k));
        self.properties_of = None;
        for c in &mut self.copies {
            c.bodies = c.bodies.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        }
        for r in &mut self.removes {
            r.bodies = r.bodies.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        }
        for sh in &mut self.shells {
            sh.bodies = sh.bodies.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        }
        for c in &mut self.combines {
            c.target = c.target.and_then(|id| id.after_delete(revolve, k));
            c.tools = c.tools.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        }
        for m in &mut self.moves {
            m.bodies = m.bodies.iter().filter_map(|id| id.after_delete(revolve, k)).collect();
        }
        self.selected_bodies.clear();
        self.renaming = None;
        self.prune_removes();
    }

    /// A mirror / pattern was deleted: its copies are gone, later ones renumber.
    pub(crate) fn shift_copy_keys(&mut self, k: usize) {
        self.shift_maker_keys(Feature::Copy(k));
    }

    /// A feature that makes bodies from others (a mirror / pattern, a Move
    /// that copies) was deleted: its bodies are gone, later ones renumber.
    pub(crate) fn shift_maker_keys(&mut self, gone: Feature) {
        self.hidden_bodies = self.hidden_bodies.iter().filter_map(|id| id.after_maker_delete(gone)).collect();
        self.body_names = self.body_names.drain().filter_map(|(id, n)| Some((id.after_maker_delete(gone)?, n))).collect();
        self.body_colors = self.body_colors.drain().filter_map(|(id, c)| Some((id.after_maker_delete(gone)?, c))).collect();
        self.body_materials = self.body_materials.drain().filter_map(|(id, m)| Some((id.after_maker_delete(gone)?, m))).collect();
        self.body_appearances = self.body_appearances.drain().filter_map(|(id, a)| Some((id.after_maker_delete(gone)?, a))).collect();
        self.face_appearances = self.face_appearances.drain(..).filter_map(|f| Some(appearance::FaceAppearance { body: f.body.after_maker_delete(gone)?, ..f })).collect();
        assembly::remap(&mut self.components, |id| id.after_maker_delete(gone));
        self.properties_of = None;
        for c in &mut self.copies {
            c.bodies = c.bodies.iter().filter_map(|id| id.after_maker_delete(gone)).collect();
        }
        for r in &mut self.removes {
            r.bodies = r.bodies.iter().filter_map(|id| id.after_maker_delete(gone)).collect();
        }
        for sh in &mut self.shells {
            sh.bodies = sh.bodies.iter().filter_map(|id| id.after_maker_delete(gone)).collect();
        }
        for m in &mut self.moves {
            m.bodies = m.bodies.iter().filter_map(|id| id.after_maker_delete(gone)).collect();
        }
        for c in &mut self.combines {
            c.target = c.target.and_then(|id| id.after_maker_delete(gone));
            c.tools = c.tools.iter().filter_map(|id| id.after_maker_delete(gone)).collect();
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
            named_views: self
                .named_views
                .iter()
                .map(|(name, p)| doc::NamedView { name: name.clone(), view: HomeView { yaw: p.yaw, pitch: p.pitch, target: p.target.to_array(), distance: p.distance }, roll: p.roll })
                .collect(),
        }
    }

    /// Show the design the way it was saved (indices past the end are ignored).
    pub(crate) fn apply_view(&mut self, v: &ViewState) {
        self.hidden_bodies = v.hidden_ids.iter().copied().collect();
        self.sketch_shown = v.sketch_shown.iter().filter(|(i, _)| *i < self.sketches.len()).copied().collect();
        self.marker = v.marker.filter(|&n| n < self.timeline.len());
        self.home = match v.home {
            Some(h) => Pose { yaw: h.yaw, pitch: h.pitch, roll: 0.0, target: Vec3::from_array(h.target), distance: h.distance },
            None => OrbitCamera::default().pose(),
        };
        self.named_views = v.named_views.iter().map(|n| (n.name.clone(), Pose { yaw: n.view.yaw, pitch: n.view.pitch, roll: n.roll, target: Vec3::from_array(n.view.target), distance: n.view.distance })).collect();
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
            toolbar_hidden: self.toolbar_hidden.clone(),
            keybinds: self.keymap.to_pairs(),
            toolbar_order: self.toolbar_order.clone(),
            style: self.style,
            recent: self.recent.clone(),
        }
    }

    /// `path` was opened or saved: first in File → Recent (merged with what
    /// other windows put there meanwhile).
    pub(crate) fn remember_recent(&mut self, path: &std::path::Path) {
        let mut list = settings::default_path().map(|p| settings::load(&p).recent).unwrap_or_else(|| self.recent.clone());
        settings::remember(&mut list, path);
        self.recent = list;
    }

    pub(crate) fn is_dirty(&self) -> bool {
        self.sketches != self.saved.sketches
            || self.extrudes != self.saved.extrudes
            || self.edge_features != self.saved.edge_features
            || self.revolves != self.saved.revolves
            || self.copies != self.saved.copies
            || self.removes != self.saved.removes
            || self.holes != self.saved.holes
            || self.thread_features != self.saved.thread_features
            || self.shells != self.saved.shells
            || self.offset_faces != self.saved.offset_faces
            || self.combines != self.saved.combines
            || self.moves != self.saved.moves
            || self.parameters != self.saved.parameters
            || self.feature_exprs != self.saved.feature_exprs
            || self.units != self.saved.units
            || self.axes != self.saved.axes
            || self.planes != self.saved.planes
            || self.timeline != self.saved.timeline
            || self.points != self.saved.points
            || self.sweeps != self.saved.sweeps
            || self.imports != self.saved.imports
            || self.components != self.saved.components
            || self.joints != self.saved.joints
            || self.sections != self.saved.sections
            || self.sorted_body_materials() != self.saved.body_materials
            || self.sorted_body_appearances() != self.saved.body_appearances
            || self.face_appearances != self.saved.face_appearances
            || self.drawing != self.saved.drawing
    }

    /// Autosave this design if it has changes and it's time (see `recovery`);
    /// once it's saved (or has nothing unsaved), its autosave goes.
    pub(crate) fn autosave(&mut self, now: f64) {
        let Some(dir) = recovery::default_dir() else { return };
        if !self.is_dirty() {
            if std::mem::take(&mut self.autosaved) {
                recovery::remove(&dir, std::process::id(), self.tab_id);
            }
            self.last_autosave = now;
            return;
        }
        if now - self.last_autosave >= recovery::every_seconds() {
            self.last_autosave = now;
            let title = self.tab_title();
            if recovery::save(&dir, std::process::id(), self.tab_id, &self.document(), self.path.as_deref(), &title).is_ok() {
                self.autosaved = true;
            }
        }
    }

    /// Open a recovered autosave in this (fresh) tab: unsaved, pointing at its own file.
    pub(crate) fn open_recovered(&mut self, r: &recovery::Recovered) -> Result<(), String> {
        let d = doc::load(&r.file)?;
        self.reset_with(d, r.original.clone());
        // Unsaved against what's on disk (or all of it, if it never was saved).
        self.saved = r.original.as_deref().and_then(|p| doc::load(p).ok()).unwrap_or_default();
        Ok(())
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
        self.holes = doc.holes.clone();
        self.thread_features = doc.thread_features.clone();
        self.shells = doc.shells.clone();
        self.shell_dialog = None;
        self.offset_faces = doc.offset_faces.clone();
        self.offset_dialog = None;
        self.combines = doc.combines.clone();
        self.combine_dialog = None;
        self.moves = doc.moves.clone();
        self.move_dialog = None;
        self.sections = doc.sections.clone();
        self.points = doc.points.clone();
        self.sweeps = doc.sweeps.clone();
        self.imports = doc.imports.clone();
        self.components = doc.components.clone();
        self.joints = doc.joints.clone();
        self.joint_dialog = None;
        self.renaming_component = None;
        self.sweep_dialog = None;
        self.point_pick = false;
        self.section_dialog = None;
        self.parameters = doc.parameters.clone();
        self.feature_exprs = doc.feature_exprs.clone();
        self.params_seen = None;
        self.thread_dialog = None;
        self.hole_dialog = None;
        self.units = doc.units;
        self.version = doc.version.0;
        self.name = None;
        self.selected_bodies.clear();
        self.axes = doc.axes.clone();
        self.planes = doc.planes.clone();
        self.plane_hidden.clear();
        self.point_hidden.clear();
        self.close_plane_dialog();
        self.plane_pick = false;
        self.axis_hidden.clear();
        self.axis_pick = false;
        self.angle_pick = false;
        self.copy_dialog = None;
        self.revolve_dialog = None;
        self.timeline = doc.timeline.clone();
        self.suppressed = doc.suppressed.clone();
        self.renaming_feature = None;
        self.apply_view(&doc.view);
        self.body_names = doc.body_names.iter().cloned().collect();
        self.body_colors = doc.body_colors.iter().cloned().collect();
        self.body_materials = doc.body_materials.iter().cloned().collect();
        self.body_appearances = doc.body_appearances.iter().cloned().collect();
        self.face_appearances = doc.face_appearances.clone();
        self.drawing = doc.drawing.clone();
        self.properties_of = None;
        self.interference = None;
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
        self.holes = doc.holes;
        self.thread_features = doc.thread_features;
        self.shells = doc.shells;
        self.shell_dialog = None;
        self.offset_faces = doc.offset_faces;
        self.offset_dialog = None;
        self.combines = doc.combines;
        self.combine_dialog = None;
        self.moves = doc.moves;
        self.move_dialog = None;
        self.sections = doc.sections;
        self.points = doc.points;
        self.sweeps = doc.sweeps;
        self.imports = doc.imports;
        self.components = doc.components;
        self.joints = doc.joints;
        self.joint_dialog = None;
        self.renaming_component = None;
        self.sweep_dialog = None;
        self.parameters = doc.parameters;
        self.feature_exprs = doc.feature_exprs;
        self.params_seen = None;
        self.thread_dialog = None;
        self.hole_dialog = None;
        self.units = doc.units;
        self.selected_bodies.clear();
        self.axes = doc.axes;
        self.planes = doc.planes;
        self.plane_hidden.retain(|&k| k < self.planes.len());
        self.point_hidden.retain(|&k| k < self.points.len());
        self.close_plane_dialog();
        self.plane_pick = false;
        self.axis_hidden.retain(|&k| k < self.axes.len());
        self.axis_pick = false;
        self.angle_pick = false;
        self.copy_dialog = None;
        self.timeline = doc.timeline;
        self.suppressed = doc.suppressed;
        self.renaming_feature = None;
        self.body_names = doc.body_names.into_iter().collect();
        self.body_colors = doc.body_colors.into_iter().collect();
        self.body_materials = doc.body_materials.into_iter().collect();
        self.body_appearances = doc.body_appearances.into_iter().collect();
        self.face_appearances = doc.face_appearances;
        self.drawing = doc.drawing;
        self.properties_of = None;
        self.interference = None;
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
                self.remember_recent(&path);
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
            .add_filter("Oxide360 design", &[doc::EXTENSION])
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
            .add_filter("Oxide360 design", &[doc::EXTENSION, doc::OLD_EXTENSION])
            .pick_file();
        if let Some(p) = picked {
            self.open_in_tab = Some(p);
            self.tab_request = Some(TabRequest::Open);
        }
    }

    /// Open the design at `p` here (its file read and checked first).
    pub(crate) fn open_path(&mut self, p: PathBuf) {
        match doc::load(&p) {
            Ok(d) => {
                self.remember_recent(&p);
                self.reset_with(d, Some(p));
            }
            Err(e) => self.alert = Some(format!("Couldn't open {}: {e}", p.display())),
        }
    }

    /// Fit (F6): glide to show every visible body and sketch, looking the same way.
    pub(crate) fn fit_view(&mut self, aspect: f32, now: f64) {
        // Just opened, still building: finish first, so there's something to fit.
        self.build_now(Slot::Committed);
        let mut pts: Vec<Vec3> = Vec::new();
        // With a dialog open, what it's previewing counts too.
        let slots: &[Slot] = if self.active.is_none() && !self.free_view() { &[Slot::Committed, Slot::Preview] } else { &[Slot::Committed] };
        for c in slots.iter().filter_map(|&k| self.caches[k as usize].as_ref()) {
            for b in c.model.bodies.iter().filter(|b| !self.hidden_bodies.contains(&b.id)) {
                // Its box's corners are enough.
                let (lo, hi) = b.mesh.positions.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
                if lo.x <= hi.x {
                    pts.extend((0..8).map(|k| Vec3::new(if k & 1 == 0 { lo.x } else { hi.x }, if k & 2 == 0 { lo.y } else { hi.y }, if k & 4 == 0 { lo.z } else { hi.z })));
                }
            }
        }
        for j in 0..self.sketches.len() {
            if self.active != Some(j) && !self.pickable_sketch(j) {
                continue;
            }
            let (sk, plane) = (&self.sketches[j], self.plane_of(j));
            let mut flat: Vec<DVec2> = Vec::new();
            for (e, ent) in sk.entities.iter().enumerate() {
                match *ent {
                    sketch::Entity::Circle { center, radius } => {
                        let c = sk.points[center];
                        flat.extend([c - DVec2::splat(radius), c + DVec2::splat(radius)]);
                    }
                    sketch::Entity::Arc { .. } => {
                        if let Some((c, r, _, _)) = sk.arc_geometry(e) {
                            flat.extend([c - DVec2::splat(r), c + DVec2::splat(r)]);
                        }
                    }
                    sketch::Entity::Spline { s } => flat.extend(sk.spline_curve(s)),
                    sketch::Entity::Ellipse { .. } => flat.extend(sk.ellipse_curve(e)),
                    sketch::Entity::Text { .. } => flat.extend(sk.text_box(e).into_iter().flatten()),
                    _ => flat.extend(sk.entity_points(e).into_iter().map(|p| sk.points[p])),
                }
            }
            pts.extend(flat.into_iter().map(|p| plane.to_world(p)));
        }
        match self.camera.fit(&pts, aspect) {
            Some(to) => self.move_camera(to, now),
            None => self.notify("Nothing to fit yet".into()),
        }
    }

    /// Glide the camera to `to` (used by the view cube and Home).
    pub(crate) fn move_camera(&mut self, to: Pose, now: f64) {
        self.turn = Some(CameraTurn { from: self.camera.pose(), to, start: now, seconds: TURN_SECONDS });
    }

    /// The feature lists a slot is built from.
    pub(crate) fn inputs(&self, slot: Slot) -> kernel::BuildInputs {
        let (mut extrudes, mut revolves, mut edge_features, mut copies, mut timeline) =
            (self.extrudes.clone(), self.revolves.clone(), self.edge_features.clone(), self.copies.clone(), self.timeline.clone());
        let axes = self.axes.clone();
        let mut planes = self.planes.clone();
        let mut holes = self.holes.clone();
        let mut thread_features = self.thread_features.clone();
        let mut shells = self.shells.clone();
        let mut offset_faces = self.offset_faces.clone();
        let mut combines = self.combines.clone();
        let mut moves = self.moves.clone();
        let mut sweeps = self.sweeps.clone();
        let mut imports = self.imports.clone();
        let mut joints = self.joints.clone();
        timeline.truncate(self.applied()); // features after the marker aren't built
        // Editing a sketch: the part as it was there (Fusion rolls the timeline
        // back to the sketch), so what you see and project existed before it.
        if let Some(p) = self.active.and_then(|i| timeline.iter().position(|f| *f == Feature::Sketch(i))) {
            timeline.truncate(p + 1);
        }
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
                if let Some(d) = self.shell_dialog.as_ref().filter(|d| !d.shell.faces.is_empty() || !d.shell.bodies.is_empty()) {
                    match d.editing {
                        Some(k) => shells[k] = d.shell.clone(),
                        None => {
                            shells.push(d.shell.clone());
                            timeline.push(Feature::Shell(shells.len() - 1));
                        }
                    }
                }
                if let Some(sf) = self.sweep_dialog.as_ref().and_then(|d| d.shown.clone()) {
                    match self.sweep_dialog.as_ref().and_then(|d| d.editing) {
                        Some(k) => sweeps[k] = sf,
                        None => {
                            sweeps.push(sf);
                            timeline.push(Feature::Sweep(sweeps.len() - 1));
                        }
                    }
                }
                if let Some(j) = self.joint_dialog.as_ref().and_then(|d| d.feature().map(|j| (d.editing, j))) {
                    match j.0 {
                        Some(k) => joints[k] = j.1,
                        None => {
                            joints.push(j.1);
                            timeline.push(Feature::Joint(joints.len() - 1));
                        }
                    }
                }
                if let Some(d) = self.import_dialog.as_ref() {
                    match d.editing {
                        Some(k) => imports[k] = d.feature.clone(),
                        None => {
                            imports.push(d.feature.clone());
                            timeline.push(Feature::Import(imports.len() - 1));
                        }
                    }
                }
                if let Some(d) = self.move_dialog.as_ref().filter(|d| !d.feature.bodies.is_empty()) {
                    match d.editing {
                        Some(k) => moves[k] = d.feature.clone(),
                        None => {
                            moves.push(d.feature.clone());
                            timeline.push(Feature::Move(moves.len() - 1));
                        }
                    }
                }
                if let Some(d) = self.combine_dialog.as_ref().filter(|d| d.ready()) {
                    match d.editing {
                        Some(k) => combines[k] = d.combine.clone(),
                        None => {
                            combines.push(d.combine.clone());
                            timeline.push(Feature::Combine(combines.len() - 1));
                        }
                    }
                }
                if let Some(d) = self.offset_dialog.as_ref().filter(|d| !d.offset.faces.is_empty()) {
                    match d.editing {
                        Some(k) => offset_faces[k] = d.offset.clone(),
                        None => {
                            offset_faces.push(d.offset.clone());
                            timeline.push(Feature::OffsetFace(offset_faces.len() - 1));
                        }
                    }
                }
                if let Some(d) = self.thread_dialog.as_ref().filter(|d| !d.shown.faces.is_empty()) {
                    match d.editing {
                        Some(k) => thread_features[k] = d.shown.clone(),
                        None => {
                            thread_features.push(d.shown.clone());
                            timeline.push(Feature::Thread(thread_features.len() - 1));
                        }
                    }
                }
                if let Some(d) = self.hole_dialog.as_ref().filter(|d| !d.shown.places.is_empty()) {
                    match d.editing {
                        Some(k) => holes[k] = d.shown.clone(),
                        None => {
                            holes.push(d.shown.clone());
                            timeline.push(Feature::Hole(holes.len() - 1));
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
        kernel::BuildInputs {
            sketches: self.sketches.clone(),
            extrudes,
            revolves,
            edge_features,
            copies,
            axis_features: axes,
            plane_features: planes,
            removes: self.removes.clone(),
            holes,
            thread_features,
            shells,
            offset_faces,
            combines,
            moves,
            points: self.points.clone(),
            sweeps,
            imports,
            joints,
            components: self.components.clone(),
            timeline,
        }
    }

    /// Make sure the kernel model for `slot` matches the current design.
    /// Rebuilds only when its inputs changed.
    pub(crate) fn ensure_model(&mut self, slot: Slot) {
        let inp = self.inputs(slot);
        let fresh = self.caches[slot as usize].as_ref().is_some_and(|c| {
            c.sketches.len() == inp.sketches.len()
                && c.sketches.iter().zip(&inp.sketches).all(|(a, b)| a.same_shape(b))
                && c.extrudes == inp.extrudes
                && c.revolves == inp.revolves
                && c.edge_features == inp.edge_features
                && c.copies == inp.copies
                && c.axes == inp.axis_features
                && c.planes == inp.plane_features
                && c.removes == inp.removes
                && c.holes == inp.holes
                && c.thread_features == inp.thread_features
                && c.shells == inp.shells
                && c.offset_faces == inp.offset_faces
                && c.combines == inp.combines
                && c.moves == inp.moves
                && c.points == inp.points
                && c.sweeps == inp.sweeps
                && c.imports == inp.imports
                && c.joints == inp.joints
                && c.components == inp.components
                && c.timeline == inp.timeline
        });
        let i = slot as usize;
        if fresh {
            self.builds[i] = None;
            return;
        }
        // Build a little each frame (the last model stays up meanwhile), so the
        // window keeps drawing and shows how far it's got. A quick build is
        // done within the frame, as before.
        let same = |b: &kernel::Build| {
            let a = b.inputs();
            a.sketches.len() == inp.sketches.len()
                && a.sketches.iter().zip(&inp.sketches).all(|(x, y)| x.same_shape(y))
                && kernel::BuildInputs { sketches: Vec::new(), ..a.clone() } == kernel::BuildInputs { sketches: Vec::new(), ..inp.clone() }
        };
        if !self.builds[i].as_ref().is_some_and(same) {
            self.builds[i] = Some(kernel::Build::start(inp));
        } else if self.stepped[i] == self.frame_no && self.caches[i].is_some() {
            return; // (once a frame)
        }
        self.stepped[i] = self.frame_no;
        let more = self.builds[i].as_mut().is_some_and(|b| b.step(std::time::Duration::from_millis(25)));
        if more {
            if self.caches[i].is_none() {
                // Nothing to show yet: an empty model until it's done.
                self.caches[i] = Some(KernelCache { model: kernel::Model::default(), ..KernelCache::empty() });
            }
            return;
        }
        let Some(b) = self.builds[i].take() else { return };
        let inp = b.inputs().clone();
        let model = b.finish();
        self.caches[i] = Some(KernelCache {
            sketches: inp.sketches,
            extrudes: inp.extrudes,
            revolves: inp.revolves,
            edge_features: inp.edge_features,
            copies: inp.copies,
            axes: inp.axis_features,
            planes: inp.plane_features,
            removes: inp.removes,
            holes: inp.holes,
            thread_features: inp.thread_features,
            shells: inp.shells,
            offset_faces: inp.offset_faces,
            combines: inp.combines,
            moves: inp.moves,
            points: inp.points,
            sweeps: inp.sweeps,
            imports: inp.imports,
            joints: inp.joints,
            components: inp.components,
            timeline: inp.timeline,
            model,
            stand_in: false,
        });
    }

    /// Take in the sketches the last committed build moved (linked
    /// projections following the part), unless they've been edited since.
    pub(crate) fn adopt_linked_sketches(&mut self) {
        let Some(c) = self.caches[Slot::Committed as usize].as_ref() else { return };
        let mut moved = Vec::new();
        for (k, m) in c.model.linked_sketches.iter().enumerate() {
            let (Some(m), Some(built), Some(now)) = (m, c.sketches.get(k), self.sketches.get(k)) else { continue };
            if now.same_shape(built) && !now.same_shape(m) {
                moved.push((k, built.clone(), m.clone()));
            }
        }
        for (k, built, m) in moved {
            let sk = &mut self.sketches[k];
            sk.points = m.points.clone();
            sk.entities = m.entities.clone();
            let to = sk.clone();
            self.history.adopt_sketch(k, &built, &to);
        }
    }

    /// Finish `slot`'s build now, however long it takes (export needs the real thing).
    pub(crate) fn build_now(&mut self, slot: Slot) {
        let i = slot as usize;
        loop {
            self.stepped[i] = u64::MAX; // (any number of goes this frame)
            self.ensure_model(slot);
            if self.builds[i].is_none() {
                break;
            }
        }
    }

    /// Is there a real model for `slot` (not just the stand-in while its first build runs)?
    pub(crate) fn ready(&self, slot: Slot) -> bool {
        self.caches[slot as usize].as_ref().is_some_and(|c| !c.stand_in)
    }

    /// The green bar (bottom right) while a rebuild is under way.
    pub(crate) fn build_progress(&self, ctx: &egui::Context) {
        let Some((done, all, next)) = self.builds.iter().flatten().map(|b| b.progress()).max_by_key(|p| p.1) else { return };
        let label = match next {
            Some(f) => format!("Building {}", self.feature_label(f)),
            None => "Finishing".into(),
        };
        let frac = (done as f32 + 0.5) / (all as f32 + 1.0);
        egui::Area::new(egui::Id::new("build_progress"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -self.timeline_height - 14.0))
            .interactable(false)
            .show(ctx, |ui| view::progress_card(ui, &label, frac));
    }

    /// A timeline entry's name ("Hole1", "Extrude2").
    pub(crate) fn feature_label(&self, f: Feature) -> String {
        match f {
            Feature::Sketch(i) => self.sketches.get(i).map(|s| s.name.clone()),
            Feature::Extrude(k) => self.extrudes.get(k).map(|e| e.name.clone()),
            Feature::Edge(k) => self.edge_features.get(k).map(|e| e.name.clone()),
            Feature::Revolve(k) => self.revolves.get(k).map(|r| r.name.clone()),
            Feature::Copy(k) => self.copies.get(k).map(|c| c.name.clone()),
            Feature::Axis(k) => self.axes.get(k).map(|a| a.name.clone()),
            Feature::Plane(k) => self.planes.get(k).map(|p| p.name.clone()),
            Feature::Remove(k) => self.removes.get(k).map(|r| r.name.clone()),
            Feature::Hole(k) => self.holes.get(k).map(|h| h.name.clone()),
            Feature::Thread(k) => self.thread_features.get(k).map(|t| t.name.clone()),
            Feature::Shell(k) => self.shells.get(k).map(|s| s.name.clone()),
            Feature::OffsetFace(k) => self.offset_faces.get(k).map(|f| f.name.clone()),
            Feature::Combine(k) => self.combines.get(k).map(|c| c.name.clone()),
            Feature::Move(k) => self.moves.get(k).map(|m| m.name.clone()),
            Feature::Point(k) => self.points.get(k).map(|p| p.name.clone()),
            Feature::Sweep(k) => self.sweeps.get(k).map(|s| s.name.clone()),
            Feature::Import(k) => self.imports.get(k).map(|s| s.name.clone()),
            Feature::Joint(k) => self.joints.get(k).map(|j| j.name.clone()),
        }
        .unwrap_or_else(|| format!("{f:?}"))
    }

    pub(crate) fn model(&self, slot: Slot) -> &kernel::Model {
        &self.caches[slot as usize].as_ref().expect("ensure_model runs first").model
    }

    pub(crate) fn export(&mut self, format: export::Format) {
        self.build_now(Slot::Committed); // export what's really there, not a build half done
        // Only what's shown, as Fusion exports what you see (hide a body to leave it out).
        let shown: Vec<usize> = self.model(Slot::Committed).bodies.iter().enumerate().filter(|(_, b)| !self.hidden_bodies.contains(&b.id)).map(|(k, _)| k).collect();
        if shown.is_empty() {
            self.alert = Some(if self.model(Slot::Committed).bodies.is_empty() { "Nothing to export yet — extrude a body first." } else { "Every body is hidden — show the ones to export." }.into());
            return;
        }
        let ext = format.extension();
        // Named with the version ("bracket v3.step"), so several exports say which is which.
        let stem = self.tab_title();
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
        let visible: Vec<&kernel::Body> = shown.iter().map(|&k| &model.bodies[k]).collect();
        let result = match format {
            export::Format::Step => kernel::write_step(&p, &visible),
            _ => {
                let names: Vec<String> = visible.iter().map(|b| self.body_name(b.id)).collect();
                let bodies: Vec<export::Body> = visible
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

    /// A STEP / STL file's bodies, to be placed in the Insert dialog and then
    /// added at the timeline marker (Fusion's Insert). The geometry is kept in
    /// the design, so the file isn't needed again.
    pub(crate) fn import_path(&mut self, path: &std::path::Path) -> Result<(), String> {
        let file = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let brep = kernel::import_file(path).map_err(|e| format!("Couldn't import {file}: {e}"))?;
        let stem = path.file_stem().map_or_else(|| "Import".into(), |s| s.to_string_lossy().into_owned());
        let name = if self.imports.iter().any(|i| i.name == stem) { format!("{stem} ({})", self.imports.len() + 1) } else { stem };
        // Placed in the Insert dialog first (it joins the timeline on OK).
        self.finish_sketch();
        self.cancel_all_dialogs();
        self.import_dialog = Some(import_dialog::ImportDialog::new(model::ImportFeature { name, source: file, brep: model::Blob::new(brep), offset: [0.0; 3], angles: [0.0; 3] }));
        Ok(())
    }

    /// Sketch `i` as a DXF (or SVG) drawing, as Fusion's Save As DXF.
    pub(crate) fn save_sketch_drawing(&mut self, i: usize, svg: bool) {
        let Some(sk) = self.sketches.get(i) else { return };
        let ext = if svg { "svg" } else { "dxf" };
        let picked = self
            .file_dialog()
            .set_title(format!("Save {} As {}", sk.name, ext.to_uppercase()))
            .add_filter(if svg { "SVG drawing" } else { "DXF drawing" }, &[ext])
            .set_file_name(format!("{}.{ext}", sk.name))
            .save_file();
        let Some(mut p) = picked else { return };
        if p.extension().is_none_or(|e| e != ext) {
            p.set_extension(ext);
        }
        let text = if svg { sketch_export::svg(sk) } else { sketch_export::dxf(sk) };
        if let Err(e) = std::fs::write(&p, text) {
            self.alert = Some(format!("Couldn't save {}: {e}", p.display()));
        }
    }

    pub(crate) fn run(&mut self, action: Pending, ctx: &egui::Context) {
        match action {
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Pending::CloseTab => self.close_me = true,
        }
    }

    pub(crate) fn run_cmd(&mut self, cmd: Cmd) {
        match cmd {
            // Like Fusion: a new design opens in a tab of its own.
            Cmd::New => self.tab_request = Some(TabRequest::New),
            // Like Fusion: a design opens in a tab of its own, so nothing here is lost.
            Cmd::Open => self.open_dialog(),
            Cmd::OpenRecent(k) => {
                if let Some(p) = self.recent.get(k).cloned() {
                    self.open_in_tab = Some(p);
                    self.tab_request = Some(TabRequest::Open);
                }
            }
            Cmd::Save => {
                self.save();
            }
            Cmd::SaveAs => {
                self.save_as();
            }
            Cmd::Export(f) => self.export(f),
            Cmd::Import => {
                let picked = self.file_dialog().set_title("Insert into Design").add_filter("STEP or STL", &["step", "stp", "stl", "STEP", "STP", "STL"]).pick_file();
                if let Some(p) = picked {
                    if let Err(e) = self.import_path(&p) {
                        self.alert = Some(e);
                    }
                }
            }
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
                ui.heading("Oxide360");
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
