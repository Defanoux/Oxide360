//! Extrude / revolve / pattern / plane / axis / fillet dialogs and their drag arrows.

use crate::*;

impl OpenCad {
    pub(crate) fn open_extrude(&mut self, editing: Option<usize>) {
        self.axis_pick = false;
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
            },
        };
        self.float = Some(FloatEdit {
            target: EditTarget::ExtrudeDistance,
            text: format!("{:.2}", d.distance),
            focus: true,
            keep_text: false,
        });
        self.dialog = Some(d);
    }

    pub(crate) fn dialog_extrude(&self) -> Option<Extrude> {
        let d = self.dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.extrudes[k].name.clone(),
            None => format!("Extrude{}", self.extrudes.len() + 1),
        };
        Some(Extrude { op: d.op, name, sketch: d.sketch?, profiles: d.profiles.clone(), distance: d.distance, extent: d.extent, distance2: d.distance2, taper: d.taper })
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
            if matches!(fe.target, EditTarget::ExtrudeDistance | EditTarget::EdgeSize | EditTarget::RevolveAngle | EditTarget::Corner(_)) {
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
            let value = sketch::parse_sizes(text).map_or(self.corner_size, |(d, _)| d);
            let line = LineHandle { origin: origin.as_dvec3(), dir: dir.as_dvec3() };
            return Some(Handle::Line { line, value, target: HandleTarget::Corner });
        }
        if let Some(d) = &self.dialog {
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
            HandleTarget::Corner => {
                // Scales a chamfer's two distances together, keeping their ratio.
                if let Some(fe) = self.float.as_mut() {
                    let v = v.max(0.01);
                    fe.text = match sketch::parse_sizes(&fe.text) {
                        Some((d1, Some(d2))) if d1 > 0.0 => format!("{v:.2}, {:.2}", d2 * v / d1),
                        _ => format!("{v:.2}"),
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
        self.cancel_extrude();
        self.cancel_revolve();
        self.cancel_edge_dialog();
        self.message = None;
        self.ensure_model(Slot::Committed);
        let d = match editing.and_then(|k| self.copies.get(k)) {
            Some(c) => CopyDialog::from_feature(editing.unwrap_or(0), c),
            None => {
                // With a single body there's nothing to choose.
                let ids: Vec<BodyId> = self.model(Slot::Committed).bodies.iter().map(|b| b.id).collect();
                CopyDialog::new(mode, if ids.len() == 1 { ids } else { Vec::new() })
            }
        };
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

    /// Close whichever tool dialog or pick mode is open.
    pub(crate) fn cancel_all_dialogs(&mut self) {
        self.cancel_extrude();
        self.cancel_revolve();
        self.cancel_edge_dialog();
        self.cancel_copy();
        self.plane_dialog = None;
        self.plane_pick = false;
        self.axis_pick = false;
        self.face_pick = false;
    }

    pub(crate) fn commit_plane(&mut self) {
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
        self.plane_dialog = None;
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
        self.plane_dialog = None;
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
        match d.panel(ui, &bodies, &axes, &planes, error.as_deref(), message.as_deref()) {
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
        self.finish_sketch();
        self.cancel_extrude();
        self.cancel_revolve();
        self.message = None;
        let d = match editing.and_then(|k| self.edge_features.get(k)) {
            Some(f) => EdgeDialog { editing, kind: f.kind, size: f.size, size2: f.size2, edges: f.edges.clone() },
            None => EdgeDialog { editing: None, kind, size: 2.0, size2: None, edges: Vec::new() },
        };
        let text = sizes_text(d.size, d.size2);
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
        Some(EdgeFeature { name, kind: d.kind, size: d.size, size2: d.size2, edges: d.edges.clone() })
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
        if let (Some(fe), Some(d)) = (self.float.as_mut(), self.edge_dialog.as_ref()) {
            if fe.target == EditTarget::EdgeSize {
                fe.text = format!("{:.2}", d.size);
            }
        }
    }

    pub(crate) fn set_distance_text(&mut self) {
        let Some(fe) = self.float.as_mut() else { return };
        match fe.target {
            EditTarget::ExtrudeDistance => {
                if let Some(d) = self.dialog.as_ref() {
                    fe.text = format!("{:.2}", d.distance);
                }
            }
            EditTarget::EdgeSize => {
                if let Some(d) = self.edge_dialog.as_ref() {
                    fe.text = sizes_text(d.size, d.size2);
                }
            }
            EditTarget::RevolveAngle => {
                if let Some(d) = self.revolve_dialog.as_ref() {
                    fe.text = format!("{:.1}", d.angle);
                }
            }
            _ => {}
        }
    }

    // ---------- panels ----------

}
