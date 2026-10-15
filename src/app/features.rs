//! Extrude / revolve / pattern / plane / axis / fillet dialogs and their drag arrows.

use crate::*;

impl OpenCad {
    pub(crate) fn open_extrude(&mut self, editing: Option<usize>) {
        self.axis_pick = false;
        self.angle_pick = false;
        self.hole_dialog = None;
        self.shell_dialog = None;
        self.offset_dialog = None;
        self.combine_dialog = None;
        self.thread_dialog = None;
        self.finish_sketch();
        self.cancel_edge_dialog();
        self.cancel_revolve();
        self.message = None;
        let d = match editing.and_then(|k| self.extrudes.get(k)) {
            Some(e) => ExtrudeDialog {
                op_chosen: true,
                cut_aimed_for: e.sketch.into(),
                editing,
                sketch: Some(e.sketch),
                profiles: e.profiles.clone(),
                distance: e.distance,
                extent: e.extent,
                distance2: e.distance2,
                taper: e.taper,
                op: e.op,
                target: e.target,
                picking_target: false,
            },
            None => ExtrudeDialog {
                op_chosen: false,
                cut_aimed_for: None,
                editing: None,
                sketch: None,
                profiles: Vec::new(),
                distance: 10.0,
                extent: model::Extent::OneSide,
                distance2: 10.0,
                taper: 0.0,
                op: Operation::NewBody, // follows what the profile touches until chosen
                target: None,
                picking_target: false,
            },
        };
        self.float = Some(FloatEdit {
            target: EditTarget::ExtrudeDistance,
            text: self.units.plain(d.distance),
            focus: true,
            keep_text: false,
        });
        self.dialog = Some(d);
        if editing.is_none() {
            self.pick_only_profile();
        }
    }

    /// A new Extrude/Revolve starts with the profile picked when the shown
    /// sketches have only one.
    fn pick_only_profile(&mut self) {
        let shown = (0..self.sketches.len()).filter(|&s| self.sketch_visible(s)).map(|s| (s, &self.sketches[s]));
        if let (Some((s, ri)), Some((sketch, profiles))) = (profile::only_profile(shown), self.profile_pick_mut()) {
            *sketch = Some(s);
            *profiles = vec![ri];
        }
    }

    pub(crate) fn dialog_extrude(&self) -> Option<Extrude> {
        let d = self.dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.extrudes[k].name.clone(),
            None => format!("Extrude{}", self.extrudes.len() + 1),
        };
        Some(Extrude { op: d.op, name, sketch: d.sketch?, profiles: d.profiles.clone(), distance: d.distance, extent: d.extent, distance2: d.distance2, taper: d.taper, target: d.target })
    }

    pub(crate) fn commit_extrude(&mut self) {
        let Some(ex) = self.dialog_extrude() else {
            self.message = Some("Select at least one profile".into());
            return;
        };
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't extrude: {e}"));
            return;
        }
        // A plane it ran up to has done its job: hide it, as Fusion does.
        self.plane_hidden.extend(ex.plane_used());
        match self.dialog.as_ref().and_then(|d| d.editing) {
            Some(k) => self.extrudes[k] = ex,
            None => {
                self.extrudes.push(ex);
                self.timeline_insert(Feature::Extrude(self.extrudes.len() - 1));
            }
        }
        self.dialog = None;
        self.float = None;
        self.message = None;
        self.arrows.clear();
    }

    pub(crate) fn cancel_extrude(&mut self) {
        self.dialog = None;
        self.float = None;
        self.message = None;
        self.arrow_drag = None;
        self.arrows.clear();
    }

    /// Put the cursor back in the number box with its text selected.
    pub(crate) fn focus_distance(&mut self) {
        if let Some(fe) = self.float.as_mut() {
            if matches!(fe.target, EditTarget::ExtrudeDistance | EditTarget::EdgeSize | EditTarget::RevolveAngle | EditTarget::PlaneOffset | EditTarget::PlaneAngle | EditTarget::OffsetDistance | EditTarget::Corner(_)) {
                fe.focus = true;
            }
        }
    }

    /// The drag arrow for whichever box is open (extrude distance, fillet /
    /// chamfer size, revolve angle), with the value it shows now.
    pub(crate) fn current_handles(&self, regions: &[Vec<Region>]) -> Vec<Handle> {
        if let (Some(d), Some(i)) = (&self.sketch_pattern, self.active) {
            let plane = self.plane_of(i);
            let at = plane.to_world(d.centroid()).as_dvec3();
            return match d.mode {
                SpMode::Rectangular => vec![
                    Handle::Line { line: LineHandle { origin: at, dir: plane.u.as_dvec3() }, value: d.spacing, target: HandleTarget::SketchSpacing1 },
                    Handle::Line { line: LineHandle { origin: at, dir: plane.v.as_dvec3() }, value: d.spacing2, target: HandleTarget::SketchSpacing2 },
                ],
                SpMode::Circular => {
                    let pivot = plane.to_world(d.center).as_dvec3();
                    let mut v: Vec<Handle> = TurnHandle::new(pivot, plane.normal().as_dvec3(), at)
                        .map(|turn| Handle::Turn { turn, value: d.angle, target: HandleTarget::SketchAngle })
                        .into_iter()
                        .collect();
                    // The distance arrow runs from the selection to the centre it turns about.
                    let out = d.toward_center();
                    let dir = (plane.u * out.x as f32 + plane.v * out.y as f32).as_dvec3();
                    v.push(Handle::Line { line: LineHandle { origin: at, dir }, value: d.radius(), target: HandleTarget::SketchRadial });
                    v
                }
            };
        }
        if let Some(d) = &self.shell_dialog {
            // The wall arrow, from the first face opened: into the part for an
            // inside wall, out of it for an outside one.
            let Some(p) = d.shell.faces.first() else { return Vec::new() };
            let at = p.face.origin + p.face.u * p.anchor[0] + p.face.v * p.anchor[1];
            let (dir, value) = match d.shell.direction {
                model::ShellDirection::Outside => (p.face.normal(), d.shell.outside),
                _ => (-p.face.normal(), d.shell.inside),
            };
            return vec![Handle::Line { line: LineHandle { origin: at, dir }, value, target: HandleTarget::ShellThickness }];
        }
        if let Some(d) = &self.offset_dialog {
            // The distance arrow, out of the first face picked (from where it was).
            let Some(p) = d.offset.faces.first() else { return Vec::new() };
            let at = p.face.origin + p.face.u * p.anchor[0] + p.face.v * p.anchor[1];
            return vec![Handle::Line { line: LineHandle { origin: at, dir: p.face.normal() }, value: d.offset.distance, target: HandleTarget::OffsetDistance }];
        }
        if let Some(d) = self.hole_dialog.as_ref().filter(|d| d.hole.extent == model::HoleExtent::Distance) {
            // The depth arrow, from the first place into the material.
            let Some((p, inward)) = d.hole.places.first().and_then(|pl| self.hole_place_world(pl)) else { return Vec::new() };
            let dir = if d.hole.flip { -inward } else { inward };
            return vec![Handle::Line { line: LineHandle { origin: p, dir }, value: d.hole.depth, target: HandleTarget::HoleDepth }];
        }
        if let Some((d, a)) = self.plane_dialog.as_ref().and_then(|d| Some((d, d.angle?))) {
            // A round arrow that swings the plane about its line.
            let (o, dir, n0) = a.line;
            let reach = (d.extent[2] - d.extent[0]).abs().max(1.0) * 0.5;
            let at = o + model::zero_normal(dir, n0).cross(dir.normalize_or_zero()) * reach;
            return TurnHandle::new(o, dir, at).map(|turn| Handle::Turn { turn, value: a.degrees, target: HandleTarget::PlaneAngle }).into_iter().collect();
        }
        if let Some(d) = &self.plane_dialog {
            let size = origin::size_for(self.camera.distance);
            let (lo, hi) = self.plane_rect(d.extent, &d.base, d.on_origin, size);
            let c = (lo + hi) * 0.5;
            let from = d.base.origin + d.base.u * c.x + d.base.v * c.y;
            return vec![Handle::Line { line: LineHandle { origin: from, dir: d.base.normal() }, value: d.distance, target: HandleTarget::PlaneOffset }];
        }
        if let Some(d) = &self.copy_dialog {
            return self.copy_handles(d);
        }
        self.current_handle(regions).into_iter().collect()
    }

    /// What the cursor at `px` means for an arrow: a distance along it, or
    /// the angle it has been swung to.
    pub(crate) fn sample_handle(&self, h: &Handle, px: Vec2, aspect: f32, min: Vec2, size: Vec2) -> Option<f64> {
        let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
        match h {
            Handle::Line { line, .. } => closest_param_on_line(&ray, line.origin.as_vec3(), line.dir.as_vec3()).map(|s| s as f64),
            Handle::Turn { turn, .. } => turn.angle_at(ray.origin.as_dvec3(), ray.dir.as_dvec3()),
        }
    }

    /// Where the bodies a pattern copies are, and the axis a circular one turns about.
    pub(crate) fn copy_geometry(&self, d: &CopyDialog) -> Option<CopyGeo> {
        let cache = self.caches[Slot::Committed as usize].as_ref()?;
        let mut bounds: Option<(Vec3, Vec3)> = None;
        for b in cache.model.bodies.iter().filter(|b| d.bodies.contains(&b.id)) {
            if let Some((lo, hi)) = model::mesh_bounds(&b.mesh) {
                bounds = Some(bounds.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
            }
        }
        let (lo, hi) = bounds?;
        let centroid = ((lo + hi) * 0.5).as_dvec3();
        let half = ((hi - lo) * 0.5).as_dvec3();
        let (axis_point, axis_dir) = match d.custom {
            Some(k) => cache.model.axes.get(k).copied().flatten()?,
            None => (DVec3::ZERO, d.axis.dir()),
        };
        let pivot = axis_point + axis_dir * (centroid - axis_point).dot(axis_dir);
        let out = centroid - pivot;
        let out_dir = out.try_normalize();
        let reach = out_dir.map_or(0.0, |o| o.abs().dot(half));
        Some(CopyGeo { centroid, axis_point, axis_dir, pivot, base_radius: out.length(), out_dir, reach })
    }

    /// The blue arrows of the open mirror / pattern dialog.
    pub(crate) fn copy_handles(&self, d: &CopyDialog) -> Vec<Handle> {
        let Some(g) = self.copy_geometry(d) else { return Vec::new() };
        match d.mode {
            CopyMode::Mirror => Vec::new(),
            CopyMode::Rectangular => vec![
                Handle::Line { line: LineHandle { origin: g.centroid, dir: d.dir.dir() }, value: d.spacing, target: HandleTarget::PatternSpacing1 },
                Handle::Line { line: LineHandle { origin: g.centroid, dir: d.dir2.dir() }, value: d.spacing2, target: HandleTarget::PatternSpacing2 },
            ],
            CopyMode::Circular => {
                let Some(out) = g.out_dir else { return Vec::new() };
                // The angle arrow swings a little inside the bodies' ring so the two don't sit on each other.
                let inner = g.pivot + out * (g.base_radius * 0.6);
                let mut v = Vec::new();
                if let Some(turn) = TurnHandle::new(g.axis_point, g.axis_dir, inner) {
                    v.push(Handle::Turn { turn, value: d.angle, target: HandleTarget::PatternAngle });
                }
                // The distance arrow stands just outside the bodies, where the copies' ring is.
                v.push(Handle::Line {
                    line: LineHandle { origin: g.pivot + out * g.reach, dir: out },
                    value: d.radial,
                    target: HandleTarget::PatternRadial,
                });
                v
            }
        }
    }

    pub(crate) fn current_handle(&self, regions: &[Vec<Region>]) -> Option<Handle> {
        if let (Some(FloatEdit { target: EditTarget::Corner(p), text, .. }), Some(i)) = (&self.float, self.active) {
            let sk = &self.sketches[i];
            let (_, q1, _, q2) = sk.corner_lines(*p)?;
            let corner = sk.points[*p];
            let bisector = ((sk.points[q1] - corner).normalize_or_zero() + (sk.points[q2] - corner).normalize_or_zero()).try_normalize()?;
            let plane = self.plane_of(i);
            let origin = plane.to_world(corner);
            let dir = (plane.to_world(corner + bisector) - origin).normalize_or_zero();
            let value = sketch::parse_sizes(text, self.units).map_or(self.corner_size, |(d, _)| d);
            let line = LineHandle { origin: origin.as_dvec3(), dir: dir.as_dvec3() };
            return Some(Handle::Line { line, value, target: HandleTarget::Corner });
        }
        if let Some(d) = &self.dialog {
            if d.extent == model::Extent::ToObject {
                return None; // its length comes from what it runs up to
            }
            let (origin, normal) = self.extrude_axis(regions)?;
            let line = LineHandle { origin: origin.as_dvec3(), dir: normal.as_dvec3() };
            let value = if d.extent == model::Extent::Symmetric { d.distance / 2.0 } else { d.distance };
            return Some(Handle::Line { line, value, target: HandleTarget::Extrude });
        }
        if let Some(d) = &self.edge_dialog {
            // (The model before the fillet may not be built yet this frame: then
            // the arrow points the usual way until it is.)
            let base = self.caches[Slot::Base as usize].as_ref().map(|c| &c.model);
            let inside = |p: DVec3| base.is_some_and(|m| m.bodies.iter().any(|b| model::inside(&b.mesh, p.as_vec3())));
            let line = handles::edge_handle(d.edges.first()?, d.size, inside)?;
            return Some(Handle::Line { line, value: d.size, target: HandleTarget::EdgeSize });
        }
        if let Some(d) = &self.revolve_dialog {
            let s = d.sketch?;
            let r = regions.get(s)?.get(*d.profiles.first()?)?;
            let spot = self.plane_of(s).to_world(r.centroid());
            let (a, b) = self.revolve_axis_line(s, d.axis, 1.0)?;
            let turn = TurnHandle::new(a.as_dvec3(), (b - a).as_dvec3(), spot.as_dvec3())?;
            return Some(Handle::Turn { turn, value: d.angle, target: HandleTarget::RevolveAngle });
        }
        None
    }

    /// Put a dragged arrow's value into its box.
    pub(crate) fn set_handle_value(&mut self, target: HandleTarget, v: f64) {
        match target {
            HandleTarget::Extrude => {
                if let Some(d) = self.dialog.as_mut() {
                    d.distance = if d.extent == model::Extent::Symmetric { v * 2.0 } else { v };
                }
            }
            HandleTarget::EdgeSize => {
                if let Some(d) = self.edge_dialog.as_mut() {
                    let v = v.max(0.01);
                    // A chamfer's two distances scale together.
                    d.size2 = d.size2.map(|d2| d2 * v / d.size.max(1e-9));
                    d.size = v;
                }
            }
            HandleTarget::RevolveAngle => {
                if let Some(d) = self.revolve_dialog.as_mut() {
                    d.angle = v.clamp(-360.0, 360.0);
                }
            }
            HandleTarget::PatternSpacing1 | HandleTarget::PatternSpacing2 | HandleTarget::PatternAngle | HandleTarget::PatternRadial => {
                if let Some(d) = self.copy_dialog.as_mut() {
                    match target {
                        HandleTarget::PatternSpacing1 => d.spacing = v,
                        HandleTarget::PatternSpacing2 => d.spacing2 = v,
                        HandleTarget::PatternAngle => d.angle = v.clamp(-360.0, 360.0),
                        _ => d.radial = v,
                    }
                }
            }
            HandleTarget::SketchSpacing1 | HandleTarget::SketchSpacing2 | HandleTarget::SketchAngle | HandleTarget::SketchRadial => {
                if let Some(d) = self.sketch_pattern.as_mut() {
                    match target {
                        HandleTarget::SketchSpacing1 => d.spacing = v,
                        HandleTarget::SketchSpacing2 => d.spacing2 = v,
                        HandleTarget::SketchRadial => d.set_radius(v),
                        _ => d.angle = v.clamp(-360.0, 360.0),
                    }
                }
            }
            HandleTarget::PlaneOffset => {
                if let Some(d) = self.plane_dialog.as_mut() {
                    d.distance = v;
                }
            }
            HandleTarget::ShellThickness => {
                if let Some(d) = self.shell_dialog.as_mut() {
                    let v = v.max(0.01);
                    match d.shell.direction {
                        model::ShellDirection::Outside => d.shell.outside = v,
                        _ => d.shell.inside = v,
                    }
                }
            }
            HandleTarget::OffsetDistance => {
                if let Some(d) = self.offset_dialog.as_mut() {
                    d.offset.distance = v;
                }
            }
            HandleTarget::HoleDepth => {
                if let Some(d) = self.hole_dialog.as_mut() {
                    d.hole.depth = v.max(0.01);
                }
            }
            HandleTarget::PlaneAngle => {
                if let Some(a) = self.plane_dialog.as_mut().and_then(|d| d.angle.as_mut()) {
                    a.degrees = v.clamp(-360.0, 360.0);
                }
            }
            HandleTarget::Corner => {
                // Scales a chamfer's two distances together, keeping their ratio.
                let unit = self.units;
                if let Some(fe) = self.float.as_mut() {
                    let v = v.max(0.01);
                    fe.text = match sketch::parse_sizes(&fe.text, unit) {
                        Some((d1, Some(d2))) if d1 > 0.0 => sizes_text(v, Some(d2 * v / d1), unit),
                        _ => unit.plain(v),
                    };
                }
            }
        }
        self.set_distance_text();
    }

    /// A revolve axis as two world points `reach` × the view distance either
    /// side of the sketch's origin / the line.
    pub(crate) fn revolve_axis_line(&self, s: usize, which: RevolveAxis, reach: f64) -> Option<(Vec3, Vec3)> {
        let sk = self.sketches.get(s)?;
        let plane = self.plane_of(s);
        let reach = (self.camera.distance as f64) * 2.0 * reach;
        if let RevolveAxis::Construction(k) = which {
            let (o, d) = self.model(Slot::Committed).axes.get(k).copied().flatten()?;
            return Some(((o - d * reach).as_vec3(), (o + d * reach).as_vec3()));
        }
        let (a, b) = match which {
            RevolveAxis::Construction(_) => return None,
            RevolveAxis::SketchX => (DVec2::new(-reach, 0.0), DVec2::new(reach, 0.0)),
            RevolveAxis::SketchY => (DVec2::new(0.0, -reach), DVec2::new(0.0, reach)),
            RevolveAxis::Line(e) => match sk.entities.get(e)? {
                Entity::Line { a, b } => {
                    let (p, q) = (sk.points[*a], sk.points[*b]);
                    let dir = (q - p).normalize_or_zero();
                    (p - dir * reach, q + dir * reach)
                }
                _ => return None,
            },
        };
        Some((plane.to_world(a), plane.to_world(b)))
    }

    /// Axis the extrude arrow moves along: (point on the sketch plane, plane normal).
    pub(crate) fn extrude_axis(&self, regions: &[Vec<Region>]) -> Option<(Vec3, Vec3)> {
        let d = self.dialog.as_ref()?;
        let s = d.sketch?;
        let r = regions.get(s)?.get(*d.profiles.first()?)?;
        let plane = self.plane_of(s);
        Some((plane.to_world(r.centroid()), plane.normal()))
    }

    /// Faces and edges picked on the far end of a deleted extrude/revolve
    /// stop following it; later feature numbers shift down.
    pub(crate) fn fix_follows(&mut self, revolve: bool, k: usize) {
        for sk in &mut self.sketches {
            if let Some(face) = sk.face.as_mut() {
                face.follow = face.follow.and_then(|f| f.after_delete(revolve, k));
            }
        }
        for ef in &mut self.edge_features {
            for r in &mut ef.edges {
                r.follow = r.follow.and_then(|f| f.after_delete(revolve, k));
            }
        }
    }

    pub(crate) fn delete_extrude(&mut self, k: usize) {
        self.fix_follows(false, k);
        self.shift_body_keys(false, k);
        self.extrudes.remove(k);
        self.timeline_remove(Feature::Extrude(k));
        for f in &mut self.timeline {
            if let Feature::Extrude(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_extrude();
    }

    // ---------- revolve ----------

    pub(crate) fn open_revolve(&mut self, editing: Option<usize>) {
        self.axis_pick = false;
        self.angle_pick = false;
        self.hole_dialog = None;
        self.shell_dialog = None;
        self.offset_dialog = None;
        self.combine_dialog = None;
        self.thread_dialog = None;
        self.finish_sketch();
        self.cancel_extrude();
        self.cancel_edge_dialog();
        self.message = None;
        let d = match editing.and_then(|k| self.revolves.get(k)) {
            Some(r) => RevolveDialog {
                editing,
                sketch: Some(r.sketch),
                profiles: r.profiles.clone(),
                axis: r.axis,
                angle: r.angle,
                op: r.op,
                picking_axis: false,
            },
            None => RevolveDialog {
                editing: None,
                sketch: None,
                profiles: Vec::new(),
                axis: RevolveAxis::SketchY,
                angle: 360.0,
                op: if self.body_count > 0 { Operation::Join } else { Operation::NewBody },
                picking_axis: false,
            },
        };
        self.float = Some(FloatEdit { target: EditTarget::RevolveAngle, text: format!("{:.1}", d.angle), focus: true, keep_text: false });
        self.revolve_dialog = Some(d);
        if editing.is_none() {
            self.pick_only_profile();
        }
    }

    pub(crate) fn dialog_revolve(&self) -> Option<Revolve> {
        let d = self.revolve_dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.revolves[k].name.clone(),
            None => format!("Revolve{}", self.revolves.len() + 1),
        };
        Some(Revolve { op: d.op, name, sketch: d.sketch?, profiles: d.profiles.clone(), axis: d.axis, angle: d.angle })
    }

    pub(crate) fn commit_revolve(&mut self) {
        let Some(rv) = self.dialog_revolve() else {
            self.message = Some("Select at least one profile".into());
            return;
        };
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't revolve: {e}"));
            return;
        }
        match self.revolve_dialog.as_ref().and_then(|d| d.editing) {
            Some(k) => self.revolves[k] = rv,
            None => {
                self.revolves.push(rv);
                self.timeline_insert(Feature::Revolve(self.revolves.len() - 1));
            }
        }
        self.cancel_revolve();
    }

    pub(crate) fn cancel_revolve(&mut self) {
        self.revolve_dialog = None;
        self.axis_hover = None;
        if self.float.as_ref().is_some_and(|f| f.target == EditTarget::RevolveAngle) {
            self.float = None;
        }
    }

    pub(crate) fn delete_revolve(&mut self, k: usize) {
        self.fix_follows(true, k);
        self.shift_body_keys(true, k);
        self.revolves.remove(k);
        self.timeline_remove(Feature::Revolve(k));
        for f in &mut self.timeline {
            if let Feature::Revolve(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_revolve();
    }

    pub(crate) fn open_copy(&mut self, mode: CopyMode, editing: Option<usize>) {
        self.axis_pick = false;
        self.angle_pick = false;
        self.hole_dialog = None;
        self.shell_dialog = None;
        self.offset_dialog = None;
        self.combine_dialog = None;
        self.thread_dialog = None;
        self.cancel_extrude();
        self.cancel_revolve();
        self.cancel_edge_dialog();
        self.message = None;
        self.ensure_model(Slot::Committed);
        let mut d = match editing.and_then(|k| self.copies.get(k)) {
            Some(c) => CopyDialog::from_feature(editing.unwrap_or(0), c),
            None => {
                // With a single body there's nothing to choose.
                let ids: Vec<BodyId> = self.model(Slot::Committed).bodies.iter().map(|b| b.id).collect();
                let mut d = CopyDialog::new(mode, if ids.len() == 1 { ids } else { Vec::new() });
                // Start with the field that needs a pick, as Fusion does.
                d.picking = if d.bodies.is_empty() {
                    Some(copy_dialog::Field::Bodies)
                } else if mode == CopyMode::Mirror {
                    Some(copy_dialog::Field::Plane)
                } else {
                    None
                };
                d
            }
        };
        if editing.is_some() {
            d.picking = None;
        }
        self.copy_dialog = Some(d);
    }

    pub(crate) fn dialog_copy(&self) -> Option<CopyFeature> {
        let d = self.copy_dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.copies[k].name.clone(),
            None => format!("{}{}", d.mode.title(), self.copies.len() + 1),
        };
        d.feature(name)
    }

    pub(crate) fn commit_copy(&mut self) {
        let Some(c) = self.dialog_copy() else {
            self.message = Some("Pick at least one body".into());
            return;
        };
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't do this: {e}"));
            return;
        }
        self.plane_hidden.extend(c.kind.plane_used());
        match self.copy_dialog.as_ref().and_then(|d| d.editing) {
            Some(k) => self.copies[k] = c,
            None => {
                self.copies.push(c);
                self.timeline_insert(Feature::Copy(self.copies.len() - 1));
            }
        }
        self.cancel_copy();
    }

    pub(crate) fn cancel_copy(&mut self) {
        self.copy_dialog = None;
        self.message = None;
    }

    pub(crate) fn delete_copy(&mut self, k: usize) {
        self.shift_copy_keys(k);
        self.copies.remove(k);
        self.timeline_remove(Feature::Copy(k));
        for f in &mut self.timeline {
            if let Feature::Copy(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_copy();
    }

    /// Open the Hole dialog (a new hole, or hole `editing`).
    pub(crate) fn open_hole(&mut self, editing: Option<usize>) {
        self.finish_sketch();
        self.cancel_all_dialogs();
        self.message = None;
        self.hole_dialog = Some(match editing.and_then(|k| self.holes.get(k).map(|h| (k, h))) {
            Some((k, h)) => HoleDialog::from_feature(k, h),
            None => HoleDialog::new(format!("Hole{}", self.holes.len() + 1)),
        });
    }

    pub(crate) fn commit_hole(&mut self) {
        let Some(d) = self.hole_dialog.as_ref() else { return };
        if d.hole.places.is_empty() {
            self.message = Some("Pick where the hole goes".into());
            return;
        }
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't make the hole: {e}"));
            return;
        }
        let h = d.hole.clone();
        match d.editing {
            Some(k) => self.holes[k] = h,
            None => {
                self.holes.push(h);
                self.timeline_insert(Feature::Hole(self.holes.len() - 1));
            }
        }
        self.cancel_hole();
    }

    pub(crate) fn cancel_hole(&mut self) {
        self.hole_dialog = None;
        self.shell_dialog = None;
        self.offset_dialog = None;
        self.combine_dialog = None;
        self.thread_dialog = None;
        self.hole_hover = None;
        self.arrows.clear();
    }

    /// What a click in the view would add to the hole: a sketch circle (on
    /// it or its centre), a sketch point, the middle of a flat face (near it)
    /// or a point on the face. With where its centre is.
    pub(crate) fn hole_place_under(&mut self, view: &View, px: Vec2, aspect: f32, min: Vec2, size: Vec2) -> Option<(model::HolePlace, DVec3)> {
        let mut best: Option<(f32, model::HolePlace, DVec3)> = None;
        let mut offer = |d: f32, place: model::HolePlace, at: DVec3| {
            if d < SNAP_PX && best.as_ref().is_none_or(|b| d < b.0) {
                best = Some((d, place, at));
            }
        };
        for s in (0..self.sketches.len()).filter(|&s| self.sketch_visible(s)) {
            let (sk, plane) = (&self.sketches[s], self.plane_of(s));
            let screen = |p: DVec2| view.project(plane.to_world(p));
            let mut used = vec![false; sk.points.len()];
            for (e, ent) in sk.entities.iter().enumerate() {
                let (c, r) = match *ent {
                    Entity::Circle { center, radius } => (center, radius),
                    Entity::Arc { center, a, .. } => (center, sk.points[center].distance(sk.points[a])),
                    _ => {
                        for p in sk.entity_points(e) {
                            used[p] = true;
                        }
                        continue;
                    }
                };
                used[c] = true;
                // On the circle or at its centre (a little ahead of a plain point there).
                let ring: Vec<Vec2> = (0..=48).filter_map(|k| screen(sk.points[c] + DVec2::from_angle(k as f64 / 48.0 * std::f64::consts::TAU) * r)).collect();
                let on_ring = ring.windows(2).map(|w| dist_to_segment_px(px, w[0], w[1])).fold(f32::MAX, f32::min);
                let at_centre = screen(sk.points[c]).map_or(f32::MAX, |q| q.distance(px));
                offer(on_ring.min(at_centre) - 0.5, model::HolePlace::SketchCircle { sketch: s, entity: e }, plane.to_world(sk.points[c]).as_dvec3());
            }
            for (p, _) in used.iter().enumerate().filter(|(_, u)| **u) {
                if let Some(q) = screen(sk.points[p]) {
                    offer(q.distance(px), model::HolePlace::SketchPoint { sketch: s, point: p }, plane.to_world(sk.points[p]).as_dvec3());
                }
            }
        }
        if let Some((_, place, at)) = best {
            return Some((place, at));
        }
        // A flat face: its middle when the cursor is near it, else the point under the cursor.
        let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
        let (t, b, face) = self.pick_face(&ray)?;
        let hit = (ray.origin + ray.dir * t).as_dvec3();
        let body = &self.model(Slot::Committed).bodies[b];
        let middle = model::ray_hit_tri(&body.mesh, ray.origin, ray.dir).and_then(|(_, tri)| body.face_centroid(*body.tri_face.get(tri)? as usize));
        let face = sketch::FacePlane { follow: None, ..face };
        let anchor = [hit.dot(face.u), hit.dot(face.v)];
        match middle.filter(|m| view.project(m.as_vec3()).is_some_and(|q| q.distance(px) < SNAP_PX * 1.5)) {
            Some(m) => Some((model::HolePlace::Face { face, anchor, centre: true }, m)),
            None => Some((model::HolePlace::Face { face, anchor, centre: false }, hit)),
        }
    }

    /// Where a hole could go that the camera can see: the middle of each flat
    /// face, and the points and circle centres of the visible sketches.
    /// Spots behind the part are left out.
    pub(crate) fn hole_spots(&self) -> Vec<DVec3> {
        let Some(cache) = self.caches[Slot::Committed as usize].as_ref() else { return Vec::new() };
        let bodies: Vec<&kernel::Body> = cache.model.bodies.iter().filter(|b| !self.hidden_bodies.contains(&b.id)).collect();
        let mut spots: Vec<DVec3> = bodies.iter().flat_map(|b| b.flat_face_centres()).collect();
        for s in (0..self.sketches.len()).filter(|&s| self.sketch_visible(s)) {
            let (sk, plane) = (&self.sketches[s], self.plane_of(s));
            let mut used = vec![false; sk.points.len()];
            for e in 0..sk.entities.len() {
                for p in sk.entity_points(e) {
                    used[p] = true;
                }
            }
            spots.extend(used.iter().enumerate().filter(|(_, u)| **u).map(|(p, _)| plane.to_world(sk.points[p]).as_dvec3()));
        }
        // Seen: nothing of a body between it and the eye.
        let lift = self.camera.distance * 1e-3;
        spots.retain(|p| {
            let to_eye = self.camera.to_eye(p.as_vec3());
            let from = p.as_vec3() + to_eye * lift;
            !bodies.iter().any(|b| model::ray_hit(&b.mesh, from, to_eye).is_some())
        });
        spots
    }

    /// Where a hole place is now (as far as the app knows) and the way it drills (unflipped).
    pub(crate) fn hole_place_world(&self, place: &model::HolePlace) -> Option<(DVec3, DVec3)> {
        match *place {
            model::HolePlace::SketchPoint { sketch, point } => {
                let plane = self.plane_of(sketch);
                Some((plane.to_world(*self.sketches.get(sketch)?.points.get(point)?).as_dvec3(), -plane.normal().as_dvec3()))
            }
            model::HolePlace::SketchCircle { sketch, entity } => {
                let sk = self.sketches.get(sketch)?;
                let c = match *sk.entities.get(entity)? {
                    Entity::Circle { center, .. } | Entity::Arc { center, .. } => center,
                    _ => return None,
                };
                let plane = self.plane_of(sketch);
                Some((plane.to_world(sk.points[c]).as_dvec3(), -plane.normal().as_dvec3()))
            }
            model::HolePlace::Face { face, anchor, centre } => {
                let q = face.origin + face.u * anchor[0] + face.v * anchor[1];
                // At the face's middle: where the kernel puts it (the face as it is now).
                let middle = centre
                    .then(|| self.caches[Slot::Committed as usize].as_ref())
                    .flatten()
                    .and_then(|c| c.model.bodies.iter().find_map(|b| b.face_at(&face, q).and_then(|f| b.face_centroid(f))));
                Some((middle.unwrap_or(q), -face.normal()))
            }
        }
    }

    /// On the first place picked: a sketch hole that would drill away from
    /// every body (but toward one the other way) is flipped, like Cut aims.
    pub(crate) fn aim_hole(&mut self) {
        let Some(place) = self.hole_dialog.as_ref().filter(|d| d.editing.is_none() && d.hole.places.len() == 1).map(|d| d.hole.places[0]) else { return };
        let Some((p, inward)) = self.hole_place_world(&place) else { return };
        let model = self.model(Slot::Committed);
        let hits = |dir: DVec3| model.bodies.iter().any(|b| model::ray_hit(&b.mesh, (p - dir * 1e-3).as_vec3(), dir.as_vec3()).is_some());
        let flip = !hits(inward) && hits(-inward);
        if let Some(d) = self.hole_dialog.as_mut() {
            d.hole.flip = flip;
        }
    }

    /// Open the Shell dialog (a new shell, or shell `editing`).
    pub(crate) fn open_shell(&mut self, editing: Option<usize>) {
        self.finish_sketch();
        self.cancel_all_dialogs();
        self.message = None;
        self.shell_dialog = Some(match editing.and_then(|k| self.shells.get(k).map(|s| (k, s))) {
            Some((k, s)) => ShellDialog::from_feature(k, s),
            None => ShellDialog::new(format!("Shell{}", self.shells.len() + 1)),
        });
    }

    pub(crate) fn commit_shell(&mut self) {
        let Some(d) = self.shell_dialog.as_ref() else { return };
        if d.shell.faces.is_empty() && d.shell.bodies.is_empty() {
            self.message = Some("Pick faces to open, or a body to hollow".into());
            return;
        }
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't shell: {e}"));
            return;
        }
        let s = d.shell.clone();
        match d.editing {
            Some(k) => self.shells[k] = s,
            None => {
                self.shells.push(s);
                self.timeline_insert(Feature::Shell(self.shells.len() - 1));
            }
        }
        self.cancel_shell();
    }

    pub(crate) fn cancel_shell(&mut self) {
        self.shell_dialog = None;
        self.shell_hover = None;
        self.arrows.clear();
    }

    pub(crate) fn delete_shell(&mut self, k: usize) {
        if k >= self.shells.len() {
            return;
        }
        self.shells.remove(k);
        self.timeline_remove(Feature::Shell(k));
        for f in &mut self.timeline {
            if let Feature::Shell(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_shell();
    }

    /// Open the Offset Face dialog (a new one — as Press Pull when
    /// `press_pull` — or Offset Face `editing`), with the distance box ready.
    pub(crate) fn open_offset(&mut self, editing: Option<usize>, press_pull: bool) {
        self.finish_sketch();
        self.cancel_all_dialogs();
        self.message = None;
        let d = match editing.and_then(|k| self.offset_faces.get(k).map(|f| (k, f))) {
            Some((k, f)) => OffsetDialog::from_feature(k, f),
            None => OffsetDialog::new(format!("Offset Face{}", self.offset_faces.len() + 1), press_pull),
        };
        self.float = Some(FloatEdit { target: EditTarget::OffsetDistance, text: self.units.plain(d.offset.distance), focus: true, keep_text: false });
        self.offset_dialog = Some(d);
    }

    pub(crate) fn commit_offset(&mut self) {
        let Some(d) = self.offset_dialog.as_ref() else { return };
        if d.offset.faces.is_empty() {
            self.message = Some("Pick the faces to move".into());
            return;
        }
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't move the face: {e}"));
            return;
        }
        let f = d.offset.clone();
        match d.editing {
            Some(k) => self.offset_faces[k] = f,
            None => {
                self.offset_faces.push(f);
                self.timeline_insert(Feature::OffsetFace(self.offset_faces.len() - 1));
            }
        }
        self.cancel_offset();
    }

    pub(crate) fn cancel_offset(&mut self) {
        self.offset_dialog = None;
        self.offset_hover = None;
        if self.float.as_ref().is_some_and(|f| f.target == EditTarget::OffsetDistance) {
            self.float = None;
        }
        self.arrows.clear();
    }

    pub(crate) fn delete_offset(&mut self, k: usize) {
        if k >= self.offset_faces.len() {
            return;
        }
        self.offset_faces.remove(k);
        self.timeline_remove(Feature::OffsetFace(k));
        for f in &mut self.timeline {
            if let Feature::OffsetFace(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_offset();
    }

    /// Open the Combine dialog (a new one — bodies selected in the browser
    /// fill it — or Combine `editing`).
    pub(crate) fn open_combine(&mut self, editing: Option<usize>) {
        let selected: Vec<BodyId> = self.selected_bodies.iter().copied().collect();
        self.finish_sketch();
        self.cancel_all_dialogs();
        self.message = None;
        self.combine_dialog = Some(match editing.and_then(|k| self.combines.get(k).map(|c| (k, c))) {
            Some((k, c)) => CombineDialog::from_feature(k, c),
            None => CombineDialog::new(format!("Combine{}", self.combines.len() + 1), &selected),
        });
    }

    pub(crate) fn commit_combine(&mut self) {
        let Some(d) = self.combine_dialog.as_ref() else { return };
        if !d.ready() {
            self.message = Some("Pick the target body and the tool bodies".into());
            return;
        }
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't combine: {e}"));
            return;
        }
        let c = d.combine.clone();
        match d.editing {
            Some(k) => self.combines[k] = c,
            None => {
                self.combines.push(c);
                self.timeline_insert(Feature::Combine(self.combines.len() - 1));
            }
        }
        self.cancel_combine();
    }

    pub(crate) fn cancel_combine(&mut self) {
        self.combine_dialog = None;
        self.combine_hover = None;
    }

    pub(crate) fn delete_combine(&mut self, k: usize) {
        if k >= self.combines.len() {
            return;
        }
        self.combines.remove(k);
        self.timeline_remove(Feature::Combine(k));
        for f in &mut self.timeline {
            if let Feature::Combine(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_combine();
    }

    pub(crate) fn combine_panel(&mut self, ui: &mut egui::Ui) {
        let error = self.dialog_error.clone();
        let Some(d) = self.combine_dialog.as_ref() else { return };
        let ids: Vec<BodyId> = self.body_ids.iter().copied().chain(d.combine.target).chain(d.combine.tools.iter().copied()).collect();
        let names: Vec<(BodyId, String)> = ids.into_iter().map(|id| (id, self.body_name(id))).collect();
        let Some(d) = self.combine_dialog.as_mut() else { return };
        match d.panel(ui, &names, error.as_deref()) {
            copy_dialog::Action::Ok => self.commit_combine(),
            copy_dialog::Action::Cancel => self.cancel_combine(),
            copy_dialog::Action::None => {}
        }
    }

    /// The bodies an open dialog has picked, when it's picking bodies now
    /// (Combine, Shell's Bodies field, Mirror / Pattern).
    pub(crate) fn dialog_bodies(&self) -> Option<Vec<BodyId>> {
        if let Some(d) = self.combine_dialog.as_ref().filter(|d| d.picking.is_some()) {
            return Some(d.combine.target.into_iter().chain(d.combine.tools.iter().copied()).collect());
        }
        if let Some(d) = self.shell_dialog.as_ref().filter(|d| d.picking == Some(shell_dialog::Field::Bodies)) {
            return Some(d.shell.bodies.clone());
        }
        if let Some(d) = self.copy_dialog.as_ref().filter(|d| d.picking == Some(copy_dialog::Field::Bodies)) {
            return Some(d.bodies.clone());
        }
        None
    }

    /// A body clicked (in the view or the browser) for the dialog picking bodies.
    pub(crate) fn dialog_pick_body(&mut self, id: BodyId) {
        if let Some(d) = self.combine_dialog.as_mut().filter(|d| d.picking.is_some()) {
            d.click(id);
        } else if let Some(d) = self.shell_dialog.as_mut().filter(|d| d.picking == Some(shell_dialog::Field::Bodies)) {
            d.toggle_body(id);
        } else if let Some(d) = self.copy_dialog.as_mut().filter(|d| d.picking == Some(copy_dialog::Field::Bodies)) {
            if d.bodies.contains(&id) {
                d.bodies.retain(|b| *b != id);
            } else {
                d.bodies.push(id);
            }
        }
    }

    /// Open the Thread dialog (a new Thread, or Thread `editing`).
    pub(crate) fn open_thread(&mut self, editing: Option<usize>) {
        self.finish_sketch();
        self.cancel_all_dialogs();
        self.message = None;
        self.ensure_model(Slot::Committed);
        self.thread_dialog = Some(match editing.and_then(|k| self.thread_features.get(k).map(|t| (k, t))) {
            // (Already threaded, the face isn't a plain cylinder any more: use what was picked.)
            Some((k, t)) => ThreadDialog::from_feature(k, t, t.faces.first().is_some_and(|f| f.internal)),
            None => ThreadDialog::new(format!("Thread{}", self.thread_features.len() + 1), self.last_thread.clone()),
        });
    }

    pub(crate) fn commit_thread(&mut self) {
        let Some(d) = self.thread_dialog.as_ref() else { return };
        if d.thread.faces.is_empty() {
            self.message = Some("Pick a cylindrical face to thread".into());
            return;
        }
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't make the thread: {e}"));
            return;
        }
        self.last_thread = d.remember.then(|| d.thread.spec.clone());
        let t = d.thread.clone();
        match d.editing {
            Some(k) => self.thread_features[k] = t,
            None => {
                self.thread_features.push(t);
                self.timeline_insert(Feature::Thread(self.thread_features.len() - 1));
            }
        }
        self.cancel_thread();
    }

    pub(crate) fn cancel_thread(&mut self) {
        self.thread_dialog = None;
        self.thread_hover = None;
    }

    pub(crate) fn delete_thread(&mut self, k: usize) {
        if k >= self.thread_features.len() {
            return;
        }
        self.thread_features.remove(k);
        self.timeline_remove(Feature::Thread(k));
        for f in &mut self.timeline {
            if let Feature::Thread(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_thread();
    }

    /// The whole cylindrical face under the cursor (a shaft or a hole) and the point hit.
    pub(crate) fn cylinder_under(&mut self, ray: &Ray) -> Option<(kernel::Cylinder, DVec3)> {
        self.ensure_model(Slot::Committed);
        let m = self.model(Slot::Committed);
        let (t, tri, b) = m
            .bodies
            .iter()
            .enumerate()
            .filter(|(_, body)| !self.hidden_bodies.contains(&body.id))
            .filter_map(|(b, body)| model::ray_hit_tri(&body.mesh, ray.origin, ray.dir).map(|(t, tri)| (t, tri, b)))
            .min_by(|x, y| x.0.total_cmp(&y.0))?;
        let body = &m.bodies[b];
        let c = kernel::cylinder_face(&body.shape, *body.tri_face.get(tri)? as usize)?;
        Some((c, ray.at(t).as_dvec3()))
    }

    /// The shown (cosmetic) threads of the model in `slot`, as helix segments.
    pub(crate) fn thread_helices(&self, slot: Slot) -> Vec<(Vec3, Vec3, [f32; 4])> {
        let Some(cache) = self.caches[slot as usize].as_ref() else { return Vec::new() };
        let mut out = Vec::new();
        for th in &cache.model.threads {
            let u = model::zero_normal(th.axis, DVec3::Z);
            let v = th.axis.cross(u);
            let turns = (th.length / th.pitch).min(200.0);
            let steps = (turns * 24.0).ceil().max(2.0) as usize;
            let at = |k: usize| {
                let s = k as f64 / steps as f64;
                let a = s * turns * std::f64::consts::TAU;
                (th.origin + th.axis * (s * turns * th.pitch) + (u * a.cos() + v * a.sin()) * th.radius).as_vec3()
            };
            out.extend((0..steps).map(|k| (at(k), at(k + 1), [0.9, 0.75, 0.35, 1.0])));
        }
        out
    }

    pub(crate) fn delete_hole(&mut self, k: usize) {
        if k >= self.holes.len() {
            return;
        }
        self.holes.remove(k);
        self.timeline_remove(Feature::Hole(k));
        for f in &mut self.timeline {
            if let Feature::Hole(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_hole();
    }

    /// Close whichever tool dialog or pick mode is open.
    pub(crate) fn cancel_all_dialogs(&mut self) {
        self.cancel_shell();
        self.cancel_offset();
        self.cancel_combine();
        self.cancel_hole();
        self.cancel_thread();
        self.cancel_extrude();
        self.cancel_revolve();
        self.cancel_edge_dialog();
        self.cancel_copy();
        self.close_plane_dialog();
        self.plane_pick = false;
        self.axis_pick = false;
        self.angle_pick = false;
        self.hole_dialog = None;
        self.shell_dialog = None;
        self.offset_dialog = None;
        self.combine_dialog = None;
        self.thread_dialog = None;
        self.face_pick = false;
    }

    /// Open the Offset Plane box with the distance ready to type into.
    pub(crate) fn open_plane_dialog(&mut self, d: PlaneDialog) {
        self.float = Some(match d.angle {
            Some(a) => FloatEdit { target: EditTarget::PlaneAngle, text: format!("{:.1}", a.degrees), focus: true, keep_text: false },
            None => FloatEdit { target: EditTarget::PlaneOffset, text: self.units.plain(d.distance), focus: true, keep_text: false },
        });
        self.plane_dialog = Some(d);
    }

    pub(crate) fn close_plane_dialog(&mut self) {
        self.plane_dialog = None;
        if self.float.as_ref().is_some_and(|f| matches!(f.target, EditTarget::PlaneOffset | EditTarget::PlaneAngle)) {
            self.float = None;
        }
    }

    pub(crate) fn commit_plane(&mut self) {
        if let Some(e) = self.dialog_error.clone().filter(|_| self.plane_dialog.is_some()) {
            self.message = Some(format!("Can't make the plane: {e}"));
            return;
        }
        let Some(d) = self.plane_dialog.as_ref() else { return };
        let name = match d.editing {
            Some(k) => self.planes[k].name.clone(),
            None => format!("Plane{}", self.planes.len() + 1),
        };
        let p = d.feature(name);
        match d.editing {
            Some(k) => self.planes[k] = p,
            None => {
                self.planes.push(p);
                self.timeline_insert(Feature::Plane(self.planes.len() - 1));
            }
        }
        self.close_plane_dialog();
    }

    pub(crate) fn delete_plane(&mut self, k: usize) {
        if self.sketches.iter().any(|s| s.plane_ref == Some(k)) {
            self.notify("A sketch is on that plane — delete the sketch first".into());
            return;
        }
        if self.copies.iter().any(|c| c.kind == model::CopyKind::MirrorPlane(k)) {
            self.notify("A mirror uses that plane — delete it or pick another plane first".into());
            return;
        }
        if self.holes.iter().any(|h| h.target == Some(model::ExtrudeTarget::Plane(k))) {
            self.notify("A hole runs to that plane — delete it or pick another object first".into());
            return;
        }
        for h in &mut self.holes {
            if let Some(model::ExtrudeTarget::Plane(j)) = &mut h.target {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        if self.extrudes.iter().any(|e| e.target == Some(model::ExtrudeTarget::Plane(k))) {
            self.notify("An extrude runs up to that plane — delete it or pick another object first".into());
            return;
        }
        for e in &mut self.extrudes {
            if let Some(model::ExtrudeTarget::Plane(j)) = &mut e.target {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        if let Some(d) = self.dialog.as_mut() {
            d.target = match d.target {
                Some(model::ExtrudeTarget::Plane(j)) if j == k => None,
                Some(model::ExtrudeTarget::Plane(j)) if j > k => Some(model::ExtrudeTarget::Plane(j - 1)),
                other => other,
            };
        }
        for c in &mut self.copies {
            if let model::CopyKind::MirrorPlane(j) = &mut c.kind {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        if let Some(d) = self.copy_dialog.as_mut() {
            d.mirror_custom = match d.mirror_custom {
                Some(j) if j == k => None,
                Some(j) if j > k => Some(j - 1),
                other => other,
            };
        }
        self.planes.remove(k);
        self.plane_hidden = self.plane_hidden.iter().filter(|&&j| j != k).map(|&j| if j > k { j - 1 } else { j }).collect();
        self.timeline_remove(Feature::Plane(k));
        for f in &mut self.timeline {
            if let Feature::Plane(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        for s in &mut self.sketches {
            if let Some(j) = s.plane_ref.as_mut() {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.close_plane_dialog();
    }

    pub(crate) fn start_sketch_on_plane(&mut self, k: usize) {
        let Some(frame) = self.caches[Slot::Committed as usize].as_ref().and_then(|c| c.model.planes.get(k).copied().flatten()) else {
            self.notify("That plane couldn't be built".into());
            return;
        };
        self.dialog = None;
        self.finish_sketch();
        let name = format!("Sketch{}", self.sketches.len() + 1);
        self.sketches.push(Sketch::on_plane(name, frame, k));
        self.plane_hidden.insert(k); // used up: hidden, as in Fusion
        let i = self.sketches.len() - 1;
        self.timeline_insert(Feature::Sketch(i));
        self.enter_sketch(i);
        if let Some(p) = self.pending_turn.as_mut() {
            p.target = frame.origin.as_vec3();
        }
    }

    /// The point a plane's square is centred on: for an origin plane its usual
    /// place, for a face's plane the middle of the bodies seen along its normal.
    pub(crate) fn plane_centre(&self, frame: &sketch::FacePlane, on_origin: bool, size: f32) -> DVec2 {
        let (u, v, n) = (frame.u, frame.v, frame.normal());
        if on_origin {
            let (lo, hi) = origin::span(size);
            let mid = (lo + hi) as f64 * 0.5;
            return DVec2::new(mid, mid);
        }
        let mut bounds: Option<(Vec3, Vec3)> = None;
        if let Some(c) = self.caches[Slot::Committed as usize].as_ref() {
            for b in &c.model.bodies {
                if let Some((lo, hi)) = model::mesh_bounds(&b.mesh) {
                    bounds = Some(bounds.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
                }
            }
        }
        let c = bounds.map_or(frame.origin, |(lo, hi)| ((lo + hi) * 0.5).as_dvec3());
        let foot = c - n * n.dot(c - frame.origin);
        DVec2::new((foot - frame.origin).dot(u), (foot - frame.origin).dot(v))
    }

    /// A plane's square in its own coordinates: (low corner, high corner).
    /// Origin planes use their usual spot; a face's plane is drawn over the
    /// face it was picked on, as a square a little bigger than it.
    pub(crate) fn plane_rect(&self, extent: [f64; 4], frame: &sketch::FacePlane, on_origin: bool, size: f32) -> (DVec2, DVec2) {
        if on_origin {
            let (lo, hi) = origin::span(size);
            return (DVec2::splat(lo as f64), DVec2::splat(hi as f64));
        }
        if extent[2] > extent[0] || extent[3] > extent[1] {
            return origin::square_over(extent);
        }
        let c = self.plane_centre(frame, on_origin, size);
        let half = DVec2::splat(size as f64 * 0.35);
        (c - half, c + half)
    }

    /// The construction plane a ray meets first (not hidden): (index, distance).
    pub(crate) fn pick_cplane(&self, ray: &Ray, size: f32) -> Option<(usize, f32)> {
        let model = &self.caches[Slot::Committed as usize].as_ref()?.model;
        (0..self.planes.len())
            .filter(|k| !self.plane_hidden.contains(k))
            .filter_map(|k| {
                let frame = model.planes.get(k).copied().flatten()?;
                let (lo, hi) = self.plane_rect(self.planes[k].extent, &frame, self.planes[k].on_origin, size);
                let (t, p) = frame.plane().intersect_t(ray)?;
                (p.cmpge(lo).all() && p.cmple(hi).all()).then_some((k, t))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }

    pub(crate) fn delete_axis(&mut self, k: usize) {
        if self.planes.iter().any(|p| p.angle.is_some_and(|a| a.hinge.uses_axis(k))) {
            self.notify("A plane at angle turns about that axis — delete the plane first".into());
            return;
        }
        for p in &mut self.planes {
            if let Some(a) = p.angle.as_mut() {
                a.hinge.axis_deleted(k);
            }
        }
        if let Some(a) = self.plane_dialog.as_mut().and_then(|d| d.angle.as_mut()) {
            a.hinge.axis_deleted(k);
        }
        self.axes.remove(k);
        self.axis_hidden = self.axis_hidden.iter().filter(|&&j| j != k).map(|&j| if j > k { j - 1 } else { j }).collect();
        self.timeline_remove(Feature::Axis(k));
        for f in &mut self.timeline {
            if let Feature::Axis(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        // A pattern that turned about it falls back to the world axis it names.
        for c in &mut self.copies {
            if let model::CopyKind::Circular { custom, .. } = &mut c.kind {
                *custom = match *custom {
                    Some(j) if j == k => None,
                    Some(j) if j > k => Some(j - 1),
                    other => other,
                };
            }
        }
        if let Some(d) = self.copy_dialog.as_mut() {
            d.custom = match d.custom {
                Some(j) if j == k => None,
                Some(j) if j > k => Some(j - 1),
                other => other,
            };
        }
    }

    /// Put a construction axis along the straight edge picked.
    pub(crate) fn add_axis(&mut self, edge: EdgeRef) {
        let name = format!("Axis{}", self.axes.len() + 1);
        self.axes.push(AxisFeature { name, edge });
        self.timeline_insert(Feature::Axis(self.axes.len() - 1));
        self.axis_pick = false;
        self.angle_pick = false;
        self.hole_dialog = None;
        self.shell_dialog = None;
        self.offset_dialog = None;
        self.combine_dialog = None;
        self.thread_dialog = None;
    }

    /// The straight body edge nearest the cursor (within snapping reach) that
    /// isn't hidden behind a body, from the model in `slot`.
    pub(crate) fn edge_under(&self, view: &View, slot: Slot, px: Vec2, ray: &Ray, straight_only: bool) -> Option<EdgeRef> {
        let base = self.model(slot);
        // How far the ray goes before it hits a body: edges further than
        // that are hidden behind something and can't be picked.
        let hit = base.bodies.iter().filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir)).reduce(f32::min);
        let slack = self.camera.distance * 0.01;
        let mut best: Option<(f32, EdgeRef)> = None;
        for body in &base.bodies {
            for e in &body.edges {
                if straight_only && !kernel::is_straight(&e.points.iter().map(|p| p.as_dvec3()).collect::<Vec<_>>()) {
                    continue;
                }
                for w in e.points.windows(2) {
                    let (Some(a), Some(b)) = (view.project(w[0]), view.project(w[1])) else { continue };
                    let ab = b - a;
                    let t = if ab.length_squared() > 0.0 { ((px - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                    let d = px.distance(a + ab * t);
                    if d > SNAP_PX || best.is_some_and(|(bd, _)| d >= bd) {
                        continue;
                    }
                    let along = (w[0].lerp(w[1], t) - ray.origin).dot(ray.dir);
                    if hit.is_none_or(|h| along <= h + slack) {
                        best = Some((d, e.at));
                    }
                }
            }
        }
        best.map(|(_, r)| r)
    }

    /// The quantity boxes that sit beside a pattern's arrows.
    pub(crate) fn copy_quantity_boxes(&mut self, ctx: &egui::Context) {
        if let Some(d) = self.sketch_pattern.as_mut() {
            let tips: Vec<Option<egui::Pos2>> = self.arrows.iter().map(|a| a.map(|(_, tip)| to_pos(tip))).collect();
            let slots: Vec<(&'static str, &mut u32)> = match d.mode {
                SpMode::Rectangular => vec![("sp_qty1", &mut d.count), ("sp_qty2", &mut d.count2)],
                SpMode::Circular => vec![("sp_qty1", &mut d.count_round)],
            };
            for ((id, n), tip) in slots.into_iter().zip(tips) {
                let Some(tip) = tip else { continue };
                egui::Area::new(egui::Id::new(id)).order(egui::Order::Foreground).fixed_pos(tip + egui::vec2(8.0, -30.0)).show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.add(egui::DragValue::new(n).range(1..=pattern::MAX_COPIES).speed(0.2).prefix("× "));
                    });
                });
            }
            return;
        }
        let Some(d) = self.copy_dialog.as_mut() else { return };
        let tips: Vec<Option<egui::Pos2>> = self.arrows.iter().map(|a| a.map(|(_, tip)| to_pos(tip))).collect();
        let slots: Vec<(&'static str, &mut u32)> = match d.mode {
            CopyMode::Mirror => Vec::new(),
            CopyMode::Rectangular => vec![("copy_qty1", &mut d.count), ("copy_qty2", &mut d.count2)],
            CopyMode::Circular => vec![("copy_qty1", &mut d.count_round)],
        };
        for ((id, n), tip) in slots.into_iter().zip(tips) {
            let Some(tip) = tip else { continue };
            egui::Area::new(egui::Id::new(id)).order(egui::Order::Foreground).fixed_pos(tip + egui::vec2(8.0, -30.0)).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.add(egui::DragValue::new(n).range(1..=model::MAX_COPIES).speed(0.2).prefix("× "));
                });
            });
        }
    }

    /// The Rectangular / Circular Pattern buttons in a sketch: the selection is what gets copied.
    pub(crate) fn start_sketch_pattern(&mut self, mode: SpMode) {
        let Some(i) = self.active else { return };
        let sel: Vec<sketch::Sel> = self.editor.selection.clone();
        if sel.is_empty() {
            self.notify("Select the geometry to pattern first (click or drag a box), then press the pattern button".into());
            return;
        }
        self.cancel_sketch_pattern();
        self.editor.set_tool(Tool::Select);
        self.editor.selection = sel.clone();
        self.sketch_pattern = Some(SketchPattern::new(mode, self.sketches[i].clone(), sel));
        self.sp_applied = None;
        self.notify("Set the pattern, then press Enter to apply (Esc cancels)".into());
    }

    /// Put the sketch back as it was before the pattern.
    pub(crate) fn cancel_sketch_pattern(&mut self) {
        if let (Some(d), Some(i)) = (self.sketch_pattern.take(), self.active) {
            if i < self.sketches.len() {
                self.sketches[i] = d.base;
            }
        }
        self.sp_applied = None;
        self.arrows.clear();
    }

    pub(crate) fn commit_sketch_pattern(&mut self) {
        let (Some(d), Some(i)) = (self.sketch_pattern.as_ref(), self.active) else { return };
        match d.result() {
            Ok(done) => {
                self.undo.push(d.base.clone());
                self.sketches[i] = done;
                self.sketch_pattern = None;
                self.sp_applied = None;
                self.arrows.clear();
                self.editor.selection.clear();
                self.dof = solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof);
            }
            Err(e) => self.notify(e),
        }
    }

    /// Keep the sketch showing what the open pattern would make.
    pub(crate) fn refresh_sketch_pattern(&mut self) {
        let (Some(d), Some(i)) = (self.sketch_pattern.as_ref(), self.active) else { return };
        let layout = d.layout();
        if self.sp_applied == Some(layout) || i >= self.sketches.len() {
            return;
        }
        // (A one-copy layout makes nothing: show the plain sketch.)
        self.sketches[i] = d.preview().unwrap_or_else(|_| d.base.clone());
        self.sp_applied = Some(layout);
    }

    /// Right panel while mirroring / patterning.
    pub(crate) fn copy_panel(&mut self, ui: &mut egui::Ui) {
        let bodies: Vec<(BodyId, String)> = self.body_ids.iter().map(|&id| (id, self.body_name(id))).collect();
        let axes: Vec<String> = self.axes.iter().map(|a| a.name.clone()).collect();
        let planes: Vec<String> = self.planes.iter().map(|p| p.name.clone()).collect();
        let (error, message) = (self.dialog_error.clone(), self.message.clone());
        let Some(d) = self.copy_dialog.as_mut() else { return };
        match d.panel(ui, &bodies, &axes, &planes, self.units, error.as_deref(), message.as_deref()) {
            copy_dialog::Action::Ok => self.commit_copy(),
            copy_dialog::Action::Cancel => self.cancel_copy(),
            copy_dialog::Action::None => {}
        }
    }

    pub(crate) fn set_angle_text(&mut self) {
        if let (Some(fe), Some(d)) = (self.float.as_mut(), self.revolve_dialog.as_ref()) {
            if fe.target == EditTarget::RevolveAngle {
                fe.text = format!("{:.1}", d.angle);
            }
        }
    }

    /// The profile selection of whichever dialog is open (extrude or revolve).
    pub(crate) fn profile_pick(&self) -> Option<(Option<usize>, &[usize])> {
        if let Some(d) = &self.dialog {
            Some((d.sketch, d.profiles.as_slice()))
        } else {
            self.revolve_dialog.as_ref().map(|d| (d.sketch, d.profiles.as_slice()))
        }
    }

    /// Can a sketch's profiles be picked (and are they shown) while picking?
    /// Only visible sketches — plus the one the dialog already uses, so
    /// editing an extrude shows its own (hidden) sketch. Like Fusion.
    pub(crate) fn pickable_sketch(&self, s: usize) -> bool {
        self.sketch_visible(s) || self.profile_pick().is_some_and(|(sk, _)| sk == Some(s))
    }

    pub(crate) fn profile_pick_mut(&mut self) -> Option<(&mut Option<usize>, &mut Vec<usize>)> {
        if self.dialog.is_some() {
            self.dialog.as_mut().map(|d| (&mut d.sketch, &mut d.profiles))
        } else {
            self.revolve_dialog.as_mut().map(|d| (&mut d.sketch, &mut d.profiles))
        }
    }

    // ---------- fillet / chamfer ----------

    pub(crate) fn open_edge_dialog(&mut self, editing: Option<usize>, kind: EdgeKind) {
        self.axis_pick = false;
        self.angle_pick = false;
        self.hole_dialog = None;
        self.shell_dialog = None;
        self.offset_dialog = None;
        self.combine_dialog = None;
        self.thread_dialog = None;
        self.finish_sketch();
        self.cancel_extrude();
        self.cancel_revolve();
        self.message = None;
        let d = match editing.and_then(|k| self.edge_features.get(k)) {
            Some(f) => EdgeDialog { editing, kind: f.kind, size: f.size, size2: f.size2, angle: f.angle, tangent_chain: true, edges: f.edges.clone() },
            None => EdgeDialog { editing: None, kind, size: 2.0, size2: None, angle: None, tangent_chain: true, edges: Vec::new() },
        };
        let text = sizes_text(d.size, d.size2, self.units);
        self.float = Some(FloatEdit { target: EditTarget::EdgeSize, text, focus: true, keep_text: false });
        self.edge_dialog = Some(d);
    }

    pub(crate) fn dialog_edge_feature(&self) -> Option<EdgeFeature> {
        let d = self.edge_dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.edge_features[k].name.clone(),
            None => {
                let n = self.edge_features.iter().filter(|f| f.kind == d.kind).count() + 1;
                format!("{}{n}", d.kind.name())
            }
        };
        let chamfer = d.kind == EdgeKind::Chamfer;
        Some(EdgeFeature {
            name,
            kind: d.kind,
            size: d.size,
            size2: d.size2.filter(|_| chamfer && d.angle.is_none()),
            angle: d.angle.filter(|_| chamfer),
            edges: d.edges.clone(),
        })
    }

    pub(crate) fn commit_edge_feature(&mut self) {
        let Some(f) = self.dialog_edge_feature() else { return };
        if f.edges.is_empty() {
            self.message = Some("Pick at least one edge".into());
            return;
        }
        if let Some(e) = self.dialog_error.clone() {
            self.message = Some(format!("Can't {}: {e}", f.kind.name().to_lowercase()));
            return;
        }
        match self.edge_dialog.as_ref().and_then(|d| d.editing) {
            Some(k) => self.edge_features[k] = f,
            None => {
                self.edge_features.push(f);
                self.timeline_insert(Feature::Edge(self.edge_features.len() - 1));
            }
        }
        self.cancel_edge_dialog();
    }

    pub(crate) fn cancel_edge_dialog(&mut self) {
        self.edge_dialog = None;
        self.edge_hover = None;
        if self.float.as_ref().is_some_and(|f| f.target == EditTarget::EdgeSize) {
            self.float = None;
        }
        self.message = None;
    }

    pub(crate) fn delete_edge_feature(&mut self, k: usize) {
        self.edge_features.remove(k);
        self.timeline_remove(Feature::Edge(k));
        for f in &mut self.timeline {
            if let Feature::Edge(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_edge_dialog();
    }

    pub(crate) fn set_edge_size_text(&mut self) {
        let unit = self.units;
        if let (Some(fe), Some(d)) = (self.float.as_mut(), self.edge_dialog.as_ref()) {
            if fe.target == EditTarget::EdgeSize {
                fe.text = unit.plain(d.size);
            }
        }
    }

    pub(crate) fn set_distance_text(&mut self) {
        let unit = self.units;
        let Some(fe) = self.float.as_mut() else { return };
        match fe.target {
            EditTarget::ExtrudeDistance => {
                if let Some(d) = self.dialog.as_ref() {
                    fe.text = unit.plain(d.distance);
                }
            }
            EditTarget::EdgeSize => {
                if let Some(d) = self.edge_dialog.as_ref() {
                    fe.text = sizes_text(d.size, d.size2, unit);
                }
            }
            EditTarget::RevolveAngle => {
                if let Some(d) = self.revolve_dialog.as_ref() {
                    fe.text = format!("{:.1}", d.angle);
                }
            }
            EditTarget::PlaneOffset => {
                if let Some(d) = self.plane_dialog.as_ref() {
                    fe.text = unit.plain(d.distance);
                }
            }
            EditTarget::PlaneAngle => {
                if let Some(a) = self.plane_dialog.as_ref().and_then(|d| d.angle) {
                    fe.text = format!("{:.1}", a.degrees);
                }
            }
            EditTarget::OffsetDistance => {
                if let Some(d) = self.offset_dialog.as_ref() {
                    fe.text = unit.plain(d.offset.distance);
                }
            }
            _ => {}
        }
    }

    // ---------- panels ----------

}
