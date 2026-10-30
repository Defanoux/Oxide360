//! Solid modeling with the OpenCASCADE kernel.
//!
//! Sketch profiles become exact B-rep faces (true lines and arcs), extrudes
//! become solids, and each extrude's operation combines them with the bodies
//! built so far: New Body, Join, Cut or Intersect. Fillets and chamfers round
//! or bevel picked edges. Bodies are then meshed for the screen and for
//! STL/3MF/OBJ, and written exactly for STEP.
//!
//! The whole model is rebuilt from the timeline whenever the design changes,
//! so editing a sketch or a distance updates everything after it.

use crate::doc::{BodyId, Feature};
use crate::model::{Blob, ImportFeature, JointFeature, JointSnap, SweepFeature, SweepKind, AxisKind, ConstructionPoint, PlaneKind, PointKind, PointRef, MoveFeature, MoveKind, CombineFeature, SplitFeature, ScaleAt, ScaleFeature, OffsetFaceFeature, ShellFeature, ShellDirection, CylinderRef, ThreadFeature, HoleExtent, HoleFeature, HolePlace, TapType, Hinge, PlaneAngle, PlaneFeature, AxisFeature, CopyFeature, ExtrudeTarget, RemoveFeature, Placement, EdgeFeature, EdgeKind, EdgeRef, Extrude, Follow, Mesh, Operation, Revolve, RevolveAxis, mesh_edges};
use crate::measure::{self, Geom};
use crate::profile::{Curve, Region, find_regions, signed_area, union};
use crate::sketch::{FacePlane, Sketch};
use glam::{DQuat, DVec2, DVec3, Vec3};
use glam24::DVec3 as KVec;
use opencascade::primitives::{Edge, EdgeType, Face, Shape, Wire};
use std::collections::HashSet;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

/// Smallest volume (mm³) that counts as "something is there".
const MIN_VOLUME: f64 = 1e-6;

/// An edge of a body that can be filleted: its shape on screen and how it's
/// remembered.
#[derive(Clone)]
pub struct BodyEdge {
    pub points: Vec<Vec3>,
    pub at: EdgeRef,
    /// What the edge is, exactly (for Measure).
    pub geom: Geom,
}

pub struct Body {
    /// Which body this is, however the list shifts (see `BodyId`).
    pub id: BodyId,
    pub shape: Shape,
    pub mesh: Mesh,
    /// Real corners only (not seams or smooth tangent joins).
    pub edges: Vec<BodyEdge>,
    /// Flat faces as (outward normal, a point on the face) — where a sketch
    /// can be started.
    pub flat_faces: Vec<(DVec3, DVec3)>,
    /// Every face, in the kernel's order, as what it is (for Measure).
    pub faces: Vec<Geom>,
    /// Which of `faces` each mesh triangle belongs to.
    pub tri_face: Vec<u32>,
}

impl Body {
    /// The triangles of one of its faces (`faces[face]`).
    pub fn face_tris(&self, face: usize) -> impl Iterator<Item = [DVec3; 3]> + '_ {
        self.tri_face
            .iter()
            .zip(&self.mesh.tris)
            .filter(move |(f, _)| **f as usize == face)
            .map(|(_, t)| t.map(|i| self.mesh.positions[i as usize].as_dvec3()))
    }

    /// The edges (indices into `edges`) joined end to end with `start` so
    /// that each runs smoothly on from the last (Fusion's Tangent Chain).
    pub fn tangent_chain(&self, start: usize) -> Vec<usize> {
        // Each edge's ends and the way it leaves them.
        let ends = |e: &BodyEdge| -> Option<[(Vec3, Vec3); 2]> {
            let p = &e.points;
            let (a, b) = (*p.first()?, *p.last()?);
            Some([(a, (p.get(1)? - a).normalize_or_zero()), (b, (p[p.len() - 2] - b).normalize_or_zero())])
        };
        let size = self.mesh.positions.iter().fold(0.0f32, |m, p| m.max(p.length())).max(1.0);
        let tol = size * 1e-5;
        let mut chain = vec![start];
        let mut todo = vec![start];
        while let Some(e) = todo.pop() {
            let Some(mine) = self.edges.get(e).and_then(ends) else { continue };
            for (j, other) in self.edges.iter().enumerate() {
                if chain.contains(&j) {
                    continue;
                }
                let Some(theirs) = ends(other) else { continue };
                // Meeting at an end, leaving it in opposite directions (to
                // within the polyline's first step on an arc).
                let smooth = mine.iter().any(|&(p, d)| theirs.iter().any(|&(q, t)| p.distance(q) < tol && d.dot(t) < -0.98));
                if smooth {
                    chain.push(j);
                    todo.push(j);
                }
            }
        }
        chain
    }

    /// The edges (indices into `edges`) round face `face` (Fusion: picking a
    /// face picks its edges).
    pub fn face_edges(&self, face: usize) -> Vec<usize> {
        let Some(f) = self.shape.faces().nth(face) else { return Vec::new() };
        let mids: Vec<DVec3> = f.edges().filter_map(|e| polyline_mid(&edge_points(&e)).map(|r| r.mid)).collect();
        (0..self.edges.len()).filter(|&i| mids.iter().any(|m| m.distance(self.edges[i].at.mid) < 1e-6)).collect()
    }

    /// The flat face lying in `plane` that has `point` on it (its index).
    pub fn face_at(&self, plane: &FacePlane, point: DVec3) -> Option<usize> {
        let n = plane.normal();
        let tol = 1e-6 * (1.0 + point.length());
        (0..self.faces.len()).find(|&f| {
            let mut tris = self.face_tris(f).peekable();
            tris.peek().is_some()
                && self.face_tris(f).all(|[a, b, c]| (b - a).cross(c - a).normalize_or_zero().dot(n) > 0.9999 && n.dot(a - plane.origin).abs() < tol.max(1e-6) * 100.0)
                && tris.any(|[a, b, c]| {
                    // Inside the triangle (same side of all three edges).
                    let s = |p: DVec3, q: DVec3| (q - p).cross(point - p).dot(n);
                    let (x, y, z) = (s(a, b), s(b, c), s(c, a));
                    (x >= -1e-9 && y >= -1e-9 && z >= -1e-9) || (x <= 1e-9 && y <= 1e-9 && z <= 1e-9)
                })
        })
    }

    /// The middle of each of its flat faces, in one pass over the mesh.
    pub fn flat_face_centres(&self) -> Vec<DVec3> {
        let mut acc = vec![(DVec3::ZERO, 0.0); self.faces.len()];
        for (f, t) in self.tri_face.iter().zip(&self.mesh.tris) {
            let [a, b, c] = t.map(|i| self.mesh.positions[i as usize].as_dvec3());
            let w = (b - a).cross(c - a).length() / 2.0;
            if let Some(slot) = acc.get_mut(*f as usize) {
                slot.0 += (a + b + c) / 3.0 * w;
                slot.1 += w;
            }
        }
        acc.iter()
            .zip(&self.faces)
            .filter(|((_, area), g)| *area > 0.0 && matches!(g, Geom::Plane { .. }))
            .map(|((sum, area), _)| *sum / *area)
            .collect()
    }

    /// The middle of one of its faces (area-weighted over its triangles).
    pub fn face_centroid(&self, face: usize) -> Option<DVec3> {
        let (mut sum, mut area) = (DVec3::ZERO, 0.0);
        for [a, b, c] in self.face_tris(face) {
            let w = (b - a).cross(c - a).length() / 2.0;
            sum += (a + b + c) / 3.0 * w;
            area += w;
        }
        (area > 0.0).then(|| sum / area)
    }
}

impl Model {
    /// How many features failed to build.
    pub fn error_count(&self) -> usize {
        let n = |v: &Vec<Option<String>>| v.iter().flatten().count();
        n(&self.sketch_errors)
            + n(&self.errors)
            + n(&self.edge_errors)
            + n(&self.revolve_errors)
            + n(&self.copy_errors)
            + n(&self.axis_errors)
            + n(&self.plane_errors)
            + n(&self.remove_errors)
            + n(&self.hole_errors)
            + n(&self.thread_errors)
            + n(&self.shell_errors)
            + n(&self.offset_face_errors)
            + n(&self.combine_errors)
            + n(&self.split_errors)
            + n(&self.scale_errors)
            + n(&self.move_errors)
            + n(&self.point_errors)
            + n(&self.sweep_errors)
            + n(&self.import_errors)
            + n(&self.joint_errors)
    }
}

/// The evaluated design.
#[derive(Default)]
pub struct Model {
    pub bodies: Vec<Body>,
    /// Where each face sketch's face is in this build (None for origin-plane
    /// sketches). The app stores these back so sketches follow their faces.
    pub sketch_frames: Vec<Option<FacePlane>>,
    /// One entry per sketch: a face sketch whose face can't be found.
    pub sketch_errors: Vec<Option<String>>,
    /// One entry per extrude (same index): why it couldn't be built.
    pub errors: Vec<Option<String>>,
    /// One entry per fillet/chamfer.
    pub edge_errors: Vec<Option<String>>,
    /// One entry per revolve.
    pub revolve_errors: Vec<Option<String>>,
    /// One entry per mirror / pattern.
    pub copy_errors: Vec<Option<String>>,
    /// One entry per Remove.
    pub remove_errors: Vec<Option<String>>,
    /// Each construction axis as built: a point on it and its direction.
    pub axes: Vec<Option<(DVec3, DVec3)>>,
    /// One entry per construction axis.
    pub axis_errors: Vec<Option<String>>,
    /// Each construction plane as built, offset included.
    pub planes: Vec<Option<FacePlane>>,
    /// One entry per construction plane.
    pub plane_errors: Vec<Option<String>>,
    /// One entry per hole.
    pub hole_errors: Vec<Option<String>>,
    /// One entry per Thread feature.
    pub thread_errors: Vec<Option<String>>,
    /// One entry per Shell.
    pub shell_errors: Vec<Option<String>>,
    /// One entry per Offset Face.
    pub offset_face_errors: Vec<Option<String>>,
    /// One entry per Combine.
    pub combine_errors: Vec<Option<String>>,
    /// One entry per Split Body.
    pub split_errors: Vec<Option<String>>,
    /// One entry per Scale.
    pub scale_errors: Vec<Option<String>>,
    /// One entry per Move/Copy.
    pub move_errors: Vec<Option<String>>,
    /// Where each Move/Copy turns its bodies about (the middle of their box
    /// before the move), for its arrows and rings.
    pub move_pivots: Vec<Option<DVec3>>,
    /// Threads shown on holes (not cut): drawn as a helix.
    pub threads: Vec<CosmeticThread>,
    /// Far-end faces of the extrudes/revolves, to tag what's picked there.
    pub caps: Vec<Cap>,
    /// For each extrude / revolve / fillet that was built: the body it made or
    /// changed (the first, if several). The timeline colours its icon by it.
    pub touched: HashMap<Feature, BodyId>,
    /// One entry per sketch: the sketch with its linked projections moved to
    /// where their edges are in this build (None when none moved). The app
    /// takes these in so projected lines follow the part.
    pub linked_sketches: Vec<Option<Sketch>>,
    /// One entry per sketch: projected entities whose edge can't be found.
    pub projection_lost: Vec<Vec<usize>>,
    /// Each construction point as built.
    pub points: Vec<Option<DVec3>>,
    /// One entry per construction point.
    pub point_errors: Vec<Option<String>>,
    /// One entry per sweep / loft.
    pub sweep_errors: Vec<Option<String>>,
    /// One entry per import.
    pub import_errors: Vec<Option<String>>,
    /// One entry per joint.
    pub joint_errors: Vec<Option<String>>,
}

fn kv(p: DVec3) -> KVec {
    KVec::new(p.x, p.y, p.z)
}

/// A sketch plane in full f64: origin and in-plane axes.
#[derive(Clone, Copy)]
struct Frame {
    origin: DVec3,
    u: DVec3,
    v: DVec3,
}

impl Frame {
    fn of(sk: &Sketch) -> Self {
        let (origin, u, v) = sk.frame();
        Frame { origin, u, v }
    }

    fn from_face(f: FacePlane) -> Self {
        Frame { origin: f.origin, u: f.u, v: f.v }
    }

    fn normal(&self) -> DVec3 {
        self.u.cross(self.v)
    }
}

/// Plane-local 2D point to world 3D, in full f64.
fn world(plane: &Frame, p: DVec2) -> DVec3 {
    plane.origin + plane.u * p.x + plane.v * p.y
}

/// Where the far end of an extrude or revolve is in this build.
#[derive(Clone, Copy, Debug)]
enum End {
    Extrude { origin: DVec3, normal: DVec3, distance: f64 },
    Revolve { origin: DVec3, axis: DVec3, profile_normal: DVec3, angle: f64 },
}

/// How a far-end cap moved since a reference was picked.
#[derive(Clone, Copy, Debug)]
enum Motion {
    Translate(DVec3),
    Rotate { origin: DVec3, axis: DVec3, radians: f64 },
}

impl Motion {
    fn point(&self, p: DVec3) -> DVec3 {
        match *self {
            Motion::Translate(t) => p + t,
            Motion::Rotate { origin, axis, radians } => origin + DQuat::from_axis_angle(axis, radians) * (p - origin),
        }
    }

    fn vector(&self, v: DVec3) -> DVec3 {
        match *self {
            Motion::Translate(_) => v,
            Motion::Rotate { axis, radians, .. } => DQuat::from_axis_angle(axis, radians) * v,
        }
    }
}

/// How the end a reference follows has moved, if that feature has been built.
fn motion_for(f: Follow, extrudes: &[Option<End>], revolves: &[Option<End>]) -> Option<Motion> {
    match f {
        Follow::Extrude { feature, distance } => match extrudes.get(feature).copied().flatten()? {
            End::Extrude { normal, distance: now, .. } => Some(Motion::Translate(normal * (now - distance))),
            End::Revolve { .. } => None,
        },
        Follow::Revolve { feature, angle } => match revolves.get(feature).copied().flatten()? {
            End::Revolve { origin, axis, angle: now, .. } => {
                Some(Motion::Rotate { origin, axis, radians: (now - angle).to_radians() })
            }
            End::Extrude { .. } => None,
        },
    }
}

fn move_face(f: &FacePlane, m: Motion) -> FacePlane {
    FacePlane { origin: m.point(f.origin), u: m.vector(f.u), v: m.vector(f.v), follow: f.follow }
}

fn move_edge(r: &EdgeRef, m: Motion) -> EdgeRef {
    EdgeRef { mid: m.point(r.mid), dir: m.vector(r.dir), faces: r.faces.map(|n| m.vector(n)), follow: r.follow }
}

/// The far-end face plane of a built extrude or revolve, for the app to tag
/// what's picked on it.
#[derive(Clone, Copy, Debug)]
pub struct Cap {
    pub follow: Follow,
    pub normal: DVec3,
    pub point: DVec3,
}

fn caps_of(extrudes: &[Option<End>], revolves: &[Option<End>]) -> Vec<Cap> {
    let mut out = Vec::new();
    for (k, e) in extrudes.iter().enumerate() {
        if let Some(End::Extrude { origin, normal, distance }) = *e {
            out.push(Cap { follow: Follow::Extrude { feature: k, distance }, normal, point: origin + normal * distance });
        }
    }
    for (k, e) in revolves.iter().enumerate() {
        if let Some(End::Revolve { origin, axis, profile_normal, angle }) = *e {
            if angle.abs() < 360.0 - 1e-9 {
                let normal = DQuat::from_axis_angle(axis, angle.to_radians()) * profile_normal;
                out.push(Cap { follow: Follow::Revolve { feature: k, angle }, normal, point: origin });
            }
        }
    }
    out
}

/// Is a flat face (normal, point on it) the far end of some extrude/revolve?
pub fn follow_for_plane(caps: &[Cap], normal: DVec3, point: DVec3, tol: f64) -> Option<Follow> {
    caps.iter()
        .rev() // the latest feature wins if two ends coincide
        .find(|c| c.normal.dot(normal).abs() > 0.9999 && c.normal.dot(point - c.point).abs() < tol)
        .map(|c| c.follow)
}

/// Is an edge lying in the far end of some extrude/revolve?
pub fn follow_for_edge(caps: &[Cap], mid: DVec3, dir: DVec3, tol: f64) -> Option<Follow> {
    caps.iter()
        .rev()
        .find(|c| c.normal.dot(mid - c.point).abs() < tol && c.normal.dot(dir).abs() < 1e-6)
        .map(|c| c.follow)
}

/// The plane a sketch is on in this build (a face sketch: where its face is now).
fn frame_for(sk: &Sketch, now: Option<FacePlane>) -> Result<Frame, String> {
    match (sk.face, now) {
        (None, _) => Ok(Frame::of(sk)),
        (Some(_), Some(now)) => Ok(Frame::from_face(now)),
        (Some(_), None) => Err("its sketch's face is gone".into()),
    }
}

/// One profile loop as a wire of exact edges. Runs of segments that came
/// from the same circle become a single true arc; collinear line pieces merge.
/// Where on an ellipse (centre `c`, longer semi-axis vector `axis`, shorter
/// `minor`) point `p` is: its parameter, counter-clockwise from the axis.
fn ellipse_param(c: DVec2, axis: DVec2, minor: f64, p: DVec2) -> f64 {
    let a = axis.length();
    let (x, y) = (axis / a, (axis / a).perp());
    let d = p - c;
    (d.dot(y) / minor).atan2(d.dot(x) / a)
}

fn ellipse_point(c: DVec2, axis: DVec2, minor: f64, t: f64) -> DVec2 {
    let a = axis.length();
    let (x, y) = (axis / a, (axis / a).perp());
    c + x * (a * t.cos()) + y * (minor * t.sin())
}

/// Where the line through `far` and `near` really meets the ellipse: the
/// crossing nearest `near` (which is only on the ellipse's polyline).
fn line_meets_ellipse(c: DVec2, axis: DVec2, minor: f64, far: DVec2, near: DVec2) -> Option<DVec2> {
    let a = axis.length();
    let (x, y) = (axis / a, (axis / a).perp());
    // In the ellipse's own frame, scaled so it's the unit circle.
    let to = |p: DVec2| DVec2::new((p - c).dot(x) / a, (p - c).dot(y) / minor);
    let (p, q) = (to(far), to(near));
    let d = q - p;
    let (qa, qb, qc) = (d.dot(d), 2.0 * p.dot(d), p.dot(p) - 1.0);
    let disc = qb * qb - 4.0 * qa * qc;
    if qa < 1e-30 || disc < 0.0 {
        return None;
    }
    let t = [(-qb - disc.sqrt()) / (2.0 * qa), (-qb + disc.sqrt()) / (2.0 * qa)].into_iter().min_by(|s, u| (s - 1.0).abs().total_cmp(&(u - 1.0).abs()))?;
    Some(far + (near - far) * t)
}

fn loop_wire(plane: &Frame, pts: &[DVec2], curves: &[Curve]) -> Result<Wire, String> {
    let n = pts.len();
    if n < 2 || curves.len() != n {
        return Err("malformed profile".into());
    }
    // An ellipse is profiled as straight pieces, so where something meets it
    // is only near the true curve: move those corners onto it (a line's onto
    // where the line really crosses it), so the exact edges join up.
    let mut pts = pts.to_vec();
    for k in 0..n {
        let (before, after) = (curves[(k + n - 1) % n], curves[k]);
        if before == after {
            continue;
        }
        let ell = [before, after].into_iter().find(|c| matches!(c, Curve::Ellipse { .. }));
        let Some(Curve::Ellipse { center, axis, minor }) = ell else { continue };
        let far = match (before, after) {
            (Curve::Line, _) => Some(pts[(k + n - 1) % n]),
            (_, Curve::Line) => Some(pts[(k + 1) % n]),
            _ => None,
        };
        let on = far.and_then(|f| line_meets_ellipse(center, axis, minor, f, pts[k]));
        pts[k] = on.unwrap_or_else(|| ellipse_point(center, axis, minor, ellipse_param(center, axis, minor, pts[k])));
    }
    let pts = &pts[..];
    let p3 = |p: DVec2| kv(world(plane, p));
    // A loop that is entirely one ellipse: a closed ellipse edge.
    if let Curve::Ellipse { center, axis, minor } = curves[0] {
        if curves.iter().all(|c| *c == curves[0]) {
            let turn = if signed_area(pts) >= 0.0 { 1.0 } else { -1.0 };
            let x = (world(plane, center + axis) - world(plane, center)).normalize();
            let edge = Edge::try_ellipse(kv(world(plane, center)), kv(plane.normal() * turn), kv(x), axis.length(), minor, None).ok_or("the kernel couldn't make this ellipse")?;
            return Ok(Wire::from_edges(&[edge]));
        }
    }
    // A loop that is entirely one circle: a full-circle edge, turning the same
    // way as the loop (counter-clockwise outer, clockwise hole).
    if let Curve::Arc { center, radius } = curves[0] {
        if curves.iter().all(|c| *c == curves[0]) {
            let turn = if signed_area(pts) >= 0.0 { 1.0 } else { -1.0 };
            let normal = plane.normal() * turn;
            return Ok(Wire::from_edges(&[Edge::circle(p3(center), kv(normal), radius)]));
        }
    }
    // Start at the beginning of a run so no arc is split across the wrap.
    let start = (0..n).find(|&i| curves[i] != curves[(i + n - 1) % n]).unwrap_or(0);
    let at = |i: usize| pts[(start + i) % n];
    let curve = |i: usize| curves[(start + i) % n];
    let mut edges = Vec::new();
    let mut i = 0;
    while i < n {
        match curve(i) {
            Curve::Line => {
                let a = at(i);
                let mut b = at(i + 1);
                let mut j = i + 1;
                while j < n && curve(j) == Curve::Line {
                    let c = at(j + 1);
                    let (d1, d2) = (b - a, c - b);
                    let collinear = d1.perp_dot(d2).abs() <= 1e-9 * d1.length() * d2.length() && d1.dot(d2) > 0.0;
                    if !collinear {
                        break;
                    }
                    b = c;
                    j += 1;
                }
                edges.push(Edge::segment(p3(a), p3(b)));
                i = j;
            }
            arc @ Curve::Arc { center, radius } => {
                let mut j = i + 1;
                while j < n && curve(j) == arc {
                    j += 1;
                }
                let (a, b) = (at(i), at(j));
                let len = j - i;
                // A point on the arc between its ends fixes which way it goes round.
                let mid = if len >= 2 {
                    at(i + len / 2)
                } else {
                    center + ((a - center) + (b - center)).normalize() * radius
                };
                edges.push(Edge::arc(p3(a), p3(mid), p3(b)));
                i = j;
            }
            // A run of one ellipse's pieces: that stretch of the exact ellipse.
            ell @ Curve::Ellipse { center, axis, minor } => {
                let mut j = i + 1;
                while j < n && curve(j) == ell {
                    j += 1;
                }
                let tau = std::f64::consts::TAU;
                let param = |p: DVec2| ellipse_param(center, axis, minor, p);
                let (ua, ub, um) = (param(at(i)), param(at(j)), param(at(i + (j - i) / 2).lerp(at(i + (j - i).div_ceil(2)), 0.5)));
                let ccw = |from: f64, to: f64| (to - from).rem_euclid(tau);
                // Counter-clockwise about the sketch's normal from one end to the other, through the middle.
                let (u1, u2) = if ccw(ua, um) <= ccw(ua, ub) { (ua, ua + ccw(ua, ub)) } else { (ub, ub + ccw(ub, ua)) };
                let x = (world(plane, center + axis) - world(plane, center)).normalize();
                let edge = Edge::try_ellipse(kv(world(plane, center)), kv(plane.normal()), kv(x), axis.length(), minor, Some((u1, u2))).ok_or("the kernel couldn't make part of this ellipse")?;
                edges.push(edge);
                i = j;
            }
            // A text's straight piece: an edge of its own.
            Curve::Text { curve: 0, .. } => {
                edges.push(Edge::segment(p3(at(i)), p3(at(i + 1))));
                i += 1;
            }
            // A run of one font curve: the smooth curve through it.
            text @ Curve::Text { .. } => {
                let mut j = i + 1;
                while j < n && curve(j) == text {
                    j += 1;
                }
                let run: Vec<KVec> = (i..=j).map(|k| p3(at(k))).collect();
                let edge = if run.len() == 2 {
                    Edge::segment(run[0], run[1])
                } else {
                    Edge::try_spline_through(&run).ok_or("the kernel couldn't make a letter's curve")?
                };
                edges.push(edge);
                i = j;
            }
            // A whole loop of one spline (a closed spline): smooth all the way round.
            spline @ Curve::Spline { .. } if i == 0 && (0..n).all(|k| curve(k) == spline) => {
                let round: Vec<KVec> = (0..n).map(|k| p3(at(k))).collect();
                edges.push(Edge::try_spline(&round, true).ok_or("the kernel couldn't make this closed spline")?);
                i = n;
            }
            // A run of one spline's pieces: the smooth curve through them.
            spline @ Curve::Spline { .. } => {
                let mut j = i + 1;
                while j < n && curve(j) == spline {
                    j += 1;
                }
                let run: Vec<KVec> = (i..=j).map(|k| p3(at(k))).collect();
                let edge = if run.len() == 2 {
                    Edge::segment(run[0], run[1])
                } else {
                    Edge::try_spline_through(&run).ok_or("the kernel couldn't make a spline in this profile")?
                };
                edges.push(edge);
                i = j;
            }
        }
    }
    Ok(Wire::from_edges(&edges))
}

/// A region (outer loop + holes) as an exact planar face.
fn region_face(plane: &Frame, r: &Region) -> Result<Face, String> {
    let loops = r.loops_with_curves();
    let outer = loop_wire(plane, loops[0].0, loops[0].1)?;
    let holes = loops[1..].iter().map(|(p, c)| loop_wire(plane, p, c)).collect::<Result<Vec<_>, _>>()?;
    Ok(Face::from_wire_with_holes(&outer, &holes))
}

/// The solid an extrude adds/removes, before combining with other bodies.
/// `frames`: where face sketches are now (see `resolve_face`); others use
/// their own plane. Also returns where its far end is.
/// Where a To Object extrude's target is in this build: (a point on its plane, its normal).
fn target_plane(t: &ExtrudeTarget, planes: &[Option<FacePlane>], bodies: &[&Shape]) -> Result<(DVec3, DVec3), String> {
    match *t {
        ExtrudeTarget::Origin(op) => Ok((DVec3::ZERO, op.plane().normal().as_dvec3())),
        ExtrudeTarget::Plane(k) => planes.get(k).copied().flatten().map(|f| (f.origin, f.normal())).ok_or_else(|| "the plane it runs up to couldn't be built".into()),
        ExtrudeTarget::Face { face, anchor } => resolve_face(bodies, &face, DVec2::from(anchor))
            .map(|f| (f.origin, f.normal()))
            .ok_or_else(|| "the face it runs up to is gone — edit it and pick again".into()),
    }
}

fn tool(sketches: &[Sketch], frames: &[Option<FacePlane>], ex: &Extrude, target: Option<(DVec3, DVec3)>) -> Result<(Shape, End), String> {
    let sk = sketches.get(ex.sketch).ok_or("its sketch was deleted")?;
    if !ex.distance.is_finite() || ex.distance.abs() < 1e-6 {
        return Err("distance is zero".into());
    }
    if ex.profiles.is_empty() {
        return Err("no profiles selected".into());
    }
    let regions = find_regions(sk);
    let picked = ex
        .profiles
        .iter()
        .map(|&p| regions.get(p).ok_or("a selected profile no longer exists — the sketch changed"))
        .collect::<Result<Vec<&Region>, _>>()?;
    let plane = frame_for(sk, frames.get(ex.sketch).copied().flatten())?;
    if ex.extent == crate::model::Extent::ToObject {
        return to_object(&plane, &picked, ex, target);
    }
    let (from, to) = ex.span();
    if (to - from).abs() < 1e-6 {
        return Err("distance is zero".into());
    }
    let dir = kv(plane.normal() * (to - from));
    let mut out: Option<Shape> = None;
    for r in union(&picked) {
        let face = region_face(&plane, &r)?;
        if ex.taper != 0.0 && ex.extent != crate::model::Extent::OneSide {
            return Err("a taper works with One Side only".into());
        }
        let mut solid: Shape = if ex.taper == 0.0 {
            face.extrude(dir).into()
        } else {
            face.try_tapered_prism(dir, -(to - from).abs() * ex.taper.to_radians().tan()).ok_or("the kernel couldn't taper that far — try a smaller angle")?
        };
        if from != 0.0 {
            solid = solid.translated(kv(plane.normal() * from));
        }
        out = Some(match out {
            None => solid,
            Some(acc) => acc.try_union(&solid).ok_or("the kernel couldn't merge the profiles")?,
        });
    }
    let end = End::Extrude { origin: plane.origin, normal: plane.normal(), distance: to };
    out.map(|s| (s, end)).ok_or_else(|| "nothing to extrude".into())
}

/// To Object: the profiles extruded until they meet the target plane (cut
/// along it where it's slanted). Made by extruding well past the plane and
/// keeping the part on the sketch's side of it.
fn to_object(plane: &Frame, picked: &[&Region], ex: &Extrude, target: Option<(DVec3, DVec3)>) -> Result<(Shape, End), String> {
    let (q, n) = target.ok_or("pick what it runs up to")?;
    if ex.taper != 0.0 {
        return Err("a taper works with One Side only".into());
    }
    let normal = plane.normal();
    let slant = normal.dot(n);
    if slant.abs() < 1e-6 {
        return Err("that plane runs alongside the extrude — it never gets there".into());
    }
    // Where the profiles' middle meets the plane, along the extrude.
    let centre2 = picked.iter().map(|r| r.centroid()).sum::<DVec2>() / picked.len().max(1) as f64;
    let centre = world(plane, centre2);
    let t = (q - centre).dot(n) / slant;
    if t.abs() < 1e-6 {
        return Err("that plane is the sketch's own plane".into());
    }
    // How far the profiles spread out, so a slanted plane is passed everywhere.
    let spread = picked.iter().flat_map(|r| r.loops()).flatten().map(|p| p.distance(centre2)).fold(0.0, f64::max);
    let lean = (1.0 - slant * slant).max(0.0).sqrt() / slant.abs();
    let reach = t.abs() + spread * lean + 1.0;
    let dir = kv(normal * t.signum() * reach * 1.5);
    let mut prism: Option<Shape> = None;
    for r in union(picked) {
        let solid: Shape = region_face(plane, &r)?.extrude(dir).into();
        prism = Some(match prism {
            None => solid,
            Some(acc) => acc.try_union(&solid).ok_or("the kernel couldn't merge the profiles")?,
        });
    }
    let prism = prism.ok_or("nothing to extrude")?;
    // Everything on the sketch's side of the plane: a big slab lying on it.
    let size = (reach + spread) * 4.0 + 100.0;
    let toward_sketch = if (centre - q).dot(n) >= 0.0 { n } else { -n };
    let a = n.any_orthonormal_vector();
    let b = n.cross(a);
    let corners = [q + (a + b) * size, q + (b - a) * size, q - (a + b) * size, q + (a - b) * size];
    let edges: Vec<Edge> = (0..4).map(|k| Edge::segment(kv(corners[k]), kv(corners[(k + 1) % 4]))).collect();
    let slab: Shape = Face::from_wire(&Wire::from_edges(&edges)).extrude(kv(toward_sketch * size * 2.0)).into();
    let solid = prism.try_intersect(&slab).ok_or("the kernel couldn't stop the extrude at that plane")?;
    if solid.volume().abs() < MIN_VOLUME {
        return Err("it doesn't reach that plane".into());
    }
    Ok((solid, End::Extrude { origin: plane.origin, normal, distance: t }))
}

/// A body cut by Section Analysis: what's left of it, meshed, and which of
/// its triangles are on the cut (shown in their own colour).
#[derive(Clone, Debug)]
pub struct SectionCut {
    /// Index into the bodies it was cut from.
    pub body: usize,
    pub mesh: Mesh,
    pub cap: Vec<bool>,
}

/// Section Analysis: the bodies cut by the plane `at` moved `offset` along
/// its normal, the half the normal points into taken away (`flip`: the
/// other half). Display only; the design isn't changed. Remembered, so it
/// only cuts again when the bodies or the plane change.
pub fn section(bodies: &[Body], planes: &[Option<FacePlane>], at: &ExtrudeTarget, offset: f64, flip: bool) -> Result<Vec<SectionCut>, String> {
    if !offset.is_finite() {
        return Err("the offset isn't a number".into());
    }
    let shapes: Vec<&Shape> = bodies.iter().map(|b| &b.shape).collect();
    let (q, n) = target_plane(at, planes, &shapes)?;
    let n = if flip { -n } else { n };
    let q = q + n * if flip { -offset } else { offset };
    section_plane(bodies, q, n)
}

/// The bodies cut along the plane through `q` with normal `n`: what's on the
/// side `n` points to is taken away (Section Analysis, the Sketch Palette's Slice).
pub fn section_plane(bodies: &[Body], q: DVec3, n: DVec3) -> Result<Vec<SectionCut>, String> {
    thread_local! {
        static SECTIONS: std::cell::RefCell<Vec<(String, Result<Vec<SectionCut>, String>)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    if !(q.is_finite() && n.is_finite()) || n.length() < 1e-9 {
        return Err("the plane isn't usable".into());
    }
    if bodies.is_empty() {
        return Ok(Vec::new()); // (no box to build round nothing: OCCT would abort)
    }
    let n = n.normalize();
    let key = format!("{:?}|{q:?}|{n:?}", bodies.iter().map(|b| fingerprint(&b.shape)).collect::<Vec<_>>());
    if let Some(hit) = SECTIONS.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, r)| r.clone())) {
        return hit;
    }
    let cut = catch_unwind(AssertUnwindSafe(|| -> Result<Vec<SectionCut>, String> {
        // A slab on the kept side of the plane, bigger than everything.
        let (lo, hi) = bodies.iter().fold((DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)), |(lo, hi), b| {
            let bb = opencascade::bounding_box::aabb(&b.shape);
            (lo.min(DVec3::new(bb.min().x, bb.min().y, bb.min().z)), hi.max(DVec3::new(bb.max().x, bb.max().y, bb.max().z)))
        });
        let slab = half_space(q, n, lo, hi);
        let mut out = Vec::new();
        for (i, body) in bodies.iter().enumerate() {
            let Some(kept) = body.shape.try_intersect(&slab) else { return Err("the kernel couldn't cut a body there".into()) };
            if kept.volume().abs() < MIN_VOLUME {
                continue; // all of it is on the side taken away
            }
            let (mesh, _) = mesh_tagged(&kept)?;
            let tol = (hi - lo).length() * 1e-6 + 1e-6;
            let cap = (0..mesh.tris.len())
                .map(|t| {
                    let p = mesh.positions[mesh.tris[t][0] as usize].as_dvec3();
                    mesh.normal(t).as_dvec3().dot(n) > 0.999 && (p - q).dot(n).abs() < tol
                })
                .collect();
            out.push(SectionCut { body: i, mesh, cap });
        }
        Ok(out)
    }))
    .unwrap_or_else(|_| Err("the geometry kernel failed cutting the section".into()));
    SECTIONS.with(|c| {
        let mut c = c.borrow_mut();
        c.push((key, cut.clone()));
        if c.len() > 8 {
            drop(c.remove(0));
        }
    });
    cut
}

/// Everything behind the plane through `q` facing `n`, as far as anything in
/// the box `lo`..`hi` reaches: a slab to cut bodies with.
fn half_space(q: DVec3, n: DVec3, lo: DVec3, hi: DVec3) -> Shape {
    let size = (hi - lo).length().max(1.0) * 2.0 + (q - (lo + hi) * 0.5).length() * 2.0 + 10.0;
    // Empty or broken bounds would give NaN corners, and OCCT aborts on those.
    let size = if size.is_finite() { size } else { 1.0e4 };
    let a = n.any_orthonormal_vector();
    let b = n.cross(a);
    let corners = [q + (a + b) * size, q + (b - a) * size, q - (a + b) * size, q + (a - b) * size];
    let edges: Vec<Edge> = (0..4).map(|k| Edge::segment(kv(corners[k]), kv(corners[(k + 1) % 4]))).collect();
    Face::from_wire(&Wire::from_edges(&edges)).extrude(kv(-n * size * 2.0)).into()
}

/// Split Body: each picked body cut along the plane. The pieces on the side
/// the plane faces keep the body's id; those behind it become new bodies
/// (`Split(k)`, pieces 0, 1…).
fn apply_split(current: Tagged, sp: &SplitFeature, k: usize, planes: &[Option<FacePlane>]) -> Result<(Tagged, Option<BodyId>), String> {
    let at = sp.at.ok_or("pick the plane or face to split along")?;
    if sp.bodies.is_empty() {
        return Err("pick the bodies to split".into());
    }
    if sp.bodies.iter().any(|b| !current.iter().any(|(_, id)| id == b)) {
        return Err("a body it splits is gone — edit it and pick again".into());
    }
    let all: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
    let (q, n) = target_plane(&at, planes, &all)?;
    let failed = || "the kernel couldn't split the body there".to_string();
    let mut out: Tagged = Vec::new();
    let mut behind: Tagged = Vec::new();
    let mut cut_any = false;
    let mut piece = 0;
    for (s, id) in current {
        if !sp.bodies.contains(&id) {
            out.push((s, id));
            continue;
        }
        let bb = opencascade::bounding_box::aabb(&s);
        let (lo, hi) = (DVec3::new(bb.min().x, bb.min().y, bb.min().z), DVec3::new(bb.max().x, bb.max().y, bb.max().z));
        let slab = half_space(q, n, lo, hi);
        let solids = |shape: Option<Shape>| -> Result<Vec<Shape>, String> { Ok(shape.ok_or_else(failed)?.solids().into_iter().filter(|p| p.volume() > MIN_VOLUME).collect()) };
        let front = solids(s.try_subtract(&slab))?;
        let back = solids(s.try_intersect(&slab))?;
        if front.is_empty() || back.is_empty() {
            out.push((s, id)); // the plane misses it: left whole
            continue;
        }
        cut_any = true;
        out.extend(front.into_iter().map(|p| (p, id)));
        for p in back {
            behind.push((p, BodyId { feature: Feature::Split(k), piece }));
            piece += 1;
        }
    }
    if !cut_any {
        return Err("the plane doesn't go through the body".into());
    }
    out.extend(behind);
    Ok((uniquify(out), Some(BodyId { feature: Feature::Split(k), piece: 0 })))
}

/// Scale: the picked bodies made bigger or smaller about its point (exact for
/// one factor; per-axis factors turn curved faces into B-splines).
fn apply_scale(current: Tagged, sc: &ScaleFeature) -> Result<(Tagged, Option<BodyId>), String> {
    if !sc.factors_ok() {
        return Err("each factor must be more than 0".into());
    }
    let picked: Vec<&Shape> = current.iter().filter(|(_, id)| sc.bodies.contains(id)).map(|(s, _)| s).collect();
    if picked.is_empty() {
        return Err("the bodies it scales are gone — edit it and pick again".into());
    }
    let point = match sc.at {
        ScaleAt::Origin => DVec3::ZERO,
        ScaleAt::Point(p) => DVec3::from_array(p),
        ScaleAt::BedCentre => {
            let (lo, hi) = picked.iter().fold((DVec3::splat(f64::MAX), DVec3::splat(f64::MIN)), |(lo, hi), s| {
                let bb = opencascade::bounding_box::aabb(s);
                (lo.min(DVec3::new(bb.min().x, bb.min().y, bb.min().z)), hi.max(DVec3::new(bb.max().x, bb.max().y, bb.max().z)))
            });
            DVec3::new((lo.x + hi.x) / 2.0, (lo.y + hi.y) / 2.0, lo.z)
        }
    };
    let f = DVec3::from_array(sc.factors());
    let first = sc.bodies.iter().copied().find(|b| current.iter().any(|(_, id)| id == b));
    let mut out: Tagged = Vec::with_capacity(current.len());
    for (s, id) in current {
        if sc.bodies.contains(&id) {
            let scaled = s.try_scaled_xyz(kv(point), kv(f)).ok_or("the geometry kernel couldn't scale the body")?;
            out.push((scaled, id));
        } else {
            out.push((s, id));
        }
    }
    Ok((out, first))
}

#[cfg(test)]
mod scale_tests {
    use super::*;

    fn block(lo: DVec3, hi: DVec3) -> Shape {
        Shape::box_from_corners(kv(lo), kv(hi))
    }

    fn id(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    fn bounds(s: &Shape) -> (DVec3, DVec3) {
        let bb = opencascade::bounding_box::aabb(s);
        (DVec3::new(bb.min().x, bb.min().y, bb.min().z), DVec3::new(bb.max().x, bb.max().y, bb.max().z))
    }

    #[test]
    fn one_factor_scales_volume_by_its_cube_about_the_point() {
        let b = block(DVec3::new(10.0, 0.0, 0.0), DVec3::new(20.0, 10.0, 10.0));
        let other = block(DVec3::new(-30.0, 0.0, 0.0), DVec3::new(-20.0, 10.0, 10.0));
        let sc = ScaleFeature { bodies: vec![id(0)], factor: 2.0, ..Default::default() };
        let (out, touched) = apply_scale(vec![(b, id(0)), (other, id(1))], &sc).unwrap();
        assert_eq!(touched, Some(id(0)));
        assert!(near(out[0].0.volume(), 8000.0), "{}", out[0].0.volume());
        assert!(near(out[1].0.volume(), 1000.0), "bodies not picked stay as they are");
        // About the origin: x 10..20 becomes 20..40.
        let (lo, hi) = bounds(&out[0].0);
        assert!((lo.x - 20.0).abs() < 1e-6 && (hi.x - 40.0).abs() < 1e-6, "{lo} {hi}");
    }

    #[test]
    fn a_cylinder_keeps_its_exact_volume_and_per_axis_factors_multiply() {
        let cyl = Shape::cylinder_radius_height(5.0, 10.0);
        let sc = ScaleFeature { bodies: vec![id(0)], factor: 0.5, ..Default::default() };
        let (out, _) = apply_scale(vec![(cyl.clone(), id(0))], &sc).unwrap();
        let exact = std::f64::consts::PI * 2.5 * 2.5 * 5.0;
        assert!((out[0].0.volume() - exact).abs() < 1e-6 * exact, "{} vs {exact}", out[0].0.volume());
        // X 2, Y 1, Z 3: an elliptic cylinder of 6× the volume.
        let sc = ScaleFeature { bodies: vec![id(0)], xyz: Some([2.0, 1.0, 3.0]), ..Default::default() };
        let (out, _) = apply_scale(vec![(cyl, id(0))], &sc).unwrap();
        // (Its faces are B-splines: the adaptive volume, as Properties uses.)
        let v = std::f64::consts::PI * 25.0 * 10.0 * 6.0;
        let (got, _) = out[0].0.volume_and_centre();
        assert!((got - v).abs() < 1e-6 * v, "{got} vs {v}");
        // Meshed before scaling (as every body on screen is): the scaled one is
        // meshed afresh, not drawn with the old triangles.
        let cyl = Shape::cylinder_radius_height(5.0, 10.0);
        mesh_tagged(&cyl).unwrap();
        let (out, _) = apply_scale(vec![(cyl, id(0))], &sc).unwrap();
        let (mesh, _) = mesh_tagged(&out[0].0).unwrap();
        let mv: f64 = mesh.tris.iter().map(|t| {
            let [p, q, r] = t.map(|i| mesh.positions[i as usize].as_dvec3());
            p.dot(q.cross(r)) / 6.0
        }).sum();
        assert!((mv - v).abs() < 0.01 * v, "mesh encloses {mv}, not {v}");
    }

    #[test]
    fn bed_centre_keeps_the_bottom_where_it_was_and_bad_factors_are_errors() {
        let b = block(DVec3::new(0.0, 0.0, 5.0), DVec3::new(10.0, 20.0, 15.0));
        let sc = ScaleFeature { bodies: vec![id(0)], at: ScaleAt::BedCentre, factor: 3.0, ..Default::default() };
        let (out, _) = apply_scale(vec![(b.clone(), id(0))], &sc).unwrap();
        let (lo, hi) = bounds(&out[0].0);
        assert!((lo.z - 5.0).abs() < 1e-6 && (hi.z - 35.0).abs() < 1e-6, "bottom stays: {lo} {hi}");
        assert!((lo.x + 10.0).abs() < 1e-6 && (hi.x - 20.0).abs() < 1e-6, "centred in x: {lo} {hi}");
        // A picked point stays put.
        let sc = ScaleFeature { bodies: vec![id(0)], at: ScaleAt::Point([10.0, 20.0, 15.0]), factor: 0.5, ..Default::default() };
        let (out, _) = apply_scale(vec![(b.clone(), id(0))], &sc).unwrap();
        let (_, hi) = bounds(&out[0].0);
        assert!((hi - DVec3::new(10.0, 20.0, 15.0)).length() < 1e-6, "{hi}");
        for xyz in [Some([1.0, 0.0, 1.0]), Some([1.0, f64::NAN, 1.0])] {
            let sc = ScaleFeature { bodies: vec![id(0)], xyz, ..Default::default() };
            assert!(apply_scale(vec![(b.clone(), id(0))], &sc).is_err());
        }
        let sc = ScaleFeature { bodies: vec![id(5)], factor: 2.0, ..Default::default() };
        assert!(apply_scale(vec![(b, id(0))], &sc).is_err(), "its bodies gone");
    }
}

#[cfg(test)]
mod split_tests {
    use super::*;
    use crate::sketch::OriginPlane;

    fn block(lo: DVec3, hi: DVec3) -> Shape {
        Shape::box_from_corners(kv(lo), kv(hi))
    }

    fn id(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    fn split(at: ExtrudeTarget, bodies: Tagged, picked: &[BodyId], planes: &[Option<FacePlane>]) -> Result<Tagged, String> {
        let sp = SplitFeature { name: "Split1".into(), bodies: picked.to_vec(), at: Some(at) };
        apply_split(bodies, &sp, 0, planes).map(|(t, _)| t)
    }

    #[test]
    fn a_box_splits_exactly_and_the_top_keeps_the_body() {
        // 10 x 20 x 30 from z = -10: the XY plane leaves 20 mm above, 10 below.
        let b = block(DVec3::new(0.0, 0.0, -10.0), DVec3::new(10.0, 20.0, 20.0));
        let out = split(ExtrudeTarget::Origin(OriginPlane::XY), vec![(b, id(0))], &[id(0)], &[]).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].1, id(0), "the side the plane faces keeps the body");
        assert!(near(out[0].0.volume(), 10.0 * 20.0 * 20.0), "{}", out[0].0.volume());
        assert_eq!(out[1].1, BodyId { feature: Feature::Split(0), piece: 0 });
        assert!(near(out[1].0.volume(), 10.0 * 20.0 * 10.0), "{}", out[1].0.volume());
    }

    #[test]
    fn construction_planes_and_faces_split_and_a_miss_is_an_error() {
        let tall = block(DVec3::ZERO, DVec3::new(10.0, 10.0, 30.0));
        // A construction plane 12 mm up.
        let plane = FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 12.0));
        let out = split(ExtrudeTarget::Plane(0), vec![(tall.clone(), id(0))], &[id(0)], &[Some(plane)]).unwrap();
        assert!(near(out[0].0.volume(), 10.0 * 10.0 * 18.0) && near(out[1].0.volume(), 10.0 * 10.0 * 12.0));
        // The top face of a short block beside it, carried on past its edges.
        let short = block(DVec3::new(20.0, 0.0, 0.0), DVec3::new(30.0, 10.0, 5.0));
        let spot = DVec3::new(25.0, 5.0, 5.0);
        let face = FacePlane::from_face(DVec3::Z, spot);
        let at = ExtrudeTarget::Face { face, anchor: [spot.dot(face.u), spot.dot(face.v)] };
        let out = split(at, vec![(tall.clone(), id(0)), (short, id(1))], &[id(0)], &[]).unwrap();
        assert_eq!(out.len(), 3, "the tall one in two, the short one untouched");
        assert!(near(out[0].0.volume(), 10.0 * 10.0 * 25.0) && near(out[2].0.volume(), 10.0 * 10.0 * 5.0));
        assert!(near(out[1].0.volume(), 10.0 * 10.0 * 5.0) && out[1].1 == id(1));
        // A plane above everything.
        let high = FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 99.0));
        let Err(e) = split(ExtrudeTarget::Plane(0), vec![(tall, id(0))], &[id(0)], &[Some(high)]) else { panic!("a plane that misses split it") };
        assert!(e.contains("doesn't go through"), "{e}");
    }
}

/// A chain of sketch curves (lines, arcs, splines, or one circle) as a wire,
/// in order from one end to the other (or round, if it closes).
fn chain_wire(sk: &Sketch, plane: &Frame, entities: &[usize]) -> Result<Wire, String> {
    chain_wire_from(sk, plane, entities).map(|(w, ..)| w)
}

/// As `chain_wire`, with where it starts and which way it sets off.
fn chain_wire_from(sk: &Sketch, plane: &Frame, entities: &[usize]) -> Result<(Wire, DVec3, DVec3), String> {
    use crate::sketch::Entity;
    if entities.is_empty() {
        return Err("no path picked".into());
    }
    let p3 = |p: DVec2| kv(world(plane, p));
    // A lone circle is a closed path.
    if let [e] = entities {
        if let Some(&Entity::Circle { center, radius }) = sk.entities.get(*e) {
            let c = sk.points[center];
            let start = world(plane, c + DVec2::new(radius, 0.0));
            return Ok((Wire::from_edges(&[Edge::circle(p3(c), kv(plane.normal()), radius)]), start, plane.normal().cross(plane.u).normalize()));
        }
    }
    // Each curve's ends (as sketch points) and its points along the way.
    let mut pieces: Vec<(usize, usize, Vec<DVec2>, usize)> = Vec::new(); // (start, end, points, entity)
    for &e in entities {
        let piece = match sk.entities.get(e) {
            Some(&Entity::Line { a, b }) => (a, b, vec![sk.points[a], sk.points[b]]),
            Some(&Entity::Arc { center, a, b }) => {
                let (c, r, a0, sweep) = sk.arc_geometry(e).ok_or("an arc of the path has no size")?;
                let _ = center;
                (a, b, (0..=2).map(|k| c + DVec2::from_angle(a0 + sweep * k as f64 / 2.0) * r).collect())
            }
            Some(&Entity::Spline { s }) => {
                let list = sk.splines.get(s).ok_or("a spline of the path is damaged")?;
                (list[0], *list.last().expect("splines have points"), sk.spline_fit(s))
            }
            _ => return Err("a path is made of lines, arcs and splines (or one circle)".into()),
        };
        pieces.push((piece.0, piece.1, piece.2, e));
    }
    // Walk the chain from an end (a point only one piece touches), or anywhere if it closes.
    let uses = |p: usize, ps: &[(usize, usize, Vec<DVec2>, usize)]| ps.iter().filter(|q| q.0 == p || q.1 == p).count();
    let start = pieces.iter().flat_map(|q| [q.0, q.1]).find(|&p| uses(p, &pieces) == 1).unwrap_or(pieces[0].0);
    let mut at = start;
    let mut left = pieces;
    let mut edges = Vec::new();
    let mut set_off: Option<(DVec3, DVec3)> = None;
    while !left.is_empty() {
        let Some(k) = left.iter().position(|q| q.0 == at || q.1 == at) else { return Err("the path's curves don't join end to end".into()) };
        let (a, b, mut pts, _) = left.remove(k);
        if a != at {
            pts.reverse();
        }
        at = if a == at { b } else { a };
        if set_off.is_none() {
            // Which way the path sets off: a line's direction, an arc's tangent at its start, a spline's first step.
            let (p0, p1) = (world(plane, pts[0]), world(plane, pts[1]));
            let dir = if pts.len() == 3 {
                // An arc through three points: the tangent at its start is square to the radius there.
                let (q0, q1, q2) = (pts[0], pts[1], pts[2]);
                let d = 2.0 * (q0.x * (q1.y - q2.y) + q1.x * (q2.y - q0.y) + q2.x * (q0.y - q1.y));
                let (s0, s1, s2) = (q0.length_squared(), q1.length_squared(), q2.length_squared());
                let c = if d.abs() < 1e-12 { q1 } else { DVec2::new((s0 * (q1.y - q2.y) + s1 * (q2.y - q0.y) + s2 * (q0.y - q1.y)) / d, (s0 * (q2.x - q1.x) + s1 * (q0.x - q2.x) + s2 * (q1.x - q0.x)) / d) };
                let r = q0 - c;
                let t = DVec2::new(-r.y, r.x) * if (q1 - q0).dot(DVec2::new(-r.y, r.x)) >= 0.0 { 1.0 } else { -1.0 };
                (world(plane, q0 + t) - p0).normalize()
            } else {
                (p1 - p0).normalize()
            };
            set_off = Some((p0, dir));
        }
        let edge = if pts.len() == 2 {
            Edge::segment(p3(pts[0]), p3(pts[1]))
        } else if pts.len() == 3 {
            Edge::arc(p3(pts[0]), p3(pts[1]), p3(pts[2]))
        } else {
            Edge::try_spline_through(&pts.iter().map(|&p| p3(p)).collect::<Vec<_>>()).ok_or("the kernel couldn't make a spline of the path")?
        };
        edges.push(edge);
    }
    let (p0, dir) = set_off.ok_or("no path picked")?;
    Ok((Wire::from_edges(&edges), p0, dir))
}

/// The solid a sweep or loft adds or takes away.
fn sweep_tool(sketches: &[Sketch], frames: &[Option<FacePlane>], sf: &SweepFeature, planes: &[Option<FacePlane>], bodies: &[&Shape]) -> Result<Shape, String> {
    let sketch_and_plane = |s: usize| -> Result<(&Sketch, Frame), String> {
        let sk = sketches.get(s).ok_or("a sketch it uses was deleted")?;
        Ok((sk, frame_for(sk, frames.get(s).copied().flatten())?))
    };
    match &sf.kind {
        SweepKind::Path { sketch, profiles, path_sketch, path } => {
            if profiles.is_empty() {
                return Err("no profile picked".into());
            }
            let (sk, plane) = sketch_and_plane(*sketch)?;
            let regions = find_regions(sk);
            let picked = profiles.iter().map(|&p| regions.get(p).ok_or("a profile no longer exists — the sketch changed")).collect::<Result<Vec<&Region>, _>>()?;
            let (psk, pplane) = sketch_and_plane(*path_sketch)?;
            let wire = chain_wire(psk, &pplane, path)?;
            let mut out: Option<Shape> = None;
            for r in union(&picked) {
                let solid = region_face(&plane, &r)?.try_sweep(&wire).ok_or("the kernel couldn't sweep the profile along that path (a tight bend, or the path passing through the profile?)")?;
                out = Some(match out {
                    None => solid,
                    Some(acc) => acc.try_union(&solid).ok_or("the kernel couldn't merge the profiles")?,
                });
            }
            out.ok_or_else(|| "nothing to sweep".into())
        }
        SweepKind::Loft { sections, ruled } => {
            if sections.len() < 2 {
                return Err("a loft needs two profiles or more".into());
            }
            let mut wires = Vec::new();
            for &(s, region) in sections {
                let (sk, plane) = sketch_and_plane(s)?;
                let regions = find_regions(sk);
                let r = regions.get(region).ok_or("a profile no longer exists — its sketch changed")?;
                let loops = r.loops_with_curves();
                wires.push(loop_wire(&plane, loops[0].0, loops[0].1)?);
            }
            Shape::try_loft(&wires, *ruled).ok_or_else(|| "the kernel couldn't loft through those profiles".into())
        }
        SweepKind::Pipe { path_sketch, path, diameter, thickness } => {
            if !(diameter.is_finite() && *diameter > 0.0) {
                return Err("the diameter must be more than zero".into());
            }
            if !(thickness.is_finite() && *thickness >= 0.0 && *thickness < diameter / 2.0) {
                return Err("the wall must be thinner than half the diameter".into());
            }
            let (psk, pplane) = sketch_and_plane(*path_sketch)?;
            let (wire, start, dir) = chain_wire_from(psk, &pplane, path)?;
            // Round sections square to the path where it starts.
            let tube = |r: f64| -> Result<Shape, String> {
                let face = Face::from_wire(&Wire::from_edges(&[Edge::circle(kv(start), kv(dir), r)]));
                face.try_sweep(&wire).ok_or_else(|| "the kernel couldn't run a pipe along that path (a bend tighter than the pipe?)".into())
            };
            let outer = tube(diameter / 2.0)?;
            if *thickness > 0.0 {
                let inner = tube(diameter / 2.0 - thickness)?;
                outer.try_subtract(&inner).ok_or_else(|| "the kernel couldn't hollow the pipe".into())
            } else {
                Ok(outer)
            }
        }
        SweepKind::Coil { at, centre, diameter, turns, height, section, size } => {
            if !(diameter.is_finite() && *diameter > 0.0 && turns.is_finite() && *turns > 0.0 && height.is_finite() && height.abs() > 1e-9 && size.is_finite() && *size > 0.0) {
                return Err("diameter, revolutions, height and section size must be more than zero".into());
            }
            let (q, n) = target_plane(at, planes, bodies)?;
            let f = FacePlane::from_face(n, q);
            let o = f.origin + f.u * centre[0] + f.v * centre[1];
            let up = if *height > 0.0 { n } else { -n };
            let s = match section {
                crate::model::CoilSection::Circle => 0,
                crate::model::CoilSection::Square => 1,
                crate::model::CoilSection::Triangle => 2,
            };
            Shape::try_coil(kv(o), kv(up), kv(f.u), diameter / 2.0, height.abs() / turns, *turns, s, *size).ok_or_else(|| "the kernel couldn't make that coil (its turns overlap? make it taller or the section smaller)".into())
        }
    }
}

/// The solid a revolve adds/removes: the picked profiles spun around the axis.
fn revolve_tool(
    sketches: &[Sketch],
    frames: &[Option<FacePlane>],
    rv: &Revolve,
    axes: &[Option<(DVec3, DVec3)>],
) -> Result<(Shape, End), String> {
    let sk = sketches.get(rv.sketch).ok_or("its sketch was deleted")?;
    if !rv.angle.is_finite() || rv.angle.abs() < 1e-6 {
        return Err("angle is zero".into());
    }
    if rv.profiles.is_empty() {
        return Err("no profiles selected".into());
    }
    let plane = frame_for(sk, frames.get(rv.sketch).copied().flatten())?;
    // The axis as a point and direction, in the sketch (2D) and in the world.
    let mut world_axis: Option<(DVec3, DVec3)> = None;
    let (a2, d2) = match rv.axis {
        RevolveAxis::Construction(k) => {
            let (o, d) = axes.get(k).copied().flatten().ok_or("its construction axis couldn't be built")?;
            if d.dot(plane.normal()).abs() > 1e-6 {
                return Err("a construction axis must run parallel to the sketch to revolve about it".into());
            }
            world_axis = Some((o, d));
            let to2 = |p: DVec3| DVec2::new((p - plane.origin).dot(plane.u), (p - plane.origin).dot(plane.v));
            (to2(o), DVec2::new(d.dot(plane.u), d.dot(plane.v)).normalize_or_zero())
        }
        RevolveAxis::SketchX => (DVec2::ZERO, DVec2::X),
        RevolveAxis::SketchY => (DVec2::ZERO, DVec2::Y),
        RevolveAxis::Line(e) => match sk.entities.get(e) {
            Some(&crate::sketch::Entity::Line { a, b }) => {
                let (p, q) = (sk.points[a], sk.points[b]);
                let d = q - p;
                if d.length() < 1e-9 {
                    return Err("the axis line has no length".into());
                }
                (p, d / d.length())
            }
            _ => return Err("the axis line no longer exists — edit it and pick another".into()),
        },
    };
    let regions = find_regions(sk);
    let picked = rv
        .profiles
        .iter()
        .map(|&p| regions.get(p).ok_or("a selected profile no longer exists — the sketch changed"))
        .collect::<Result<Vec<&Region>, _>>()?;
    // The profile must stay on one side of the axis (touching it is fine):
    // spinning something across its own axis would pass through itself.
    let sides: Vec<f64> = picked.iter().flat_map(|r| r.loops()).flatten().map(|p| d2.perp_dot(*p - a2)).collect();
    let scale = sides.iter().fold(1.0f64, |m, s| m.max(s.abs()));
    let eps = 1e-9 * scale;
    if sides.iter().any(|&s| s > eps) && sides.iter().any(|&s| s < -eps) {
        return Err("the profile crosses the axis".into());
    }
    let (origin, dir) = world_axis.unwrap_or_else(|| (world(&plane, a2), plane.u * d2.x + plane.v * d2.y));
    let mut out: Option<Shape> = None;
    for r in union(&picked) {
        let solid = region_face(&plane, &r)?
            .try_revolve(kv(origin), kv(dir), rv.angle.clamp(-360.0, 360.0).to_radians())
            .ok_or("the kernel couldn't revolve this profile")?;
        out = Some(match out {
            None => solid,
            Some(acc) => acc.try_union(&solid).ok_or("the kernel couldn't merge the profiles")?,
        });
    }
    let end = End::Revolve { origin, axis: dir.normalize(), profile_normal: plane.normal(), angle: rv.angle.clamp(-360.0, 360.0) };
    out.filter(|s| s.volume().abs() > MIN_VOLUME).map(|s| (s, end)).ok_or_else(|| "nothing to revolve".into())
}

/// Bodies as they pass through the timeline, each with its identity.
type Tagged = Vec<(Shape, BodyId)>;

/// Give pieces that ended up with the same id (a cut that split a body, a
/// feature that made two solids) their own piece numbers.
fn uniquify(bodies: Tagged) -> Tagged {
    let mut used: HashSet<BodyId> = HashSet::new();
    let mut out = Vec::new();
    for (s, id) in bodies {
        let mut id = id;
        while !used.insert(id) {
            id.piece += 1;
        }
        out.push((s, id));
    }
    out
}

/// Combine a tool solid with the existing bodies. `made_by` names what new
/// bodies are called after.
fn apply(bodies: Tagged, tool: Shape, op: Operation, made_by: Feature) -> Result<(Tagged, Option<BodyId>), String> {
    let failed = || format!("the kernel couldn't {} these shapes", op.name().to_lowercase());
    let fresh = BodyId { feature: made_by, piece: 0 };
    let tag = |solids: Vec<Shape>, id: BodyId| -> Tagged { solids.into_iter().map(|s| (s, id)).collect() };
    let mut touched: Option<BodyId> = None;
    let result = match op {
        Operation::NewBody => {
            touched = Some(fresh);
            bodies.into_iter().chain(tag(tool.solids(), fresh)).collect()
        }
        Operation::Join => {
            // Merge with every body it overlaps or touches (fusing two touching
            // solids leaves fewer solids than they had separately). The merged
            // body keeps the oldest one's identity.
            let mut acc = tool;
            let mut acc_id: Option<BodyId> = None;
            let mut rest = Vec::new();
            for (b, id) in bodies {
                let before = b.solids().len() + acc.solids().len();
                let fused = b.try_union(&acc).ok_or_else(failed)?;
                if fused.solids().len() < before {
                    acc = fused;
                    acc_id = acc_id.or(Some(id));
                } else {
                    rest.push((b, id));
                }
            }
            touched = Some(acc_id.unwrap_or(fresh));
            rest.extend(tag(acc.solids(), acc_id.unwrap_or(fresh)));
            rest
        }
        Operation::Cut => {
            let mut out = Vec::new();
            for (b, id) in bodies {
                // A cut can split a body in two, or remove it entirely.
                let cut = b.try_subtract(&tool).ok_or_else(failed)?;
                if touched.is_none() && (cut.volume() - b.volume()).abs() > MIN_VOLUME {
                    touched = Some(id);
                }
                out.extend(tag(cut.solids().into_iter().filter(|s| s.volume() > MIN_VOLUME).collect(), id));
            }
            out
        }
        Operation::Intersect => {
            let mut out = Vec::new();
            for (b, id) in bodies {
                let common = b.try_intersect(&tool).ok_or_else(failed)?;
                if touched.is_none() && (common.volume() - b.volume()).abs() > MIN_VOLUME {
                    touched = Some(id);
                }
                if common.volume() > MIN_VOLUME { out.extend(tag(common.solids(), id)) } else { out.push((b, id)) }
            }
            out
        }
    };
    Ok((uniquify(result), touched))
}

/// The point halfway along a polyline (by length) and its direction there.
fn polyline_mid(points: &[DVec3]) -> Option<EdgeRef> {
    let total: f64 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    if points.len() < 2 || total <= 0.0 {
        return None;
    }
    let mut walked = 0.0;
    for w in points.windows(2) {
        let len = w[0].distance(w[1]);
        if len > 0.0 && walked + len >= total * 0.5 {
            let t = (total * 0.5 - walked) / len;
            return Some(EdgeRef { mid: w[0].lerp(w[1], t), dir: (w[1] - w[0]) / len, faces: [DVec3::ZERO; 2], follow: None });
        }
        walked += len;
    }
    None
}

/// Every edge of a shape (each once) with its reference: middle point,
/// direction there, and the normals of the two faces that meet at it.
fn shape_edges(shape: &Shape) -> Vec<(Edge, EdgeRef, Vec<DVec3>)> {
    let mut out: Vec<(Edge, EdgeRef, Vec<DVec3>)> = Vec::new();
    // Edges by their middle point (to 1e-9), to find each one fast.
    let key = |p: DVec3| ((p.x * 1e9).round() as i64, (p.y * 1e9).round() as i64, (p.z * 1e9).round() as i64);
    let mut by_mid: HashMap<(i64, i64, i64), usize> = HashMap::new();
    for e in shape.edges() {
        let pts = edge_points(&e);
        let Some(r) = polyline_mid(&pts) else { continue };
        // The explorer visits shared edges once per face; keep the first.
        if by_mid.contains_key(&key(r.mid)) || out.iter().rev().take(8).any(|(_, o, _)| o.mid.distance(r.mid) < 1e-9) {
            continue;
        }
        by_mid.insert(key(r.mid), out.len());
        out.push((e, r, pts));
    }
    // Which faces each edge belongs to: walk every face's edges and match
    // them to the list above by their middle point.
    let mut normals: Vec<Vec<DVec3>> = vec![Vec::new(); out.len()];
    for face in shape.faces() {
        for (mid, n) in face_edge_normals(&face) {
            let found = by_mid.get(&key(mid)).copied().or_else(|| out.iter().position(|(_, o, _)| o.mid.distance(mid) < 1e-9));
            if let Some(i) = found {
                if n.is_finite() && normals[i].len() < 2 {
                    normals[i].push(n);
                }
            }
        }
    }
    for (i, (_, r, _)) in out.iter_mut().enumerate() {
        if let [a, b] = normals[i][..] {
            r.faces = [a, b];
        } else if let [a] = normals[i][..] {
            r.faces = [a, a];
        }
    }
    out
}

thread_local! {
    static FACE_EDGE_NORMALS: std::cell::RefCell<HashMap<u64, Vec<(Face, Vec<(DVec3, DVec3)>)>>> = std::cell::RefCell::new(HashMap::new());
}

/// A face's edges, each as (its middle, the face's normal there) — remembered
/// per face (finding a normal on a curved face is slow).
fn face_edge_normals(face: &Face) -> Vec<(DVec3, DVec3)> {
    let h = face.hash_code();
    if let Some(v) = FACE_EDGE_NORMALS.with(|c| c.borrow().get(&h).and_then(|v| v.iter().find(|(f, _)| f.is_same(face)).map(|(_, n)| n.clone()))) {
        return v;
    }
    let v: Vec<(DVec3, DVec3)> = face
        .edges()
        .filter_map(|e| {
            let r = polyline_mid(&edge_points(&e))?;
            let n = face.normal_at(kv(r.mid));
            Some((r.mid, DVec3::new(n.x, n.y, n.z).normalize()))
        })
        .collect();
    FACE_EDGE_NORMALS.with(|c| {
        let mut c = c.borrow_mut();
        if c.values().map(Vec::len).sum::<usize>() > 50_000 {
            c.clear();
        }
        c.entry(h).or_default().push((face.clone(), v.clone()));
    });
    v
}

thread_local! {
    static EDGE_POINTS: std::cell::RefCell<HashMap<u64, Vec<(Edge, Vec<DVec3>)>>> = std::cell::RefCell::new(HashMap::new());
}

/// Points along an edge (remembered per edge: a thread's helical edges are
/// slow to sample, and a boolean keeps the edges it doesn't touch).
fn edge_points(e: &Edge) -> Vec<DVec3> {
    let h = e.hash_code();
    if let Some(p) = EDGE_POINTS.with(|c| c.borrow().get(&h).and_then(|v| v.iter().find(|(x, _)| x.is_same(e)).map(|(_, p)| p.clone()))) {
        return p;
    }
    let pts: Vec<DVec3> = e.approximation_segments().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
    EDGE_POINTS.with(|c| {
        let mut c = c.borrow_mut();
        if c.values().map(Vec::len).sum::<usize>() > 100_000 {
            c.clear();
        }
        c.entry(h).or_default().push((e.clone(), pts.clone()));
    });
    pts
}

/// Do two edges sit between faces pointing the same ways (in either order)?
fn same_faces(a: &[DVec3; 2], b: &[DVec3; 2]) -> bool {
    let near = |x: DVec3, y: DVec3| x.dot(y) > 0.99;
    (near(a[0], b[0]) && near(a[1], b[1])) || (near(a[0], b[1]) && near(a[1], b[0]))
}

/// Edges worth offering for fillets: the ones at a real corner of the
/// display mesh. Seams on cylinders and smooth line-to-arc joins drop out.
fn pickable_edges(shape: &Shape, mesh: &Mesh) -> Vec<BodyEdge> {
    let sharp: Vec<(DVec3, DVec3)> = mesh_edges(mesh)
        .into_iter()
        .filter(|e| e.sharp && e.tris[0] != e.tris[1])
        .map(|e| (mesh.positions[e.a as usize].as_dvec3(), mesh.positions[e.b as usize].as_dvec3()))
        .collect();
    let bb = opencascade::bounding_box::aabb(shape);
    let tol = (bb.max() - bb.min()).length() * 1e-3 + 1e-6;
    // The sharp segments in a grid, so each test looks only nearby (a part
    // with a thread has thousands of edges and of sharp segments).
    let mean = sharp.iter().map(|(a, b)| a.distance(*b)).sum::<f64>() / sharp.len().max(1) as f64;
    let cell = mean.max(tol * 4.0).max(1e-9);
    let at = |p: DVec3| ((p.x / cell).floor() as i64, (p.y / cell).floor() as i64, (p.z / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    let mut everywhere: Vec<usize> = Vec::new(); // too long to bin
    for (i, &(a, b)) in sharp.iter().enumerate() {
        let (lo, hi) = (at(a.min(b) - DVec3::splat(tol)), at(a.max(b) + DVec3::splat(tol)));
        let cells = (hi.0 - lo.0 + 1) * (hi.1 - lo.1 + 1) * (hi.2 - lo.2 + 1);
        if cells > 512 {
            everywhere.push(i);
            continue;
        }
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    grid.entry((x, y, z)).or_default().push(i);
                }
            }
        }
    }
    let near_sharp = |p: DVec3| {
        let close = |&i: &usize| {
            let (a, b) = sharp[i];
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-30)).clamp(0.0, 1.0);
            p.distance(a + ab * t) < tol
        };
        grid.get(&at(p)).is_some_and(|v| v.iter().any(close)) || everywhere.iter().any(close)
    };
    shape_edges(shape)
        .into_iter()
        .filter(|(_, r, pts)| {
            // Test the midpoint and the points either side of it.
            let q = pts.len() / 4;
            near_sharp(r.mid) && near_sharp(pts[q]) && near_sharp(pts[pts.len() - 1 - q])
        })
        .map(|(e, at, pts)| BodyEdge { points: pts.iter().map(|p| p.as_vec3()).collect(), at, geom: edge_geom(&e, &pts) })
        .collect()
}

/// Find the edge a reference means in the current bodies: among edges running
/// the same way *and* lying between faces pointing the same ways, the
/// nearest. That pins down "the top-front edge" however much the part grows.
/// References from older files (no face directions) only match nearby edges.
fn find_edge<'a>(edges: &'a [(usize, Edge, EdgeRef)], r: &EdgeRef, reach: f64) -> Option<&'a (usize, Edge, EdgeRef)> {
    let known_faces = r.faces[0] != DVec3::ZERO;
    let reach = if known_faces { f64::INFINITY } else { reach };
    edges
        .iter()
        .filter(|(_, _, e)| e.dir.dot(r.dir).abs() > 0.99)
        .filter(|(_, _, e)| !known_faces || same_faces(&e.faces, &r.faces))
        .map(|c| (c, c.2.mid.distance(r.mid)))
        .filter(|&(_, d)| d < reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c)
}

/// Fillet or chamfer the referenced edges.
fn apply_edges(tagged: Tagged, f: &EdgeFeature, ext: &[Option<End>], rev: &[Option<End>]) -> Result<(Tagged, Option<BodyId>), String> {
    let (bodies, ids): (Vec<Shape>, Vec<BodyId>) = tagged.into_iter().unzip();
    if !f.size.is_finite() || f.size <= 0.0 {
        return Err(format!("{} must be greater than zero", f.kind.size_label().to_lowercase()));
    }
    if f.edges.is_empty() {
        return Err("no edges selected".into());
    }
    let mut all: Vec<(usize, Edge, EdgeRef)> = Vec::new();
    let mut lo = DVec3::splat(f64::MAX);
    let mut hi = DVec3::splat(f64::MIN);
    for (i, b) in bodies.iter().enumerate() {
        let bb = opencascade::bounding_box::aabb(b);
        lo = lo.min(DVec3::new(bb.min().x, bb.min().y, bb.min().z));
        hi = hi.max(DVec3::new(bb.max().x, bb.max().y, bb.max().z));
        all.extend(shape_edges(b).into_iter().map(|(e, r, _)| (i, e, r)));
    }
    let reach = (hi - lo).length() * 0.25;
    let mut picked: Vec<Vec<(usize, DVec3)>> = vec![Vec::new(); bodies.len()];
    let mut used: HashSet<(usize, usize)> = HashSet::new();
    for r in &f.edges {
        // An edge on the far end of an extrude/revolve moves with it first.
        let moved = r.follow.and_then(|fl| motion_for(fl, ext, rev)).map(|m| move_edge(r, m));
        let r = moved.as_ref().unwrap_or(r);
        let hit = find_edge(&all, r, reach).ok_or("an edge it rounds no longer exists — edit it and pick again")?;
        let idx = all.iter().position(|c| std::ptr::eq(c, hit)).expect("hit is from this list");
        if used.insert((hit.0, idx)) {
            // (A two-distance chamfer measures its first distance on the face the user's pick faced.)
            picked[hit.0].push((idx, r.faces[0]));
        }
    }
    let mut out = Vec::new();
    for (i, b) in bodies.into_iter().enumerate() {
        if picked[i].is_empty() {
            out.push((b, ids[i]));
            continue;
        }
        let edges = picked[i].iter().map(|&(j, _)| &all[j].1);
        let two = f.size2.filter(|d2| f.kind == EdgeKind::Chamfer && (d2 - f.size).abs() > 1e-12);
        let faces_for = |b: &Shape| -> Result<Vec<(&Edge, Face)>, String> {
            let with: Option<Vec<(&Edge, Face)>> = picked[i].iter().map(|&(j, want)| edge_face(b, &all[j].2, want).map(|face| (&all[j].1, face))).collect();
            with.ok_or_else(|| "couldn't find the face a chamfer distance is measured on".into())
        };
        if let Some(a) = f.angle.filter(|_| f.kind == EdgeKind::Chamfer) {
            if !(a.is_finite() && a > 0.0 && a < 90.0) {
                return Err("the chamfer angle must be between 0° and 90°".into());
            }
        }
        if two.is_some_and(|d2| !d2.is_finite() || d2 <= 0.0) {
            return Err("chamfer distance must be greater than zero".into());
        }
        // How far it reaches across each face beside an edge (on the face the
        // pick faced, on the other): wider than a face, OCCT may run it on
        // through the wall beyond, which Fusion refuses. So does this.
        let setback = |r: &EdgeRef| -> (f64, f64) {
            match (f.kind, two, f.angle) {
                (EdgeKind::Fillet, ..) => {
                    let bend = r.faces[0].angle_between(r.faces[1]);
                    let s = f.size * (bend / 2.0).tan();
                    (s, s)
                }
                (EdgeKind::Chamfer, _, Some(a)) => (f.size, f.size * a.to_radians().tan()),
                (EdgeKind::Chamfer, Some(d2), None) => (f.size, d2),
                (EdgeKind::Chamfer, None, None) => (f.size, f.size),
            }
        };
        let tol = shape_tol(&b);
        for &(j, want) in &picked[i] {
            let r = &all[j].2;
            let first = if r.faces[0].dot(want) >= r.faces[1].dot(want) { 0 } else { 1 };
            let (near, far) = setback(r);
            for (k, need) in [(first, near), (1 - first, far)] {
                let Some(face) = edge_face(&b, r, r.faces[k]) else { continue };
                if let Some(w) = face_width(&face, &edge_points(&all[j].1), tol) {
                    if need > w * (1.0 + 1e-6) + tol {
                        return Err(format!(
                            "{} {} is wider than the face beside the edge ({})",
                            f.kind.size_label().to_lowercase(),
                            f.size,
                            crate::units::Unit::Mm.plain(w)
                        ));
                    }
                }
            }
        }
        let attempt = |k: f64| -> Result<Option<Shape>, String> {
            let shape = match (f.kind, two, f.angle) {
                (EdgeKind::Chamfer, _, Some(a)) => b.try_chamfer_edges_da(f.size * k, a.to_radians(), faces_for(&b)?),
                (EdgeKind::Fillet, ..) => b.try_fillet_edges(f.size * k, edges.clone()),
                (EdgeKind::Chamfer, None, None) => b.try_chamfer_edges(f.size * k, edges.clone()),
                (EdgeKind::Chamfer, Some(d2), None) => b.try_chamfer_edges_two(f.size * k, d2 * k, faces_for(&b)?),
            };
            Ok(shape.filter(|s| s.volume() > MIN_VOLUME))
        };
        // A round exactly as wide as the face next to it (2 on a 2 mm wall)
        // uses that face up; OCCT's fillet can't, and fails right there. A
        // hair under (a millionth) works and leaves a face nobody can see.
        let done = match attempt(1.0)? {
            Some(s) => s,
            None => attempt(1.0 - 1e-6)?.ok_or_else(|| format!("{} {} is too large for these edges", f.kind.size_label().to_lowercase(), f.size))?,
        };
        out.extend(done.solids().into_iter().map(|s| (s, ids[i])));
    }
    let touched = (0..ids.len()).find(|&i| !picked[i].is_empty()).map(|i| ids[i]);
    Ok((uniquify(out), touched))
}

/// How far a flat face (normal `n`, boundary `segments`) runs in from the
/// point `p` on its edge running along `t`: the distance, across the face, to
/// the other side. `None` when neither way crosses the face.
fn width_across(n: DVec3, segments: &[(DVec3, DVec3)], p: DVec3, t: DVec3) -> Option<f64> {
    let side = n.cross(t).normalize_or_zero();
    if side == DVec3::ZERO {
        return None;
    }
    let scale = segments.iter().map(|(a, b)| a.distance(*b)).sum::<f64>().max(1e-9);
    let skip = scale * 1e-9;
    // Crossings of the ray p + s·dir with the boundary, as distances s.
    let hits = |dir: DVec3| -> Vec<f64> {
        let mut out = Vec::new();
        for &(a, b) in segments {
            // In the face's plane: p + s·dir = a + u·(b − a).
            let e = b - a;
            let denom = dir.cross(e).dot(n);
            if denom.abs() < 1e-15 {
                continue;
            }
            let w = a - p;
            let s = w.cross(e).dot(n) / denom;
            let u = w.cross(dir).dot(n) / denom;
            if s > skip && (-1e-12..1.0 - 1e-12).contains(&u) {
                out.push(s);
            }
        }
        out
    };
    // Into the face is the way that crosses its boundary an odd number of times.
    [side, -side].into_iter().map(hits).find(|h| h.len() % 2 == 1).and_then(|h| h.into_iter().reduce(f64::min))
}

/// The narrowest a flat face is across from points along `edge` (one of its
/// edges); `None` for a curved face.
fn face_width(face: &Face, edge: &[DVec3], tol: f64) -> Option<f64> {
    let (n, _, _) = face_flat(face, tol)?;
    let segments: Vec<(DVec3, DVec3)> = face.edges().flat_map(|e| edge_points(&e).windows(2).map(|w| (w[0], w[1])).collect::<Vec<_>>()).collect();
    let k = edge.len().saturating_sub(1);
    if k == 0 {
        return None;
    }
    // A few points along the edge (not its ends, where other faces meet it).
    [0.25, 0.5, 0.75]
        .into_iter()
        .filter_map(|f: f64| {
            let i = ((k as f64 * f) as usize).min(k - 1);
            let (a, b) = (edge[i], edge[i + 1]);
            width_across(n, &segments, a.lerp(b, 0.5), (b - a).normalize_or_zero())
        })
        .reduce(f64::min)
}

/// Press Pull on a rounded face: which Fillet feature made it. The face is
/// part of a cylinder of the fillet's radius, running along one of its
/// edges, with that edge where the round's corner was: r / cos(bend/2) from
/// the axis. None when nothing matches (a hole, say).
pub fn fillet_for_face(edge_features: &[EdgeFeature], body: &Body, face: usize) -> Option<usize> {
    let Some(Geom::Cylinder { axis, radius, .. }) = body.faces.get(face) else { return None };
    let [a, b, c] = body.face_tris(face).next()?;
    let mid = (a + b + c) / 3.0;
    let n = (b - a).cross(c - a).normalize_or_zero();
    // Convex round: the axis is inward of the face; concave: outward.
    let axes = [mid - n * *radius, mid + n * *radius];
    let off_axis = |p: DVec3, o: DVec3| {
        let d = p - o;
        (d - *axis * d.dot(*axis)).length()
    };
    let tol = |want: f64| 0.02 * want + 1e-6;
    edge_features.iter().position(|ef| {
        ef.kind == EdgeKind::Fillet
            && (ef.size - radius).abs() <= 1e-5 * radius.max(1.0)
            && ef.edges.iter().any(|e| {
                if e.dir.dot(*axis).abs() < 1.0 - 1e-6 {
                    return false;
                }
                let bend = e.faces[0].angle_between(e.faces[1]);
                let corner = if e.faces[0] == DVec3::ZERO || !bend.is_finite() { radius * std::f64::consts::SQRT_2 } else { radius / (bend / 2.0).cos() };
                axes.iter().any(|&o| (off_axis(e.mid, o) - corner).abs() <= tol(corner))
            })
    })
}

/// The face of `shape` that has `edge` as one of its edges and whose outward
/// normal there points most like `want` (any such face if `want` is zero).
fn edge_face(shape: &Shape, edge: &EdgeRef, want: DVec3) -> Option<Face> {
    let mut best: Option<(f64, Face)> = None;
    for face in shape.faces() {
        let has_edge = face.edges().any(|e| {
            let pts: Vec<DVec3> = e.approximation_segments().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
            polyline_mid(&pts).is_some_and(|r| r.mid.distance(edge.mid) < 1e-6)
        });
        if !has_edge {
            continue;
        }
        let n = face.normal_at(kv(edge.mid));
        let score = DVec3::new(n.x, n.y, n.z).normalize_or_zero().dot(want);
        if best.as_ref().is_none_or(|(s, _)| score > *s) {
            best = Some((score, face));
        }
    }
    best.map(|(_, f)| f)
}

/// Triangulate a body for drawing and mesh export. Faces are meshed
/// separately by the kernel; shared vertices are welded back together so
/// edges between faces can be found and drawn.
/// Also gives the face (in `shape.faces()` order) each triangle came from.
pub fn mesh_tagged(shape: &Shape) -> Result<(Mesh, Vec<u32>), String> {
    let bb = opencascade::bounding_box::aabb(shape);
    let diag = (bb.max() - bb.min()).length();
    let deflection = (diag * 0.0005).clamp(0.001, 0.5);
    let m = shape.mesh_with_tolerance(deflection).map_err(|e| format!("meshing failed: {e:?}"))?;
    let q = (diag * 1e-7).max(1e-9);
    let mut index: HashMap<(i64, i64, i64), u32> = HashMap::new();
    let mut out = Mesh::default();
    let remap: Vec<u32> = m
        .vertices
        .iter()
        .map(|v| {
            let k = ((v.x / q).round() as i64, (v.y / q).round() as i64, (v.z / q).round() as i64);
            *index.entry(k).or_insert_with(|| {
                out.positions.push(Vec3::new(v.x as f32, v.y as f32, v.z as f32));
                (out.positions.len() - 1) as u32
            })
        })
        .collect();
    let mut tri_face = Vec::new();
    for (i, t) in m.indices.chunks_exact(3).enumerate() {
        let tri = [remap[t[0]], remap[t[1]], remap[t[2]]];
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
            out.tris.push(tri);
            tri_face.push(m.face_ends.partition_point(|&end| end <= i) as u32);
        }
    }
    Ok((out, tri_face))
}

/// What an edge is, from its curve type and points along it.
fn edge_geom(e: &Edge, pts: &[DVec3]) -> Geom {
    let length = pts.windows(2).map(|w| w[0].distance(w[1])).sum();
    match (e.edge_type(), pts) {
        (EdgeType::Line, [a, .., b]) => Geom::Line { a: *a, b: *b },
        (EdgeType::Circle, _) => measure::circle_of(pts).unwrap_or(Geom::Curve { length }),
        _ => Geom::Curve { length },
    }
}

/// How long an edge is (exact for lines and circles).
fn geom_length(g: &Geom) -> f64 {
    match *g {
        Geom::Line { a, b } => a.distance(b),
        Geom::Circle { radius, sweep, .. } => radius * sweep,
        Geom::Curve { length } => length,
        _ => 0.0,
    }
}

/// Every face of a shape (in `shape.faces()` order) as what it is: a plane
/// with its area and perimeter, a cylinder (or part of one: a fillet) with
/// its radius, or just a surface with its area.
fn face_geoms(shape: &Shape) -> Vec<Geom> {
    let tol = shape_tol(shape);
    shape.faces().map(|face| face_info(&face, tol).1).collect()
}

/// What's known about one face: (flat: outward normal, a point, its outline;
/// what it is). Faces a boolean left alone are the same faces as before, so
/// this is remembered per face (a part with a modeled thread has hundreds).
type FaceInfo = (Option<(DVec3, DVec3, Vec<DVec3>)>, Geom);

thread_local! {
    static FACE_INFO: std::cell::RefCell<HashMap<u64, Vec<(Face, FaceInfo)>>> = std::cell::RefCell::new(HashMap::new());
}

fn face_info(face: &Face, tol: f64) -> FaceInfo {
    let h = face.hash_code();
    if let Some(known) = FACE_INFO.with(|c| c.borrow().get(&h).and_then(|v| v.iter().find(|(f, _)| f.is_same(face)).map(|(_, i)| i.clone()))) {
        return known;
    }
    let flat = face_flat(face, tol);
    let info = (flat.clone(), face_geom(face, tol, flat.map(|(n, _, _)| n)));
    FACE_INFO.with(|c| {
        let mut c = c.borrow_mut();
        // (Kept faces stay alive, so a hash can't be reused by another face.)
        if c.values().map(Vec::len).sum::<usize>() > 50_000 {
            c.clear();
        }
        c.entry(h).or_default().push((face.clone(), info.clone()));
    });
    info
}

/// What a face is, for Measure (`flat`: its normal, if it's flat).
fn face_geom(face: &Face, tol: f64, flat: Option<DVec3>) -> Geom {
    {
        {
            let area = face.surface_area();
            let edges: Vec<(Vec<DVec3>, Geom)> = face
                .edges()
                .map(|e| {
                    let pts: Vec<DVec3> = e.approximation_segments().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
                    let g = edge_geom(&e, &pts);
                    (pts, g)
                })
                .collect();
            // A seam (where a cylinder's surface closes) is in the face twice; it isn't boundary.
            let mid = |pts: &[DVec3]| polyline_mid(pts).map(|r| r.mid);
            let boundary: Vec<&(Vec<DVec3>, Geom)> = edges
                .iter()
                .enumerate()
                .filter(|(i, (p, _))| !edges.iter().enumerate().any(|(j, (q, _))| j != *i && matches!((mid(p), mid(q)), (Some(a), Some(b)) if a.distance(b) < tol)))
                .map(|(_, e)| e)
                .collect();
            let loop_length = boundary.iter().map(|(_, g)| geom_length(g)).sum();
            if let Some(normal) = flat {
                // Round (a disc, or a ring round one centre): its centre too.
                let centres: Vec<DVec3> = boundary.iter().filter_map(|(_, g)| if let Geom::Circle { center, .. } = g { Some(*center) } else { None }).collect();
                let center = (!centres.is_empty() && centres.len() == boundary.len() && centres.iter().all(|c| c.distance(centres[0]) < tol * 10.0)).then(|| centres[0]);
                return Geom::Plane { normal, area, loop_length, center };
            }
            // A cylinder: its circular edges share an axis and a radius, and the
            // surface faces straight out from that axis along them.
            let circles: Vec<(&Vec<DVec3>, DVec3, f64)> = boundary
                .iter()
                .filter_map(|(p, g)| match *g {
                    Geom::Circle { normal, radius, .. } => Some((p, normal, radius)),
                    _ => None,
                })
                .collect();
            if let Some(&(_, axis, radius)) = circles.first() {
                let same = circles.iter().all(|&(_, n, r)| n.dot(axis).abs() > 1.0 - 1e-9 && (r - radius).abs() <= 1e-7 * radius.max(1.0));
                let square_on = circles.iter().all(|(pts, ..)| {
                    pts.iter().step_by((pts.len() / 6).max(1)).all(|p| {
                        let n = face.normal_at(kv(*p));
                        DVec3::new(n.x, n.y, n.z).normalize_or_zero().dot(axis).abs() < 1e-6
                    })
                });
                if same && square_on {
                    return Geom::Cylinder { axis, radius, area, loop_length };
                }
            }
            Geom::Surface { area, loop_length }
        }
    }
}

/// The flat faces of a shape as (outward normal, point on it).
fn flat_faces(shape: &Shape) -> Vec<(DVec3, DVec3)> {
    flat_faces_with_outline(shape).into_iter().map(|(n, q, _)| (n, q)).collect()
}

/// How close two points must be to count as the same, for a shape this size.
fn shape_tol(shape: &Shape) -> f64 {
    let bb = opencascade::bounding_box::aabb(shape);
    ((bb.max() - bb.min()).length() * 1e-7).max(1e-9)
}

/// The flat faces of a shape, with points around each face's boundary.
fn flat_faces_with_outline(shape: &Shape) -> Vec<(DVec3, DVec3, Vec<DVec3>)> {
    let tol = shape_tol(shape);
    shape.faces().filter_map(|face| face_info(&face, tol).0).collect()
}

/// A flat face's (outward normal, a point on it, points round its boundary).
/// A face is flat when the surface normal is the same everywhere along its
/// edges and those edges all lie in one plane.
fn face_flat(face: &Face, tol: f64) -> Option<(DVec3, DVec3, Vec<DVec3>)> {
    let pts: Vec<DVec3> = face
        .edges()
        .flat_map(|e| e.approximation_segments().collect::<Vec<_>>())
        .map(|p| DVec3::new(p.x, p.y, p.z))
        .collect();
    if pts.len() < 3 {
        return None;
    }
    let step = (pts.len() / 12).max(1);
    let sample: Vec<DVec3> = pts.iter().step_by(step).copied().collect();
    // Points on the face's own boundary always project onto its surface.
    let normals: Vec<DVec3> = sample
        .iter()
        .map(|p| {
            let n = face.normal_at(kv(*p));
            DVec3::new(n.x, n.y, n.z).normalize()
        })
        .collect();
    let n0 = normals[0];
    let flat = n0.is_finite()
        && normals.iter().all(|n| n.dot(n0) > 1.0 - 1e-9)
        && pts.iter().all(|p| (*p - sample[0]).dot(n0).abs() < tol);
    flat.then(|| (n0, sample[0], pts))
}

/// Where on its plane a sketch's geometry sits (average of its points), used
/// to tell which face it was drawn on.
fn sketch_anchor(sk: &Sketch) -> DVec2 {
    let used: Vec<DVec2> = sk
        .entities
        .iter()
        .flat_map(|e| match *e {
            crate::sketch::Entity::Line { a, b } => vec![sk.points[a], sk.points[b]],
            crate::sketch::Entity::Circle { center, .. } => vec![sk.points[center]],
            crate::sketch::Entity::Point { .. } => vec![],
            crate::sketch::Entity::Arc { center, a, b } => vec![sk.points[center], sk.points[a], sk.points[b]],
            crate::sketch::Entity::Spline { s } => sk.spline_fit(s),
            crate::sketch::Entity::Ellipse { center, .. } => vec![sk.points[center]],
            crate::sketch::Entity::Text { anchor, .. } => vec![sk.points[anchor]],
        })
        .collect();
    if used.is_empty() { DVec2::ZERO } else { used.iter().copied().sum::<DVec2>() / used.len() as f64 }
}

/// Find where a face sketch's face is now. Flat faces facing the same way are
/// candidates; the one the sketch's geometry sits over wins (so a big change
/// in thickness still finds it), nearest plane first. If the sketch isn't
/// over any of them, fall back to the nearest plane within a quarter of the
/// model's size. Only the plane's offset can change, so the sketch keeps its
/// axes and just slides along the normal with its face.
fn resolve_face(shapes: &[&Shape], f: &FacePlane, anchor: DVec2) -> Option<FacePlane> {
    // Checking every face is slow on a part with a modeled thread: remember
    // the answer for the same bodies and the same face.
    let key = key_of(&format!("{}|{f:?}|{anchor:?}", shapes.iter().map(|s| fingerprint(s)).collect::<Vec<_>>().join(";")));
    if let Some(found) = FACES_FOUND.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, r)| *r)) {
        return found;
    }
    let found = resolve_face_now(shapes, f, anchor);
    FACES_FOUND.with(|c| {
        let mut c = c.borrow_mut();
        c.push((key, found));
        bounded(&mut c);
    });
    found
}

fn resolve_face_now(shapes: &[&Shape], f: &FacePlane, anchor: DVec2) -> Option<FacePlane> {
    let n = f.normal();
    let offset = n.dot(f.origin);
    let mut lo = DVec3::splat(f64::MAX);
    let mut hi = DVec3::splat(f64::MIN);
    let mut over: Option<(f64, DVec3)> = None; // faces under the sketch
    let mut any: Option<(f64, DVec3)> = None; // any face facing the same way
    let better = |best: &Option<(f64, DVec3)>, d: f64| best.is_none_or(|(bd, _)| d < bd);
    for s in shapes {
        let bb = opencascade::bounding_box::aabb(s);
        lo = lo.min(DVec3::new(bb.min().x, bb.min().y, bb.min().z));
        hi = hi.max(DVec3::new(bb.max().x, bb.max().y, bb.max().z));
        for (fnormal, q, outline) in flat_faces_with_outline(s) {
            if fnormal.dot(n) <= 0.9999 {
                continue;
            }
            let d = (n.dot(q) - offset).abs();
            // The face's extent in sketch coordinates (its outline projected
            // onto the sketch axes).
            let (mut a, mut b) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
            for p in &outline {
                let w = DVec2::new(p.dot(f.u), p.dot(f.v));
                a = a.min(w);
                b = b.max(w);
            }
            let slack = 1e-6 * (b - a).length().max(1.0);
            if anchor.cmpge(a - slack).all() && anchor.cmple(b + slack).all() && better(&over, d) {
                over = Some((d, q));
            }
            if better(&any, d) {
                any = Some((d, q));
            }
        }
    }
    let reach = (hi - lo).length() * 0.25;
    over.or(any.filter(|&(d, _)| d <= reach)).map(|(_, q)| FacePlane { origin: n * n.dot(q), ..*f })
}

/// `radial` mm away from an axis, in the direction the body's middle lies from it.
fn radial_shift(body: &Shape, origin: DVec3, axis: DVec3, radial: f64) -> DVec3 {
    let bb = opencascade::bounding_box::aabb(body);
    let mid = (DVec3::new(bb.min().x, bb.min().y, bb.min().z) + DVec3::new(bb.max().x, bb.max().y, bb.max().z)) * 0.5;
    let axis = axis.normalize_or_zero();
    let from = mid - origin;
    let r = from - axis * from.dot(axis);
    r.try_normalize().map_or(DVec3::ZERO, |d| d * radial)
}

/// Where a point reference is in what's built so far: a body corner is the
/// corner nearest where it was picked.
fn resolve_point(bodies: &[&Shape], p: PointRef, points: &[Option<DVec3>]) -> Result<DVec3, String> {
    match p {
        PointRef::Origin => Ok(DVec3::ZERO),
        PointRef::Point(k) => points.get(k).copied().flatten().ok_or_else(|| "the construction point it goes through couldn't be placed".into()),
        PointRef::Corner(q) => bodies
            .iter()
            .flat_map(|b| shape_edges(b).into_iter().flat_map(|(_, _, pts)| [pts.first().copied(), pts.last().copied()]).flatten().collect::<Vec<_>>())
            .min_by(|a, b| a.distance(q).total_cmp(&b.distance(q)))
            .ok_or_else(|| "the corner it goes through is gone".into()),
    }
}

/// Where a construction point is in what's built so far.
fn place_point(bodies: &[&Shape], p: &ConstructionPoint) -> Result<DVec3, String> {
    match p.kind {
        PointKind::Corner(q) => resolve_point(bodies, PointRef::Corner(q), &[]),
        PointKind::OnFace { face, anchor } => resolve_snap(bodies, &JointSnap::Face { face, anchor }).map(|(_, p, _)| p),
        PointKind::CircleCentre(r) => {
            let all: Vec<(usize, Edge, EdgeRef, Vec<DVec3>)> = bodies.iter().enumerate().flat_map(|(i, b)| shape_edges(b).into_iter().map(move |(e, r, pts)| (i, e, r, pts))).collect();
            let list: Vec<(usize, Edge, EdgeRef)> = all.iter().map(|(i, e, r, _)| (*i, e.clone(), *r)).collect();
            let hit = find_edge(&list, &r, f64::INFINITY).ok_or("the circle it's at the centre of is gone")?;
            let k = list.iter().position(|c| std::ptr::eq(c, hit)).expect("from this list");
            match edge_geom(&all[k].1, &all[k].3) {
                Geom::Circle { center, .. } => Ok(center),
                _ => Err("its edge isn't round any more".into()),
            }
        }
    }
}

/// A midplane, three-point or tangent plane, from what's built so far
/// (`planes`: the construction planes before it).
pub fn construct_plane(kind: &PlaneKind, bodies: &[&Shape], planes: &[Option<FacePlane>], points: &[Option<DVec3>]) -> Result<FacePlane, String> {
    match kind {
        PlaneKind::Midplane(a, b) => {
            let ((q1, n1), (q2, n2)) = (target_plane(a, planes, bodies)?, target_plane(b, planes, bodies)?);
            if n1.dot(n2).abs() > 1.0 - 1e-9 {
                // Parallel: halfway between, facing as the first.
                let q = q1 + n1 * ((q2 - q1).dot(n1) / 2.0);
                Ok(FacePlane::from_face(n1, q))
            } else {
                // Meeting: the plane halving the angle between them, through where they meet.
                let (d1, d2, c) = (n1.dot(q1), n2.dot(q2), n1.dot(n2));
                let p = (n1 * (d1 - d2 * c) + n2 * (d2 - d1 * c)) / (1.0 - c * c);
                let n = (n1 - n2).try_normalize().ok_or("those two are the same plane")?;
                Ok(FacePlane::from_face(n, p))
            }
        }
        PlaneKind::ThreePoints(pts) => {
            let [a, b, c] = [resolve_point(bodies, pts[0], points)?, resolve_point(bodies, pts[1], points)?, resolve_point(bodies, pts[2], points)?];
            let n = (b - a).cross(c - a).try_normalize().filter(|n| n.is_finite()).ok_or("the three points are in a line")?;
            Ok(FacePlane::from_face(n, a))
        }
        PlaneKind::Tangent(r) => {
            let c = find_cylinder(bodies, r).ok_or("the cylinder it touches is gone")?;
            let along = (r.at - c.axis_point).dot(c.axis);
            let radial = (r.at - c.axis_point - c.axis * along).try_normalize().ok_or("pick the cylinder away from its axis")?;
            Ok(FacePlane::from_face(radial, c.axis_point + c.axis * along + radial * c.radius))
        }
    }
}

/// The most triangles an STL may have to be made into a solid (each becomes
/// a face before flat areas are merged; much more takes minutes).
pub const STL_LIMIT: usize = 50_000;

/// How many triangles an STL file has (binary: its header says; text: its
/// `facet` lines). None when it isn't readable.
pub fn stl_triangles(path: &Path) -> Option<usize> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut head = [0u8; 84];
    let got = f.read(&mut head).ok()?;
    if got == 84 {
        let n = u32::from_le_bytes([head[80], head[81], head[82], head[83]]) as u64;
        if 84 + 50 * n == len {
            return Some(n as usize);
        }
    }
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.lines().filter(|l| l.trim_start().starts_with("facet")).count())
}

/// A STEP or STL file's solids as BRep text, to keep in the design (see
/// `ImportFeature`). STL triangles are sewn into solids, with coplanar
/// ones merged into single faces.
pub fn import_file(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let shape = match ext.as_str() {
        "step" | "stp" => Shape::try_read_step(path).ok_or("couldn't read it as STEP")?,
        "stl" => {
            let n = stl_triangles(path).ok_or("couldn't read it as STL")?;
            if n > STL_LIMIT {
                return Err(format!(
                    "it has {n} triangles; solids can be made from meshes of up to {STL_LIMIT}. Reduce it first (PrusaSlicer: right-click it, Simplify; Meshmixer: Select All, then Reduce)"
                ));
            }
            let s = Shape::try_read_stl(path).ok_or("couldn't read it as STL")?;
            if s.solids().is_empty() {
                return Err("the mesh has holes in it (it isn't closed), so it can't be a solid — repair it (PrusaSlicer, Meshmixer) and try again".into());
            }
            s
        }
        _ => return Err("only STEP (.step, .stp) and STL (.stl) files can be imported".into()),
    };
    if shape.solids().is_empty() {
        return Err("there are no solids in it (only surfaces)".into());
    }
    let text = shape.to_brep_string();
    if text.is_empty() {
        return Err("the geometry kernel couldn't store it".into());
    }
    Ok(text)
}

/// The points a click can snap to on the flat face of `body` that mesh
/// triangle `tri` is part of (as Fusion offers when you hover a face): its
/// middle, the middle of each straight side, and its corners — plus the
/// face's outward normal. None when that face isn't flat. (A hole's centre
/// is picked by its round edge instead.)
pub fn face_snaps(body: &Body, tri: usize) -> Option<(DVec3, Vec<DVec3>)> {
    let face = *body.tri_face.get(tri)? as usize;
    if !matches!(body.faces.get(face), Some(Geom::Plane { .. })) {
        return None;
    }
    let n = body.mesh.normal(tri).as_dvec3();
    // The middle: the face's own triangles, weighted by area.
    let (mut sum, mut area) = (DVec3::ZERO, 0.0);
    for (_, idx) in body.mesh.tris.iter().enumerate().filter(|(t, _)| body.tri_face.get(*t) == Some(&(face as u32))) {
        let [a, b, c] = idx.map(|i| body.mesh.positions[i as usize].as_dvec3());
        let w = (b - a).cross(c - a).length() / 2.0;
        sum += (a + b + c) / 3.0 * w;
        area += w;
    }
    if area <= 0.0 {
        return None;
    }
    // Exactly, from the face itself (the mesh's only approximates a round hole).
    let exact = catch_unwind(AssertUnwindSafe(|| body.shape.faces().nth(face).map(|f| f.center_of_mass()))).ok().flatten().map(|c| DVec3::new(c.x, c.y, c.z));
    let centre = exact.unwrap_or(sum / area);
    let on_plane = |p: DVec3| n.dot(p - centre).abs() < 1e-6 * (1.0 + centre.length());
    let mut points = vec![centre];
    let add = |p: DVec3, points: &mut Vec<DVec3>| {
        if !points.iter().any(|q| q.distance(p) < 1e-6) {
            points.push(p);
        }
    };
    let sides: Vec<&BodyEdge> = body.edges.iter().filter(|e| e.at.faces.iter().any(|f| f.dot(n) > 0.999)).collect();
    // Middles of straight sides first, then corners (the ends of straight sides and of arcs).
    for e in &sides {
        if let Geom::Line { a, b } = e.geom {
            if on_plane(a) && on_plane(b) {
                add((a + b) / 2.0, &mut points);
            }
        }
    }
    for e in &sides {
        let ends = match e.geom {
            Geom::Line { a, b } => Some((a, b)),
            Geom::Circle { sweep, .. } if sweep < std::f64::consts::TAU - 1e-9 => e.points.first().zip(e.points.last()).map(|(a, b)| (a.as_dvec3(), b.as_dvec3())),
            _ => None,
        };
        if let Some((a, b)) = ends.filter(|(a, b)| on_plane(*a) && on_plane(*b)) {
            add(a, &mut points);
            add(b, &mut points);
        }
    }
    Some((n, points))
}

/// A joint origin in `bodies`: which body it's on, the point, and its Z.
pub fn resolve_snap(bodies: &[&Shape], s: &JointSnap) -> Result<(usize, DVec3, DVec3), String> {
    match *s {
        JointSnap::Corner(q) => bodies
            .iter()
            .enumerate()
            .flat_map(|(i, b)| shape_edges(b).into_iter().flat_map(move |(_, _, pts)| [pts.first().copied(), pts.last().copied()]).flatten().map(move |p| (i, p)))
            .min_by(|a, b| a.1.distance(q).total_cmp(&b.1.distance(q)))
            .map(|(i, p)| (i, p, DVec3::Z))
            .ok_or_else(|| "the corner it's at is gone".into()),
        JointSnap::Circle(r) => {
            let all: Vec<(usize, Edge, EdgeRef, Vec<DVec3>)> = bodies.iter().enumerate().flat_map(|(i, b)| shape_edges(b).into_iter().map(move |(e, r, pts)| (i, e, r, pts))).collect();
            let list: Vec<(usize, Edge, EdgeRef)> = all.iter().map(|(i, e, r, _)| (*i, e.clone(), *r)).collect();
            let hit = find_edge(&list, &r, f64::INFINITY).ok_or("the round edge it's at is gone")?;
            let k = list.iter().position(|c| std::ptr::eq(c, hit)).expect("from this list");
            let Geom::Circle { center, normal, .. } = edge_geom(&all[k].1, &all[k].3) else { return Err("its edge isn't round any more".into()) };
            // Out of the flat face the circle lies on (one of the two faces meeting there).
            let z = all[k].2.faces.into_iter().find(|f| f.dot(normal).abs() > 0.99).unwrap_or(normal);
            Ok((all[k].0, center, z.normalize()))
        }
        JointSnap::Face { face, anchor } => {
            // On the body whose face it is: the spot must be on (or at) that body.
            for (i, b) in bodies.iter().enumerate() {
                if let Some(f) = resolve_face(&[*b], &face, DVec2::from(anchor)) {
                    let n = f.normal();
                    let p = n * n.dot(f.origin) + f.u * anchor[0] + f.v * anchor[1];
                    let on = exact_bounds(b).is_some_and(|(lo, hi)| {
                        let tol = 1e-6 * (hi - lo).max_element().max(1.0);
                        p.cmpge(lo - DVec3::splat(tol)).all() && p.cmple(hi + DVec3::splat(tol)).all()
                    });
                    if on {
                        return Ok((i, p, n));
                    }
                }
            }
            Err("the face it's on is gone".into())
        }
    }
}

/// The rotation turning frame (`za`, its x from `zero_normal`) onto (`zb`, likewise).
fn frame_turn(za: DVec3, zb: DVec3) -> glam::DQuat {
    let frame = |z: DVec3| {
        let x = crate::model::zero_normal(z, DVec3::X);
        glam::DMat3::from_cols(x, z.cross(x), z)
    };
    glam::DQuat::from_mat3(&(frame(zb) * frame(za).transpose())).normalize()
}

/// Move the component at the joint's `a` so it meets `b` (see `JointFeature`).
fn apply_joint(bodies: Tagged, j: &JointFeature, components: &[crate::assembly::Component]) -> Result<(Tagged, Option<BodyId>), String> {
    let shapes: Vec<&Shape> = bodies.iter().map(|(s, _)| s).collect();
    let (ia, pa, za) = resolve_snap(&shapes, &j.a)?;
    let moving_id = bodies[ia].1;
    let group: Vec<BodyId> = crate::assembly::owner(components, moving_id).map_or_else(|| vec![moving_id], |k| components[k].bodies.clone());
    let moves = |id: &BodyId| *id == moving_id || group.contains(id);
    // The other end must be on a part that stays.
    let still: Vec<usize> = (0..bodies.len()).filter(|&i| !moves(&bodies[i].1)).collect();
    let still_shapes: Vec<&Shape> = still.iter().map(|&i| &bodies[i].0).collect();
    let (_, pb, zb) = resolve_snap(&still_shapes, &j.b).map_err(|e| if still.is_empty() { "the second point must be on another part".to_string() } else { e })?;
    let target = if j.flip { zb } else { -zb };
    let (turn, slide) = j.kind.motion(j.motion);
    let (j_angle, j_offset) = (j.angle + turn, j.offset + slide);
    let (axis, angle) = frame_turn(za, target).to_axis_angle();
    let place = |s: &Shape| -> Shape {
        let mut out = if angle.abs() > 1e-12 { s.rotated_about(kv(pa), kv(axis), angle) } else { s.clone() };
        if pb != pa {
            out = out.translated(kv(pb - pa));
        }
        if j_angle != 0.0 {
            out = out.rotated_about(kv(pb), kv(zb), j_angle.to_radians());
        }
        if j_offset != 0.0 {
            out = out.translated(kv(zb * j_offset));
        }
        out
    };
    let out: Tagged = bodies.into_iter().map(|(s, id)| if moves(&id) { (place(&s), id) } else { (s, id) }).collect();
    Ok((out, Some(moving_id)))
}

/// An import turned about the middle of its box, then moved (as Free Move).
fn placed(shape: &Shape, angles: [f64; 3], offset: [f64; 3]) -> Shape {
    let mut out = shape.clone();
    if angles.iter().any(|a| *a != 0.0) {
        if let Some((lo, hi)) = exact_bounds(shape) {
            let pivot = (lo + hi) * 0.5;
            for (axis, deg) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().zip(angles) {
                if deg != 0.0 {
                    out = out.rotated_about(kv(pivot), kv(axis), deg.to_radians());
                }
            }
        }
    }
    let t = DVec3::from(offset);
    if t != DVec3::ZERO { out.translated(kv(t)) } else { out }
}

/// An import's box once turned by `angles` (before any move): what the
/// Insert dialog's Centre / Place on ground / click place by.
pub fn import_bounds(brep: &Blob, angles: [f64; 3]) -> Option<(DVec3, DVec3)> {
    let shape = imported(brep).ok()?;
    exact_bounds(&placed(&shape, angles, [0.0; 3]))
}

thread_local! {
    /// Imports read back from their text lately (reading big ones takes a while).
    static IMPORTED: std::cell::RefCell<Vec<(Blob, Shape)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// An import's shape, from the text kept in the design.
fn imported(brep: &Blob) -> Result<Shape, String> {
    if let Some(s) = IMPORTED.with(|c| c.borrow().iter().find(|(b, _)| b == brep).map(|(_, s)| s.clone())) {
        return Ok(s);
    }
    let shape = Shape::try_from_brep_string(&brep.0).filter(|s| !s.solids().is_empty()).ok_or("its stored geometry couldn't be read")?;
    IMPORTED.with(|c| {
        let mut c = c.borrow_mut();
        c.push((brep.clone(), shape.clone()));
        if c.len() > 8 {
            c.remove(0);
        }
    });
    Ok(shape)
}

/// An axis through two points or through a cylinder: a point on it and its direction.
pub fn construct_axis(kind: &AxisKind, bodies: &[&Shape], points: &[Option<DVec3>]) -> Result<(DVec3, DVec3), String> {
    match kind {
        AxisKind::TwoPoints([a, b]) => {
            let (a, b) = (resolve_point(bodies, *a, points)?, resolve_point(bodies, *b, points)?);
            let d = (b - a).try_normalize().filter(|d| d.is_finite()).ok_or("the two points are the same")?;
            Ok((a, d))
        }
        AxisKind::Cylinder(r) => {
            let c = find_cylinder(bodies, r).ok_or("the cylinder it runs through is gone")?;
            Ok((c.axis_point, c.axis))
        }
    }
}

/// Where a construction axis is now: along the straight edge it was picked on.
fn resolve_axis(bodies: &Tagged, af: &AxisFeature, points: &[Option<DVec3>]) -> Result<(DVec3, DVec3), String> {
    if let Some(kind) = &af.kind {
        return construct_axis(kind, &bodies.iter().map(|(s, _)| s).collect::<Vec<_>>(), points);
    }
    let mut all: Vec<(usize, Edge, EdgeRef)> = Vec::new();
    for (i, (b, _)) in bodies.iter().enumerate() {
        all.extend(shape_edges(b).into_iter().filter(|(_, _, pts)| is_straight(pts)).map(|(e, r, _)| (i, e, r)));
    }
    let found = find_edge(&all, &af.edge, f64::INFINITY).ok_or("the edge it was made on no longer exists — delete it and pick again")?;
    Ok((found.2.mid, found.2.dir.normalize()))
}

/// Plane at Angle: the line it turns about as built so far (a point on it, its
/// direction) and which way the plane faces at 0°.
fn hinge_line(
    a: &PlaneAngle,
    bodies: &Tagged,
    axes: &[Option<(DVec3, DVec3)>],
    axis_features: &[AxisFeature],
    sketches: &[Sketch],
    frames: &[Option<FacePlane>],
) -> Result<(DVec3, DVec3, DVec3), String> {
    match a.hinge {
        Hinge::World(w) => Ok((DVec3::ZERO, w.dir(), Hinge::world_normal(w))),
        Hinge::Axis(k) => {
            let (o, d) = axes.get(k).copied().flatten().ok_or("its construction axis couldn't be built")?;
            Ok((o, d, axis_features.get(k).map_or(DVec3::Z, |af| af.edge.faces[0])))
        }
        Hinge::Edge(r) => {
            let (o, d) = resolve_axis(bodies, &AxisFeature { name: String::new(), edge: r, kind: None }, &[])?;
            Ok((o, d, r.faces[0]))
        }
        Hinge::SketchLine { sketch, line } => {
            let sk = sketches.get(sketch).ok_or("its sketch was deleted")?;
            let plane = frame_for(sk, frames.get(sketch).copied().flatten())?;
            match sk.entities.get(line) {
                Some(&crate::sketch::Entity::Line { a, b }) => {
                    let (p, q) = (world(&plane, sk.points[a]), world(&plane, sk.points[b]));
                    if p.distance(q) < 1e-9 {
                        return Err("its line has no length".into());
                    }
                    Ok(((p + q) * 0.5, q - p, plane.normal()))
                }
                _ => Err("its line no longer exists — delete the plane and make it again".into()),
            }
        }
    }
}

/// A thread that is only drawn (Fusion's cosmetic thread).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CosmeticThread {
    /// Where it starts (on the face) and which way it runs.
    pub origin: DVec3,
    pub axis: DVec3,
    /// Drawn on the surface it's on: just inside a hole's wall, just
    /// outside a shaft (so the part hides it from the far side).
    pub radius: f64,
    pub pitch: f64,
    pub length: f64,
}

/// The middle of the flat face found for (`face`, `anchor`) (its area centroid).
fn face_centre(bodies: &[&Shape], face: &FacePlane, anchor: DVec2) -> Option<DVec3> {
    let n = face.normal();
    let mut best: Option<(f64, DVec3)> = None;
    for s in bodies {
        let tol = shape_tol(s);
        for f in s.faces() {
            let Some((fnormal, q, outline)) = face_flat(&f, tol) else { continue };
            if fnormal.dot(n) <= 0.9999 || (q - face.origin).dot(n).abs() > tol.max(1e-6) {
                continue;
            }
            let (mut a, mut b) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
            for p in &outline {
                let w = DVec2::new(p.dot(face.u), p.dot(face.v));
                (a, b) = (a.min(w), b.max(w));
            }
            // The face under the anchor; else the nearest one.
            let d = if anchor.cmpge(a).all() && anchor.cmple(b).all() { 0.0 } else { ((a + b) * 0.5).distance(anchor) };
            if best.is_none_or(|(bd, _)| d < bd) {
                let c = f.center_of_mass();
                best = Some((d, DVec3::new(c.x, c.y, c.z)));
            }
        }
    }
    best.map(|(_, c)| c)
}

/// Where each place of a hole is now: the point on its face, the way it
/// drills, and the diameter of the circle it sits on (if it does).
fn hole_places(h: &HoleFeature, sketches: &[Sketch], frames: &[Option<FacePlane>], bodies: &[&Shape]) -> Result<Vec<(DVec3, DVec3, Option<f64>)>, String> {
    let sketch_at = |s: usize| -> Result<(&Sketch, Frame), String> {
        let sk = sketches.get(s).ok_or("its sketch was deleted")?;
        Ok((sk, frame_for(sk, frames.get(s).copied().flatten())?))
    };
    let mut out = Vec::new();
    for place in &h.places {
        let (p, inward, circle) = match *place {
            HolePlace::SketchPoint { sketch, point } => {
                let (sk, plane) = sketch_at(sketch)?;
                let q = sk.points.get(point).ok_or("a point it was on is gone — edit it and pick again")?;
                (world(&plane, *q), -plane.normal(), None)
            }
            HolePlace::SketchCircle { sketch, entity } => {
                let (sk, plane) = sketch_at(sketch)?;
                match sk.entities.get(entity) {
                    Some(&crate::sketch::Entity::Circle { center, radius }) => (world(&plane, sk.points[center]), -plane.normal(), Some(radius * 2.0)),
                    Some(&crate::sketch::Entity::Arc { center, a, .. }) => {
                        (world(&plane, sk.points[center]), -plane.normal(), Some(sk.points[center].distance(sk.points[a]) * 2.0))
                    }
                    _ => return Err("a circle it was on is gone — edit it and pick again".into()),
                }
            }
            HolePlace::Face { face, anchor, centre } => {
                let a = DVec2::from(anchor);
                let now = resolve_face(bodies, &face, a).ok_or("the face it was on is gone — edit it and pick again")?;
                let p = if centre {
                    face_centre(bodies, &now, a).ok_or("the face it was on is gone — edit it and pick again")?
                } else {
                    now.origin + now.u * a.x + now.v * a.y
                };
                (p, -now.normal(), None)
            }
        };
        out.push((p, if h.flip { -inward } else { inward }, circle));
    }
    Ok(out)
}

/// The groove a tapped hole's thread cuts beyond the drilled (minor) bore:
/// the space the screw's tooth fills (basic 60° profile: 3P/4 wide at the
/// minor diameter, P/8 at the major), starting a little inside the bore so
/// it shares no face with it. From `start` to `end` along the axis from `p`
/// (an NPT thread has its stated size at `end`, the hole's small end).
fn thread_groove(t: &crate::threads::Thread, p: DVec3, dir: DVec3, start: f64, end: f64, right_hand: bool) -> Result<Shape, String> {
    let pitch = t.pitch;
    let length = end - start;
    let (taper, (radii, widths)) = if t.standard == crate::threads::Standard::Npt {
        // A sharp V 0.8 P deep; at the sweep's start wider than the small end
        // by the taper (radius: half of 1:16).
        let e = t.npt_small_pitch_diameter() / 2.0 + length * crate::threads::NPT_TAPER / 2.0;
        let (r_in, r_out) = (e - 0.4 * pitch - 0.1 * pitch, e + 0.4 * pitch);
        let w_out = pitch / 8.0;
        (-crate::threads::NPT_TAPER / 2.0, ((r_in, r_out), (w_out + 2.0 * (r_out - r_in) * 30f64.to_radians().tan(), w_out)))
    } else {
        (0.0, thread_profile(t.minor() / 2.0, true, pitch))
    };
    let ((r_in, r_out), (w_in, w_out)) = (radii, widths);
    let dir = dir.normalize();
    let x = crate::model::zero_normal(dir, DVec3::Z);
    Shape::try_thread(kv(p + dir * start), kv(dir), kv(x), (r_in, r_out), (w_in, w_out), pitch, length, right_hand, taper)
        .ok_or_else(|| "the kernel couldn't model this thread — try it cosmetic (Modeled off)".into())
}

/// A whole cylindrical face of a body: the centre of one end, the axis
/// (toward the other end), its radius and length, and whether it's the
/// inside of a hole (material outside it) or a shaft.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cylinder {
    pub axis_point: DVec3,
    pub axis: DVec3,
    pub radius: f64,
    pub length: f64,
    pub internal: bool,
}

impl Cylinder {
    pub fn as_ref(&self, at: DVec3) -> CylinderRef {
        CylinderRef { axis_point: self.axis_point, axis: self.axis, radius: self.radius, at, internal: self.internal, length: self.length }
    }
}

/// The face as a whole cylinder (each end a full circle, though a join may
/// have split it into arcs), if it is one.
fn face_cylinder(face: &Face) -> Option<Cylinder> {
    // The ends: arcs grouped by centre, (centre, normal, radius, swept so far, a point on it).
    let mut ends: Vec<(DVec3, DVec3, f64, f64, DVec3)> = Vec::new();
    for e in face.edges() {
        let pts: Vec<DVec3> = e.approximation_segments().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
        match edge_geom(&e, &pts) {
            Geom::Circle { center, normal, radius, sweep } => match ends.iter_mut().find(|g| g.0.distance(center) < 1e-6 * radius.max(1.0)) {
                Some(g) => g.3 += sweep,
                None => ends.push((center, normal, radius, sweep, pts[0])),
            },
            Geom::Line { .. } => {} // the seam
            _ => return None,
        }
    }
    let full = |g: &(DVec3, DVec3, f64, f64, DVec3)| (g.3 - std::f64::consts::TAU).abs() < 1e-6;
    let [(c0, n0, r0, _, p0), (c1, n1, r1, _, _)] = ends[..] else { return None };
    if !ends.iter().all(full) {
        return None;
    }
    let along = c1 - c0;
    if along.length() < 1e-9 || (r0 - r1).abs() > 1e-7 * r0.max(1.0) || n0.dot(n1).abs() < 1.0 - 1e-9 || along.normalize().dot(n0).abs() < 1.0 - 1e-9 {
        return None;
    }
    let axis = along.normalize();
    let n = face.normal_at(kv(p0));
    let radial = p0 - c0 - axis * (p0 - c0).dot(axis);
    let internal = DVec3::new(n.x, n.y, n.z).dot(radial) < 0.0;
    Some(Cylinder { axis_point: c0, axis, radius: r0, length: along.length(), internal })
}

/// Face `index` of a body (in the kernel's order, as `Body::faces`) as a cylinder.
pub fn cylinder_face(shape: &Shape, index: usize) -> Option<Cylinder> {
    shape.faces().nth(index).and_then(|f| face_cylinder(&f))
}

/// The cylinder a thread was put on, found again among the bodies: same
/// axis direction, radius close, axis nearest the one picked.
pub fn find_cylinder(bodies: &[&Shape], r: &CylinderRef) -> Option<Cylinder> {
    let line_dist = |c: &Cylinder| {
        let d = r.axis_point - c.axis_point;
        (d - c.axis * d.dot(c.axis)).length()
    };
    bodies
        .iter()
        .flat_map(|s| s.faces().filter_map(|f| face_cylinder(&f)).collect::<Vec<_>>())
        .filter(|c| c.axis.dot(r.axis).abs() > 1.0 - 1e-6 && (c.radius - r.radius).abs() < 0.5 * r.radius)
        .min_by(|a, b| (line_dist(a) + (a.radius - r.radius).abs()).total_cmp(&(line_dist(b) + (b.radius - r.radius).abs())))
        .filter(|c| line_dist(c) < r.radius.max(1.0))
}

/// The basic 60° thread profile cut into a cylinder of radius `r` (a hole:
/// outward, a shaft: inward), 5H/8 deep, starting 0.1 P on the air side so
/// it shares no face with the cylinder: (r_in, r_out, w_in, w_out).
pub fn thread_profile(r: f64, internal: bool, pitch: f64) -> ((f64, f64), (f64, f64)) {
    let tan30 = 30f64.to_radians().tan();
    let depth = 0.625 * crate::threads::H_PER_PITCH * pitch;
    if internal {
        let (r_in, r_out) = (r - 0.1 * pitch, r + depth);
        let w_out = pitch / 8.0;
        ((r_in, r_out), (w_out + 2.0 * (r_out - r_in) * tan30, w_out))
    } else {
        let (r_in, r_out) = (r - depth, r + 0.1 * pitch);
        let w_in = pitch / 4.0;
        ((r_in, r_out), (w_in, w_in + 2.0 * (r_out - r_in) * tan30))
    }
}

/// How much a thread on a cylinder of radius `r` takes out per unit length:
/// the profile's part inside the material, screwed round.
pub fn thread_removal_per_length(r: f64, internal: bool, pitch: f64) -> f64 {
    let ((r_in, r_out), (w_in, w_out)) = thread_profile(r, internal, pitch);
    let width = |x: f64| w_in + (w_out - w_in) * (x - r_in) / (r_out - r_in);
    let (a, b) = if internal { (r, r_out) } else { (r_in, r) };
    let (wa, wb) = (width(a), width(b));
    let area = (wa + wb) / 2.0 * (b - a);
    let r_c = a + (b - a) * (wa + 2.0 * wb) / (3.0 * (wa + wb));
    area * std::f64::consts::TAU * r_c / pitch
}

/// The face of `shape` (its number in `faces()` order) that a picked face
/// means now: flat, in the same plane, with the anchor on it.
fn face_index(shape: &Shape, face: &FacePlane, anchor: DVec2) -> Option<usize> {
    let now = resolve_face(&[shape], face, anchor)?;
    let n = now.normal();
    let tol = shape_tol(shape);
    shape.faces().enumerate().find_map(|(i, f)| {
        let (fnormal, q, outline) = face_info(&f, tol).0?;
        if fnormal.dot(n) <= 0.9999 || (q - now.origin).dot(n).abs() > tol.max(1e-6) * 100.0 {
            return None;
        }
        let (mut a, mut b) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for p in &outline {
            let w = DVec2::new(p.dot(now.u), p.dot(now.v));
            (a, b) = (a.min(w), b.max(w));
        }
        let slack = 1e-6 * (b - a).length().max(1.0);
        (anchor.cmpge(a - slack).all() && anchor.cmple(b + slack).all()).then_some(i)
    })
}

/// The middle of the shapes' bounding box (None for none).
fn box_middle(shapes: &[&Shape]) -> Option<DVec3> {
    let mut lo = DVec3::splat(f64::MAX);
    let mut hi = DVec3::splat(f64::MIN);
    for s in shapes {
        let bb = opencascade::bounding_box::aabb(s);
        lo = lo.min(DVec3::new(bb.min().x, bb.min().y, bb.min().z));
        hi = hi.max(DVec3::new(bb.max().x, bb.max().y, bb.max().z));
    }
    (!shapes.is_empty()).then(|| (lo + hi) * 0.5)
}

/// Move/Copy: the bodies picked, moved (keeping their ids, in place in the
/// list) — or, with `copy`, moved copies added as new bodies.
fn apply_move(bodies: Tagged, mf: &MoveFeature, k: usize, axes: &[Option<(DVec3, DVec3)>], pivot: DVec3) -> Result<(Tagged, Option<BodyId>), String> {
    if !mf.kind.is_finite() {
        return Err("the move isn't a number".into());
    }
    let moved = |s: &Shape| -> Result<Shape, String> {
        Ok(match mf.kind {
            MoveKind::Free { offset, angles } => {
                let mut out = s.clone();
                for (axis, deg) in [DVec3::X, DVec3::Y, DVec3::Z].into_iter().zip(angles) {
                    if deg != 0.0 {
                        out = out.rotated_about(kv(pivot), kv(axis), deg.to_radians());
                    }
                }
                let t = DVec3::from(offset);
                if t != DVec3::ZERO { out.translated(kv(t)) } else { out }
            }
            MoveKind::Rotate { axis, custom, angle } => {
                let (o, d) = match custom {
                    Some(c) => axes.get(c).copied().flatten().ok_or("the axis it turns about is gone")?,
                    None => (DVec3::ZERO, axis.dir()),
                };
                s.rotated_about(kv(o), kv(d), angle.to_radians())
            }
        })
    };
    let mut out: Tagged = Vec::new();
    let mut made: Tagged = Vec::new();
    let mut first = None;
    for (s, id) in bodies {
        if !mf.bodies.contains(&id) {
            out.push((s, id));
            continue;
        }
        let m = moved(&s)?;
        if mf.copy {
            let new = BodyId { feature: Feature::Move(k), piece: made.len() };
            first = first.or(Some(new));
            made.extend(m.solids().into_iter().map(|x| (x, new)));
            out.push((s, id));
        } else {
            first = first.or(Some(id));
            out.extend(m.solids().into_iter().map(|x| (x, id)));
        }
    }
    out.extend(made);
    Ok((uniquify(out), first))
}

/// Combine: the tools joined to, cut from or intersected with the target
/// (each piece of it). The result keeps the target's id; the tools go unless
/// kept.
fn apply_combine(current: Tagged, c: &CombineFeature) -> Result<(Tagged, Option<BodyId>), String> {
    let target = c.target.ok_or("pick the target body")?;
    if c.op == Operation::NewBody {
        return Err("Combine joins, cuts or intersects".into());
    }
    let tools: Vec<BodyId> = c.tools.iter().copied().filter(|t| *t != target).collect();
    if tools.is_empty() {
        return Err("pick the tool bodies".into());
    }
    let target_shapes: Vec<Shape> = current.iter().filter(|(_, id)| *id == target).map(|(s, _)| s.clone()).collect();
    if target_shapes.is_empty() {
        return Err("the target body is gone — edit it and pick again".into());
    }
    if tools.iter().any(|t| !current.iter().any(|(_, id)| id == t)) {
        return Err("a tool body is gone — edit it and pick again".into());
    }
    let tool_shapes: Vec<Shape> = current.iter().filter(|(_, id)| tools.contains(id)).map(|(s, _)| s.clone()).collect();
    let failed = || format!("the kernel couldn't {} these bodies", c.op.name().to_lowercase());
    let mut made: Vec<Shape> = Vec::new();
    for t in target_shapes {
        let mut acc = t;
        for tool in &tool_shapes {
            acc = match c.op {
                Operation::Join => acc.try_union(tool),
                Operation::Cut => acc.try_subtract(tool),
                _ => acc.try_intersect(tool),
            }
            .ok_or_else(failed)?;
        }
        made.extend(acc.solids().into_iter().filter(|s| s.volume() > MIN_VOLUME));
    }
    if made.is_empty() {
        return Err(match c.op {
            Operation::Intersect => "the bodies don't overlap".into(),
            _ => "nothing of the target is left".into(),
        });
    }
    // The result where the target was in the list; the tools dropped unless kept.
    let mut out: Tagged = Vec::new();
    let mut placed = false;
    for (s, id) in current {
        if id == target {
            if !placed {
                out.extend(made.iter().map(|m| (m.clone(), target)));
                placed = true;
            }
        } else if !tools.contains(&id) || c.keep_tools {
            out.push((s, id));
        }
    }
    Ok((uniquify(out), Some(target)))
}

/// Offset Face: the faces picked move out along their normals by the
/// distance (in, when negative), each body's faces in one go; OCCT's faster
/// mode first, its sturdier intersecting mode when that fails.
fn apply_offset_faces(current: Tagged, of: &OffsetFaceFeature) -> Result<(Tagged, Option<BodyId>), String> {
    if of.faces.is_empty() {
        return Err("pick the faces to move".into());
    }
    if !of.distance.is_finite() {
        return Err("the distance isn't a number".into());
    }
    let mut found = vec![false; of.faces.len()];
    let mut out = Vec::new();
    let mut touched = None;
    for (shape, id) in current {
        let mut picked: Vec<i32> = Vec::new();
        for (k, p) in of.faces.iter().enumerate() {
            if let Some(i) = face_index(&shape, &p.face, DVec2::from(p.anchor)) {
                found[k] = true;
                if !picked.contains(&(i as i32)) {
                    picked.push(i as i32);
                }
            }
        }
        if picked.is_empty() || of.distance == 0.0 {
            out.push((shape, id));
            continue;
        }
        let offsets = vec![of.distance; picked.len()];
        let made = shape
            .try_offset_faces(&picked, &offsets, false)
            .or_else(|| shape.try_offset_faces(&picked, &offsets, true))
            .filter(|s| s.volume() > MIN_VOLUME)
            .ok_or("couldn't move the face that far")?;
        touched = touched.or(Some(id));
        out.extend(made.solids().into_iter().map(|s| (s, id)));
    }
    if found.iter().any(|f| !f) {
        return Err("a face it moves is gone — edit it and pick again".into());
    }
    Ok((uniquify(out), touched))
}

/// Shell: each body with faces picked on it is hollowed through them; the
/// bodies listed are hollowed closed all round.
fn apply_shell(current: Tagged, sf: &ShellFeature) -> Result<(Tagged, Option<BodyId>), String> {
    let (inside, outside) = sf.walls()?;
    if sf.faces.is_empty() && sf.bodies.is_empty() {
        return Err("pick faces to open, or a body to hollow".into());
    }
    let mut found = vec![false; sf.faces.len()];
    let mut out = Vec::new();
    let mut touched = None;
    let failed = || "the kernel couldn't shell this — try a thinner wall".to_string();
    for (shape, id) in current {
        let open: Vec<i32> = sf
            .faces
            .iter()
            .enumerate()
            .filter_map(|(k, p)| {
                let i = face_index(&shape, &p.face, DVec2::from(p.anchor))?;
                found[k] = true;
                Some(i as i32)
            })
            .collect();
        let whole = sf.bodies.contains(&id);
        if open.is_empty() && !whole {
            out.push((shape, id));
            continue;
        }
        let r = sf.rounded;
        let made = if open.is_empty() {
            // Closed all round: the space between the part shrunk and grown.
            let outer = if outside > 0.0 { shape.try_offset(outside, r).ok_or_else(failed)? } else { shape.clone() };
            let inner = if inside > 0.0 { shape.try_offset(-inside, r).ok_or_else(failed)? } else { shape.clone() };
            outer.try_subtract(&inner).ok_or_else(failed)?
        } else {
            let thick = |t: f64| shape.try_thick_solid(&open, t, r).ok_or_else(failed);
            match sf.direction {
                ShellDirection::Inside => thick(-inside)?,
                ShellDirection::Outside => thick(outside)?,
                ShellDirection::Both => thick(-inside)?.try_union(&thick(outside)?).ok_or_else(failed)?,
            }
        };
        // OCCT hands back the part unchanged when the wall is too thick for it.
        if (made.volume() - shape.volume()).abs() <= 1e-9 * shape.volume().abs().max(1.0) || made.volume() <= MIN_VOLUME {
            return Err("the wall is thicker than the part allows".into());
        }
        touched = touched.or(Some(id));
        out.extend(made.solids().into_iter().map(|s| (s, id)));
    }
    if found.contains(&false) {
        return Err("a face it opens is gone — edit it and pick again".into());
    }
    if sf.bodies.iter().any(|b| !out.iter().any(|(_, id)| id == b)) {
        return Err("a body it hollows is gone".into());
    }
    Ok((out, touched))
}

/// Cut or draw a Thread feature's threads.
fn apply_threads(current: Tagged, tf: &ThreadFeature, made_by: Feature, shown: &mut Vec<CosmeticThread>) -> Result<(Tagged, Option<BodyId>), String> {
    let t = tf.spec.thread().ok_or_else(|| format!("unknown thread {}", tf.spec.designation))?;
    if tf.faces.is_empty() {
        return Err("no face picked".into());
    }
    let mut acc = current;
    let mut touched = None;
    for r in &tf.faces {
        let bodies: Vec<&Shape> = acc.iter().map(|(s, _)| s).collect();
        let c = find_cylinder(&bodies, r).ok_or("the face it was on is gone — edit it and pick again")?;
        // Start from the end nearer where it was clicked.
        let (start, dir) = if (r.at - c.axis_point).dot(c.axis) <= c.length / 2.0 { (c.axis_point, c.axis) } else { (c.axis_point + c.axis * c.length, -c.axis) };
        let (from, to) = tf.span(0.0, c.length)?;
        if !tf.spec.modeled {
            let radius = c.radius * if c.internal { 0.99 } else { 1.01 };
            shown.push(CosmeticThread { origin: start + dir * from, axis: dir, radius, pitch: t.pitch, length: to - from });
            continue;
        }
        let ((r_in, r_out), widths) = thread_profile(c.radius, c.internal, t.pitch);
        if r_in <= 0.0 {
            return Err("the cylinder is too thin for this thread".into());
        }
        // Swept past both ends, then trimmed to the span (so a shoulder isn't notched).
        let lead = 1.5 * t.pitch;
        let x = crate::model::zero_normal(dir, DVec3::Z);
        // The groove only depends on the cylinder and the thread: remember it
        // (a change elsewhere in the part needn't sweep it again).
        let key = format!("groove|{start:?}|{dir:?}|{:?}|{from}|{to}|{:?}", (c.radius, c.internal), tf.spec);
        let groove = match GROOVES.with(|g| g.borrow().iter().find(|(k, _)| *k == key).map(|(_, s)| s.clone())) {
            Some(g) => g,
            None => {
                let swept = Shape::try_thread(kv(start + dir * (from - lead)), kv(dir), kv(x), (r_in, r_out), widths, t.pitch, to - from + 2.0 * lead, tf.spec.right_hand, 0.0)
                    .ok_or("the kernel couldn't model this thread — try it cosmetic (Modeled off)")?;
                let span = Shape::cylinder(kv(start + dir * from), r_out + 1.0, kv(dir), to - from);
                let g = swept.try_intersect(&span).ok_or("the kernel couldn't trim this thread")?;
                GROOVES.with(|c| {
                    let mut c = c.borrow_mut();
                    c.push((key, g.clone()));
                    bounded(&mut c);
                });
                g
            }
        };
        let before: f64 = acc.iter().map(|(s, _)| s.volume()).sum();
        let (next, t_id) = apply(acc, groove, Operation::Cut, made_by)?;
        let removed = before - next.iter().map(|(s, _)| s.volume()).sum::<f64>();
        // OCCT can fail quietly beside a helix: check it took out about what it should.
        let want = thread_removal_per_length(c.radius, c.internal, t.pitch) * (to - from);
        // (Less is also a thread breaking through a thin wall; much less, or
        // more, is a cut gone wrong.)
        if removed < want * 0.5 || removed > want * 1.02 {
            return Err("the kernel couldn't cut this thread — try it cosmetic (Modeled off)".into());
        }
        (acc, touched) = (next, touched.or(t_id));
    }
    Ok((acc, touched))
}

// --- Remembering slow results between rebuilds ---
//
// Every change rebuilds the timeline from the start, and booleans on a part
// with a modeled thread (and analysing its hundreds of helical faces) take a
// second or more. A feature whose inputs are unchanged gives back its last
// result, and a body that is literally the same shape keeps its mesh and
// faces. Input bodies are recognised by a fingerprint: a remembered result's
// shapes carry one from the inputs that made them (free); a shape built fresh
// gets a geometric one (volume, faces, box).

/// How many results are kept.
const MEMO: usize = 64;

/// A feature's result: bodies, the body it made or changed, threads it
/// shows, and where an extrude's or revolve's far end is.
type FeatureResult = (Tagged, Option<BodyId>, Vec<CosmeticThread>, Option<End>);

/// A short key for everything a feature reads.
fn key_of(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    format!("{:016x}{}", h.finish(), text.len())
}

/// What a sketch contributes to the solid (not its names, labels or units).
fn sketch_geometry(sk: Option<&Sketch>) -> String {
    sk.map_or(String::new(), |s| format!("{:?}{:?}{:?}{:?}{:?}{:?}", s.plane, s.face, s.plane_ref, s.points, s.entities, s.splines))
}
type BodyData = (Mesh, Vec<u32>, Vec<BodyEdge>, Vec<(DVec3, DVec3)>, Vec<Geom>);

thread_local! {
    static FINGERPRINTS: std::cell::RefCell<Vec<(Shape, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    static FEATURES: std::cell::RefCell<Vec<(String, FeatureResult)>> = const { std::cell::RefCell::new(Vec::new()) };
    static BODIES: std::cell::RefCell<Vec<(Shape, BodyData)>> = const { std::cell::RefCell::new(Vec::new()) };
    static GROOVES: std::cell::RefCell<Vec<(String, Shape)>> = const { std::cell::RefCell::new(Vec::new()) };
    static FACES_FOUND: std::cell::RefCell<Vec<(String, Option<FacePlane>)>> = const { std::cell::RefCell::new(Vec::new()) };
    static PROPERTIES: std::cell::RefCell<Vec<(Shape, (f64, f64, DVec3))>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// A body's volume (mm³), surface area (mm²) and centre of mass, exactly
/// (remembered per shape: Measure asks every frame the cursor is over it).
/// A body's exact bounding box (min, max), from its geometry.
pub fn exact_bounds(shape: &Shape) -> Option<(DVec3, DVec3)> {
    let (lo, hi) = catch_unwind(AssertUnwindSafe(|| shape.try_exact_bounds())).ok().flatten()?;
    Some((DVec3::new(lo.x, lo.y, lo.z), DVec3::new(hi.x, hi.y, hi.z)))
}

/// The lines of a 2-D drawing view (exact hidden-line removal by
/// OpenCASCADE), as polylines in paper millimetres at 1:1, Y up the paper.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewLines {
    /// Sharp edges you can see.
    pub visible: Vec<Vec<DVec2>>,
    /// Outlines of curved faces you can see (a cylinder's sides).
    pub outline: Vec<Vec<DVec2>>,
    /// Smooth edges you can see (where a fillet meets a face; drawn thin).
    pub smooth: Vec<Vec<DVec2>>,
    /// Edges and outlines behind (dashed).
    pub hidden: Vec<Vec<DVec2>>,
}

impl ViewLines {
    /// Every line, for sizing the view.
    pub fn all(&self) -> impl Iterator<Item = &Vec<DVec2>> {
        self.visible.iter().chain(&self.outline).chain(&self.smooth).chain(&self.hidden)
    }

    /// The box round every line (min, max), if there are any.
    pub fn bounds(&self) -> Option<(DVec2, DVec2)> {
        self.all().flatten().fold(None, |b, &p| Some(b.map_or((p, p), |(lo, hi): (DVec2, DVec2)| (lo.min(p), hi.max(p)))))
    }
}

thread_local! {
    static VIEWS: std::cell::RefCell<Vec<(String, Option<ViewLines>)>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The bodies seen from the side `toward_viewer` points to, `right` along
/// the paper's X (Y is toward_viewer × right). Remembered for the same bodies
/// and direction: it's slow on a detailed part. None if OpenCASCADE fails.
pub fn hlr_view(bodies: &[&Shape], toward_viewer: DVec3, right: DVec3) -> Option<ViewLines> {
    let key = key_of(&format!("{}|{toward_viewer:?}|{right:?}", bodies.iter().map(|s| fingerprint(s)).collect::<Vec<_>>().join(";")));
    if let Some(found) = VIEWS.with(|c| c.borrow().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone())) {
        return found;
    }
    let found = hlr_view_now(bodies, toward_viewer, right);
    VIEWS.with(|c| {
        let mut c = c.borrow_mut();
        c.push((key, found.clone()));
        // Views hold many points: keep fewer than the other memos.
        if c.len() > 24 {
            c.remove(0);
        }
    });
    found
}

fn hlr_view_now(bodies: &[&Shape], toward_viewer: DVec3, right: DVec3) -> Option<ViewLines> {
    if bodies.is_empty() {
        return Some(ViewLines::default());
    }
    let all: Shape = opencascade::primitives::Compound::from_shapes(bodies.iter().copied()).into();
    let lines = catch_unwind(AssertUnwindSafe(|| all.try_hlr(toward_viewer.to_array(), right.to_array(), 0.01))).ok().flatten()?;
    let mut v = ViewLines::default();
    let mut hidden = Vec::new();
    for (kind, pts) in lines {
        let pts: Vec<DVec2> = pts.into_iter().map(DVec2::from).collect();
        match kind {
            0 => v.visible.push(pts),
            1 => v.outline.push(pts),
            2 => v.smooth.push(pts),
            _ => hidden.push(pts),
        }
    }
    v.hidden = drop_covered(hidden, v.visible.iter().chain(&v.outline).chain(&v.smooth));
    Some(v)
}

/// The parts of `lines` not lying on `seen` (a back edge right behind a
/// front one isn't drawn dashed over it).
fn drop_covered<'a>(lines: Vec<Vec<DVec2>>, seen: impl Iterator<Item = &'a Vec<DVec2>>) -> Vec<Vec<DVec2>> {
    let segs: Vec<(DVec2, DVec2)> = seen.flat_map(|l| l.windows(2).map(|w| (w[0], w[1]))).collect();
    let Some((lo, hi)) = segs.iter().flat_map(|&(a, b)| [a, b]).fold(None, |r, p| Some(r.map_or((p, p), |(lo, hi): (DVec2, DVec2)| (lo.min(p), hi.max(p))))) else { return lines };
    let tol = 1e-6 * (1.0 + (hi - lo).length());
    // The seen segments by grid cell, so each test looks at a few.
    let cell = ((hi - lo).max_element() / 64.0).max(1e-3);
    let at = |p: DVec2| (((p.x - lo.x) / cell).floor() as i64, ((p.y - lo.y) / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, &(a, b)) in segs.iter().enumerate() {
        let (c0, c1) = (at(a.min(b) - tol), at(a.max(b) + tol));
        for x in c0.0..=c1.0 {
            for y in c0.1..=c1.1 {
                grid.entry((x, y)).or_default().push(i);
            }
        }
    }
    let on_seen = |p: DVec2| {
        grid.get(&at(p)).is_some_and(|ids| {
            ids.iter().any(|&i| {
                let (a, b) = segs[i];
                let d = b - a;
                let t = if d.length_squared() > 0.0 { ((p - a).dot(d) / d.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                (a + d * t).distance(p) <= tol
            })
        })
    };
    let mut out = Vec::new();
    for line in lines {
        let mut run: Vec<DVec2> = Vec::new();
        for w in line.windows(2) {
            // Covered when both ends and the middle lie on seen lines.
            let covered = on_seen(w[0]) && on_seen(w[1]) && on_seen((w[0] + w[1]) * 0.5);
            if covered {
                if run.len() > 1 {
                    out.push(std::mem::take(&mut run));
                }
                run.clear();
            } else {
                if run.is_empty() {
                    run.push(w[0]);
                }
                run.push(w[1]);
            }
        }
        if run.len() > 1 {
            out.push(run);
        }
    }
    out
}

#[cfg(test)]
mod hlr_tests {
    use super::*;

    fn block(lo: DVec3, hi: DVec3) -> Shape {
        Shape::box_from_corners(kv(lo), kv(hi))
    }

    fn close(a: DVec2, b: DVec2) -> bool {
        a.distance(b) < 1e-6
    }

    #[test]
    fn a_box_from_the_front_and_the_top_is_exactly_its_size() {
        let b = block(DVec3::ZERO, DVec3::new(10.0, 20.0, 30.0));
        let front = hlr_view(&[&b], DVec3::NEG_Y, DVec3::X).unwrap();
        let (lo, hi) = front.bounds().unwrap();
        assert!(close(lo, DVec2::new(0.0, 0.0)) && close(hi, DVec2::new(10.0, 30.0)), "front: {lo} {hi}");
        assert!(front.hidden.is_empty(), "the back edges lie under the front ones: not drawn");
        assert_eq!(front.visible.len(), 4, "four sides of a rectangle");
        let top = hlr_view(&[&b], DVec3::Z, DVec3::X).unwrap();
        let (lo, hi) = top.bounds().unwrap();
        assert!(close(lo, DVec2::ZERO) && close(hi, DVec2::new(10.0, 20.0)), "top, Y up the paper: {lo} {hi}");
    }

    #[test]
    fn a_hole_behind_the_front_face_is_hidden_lines() {
        // A 40 x 20 x 10 block with a 6 mm hole straight down through it at x = 12.
        let b = block(DVec3::ZERO, DVec3::new(40.0, 20.0, 10.0));
        let hole = Shape::cylinder(kv(DVec3::new(12.0, 10.0, -1.0)), 3.0, kv(DVec3::Z), 12.0);
        let part = b.try_subtract(&hole).unwrap();
        let front = hlr_view(&[&part], DVec3::NEG_Y, DVec3::X).unwrap();
        let xs: Vec<f64> = front.hidden.iter().flatten().map(|p| p.x).collect();
        assert!(!xs.is_empty(), "the hole shows as hidden lines");
        let (lo, hi) = xs.iter().fold((f64::MAX, f64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        assert!((lo - 9.0).abs() < 1e-6 && (hi - 15.0).abs() < 1e-6, "its sides at 12 ± 3: {lo} {hi}");
        // From the top it's a visible circle of radius 3 round (12, 10).
        let top = hlr_view(&[&part], DVec3::Z, DVec3::X).unwrap();
        let on_circle = top.visible.iter().flatten().filter(|p| ((**p - DVec2::new(12.0, 10.0)).length() - 3.0).abs() < 1e-6).count();
        assert!(on_circle > 8, "the hole's edge seen from above");
    }

    #[test]
    fn a_cylinder_from_the_side_has_outlines_at_its_radius() {
        let c = Shape::cylinder(kv(DVec3::ZERO), 5.0, kv(DVec3::Z), 20.0);
        let front = hlr_view(&[&c], DVec3::NEG_Y, DVec3::X).unwrap();
        // Its sides: one is an outline, the other its seam edge lying on it.
        let mut sides: Vec<f64> = front.visible.iter().chain(&front.outline).filter(|l| l.iter().all(|p| (p.x - l[0].x).abs() < 1e-6)).map(|l| l[0].x).collect();
        sides.sort_by(f64::total_cmp);
        assert!(sides.len() == 2 && (sides[0] + 5.0).abs() < 1e-6 && (sides[1] - 5.0).abs() < 1e-6, "{sides:?}");
        assert!(front.hidden.is_empty(), "the back half of the end circles lies on the front half: not drawn");
        let (lo, hi) = front.bounds().unwrap();
        assert!(close(lo, DVec2::new(-5.0, 0.0)) && close(hi, DVec2::new(5.0, 20.0)), "{lo} {hi}");
    }
}

/// Where bodies overlap (Fusion's Inspect → Interference): each pair (by
/// index) that shares volume, with the shared solid. Touching isn't overlapping.
pub fn interference(bodies: &[&Shape]) -> Vec<(usize, usize, Shape)> {
    let boxes: Vec<Option<(DVec3, DVec3)>> = bodies.iter().map(|b| exact_bounds(b)).collect();
    let mut out = Vec::new();
    for i in 0..bodies.len() {
        for j in i + 1..bodies.len() {
            // Boxes apart: nothing to check.
            let (Some((alo, ahi)), Some((blo, bhi))) = (boxes[i], boxes[j]) else { continue };
            if alo.cmpgt(bhi).any() || blo.cmpgt(ahi).any() {
                continue;
            }
            let Some(common) = catch_unwind(AssertUnwindSafe(|| bodies[i].try_intersect(bodies[j]))).ok().flatten() else { continue };
            let volume = common.volume();
            let scale = (ahi - alo).max_element().max((bhi - blo).max_element());
            if volume > 1e-9 * scale * scale * scale && !common.solids().is_empty() {
                out.push((i, j, common));
            }
        }
    }
    out
}

pub fn body_properties(shape: &Shape) -> (f64, f64, DVec3) {
    if let Some(p) = PROPERTIES.with(|c| c.borrow().iter().find(|(k, _)| k.is_same(shape)).map(|(_, p)| *p)) {
        return p;
    }
    let (volume, c) = shape.volume_and_centre();
    let p = (volume, shape.surface_area(), DVec3::new(c.x, c.y, c.z));
    PROPERTIES.with(|c| {
        let mut c = c.borrow_mut();
        c.push((shape.clone(), p));
        bounded(&mut c);
    });
    p
}

/// Keep at most MEMO entries (dropping the oldest).
fn bounded<T>(v: &mut Vec<T>) {
    if v.len() > MEMO {
        v.drain(..v.len() - MEMO);
    }
}

/// A shape's fingerprint (the same shape's is remembered).
fn fingerprint(s: &Shape) -> String {
    if let Some(f) = FINGERPRINTS.with(|c| c.borrow().iter().find(|(k, _)| k.is_same(s)).map(|(_, f)| f.clone())) {
        return f;
    }
    let bb = opencascade::bounding_box::aabb(s);
    let f = format!("{:.9e}/{}/{:.6?}/{:.6?}", s.volume(), s.faces().count(), bb.min(), bb.max());
    FINGERPRINTS.with(|c| {
        let mut c = c.borrow_mut();
        c.push((s.clone(), f.clone()));
        bounded(&mut c);
    });
    f
}

/// The bodies going into a feature, as a key.
fn bodies_key(bodies: &Tagged) -> String {
    bodies.iter().map(|(s, id)| format!("{}#{id:?}", fingerprint(s))).collect::<Vec<_>>().join(";")
}

/// A feature's result from before, if its inputs (`key`) are the same.
fn remembered(key: &str) -> Option<FeatureResult> {
    FEATURES.with(|c| c.borrow().iter().find(|(k, _)| k == key).map(|(_, r)| r.clone()))
}

fn remember(key: String, r: &FeatureResult) {
    // Its shapes are known by where they came from, so later features don't
    // have to measure them.
    FINGERPRINTS.with(|c| {
        let mut c = c.borrow_mut();
        for (i, (s, _)) in r.0.iter().enumerate() {
            c.push((s.clone(), format!("from {key}#{i}")));
        }
        bounded(&mut c);
    });
    FEATURES.with(|c| {
        let mut c = c.borrow_mut();
        c.retain(|(k, _)| *k != key);
        c.push((key, r.clone()));
        bounded(&mut c);
    });
}

/// What a hole cuts at each place: the bore, and a modeled thread's groove
/// with the most of it that lies inside the bore (cut by the bore anyway).
type HoleCuts = Vec<(Shape, Option<(Shape, f64)>)>;

/// Per unit length, the part of a hole's thread groove (on a bore of radius
/// `r`) that starts inside the bore so it shares no face with it.
fn thread_strip(r: f64, pitch: f64) -> f64 {
    let ((r_in, r_out), (w_in, w_out)) = thread_profile(r, true, pitch);
    let width = |x: f64| w_in + (w_out - w_in) * (x - r_in) / (r_out - r_in);
    let (wa, wb) = (width(r_in), width(r));
    let area = (wa + wb) / 2.0 * (r - r_in);
    let r_c = r_in + (r - r_in) * (wa + 2.0 * wb) / (3.0 * (wa + wb));
    area * std::f64::consts::TAU * r_c / pitch
}

/// The cutters for a hole and the threads it only shows.
fn hole_tools(
    h: &HoleFeature,
    sketches: &[Sketch],
    frames: &[Option<FacePlane>],
    planes: &[Option<FacePlane>],
    bodies: &[&Shape],
) -> Result<(HoleCuts, Vec<CosmeticThread>), String> {
    if h.places.is_empty() {
        return Err("no place picked".into());
    }
    if h.tap == TapType::TaperTapped && h.kind == crate::model::HoleKind::Counterbore {
        return Err("a taper tapped hole can't have a counterbore".into());
    }
    // How far "All" has to go: past every body.
    let (mut lo, mut hi) = (DVec3::splat(f64::MAX), DVec3::splat(f64::MIN));
    for s in bodies {
        let bb = opencascade::bounding_box::aabb(s);
        lo = lo.min(DVec3::new(bb.min().x, bb.min().y, bb.min().z));
        hi = hi.max(DVec3::new(bb.max().x, bb.max().y, bb.max().z));
    }
    let mut tools = Vec::new();
    let mut threads = Vec::new();
    for ((p, dir, circle), place) in hole_places(h, sketches, frames, bodies)?.into_iter().zip(&h.places) {
        let depth = match h.extent {
            HoleExtent::Distance => h.depth,
            HoleExtent::All => {
                if bodies.is_empty() {
                    return Err("there's nothing to drill through".into());
                }
                // As far as the furthest corner of the bodies along the hole.
                let corners = (0..8).map(|i| DVec3::new([lo.x, hi.x][i & 1], [lo.y, hi.y][(i >> 1) & 1], [lo.z, hi.z][(i >> 2) & 1]));
                corners.map(|c| (c - p).dot(dir.normalize())).fold(0.0, f64::max) + 1.0
            }
            HoleExtent::To => {
                let t = h.target.as_ref().ok_or("pick what it runs to")?;
                let (q, n) = target_plane(t, planes, bodies)?;
                let slant = dir.dot(n);
                if slant.abs() < 1e-6 {
                    return Err("the hole runs alongside that plane and never reaches it".into());
                }
                let d = (q - p).dot(n) / slant;
                if d <= 1e-6 {
                    return Err("that plane is behind the hole — flip it".into());
                }
                d
            }
        };
        // NPT: the stated size at the hole's small (far) end, wider toward the face.
        let bore = h.bore(circle)? + if h.tap == TapType::TaperTapped { depth * crate::threads::NPT_TAPER } else { 0.0 };
        // Start the cutter a little above the face where that's surely air
        // (a body face drilled inward), so it shares no face with the part;
        // a sketch plane can be anywhere, so its hole starts right on it.
        let in_air = matches!(place, HolePlace::Face { .. }) && !h.flip;
        let lead = if in_air { bore.max(1.0) } else { 0.0 };
        let pts = h.profile(bore, depth, lead)?;
        let radial = crate::model::zero_normal(dir, DVec3::Z);
        let frame = Frame { origin: p, u: radial, v: dir.normalize() };
        let wire = loop_wire(&frame, &pts, &vec![Curve::Line; pts.len()])?;
        let tool = Face::from_wire(&wire)
            .try_revolve(kv(p), kv(dir.normalize()), std::f64::consts::TAU)
            .ok_or("the kernel couldn't make the hole's shape")?;
        let mut groove = None;
        if matches!(h.tap, TapType::Tapped | TapType::TaperTapped) {
            let t = h.thread.thread().ok_or_else(|| format!("unknown thread {}", h.thread.designation))?;
            // As in Fusion, only Tapped threads can be modeled (taper tapped are shown).
            if h.thread.modeled && h.tap == TapType::Tapped {
                // Run in from above the face (air) so the entry is cut cleanly.
                let start = if in_air { -1.5 * t.pitch } else { 0.0 };
                // A blind thread stops half a pitch short of the bottom (clear of
                // where the bore meets its tip); "All" runs on out of the part.
                let end = match (h.thread_full, h.extent) {
                    (true, HoleExtent::All) => depth,
                    (true, _) => depth - 0.5 * t.pitch,
                    (false, _) => h.thread_depth(depth).min(depth - 0.5 * t.pitch),
                };
                if end - start < t.pitch {
                    return Err("the hole is too shallow for a modeled thread".into());
                }
                // What it may share with the bore: the strip along its length, and
                // all of it where it runs through air the hole's head (counterbore
                // or countersink) or the run-in above the face takes out anyway.
                let head = match h.kind {
                    crate::model::HoleKind::Simple => 0.0,
                    crate::model::HoleKind::Counterbore => h.cbore_depth,
                    crate::model::HoleKind::Countersink => (h.csink_diameter / 2.0 - bore / 2.0) / (h.csink_angle / 2.0).to_radians().tan(),
                };
                let whole = thread_removal_per_length(bore / 2.0, true, t.pitch) + thread_strip(bore / 2.0, t.pitch);
                let shared = thread_strip(bore / 2.0, t.pitch) * (end - start) + whole * (head.max(0.0) - start);
                groove = Some((thread_groove(&t, p, dir, start, end, h.thread.right_hand)?, shared));
            } else {
                threads.push(CosmeticThread { origin: p, axis: dir.normalize(), radius: bore / 2.0 * 0.99, pitch: t.pitch, length: h.thread_depth(depth) });
            }
        }
        tools.push((tool, groove));
    }
    Ok((tools, threads))
}

/// Is a polyline a straight run (all points on the line through its ends)?
pub fn is_straight(pts: &[DVec3]) -> bool {
    let (Some(&a), Some(&b)) = (pts.first(), pts.last()) else { return false };
    let d = b - a;
    d.length() > 1e-9 && pts.iter().all(|p| (*p - a).cross(d).length() <= 1e-7 * d.length().max(1.0))
}

/// Evaluate the timeline. A feature that fails is skipped (with its error
/// recorded) and the rest still build — like a red feature in Fusion.
#[cfg(test)]
pub fn build(
    sketches: &[Sketch],
    extrudes: &[Extrude],
    revolves: &[Revolve],
    edge_features: &[EdgeFeature],
    timeline: &[Feature],
) -> Model {
    build_with(sketches, extrudes, revolves, edge_features, &[], &[], &[], timeline)
}

/// Copy the picked bodies once per placement (mirror or pattern). The copies
/// are new bodies; the originals stay.
fn apply_copy(bodies: Tagged, cf: &CopyFeature, k: usize, axes: &[Option<(DVec3, DVec3)>], planes: &[Option<FacePlane>]) -> Result<(Tagged, Option<BodyId>), String> {
    let sources: Vec<usize> = (0..bodies.len()).filter(|&i| cf.bodies.contains(&bodies[i].1)).collect();
    if sources.is_empty() {
        return Err("the bodies it copies are gone — edit it and pick again".into());
    }
    let placements = cf.kind.placements(axes, planes)?;
    if placements.is_empty() {
        return Err("it makes no copies — raise the quantity".into());
    }
    let mut made: Tagged = Vec::new();
    let mut piece = 0;
    for p in placements {
        for &i in &sources {
            let src = &bodies[i].0;
            let copy = match p {
                Placement::Translate(t) => src.translated(kv(t)),
                Placement::Rotate { origin, axis, radians, radial } => {
                    // Pushed out from the axis first (along the way the body lies from it), then turned.
                    let out = if radial != 0.0 { radial_shift(src, origin, axis, radial) } else { DVec3::ZERO };
                    let moved = if out == DVec3::ZERO { None } else { Some(src.translated(kv(out))) };
                    moved.as_ref().unwrap_or(src).rotated_about(kv(origin), kv(axis), radians)
                }
                Placement::Reflect { origin, normal } => src.mirrored_in_plane(kv(origin), kv(normal)),
            };
            let id = BodyId { feature: Feature::Copy(k), piece };
            piece += 1;
            // A reflection can leave a solid inside-out; the volume tells.
            let copy = if copy.volume() < 0.0 { return Err("the kernel couldn't mirror this body".into()) } else { copy };
            made.extend(copy.solids().into_iter().map(|s| (s, id)));
        }
    }
    let first = made.first().map(|(_, id)| *id);
    Ok((uniquify(bodies.into_iter().chain(made).collect()), first))
}

#[cfg(test)]
pub fn build_with(
    sketches: &[Sketch],
    extrudes: &[Extrude],
    revolves: &[Revolve],
    edge_features: &[EdgeFeature],
    copies: &[CopyFeature],
    axis_features: &[AxisFeature],
    plane_features: &[PlaneFeature],
    timeline: &[Feature],
) -> Model {
    build_all(sketches, extrudes, revolves, edge_features, copies, axis_features, plane_features, &[], &[], &[], &[], timeline)
}

/// `build_with`, plus Remove features.
/// What a build reads (its own copy, so it can run over several frames).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildInputs {
    pub sketches: Vec<Sketch>,
    pub extrudes: Vec<Extrude>,
    pub revolves: Vec<Revolve>,
    pub edge_features: Vec<EdgeFeature>,
    pub copies: Vec<CopyFeature>,
    pub axis_features: Vec<AxisFeature>,
    pub plane_features: Vec<PlaneFeature>,
    pub removes: Vec<RemoveFeature>,
    pub holes: Vec<HoleFeature>,
    pub thread_features: Vec<ThreadFeature>,
    pub shells: Vec<ShellFeature>,
    pub offset_faces: Vec<OffsetFaceFeature>,
    pub combines: Vec<CombineFeature>,
    pub splits: Vec<SplitFeature>,
    pub scales: Vec<ScaleFeature>,
    pub moves: Vec<MoveFeature>,
    pub points: Vec<ConstructionPoint>,
    pub sweeps: Vec<SweepFeature>,
    pub imports: Vec<ImportFeature>,
    pub joints: Vec<JointFeature>,
    /// Which bodies go together (a joint moves a whole component).
    pub components: Vec<crate::assembly::Component>,
    pub timeline: Vec<Feature>,
}

/// Sketch `sk`'s linked projections against the bodies built so far
/// (`frame`: where a face or construction-plane sketch is in this build).
fn relink(sk: &Sketch, frame: Option<FacePlane>, bodies: &Tagged, ext: &[Option<End>], rev: &[Option<End>]) -> (Option<Sketch>, Vec<usize>, Option<String>) {
    if sk.projections.is_empty() {
        return (None, Vec::new(), None);
    }
    let Ok(plane) = (if sk.plane_ref.is_some() { frame.map(Frame::from_face).ok_or(()) } else { frame_for(sk, frame).map_err(|_| ()) }) else {
        // The sketch's face or plane is gone (its own error says so): none of its edges can be found either.
        return (None, sk.projections.iter().map(|&(e, _)| e).collect(), None);
    };
    let found = catch_unwind(AssertUnwindSafe(|| {
        let mut all: Vec<(usize, Edge, EdgeRef)> = Vec::new();
        let mut pts: Vec<Vec<DVec3>> = Vec::new();
        for (i, (b, _)) in bodies.iter().enumerate() {
            for (e, r, p) in shape_edges(b) {
                all.push((i, e, r));
                pts.push(p);
            }
        }
        sk.projections
            .iter()
            .map(|(_, r)| {
                let moved = r.follow.and_then(|fl| motion_for(fl, ext, rev)).map(|m| move_edge(r, m));
                let r = moved.as_ref().unwrap_or(r);
                let hit = find_edge(&all, r, f64::INFINITY)?;
                let idx = all.iter().position(|c| std::ptr::eq(c, hit))?;
                Some(pts[idx].clone())
            })
            .collect::<Vec<_>>()
    }))
    .unwrap_or_default();
    let find = |r: &EdgeRef| sk.projections.iter().position(|(_, q)| q == r).and_then(|i| found.get(i).cloned().flatten());
    crate::project::follow_links(sk, plane.origin, plane.u, plane.v, find)
}

/// A build of the timeline that can be run a little at a time (so the window
/// keeps drawing — and shows how far it's got — while a slow part is built).
pub struct Build {
    inp: BuildInputs,
    sweep_errors: Vec<Option<String>>,
    import_errors: Vec<Option<String>>,
    joint_errors: Vec<Option<String>>,
    point_pos: Vec<Option<DVec3>>,
    point_errors: Vec<Option<String>>,
    /// The next timeline entry to build.
    next: usize,
    hole_errors: Vec<Option<String>>,
    thread_errors: Vec<Option<String>>,
    shell_errors: Vec<Option<String>>,
    offset_face_errors: Vec<Option<String>>,
    combine_errors: Vec<Option<String>>,
    split_errors: Vec<Option<String>>,
    scale_errors: Vec<Option<String>>,
    move_errors: Vec<Option<String>>,
    move_pivots: Vec<Option<DVec3>>,
    threads: Vec<CosmeticThread>,
    remove_errors: Vec<Option<String>>,
    planes: Vec<Option<FacePlane>>,
    plane_errors: Vec<Option<String>>,
    axes: Vec<Option<(DVec3, DVec3)>>,
    axis_errors: Vec<Option<String>>,
    copy_errors: Vec<Option<String>>,
    shapes: Tagged,
    errors: Vec<Option<String>>,
    edge_errors: Vec<Option<String>>,
    revolve_errors: Vec<Option<String>>,
    extrude_ends: Vec<Option<End>>,
    revolve_ends: Vec<Option<End>>,
    frames: Vec<Option<FacePlane>>,
    sketch_errors: Vec<Option<String>>,
    /// The sketches as built: linked projections moved with the part.
    live: Vec<Sketch>,
    linked: Vec<Option<Sketch>>,
    lost: Vec<Vec<usize>>,
    touched_by: HashMap<Feature, BodyId>,
    timing: bool,
    started: std::time::Instant,
    step: std::time::Instant,
}

impl Build {
    pub fn start(inp: BuildInputs) -> Build {
        opencascade::catch_crashes();
        let BuildInputs { sketches, extrudes, revolves, edge_features, copies, axis_features, plane_features, removes, holes, thread_features, shells, offset_faces, combines, splits, scales, moves, points, sweeps, imports, joints, components: _, timeline: _ } = &inp;
        let sweep_errors: Vec<Option<String>> = vec![None; sweeps.len()];
        let import_errors: Vec<Option<String>> = vec![None; imports.len()];
        let joint_errors: Vec<Option<String>> = vec![None; joints.len()];
        let (point_pos, point_errors): (Vec<Option<DVec3>>, Vec<Option<String>>) = (vec![None; points.len()], vec![None; points.len()]);
        let _ = (extrudes, revolves, edge_features, copies, axis_features, plane_features, removes, holes, thread_features, sketches, shells);
        let offset_face_errors: Vec<Option<String>> = vec![None; offset_faces.len()];
        let combine_errors: Vec<Option<String>> = vec![None; combines.len()];
        let split_errors: Vec<Option<String>> = vec![None; splits.len()];
        let scale_errors: Vec<Option<String>> = vec![None; scales.len()];
        let (move_errors, move_pivots): (Vec<Option<String>>, Vec<Option<DVec3>>) = (vec![None; moves.len()], vec![None; moves.len()]);
        let hole_errors: Vec<Option<String>> = vec![None; holes.len()];
        let thread_errors: Vec<Option<String>> = vec![None; thread_features.len()];
        let shell_errors: Vec<Option<String>> = vec![None; shells.len()];
        let threads: Vec<CosmeticThread> = Vec::new();
        let remove_errors: Vec<Option<String>> = vec![None; removes.len()];
        let planes: Vec<Option<FacePlane>> = vec![None; plane_features.len()];
        let plane_errors: Vec<Option<String>> = vec![None; plane_features.len()];
        let axes: Vec<Option<(DVec3, DVec3)>> = vec![None; axis_features.len()];
        let axis_errors: Vec<Option<String>> = vec![None; axis_features.len()];
        let copy_errors: Vec<Option<String>> = vec![None; copies.len()];
        let shapes: Tagged = Vec::new();
        let errors: Vec<Option<String>> = vec![None; extrudes.len()];
        let edge_errors: Vec<Option<String>> = vec![None; edge_features.len()];
        let revolve_errors: Vec<Option<String>> = vec![None; revolves.len()];
        let extrude_ends: Vec<Option<End>> = vec![None; extrudes.len()];
        let revolve_ends: Vec<Option<End>> = vec![None; revolves.len()];
        let frames: Vec<Option<FacePlane>> = sketches.iter().map(|s| s.face).collect();
        let sketch_errors: Vec<Option<String>> = vec![None; sketches.len()];
        let touched_by: HashMap<Feature, BodyId> = HashMap::new();
        let (live, linked, lost) = (sketches.clone(), vec![None; sketches.len()], vec![Vec::new(); sketches.len()]);
        // OPENCAD_TIMING=1: how long each feature takes (to find what's slow).
        let timing = std::env::var_os("OPENCAD_TIMING").is_some();
        let started = std::time::Instant::now();
        let step = std::time::Instant::now();
        Build { next: 0, sweep_errors, import_errors, joint_errors, point_pos, point_errors, hole_errors, thread_errors, shell_errors, offset_face_errors, combine_errors, split_errors, scale_errors, move_errors, move_pivots, threads, remove_errors, planes, plane_errors, axes, axis_errors, copy_errors, shapes, errors, edge_errors, revolve_errors, extrude_ends, revolve_ends, frames, sketch_errors, live, linked, lost, touched_by, timing, started, step, inp }
    }

    /// How far it's got: (entries built, all of them, the one being built next).
    pub fn progress(&self) -> (usize, usize, Option<Feature>) {
        (self.next, self.inp.timeline.len(), self.inp.timeline.get(self.next).copied())
    }

    /// The inputs it's building.
    pub fn inputs(&self) -> &BuildInputs {
        &self.inp
    }

    /// Build timeline entries until `budget` is used up (at least one).
    /// False once every entry is built (then `finish`).
    pub fn step(&mut self, budget: std::time::Duration) -> bool {
        let began = std::time::Instant::now();
        let first = self.next;
        let Build { inp, next, sweep_errors, import_errors, joint_errors, point_pos, point_errors, hole_errors, thread_errors, shell_errors, offset_face_errors, combine_errors, split_errors, scale_errors, move_errors, move_pivots, threads, remove_errors, planes, plane_errors, axes, axis_errors, copy_errors, shapes, errors, edge_errors, revolve_errors, extrude_ends, revolve_ends, frames, sketch_errors, live, linked, lost, touched_by, timing, started, step } = self;
        let BuildInputs { sketches: _, extrudes, revolves, edge_features, copies, axis_features, plane_features, removes, holes, thread_features, shells, offset_faces, combines, splits, scales, moves, points, sweeps, imports, joints, components, timeline } = &*inp;
        let _ = (&timing, &started);
        loop {
            if *next >= timeline.len() {
                return false;
            }
            if *next > first && began.elapsed() >= budget {
                return true;
            }
            let f = &timeline[*next];
            *next += 1;
            let sketches: &[Sketch] = live;
            if *timing {
                eprintln!("  {:>8.1} ms  before {f:?}", step.elapsed().as_secs_f64() * 1e3);
                *step = std::time::Instant::now();
            }
            let current = std::mem::take(shapes);
            if let Feature::Move(k) = *f {
                let picked: Vec<&Shape> = current.iter().filter(|(_, id)| moves.get(k).is_some_and(|m| m.bodies.contains(id))).map(|(s, _)| s).collect();
                move_pivots[k] = box_middle(&picked);
            }
            // Keep a copy so a failed operation leaves the previous bodies intact.
            // Extrudes, revolves, fillets and patterns: the same inputs as last
            // time give the same result (holes and threads do this themselves).
            let memo = match *f {
                Feature::Extrude(k) => extrudes.get(k).map(|ex| format!("{f:?}|{ex:?}|{}|{:?}|{planes:?}", sketch_geometry(sketches.get(ex.sketch)), frames.get(ex.sketch))),
                Feature::Revolve(k) => revolves.get(k).map(|rv| format!("{f:?}|{rv:?}|{}|{:?}|{axes:?}", sketch_geometry(sketches.get(rv.sketch)), frames.get(rv.sketch))),
                Feature::Edge(k) => edge_features.get(k).map(|ef| format!("{f:?}|{ef:?}|{extrude_ends:?}|{revolve_ends:?}")),
                Feature::Copy(k) => copies.get(k).map(|c| format!("{f:?}|{c:?}|{axes:?}|{planes:?}")),
                Feature::Shell(k) => shells.get(k).map(|sf| format!("{f:?}|{sf:?}")),
                Feature::OffsetFace(k) => offset_faces.get(k).map(|of| format!("{f:?}|{of:?}")),
                Feature::Combine(k) => combines.get(k).map(|c| format!("{f:?}|{c:?}")),
                Feature::Split(k) => splits.get(k).map(|c| format!("{f:?}|{c:?}")),
                Feature::Scale(k) => scales.get(k).map(|c| format!("{f:?}|{c:?}")),
                Feature::Move(k) => moves.get(k).map(|m| format!("{f:?}|{m:?}|{axes:?}")),
                _ => None,
            }
            .map(|inputs| key_of(&format!("{}|{inputs}", bodies_key(&current))));
            if let Some((next, touched, _, end)) = memo.as_deref().and_then(remembered) {
                match *f {
                    Feature::Extrude(k) => extrude_ends[k] = end,
                    Feature::Revolve(k) => revolve_ends[k] = end,
                    _ => {}
                }
                if let Some(id) = touched {
                    touched_by.insert(*f, id);
                }
                *shapes = next;
                continue;
            }
            let backup: Tagged = current.iter().flat_map(|(s, id)| s.solids().into_iter().map(move |x| (x, *id))).collect();
            let (result, slot) = match *f {
                Feature::Point(k) => {
                    if let Some(p) = points.get(k) {
                        let bodies: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
                        match catch_unwind(AssertUnwindSafe(|| place_point(&bodies, p))).unwrap_or_else(|_| Err("the geometry kernel couldn't place it".into())) {
                            Ok(q) => point_pos[k] = Some(q),
                            Err(e) => point_errors[k] = Some(e),
                        }
                    }
                    *shapes = current;
                    continue;
                }
                Feature::Plane(k) => {
                    if let Some((kind, offset)) = plane_features.get(k).and_then(|pf| pf.kind.map(|kd| (kd, pf.offset))) {
                        let bodies: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
                        let made = catch_unwind(AssertUnwindSafe(|| construct_plane(&kind, &bodies, &planes, point_pos))).unwrap_or_else(|_| Err("the geometry kernel couldn't place it".into()));
                        match made {
                            Ok(f) => planes[k] = Some(FacePlane { origin: f.origin + f.normal() * offset, ..f }),
                            Err(e) => plane_errors[k] = Some(e),
                        }
                    } else if let Some(a) = plane_features.get(k).and_then(|pf| pf.angle) {
                        let made = catch_unwind(AssertUnwindSafe(|| hinge_line(&a, &current, &axes, axis_features, sketches, &frames)))
                            .unwrap_or_else(|_| Err("the geometry kernel couldn't find its line".into()))
                            .and_then(|(o, d, n0)| crate::model::angle_frame(o, d, n0, a.degrees).ok_or_else(|| "its line has no length".into()));
                        match made {
                            Ok(f) => planes[k] = Some(f),
                            Err(e) => plane_errors[k] = Some(e),
                        }
                    } else if let Some(pf) = plane_features.get(k) {
                        let now = if pf.on_origin {
                            Some(pf.base)
                        } else {
                            let bodies: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
                            catch_unwind(AssertUnwindSafe(|| resolve_face(&bodies, &pf.base, DVec2::from(pf.anchor)))).ok().flatten()
                        };
                        match now {
                            Some(f) => planes[k] = Some(FacePlane { origin: f.origin + f.normal() * pf.offset, follow: None, ..f }),
                            None => plane_errors[k] = Some("the face it was measured from is gone".into()),
                        }
                    }
                    *shapes = current;
                    continue;
                }
                Feature::Sketch(k) if sketches.get(k).is_some_and(|s| s.plane_ref.is_some()) => {
                    // A sketch on a construction plane goes wherever the plane is now.
                    let c = sketches[k].plane_ref.unwrap_or(0);
                    match planes.get(c).copied().flatten() {
                        Some(f) => frames[k] = Some(f),
                        None => {
                            frames[k] = None;
                            sketch_errors[k] = Some("the plane it is on couldn't be built".into());
                        }
                    }
                    let (moved, gone, err) = relink(&sketches[k], frames[k], &current, extrude_ends, revolve_ends);
                    lost[k] = gone;
                    if let Some(e) = err {
                        sketch_errors[k].get_or_insert(e);
                    }
                    if let Some(m) = moved {
                        live[k] = m.clone();
                        linked[k] = Some(m);
                    }
                    *shapes = current;
                    continue;
                }
                Feature::Sketch(k) => {
                    // A sketch on a face follows that face as the part changes.
                    if let Some(sk) = sketches.get(k).filter(|s| s.face.is_some()) {
                        let mut face = sk.face.expect("checked above");
                        // On the far end of an extrude/revolve: move with it first.
                        if let Some(m) = face.follow.and_then(|fl| motion_for(fl, &extrude_ends, &revolve_ends)) {
                            face = move_face(&face, m);
                        }
                        let anchor = sketch_anchor(sk);
                        match catch_unwind(AssertUnwindSafe(|| resolve_face(&current.iter().map(|(s, _)| s).collect::<Vec<_>>(), &face, anchor))) {
                            Ok(Some(now)) => frames[k] = Some(now),
                            _ => {
                                // Lost: features built from it fail rather than
                                // build in the wrong place.
                                frames[k] = None;
                                sketch_errors[k] = Some("the face it was on is gone".into());
                            }
                        }
                    }
                    if let Some(sk) = sketches.get(k) {
                        let (moved, gone, err) = relink(sk, frames[k], &current, extrude_ends, revolve_ends);
                        lost[k] = gone;
                        if let Some(e) = err {
                            sketch_errors[k].get_or_insert(e);
                        }
                        if let Some(m) = moved {
                            live[k] = m.clone();
                            linked[k] = Some(m);
                        }
                    }
                    *shapes = current;
                    continue;
                }
                Feature::Sweep(k) => {
                    let Some(sf) = sweeps.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let planes_now = &planes;
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        let bodies: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
                        let shape = sweep_tool(sketches, &frames, sf, planes_now, &bodies)?;
                        apply(current, shape, sf.op, Feature::Sweep(k))
                    }));
                    (r, &mut sweep_errors[k])
                }
                Feature::Joint(k) => {
                    let Some(j) = joints.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let r = catch_unwind(AssertUnwindSafe(|| apply_joint(current, j, components)));
                    (r, &mut joint_errors[k])
                }
                Feature::Import(k) => {
                    let Some(im) = imports.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        let shape = imported(&im.brep)?;
                        apply(current, placed(&shape, im.angles, im.offset), Operation::NewBody, Feature::Import(k))
                    }));
                    (r, &mut import_errors[k])
                }
                Feature::Extrude(k) => {
                    let Some(ex) = extrudes.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let ends = &mut *extrude_ends;
                    let planes_now = &planes;
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        let target = match &ex.target {
                            Some(t) if ex.extent == crate::model::Extent::ToObject => {
                                let bodies: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
                                Some(target_plane(t, planes_now, &bodies)?)
                            }
                            _ => None,
                        };
                        let (shape, end) = tool(sketches, &frames, ex, target)?;
                        ends[k] = Some(end);
                        apply(current, shape, ex.op, Feature::Extrude(k))
                    }));
                    (r, &mut errors[k])
                }
                Feature::Revolve(k) => {
                    let Some(rv) = revolves.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let ends = &mut *revolve_ends;
                    let r = catch_unwind(AssertUnwindSafe(|| {
                        let (shape, end) = revolve_tool(sketches, &frames, rv, &axes)?;
                        ends[k] = Some(end);
                        apply(current, shape, rv.op, Feature::Revolve(k))
                    }));
                    (r, &mut revolve_errors[k])
                }
                Feature::Axis(k) => {
                    // An axis changes no body: find its edge in what's built so far.
                    if let Some(af) = axis_features.get(k) {
                        match catch_unwind(AssertUnwindSafe(|| resolve_axis(&current, af, point_pos))) {
                            Ok(Ok(a)) => axes[k] = Some(a),
                            Ok(Err(e)) => axis_errors[k] = Some(e),
                            Err(_) => axis_errors[k] = Some("the geometry kernel couldn't find its edge".into()),
                        }
                    }
                    *shapes = current;
                    continue;
                }
                Feature::Hole(k) => {
                    let Some(h) = holes.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let (fr, pl) = (&frames, &planes);
                    // Same bodies in, same hole, same places and planes: the same result.
                    let places = {
                        let bodies: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
                        format!("{:?}", catch_unwind(AssertUnwindSafe(|| hole_places(h, sketches, fr, &bodies))).ok())
                    };
                    let key = format!("hole{k}|{}|{h:?}|{places}|{pl:?}", bodies_key(&current));
                    if let Some((next, touched, shown, _)) = remembered(&key) {
                        if let Some(id) = touched {
                            touched_by.insert(*f, id);
                        }
                        threads.extend(shown);
                        *shapes = next;
                        continue;
                    }
                    let mut shown = Vec::new();
                    let r = catch_unwind(AssertUnwindSafe(|| -> Result<(Tagged, Option<BodyId>), String> {
                        let bodies: Vec<&Shape> = current.iter().map(|(s, _)| s).collect();
                        let (tools, th) = hole_tools(h, sketches, fr, pl, &bodies)?;
                        shown = th;
                        drop(bodies);
                        let volume = |t: &Tagged| t.iter().map(|(s, _)| s.volume()).sum::<f64>();
                        let mut acc = current;
                        let mut touched = None;
                        for (bore, groove) in tools {
                            let Some((groove, strip)) = groove else {
                                let (next, t) = apply(acc, bore, Operation::Cut, Feature::Hole(k))?;
                                (acc, touched) = (next, touched.or(t));
                                continue;
                            };
                            // A modeled thread: what the bore and groove should take out
                            // together, measured on the part as it is (OCCT can fail quietly
                            // beside a helix; better an error than a wrong part).
                            let overlap = |a: &Shape| acc.iter().filter_map(|(s, _)| s.try_intersect(a)).map(|c| c.volume()).sum::<f64>();
                            let (ob, og) = (overlap(&bore), overlap(&groove));
                            let before = volume(&acc);
                            let (next, t2) = apply(acc, bore, Operation::Cut, Feature::Hole(k))?;
                            let (next, t1) = apply(next, groove, Operation::Cut, Feature::Hole(k))?;
                            let removed = before - volume(&next);
                            // Bore and groove together, less the strip they share (at most).
                            if removed < (ob + og - strip) * 0.98 || removed > (ob + og) * 1.02 {
                                return Err("the kernel couldn't cut this thread — try it cosmetic (Modeled off)".into());
                            }
                            (acc, touched) = (next, touched.or(t1).or(t2));
                        }
                        Ok((acc, touched))
                    }));
                    if let Ok(Ok((next, touched))) = &r {
                        remember(key, &(next.clone(), *touched, shown.clone(), None));
                        threads.extend(shown);
                    }
                    (r, &mut hole_errors[k])
                }
                Feature::Thread(k) => {
                    let Some(tf) = thread_features.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let key = format!("thread{k}|{}|{tf:?}", bodies_key(&current));
                    if let Some((next, touched, shown, _)) = remembered(&key) {
                        if let Some(id) = touched {
                            touched_by.insert(*f, id);
                        }
                        threads.extend(shown);
                        *shapes = next;
                        continue;
                    }
                    let mut shown = Vec::new();
                    let r = catch_unwind(AssertUnwindSafe(|| apply_threads(current, tf, Feature::Thread(k), &mut shown)));
                    if let Ok(Ok((next, touched))) = &r {
                        remember(key, &(next.clone(), *touched, shown.clone(), None));
                        threads.extend(shown);
                    }
                    (r, &mut thread_errors[k])
                }
                Feature::Shell(k) => {
                    let Some(sf) = shells.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let r = catch_unwind(AssertUnwindSafe(|| apply_shell(current, sf)));
                    (r, &mut shell_errors[k])
                }
                Feature::OffsetFace(k) => {
                    let Some(of) = offset_faces.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let r = catch_unwind(AssertUnwindSafe(|| apply_offset_faces(current, of)));
                    (r, &mut offset_face_errors[k])
                }
                Feature::Move(k) => {
                    let (Some(mf), Some(pivot)) = (moves.get(k), move_pivots[k]) else {
                        if moves.get(k).is_some() {
                            move_errors[k] = Some("the bodies it moves are gone — edit it and pick again".into());
                        }
                        *shapes = current;
                        continue;
                    };
                    let axes_now = &axes;
                    let r = catch_unwind(AssertUnwindSafe(|| apply_move(current, mf, k, axes_now, pivot)));
                    (r, &mut move_errors[k])
                }
                Feature::Combine(k) => {
                    let Some(c) = combines.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let r = catch_unwind(AssertUnwindSafe(|| apply_combine(current, c)));
                    (r, &mut combine_errors[k])
                }
                Feature::Split(k) => {
                    let Some(sp) = splits.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let planes_now = &planes;
                    let r = catch_unwind(AssertUnwindSafe(|| apply_split(current, sp, k, planes_now)));
                    (r, &mut split_errors[k])
                }
                Feature::Scale(k) => {
                    let Some(sc) = scales.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let r = catch_unwind(AssertUnwindSafe(|| apply_scale(current, sc)));
                    (r, &mut scale_errors[k])
                }
                Feature::Remove(k) => {
                    // Take the bodies out; the rest carry on.
                    if let Some(rf) = removes.get(k) {
                        let before = current.len();
                        let kept: Tagged = current.into_iter().filter(|(_, id)| !rf.bodies.contains(id)).collect();
                        if kept.len() == before {
                            remove_errors[k] = Some("the body it removes is gone".into());
                        }
                        *shapes = kept;
                    } else {
                        *shapes = current;
                    }
                    continue;
                }
                Feature::Copy(k) => {
                    let Some(cf) = copies.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let (axes_now, planes_now) = (&axes, &planes);
                    let r = catch_unwind(AssertUnwindSafe(|| apply_copy(current, cf, k, axes_now, planes_now)));
                    (r, &mut copy_errors[k])
                }
                Feature::Edge(k) => {
                    let Some(ef) = edge_features.get(k) else {
                        *shapes = current;
                        continue;
                    };
                    let (ext, rev) = (&extrude_ends, &revolve_ends);
                    let r = catch_unwind(AssertUnwindSafe(|| apply_edges(current, ef, ext, rev)));
                    (r, &mut edge_errors[k])
                }
            };
            *shapes = match result {
                Ok(Ok((next, touched))) => {
                    if let Some(id) = touched {
                        touched_by.insert(*f, id);
                    }
                    if let Some(key) = memo {
                        let end = match *f {
                            Feature::Extrude(k) => extrude_ends[k],
                            Feature::Revolve(k) => revolve_ends[k],
                            _ => None,
                        };
                        remember(key, &(next.clone(), touched, Vec::new(), end));
                    }
                    next
                }
                Ok(Err(e)) => {
                    *slot = Some(e);
                    backup
                }
                Err(_) => {
                    *slot = Some("the geometry kernel couldn't build this feature".into());
                    backup
                }
            };
        }
    }

    /// The model built (bodies analysed for picking and Measure).
    pub fn finish(self) -> Model {
        let Build { inp: _, next: _, sweep_errors, import_errors, joint_errors, point_pos, point_errors, hole_errors, thread_errors, shell_errors, offset_face_errors, combine_errors, split_errors, scale_errors, move_errors, move_pivots, threads, remove_errors, planes, plane_errors, axes, axis_errors, copy_errors, shapes, errors, edge_errors, revolve_errors, extrude_ends, revolve_ends, frames, sketch_errors, live: _, linked, lost, touched_by, timing, started, mut step } = self;
    if timing {
        eprintln!("  {:>8.1} ms  last feature; analysing bodies…", step.elapsed().as_secs_f64() * 1e3);
        step = std::time::Instant::now();
    }
    let bodies = shapes
        .into_iter()
        .filter_map(|(shape, id)| {
            // The very same shape as last time (a remembered hole or thread): its analysis too.
            if let Some((mesh, tri_face, edges, flat_faces, faces)) = BODIES.with(|c| c.borrow().iter().find(|(s, _)| s.is_same(&shape)).map(|(_, d)| d.clone())) {
                return Some(Body { id, shape, mesh, edges, flat_faces, faces, tri_face });
            }
            let (mesh, tri_face) = catch_unwind(AssertUnwindSafe(|| mesh_tagged(&shape))).ok()?.ok()?;
            let edges = catch_unwind(AssertUnwindSafe(|| pickable_edges(&shape, &mesh))).unwrap_or_default();
            let flat_faces = catch_unwind(AssertUnwindSafe(|| flat_faces(&shape))).unwrap_or_default();
            let faces = catch_unwind(AssertUnwindSafe(|| face_geoms(&shape))).unwrap_or_default();
            BODIES.with(|c| {
                let mut c = c.borrow_mut();
                c.push((shape.clone(), (mesh.clone(), tri_face.clone(), edges.clone(), flat_faces.clone(), faces.clone())));
                bounded(&mut c);
            });
            Some(Body { id, shape, mesh, edges, flat_faces, faces, tri_face })
        })
        .collect();
    if timing {
        eprintln!("  {:>8.1} ms  bodies; build {:.1} ms in all", step.elapsed().as_secs_f64() * 1e3, started.elapsed().as_secs_f64() * 1e3);
    }
    let caps = caps_of(&extrude_ends, &revolve_ends);
    Model { bodies, sketch_frames: frames, sketch_errors, errors, edge_errors, revolve_errors, copy_errors, remove_errors, axes, axis_errors, planes, plane_errors, hole_errors, thread_errors, shell_errors, offset_face_errors, combine_errors, split_errors, scale_errors, move_errors, move_pivots, threads, caps, touched: touched_by, linked_sketches: linked, projection_lost: lost, points: point_pos, point_errors, sweep_errors, import_errors, joint_errors }
    }
}

/// Evaluate the timeline. A feature that fails is skipped (with its error
/// recorded) and the rest still build — like a red feature in Fusion.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn build_all(
    sketches: &[Sketch],
    extrudes: &[Extrude],
    revolves: &[Revolve],
    edge_features: &[EdgeFeature],
    copies: &[CopyFeature],
    axis_features: &[AxisFeature],
    plane_features: &[PlaneFeature],
    removes: &[RemoveFeature],
    holes: &[HoleFeature],
    thread_features: &[ThreadFeature],
    shells: &[ShellFeature],
    timeline: &[Feature],
) -> Model {
    let mut b = Build::start(BuildInputs {
        sketches: sketches.to_vec(),
        extrudes: extrudes.to_vec(),
        revolves: revolves.to_vec(),
        edge_features: edge_features.to_vec(),
        copies: copies.to_vec(),
        axis_features: axis_features.to_vec(),
        plane_features: plane_features.to_vec(),
        removes: removes.to_vec(),
        holes: holes.to_vec(),
        thread_features: thread_features.to_vec(),
        shells: shells.to_vec(),
        offset_faces: Vec::new(),
        combines: Vec::new(),
        splits: Vec::new(),
        scales: Vec::new(),
        moves: Vec::new(),
        points: Vec::new(),
        sweeps: Vec::new(),
        imports: Vec::new(),
        joints: Vec::new(),
        components: Vec::new(),
        timeline: timeline.to_vec(),
    });
    while b.step(std::time::Duration::MAX) {}
    b.finish()
}

/// Exact STEP export of all bodies (true planes, cylinders, arcs).
pub fn write_step(path: &Path, bodies: &[&Body]) -> Result<(), String> {
    Shape::write_all_step(bodies.iter().map(|b| &b.shape), path).map_err(|e| format!("STEP export failed: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, Entity, OriginPlane, Tool};
    use std::f64::consts::PI;

    /// Timelines without revolves.
    fn build4(sketches: &[Sketch], extrudes: &[Extrude], edges: &[EdgeFeature], timeline: &[Feature]) -> Model {
        build(sketches, extrudes, &[], edges, timeline)
    }

    fn free(x: f64, y: f64) -> Anchor {
        Anchor::Free(DVec2::new(x, y))
    }

    fn rect(sk: &mut Sketch, x0: f64, y0: f64, x1: f64, y1: f64) {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        let first = sk.points.len();
        for p in [(x0, y0), (x1, y0), (x1, y1), (x0, y1)] {
            ed.click(sk, free(p.0, p.1));
        }
        ed.click(sk, Anchor::Point(first));
    }

    fn circle(sk: &mut Sketch, cx: f64, cy: f64, r: f64) {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(sk, free(cx, cy));
        ed.click(sk, free(cx + r, cy));
    }

    fn ex(sketch: usize, profiles: Vec<usize>, distance: f64, op: Operation) -> Extrude {
        Extrude { op, name: "E".into(), sketch, profiles, distance, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None }
    }

    fn timeline(sketches: usize, extrudes: usize) -> Vec<Feature> {
        (0..sketches).map(Feature::Sketch).chain((0..extrudes).map(Feature::Extrude)).collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-6 * b.abs().max(1.0)
    }

    fn total(m: &Model) -> f64 {
        m.bodies.iter().map(|b| b.shape.volume()).sum()
    }

    /// Every directed mesh edge matched by its reverse: watertight.
    fn closed(m: &Mesh) -> bool {
        let mut count: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &m.tris {
            for k in 0..3 {
                *count.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        count.iter().all(|(&(a, b), &c)| c == 1 && count.get(&(b, a)) == Some(&1))
    }


    /// A 4×3×5 box (volume 60) standing at x 2..6, y 0..3, plus a copy feature.
    fn copies_of_a_box(kind: crate::model::CopyKind) -> Model {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 2.0, 0.0, 6.0, 3.0);
        let c = CopyFeature { name: "C".into(), bodies: vec![BodyId { feature: Feature::Extrude(0), piece: 0 }], kind };
        let mut t = timeline(1, 1);
        t.push(Feature::Copy(0));
        build_with(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[c], &[], &[], &t)
    }

    /// (OCCT pads bounding boxes by 1e-7.)
    fn box_of(b: &Body) -> (DVec3, DVec3) {
        let bb = opencascade::bounding_box::aabb(&b.shape);
        (DVec3::new(bb.min().x, bb.min().y, bb.min().z), DVec3::new(bb.max().x, bb.max().y, bb.max().z))
    }

    fn extruded(extent: crate::model::Extent, distance: f64, distance2: f64) -> Model {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 0.0, 4.0, 3.0);
        let mut e = ex(0, vec![0], distance, Operation::NewBody);
        e.extent = extent;
        e.distance2 = distance2;
        build_with(&[sk], &[e], &[], &[], &[], &[], &[], &timeline(1, 1))
    }

    fn tapered(distance: f64, taper: f64, circle_not_square: bool) -> Model {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        if circle_not_square {
            circle(&mut sk, 0.0, 0.0, 5.0);
        } else {
            rect(&mut sk, 0.0, 0.0, 10.0, 10.0);
        }
        let mut e = ex(0, vec![0], distance, Operation::NewBody);
        e.taper = taper;
        build_with(&[sk], &[e], &[], &[], &[], &[], &[], &timeline(1, 1))
    }

    #[test]
    fn tapered_square_is_an_exact_frustum() {
        let m = tapered(10.0, 10.0, false);
        assert_eq!(m.errors, vec![None]);
        let top = 10.0 - 2.0 * 10.0 * 10.0_f64.to_radians().tan();
        let exact = 10.0 / 3.0 * (100.0 + top * top + 10.0 * top);
        assert!(close(total(&m), exact), "{} vs {exact}", total(&m));
        // (A lofted solid's OCCT box is loose; the mesh corners are exact.)
        let (lo, hi) = crate::model::mesh_bounds(&m.bodies[0].mesh).unwrap();
        assert!((hi.z - 10.0).abs() < 1e-5 && lo.z.abs() < 1e-5 && (hi.x - 10.0).abs() < 1e-5, "{lo:?} {hi:?}");
    }

    #[test]
    fn tapered_circle_is_an_exact_cone_frustum_and_negative_widens() {
        let m = tapered(10.0, 5.0, true);
        assert_eq!(m.errors, vec![None]);
        let r2 = 5.0 - 10.0 * 5.0_f64.to_radians().tan();
        let exact = std::f64::consts::PI * 10.0 / 3.0 * (25.0 + r2 * r2 + 5.0 * r2);
        assert!(close(total(&m), exact), "{} vs {exact}", total(&m));
        let m = tapered(10.0, -5.0, true);
        let r2 = 5.0 + 10.0 * 5.0_f64.to_radians().tan();
        let exact = std::f64::consts::PI * 10.0 / 3.0 * (25.0 + r2 * r2 + 5.0 * r2);
        assert!(close(total(&m), exact), "{} vs {exact}", total(&m));
    }

    #[test]
    fn tapering_downward_mirrors_it() {
        let m = tapered(-10.0, 10.0, false);
        assert_eq!(m.errors, vec![None]);
        let (lo, hi) = crate::model::mesh_bounds(&m.bodies[0].mesh).unwrap();
        assert!((lo.z + 10.0).abs() < 1e-5 && hi.z.abs() < 1e-5, "{lo:?} {hi:?}");
        let top = 10.0 - 2.0 * 10.0 * 10.0_f64.to_radians().tan();
        assert!(close(total(&m), 10.0 / 3.0 * (100.0 + top * top + 10.0 * top)));
    }

    #[test]
    fn a_taper_that_closes_the_profile_is_an_error_not_a_crash() {
        let m = tapered(10.0, 70.0, false);
        assert!(m.errors[0].is_some() || m.bodies.is_empty());
        let m = tapered(10.0, 85.0, true);
        assert!(m.errors[0].is_some() || m.bodies.is_empty());
    }

    #[test]
    fn tapering_a_ring_narrows_the_outside_and_widens_the_hole() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, -10.0, -10.0, 10.0, 10.0);
        circle(&mut sk, 0.0, 0.0, 4.0);
        let ring = crate::profile::find_regions(&sk).iter().position(|r| r.holes.len() == 1).expect("a region with a hole");
        let mut e = ex(0, vec![ring], 10.0, Operation::NewBody);
        e.taper = 5.0;
        let m = build_with(&[sk], &[e], &[], &[], &[], &[], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        let d = 10.0 * 5.0_f64.to_radians().tan();
        let b = 4.0 + d; // the hole's radius at the top
        let outer = 10.0 / 3.0 * (400.0 + (2.0 * (10.0 - d)) * (2.0 * (10.0 - d)) + 20.0 * 2.0 * (10.0 - d));
        let hole = std::f64::consts::PI * 10.0 / 3.0 * (16.0 + b * b + 4.0 * b);
        assert!(close(total(&m), outer - hole), "{} vs {}", total(&m), outer - hole);
    }

    #[test]
    fn symmetric_extrude_reaches_half_the_length_each_way() {
        let m = extruded(crate::model::Extent::Symmetric, 10.0, 0.0);
        assert!(close(total(&m), 120.0), "{}", total(&m));
        let (lo, hi) = box_of(&m.bodies[0]);
        assert!((lo.z + 5.0).abs() < 1e-6 && (hi.z - 5.0).abs() < 1e-6, "{lo:?} {hi:?}");
        // A negative length is the same body.
        let m = extruded(crate::model::Extent::Symmetric, -10.0, 0.0);
        let (lo, hi) = box_of(&m.bodies[0]);
        assert!((lo.z + 5.0).abs() < 1e-6 && (hi.z - 5.0).abs() < 1e-6);
    }

    #[test]
    fn two_sided_extrude_reaches_each_distance_its_own_way() {
        let m = extruded(crate::model::Extent::TwoSides, 10.0, 4.0);
        assert!(close(total(&m), 12.0 * 14.0), "{}", total(&m));
        let (lo, hi) = box_of(&m.bodies[0]);
        assert!((lo.z + 4.0).abs() < 1e-6 && (hi.z - 10.0).abs() < 1e-6, "{lo:?} {hi:?}");
        // Flipped: side 1 goes down, side 2 up.
        let m = extruded(crate::model::Extent::TwoSides, -10.0, 4.0);
        let (lo, hi) = box_of(&m.bodies[0]);
        assert!((lo.z + 10.0).abs() < 1e-6 && (hi.z - 4.0).abs() < 1e-6, "{lo:?} {hi:?}");
    }

    #[test]
    fn an_extrude_with_no_extent_in_an_old_file_is_one_sided() {
        let json = r#"{"op":"NewBody","name":"E","sketch":0,"profiles":[0],"distance":5.0}"#;
        let e: Extrude = serde_json::from_str(json).unwrap();
        assert_eq!(e.extent, crate::model::Extent::OneSide);
        assert_eq!(e.span(), (0.0, 5.0));
    }

    #[test]
    fn mirror_reflects_across_an_origin_plane_with_the_same_volume() {
        // YZ plane (x = 0): the box lands at x −6..−2.
        let m = copies_of_a_box(crate::model::CopyKind::Mirror(OriginPlane::YZ));
        assert_eq!(m.copy_errors, vec![None]);
        assert_eq!(m.bodies.len(), 2);
        assert!(close(total(&m), 120.0), "{}", total(&m));
        let (lo, hi) = box_of(&m.bodies[1]);
        assert!((lo - DVec3::new(-6.0, 0.0, 0.0)).length() < 1e-6 && (hi - DVec3::new(-2.0, 3.0, 5.0)).length() < 1e-6, "{lo:?} {hi:?}");
        assert!(m.bodies.iter().all(|b| closed(&b.mesh)));
        assert_eq!(m.bodies[1].id, BodyId { feature: Feature::Copy(0), piece: 0 });
        assert_eq!(m.touched.get(&Feature::Copy(0)), Some(&m.bodies[1].id));
    }

    #[test]
    fn rectangular_pattern_makes_a_grid_of_bodies() {
        use crate::model::{CopyKind, WorldAxis};
        let m = copies_of_a_box(CopyKind::Rectangular { dir: WorldAxis::X, count: 3, spacing: 10.0, dir2: WorldAxis::Y, count2: 2, spacing2: 7.0 });
        assert_eq!(m.copy_errors, vec![None]);
        assert_eq!(m.bodies.len(), 6);
        assert!(close(total(&m), 360.0));
        let mut spots: Vec<(i64, i64)> = m.bodies.iter().map(|b| { let (lo, _) = box_of(b); (lo.x.round() as i64, lo.y.round() as i64) }).collect();
        spots.sort();
        assert_eq!(spots, vec![(2, 0), (2, 7), (12, 0), (12, 7), (22, 0), (22, 7)]);
    }

    #[test]
    fn circular_pattern_spaces_copies_round_the_axis() {
        use crate::model::{CopyKind, WorldAxis};
        // Four copies over 360°: 90° apart. The box (x 2..6, y 0..3) rotated 90° about Z lands at x −3..0, y 2..6.
        let m = copies_of_a_box(CopyKind::Circular { axis: WorldAxis::Z, custom: None, count: 4, angle: 360.0, radial: 0.0 });
        assert_eq!(m.bodies.len(), 4);
        assert!(close(total(&m), 240.0));
        let (lo, hi) = box_of(&m.bodies[1]);
        assert!((lo - DVec3::new(-3.0, 2.0, 0.0)).length() < 1e-6 && (hi - DVec3::new(0.0, 6.0, 5.0)).length() < 1e-6, "{lo:?} {hi:?}");
        // Over 180° the last copy sits at the end of the angle: three copies at 0°, 90°, 180°.
        let m = copies_of_a_box(CopyKind::Circular { axis: WorldAxis::Z, custom: None, count: 3, angle: 180.0, radial: 0.0 });
        let (lo, hi) = box_of(&m.bodies[2]);
        assert!((lo - DVec3::new(-6.0, -3.0, 0.0)).length() < 1e-6 && (hi - DVec3::new(-2.0, 0.0, 5.0)).length() < 1e-6, "{lo:?} {hi:?}");
    }

    #[test]
    fn circular_pattern_can_push_the_copies_out_from_the_axis() {
        use crate::model::{CopyKind, WorldAxis};
        // The box is centred at (4, 1.5); a half turn about Z with 10 mm more radius puts the copy's
        // middle at distance 4.27 + 10 on the other side of the axis.
        let m = copies_of_a_box(CopyKind::Circular { axis: WorldAxis::Z, custom: None, count: 2, angle: 360.0, radial: 10.0 });
        assert_eq!(m.copy_errors, vec![None]);
        let (lo, hi) = box_of(&m.bodies[1]);
        let mid = (lo + hi) * 0.5;
        let want = -(DVec3::new(4.0, 1.5, 0.0).normalize() * (DVec3::new(4.0, 1.5, 0.0).length() + 10.0));
        assert!((mid.x - want.x).abs() < 1e-6 && (mid.y - want.y).abs() < 1e-6, "{mid:?} vs {want:?}");
        assert!(close(total(&m), 120.0));
    }

    #[test]
    fn copies_with_nothing_to_copy_are_errors_not_crashes() {
        use crate::model::{CopyKind, WorldAxis};
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 2.0, 0.0, 6.0, 3.0);
        let gone = BodyId { feature: Feature::Extrude(7), piece: 0 };
        let c = CopyFeature { name: "C".into(), bodies: vec![gone], kind: CopyKind::Mirror(OriginPlane::XY) };
        let one = CopyFeature { name: "C".into(), bodies: vec![BodyId { feature: Feature::Extrude(0), piece: 0 }], kind: CopyKind::Circular { axis: WorldAxis::Z, custom: None, count: 1, angle: 360.0, radial: 0.0 } };
        let mut t = timeline(1, 1);
        t.extend([Feature::Copy(0), Feature::Copy(1)]);
        let m = build_with(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[c, one], &[], &[], &t);
        assert!(m.copy_errors[0].as_deref().is_some_and(|e| e.contains("gone")));
        assert!(m.copy_errors[1].as_deref().is_some_and(|e| e.contains("no copies")));
        assert_eq!(m.bodies.len(), 1, "the original is untouched");
    }

    #[test]
    fn a_construction_axis_on_an_edge_is_found_and_patterns_turn_about_it() {
        use crate::model::{CopyKind, WorldAxis};
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 2.0, 0.0, 6.0, 3.0);
        let edge = EdgeRef { mid: DVec3::new(6.0, 3.0, 2.5), dir: DVec3::Z, faces: [DVec3::ZERO; 2], follow: None };
        let axis = AxisFeature { name: "Axis1".into(), edge, kind: None };
        let c = CopyFeature {
            name: "C".into(),
            bodies: vec![BodyId { feature: Feature::Extrude(0), piece: 0 }],
            kind: CopyKind::Circular { axis: WorldAxis::X, custom: Some(0), count: 2, angle: 360.0, radial: 0.0 },
        };
        let mut t = timeline(1, 1);
        t.extend([Feature::Axis(0), Feature::Copy(0)]);
        let m = build_with(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[c], &[axis], &[], &t);
        assert_eq!(m.axis_errors, vec![None]);
        let (o, d) = m.axes[0].unwrap();
        assert!((o.x - 6.0).abs() < 1e-9 && (o.y - 3.0).abs() < 1e-9 && d.dot(DVec3::Z).abs() > 0.999999);
        assert_eq!(m.copy_errors, vec![None]);
        // Half a turn about the box's corner edge swings it to x 6..10, y 3..6.
        let (lo, hi) = box_of(&m.bodies[1]);
        assert!((lo - DVec3::new(6.0, 3.0, 0.0)).length() < 1e-6 && (hi - DVec3::new(10.0, 6.0, 5.0)).length() < 1e-6, "{lo:?} {hi:?}");
    }

    #[test]
    fn a_curved_or_missing_edge_makes_an_axis_error() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 2.0, 0.0, 6.0, 3.0);
        let nowhere = EdgeRef { mid: DVec3::new(60.0, 3.0, 2.5), dir: DVec3::X, faces: [DVec3::ZERO; 2], follow: None };
        let mut t = timeline(1, 1);
        t.push(Feature::Axis(0));
        let m = build_with(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[], &[AxisFeature { name: "A".into(), edge: nowhere, kind: None }], &[], &t);
        // (No box edge runs along X near there… the nearest X edge is taken, so it resolves; a wrong direction can't.)
        let wrong = EdgeRef { mid: DVec3::new(6.0, 3.0, 2.5), dir: DVec3::new(0.0, 0.6, 0.8), faces: [DVec3::ZERO; 2], follow: None };
        assert!(m.axes.len() == 1);
        let mut sk2 = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk2, 2.0, 0.0, 6.0, 3.0);
        let m2 = build_with(&[sk2], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[], &[AxisFeature { name: "A".into(), edge: wrong, kind: None }], &[], &t);
        assert!(m2.axis_errors[0].as_deref().is_some_and(|e| e.contains("no longer exists")));
        assert_eq!(m2.axes, vec![None]);
    }

    #[test]
    fn straight_runs_are_told_from_curves() {
        let p = |x: f64, y: f64| DVec3::new(x, y, 0.0);
        assert!(is_straight(&[p(0.0, 0.0), p(5.0, 5.0), p(10.0, 10.0)]));
        assert!(!is_straight(&[p(0.0, 0.0), p(5.0, 1.0), p(10.0, 0.0)]));
        assert!(!is_straight(&[p(1.0, 1.0)]) && !is_straight(&[]));
    }

    #[test]
    fn revolve_about_a_construction_axis() {
        // A box at x 2..6, y 0..3 (z 0..5). Its bottom back edge (y = 3, z = 0) runs along X.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 2.0, 0.0, 6.0, 3.0);
        let mut sk2 = Sketch::new("P".into(), OriginPlane::XY);
        rect(&mut sk2, 2.0, 5.0, 4.0, 7.0); // 2 × 2, from 2 to 4 mm off the axis
        let edge = EdgeRef { mid: DVec3::new(4.0, 3.0, 0.0), dir: DVec3::X, faces: [DVec3::ZERO; 2], follow: None };
        let axis = AxisFeature { name: "Axis1".into(), edge, kind: None };
        let rv = |axis: RevolveAxis| Revolve { op: Operation::NewBody, name: "R".into(), sketch: 1, profiles: vec![0], axis, angle: 360.0 };
        let t = vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Axis(0), Feature::Sketch(1), Feature::Revolve(0)];
        let m = build_with(&[sk.clone(), sk2.clone()], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[rv(RevolveAxis::Construction(0))], &[], &[], &[axis.clone()], &[], &t);
        assert_eq!(m.revolve_errors, vec![None], "{:?}", m.revolve_errors);
        // A ring, outer radius 4, inner 2, 2 mm long.
        assert!(close(total(&m), 60.0 + PI * (16.0 - 4.0) * 2.0), "{}", total(&m));
        // An axis standing up through the sketch plane can't be revolved about.
        let up = AxisFeature { name: "Up".into(), edge: EdgeRef { mid: DVec3::new(6.0, 3.0, 2.5), dir: DVec3::Z, faces: [DVec3::ZERO; 2], follow: None }, kind: None };
        let m = build_with(&[sk, sk2], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[rv(RevolveAxis::Construction(0))], &[], &[], &[up], &[], &t);
        assert!(m.revolve_errors[0].as_deref().is_some_and(|e| e.contains("parallel")), "{:?}", m.revolve_errors);
    }

    #[test]
    fn mirror_in_a_construction_plane() {
        use crate::model::PlaneFeature;
        // The 4×3×5 box at x 2..6, mirrored in a plane 10 mm along X from the YZ plane: x 14..18.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 2.0, 0.0, 6.0, 3.0);
        let yz = FacePlane::from_face(DVec3::X, DVec3::ZERO);
        let plane = PlaneFeature { name: "P".into(), base: yz, on_origin: true, offset: 10.0, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None, kind: None };
        let c = CopyFeature { name: "M".into(), bodies: vec![BodyId { feature: Feature::Extrude(0), piece: 0 }], kind: crate::model::CopyKind::MirrorPlane(0) };
        let t = [Feature::Sketch(0), Feature::Extrude(0), Feature::Plane(0), Feature::Copy(0)];
        let m = build_all(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[c], &[], &[plane], &[], &[], &[], &[], &t);
        assert_eq!(m.copy_errors, vec![None]);
        assert_eq!(m.bodies.len(), 2);
        let copy = m.bodies.iter().find(|b| b.id.feature == Feature::Copy(0)).unwrap();
        let (lo, hi) = box_of(copy);
        assert!((lo - DVec3::new(14.0, 0.0, 0.0)).length() < 1e-6 && (hi - DVec3::new(18.0, 3.0, 5.0)).length() < 1e-6, "{lo} {hi}");
        assert!(close(copy.shape.volume(), 60.0));
        // Before its plane is built, the mirror can't work.
        let c2 = CopyFeature { name: "M".into(), bodies: vec![BodyId { feature: Feature::Extrude(0), piece: 0 }], kind: crate::model::CopyKind::MirrorPlane(0) };
        let mut sk2 = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk2, 2.0, 0.0, 6.0, 3.0);
        let early = build_all(&[sk2], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[c2], &[], &[PlaneFeature { name: "P".into(), base: yz, on_origin: true, offset: 10.0, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None, kind: None }], &[], &[], &[], &[], &[Feature::Sketch(0), Feature::Extrude(0), Feature::Copy(0), Feature::Plane(0)]);
        assert!(early.copy_errors[0].is_some());
    }

    #[test]
    fn construction_planes_offset_from_origin_planes_and_faces_carry_sketches() {
        use crate::model::PlaneFeature;
        // A 4×3×5 box at x 2..6. Plane 0: 2 mm above the XY plane. Plane 1: 3 mm above the box's top face.
        let mut base = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut base, 2.0, 0.0, 6.0, 3.0);
        let xy = FacePlane::from_face(DVec3::Z, DVec3::ZERO);
        let top = FacePlane::from_face(DVec3::Z, DVec3::new(4.0, 1.5, 5.0));
        let planes = [
            PlaneFeature { name: "P0".into(), base: xy, on_origin: true, offset: 2.0, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None, kind: None },
            PlaneFeature { name: "P1".into(), base: top, on_origin: false, offset: 3.0, anchor: [4.0, 1.5], extent: [0.0; 4], angle: None, kind: None },
        ];
        let mut on0 = Sketch::on_plane("A".into(), xy, 0);
        rect(&mut on0, 0.0, 0.0, 1.0, 1.0);
        let mut on1 = Sketch::on_plane("B".into(), top, 1);
        rect(&mut on1, 0.0, 0.0, 2.0, 2.0);
        let t = vec![
            Feature::Sketch(0),
            Feature::Extrude(0),
            Feature::Plane(0),
            Feature::Plane(1),
            Feature::Sketch(1),
            Feature::Extrude(1),
            Feature::Sketch(2),
            Feature::Extrude(2),
        ];
        let extrudes = [ex(0, vec![0], 5.0, Operation::NewBody), ex(1, vec![0], 1.0, Operation::NewBody), ex(2, vec![0], 1.0, Operation::NewBody)];
        let m = build_with(&[base, on0, on1], &extrudes, &[], &[], &[], &[], &planes, &t);
        assert_eq!(m.plane_errors, vec![None, None]);
        assert!((m.planes[0].unwrap().origin.z - 2.0).abs() < 1e-9);
        assert!((m.planes[1].unwrap().origin.z - 8.0).abs() < 1e-9, "{:?}", m.planes[1]);
        assert_eq!(m.sketch_errors, vec![None, None, None]);
        let lows: Vec<f64> = m.bodies.iter().map(|b| box_of(b).0.z).collect();
        assert!((lows[1] - 2.0).abs() < 1e-6 && (lows[2] - 8.0).abs() < 1e-6, "{lows:?}");
        assert!(close(total(&m), 60.0 + 1.0 + 4.0));
        // Grow the box: the face-based plane (and its sketch) rides up with it.
        let mut base2 = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut base2, 2.0, 0.0, 6.0, 3.0);
        let mut on1b = Sketch::on_plane("B".into(), top, 1);
        rect(&mut on1b, 0.0, 0.0, 2.0, 2.0);
        let m = build_with(&[base2, Sketch::on_plane("A".into(), xy, 0), on1b], &[ex(0, vec![0], 9.0, Operation::NewBody), ex(2, vec![0], 1.0, Operation::NewBody)], &[], &[], &[], &[], &planes, &[Feature::Sketch(0), Feature::Extrude(0), Feature::Plane(1), Feature::Sketch(2), Feature::Extrude(1)]);
        assert!((m.planes[1].unwrap().origin.z - 12.0).abs() < 1e-9, "{:?}", m.planes[1]);
    }

    fn ellipse(sk: &mut Sketch, c: (f64, f64), major: (f64, f64), b: f64) {
        let mut ed = Editor::default();
        ed.set_tool(Tool::Ellipse);
        ed.click(sk, free(c.0, c.1));
        ed.click(sk, free(c.0 + major.0, c.1 + major.1));
        let n = DVec2::new(major.0, major.1).normalize().perp();
        ed.click(sk, Anchor::Free(DVec2::new(c.0, c.1) + n * b));
    }

    #[test]
    fn ellipses_extrude_exactly_whole_as_holes_and_cut_by_lines() {
        // Whole: π a b h.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        ellipse(&mut sk, (0.0, 0.0), (20.0, 0.0), 10.0);
        let regions = find_regions(&sk);
        assert_eq!(regions.len(), 1, "one closed region");
        let m = build4(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        assert!(close(total(&m), PI * 20.0 * 10.0 * 5.0), "{}", total(&m));
        // Turned 30°, the shorter axis drawn first: the same.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let d = DVec2::from_angle(30f64.to_radians()) * 6.0;
        ellipse(&mut sk, (5.0, 5.0), (d.x, d.y), 15.0);
        let m = build4(&[sk], &[ex(0, vec![0], 2.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert!(close(total(&m), PI * 15.0 * 6.0 * 2.0), "{}", total(&m));
        // A hole in a plate.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, -30.0, -20.0, 30.0, 20.0);
        ellipse(&mut sk, (0.0, 0.0), (20.0, 0.0), 10.0);
        let ring = find_regions(&sk).iter().position(|r| r.holes.len() == 1).expect("the plate round the ellipse");
        let m = build4(&[sk], &[ex(0, vec![ring], 4.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert!(close(total(&m), (60.0 * 40.0 - PI * 200.0) * 4.0), "{}", total(&m));
        // Cut in half by a line across it (through the middle, along the short axis).
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        ellipse(&mut sk, (0.0, 0.0), (20.0, 0.0), 10.0);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, free(0.0, -15.0));
        ed.click(&mut sk, free(0.0, 15.0));
        let halves: Vec<usize> = find_regions(&sk).iter().enumerate().filter(|(_, r)| r.area() > 100.0).map(|(i, _)| i).collect();
        assert_eq!(halves.len(), 2, "the line splits it");
        let m = build4(&[sk], &[ex(0, vec![halves[0]], 3.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        assert!(close(total(&m), PI * 200.0 / 2.0 * 3.0), "{}", total(&m));
    }

    #[test]
    fn a_control_point_spline_bounds_a_profile() {
        // An arch pulled toward (10, 20) and (30, 20), closed by a line along the bottom.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::ControlSpline);
        for p in [(0.0, 0.0), (10.0, 20.0), (30.0, 20.0), (40.0, 0.0)] {
            ed.click(&mut sk, free(p.0, p.1));
        }
        ed.finish_spline(&mut sk);
        assert_eq!(sk.control_splines, vec![(0, 3)]);
        ed.set_tool(Tool::Line);
        let ends: Vec<Anchor> = sk.splines[0].iter().map(|&p| Anchor::Point(p)).collect();
        ed.click(&mut sk, ends[3]);
        ed.click(&mut sk, ends[0]);
        let regions = find_regions(&sk);
        assert_eq!(regions.len(), 1);
        // Four points, degree 3: a cubic Bézier, y = 60 t (1 − t); the area
        // under it (∫ y dx) is exactly 420.
        let m = build4(&[sk], &[ex(0, vec![0], 2.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        assert!((total(&m) - 420.0 * 2.0).abs() < 1e-3, "{}", total(&m));
    }

    /// A block, and a second sketch with the block's front edge projected
    /// (linked) and a 10 × 10 square hanging off its right end.
    fn projected_square(width: f64) -> (Sketch, Sketch) {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, width, 10.0);
        let base = build4(&[a.clone()], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        let front = base.bodies[0].edges.iter().find(|e| e.at.mid.distance(DVec3::new(width / 2.0, 0.0, 5.0)) < 1e-6).expect("top-front edge");
        let pts: Vec<DVec3> = front.points.iter().map(|p| p.as_dvec3()).collect();
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, width, 0.0, width + 10.0, 10.0);
        {
            use crate::sketch::Constraint::*;
            for c in [Horizontal(0), Vertical(1), Horizontal(2), Vertical(3), Length(0, 10.0), Length(3, 10.0)] {
                crate::solver::add_constraints(&mut b, &[c]).unwrap();
            }
        }
        let corner = 1; // the square's first corner, at (width, 0)
        assert!(b.points[corner].distance(DVec2::new(width, 0.0)) < 1e-9);
        assert_eq!(crate::project::project_all(&mut b, DVec3::ZERO, DVec3::X, DVec3::Y, &[(Some(front.at), pts)]), 1);
        // The square's corner becomes the projected line's right end.
        let line = b.entities.len() - 1;
        let Entity::Line { a: p, b: q } = b.entities[line] else { panic!() };
        let end = if b.points[p].x > b.points[q].x { p } else { q };
        for e in &mut b.entities[..line] {
            if let Entity::Line { a, b } = e {
                for x in [a, b] {
                    if *x == corner {
                        *x = end;
                    }
                }
            }
        }
        crate::solver::solve(&mut b, &[]).unwrap();
        (a, b)
    }

    #[test]
    fn linked_projections_follow_the_edge_and_carry_what_is_drawn_on_them() {
        let (mut a, b) = projected_square(20.0);
        let t = [Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1)];
        let exs = [ex(0, vec![0], 5.0, Operation::NewBody), ex(1, vec![0], 3.0, Operation::NewBody)];
        let m = build4(&[a.clone(), b.clone()], &exs, &[], &t);
        assert_eq!(m.linked_sketches, vec![None, None], "nothing moved");
        assert_eq!(m.projection_lost, vec![Vec::<usize>::new(), Vec::new()]);
        assert!(close(total(&m), 20.0 * 10.0 * 5.0 + 10.0 * 10.0 * 3.0));
        // The block gets 10 mm wider: the projected edge, and the square on its end, move with it.
        for p in &mut a.points {
            if (p.x - 20.0).abs() < 1e-9 {
                p.x = 30.0;
            }
        }
        let m = build4(&[a.clone(), b.clone()], &exs, &[], &t);
        let moved = m.linked_sketches[1].as_ref().expect("the second sketch follows");
        let xs: Vec<f64> = moved.points.iter().skip(1).filter(|p| p.y.abs() < 1e-9).map(|p| p.x).collect();
        assert!(xs.iter().any(|x| (x - 30.0).abs() < 1e-9) && xs.iter().any(|x| (x - 40.0).abs() < 1e-9), "{xs:?}");
        // And what's built from the sketch is built from it as moved: the square's extrude starts at x = 30.
        assert!(close(total(&m), 30.0 * 10.0 * 5.0 + 10.0 * 10.0 * 3.0));
        let square = m.bodies.iter().find(|bd| bd.id.feature == Feature::Extrude(1)).expect("square body");
        let (lo, hi) = bbox(square);
        assert!((lo.x - 30.0).abs() < 1e-6 && (hi.x - 40.0).abs() < 1e-6, "{lo} {hi}");
        // Taken in, the sketch builds the same with nothing more to move.
        let m2 = build4(&[a.clone(), moved.clone()], &exs, &[], &t);
        assert_eq!(m2.linked_sketches, vec![None, None]);
        assert!(close(total(&m2), total(&m)));
    }

    #[test]
    fn a_projection_whose_edge_is_gone_is_lost_and_stays_put() {
        let (a, b) = projected_square(20.0);
        // Without the block (its extrude not in the timeline) the edge isn't there.
        let m = build4(&[a, b.clone()], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[Feature::Sketch(0), Feature::Sketch(1)]);
        let line = b.projections[0].0;
        assert_eq!(m.projection_lost[1], vec![line]);
        assert_eq!(m.linked_sketches[1], None);
        assert_eq!(m.sketch_errors[1], None, "a warning, not an error");
    }

    fn text_sketch(words: &str, height: f64) -> Sketch {
        let mut sk = Sketch::new("T".into(), OriginPlane::XY);
        let st = crate::sketch::SketchText { text: words.into(), height, font: crate::text::HACK.into(), bold: false, italic: false, angle: 0.0 };
        sk.add_text(Anchor::Free(DVec2::new(10.0, 5.0)), st);
        sk
    }

    #[test]
    fn straight_letters_extrude_to_their_exact_area() {
        // Hack's H is all straight lines: the solid is exactly its outline's area times the distance.
        let sk = text_sketch("H", 10.0);
        let regions = find_regions(&sk);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].text_entity(), Some(0), "a letter knows its text");
        let c = &sk.text_contours(0)[0];
        let area = (0..c.pts.len()).map(|i| c.pts[i].perp_dot(c.pts[(i + 1) % c.pts.len()])).sum::<f64>().abs() / 2.0;
        let m = build4(&[sk], &[ex(0, vec![0], 4.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        assert!(close(total(&m), area * 4.0), "{} vs {}", total(&m), area * 4.0);
        let (lo, hi) = bbox(&m.bodies[0]);
        assert!((lo.y - 5.0).abs() < 1e-6 && (hi.y - 15.0).abs() < 1e-6, "on the baseline at the anchor, capitals 10 tall: {lo} {hi}");
    }

    #[test]
    fn round_letters_make_smooth_solids_with_their_holes() {
        let sk = text_sketch("O8", 10.0);
        let regions = find_regions(&sk);
        // The letters: O a ring (one region with a hole), 8 one region with two holes;
        // the spaces inside them are regions too, but not the text's ink.
        let ink: Vec<usize> = (0..regions.len()).filter(|&r| regions[r].text_entity() == Some(0) && regions[r].is_text_ink(&sk, 0)).collect();
        assert_eq!(regions.len(), 5);
        assert_eq!(ink.len(), 2);
        assert_eq!(ink.iter().map(|&r| regions[r].holes.len()).sum::<usize>(), 3);
        let flat: f64 = ink.iter().map(|&r| regions[r].exact_area()).sum();
        let m = build4(&[sk], &[ex(0, ink, 2.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        // The font's curves are smooth in the solid: within a hair of the flattened outline's area.
        assert!((total(&m) - flat * 2.0).abs() < flat * 2.0 * 2e-3, "{} vs {}", total(&m), flat * 2.0);
        // Few faces: each font curve is one smooth face, not dozens of flat ones.
        let faces = m.bodies.iter().map(|b| b.faces.len()).sum::<usize>();
        assert!(faces < 80, "{faces} faces");
    }

    #[test]
    fn a_cylinder_projects_linked_and_stays_put() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        circle(&mut a, 60.0, 0.0, 20.0);
        let exs = [ex(0, vec![0], 25.0, Operation::NewBody)];
        let base = build4(&[a.clone()], &exs, &[], &timeline(1, 1));
        let edges: Vec<(Option<EdgeRef>, Vec<DVec3>)> = base.bodies[0].edges.iter().map(|e| (Some(e.at), e.points.iter().map(|p| p.as_dvec3()).collect())).collect();
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        let n = crate::project::project_all(&mut b, DVec3::ZERO, DVec3::X, DVec3::Y, &edges);
        assert_eq!(n, 1, "top and bottom circles are the same seen from above");
        let m = build4(&[a, b.clone()], &exs, &[], &[Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1)]);
        assert_eq!(m.projection_lost[1], Vec::<usize>::new());
        // From the view's single-precision edge it's a hair off; the build puts it exactly on the edge.
        let moved = m.linked_sketches[1].as_ref().expect("snapped onto the exact circle");
        let Entity::Circle { center, radius } = moved.entities[0] else { panic!() };
        assert!(moved.points[center].distance(DVec2::new(60.0, 0.0)) < 1e-9 && (radius - 20.0).abs() < 1e-9);
        assert!(moved.points[center].distance(b.points[center]) < 1e-5);
    }

    #[test]
    fn a_sketch_whose_face_is_gone_has_all_its_projections_lost() {
        let (a, mut b) = projected_square(20.0);
        let line = b.projections[0].0;
        // On a face that isn't there (as when the extrude making it is deleted).
        b.face = Some(crate::sketch::FacePlane { origin: DVec3::new(0.0, 0.0, 100.0), u: DVec3::X, v: DVec3::Y, follow: None });
        let m = build4(&[a, b], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &timeline(2, 1));
        assert!(m.sketch_errors[1].is_some());
        assert_eq!(m.projection_lost[1], vec![line]);
    }

    #[test]
    fn a_section_keeps_the_half_behind_the_plane_with_its_cut_face() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 10.0);
        let m = build4(&[a], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        let yz = ExtrudeTarget::Origin(OriginPlane::YZ);
        let n = OriginPlane::YZ.plane().normal().as_dvec3();
        // The plane x = 5 (along its normal, whichever way that points).
        let offset = 5.0 * n.x.signum();
        let volume = |cuts: &[SectionCut]| -> f64 {
            // Signed volume of a closed mesh: sum of tetrahedra from the origin.
            cuts.iter().map(|c| c.mesh.tris.iter().map(|t| {
                let [p, q, r] = t.map(|i| c.mesh.positions[i as usize].as_dvec3());
                p.dot(q.cross(r)) / 6.0
            }).sum::<f64>()).sum()
        };
        let kept = section(&m.bodies, &m.planes, &yz, offset, false).unwrap();
        let other = section(&m.bodies, &m.planes, &yz, offset, true).unwrap();
        let (v1, v2) = (volume(&kept), volume(&other));
        assert!((v1 + v2 - 1000.0).abs() < 1e-3, "the two halves make the box: {v1} + {v2}");
        let small = if n.x > 0.0 { v1 } else { v2 };
        assert!((small - 250.0).abs() < 1e-3, "the half on the x < 5 side: {small}");
        // The cut face is marked, and it's the 10 × 5 cross-section.
        let cap_area: f64 = kept[0].mesh.tris.iter().zip(&kept[0].cap).filter(|(_, c)| **c).map(|(t, _)| {
            let [p, q, r] = t.map(|i| kept[0].mesh.positions[i as usize].as_dvec3());
            (q - p).cross(r - p).length() / 2.0
        }).sum();
        assert!((cap_area - 50.0).abs() < 1e-3, "{cap_area}");
        // Past the part: nothing taken away, or nothing left.
        let all = section(&m.bodies, &m.planes, &yz, 100.0 * n.x.signum(), false).unwrap();
        let none = section(&m.bodies, &m.planes, &yz, -100.0 * n.x.signum(), false).unwrap();
        assert!((volume(&all) - 1000.0).abs() < 1e-3 && none.is_empty());
        // Slice: any plane by point and normal; the side the normal points to goes.
        let below = section_plane(&m.bodies, DVec3::new(0.0, 0.0, 2.0), DVec3::Z * 3.0).unwrap();
        assert!((volume(&below) - 400.0).abs() < 1e-3, "20 × 10 × 2 left: {}", volume(&below));
        assert!(section_plane(&m.bodies, DVec3::ZERO, DVec3::ZERO).is_err());
        // No bodies (a sketch before the first feature, edited with Slice on).
        assert!(section_plane(&[], DVec3::ZERO, DVec3::Z).unwrap().is_empty());
    }

    #[test]
    fn midplanes_three_point_planes_and_new_axes_land_where_they_should() {
        use crate::model::{AxisKind, PlaneKind, PointRef};
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 10.0);
        circle(&mut a, 40.0, 5.0, 3.0);
        let exs = [ex(0, vec![0, 1], 5.0, Operation::NewBody)];
        let base = |kind: PlaneKind| PlaneFeature { name: "P".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: false, offset: 0.0, anchor: [0.0; 2], extent: [0.0; 4], angle: None, kind: Some(kind) };
        let up10 = PlaneFeature { name: "Up".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: true, offset: 10.0, anchor: [0.0; 2], extent: [0.0; 4], angle: None, kind: None };
        let corner = |x: f64, y: f64, z: f64| PointRef::Corner(DVec3::new(x, y, z) + DVec3::splat(0.4)); // picked a little off
        let planes = [
            up10,
            // Halfway between XY and the plane 10 up: z = 5.
            base(PlaneKind::Midplane(ExtrudeTarget::Origin(OriginPlane::XY), ExtrudeTarget::Plane(0))),
            // Through three top corners of the block: z = 5.
            base(PlaneKind::ThreePoints([corner(0.0, 0.0, 5.0), corner(20.0, 0.0, 5.0), corner(0.0, 10.0, 5.0)])),
            // XZ and YZ meet along Z: the plane halving them.
            base(PlaneKind::Midplane(ExtrudeTarget::Origin(OriginPlane::XZ), ExtrudeTarget::Origin(OriginPlane::YZ))),
        ];
        let cyl = CylinderRef { axis_point: DVec3::new(40.0, 5.0, 0.0), axis: DVec3::Z, radius: 3.0, at: DVec3::new(43.0, 5.0, 2.0), internal: false, length: 5.0 };
        let planes = [planes.to_vec(), vec![base(PlaneKind::Tangent(cyl))]].concat();
        let axes = [
            AxisFeature { name: "A1".into(), edge: EdgeRef::default(), kind: Some(AxisKind::TwoPoints([corner(0.0, 0.0, 0.0), corner(20.0, 10.0, 5.0)])) },
            AxisFeature { name: "A2".into(), edge: EdgeRef::default(), kind: Some(AxisKind::Cylinder(CylinderRef { axis_point: DVec3::new(40.2, 5.1, 0.0), ..cyl })) },
        ];
        let mut t = vec![Feature::Sketch(0), Feature::Extrude(0)];
        t.extend((0..planes.len()).map(Feature::Plane));
        t.extend((0..axes.len()).map(Feature::Axis));
        let m = build_all(&[a], &exs, &[], &[], &[], &axes, &planes, &[], &[], &[], &[], &t);
        assert!(m.plane_errors.iter().all(|e| e.is_none()) && m.axis_errors.iter().all(|e| e.is_none()), "{:?} {:?}", m.plane_errors, m.axis_errors);
        let height = |k: usize| { let f = m.planes[k].unwrap(); assert!(f.normal().cross(DVec3::Z).length() < 1e-9, "flat"); f.origin.dot(f.normal()) * f.normal().z };
        assert!((height(1) - 5.0).abs() < 1e-9 && (height(2) - 5.0).abs() < 1e-9);
        let bisect = m.planes[3].unwrap();
        assert!(bisect.normal().z.abs() < 1e-9 && (bisect.normal().x.abs() - bisect.normal().y.abs()).abs() < 1e-9, "the 45° plane between XZ and YZ: {:?}", bisect.normal());
        assert!(bisect.origin.dot(bisect.normal()).abs() < 1e-9, "through the Z axis");
        // Tangent to the cylinder (radius 3 round x = 40, y = 5) where it was picked: the plane x = 43.
        let tangent = m.planes[4].unwrap();
        assert!((tangent.normal() - DVec3::X).length() < 1e-9 && (tangent.origin.dot(DVec3::X) - 43.0).abs() < 1e-9);
        let (o, d) = m.axes[0].unwrap();
        assert!(o.distance(DVec3::ZERO) < 1e-9 && (d - DVec3::new(20.0, 10.0, 5.0).normalize()).length() < 1e-9, "corner to corner");
        let (o, d) = m.axes[1].unwrap();
        assert!(d.cross(DVec3::Z).length() < 1e-9 && (o - DVec3::new(40.0, 5.0, o.z)).length() < 1e-9, "the cylinder's own axis");
    }

    #[test]
    fn construction_points_sit_on_corners_and_circle_centres_and_planes_go_through_them() {
        use crate::model::{ConstructionPoint, PlaneKind, PointKind, PointRef};
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 10.0);
        circle(&mut a, 40.0, 5.0, 3.0);
        let base = build4(&[a.clone()], &[ex(0, vec![0, 1], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        let rim = base.bodies.iter().flat_map(|b| &b.edges).find(|e| matches!(e.geom, Geom::Circle { center, .. } if (center.z - 5.0).abs() < 1e-9)).expect("the shaft's top rim").at;
        let points = vec![
            ConstructionPoint { name: "P1".into(), kind: PointKind::Corner(DVec3::new(20.3, 10.2, 5.1)) },
            ConstructionPoint { name: "P2".into(), kind: PointKind::CircleCentre(rim) },
        ];
        let plane = PlaneFeature { name: "Through".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: false, offset: 0.0, anchor: [0.0; 2], extent: [0.0; 4], angle: None,
            kind: Some(PlaneKind::ThreePoints([PointRef::Point(0), PointRef::Point(1), PointRef::Corner(DVec3::new(0.0, 0.0, 5.0))])) };
        let mut b = Build::start(BuildInputs {
            sketches: vec![a], extrudes: vec![ex(0, vec![0, 1], 5.0, Operation::NewBody)], revolves: vec![], edge_features: vec![], copies: vec![], axis_features: vec![],
            plane_features: vec![plane], removes: vec![], holes: vec![], thread_features: vec![], shells: vec![], offset_faces: vec![], combines: vec![], splits: vec![], scales: vec![], moves: vec![], points, sweeps: vec![], imports: vec![], joints: vec![], components: vec![],
            timeline: vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Point(0), Feature::Point(1), Feature::Plane(0)],
        });
        while b.step(std::time::Duration::MAX) {}
        let m = b.finish();
        assert_eq!(m.point_errors, vec![None, None]);
        assert!(m.points[0].unwrap().distance(DVec3::new(20.0, 10.0, 5.0)) < 1e-9, "on the nearest corner");
        assert!(m.points[1].unwrap().distance(DVec3::new(40.0, 5.0, 5.0)) < 1e-9, "at the rim's centre");
        // All three are at z = 5: the plane is the top.
        let f = m.planes[0].expect("built");
        assert!(f.normal().cross(DVec3::Z).length() < 1e-9 && (f.origin.dot(f.normal()) * f.normal().z - 5.0).abs() < 1e-9);
    }

    #[test]
    fn interference_finds_exactly_the_shared_volume_and_bounds_are_exact() {
        // Block A 0..20 × 0..10 × 0..5; block B 15..30 × 5..15 overlaps it by
        // 5 × 5 × 5; shaft C r 3 at (50, 5) is apart; block D 20..25 only touches A.
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 10.0);
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 15.0, 5.0, 30.0, 15.0);
        let mut c = Sketch::new("C".into(), OriginPlane::XY);
        circle(&mut c, 50.0, 5.0, 3.0);
        let mut d = Sketch::new("D".into(), OriginPlane::XY);
        rect(&mut d, 20.0, -5.0, 25.0, -1.0);
        rect(&mut d, 20.0, 0.0, 25.0, 3.0);
        let exs = [ex(0, vec![0], 5.0, Operation::NewBody), ex(1, vec![0], 5.0, Operation::NewBody), ex(2, vec![0], 5.0, Operation::NewBody), ex(3, vec![1], 5.0, Operation::NewBody)];
        let m = build4(&[a, b, c, d], &exs, &[], &timeline(4, 4));
        assert_eq!(m.bodies.len(), 4);
        let shapes: Vec<&Shape> = m.bodies.iter().map(|b| &b.shape).collect();
        let hits = interference(&shapes);
        assert_eq!(hits.len(), 1, "only A and B overlap (D just touches A)");
        let (i, j, common) = &hits[0];
        let made_by = |k: usize| m.bodies[k].id.feature;
        assert_eq!((made_by(*i), made_by(*j)), (Feature::Extrude(0), Feature::Extrude(1)));
        assert!(close(common.volume(), 125.0), "{}", common.volume());
        // The shaft's box is exact: 47..53 × 2..8 × 0..5.
        let shaft = m.bodies.iter().find(|b| b.id.feature == Feature::Extrude(2)).unwrap();
        let (lo, hi) = exact_bounds(&shaft.shape).unwrap();
        assert!(lo.distance(DVec3::new(47.0, 2.0, 0.0)) < 1e-6 && hi.distance(DVec3::new(53.0, 8.0, 5.0)) < 1e-6, "{lo} {hi}");
    }

    #[test]
    fn joints_put_one_part_onto_another_then_turn_slide_and_flip_it() {
        use crate::assembly::Component;
        use crate::model::{JointKind, JointSnap};
        // A: 0..20 × 0..20 × 0..10. B: 50..70 × 0..10 × 0..5. C: a 2 × 2 × 2 cube on B's far end (50..52 × 0..2 × 5..7)... apart, 80..82.
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 20.0);
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 50.0, 0.0, 70.0, 10.0);
        let mut c = Sketch::new("C".into(), OriginPlane::XY);
        rect(&mut c, 80.0, 0.0, 82.0, 2.0);
        let exs = vec![ex(0, vec![0], 10.0, Operation::NewBody), ex(1, vec![0], 5.0, Operation::NewBody), ex(2, vec![0], 2.0, Operation::NewBody)];
        let id = |k: usize| BodyId { feature: Feature::Extrude(k), piece: 0 };
        let face = |n: DVec3, q: DVec3| {
            let f = FacePlane::from_face(n, q);
            JointSnap::Face { face: f, anchor: [q.dot(f.u), q.dot(f.v)] }
        };
        // B's bottom (at 55, 5) onto A's top (at 10, 10).
        let joint = |kind, angle, offset, flip| JointFeature { name: "J".into(), a: face(DVec3::NEG_Z, DVec3::new(55.0, 5.0, 0.0)), b: face(DVec3::Z, DVec3::new(10.0, 10.0, 10.0)), kind, angle, offset, flip, motion: 0.0 };
        let run = |j: JointFeature, components: Vec<Component>| {
            let mut bld = Build::start(BuildInputs {
                sketches: vec![a.clone(), b.clone(), c.clone()], extrudes: exs.clone(), revolves: vec![], edge_features: vec![], copies: vec![], axis_features: vec![], plane_features: vec![], removes: vec![], holes: vec![],
                thread_features: vec![], shells: vec![], offset_faces: vec![], combines: vec![], splits: vec![], scales: vec![], moves: vec![], points: vec![], sweeps: vec![], imports: vec![], joints: vec![j], components,
                timeline: vec![Feature::Sketch(0), Feature::Sketch(1), Feature::Sketch(2), Feature::Extrude(0), Feature::Extrude(1), Feature::Extrude(2), Feature::Joint(0)],
            });
            while bld.step(std::time::Duration::MAX) {}
            bld.finish()
        };
        let boxes = |m: &Model, k: usize| exact_bounds(&m.bodies.iter().find(|b| b.id == id(k)).unwrap().shape).unwrap();
        let near = |got: (DVec3, DVec3), lo: [f64; 3], hi: [f64; 3]| got.0.distance(DVec3::from(lo)) < 1e-6 && got.1.distance(DVec3::from(hi)) < 1e-6;
        // Rigid: B sits on A's top, the two spots together.
        let m = run(joint(JointKind::Rigid, 0.0, 0.0, false), vec![]);
        assert_eq!(m.joint_errors, vec![None]);
        assert!(near(boxes(&m, 1), [5.0, 5.0, 10.0], [25.0, 15.0, 15.0]), "{:?}", boxes(&m, 1));
        assert!(near(boxes(&m, 0), [0.0; 3], [20.0, 20.0, 10.0]), "A stays");
        assert!(close(m.bodies.iter().find(|b| b.id == id(1)).unwrap().shape.volume(), 1000.0));
        // Revolute 90°: turned about A's top at (10, 10).
        let m = run(joint(JointKind::Revolute, 90.0, 0.0, false), vec![]);
        assert!(near(boxes(&m, 1), [5.0, 5.0, 10.0], [15.0, 25.0, 15.0]), "{:?}", boxes(&m, 1));
        // Slider 3: lifted along the joint's axis.
        let m = run(joint(JointKind::Slider, 0.0, 3.0, false), vec![]);
        assert!(near(boxes(&m, 1), [5.0, 5.0, 13.0], [25.0, 15.0, 18.0]), "{:?}", boxes(&m, 1));
        // Flipped: facing the same way, so it sinks into A instead.
        let m = run(joint(JointKind::Rigid, 0.0, 0.0, true), vec![]);
        assert!(near(boxes(&m, 1), [5.0, 5.0, 5.0], [25.0, 15.0, 10.0]), "{:?}", boxes(&m, 1));
        // B and C in one component: C goes along (30 to the right of B's start, 5 lower).
        let m = run(joint(JointKind::Rigid, 0.0, 0.0, false), vec![Component { name: "BC".into(), bodies: vec![id(1), id(2)] }]);
        assert!(near(boxes(&m, 2), [35.0, 5.0, 10.0], [37.0, 7.0, 12.0]), "{:?}", boxes(&m, 2));
        // Motion: a revolute joint turns by it, a slider slides, a rigid one ignores it.
        let m = run(JointFeature { motion: 90.0, ..joint(JointKind::Revolute, 0.0, 0.0, false) }, vec![]);
        assert!(near(boxes(&m, 1), [5.0, 5.0, 10.0], [15.0, 25.0, 15.0]), "{:?}", boxes(&m, 1));
        let m = run(JointFeature { motion: 3.0, ..joint(JointKind::Slider, 0.0, 0.0, false) }, vec![]);
        assert!(near(boxes(&m, 1), [5.0, 5.0, 13.0], [25.0, 15.0, 18.0]), "{:?}", boxes(&m, 1));
        let m = run(JointFeature { motion: 90.0, ..joint(JointKind::Rigid, 0.0, 0.0, false) }, vec![]);
        assert!(near(boxes(&m, 1), [5.0, 5.0, 10.0], [25.0, 15.0, 15.0]), "rigid: locked");
        // Both ends on the same part: refused, nothing moves.
        let both = JointFeature { b: face(DVec3::Z, DVec3::new(60.0, 5.0, 5.0)), ..joint(JointKind::Rigid, 0.0, 0.0, false) };
        let m = run(both, vec![]);
        assert!(m.joint_errors[0].is_some());
        assert!(near(boxes(&m, 1), [50.0, 0.0, 0.0], [70.0, 10.0, 5.0]));
    }

    #[test]
    fn a_flat_face_offers_its_middle_side_middles_and_corners() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 10.0);
        circle(&mut a, 5.0, 5.0, 2.0);
        // The block with the hole in it (the region that isn't the circle).
        let ring = (0..find_regions(&a).len()).find(|&k| find_regions(&a)[k].area() > 50.0).unwrap();
        let m = build4(&[a], &[ex(0, vec![ring], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        let b = &m.bodies[0];
        let top = (0..b.mesh.tris.len()).find(|&t| b.mesh.normal(t).z > 0.99).unwrap();
        let (n, pts) = face_snaps(b, top).expect("the top is flat");
        assert!((n - DVec3::Z).length() < 1e-6);
        let has = |x: f64, y: f64| pts.iter().any(|p| p.distance(DVec3::new(x, y, 5.0)) < 1e-6);
        // The middle of the face (its area, the hole taken out: pulled away from the hole's side).
        let hole = std::f64::consts::PI * 4.0;
        let cx = (200.0 * 10.0 - hole * 5.0) / (200.0 - hole);
        assert!((pts[0] - DVec3::new(cx, 5.0, 5.0)).length() < 1e-9, "{:?}", pts[0]);
        for (x, y) in [(10.0, 0.0), (20.0, 5.0), (10.0, 10.0), (0.0, 5.0), (0.0, 0.0), (20.0, 0.0), (20.0, 10.0), (0.0, 10.0)] {
            assert!(has(x, y), "({x}, {y}) missing from {pts:?}");
        }
        assert_eq!(pts.len(), 9, "middle, 4 side middles, 4 corners — nothing from the hole");
        // A construction point at the face's middle sits there.
        let f = FacePlane::from_face(n, pts[0]);
        let at = place_point(&[&b.shape], &ConstructionPoint { name: "P".into(), kind: crate::model::PointKind::OnFace { face: f, anchor: [pts[0].dot(f.u), pts[0].dot(f.v)] } }).unwrap();
        assert!(at.distance(pts[0]) < 1e-9, "{at}");
        // A side face is flat too; the hole's wall isn't.
        let wall = (0..b.mesh.tris.len()).find(|&t| matches!(b.faces[b.tri_face[t] as usize], Geom::Cylinder { .. })).unwrap();
        assert!(face_snaps(b, wall).is_none());
    }

    #[test]
    fn a_revolute_joint_at_two_holes_lays_the_lever_on_the_base_and_turns_it() {
        use crate::model::{JointKind, JointSnap};
        // Base 60 × 40 × 10 with a hole Ø6 at (10, 10); lever 50 × 10 × 5 off to the side with a hole at (85, 5).
        let mut base = Sketch::new("Base".into(), OriginPlane::XY);
        rect(&mut base, 0.0, 0.0, 60.0, 40.0);
        circle(&mut base, 10.0, 10.0, 3.0);
        let mut lever = Sketch::new("Lever".into(), OriginPlane::XY);
        rect(&mut lever, 80.0, 0.0, 130.0, 10.0);
        circle(&mut lever, 85.0, 5.0, 3.0);
        // The regions with a hole: the one that isn't the circle.
        let ring = |sk: &Sketch| (0..find_regions(sk).len()).find(|&k| find_regions(sk)[k].area() > 100.0).unwrap();
        let exs = vec![ex(0, vec![ring(&base)], 10.0, Operation::NewBody), ex(1, vec![ring(&lever)], 5.0, Operation::NewBody)];
        let before = build4(&[base.clone(), lever.clone()], &exs, &[], &timeline(2, 2));
        // The holes' top rims, as a click would pick them.
        let rim = |k: usize, z: f64| {
            before.bodies.iter().find(|b| b.id.feature == Feature::Extrude(k)).unwrap().edges.iter()
                .find(|e| matches!(e.geom, Geom::Circle { center, .. } if (center.z - z).abs() < 1e-9)).unwrap().at
        };
        let run = |angle: f64| {
            let j = JointFeature { name: "J".into(), a: JointSnap::Circle(rim(1, 5.0)), b: JointSnap::Circle(rim(0, 10.0)), kind: JointKind::Revolute, angle, offset: 0.0, flip: false, motion: 0.0 };
            let mut bld = Build::start(BuildInputs {
                sketches: vec![base.clone(), lever.clone()], extrudes: exs.clone(), revolves: vec![], edge_features: vec![], copies: vec![], axis_features: vec![], plane_features: vec![], removes: vec![], holes: vec![],
                thread_features: vec![], shells: vec![], offset_faces: vec![], combines: vec![], splits: vec![], scales: vec![], moves: vec![], points: vec![], sweeps: vec![], imports: vec![], joints: vec![j], components: vec![],
                timeline: vec![Feature::Sketch(0), Feature::Sketch(1), Feature::Extrude(0), Feature::Extrude(1), Feature::Joint(0)],
            });
            while bld.step(std::time::Duration::MAX) {}
            let m = bld.finish();
            assert_eq!(m.joint_errors, vec![None]);
            exact_bounds(&m.bodies.iter().find(|b| b.id.feature == Feature::Extrude(1)).unwrap().shape).unwrap()
        };
        // Its top face down on the base's top (z 10..15), the holes lined up at (10, 10).
        let (lo, hi) = run(0.0);
        assert!((lo.z - 10.0).abs() < 1e-6 && (hi.z - 15.0).abs() < 1e-6, "{lo} {hi}");
        let centre_x = |lo: DVec3, hi: DVec3| (lo.x + hi.x) / 2.0;
        assert!((hi.x - lo.x - 50.0).abs() < 1e-6 && (hi.y - lo.y - 10.0).abs() < 1e-6, "lying flat along x: {lo} {hi}");
        // The hole at (10, 10): the lever reaches 5 one way and 45 the other from it.
        assert!(((lo.x - 10.0).abs() - 5.0).abs() < 1e-6 || ((hi.x - 10.0).abs() - 5.0).abs() < 1e-6, "{lo} {hi} (middle {})", centre_x(lo, hi));
        // Turned 90° about the hole: now along y, the hole still at (10, 10).
        let (lo, hi) = run(90.0);
        assert!((hi.x - lo.x - 10.0).abs() < 1e-6 && (hi.y - lo.y - 50.0).abs() < 1e-6, "turned to lie along y: {lo} {hi}");
        assert!((lo.x - 5.0).abs() < 1e-6 && (hi.x - 15.0).abs() < 1e-6, "{lo} {hi}");
    }

    fn run_imports(imports: Vec<ImportFeature>) -> Model {
        let timeline = (0..imports.len()).map(Feature::Import).collect();
        let mut b = Build::start(BuildInputs {
            sketches: vec![], extrudes: vec![], revolves: vec![], edge_features: vec![], copies: vec![], axis_features: vec![], plane_features: vec![], removes: vec![], holes: vec![],
            thread_features: vec![], shells: vec![], offset_faces: vec![], combines: vec![], splits: vec![], scales: vec![], moves: vec![], points: vec![], sweeps: vec![], imports, joints: vec![], components: vec![], timeline,
        });
        while b.step(std::time::Duration::MAX) {}
        b.finish()
    }

    #[test]
    fn step_and_stl_files_come_in_as_exact_bodies() {
        // A 20 × 10 × 5 block and, apart from it, a round shaft r 3 × 5.
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 10.0);
        circle(&mut a, 40.0, 5.0, 3.0);
        let made = build4(&[a], &[ex(0, vec![0, 1], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        let dir = std::env::temp_dir().join(format!("opencad-import-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (step, stl) = (dir.join("part.step"), dir.join("part.stl"));
        write_step(&step, &made.bodies.iter().collect::<Vec<_>>()).unwrap();
        let block: Vec<&Body> = made.bodies.iter().filter(|b| b.shape.volume() > 500.0).collect();
        crate::export::write(&stl, crate::export::Format::Stl, &[crate::export::Body { name: "block".into(), mesh: &block[0].mesh }]).unwrap();
        let import = |p: &Path| ImportFeature { name: "I".into(), source: "x".into(), brep: Blob::new(import_file(p).unwrap()), offset: [0.0; 3], angles: [0.0; 3] };
        let m = run_imports(vec![import(&step), import(&stl)]);
        assert_eq!(m.import_errors, vec![None, None]);
        let from = |k: usize| m.bodies.iter().filter(|b| b.id.feature == Feature::Import(k)).map(|b| b.shape.volume()).collect::<Vec<f64>>();
        // STEP keeps true cylinders: exact.
        let mut v = from(0);
        v.sort_by(f64::total_cmp);
        assert_eq!(v.len(), 2);
        assert!(close(v[0], PI * 9.0 * 5.0) && close(v[1], 1000.0), "{v:?}");
        // STL is triangles, but a block's triangles are its faces: exact too,
        // with each side's two triangles merged back into one face.
        assert!(close(from(1)[0], 1000.0), "{:?}", from(1));
        let stl_body = m.bodies.iter().find(|b| b.id.feature == Feature::Import(1)).unwrap();
        assert_eq!(stl_body.shape.faces().count(), 6);
        // Placed: the block turned 90° about Z through its middle (10, 5),
        // then moved 100 along X: 105..115 × −5..15, volume unchanged.
        let moved = ImportFeature { offset: [100.0, 0.0, 0.0], angles: [0.0, 0.0, 90.0], ..import(&stl) };
        assert_eq!(import_bounds(&moved.brep, moved.angles).map(|(lo, hi)| ((lo.x * 1e6).round() / 1e6, (hi.y * 1e6).round() / 1e6)), Some((5.0, 15.0)));
        let m = run_imports(vec![moved]);
        let (lo, hi) = exact_bounds(&m.bodies[0].shape).unwrap();
        assert!(lo.distance(DVec3::new(105.0, -5.0, 0.0)) < 1e-6 && hi.distance(DVec3::new(115.0, 15.0, 5.0)) < 1e-6, "{lo} {hi}");
        assert!(close(m.bodies[0].shape.volume(), 1000.0));
        // A mesh with a hole (one lone triangle) isn't a solid: refused, saying why.
        let open = dir.join("open.stl");
        std::fs::write(&open, "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n").unwrap();
        assert!(import_file(&open).unwrap_err().contains("holes"));
        assert_eq!(stl_triangles(&open), Some(1));
        assert_eq!(stl_triangles(&stl), Some(12), "a block: two triangles a side (binary)");
        // Not a part file, or not there: refused with a reason.
        assert!(import_file(&dir.join("nothing.step")).is_err());
        assert!(import_file(&dir.join("notes.txt")).unwrap_err().contains("STEP"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn run_sweeps(sketches: Vec<Sketch>, planes: Vec<PlaneFeature>, sweeps: Vec<SweepFeature>, timeline: Vec<Feature>) -> Model {
        let mut b = Build::start(BuildInputs {
            sketches, extrudes: vec![], revolves: vec![], edge_features: vec![], copies: vec![], axis_features: vec![], plane_features: planes, removes: vec![], holes: vec![],
            thread_features: vec![], shells: vec![], offset_faces: vec![], combines: vec![], splits: vec![], scales: vec![], moves: vec![], points: vec![], sweeps, imports: vec![], joints: vec![], components: vec![], timeline,
        });
        while b.step(std::time::Duration::MAX) {}
        b.finish()
    }

    #[test]
    fn a_sweep_carries_the_profile_the_length_of_its_path() {
        use crate::sketch::Entity;
        // A 10 × 10 square on YZ, centred on the origin.
        let mut prof = Sketch::new("P".into(), OriginPlane::YZ);
        rect(&mut prof, -5.0, -5.0, 5.0, 5.0);
        // Paths on XY from the origin: straight 30 along X, and a quarter circle of radius 20 bending toward +Y.
        let mut straight = Sketch::new("L".into(), OriginPlane::XY);
        straight.points.push(DVec2::new(30.0, 0.0));
        straight.entities.push(Entity::Line { a: 0, b: 1 });
        let mut bend = Sketch::new("A".into(), OriginPlane::XY);
        bend.points.extend([DVec2::new(0.0, 20.0), DVec2::new(20.0, 20.0)]);
        bend.entities.push(Entity::Arc { center: 1, a: 0, b: 2 });
        let sweep = |path_sketch: usize| SweepFeature { name: "S".into(), kind: SweepKind::Path { sketch: 0, profiles: vec![0], path_sketch, path: vec![0] }, op: Operation::NewBody };
        let m = run_sweeps(vec![prof.clone(), straight, bend], vec![], vec![sweep(1), sweep(2)], vec![Feature::Sketch(0), Feature::Sketch(1), Feature::Sketch(2), Feature::Sweep(0), Feature::Sweep(1)]);
        assert_eq!(m.sweep_errors, vec![None, None]);
        let vols: Vec<f64> = m.bodies.iter().map(|b| b.shape.volume()).collect();
        assert_eq!(vols.len(), 2);
        assert!(close(vols[0], 3000.0), "{}", vols[0]);
        // Pappus: area × the distance its centre travels, a quarter of a 20 circle.
        assert!(close(vols[1], 100.0 * std::f64::consts::FRAC_PI_2 * 20.0), "{}", vols[1]);
    }

    #[test]
    fn pipes_and_coils_have_their_exact_volumes() {
        use crate::model::CoilSection;
        use crate::sketch::Entity;
        let mut path = Sketch::new("L".into(), OriginPlane::XY);
        path.points.push(DVec2::new(30.0, 0.0));
        path.entities.push(Entity::Line { a: 0, b: 1 });
        let pipe = |thickness| SweepFeature { name: "P".into(), kind: SweepKind::Pipe { path_sketch: 0, path: vec![0], diameter: 4.0, thickness }, op: Operation::NewBody };
        let coil = |section| SweepFeature {
            name: "C".into(),
            kind: SweepKind::Coil { at: ExtrudeTarget::Origin(OriginPlane::XY), centre: [50.0, 0.0], diameter: 20.0, turns: 2.0, height: 20.0, section, size: 2.0 },
            op: Operation::NewBody,
        };
        let m = run_sweeps(vec![path], vec![], vec![pipe(0.0), pipe(0.5), coil(CoilSection::Circle), coil(CoilSection::Square)], vec![Feature::Sketch(0), Feature::Sweep(0), Feature::Sweep(1), Feature::Sweep(2), Feature::Sweep(3)]);
        assert_eq!(m.sweep_errors, vec![None, None, None, None]);
        let vol = |k: usize| m.bodies.iter().filter(|b| b.id.feature == Feature::Sweep(k)).map(|b| b.shape.volume()).sum::<f64>();
        use std::f64::consts::PI;
        assert!(close(vol(0), PI * 4.0 * 30.0), "solid pipe {}", vol(0));
        assert!(close(vol(1), PI * (4.0 - 1.5 * 1.5) * 30.0), "hollow pipe {}", vol(1));
        // A screw motion of a section in the plane through the axis: area × 2πR × turns.
        assert!((vol(2) - PI * 1.0 * 2.0 * PI * 10.0 * 2.0).abs() < 1e-3 * vol(2), "round coil {}", vol(2));
        assert!((vol(3) - 4.0 * 2.0 * PI * 10.0 * 2.0).abs() < 1e-3 * vol(3), "square coil {}", vol(3));
        // It stands on the plane, centred where asked, as tall as asked.
        let c = m.bodies.iter().find(|b| b.id.feature == Feature::Sweep(2)).unwrap();
        let (lo, hi) = bbox(c);
        assert!(((lo.x + hi.x) / 2.0 - 50.0).abs() < 0.5 && (hi.z - lo.z - 22.0).abs() < 0.1, "{lo} {hi}");
    }

    #[test]
    fn a_loft_between_a_small_and_a_big_square_is_a_frustum() {
        let mut low = Sketch::new("Low".into(), OriginPlane::XY);
        rect(&mut low, -5.0, -5.0, 5.0, 5.0);
        let up = PlaneFeature { name: "Up".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: true, offset: 20.0, anchor: [0.0; 2], extent: [0.0; 4], angle: None, kind: None };
        let mut high = Sketch::on_plane("High".into(), FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 20.0)), 0);
        rect(&mut high, -10.0, -10.0, 10.0, 10.0);
        let loft = |ruled| SweepFeature { name: "L".into(), kind: SweepKind::Loft { sections: vec![(0, 0), (1, 0)], ruled }, op: Operation::NewBody };
        let m = run_sweeps(vec![low, high], vec![up], vec![loft(true), loft(false)], vec![Feature::Sketch(0), Feature::Plane(0), Feature::Sketch(1), Feature::Sweep(0), Feature::Sweep(1)]);
        assert_eq!(m.sweep_errors, vec![None, None]);
        // h/3 · (A1 + A2 + √(A1·A2)).
        let frustum = 20.0 / 3.0 * (100.0 + 400.0 + 200.0);
        for b in &m.bodies {
            assert!(close(b.shape.volume(), frustum), "{} vs {frustum}", b.shape.volume());
        }
        // Fewer than two profiles is refused.
        let one = SweepFeature { kind: SweepKind::Loft { sections: vec![(0, 0)], ruled: false }, ..loft(false) };
        let mut low2 = Sketch::new("Low".into(), OriginPlane::XY);
        rect(&mut low2, -5.0, -5.0, 5.0, 5.0);
        let m = run_sweeps(vec![low2], vec![], vec![one], vec![Feature::Sketch(0), Feature::Sweep(0)]);
        assert!(m.sweep_errors[0].is_some());
    }

    #[test]
    fn body_properties_are_exact() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 10.0);
        let m = build4(&[a], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        let (v, area, c) = body_properties(&m.bodies[0].shape);
        assert!(close(v, 1000.0));
        assert!(close(area, 2.0 * (200.0 + 50.0 + 100.0)));
        assert!(c.distance(DVec3::new(10.0, 5.0, 2.5)) < 1e-9);
        assert_eq!(body_properties(&m.bodies[0].shape), (v, area, c), "remembered");
    }

    #[test]
    fn redefining_a_sketch_plane_moves_what_is_built_from_it() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let m = build4(&[a.clone()], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[], &timeline(1, 1));
        let (lo, hi) = bbox(&m.bodies[0]);
        assert!((hi.z - lo.z - 10.0).abs() < 1e-6, "up out of XY");
        // Onto XZ: the same rectangle, now standing up, extruded along Y.
        a.redefine_plane(&Sketch::new(String::new(), OriginPlane::XZ));
        let m = build4(&[a.clone()], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.sketch_errors, vec![None]);
        assert!(close(total(&m), 15000.0));
        let (lo, hi) = bbox(&m.bodies[0]);
        assert!((hi.y - lo.y - 10.0).abs() < 1e-6 && (hi.z - lo.z - 30.0).abs() < 1e-6, "{lo} {hi}");
        // Onto a side of another block, clicked at a spot: it lands centred
        // there, so its extrusion grows out of that face (and joins it).
        let mut base = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut base, 0.0, 0.0, 60.0, 40.0);
        let mut c = Sketch::new("C".into(), OriginPlane::XY);
        rect(&mut c, 100.0, 100.0, 110.0, 110.0); // far away at first
        let side = FacePlane::from_face(DVec3::NEG_Y, DVec3::new(0.0, 0.0, 0.0));
        c.redefine_onto_face_at(side, DVec3::new(30.0, 0.0, 20.0));
        let tl = vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1)];
        let m = build4(&[base, c], &[ex(0, vec![0], 40.0, Operation::NewBody), ex(1, vec![0], 5.0, Operation::Join)], &[], &tl);
        assert_eq!(m.errors, vec![None, None]);
        assert_eq!(m.bodies.len(), 1, "joined to the block");
        assert!(close(total(&m), 60.0 * 40.0 * 40.0 + 10.0 * 10.0 * 5.0), "{}", total(&m));
        let (lo, hi) = bbox(&m.bodies[0]);
        assert!((lo.y + 5.0).abs() < 1e-6 && hi.y > 39.0, "it sticks out of the front face: {lo} {hi}");
        // Onto the top of what it makes itself: there's no such face yet when
        // the sketch is built — an error the app refuses.
        let top = FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0));
        let mut b = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut b, 0.0, 0.0, 50.0, 30.0);
        b.redefine_plane(&Sketch::on_face(String::new(), top));
        let m = build4(&[b], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert!(m.sketch_errors[0].is_some() || m.error_count() > 0);
    }

    #[test]
    fn plate_with_hole_is_exact() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 0.0, 50.0, 30.0);
        circle(&mut sk, 25.0, 15.0, 8.0);
        let ring = find_regions(&sk).iter().position(|r| r.holes.len() == 1).unwrap();
        let m = build4(&[sk], &[ex(0, vec![ring], 10.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        assert_eq!(m.bodies.len(), 1);
        // Exact cylinder, not a 96-gon: matches π r² to 1e-6.
        assert!(close(m.bodies[0].shape.volume(), 15000.0 - PI * 64.0 * 10.0), "{}", total(&m));
        assert!(closed(&m.bodies[0].mesh), "display mesh is watertight");
    }

    #[test]
    fn belt_uses_true_arcs() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        circle(&mut sk, 0.0, 0.0, 10.0);
        circle(&mut sk, 40.0, 0.0, 10.0);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for y in [10.0, -10.0] {
            ed.click(&mut sk, free(-20.0, y));
            ed.click(&mut sk, free(60.0, y));
            ed.cancel();
        }
        let m = build4(&[sk], &[ex(0, vec![0, 1, 2], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.bodies.len(), 1, "one body, no seams");
        assert!(close(total(&m), (800.0 + PI * 100.0) * 5.0), "{}", total(&m));
    }

    #[test]
    fn negative_distance_and_other_planes() {
        for plane in OriginPlane::ALL {
            let mut sk = Sketch::new("S".into(), plane);
            rect(&mut sk, 0.0, 0.0, 20.0, 10.0);
            let m = build4(&[sk], &[ex(0, vec![0], -7.0, Operation::NewBody)], &[], &timeline(1, 1));
            assert!(close(total(&m), 1400.0), "{plane:?}: {}", total(&m));
        }
    }

    /// Base plate 50×30×10 on XY, then a second XY sketch with the given rect,
    /// extruded with `op`.
    fn two_step(r: (f64, f64, f64, f64), distance: f64, op: Operation) -> Model {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, r.0, r.1, r.2, r.3);
        build4(
            &[a, b],
            &[ex(0, vec![0], 10.0, Operation::NewBody), ex(1, vec![0], distance, op)],
            &[],
            &timeline(2, 2),
        )
    }

    #[test]
    fn join_merges_overlapping_and_touching_bodies() {
        // Overlapping: a 10×10×20 post through the plate.
        let m = two_step((20.0, 10.0, 30.0, 20.0), 20.0, Operation::Join);
        assert_eq!(m.bodies.len(), 1);
        assert!(close(total(&m), 15000.0 + 1000.0), "{}", total(&m));
        // Touching only along a face: a block next to the plate.
        let m = two_step((50.0, 0.0, 60.0, 30.0), 10.0, Operation::Join);
        assert_eq!(m.bodies.len(), 1, "touching bodies join too");
        assert!(close(total(&m), 18000.0));
        // Far away: nothing to join, so it becomes its own body.
        let m = two_step((100.0, 0.0, 110.0, 10.0), 10.0, Operation::Join);
        assert_eq!(m.bodies.len(), 2);
    }

    #[test]
    fn new_body_keeps_bodies_separate() {
        let m = two_step((20.0, 10.0, 30.0, 20.0), 20.0, Operation::NewBody);
        assert_eq!(m.bodies.len(), 2);
    }

    #[test]
    fn cut_removes_and_can_split() {
        // A 10 mm-deep pocket.
        let m = two_step((10.0, 10.0, 20.0, 20.0), 10.0, Operation::Cut);
        assert_eq!(m.bodies.len(), 1);
        assert!(close(total(&m), 15000.0 - 1000.0));
        // A slot across the whole plate splits it into two bodies.
        let m = two_step((20.0, -5.0, 30.0, 35.0), 10.0, Operation::Cut);
        assert_eq!(m.bodies.len(), 2, "split into two pieces");
        assert!(close(total(&m), 15000.0 - 3000.0));
        // Cutting everything away leaves nothing.
        let m = two_step((-5.0, -5.0, 55.0, 35.0), 10.0, Operation::Cut);
        assert!(m.bodies.is_empty());
    }

    #[test]
    fn intersect_keeps_only_the_overlap() {
        let m = two_step((-5.0, -5.0, 25.0, 35.0), 10.0, Operation::Intersect);
        assert_eq!(m.bodies.len(), 1);
        assert!(close(total(&m), 25.0 * 30.0 * 10.0));
    }

    fn ids(m: &Model) -> Vec<BodyId> {
        m.bodies.iter().map(|b| b.id).collect()
    }

    fn id(feature: Feature, piece: usize) -> BodyId {
        BodyId { feature, piece }
    }

    #[test]
    fn bodies_are_identified_by_the_feature_that_made_them() {
        // A plate, then a separate block far away: two new bodies.
        let m = two_step((100.0, 0.0, 120.0, 10.0), 5.0, Operation::NewBody);
        assert_eq!(ids(&m), vec![id(Feature::Extrude(0), 0), id(Feature::Extrude(1), 0)]);
    }

    #[test]
    fn join_keeps_the_older_bodys_identity_and_cut_keeps_ids_through_a_split() {
        let m = two_step((20.0, 10.0, 30.0, 20.0), 20.0, Operation::Join);
        assert_eq!(ids(&m), vec![id(Feature::Extrude(0), 0)], "the post joined the plate, which keeps its id");
        // A join that touches nothing is a new body named after its own feature.
        let m = two_step((100.0, 0.0, 120.0, 10.0), 5.0, Operation::Join);
        assert_eq!(ids(&m), vec![id(Feature::Extrude(0), 0), id(Feature::Extrude(1), 0)]);
        // A slot across the plate splits it: both pieces descend from the plate, numbered.
        let m = two_step((20.0, -5.0, 30.0, 35.0), 10.0, Operation::Cut);
        assert_eq!(ids(&m), vec![id(Feature::Extrude(0), 0), id(Feature::Extrude(0), 1)]);
    }

    #[test]
    fn fillets_and_intersects_keep_body_ids() {
        let m = plate_with(10.0, Some((EdgeKind::Fillet, 2.0, picked_top_front(10.0))));
        assert_eq!(ids(&m), vec![id(Feature::Extrude(0), 0)]);
        let m = two_step((-5.0, -5.0, 25.0, 35.0), 10.0, Operation::Intersect);
        assert_eq!(ids(&m), vec![id(Feature::Extrude(0), 0)]);
    }

    #[test]
    fn building_only_part_of_the_timeline_rolls_the_design_back() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 20.0, 10.0, 30.0, 20.0);
        let exs = [ex(0, vec![0], 10.0, Operation::NewBody), ex(1, vec![0], 10.0, Operation::Cut)];
        let full = [Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1)];
        let all = build4(&[a.clone(), b.clone()], &exs, &[], &full);
        assert!(close(total(&all), 15000.0 - 1000.0));
        // Rolled back to before the cut: the plate is whole, and the cut has no error.
        let back = build4(&[a, b], &exs, &[], &full[..3]);
        assert!(close(total(&back), 15000.0));
        assert_eq!(back.errors, vec![None, None]);
        assert_eq!(ids(&back), vec![id(Feature::Extrude(0), 0)]);
    }

    #[test]
    fn each_feature_remembers_the_body_it_touched() {
        let plate = id(Feature::Extrude(0), 0);
        let m = two_step((100.0, 0.0, 120.0, 10.0), 5.0, Operation::NewBody);
        assert_eq!(m.touched[&Feature::Extrude(0)], plate);
        assert_eq!(m.touched[&Feature::Extrude(1)], id(Feature::Extrude(1), 0));
        let m = two_step((20.0, 10.0, 30.0, 20.0), 20.0, Operation::Join);
        assert_eq!(m.touched[&Feature::Extrude(1)], plate, "joined into the plate");
        let m = two_step((10.0, 10.0, 20.0, 20.0), 10.0, Operation::Cut);
        assert_eq!(m.touched[&Feature::Extrude(1)], plate, "cut the plate");
        // A cut that misses everything touched nothing.
        let m = two_step((100.0, 100.0, 110.0, 110.0), 10.0, Operation::Cut);
        assert!(!m.touched.contains_key(&Feature::Extrude(1)));
        let m = plate_with(10.0, Some((EdgeKind::Fillet, 2.0, picked_top_front(10.0))));
        assert_eq!(m.touched[&Feature::Edge(0)], plate);
    }

    #[test]
    fn a_broken_feature_is_skipped_not_fatal() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let m = build4(
            &[a],
            &[ex(0, vec![0], 10.0, Operation::NewBody), ex(0, vec![7], 5.0, Operation::Join)],
            &[],
            &timeline(1, 2),
        );
        assert!(m.errors[0].is_none() && m.errors[1].is_some());
        assert_eq!(m.bodies.len(), 1, "earlier body still there");
        assert!(close(total(&m), 15000.0));
    }

    /// A 50×30×H plate with an optional edge feature on the given edge reference.
    fn plate_with(height: f64, edge: Option<(EdgeKind, f64, EdgeRef)>) -> Model {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let features: Vec<EdgeFeature> = edge
            .into_iter()
            .map(|(kind, size, r)| EdgeFeature { name: "F".into(), kind, size, size2: None, angle: None, edges: vec![r] })
            .collect();
        let mut tl = timeline(1, 1);
        if !features.is_empty() {
            tl.push(Feature::Edge(0));
        }
        build4(&[a], &[ex(0, vec![0], height, Operation::NewBody)], &features, &tl)
    }

    /// The top-front edge of the plate (runs along X at y = 0), as an older
    /// file would store it (no face directions).
    fn top_front(height: f64) -> EdgeRef {
        EdgeRef { mid: DVec3::new(25.0, 0.0, height), dir: DVec3::X, ..Default::default() }
    }

    #[test]
    fn box_offers_its_twelve_edges_and_cylinder_only_its_two_circles() {
        let m = plate_with(10.0, None);
        assert_eq!(m.bodies[0].edges.len(), 12);
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        circle(&mut sk, 0.0, 0.0, 10.0);
        let c = build4(&[sk], &[ex(0, vec![0], 20.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(c.bodies[0].edges.len(), 2, "top and bottom circles; the seam isn't offered");
    }

    #[test]
    fn fillet_and_chamfer_remove_the_exact_volume() {
        let (r, len) = (2.0, 50.0);
        let m = plate_with(10.0, Some((EdgeKind::Fillet, r, top_front(10.0))));
        assert_eq!(m.edge_errors, vec![None]);
        assert!(close(total(&m), 15000.0 - r * r * (1.0 - PI / 4.0) * len), "{}", total(&m));
        let m = plate_with(10.0, Some((EdgeKind::Chamfer, r, top_front(10.0))));
        assert!(close(total(&m), 15000.0 - r * r / 2.0 * len), "{}", total(&m));
        assert!(closed(&m.bodies[0].mesh));
    }

    #[test]
    fn impossible_fillet_is_an_error_not_a_crash() {
        let m = plate_with(10.0, Some((EdgeKind::Fillet, 25.0, top_front(10.0))));
        assert!(m.edge_errors[0].is_some(), "radius larger than the plate is thick");
        assert!(close(total(&m), 15000.0), "the plate is still there, unrounded");
    }

    /// The top-front edge of a freshly built plate, the way the app picks it (with its faces).
    fn picked_top_front(height: f64) -> EdgeRef {
        let base = plate_with(height, None);
        base.bodies[0]
            .edges
            .iter()
            .map(|e| e.at)
            .find(|r| r.dir.dot(DVec3::X).abs() > 0.99 && r.mid.y.abs() < 1e-6 && (r.mid.z - height).abs() < 1e-6)
            .expect("top-front edge is offered")
    }

    fn two_distance_chamfer(d1: f64, d2: f64, first_face: DVec3) -> Model {
        let mut r = picked_top_front(10.0);
        let other = if r.faces[0].dot(first_face) > 0.99 { r.faces[1] } else { r.faces[0] };
        r.faces = [first_face, other];
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let f = EdgeFeature { name: "C".into(), kind: EdgeKind::Chamfer, size: d1, size2: Some(d2), angle: None, edges: vec![r] };
        let mut tl = timeline(1, 1);
        tl.push(Feature::Edge(0));
        build4(&[a], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[f], &tl)
    }

    #[test]
    fn a_tangent_chain_and_a_face_pick_take_the_rounded_outline() {
        // The plate with its four upright edges rounded: the top outline is
        // 4 lines and 4 arcs, each running smoothly into the next.
        let base = plate_with(10.0, None);
        let upright: Vec<EdgeRef> = base.bodies[0].edges.iter().map(|e| e.at).filter(|r| r.dir.dot(DVec3::Z).abs() > 0.99).collect();
        assert_eq!(upright.len(), 4);
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let f = EdgeFeature { name: "F".into(), kind: EdgeKind::Fillet, size: 3.0, size2: None, angle: None, edges: upright };
        let mut tl = timeline(1, 1);
        tl.push(Feature::Edge(0));
        let m = build4(&[a], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[f], &tl);
        let b = &m.bodies[0];
        let top = |i: &usize| (b.edges[*i].at.mid.z - 10.0).abs() < 1e-6;
        let start = (0..b.edges.len()).find(|i| top(i) && b.edges[*i].at.dir.dot(DVec3::X).abs() > 0.99).expect("a straight top edge");
        let chain = b.tangent_chain(start);
        assert_eq!(chain.len(), 8, "{chain:?}");
        assert!(chain.iter().all(top), "only the top outline");
        // The top face's edges are the same eight.
        let face = (0..b.faces.len()).find(|&f| b.face_tris(f).all(|t| t.iter().all(|p| (p.z - 10.0).abs() < 1e-9))).unwrap();
        let mut around = b.face_edges(face);
        around.sort();
        let mut chain = chain;
        chain.sort();
        assert_eq!(around, chain);
        // A plain box's edges meet at corners: a chain of one.
        let plain = &base.bodies[0];
        assert_eq!(plain.tangent_chain(0), vec![0]);
    }

    #[test]
    fn press_pull_finds_the_fillet_a_rounded_face_came_from() {
        let edge = picked_top_front(10.0);
        let fillets = [
            EdgeFeature { name: "Other".into(), kind: EdgeKind::Fillet, size: 3.0, size2: None, angle: None, edges: vec![edge] },
            EdgeFeature { name: "F".into(), kind: EdgeKind::Fillet, size: 2.0, size2: None, angle: None, edges: vec![edge] },
        ];
        let m = plate_with(10.0, Some((EdgeKind::Fillet, 2.0, edge)));
        let b = &m.bodies[0];
        let round = (0..b.faces.len()).find(|&f| matches!(b.faces[f], Geom::Cylinder { .. })).expect("the round");
        assert_eq!(fillet_for_face(&fillets, b, round), Some(1), "same radius, along that edge");
        let flat = (0..b.faces.len()).find(|&f| matches!(b.faces[f], Geom::Plane { .. })).unwrap();
        assert_eq!(fillet_for_face(&fillets, b, flat), None);
        // A Ø4 hole (radius 2 too) isn't a fillet.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 0.0, 50.0, 30.0);
        circle(&mut sk, 25.0, 15.0, 2.0);
        let ring = find_regions(&sk).iter().position(|r| r.holes.len() == 1).unwrap();
        let m = build4(&[sk], &[ex(0, vec![ring], 10.0, Operation::NewBody)], &[], &timeline(1, 1));
        let b = &m.bodies[0];
        let hole = (0..b.faces.len()).find(|&f| matches!(b.faces[f], Geom::Cylinder { .. })).unwrap();
        assert_eq!(fillet_for_face(&fillets, b, hole), None);
    }

    #[test]
    fn a_chamfer_by_distance_and_angle_cuts_its_triangle() {
        // 2 mm along the top face, 30° down from it: the other leg is 2 tan 30°.
        let build = |first: DVec3, d: f64, a: f64| {
            let mut r = picked_top_front(10.0);
            let other = if r.faces[0].dot(first) > 0.99 { r.faces[1] } else { r.faces[0] };
            r.faces = [first, other];
            let mut s = Sketch::new("A".into(), OriginPlane::XY);
            rect(&mut s, 0.0, 0.0, 50.0, 30.0);
            let f = EdgeFeature { name: "C".into(), kind: EdgeKind::Chamfer, size: d, size2: None, angle: Some(a), edges: vec![r] };
            let mut tl = timeline(1, 1);
            tl.push(Feature::Edge(0));
            build4(&[s], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[f], &tl)
        };
        let m = build(DVec3::Z, 2.0, 30.0);
        assert_eq!(m.edge_errors, vec![None]);
        let want = 0.5 * 2.0 * (2.0 * 30f64.to_radians().tan()) * 50.0;
        assert!(close(15000.0 - total(&m), want), "{} vs {want}", 15000.0 - total(&m));
        // At 45° it's the equal-distance chamfer.
        assert!(close(15000.0 - total(&build(DVec3::Z, 2.0, 45.0)), 0.5 * 4.0 * 50.0));
        // Measured on the front face instead: the legs swap, the same amount.
        assert!(close(15000.0 - total(&build(DVec3::NEG_Y, 2.0, 30.0)), want));
        assert!(build(DVec3::Z, 2.0, 95.0).edge_errors[0].is_some(), "an angle past 90°");
    }

    #[test]
    fn a_fillet_moved_before_its_body_is_an_error_the_app_can_refuse() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let f = EdgeFeature { name: "F".into(), kind: EdgeKind::Fillet, size: 2.0, size2: None, angle: None, edges: vec![picked_top_front(10.0)] };
        let mut tl = timeline(1, 1);
        tl.push(Feature::Edge(0));
        let ok = build4(&[a.clone()], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[f.clone()], &tl);
        assert_eq!(ok.error_count(), 0);
        let swapped = crate::doc::moved(&tl, 2, 0).unwrap();
        let bad = build4(&[a], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[f], &swapped);
        assert!(bad.error_count() > 0);
    }

    #[test]
    fn suppressing_a_feature_leaves_it_unbuilt() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let mut tl = timeline(1, 1);
        tl.retain(|f| *f != Feature::Extrude(0));
        let m = build4(&[a], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[], &tl);
        assert!(m.bodies.is_empty());
    }

    #[test]
    fn two_distance_chamfer_removes_d1_d2_over_two_times_length() {
        // 2 on the top face, 5 on the front face: removes 2·5/2 × 50 = 250.
        let m = two_distance_chamfer(2.0, 5.0, DVec3::Z);
        assert_eq!(m.edge_errors, vec![None]);
        assert!(close(total(&m), 15000.0 - 250.0), "{}", total(&m));
        // The first distance is on the face the pick faced: the top face's front edge moved back 2.
        let top_front_y = m.bodies[0].mesh.positions.iter().filter(|p| (p.z - 10.0).abs() < 1e-4).map(|p| p.y).fold(f32::MAX, f32::min);
        assert!((top_front_y - 2.0).abs() < 1e-4, "top face starts at y = {top_front_y}");
        // Swapping which face is first swaps where each distance goes.
        let m = two_distance_chamfer(2.0, 5.0, DVec3::NEG_Y);
        assert!(close(total(&m), 15000.0 - 250.0), "swapped: {}", total(&m));
        let top_front_y = m.bodies[0].mesh.positions.iter().filter(|p| (p.z - 10.0).abs() < 1e-4).map(|p| p.y).fold(f32::MAX, f32::min);
        assert!((top_front_y - 5.0).abs() < 1e-4, "top face starts at y = {top_front_y}");
    }

    #[test]
    fn equal_second_distance_is_an_ordinary_chamfer_and_bad_ones_are_errors() {
        let m = two_distance_chamfer(3.0, 3.0, DVec3::Z);
        assert!(close(total(&m), 15000.0 - 9.0 / 2.0 * 50.0), "{}", total(&m));
        let m = two_distance_chamfer(2.0, 0.0, DVec3::Z);
        assert!(m.edge_errors[0].is_some());
        let m = two_distance_chamfer(2.0, 25.0, DVec3::Z);
        assert!(m.edge_errors[0].is_some(), "25 mm doesn't fit on a 10 mm tall face");
        assert!(close(total(&m), 15000.0), "the plate is still there");
    }

    #[test]
    fn picked_edge_survives_a_big_change_and_stays_the_right_edge() {
        // Pick the top-front edge the way the app does: from the built body.
        let base = plate_with(10.0, None);
        let picked = base.bodies[0]
            .edges
            .iter()
            .map(|e| e.at)
            .find(|r| r.dir.dot(DVec3::X).abs() > 0.99 && r.mid.y.abs() < 1e-6 && (r.mid.z - 10.0).abs() < 1e-6)
            .expect("top-front edge is offered");
        assert!(same_faces(&picked.faces, &[DVec3::Z, DVec3::NEG_Y]), "{:?}", picked.faces);
        // The plate becomes 100 thick: the old position is now nearer the
        // *bottom*-front edge (10 away) than the top-front one (90 away).
        let m = plate_with(100.0, Some((EdgeKind::Fillet, 2.0, picked)));
        assert_eq!(m.edge_errors, vec![None]);
        assert!(close(total(&m), 150000.0 - 4.0 * (1.0 - PI / 4.0) * 50.0));
        // And it rounded the top: no corner left at the top-front.
        let mesh = &m.bodies[0].mesh;
        let corner_left = mesh.positions.iter().any(|p| p.y.abs() < 1e-4 && (p.z - 100.0).abs() < 1e-4);
        assert!(!corner_left, "the top-front edge was rounded, not the bottom one");
    }

    #[test]
    fn edge_is_found_again_after_a_dimension_change() {
        // Picked when the plate was 10 thick; the plate is now 12 thick.
        let m = plate_with(12.0, Some((EdgeKind::Fillet, 2.0, top_front(10.0))));
        assert_eq!(m.edge_errors, vec![None]);
        assert!(close(total(&m), 18000.0 - 4.0 * (1.0 - PI / 4.0) * 50.0));
        // Something nowhere near any edge is reported, not guessed.
        let far = EdgeRef { mid: DVec3::new(500.0, 500.0, 500.0), dir: DVec3::X, ..Default::default() };
        let m = plate_with(10.0, Some((EdgeKind::Fillet, 2.0, far)));
        assert!(m.edge_errors[0].is_some());
    }

    /// Two separate rectangles extruded as one New Body feature: pieces 0 and 1.
    fn two_blocks(removes: &[RemoveFeature], tl: &[Feature]) -> Model {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 0.0, 10.0, 10.0);
        rect(&mut sk, 20.0, 0.0, 50.0, 10.0);
        build_all(&[sk], &[ex(0, vec![0, 1], 5.0, Operation::NewBody)], &[], &[], &[], &[], &[], removes, &[], &[], &[], tl)
    }

    #[test]
    fn remove_takes_one_body_out_and_leaves_the_rest() {
        let all = two_blocks(&[], &timeline(1, 1));
        assert_eq!(all.bodies.len(), 2);
        assert!(close(total(&all), 500.0 + 1500.0));
        // Remove the big one (whichever piece it is).
        let big = all.bodies.iter().find(|b| close(b.shape.volume(), 1500.0)).unwrap().id;
        let rf = RemoveFeature { name: "Remove1".into(), bodies: vec![big] };
        let mut tl = timeline(1, 1);
        tl.push(Feature::Remove(0));
        let m = two_blocks(std::slice::from_ref(&rf), &tl);
        assert_eq!(m.remove_errors, vec![None]);
        assert_eq!(m.bodies.len(), 1);
        assert!(close(total(&m), 500.0));
        // Before the extrude that makes it, there's nothing to remove: an error.
        let early = two_blocks(std::slice::from_ref(&rf), &[Feature::Sketch(0), Feature::Remove(0), Feature::Extrude(0)]);
        assert_eq!(early.error_count(), 1);
        assert!(early.remove_errors[0].as_deref().is_some_and(|e| e.contains("gone")));
        assert_eq!(early.bodies.len(), 2, "the rest still builds");
    }

    #[test]
    fn a_spline_and_a_line_close_a_profile_that_extrudes() {
        // A spline arching over (0,0)…(40,0), closed by the line along the bottom.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Spline);
        for (x, y) in [(0.0, 0.0), (10.0, 12.0), (30.0, 12.0), (40.0, 0.0)] {
            ed.click(&mut sk, free(x, y));
        }
        ed.finish_spline(&mut sk);
        let (a, b) = (sk.splines[0][0], sk.splines[0][3]);
        sk.entities.push(crate::sketch::Entity::Line { a, b });
        let regions = find_regions(&sk);
        assert_eq!(regions.len(), 1, "one closed profile");
        let area = regions[0].area();
        let m = build4(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        let v = total(&m);
        // The solid follows the smooth curve; the profile's polygon is its
        // sampled version, so they agree closely but not exactly.
        assert!((v - area * 5.0).abs() < 1e-3 * area * 5.0, "volume {v} vs {}", area * 5.0);
    }

    #[test]
    fn a_closed_spline_is_a_profile_on_its_own() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Spline);
        for (x, y) in [(0.0, 0.0), (20.0, -5.0), (30.0, 10.0), (10.0, 18.0)] {
            ed.click(&mut sk, free(x, y));
        }
        // Back on the first point: closed, and the tool is done.
        ed.click(&mut sk, free(0.0, 0.0));
        assert!(!ed.busy());
        let list = &sk.splines[0];
        assert_eq!((list.len(), list.first(), list.last()), (5, list.last(), list.first()));
        let curve = sk.spline_curve(0);
        // Smooth where it closes: the direction in matches the direction out.
        let (d_in, d_out) = ((curve[curve.len() - 1] - curve[curve.len() - 2]).normalize(), (curve[1] - curve[0]).normalize());
        assert!(d_in.dot(d_out) > 0.99, "a corner where it closes: {d_in} vs {d_out}");
        let regions = find_regions(&sk);
        assert_eq!(regions.len(), 1);
        let area = regions[0].area();
        let m = build4(&[sk], &[ex(0, vec![0], 4.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        assert!((total(&m) - area * 4.0).abs() < 1e-3 * area * 4.0, "{} vs {}", total(&m), area * 4.0);
    }

    /// A 10 × 10 square at x 0..10, y 0..10 extruded To Object.
    fn square_to(target: ExtrudeTarget, sketch_plane: Option<usize>, planes: &[crate::model::PlaneFeature], base: bool) -> Model {
        use crate::model::Extent;
        let mut sketches = Vec::new();
        let mut extrudes = Vec::new();
        let mut tl = Vec::new();
        if base {
            // A 50 × 30 × 10 plate at the origin first.
            let mut b = Sketch::new("B".into(), OriginPlane::XY);
            rect(&mut b, 0.0, 0.0, 50.0, 30.0);
            sketches.push(b);
            extrudes.push(ex(0, vec![0], 10.0, Operation::NewBody));
            tl.extend([Feature::Sketch(0), Feature::Extrude(0)]);
        }
        for k in 0..planes.len() {
            tl.push(Feature::Plane(k));
        }
        let mut sk = match sketch_plane {
            Some(k) => Sketch::on_plane("S".into(), FacePlane::from_face(DVec3::Z, DVec3::ZERO), k),
            None => Sketch::new("S".into(), OriginPlane::XY),
        };
        rect(&mut sk, 0.0, 0.0, 10.0, 10.0);
        let s = sketches.len();
        sketches.push(sk);
        let mut e = ex(s, vec![0], 1.0, Operation::NewBody);
        (e.extent, e.target) = (Extent::ToObject, Some(target));
        extrudes.push(e);
        tl.extend([Feature::Sketch(s), Feature::Extrude(extrudes.len() - 1)]);
        build_all(&sketches, &extrudes, &[], &[], &[], &[], planes, &[], &[], &[], &[], &tl)
    }

    fn offset_plane(z: f64) -> crate::model::PlaneFeature {
        crate::model::PlaneFeature { name: "P".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: true, offset: z, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None, kind: None }
    }

    #[test]
    fn to_object_runs_up_to_a_plane_or_down_to_one() {
        // Up from the XY plane to a construction plane at z = 25: 10 × 10 × 25.
        let m = square_to(ExtrudeTarget::Plane(0), None, &[offset_plane(25.0)], false);
        assert_eq!(m.errors, vec![None]);
        assert!(close(total(&m), 2500.0), "{}", total(&m));
        let (lo, hi) = box_of(&m.bodies[0]);
        assert!(lo.z.abs() < 1e-6 && (hi.z - 25.0).abs() < 1e-6);
        // Down from a sketch on a plane at z = 20 to the XY plane: 10 × 10 × 20.
        let m = square_to(ExtrudeTarget::Origin(OriginPlane::XY), Some(0), &[offset_plane(20.0)], false);
        assert_eq!(m.errors, vec![None]);
        assert!(close(total(&m), 2000.0), "{}", total(&m));
        // A plane alongside the extrude never gets reached.
        let m = square_to(ExtrudeTarget::Origin(OriginPlane::XZ), None, &[], false);
        assert!(m.errors[0].as_deref().is_some_and(|e| e.contains("alongside")), "{:?}", m.errors);
    }

    #[test]
    fn to_object_stops_at_a_body_face_found_again() {
        // From a plane at z = 30 down to the plate's top face (z = 10): 10 × 10 × 20,
        // joined onto the plate as a separate body.
        let top = FacePlane::from_face(DVec3::Z, DVec3::new(25.0, 15.0, 10.0));
        let anchor = [DVec3::new(25.0, 15.0, 10.0).dot(top.u), DVec3::new(25.0, 15.0, 10.0).dot(top.v)];
        let m = square_to(ExtrudeTarget::Face { face: top, anchor }, Some(0), &[offset_plane(30.0)], true);
        assert_eq!(m.errors, vec![None, None]);
        assert!(close(total(&m), 15000.0 + 2000.0), "{}", total(&m));
    }

    fn angled(hinge: Hinge, degrees: f64) -> PlaneFeature {
        PlaneFeature { name: "P".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: false, offset: 0.0, anchor: [0.0; 2], extent: [0.0; 4], angle: Some(PlaneAngle { hinge, degrees }), kind: None }
    }

    #[test]
    fn a_plane_at_angle_about_an_origin_axis_slants_a_to_object_extrude() {
        use crate::model::{Extent, WorldAxis};
        // 30° about X from the XY plane: z = y·tan 30°. A 10 × 10 square at y 10..20
        // runs up to it: 10 × ∫ y tan30 dy over 10..20 = 1500 tan 30°.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 10.0, 10.0, 20.0);
        let mut e = ex(0, vec![0], 1.0, Operation::NewBody);
        (e.extent, e.target) = (Extent::ToObject, Some(ExtrudeTarget::Plane(0)));
        let planes = [angled(Hinge::World(WorldAxis::X), 30.0)];
        let m = build_all(&[sk], &[e], &[], &[], &[], &[], &planes, &[], &[], &[], &[], &[Feature::Plane(0), Feature::Sketch(0), Feature::Extrude(0)]);
        assert_eq!((m.plane_errors.clone(), m.errors.clone()), (vec![None], vec![None]));
        let n = m.planes[0].unwrap().normal();
        assert!(n.distance(DVec3::new(0.0, -0.5, 30f64.to_radians().cos())) < 1e-12, "{n}");
        let want = 1500.0 * 30f64.to_radians().tan();
        assert!(close(total(&m), want), "{} vs {want}", total(&m));
    }

    #[test]
    fn a_plane_at_angle_follows_its_sketch_line_edge_or_axis() {
        use crate::model::AxisFeature;
        // A sketch line along Y at x = 4 on XY; 90° stands the plane up as x = 4.
        let mut sk = Sketch::new("L".into(), OriginPlane::XY);
        sk.points.extend([DVec2::new(4.0, 0.0), DVec2::new(4.0, 10.0)]);
        let n = sk.points.len();
        sk.entities.push(crate::sketch::Entity::Line { a: n - 2, b: n - 1 });
        let line = sk.entities.len() - 1;
        let planes = [angled(Hinge::SketchLine { sketch: 0, line }, 90.0)];
        let m = build_all(&[sk], &[], &[], &[], &[], &[], &planes, &[], &[], &[], &[], &[Feature::Sketch(0), Feature::Plane(0)]);
        assert_eq!(m.plane_errors, vec![None]);
        let f = m.planes[0].unwrap();
        assert!(f.normal().cross(DVec3::X).length() < 1e-12 && (f.origin - DVec3::new(4.0, 5.0, 0.0)).length() < 1e-9, "{f:?}");

        // A 50 × 30 × h plate; its top front edge (along X at y 0, z h), between
        // the top (0°) and front faces. At 45° the plane leans out over the front.
        let edge = EdgeRef { mid: DVec3::new(25.0, 0.0, 10.0), dir: DVec3::X, faces: [DVec3::Z, DVec3::NEG_Y], follow: None };
        let axes = [AxisFeature { name: "A".into(), edge, kind: None }];
        let planes = [angled(Hinge::Edge(edge), 45.0), angled(Hinge::Axis(0), 0.0)];
        for h in [10.0, 20.0] {
            let mut b = Sketch::new("B".into(), OriginPlane::XY);
            rect(&mut b, 0.0, 0.0, 50.0, 30.0);
            let t = [Feature::Sketch(0), Feature::Extrude(0), Feature::Axis(0), Feature::Plane(0), Feature::Plane(1)];
            let m = build_all(&[b], &[ex(0, vec![0], h, Operation::NewBody)], &[], &[], &[], &axes, &planes, &[], &[], &[], &[], &t);
            assert_eq!(m.plane_errors, vec![None, None], "h {h}");
            let (lean, flat) = (m.planes[0].unwrap(), m.planes[1].unwrap());
            let s = 0.5f64.sqrt();
            assert!(lean.normal().distance(DVec3::new(0.0, -s, s)) < 1e-9, "{}", lean.normal());
            assert!(flat.normal().distance(DVec3::Z) < 1e-9, "0° lies on the top face");
            for f in [lean, flat] {
                assert!((f.origin.y).abs() < 1e-9 && (f.origin.z - h).abs() < 1e-9, "the plane rides on the edge at h {h}: {:?}", f.origin);
            }
        }
        // A line that's gone is an error, not a crash.
        let m = build_all(&[Sketch::new("E".into(), OriginPlane::XY)], &[], &[], &[], &[], &[], &[angled(Hinge::SketchLine { sketch: 0, line: 0 }, 10.0)], &[], &[], &[], &[], &[Feature::Sketch(0), Feature::Plane(0)]);
        assert!(m.plane_errors[0].is_some() && m.planes[0].is_none());
    }

    #[test]
    fn to_object_cuts_the_end_along_a_slanted_plane() {
        // The plane z = 10 + y/2: over the square (y 0..10) the top runs from
        // z 10 to 15, so the volume is 100 × 12.5.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 0.0, 10.0, 10.0);
        let mut e = ex(0, vec![0], 1.0, Operation::NewBody);
        e.extent = crate::model::Extent::ToObject;
        let n = DVec3::new(0.0, -0.5, 1.0).normalize();
        let (shape, _) = tool(&[sk], &[None], &e, Some((DVec3::new(0.0, 0.0, 10.0), n))).unwrap();
        assert!(close(shape.volume(), 1250.0), "{}", shape.volume());
    }

    /// A 50 × 30 × `h` plate (sketch 0, extrude 0), then sketch 1 (if given)
    /// and the holes, in that order.
    fn drilled(h: f64, sk: Option<Sketch>, holes: &[HoleFeature], planes: &[PlaneFeature]) -> Model {
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 0.0, 0.0, 50.0, 30.0);
        let mut sketches = vec![b];
        let mut tl = vec![Feature::Sketch(0), Feature::Extrude(0)];
        tl.extend((0..planes.len()).map(Feature::Plane));
        if let Some(s) = sk {
            sketches.push(s);
            tl.push(Feature::Sketch(1));
        }
        tl.extend((0..holes.len()).map(Feature::Hole));
        build_all(&sketches, &[ex(0, vec![0], h, Operation::NewBody)], &[], &[], &[], &[], planes, &[], holes, &[], &[], &tl)
    }

    /// On the plate's top face at (x, y), as picked at thickness `h`.
    fn on_top(x: f64, y: f64, h: f64, centre: bool) -> HolePlace {
        let face = FacePlane::from_face(DVec3::Z, DVec3::new(x, y, h));
        let q = DVec3::new(x, y, h);
        HolePlace::Face { face, anchor: [q.dot(face.u), q.dot(face.v)], centre }
    }

    #[test]
    fn holes_remove_exactly_their_bore_tip_counterbore_and_countersink() {
        use crate::model::HoleKind;
        let plate = 15000.0;
        let r: f64 = 2.5;
        // Flat, 4 deep, on the top face.
        let flat = HoleFeature { places: vec![on_top(10.0, 10.0, 10.0, false)], depth: 4.0, diameter: 5.0, tip_angle: None, ..Default::default() };
        let m = drilled(10.0, None, &[flat.clone()], &[]);
        assert_eq!(m.hole_errors, vec![None]);
        assert!(close(total(&m), plate - PI * r * r * 4.0), "{}", total(&m));
        // A 118° drill tip adds a cone r / tan 59° deep.
        let tipped = HoleFeature { tip_angle: Some(118.0), ..flat.clone() };
        let cone = PI * r * r * (r / 59f64.to_radians().tan()) / 3.0;
        assert!(close(total(&drilled(10.0, None, &[tipped], &[])), plate - PI * r * r * 4.0 - cone));
        // Counterbore Ø10 × 3, right through.
        let cb = HoleFeature { kind: HoleKind::Counterbore, cbore_diameter: 10.0, cbore_depth: 3.0, extent: HoleExtent::All, ..flat.clone() };
        assert!(close(total(&drilled(10.0, None, &[cb], &[])), plate - PI * 25.0 * 3.0 - PI * r * r * 7.0));
        // Countersink Ø10 at 90°: a frustum 2.5 deep, then the bore to 8.
        let cs = HoleFeature { kind: HoleKind::Countersink, csink_diameter: 10.0, csink_angle: 90.0, depth: 8.0, ..flat.clone() };
        let frustum = PI * 2.5 * (25.0 + 5.0 * r + r * r) / 3.0;
        assert!(close(total(&drilled(10.0, None, &[cs], &[])), plate - frustum - PI * r * r * 5.5));
        // Two places: twice the flat hole.
        let two = HoleFeature { places: vec![on_top(10.0, 10.0, 10.0, false), on_top(30.0, 20.0, 10.0, false)], ..flat.clone() };
        assert!(close(total(&drilled(10.0, None, &[two], &[])), plate - 2.0 * PI * r * r * 4.0));
    }

    #[test]
    fn holes_follow_their_face_sketch_points_and_circles() {
        let r: f64 = 2.5;
        // Picked on the top at 10 mm; the plate grows to 20 and the hole rides up.
        let h = HoleFeature { places: vec![on_top(10.0, 10.0, 10.0, false)], depth: 4.0, tip_angle: None, ..Default::default() };
        let m = drilled(20.0, None, &[h], &[]);
        assert_eq!(m.hole_errors, vec![None]);
        assert!(close(total(&m), 30000.0 - PI * r * r * 4.0));
        let mesh = &m.bodies[0].mesh;
        assert!(!crate::model::inside(mesh, Vec3::new(10.0, 10.0, 18.0)) && crate::model::inside(mesh, Vec3::new(10.0, 10.0, 15.0)));
        // The centre of the top face, right through.
        let mid = HoleFeature { places: vec![on_top(5.0, 5.0, 10.0, true)], extent: HoleExtent::All, diameter: 4.0, tip_angle: None, ..Default::default() };
        let m = drilled(10.0, None, &[mid], &[]);
        assert!(close(total(&m), 15000.0 - PI * 4.0 * 10.0), "{}", total(&m));
        assert!(!crate::model::inside(&m.bodies[0].mesh, Vec3::new(25.0, 15.0, 5.0)), "drilled at the middle");
        // A sketch on XY under the plate: a point and a Ø6 circle, drilled up (flipped).
        let mut sk = Sketch::new("H".into(), OriginPlane::XY);
        sk.points.push(DVec2::new(40.0, 10.0));
        let point = sk.points.len() - 1;
        circle(&mut sk, 25.0, 15.0, 3.0);
        let entity = sk.entities.len() - 1;
        let up = HoleFeature {
            places: vec![HolePlace::SketchPoint { sketch: 1, point }, HolePlace::SketchCircle { sketch: 1, entity }],
            extent: HoleExtent::All,
            diameter: 2.0,
            tip_angle: None,
            flip: true,
            ..Default::default()
        };
        let m = drilled(10.0, Some(sk.clone()), &[up.clone()], &[]);
        assert_eq!(m.hole_errors, vec![None]);
        assert!(close(total(&m), 15000.0 - PI * 1.0 * 10.0 - PI * 9.0 * 10.0), "{}", total(&m));
        // Not flipped it drills down, away from the plate: an error-free no-op.
        let m = drilled(10.0, Some(sk), &[HoleFeature { flip: false, extent: HoleExtent::Distance, ..up }], &[]);
        assert!(close(total(&m), 15000.0));
    }

    /// Pipe-shell sweeps are B-spline approximations of the helix, so thread
    /// volumes are checked to 1e-3 (relative), set before running, not tuned.
    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-3 * b.abs()
    }

    #[test]
    fn a_thread_solid_is_its_profile_screwed_round() {
        // A screw motion sweeps volume = profile area × 2π × centroid radius × turns.
        let (r_in, r_out, w_in, w_out, pitch, length) = (2.4, 3.0, 0.8, 0.125, 1.0, 6.0);
        let shape = Shape::try_thread(kv(DVec3::ZERO), kv(DVec3::Z), kv(DVec3::X), (r_in, r_out), (w_in, w_out), pitch, length, true, 0.0).expect("a thread");
        let h = r_out - r_in;
        let area = (w_in + w_out) / 2.0 * h;
        let r_c = r_in + h * (w_in + 2.0 * w_out) / (3.0 * (w_in + w_out));
        let want = area * 2.0 * PI * r_c * (length / pitch);
        assert!(near(shape.volume(), want), "{} vs {want}", shape.volume());
        let left = Shape::try_thread(kv(DVec3::ZERO), kv(DVec3::Z), kv(DVec3::X), (r_in, r_out), (w_in, w_out), pitch, length, false, 0.0).unwrap();
        assert!(near(left.volume(), want), "left hand: the same amount");
        // Bad sizes: None, not a crash.
        assert!(Shape::try_thread(kv(DVec3::ZERO), kv(DVec3::Z), kv(DVec3::X), (3.0, 2.0), (w_in, w_out), pitch, length, true, 0.0).is_none());
    }

    #[test]
    fn a_modeled_tapped_hole_cuts_the_thread_beyond_its_bore() {
        use crate::model::ThreadSpec;
        let t = crate::threads::Thread::find("M6x1").unwrap();
        let (p, r1, r2) = (t.pitch, t.minor() / 2.0, t.major / 2.0);
        let thread = ThreadSpec { designation: "M6x1".into(), modeled: true, ..Default::default() };
        let through = HoleFeature { places: vec![on_top(25.0, 15.0, 10.0, false)], tap: TapType::Tapped, extent: HoleExtent::All, tip_angle: None, thread, ..Default::default() };
        let m = drilled(10.0, None, &[through.clone()], &[]);
        assert_eq!(m.hole_errors, vec![None]);
        assert!(m.threads.is_empty(), "a modeled thread isn't drawn as well");
        // Right through 10 mm, the groove past both faces: by the helix's symmetry
        // it removes area × 2π × centroid radius per pitch along the axis.
        let (w_in, w_out, h) = (0.75 * p, p / 8.0, r2 - r1);
        let area = (w_in + w_out) / 2.0 * h;
        let r_c = r1 + h * (w_in + 2.0 * w_out) / (3.0 * (w_in + w_out));
        let groove = area * 2.0 * PI * r_c * 10.0 / p;
        let bore = PI * r1 * r1 * 10.0;
        let removed = 15000.0 - total(&m);
        assert!(near(removed - bore, groove), "groove {} vs {groove}", removed - bore);
        assert!(removed > bore && removed < PI * r2 * r2 * 10.0, "between the minor and major bores");
        // Left hand: the same amount; blind and NPT taper: built, within bounds.
        let left = HoleFeature { thread: ThreadSpec { right_hand: false, ..through.thread.clone() }, ..through.clone() };
        assert!(near(15000.0 - total(&drilled(10.0, None, &[left], &[])) - bore, groove));
        let blind = HoleFeature { extent: HoleExtent::Distance, depth: 6.0, tip_angle: Some(118.0), ..through.clone() };
        let m = drilled(10.0, None, &[blind], &[]);
        assert_eq!(m.hole_errors, vec![None]);
        let removed = 15000.0 - total(&m);
        assert!(removed > PI * r1 * r1 * 6.0 && removed < PI * r2 * r2 * 6.0 + PI * r1 * r1 * r1, "{removed}");
        let npt = HoleFeature {
            tap: TapType::TaperTapped,
            extent: HoleExtent::Distance,
            depth: 8.0,
            thread: ThreadSpec { standard: crate::threads::Standard::Npt, designation: "1/8-27 NPT".into(), modeled: true, ..Default::default() },
            ..through
        };
        // Taper tapped is shown, not modeled (as in Fusion): a 1:16 tapered bore,
        // the stated size's minor diameter at the bottom.
        let m = drilled(10.0, None, &[npt], &[]);
        assert_eq!(m.hole_errors, vec![None]);
        assert_eq!(m.threads.len(), 1);
        let small = crate::threads::Thread::find("1/8-27 NPT").unwrap().minor() / 2.0;
        let large = small + 8.0 * crate::threads::NPT_TAPER / 2.0;
        let frustum = PI * 8.0 * (large * large + large * small + small * small) / 3.0;
        assert!(close(15000.0 - total(&m), frustum), "{} vs {frustum}", 15000.0 - total(&m));
    }

    /// An Ø`2r` × `len` shaft standing on XY (sketch 0, extrude 0), threaded by `t`.
    fn threaded_shaft(r: f64, len: f64, t: Option<ThreadFeature>) -> Model {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        circle(&mut sk, 0.0, 0.0, r);
        let mut tl = vec![Feature::Sketch(0), Feature::Extrude(0)];
        let tf: Vec<ThreadFeature> = t.into_iter().collect();
        tl.extend((0..tf.len()).map(Feature::Thread));
        build_all(&[sk], &[ex(0, vec![0], len, Operation::NewBody)], &[], &[], &[], &[], &[], &[], &[], &tf, &[], &tl)
    }

    /// The round face of the first body, as a thread would pick it (clicked near z = `z`).
    fn round_face(m: &Model, z: f64) -> CylinderRef {
        let b = &m.bodies[0];
        let c = (0..b.faces.len()).find_map(|i| cylinder_face(&b.shape, i)).expect("a round face");
        c.as_ref(DVec3::new(c.radius, 0.0, z))
    }

    fn thread_on(face: CylinderRef, designation: &str, modeled: bool) -> ThreadFeature {
        let spec = crate::model::ThreadSpec { designation: designation.into(), modeled, ..Default::default() };
        ThreadFeature { name: "T".into(), faces: vec![face], spec, full_length: true, offset: 0.0, length: 10.0 }
    }

    #[test]
    fn a_thread_on_a_shaft_cuts_its_profile_along_the_face() {
        let plain = threaded_shaft(4.0, 20.0, None);
        let face = round_face(&plain, 1.0);
        assert!(!cylinder_face(&plain.bodies[0].shape, 0).is_some_and(|c| c.internal), "a shaft is external");
        let full = PI * 16.0 * 20.0;
        // Cosmetic: drawn, not cut.
        let m = threaded_shaft(4.0, 20.0, Some(thread_on(face, "M8x1.25", false)));
        assert_eq!(m.thread_errors, vec![None]);
        assert!(close(total(&m), full) && m.threads.len() == 1);
        assert!((m.threads[0].length - 20.0).abs() < 1e-9 && m.threads[0].origin.z.abs() < 1e-9, "from the end nearer the click");
        // Modeled, full length: exactly the profile's part in the material, screwed along 20 mm.
        let m = threaded_shaft(4.0, 20.0, Some(thread_on(face, "M8x1.25", true)));
        assert_eq!(m.thread_errors, vec![None]);
        let want = thread_removal_per_length(4.0, false, 1.25) * 20.0;
        assert!(near(full - total(&m), want), "{} vs {want}", full - total(&m));
        // 8 mm of it, 5 mm in from the end nearer the click (the top here).
        let mut part = thread_on(round_face(&plain, 19.0), "M8x1.25", true);
        (part.full_length, part.offset, part.length) = (false, 5.0, 8.0);
        let m = threaded_shaft(4.0, 20.0, Some(part));
        assert!(near(full - total(&m), thread_removal_per_length(4.0, false, 1.25) * 8.0));
        // The shaft grows to 30: the full-length thread follows.
        let m = threaded_shaft(4.0, 30.0, Some(thread_on(face, "M8x1.25", true)));
        assert!(near(PI * 16.0 * 30.0 - total(&m), thread_removal_per_length(4.0, false, 1.25) * 30.0));
    }

    #[test]
    fn a_thread_in_a_hole_cuts_outward() {
        // A Ø5 hole through the 10 mm plate, then an M6 thread in it.
        let hole = HoleFeature { places: vec![on_top(25.0, 15.0, 10.0, false)], extent: HoleExtent::All, diameter: 5.0, tip_angle: None, ..Default::default() };
        let plate = drilled(10.0, None, &[hole.clone()], &[]);
        let b = &plate.bodies[0];
        let c = (0..b.faces.len()).find_map(|i| cylinder_face(&b.shape, i)).expect("the bore");
        assert!(c.internal && (c.radius - 2.5).abs() < 1e-9);
        let face = c.as_ref(DVec3::new(27.5, 15.0, 9.0));
        let mut sk = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 0.0, 50.0, 30.0);
        let tl = [Feature::Sketch(0), Feature::Extrude(0), Feature::Hole(0), Feature::Thread(0)];
        let m = build_all(&[sk], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[], &[], &[], &[], &[], &[], &[hole], &[thread_on(face, "M6x1", true)], &[], &tl);
        assert_eq!(m.thread_errors, vec![None]);
        let want = thread_removal_per_length(2.5, true, 1.0) * 10.0;
        assert!(near(total(&plate) - total(&m), want), "{} vs {want}", total(&plate) - total(&m));
    }

    #[test]
    fn thread_grooves_cut_reliably_into_shafts_and_bores() {
        // Each row: a 20 mm cylinder (a shaft, or a bore in a block) and a groove
        // swept past both ends; it must take out exactly its profile's part in
        // the material, screwed along 20 mm, whichever way it's swept.
        for (internal, r, p) in [(true, 2.4587, 1.0), (false, 4.0, 1.25), (false, 3.0, 1.0), (true, 4.0, 1.25)] {
            for dir in [DVec3::Z, DVec3::NEG_Z] {
                let ((r_in, r_out), widths) = thread_profile(r, internal, p);
                let start = DVec3::new(0.0, 0.0, 10.0) - dir * 11.5;
                let g = Shape::try_thread(kv(start), kv(dir), kv(DVec3::X), (r_in, r_out), widths, p, 23.0, true, 0.0).unwrap();
                let cyl = Shape::cylinder(kv(DVec3::ZERO), r, kv(DVec3::Z), 20.0);
                let block = Shape::box_from_corners(kv(DVec3::new(-10.0, -10.0, 0.0)), kv(DVec3::new(10.0, 10.0, 20.0)));
                let obj = if internal { block.try_subtract(&cyl).unwrap() } else { cyl };
                let removed = obj.volume() - obj.try_subtract(&g).unwrap().volume();
                let want = thread_removal_per_length(r, internal, p) * 20.0;
                assert!(near(removed, want), "internal {internal} r {r} p {p} dir {dir}: {removed} vs {want}");
            }
        }
    }

    #[test]
    fn an_unchanged_threaded_hole_is_not_cut_again() {
        let thread = crate::model::ThreadSpec { designation: "M6x1".into(), modeled: true, ..Default::default() };
        let h = HoleFeature { places: vec![on_top(25.0, 15.0, 10.0, false)], tap: TapType::Tapped, depth: 8.0, thread, ..Default::default() };
        let a = drilled(10.0, None, &[h.clone()], &[]);
        let b = drilled(10.0, None, &[h.clone()], &[]);
        assert!(a.bodies[0].shape.is_same(&b.bodies[0].shape), "the second build reuses the first's result");
        assert_eq!(a.bodies[0].faces, b.bodies[0].faces);
        // Anything that changes it is built afresh.
        let deeper = HoleFeature { depth: 9.0, ..h };
        let c = drilled(10.0, None, &[deeper], &[]);
        assert!(!c.bodies[0].shape.is_same(&a.bodies[0].shape));
        assert!(c.bodies[0].shape.volume() < a.bodies[0].shape.volume());
        let d = drilled(12.0, None, &[HoleFeature { depth: 8.0, ..c_hole() }], &[]);
        assert_eq!(d.hole_errors, vec![None], "a different plate is a different input");
    }

    fn c_hole() -> HoleFeature {
        HoleFeature { places: vec![on_top(25.0, 15.0, 12.0, false)], ..Default::default() }
    }

    #[test]
    fn a_boss_on_a_plate_is_a_threadable_cylinder() {
        // A Ø8 × 8 boss joined on the plate (its base circle comes out split in arcs).
        let build = |t: &[ThreadFeature]| {
            let mut b = Sketch::new("B".into(), OriginPlane::XY);
            rect(&mut b, 0.0, 0.0, 50.0, 30.0);
            let top = FacePlane::from_face(DVec3::Z, DVec3::new(25.0, 15.0, 10.0));
            let mut boss = Sketch::on_face("C".into(), top);
            circle(&mut boss, 5.0, 3.0, 4.0);
            let mut tl = vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1)];
            tl.extend((0..t.len()).map(Feature::Thread));
            build_all(&[b, boss], &[ex(0, vec![0], 10.0, Operation::NewBody), ex(1, vec![0], 8.0, Operation::Join)], &[], &[], &[], &[], &[], &[], &[], t, &[], &tl)
        };
        let m = build(&[]);
        let body = &m.bodies[0];
        let c = (0..body.faces.len()).find_map(|i| cylinder_face(&body.shape, i).filter(|c| !c.internal));
        let c = c.expect("the boss is a whole cylinder");
        assert!((c.radius - 4.0).abs() < 1e-9 && (c.length - 8.0).abs() < 1e-9, "{c:?}");
        // A modeled M8 thread on it, the plate under it untouched.
        let face = c.as_ref(c.axis_point + DVec3::X * 4.0 + c.axis * 7.0);
        let threaded = build(&[thread_on(face, "M8x1.25", true)]);
        assert_eq!(threaded.thread_errors, vec![None]);
        let want = thread_removal_per_length(4.0, false, 1.25) * 8.0;
        assert!(near(total(&m) - total(&threaded), want), "{} vs {want}", total(&m) - total(&threaded));
    }

    #[test]
    fn a_countersunk_modeled_tapped_hole_cuts() {
        use crate::model::{HoleKind, ThreadSpec};
        let thread = ThreadSpec { designation: "M6x1".into(), modeled: true, ..Default::default() };
        let h = HoleFeature {
            places: vec![on_top(25.0, 15.0, 20.0, false)],
            kind: HoleKind::Countersink,
            csink_diameter: 12.4,
            csink_angle: 90.0,
            tap: TapType::Tapped,
            depth: 10.0,
            thread,
            ..Default::default()
        };
        let m = drilled(20.0, None, &[h.clone()], &[]);
        assert_eq!(m.hole_errors, vec![None], "the thread through the countersink's air isn't a failure");
        let cb = HoleFeature { kind: HoleKind::Counterbore, cbore_diameter: 11.0, cbore_depth: 6.4, ..h };
        assert_eq!(drilled(20.0, None, &[cb], &[]).hole_errors, vec![None]);
    }

    #[test]
    fn shell_and_offset_primitives_are_exact() {
        let m = plate_with(10.0, None);
        let b = &m.bodies[0];
        let top = (0..b.faces.len()).find(|&f| b.face_tris(f).all(|t| t.iter().all(|p| (p.z - 10.0).abs() < 1e-9))).unwrap() as i32;
        let inside = b.shape.try_thick_solid(&[top], -2.0, false).expect("inside shell");
        assert!(close(inside.volume(), 15000.0 - 46.0 * 26.0 * 8.0), "{}", inside.volume());
        let outside = b.shape.try_thick_solid(&[top], 2.0, false).expect("outside shell");
        assert!(close(outside.volume(), 54.0 * 34.0 * 12.0 - 15000.0), "{}", outside.volume());
        let core = b.shape.try_offset(-2.0, false).expect("shrunk");
        assert!(close(b.shape.volume() - core.volume(), 15000.0 - 46.0 * 26.0 * 6.0));
        let rounded = b.shape.try_thick_solid(&[top], 2.0, true).expect("rounded");
        assert!(rounded.volume() < outside.volume() && rounded.volume() > 50.0 * 30.0 * 2.0, "rounded corners take a little off");
        assert!(b.shape.try_thick_solid(&[99], -2.0, false).is_none(), "a face that isn't there");
        // Too thick: OCCT gives the part back unchanged (the Shell step reports it).
        let same = b.shape.try_thick_solid(&[top], -20.0, false);
        assert!(same.is_none_or(|x| close(x.volume(), 15000.0)));
    }

    #[test]
    fn offset_faces_primitive_moves_a_face_exactly() {
        let m = plate_with(10.0, None);
        let b = &m.bodies[0];
        let face_at = |z: f64| (0..b.faces.len()).find(|&f| b.face_tris(f).all(|t| t.iter().all(|p| (p.z - z).abs() < 1e-9))).unwrap() as i32;
        let (top, bottom) = (face_at(10.0), face_at(0.0));
        for inter in [false, true] {
            let up = b.shape.try_offset_faces(&[top], &[5.0], inter).expect("pulled");
            assert!(close(up.volume(), 50.0 * 30.0 * 15.0), "{inter}: {}", up.volume());
            let down = b.shape.try_offset_faces(&[top], &[-3.0], inter).expect("pushed");
            assert!(close(down.volume(), 50.0 * 30.0 * 7.0), "{}", down.volume());
            let both = b.shape.try_offset_faces(&[top, bottom], &[2.0, 1.0], inter).expect("two faces");
            assert!(close(both.volume(), 50.0 * 30.0 * 13.0), "{}", both.volume());
        }
        // The end face of a plate with a rounded edge: the round stretches with it.
        let m = plate_with(10.0, Some((EdgeKind::Fillet, 2.0, top_front(10.0))));
        let fb = &m.bodies[0];
        let end = (0..fb.faces.len()).find(|&f| fb.face_tris(f).all(|t| t.iter().all(|p| (p.x - 50.0).abs() < 1e-9))).unwrap() as i32;
        for inter in [false, true] {
            let longer = fb.shape.try_offset_faces(&[end], &[5.0], inter).expect("end pulled");
            let want = 55.0 * 30.0 * 10.0 - 4.0 * (1.0 - PI / 4.0) * 55.0;
            assert!(close(longer.volume(), want), "{inter}: {} vs {want}", longer.volume());
        }
        assert!(b.shape.try_offset_faces(&[99], &[1.0], false).is_none(), "a face that isn't there");
        assert!(b.shape.try_offset_faces(&[top], &[-10.0], false).is_none_or(|s| s.volume() < 1e-6), "pushed right through");
    }

    /// A 50×30×10 plate (body Extrude 0) and a 10×10×20 post through it from
    /// z −5 to 15 (body Extrude 1; 1000 of it inside the plate), then Combine `c`.
    fn combined(c: CombineFeature) -> Model {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 20.0, 10.0, 30.0, 20.0);
        let post = Extrude { extent: crate::model::Extent::TwoSides, distance2: 5.0, ..ex(1, vec![0], 15.0, Operation::NewBody) };
        let mut build = Build::start(BuildInputs {
            sketches: vec![a, b],
            extrudes: vec![ex(0, vec![0], 10.0, Operation::NewBody), post],
            combines: vec![c],
            timeline: vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1), Feature::Combine(0)],
            ..Default::default()
        });
        while build.step(std::time::Duration::MAX) {}
        build.finish()
    }

    #[test]
    fn combine_joins_cuts_and_intersects_exactly() {
        let plate = BodyId { feature: Feature::Extrude(0), piece: 0 };
        let post = BodyId { feature: Feature::Extrude(1), piece: 0 };
        let c = CombineFeature { target: Some(plate), tools: vec![post], ..Default::default() };
        // Without tools it's an error and both bodies stay: 15000 + 2000.
        let m = combined(CombineFeature { tools: Vec::new(), ..c.clone() });
        assert!(m.combine_errors[0].is_some(), "no tools");
        assert!(close(total(&m), 17000.0), "{}", total(&m));
        let m = combined(c.clone());
        assert_eq!(m.combine_errors, vec![None]);
        assert_eq!(m.bodies.len(), 1);
        assert_eq!(m.bodies[0].id, plate, "the target keeps its identity");
        assert!(close(total(&m), 16000.0), "{}", total(&m));
        let m = combined(CombineFeature { op: Operation::Cut, ..c.clone() });
        assert_eq!(m.bodies.len(), 1);
        assert!(close(total(&m), 14000.0), "{}", total(&m));
        let m = combined(CombineFeature { op: Operation::Cut, keep_tools: true, ..c.clone() });
        assert_eq!(m.bodies.len(), 2, "the post is kept");
        assert!(close(total(&m), 16000.0), "{}", total(&m));
        let m = combined(CombineFeature { op: Operation::Intersect, ..c.clone() });
        assert!(close(total(&m), 1000.0), "{}", total(&m));
        // The post as the target: the plate cut from it leaves its two ends.
        let m = combined(CombineFeature { target: Some(post), tools: vec![plate], op: Operation::Cut, ..c.clone() });
        assert_eq!(m.bodies.len(), 2);
        assert!(m.bodies.iter().all(|b| b.id.feature == Feature::Extrude(1)));
        assert!(close(total(&m), 1000.0), "{}", total(&m));
        let gone = CombineFeature { tools: vec![BodyId { feature: Feature::Extrude(5), piece: 0 }], ..c };
        assert!(combined(gone).combine_errors[0].as_deref().is_some_and(|e| e.contains("gone")));
    }

    /// The 50×30×10 plate (body Extrude 0), then Move `m`.
    fn moved(m: MoveFeature) -> Model {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let mut build = Build::start(BuildInputs {
            sketches: vec![a],
            extrudes: vec![ex(0, vec![0], 10.0, Operation::NewBody)],
            moves: vec![m],
            timeline: vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Move(0)],
            ..Default::default()
        });
        while build.step(std::time::Duration::MAX) {}
        build.finish()
    }

    fn bbox(b: &Body) -> (DVec3, DVec3) {
        let bb = opencascade::bounding_box::aabb(&b.shape);
        (DVec3::new(bb.min().x, bb.min().y, bb.min().z), DVec3::new(bb.max().x, bb.max().y, bb.max().z))
    }

    #[test]
    fn move_translates_turns_and_copies_bodies() {
        let plate = BodyId { feature: Feature::Extrude(0), piece: 0 };
        let near = |a: DVec3, b: DVec3| a.distance(b) < 1e-6;
        // Moved 10 along X and 5 up: same body, same volume, new place.
        let m = moved(MoveFeature { bodies: vec![plate], kind: MoveKind::Free { offset: [10.0, 0.0, 5.0], angles: [0.0; 3] }, ..Default::default() });
        assert_eq!(m.move_errors, vec![None]);
        assert_eq!((m.bodies.len(), m.bodies[0].id), (1, plate));
        assert!(close(total(&m), 15000.0));
        let (lo, hi) = bbox(&m.bodies[0]);
        assert!(near(lo, DVec3::new(10.0, 0.0, 5.0)) && near(hi, DVec3::new(60.0, 30.0, 15.0)), "{lo} {hi}");
        assert_eq!(m.move_pivots, vec![Some(DVec3::new(25.0, 15.0, 5.0))], "the box's middle before the move");
        // Turned 90° about Z through its middle: 30 wide in X, 50 in Y, same middle.
        let m = moved(MoveFeature { bodies: vec![plate], kind: MoveKind::Free { offset: [0.0; 3], angles: [0.0, 0.0, 90.0] }, ..Default::default() });
        let (lo, hi) = bbox(&m.bodies[0]);
        assert!(near(lo, DVec3::new(10.0, -10.0, 0.0)) && near(hi, DVec3::new(40.0, 40.0, 10.0)), "{lo} {hi}");
        // Rotate 180° about the origin's Z axis, as a copy: two bodies.
        let m = moved(MoveFeature { bodies: vec![plate], kind: MoveKind::Rotate { axis: crate::model::WorldAxis::Z, custom: None, angle: 180.0 }, copy: true, ..Default::default() });
        assert_eq!(m.bodies.len(), 2);
        assert!(close(total(&m), 30000.0));
        let copy = m.bodies.iter().find(|b| b.id.feature == Feature::Move(0)).expect("the copy");
        let (lo, hi) = bbox(copy);
        assert!(near(lo, DVec3::new(-50.0, -30.0, 0.0)) && near(hi, DVec3::new(0.0, 0.0, 10.0)), "{lo} {hi}");
        // Nothing picked that exists: an error, the plate stays.
        let m = moved(MoveFeature { bodies: vec![BodyId { feature: Feature::Extrude(4), piece: 0 }], ..Default::default() });
        assert!(m.move_errors[0].is_some());
        assert!(close(total(&m), 15000.0));
    }

    /// The plate (sketch 0, extrude 0) `h` thick, then offset face `of`.
    fn offset(h: f64, of: OffsetFaceFeature) -> Model {
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 0.0, 0.0, 50.0, 30.0);
        let mut build = Build::start(BuildInputs {
            sketches: vec![b],
            extrudes: vec![ex(0, vec![0], h, Operation::NewBody)],
            offset_faces: vec![of],
            timeline: vec![Feature::Sketch(0), Feature::Extrude(0), Feature::OffsetFace(0)],
            ..Default::default()
        });
        while build.step(std::time::Duration::MAX) {}
        build.finish()
    }

    #[test]
    fn offset_face_moves_the_picked_face_and_finds_it_again() {
        let up = OffsetFaceFeature { faces: vec![top_face(10.0)], distance: 5.0, ..Default::default() };
        let m = offset(10.0, up.clone());
        assert_eq!(m.offset_face_errors, vec![None]);
        assert!(close(total(&m), 50.0 * 30.0 * 15.0), "{}", total(&m));
        let down = OffsetFaceFeature { distance: -3.0, ..up.clone() };
        assert!(close(total(&offset(10.0, down)), 50.0 * 30.0 * 7.0));
        // The plate grows to 20: the top is found again there.
        let m = offset(20.0, up.clone());
        assert_eq!(m.offset_face_errors, vec![None]);
        assert!(close(total(&m), 50.0 * 30.0 * 25.0), "{}", total(&m));
        // Top and an end together.
        let end = {
            let face = FacePlane::from_face(DVec3::X, DVec3::new(50.0, 0.0, 0.0));
            let q = DVec3::new(50.0, 10.0, 5.0);
            crate::model::PickedFace { face, anchor: [q.dot(face.u), q.dot(face.v)] }
        };
        let two = OffsetFaceFeature { faces: vec![top_face(10.0), end], distance: 2.0, ..Default::default() };
        assert!(close(total(&offset(10.0, two)), 52.0 * 30.0 * 12.0));
        // Errors, not crashes: through the part, a face that's gone, nothing picked.
        let through = OffsetFaceFeature { distance: -10.0, ..up.clone() };
        let m = offset(10.0, through);
        assert!(m.offset_face_errors[0].is_some());
        assert!(close(total(&m), 15000.0), "the plate stays as it was");
        let away = OffsetFaceFeature { faces: vec![crate::model::PickedFace { anchor: [500.0, 500.0], ..top_face(10.0) }], ..up.clone() };
        assert!(offset(10.0, away).offset_face_errors[0].as_deref().is_some_and(|e| e.contains("gone")));
        assert!(offset(10.0, OffsetFaceFeature { faces: Vec::new(), ..up }).offset_face_errors[0].is_some());
    }

    /// The plate (sketch 0, extrude 0) `h` thick, then shell `sf`.
    fn shelled(h: f64, sf: ShellFeature) -> Model {
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 0.0, 0.0, 50.0, 30.0);
        build_all(&[b], &[ex(0, vec![0], h, Operation::NewBody)], &[], &[], &[], &[], &[], &[], &[], &[], &[sf], &[Feature::Sketch(0), Feature::Extrude(0), Feature::Shell(0)])
    }

    #[test]
    fn a_round_as_wide_as_the_wall_uses_the_rim_up() {
        // A 2 mm round on the rim of a box with 2 mm walls takes the whole
        // flat rim. At exactly 2 OCCT's fillet builder segfaults (caught as a
        // failure); the retry a millionth under makes it.
        let open = ShellFeature { faces: vec![top_face(10.0)], ..Default::default() };
        let m = shelled(10.0, open.clone());
        let box_volume = 15000.0 - 46.0 * 26.0 * 8.0;
        let rim = m.bodies[0].edges.iter().map(|e| e.at).find(|r| r.dir.dot(DVec3::X).abs() > 0.99 && (r.mid - DVec3::new(25.0, 30.0, 10.0)).length() < 1e-6).expect("the back outer rim");
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 0.0, 0.0, 50.0, 30.0);
        let tl = [Feature::Sketch(0), Feature::Extrude(0), Feature::Shell(0), Feature::Edge(0)];
        let rounded = |kind, size: f64| {
            let f = EdgeFeature { name: "F".into(), kind, size, size2: None, angle: None, edges: vec![rim] };
            build_all(&[b.clone()], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[], &[f], &[], &[], &[], &[], &[], &[], &[open.clone()], &tl)
        };
        let m = rounded(EdgeKind::Fillet, 2.0);
        assert_eq!(m.edge_errors, vec![None]);
        let want = box_volume - 4.0 * (1.0 - PI / 4.0) * 50.0;
        assert!((total(&m) - want).abs() < 1e-3, "{} vs {want}", total(&m));
        assert_eq!(m.bodies[0].flat_face_centres().len(), 11, "no hairline face offered");
        let m = rounded(EdgeKind::Chamfer, 2.0);
        assert_eq!(m.edge_errors, vec![None]);
        assert!((total(&m) - (box_volume - 2.0 * 50.0)).abs() < 1e-3, "{}", total(&m));
        // Wider than the wall is still too large.
        let m = rounded(EdgeKind::Fillet, 2.001);
        assert!(m.edge_errors[0].is_some());
        assert!(close(total(&m), box_volume), "the box stays, unrounded: {}", total(&m));
        // A plain one is exact.
        assert!(close(total(&rounded(EdgeKind::Fillet, 1.0)), box_volume - (1.0 - PI / 4.0) * 50.0));
    }

    #[test]
    fn a_round_wider_than_the_rim_is_refused_even_where_occt_would_make_it() {
        // Open at the top and the front: the left wall's top edge runs out
        // to the open end, and OCCT would run a 13 mm round on through the
        // wall. Fusion refuses a round wider than the face beside it.
        let front = {
            let face = FacePlane::from_face(DVec3::NEG_Y, DVec3::ZERO);
            let q = DVec3::new(20.0, 0.0, 20.0);
            crate::model::PickedFace { face, anchor: [q.dot(face.u), q.dot(face.v)] }
        };
        let open = ShellFeature { faces: vec![top_face(40.0), front], ..Default::default() };
        let m = shelled(40.0, open.clone());
        assert_eq!(m.shell_errors, vec![None]);
        let box_volume = total(&m);
        let wall = m.bodies[0].edges.iter().map(|e| e.at).find(|r| (r.mid - DVec3::new(0.0, 15.0, 40.0)).length() < 1e-6).expect("the left wall's outer top edge");
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 0.0, 0.0, 50.0, 30.0);
        let tl = [Feature::Sketch(0), Feature::Extrude(0), Feature::Shell(0), Feature::Edge(0)];
        let rounded = |kind, size: f64| {
            let f = EdgeFeature { name: "F".into(), kind, size, size2: None, angle: None, edges: vec![wall] };
            build_all(&[b.clone()], &[ex(0, vec![0], 40.0, Operation::NewBody)], &[], &[f], &[], &[], &[], &[], &[], &[], &[open.clone()], &tl)
        };
        for (kind, size) in [(EdgeKind::Fillet, 13.3), (EdgeKind::Fillet, 2.1), (EdgeKind::Chamfer, 3.0)] {
            let m = rounded(kind, size);
            assert!(m.edge_errors[0].as_deref().is_some_and(|e| e.contains("wider than the face")), "{size}: {:?}", m.edge_errors);
            assert!(close(total(&m), box_volume));
        }
        let m = rounded(EdgeKind::Fillet, 2.0);
        assert_eq!(m.edge_errors, vec![None]);
        assert!((total(&m) - (box_volume - 4.0 * (1.0 - PI / 4.0) * 30.0)).abs() < 1e-3, "{}", box_volume - total(&m));
    }

    fn top_face(h: f64) -> crate::model::PickedFace {
        let face = FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, h));
        let q = DVec3::new(20.0, 10.0, h);
        crate::model::PickedFace { face, anchor: [q.dot(face.u), q.dot(face.v)] }
    }

    #[test]
    fn a_shell_hollows_exactly_inside_outside_both_and_closed() {
        let open = ShellFeature { faces: vec![top_face(10.0)], ..Default::default() };
        let m = shelled(10.0, open.clone());
        assert_eq!(m.shell_errors, vec![None]);
        assert!(close(total(&m), 15000.0 - 46.0 * 26.0 * 8.0), "{}", total(&m));
        let out = ShellFeature { direction: ShellDirection::Outside, ..open.clone() };
        assert!(close(total(&shelled(10.0, out)), 54.0 * 34.0 * 12.0 - 15000.0));
        let both = ShellFeature { direction: ShellDirection::Both, inside: 1.0, outside: 1.0, ..open.clone() };
        let m = shelled(10.0, both);
        assert_eq!(m.shell_errors, vec![None]);
        assert!(close(total(&m), 52.0 * 32.0 * 11.0 - 48.0 * 28.0 * 9.0), "{}", total(&m));
        let closed = ShellFeature { faces: Vec::new(), bodies: vec![BodyId { feature: Feature::Extrude(0), piece: 0 }], ..open.clone() };
        assert!(close(total(&shelled(10.0, closed)), 15000.0 - 46.0 * 26.0 * 6.0));
        // The plate grows to 20: the opened face is found again on the new top.
        let m = shelled(20.0, open.clone());
        assert_eq!(m.shell_errors, vec![None]);
        assert!(close(total(&m), 30000.0 - 46.0 * 26.0 * 18.0), "{}", total(&m));
        // Too thick, nothing picked, a face that's gone: errors, not crashes.
        let thick = ShellFeature { inside: 20.0, ..open.clone() };
        assert!(shelled(10.0, thick).shell_errors[0].as_deref().is_some_and(|e| e.contains("thicker")));
        let nothing = ShellFeature { faces: Vec::new(), ..open.clone() };
        assert!(shelled(10.0, nothing).shell_errors[0].is_some());
        let away = ShellFeature { faces: vec![crate::model::PickedFace { anchor: [500.0, 500.0], ..top_face(10.0) }], ..open };
        assert!(shelled(10.0, away).shell_errors[0].as_deref().is_some_and(|e| e.contains("gone")));
    }

    #[test]
    fn a_face_knows_its_middle() {
        let m = plate_with(10.0, None);
        let b = &m.bodies[0];
        let top = (0..b.faces.len()).find(|&f| b.face_tris(f).all(|t| t.iter().all(|p| (p.z - 10.0).abs() < 1e-9))).unwrap();
        assert!(b.face_centroid(top).unwrap().distance(DVec3::new(25.0, 15.0, 10.0)) < 1e-9);
        let centres = b.flat_face_centres();
        assert_eq!(centres.len(), 6);
        assert!(centres.iter().any(|c| c.distance(DVec3::new(25.0, 15.0, 0.0)) < 1e-9) && centres.iter().any(|c| c.distance(DVec3::new(0.0, 15.0, 5.0)) < 1e-9));
        let plane = FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0));
        assert_eq!(b.face_at(&plane, DVec3::new(10.0, 3.0, 10.0)), Some(top), "found again from a point on it");
        assert_eq!(b.face_at(&plane, DVec3::new(60.0, 3.0, 10.0)), None, "off the face");
    }

    #[test]
    fn holes_run_to_a_plane_and_take_screw_sizes() {
        // From the top (z 10) down to a plane at z 6: 4 deep.
        let plane = PlaneFeature { name: "P".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: true, offset: 6.0, anchor: [0.0; 2], extent: [0.0; 4], angle: None, kind: None };
        let to = HoleFeature { places: vec![on_top(10.0, 10.0, 10.0, false)], extent: HoleExtent::To, target: Some(ExtrudeTarget::Plane(0)), tip_angle: None, diameter: 5.0, ..Default::default() };
        let m = drilled(10.0, None, &[to.clone()], &[plane.clone()]);
        assert_eq!(m.hole_errors, vec![None]);
        assert!(close(total(&m), 15000.0 - PI * 6.25 * 4.0));
        // A plane above the face is behind the hole.
        let up = PlaneFeature { offset: 14.0, ..plane };
        assert!(drilled(10.0, None, &[to.clone()], &[up]).hole_errors[0].as_deref().is_some_and(|e| e.contains("behind")));
        // Clearance M6 (normal fit, Ø6.6) right through; tapped M6: drilled at the minor Ø, thread shown.
        let clear = HoleFeature { tap: TapType::Clearance, extent: HoleExtent::All, target: None, ..to.clone() };
        assert!(close(total(&drilled(10.0, None, &[clear], &[])), 15000.0 - PI * 3.3 * 3.3 * 10.0));
        let tapped = HoleFeature { tap: TapType::Tapped, extent: HoleExtent::All, target: None, ..to };
        let m = drilled(10.0, None, &[tapped], &[]);
        let minor = crate::threads::Thread::find("M6x1").unwrap().minor() / 2.0;
        assert!(close(total(&m), 15000.0 - PI * minor * minor * 10.0));
        assert_eq!(m.threads.len(), 1);
        assert!((m.threads[0].radius - minor * 0.99).abs() < 1e-12 && m.threads[0].axis.distance(DVec3::NEG_Z) < 1e-12, "drawn on the bore's wall");
    }

    #[test]
    fn inside_tells_material_from_air() {
        let m = plate_with(10.0, None); // 50 × 30 × 10
        let mesh = &m.bodies[0].mesh;
        assert!(crate::model::inside(mesh, Vec3::new(25.0, 15.0, 5.0)));
        assert!(crate::model::inside(mesh, Vec3::new(0.01, 0.01, 9.99)));
        assert!(!crate::model::inside(mesh, Vec3::new(25.0, 15.0, 10.5)));
        assert!(!crate::model::inside(mesh, Vec3::new(-0.1, 15.0, 5.0)));
    }

    #[test]
    fn measure_box_faces_are_tagged_and_exact() {
        // 50 × 30 × 10 plate: faces 1500, 300 and 500 mm², two of each.
        let m = plate_with(10.0, None);
        let b = &m.bodies[0];
        assert_eq!(b.faces.len(), 6);
        assert_eq!(b.tri_face.len(), b.mesh.tris.len());
        let mut areas: Vec<f64> = b.faces.iter().map(|g| match *g {
            Geom::Plane { area, loop_length: perimeter, normal, center } => {
                assert_eq!(center, None);
                // Every triangle tagged with this face lies flat in it, facing the same way.
                let i = b.faces.iter().position(|h| h == g).unwrap();
                assert!(b.face_tris(i).count() >= 2);
                for (t, &f) in b.mesh.tris.iter().enumerate().map(|(t, _)| t).zip(&b.tri_face) {
                    if f as usize == i {
                        assert!(b.mesh.normal(t).as_dvec3().dot(normal) > 0.9999);
                    }
                }
                let sides = if area == 1500.0 { 160.0 } else if area == 300.0 { 80.0 } else { 120.0 };
                assert!((perimeter - sides).abs() < 1e-9, "area {area}: perimeter {perimeter}");
                area
            }
            ref other => panic!("box face read as {other:?}"),
        }).collect();
        areas.sort_by(f64::total_cmp);
        let want = [300.0, 300.0, 500.0, 500.0, 1500.0, 1500.0];
        assert!(areas.iter().zip(want).all(|(a, w)| (a - w).abs() < 1e-9), "{areas:?}");
        // Straight edges know their exact length.
        let long = b.edges.iter().filter(|e| matches!(e.geom, Geom::Line { a, b } if (a.distance(b) - 50.0).abs() < 1e-9)).count();
        assert_eq!(long, 4);
    }

    #[test]
    fn measure_cylinder_reads_radius_circumference_and_side_area() {
        // r = 10, h = 20: side 2πrh = 400π, rims 20π long.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        circle(&mut sk, 0.0, 0.0, 10.0);
        let m = build4(&[sk], &[ex(0, vec![0], 20.0, Operation::NewBody)], &[], &timeline(1, 1));
        let b = &m.bodies[0];
        let side = b.faces.iter().find_map(|g| match *g {
            Geom::Cylinder { axis, radius, area, loop_length } => {
                // Two rims (the seam isn't boundary).
                assert!((loop_length - 40.0 * PI).abs() < 1e-9, "{loop_length}");
                Some((axis, radius, area))
            }
            _ => None,
        });
        let (axis, radius, area) = side.expect("the side is a cylinder");
        assert!(axis.z.abs() > 1.0 - 1e-9 && (radius - 10.0).abs() < 1e-9 && (area - 400.0 * PI).abs() < 1e-6, "{radius} {area}");
        let caps: Vec<f64> = b.faces.iter().filter_map(|g| match *g {
            Geom::Plane { area, loop_length, center, .. } => {
                assert!((loop_length - 20.0 * PI).abs() < 1e-9);
                let c = center.expect("a round face has a centre");
                assert!(c.x.abs() < 1e-9 && c.y.abs() < 1e-9);
                Some(area)
            }
            _ => None,
        }).collect();
        assert!(caps.len() == 2 && caps.iter().all(|a| (a - 100.0 * PI).abs() < 1e-6), "{caps:?}");
        let rims: Vec<&BodyEdge> = b.edges.iter().filter(|e| matches!(e.geom, Geom::Circle { .. })).collect();
        assert_eq!(rims.len(), 2);
        for e in rims {
            let rows = measure::describe(&e.geom, 0);
            assert_eq!(rows[0].label, "Circumference");
            assert!((rows[0].value - 20.0 * PI).abs() < 1e-9 && (rows[1].value - 10.0).abs() < 1e-9 && (rows[2].value - 20.0).abs() < 1e-9);
        }
        // Rim to rim: 20 apart, parallel.
        let pick = |i: usize| {
            let e = b.edges.iter().filter(|e| matches!(e.geom, Geom::Circle { .. })).nth(i).unwrap();
            let pts: Vec<DVec3> = e.points.iter().map(|p| p.as_dvec3()).collect();
            measure::Picked {
                pick: measure::Pick::Edge { body: 0, edge: i },
                geom: e.geom.clone(),
                samples: measure::Samples { segments: pts.windows(2).map(|w| [w[0], w[1]]).collect(), ..Default::default() },
            }
        };
        let (rows, _) = measure::compare(&pick(0), &pick(1));
        assert!((rows[0].value - 20.0).abs() < 1e-6, "{rows:?}");
        assert_eq!(rows.iter().find(|r| r.label == "Angle").unwrap().value, 0.0);
        assert!(rows.iter().any(|r| r.label == "Center distance" && (r.value - 20.0).abs() < 1e-9));
    }

    #[test]
    fn measure_fillet_face_is_a_cylinder_of_the_fillet_radius() {
        let m = plate_with(10.0, Some((EdgeKind::Fillet, 2.0, top_front(10.0))));
        assert_eq!(m.edge_errors, vec![None]);
        let b = &m.bodies[0];
        let r = b.faces.iter().find_map(|g| match *g {
            Geom::Cylinder { radius, area, .. } => Some((radius, area)),
            // (The other faces of the plate stay flat.)
            _ => None,
        });
        // A quarter cylinder of r = 2 along 50 mm: area πr/2 · 50.
        let (radius, area) = r.expect("fillet face");
        assert!((radius - 2.0).abs() < 1e-9 && (area - PI * 50.0).abs() < 1e-6, "{radius} {area}");
    }

    #[test]
    fn flat_faces_found_and_curved_ones_skipped() {
        let m = plate_with(10.0, None);
        assert_eq!(m.bodies[0].flat_faces.len(), 6, "a box has six flat faces");
        let top = m.bodies[0].flat_faces.iter().find(|(n, _)| n.z > 0.99).unwrap();
        assert!((top.1.z - 10.0).abs() < 1e-9, "top face at z = 10, normal pointing out");
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        circle(&mut sk, 0.0, 0.0, 10.0);
        let c = build4(&[sk], &[ex(0, vec![0], 20.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(c.bodies[0].flat_faces.len(), 2, "a cylinder: top and bottom, not its side");
    }

    #[test]
    fn sketch_on_a_face_builds_on_the_part() {
        use crate::sketch::FacePlane;
        // Plate 50×30×10, then a sketch on its top face.
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let top = FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0));
        let mut b = Sketch::on_face("B".into(), top);
        rect(&mut b, 10.0, 10.0, 20.0, 20.0);
        let two = |distance, op| {
            let m = build4(
                &[a.clone(), b.clone()],
                &[ex(0, vec![0], 10.0, Operation::NewBody), ex(1, vec![0], distance, op)],
                &[],
                &[Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1)],
            );
            assert_eq!(m.sketch_errors, vec![None, None], "the face sketch found its face");
            m
        };
        // A boss standing on the top face joins into one body.
        let m = two(5.0, Operation::Join);
        assert_eq!(m.bodies.len(), 1);
        assert!(close(total(&m), 15000.0 + 500.0));
        let bb = opencascade::bounding_box::aabb(&m.bodies[0].shape);
        assert!((bb.max().z - 15.0).abs() < 1e-6, "boss rises from z = 10 to 15");
        // A pocket cut down into it (negative = into the part).
        let m = two(-4.0, Operation::Cut);
        assert!(close(total(&m), 15000.0 - 400.0));
    }

    #[test]
    fn face_sketch_follows_its_face_when_the_part_changes() {
        use crate::sketch::FacePlane;
        // Sketch picked on the top of a 10-thick plate…
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let picked = FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0));
        let mut b = Sketch::on_face("B".into(), picked);
        rect(&mut b, 10.0, 10.0, 20.0, 20.0);
        // …then the plate is made 12 thick.
        // Timeline as the app makes it: the face sketch comes after the plate.
        let m = build4(
            &[a.clone(), b.clone()],
            &[ex(0, vec![0], 12.0, Operation::NewBody), ex(1, vec![0], 5.0, Operation::Join)],
            &[],
            &[Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1)],
        );
        assert_eq!(m.sketch_errors, vec![None, None]);
        let now = m.sketch_frames[1].unwrap();
        assert!((now.origin.z - 12.0).abs() < 1e-9, "sketch moved up with the face");
        assert_eq!((now.u, now.v), (picked.u, picked.v), "same axes, so the sketch doesn't rotate");
        assert_eq!(m.bodies.len(), 1, "the boss still sits on the plate and joins it");
        let bb = opencascade::bounding_box::aabb(&m.bodies[0].shape);
        assert!((bb.max().z - 17.0).abs() < 1e-6);
        assert!(close(total(&m), 50.0 * 30.0 * 12.0 + 500.0));
        assert!(m.sketch_frames[0].is_none(), "origin-plane sketches aren't moved");
        // A big change: 10 → 40 thick. The sketch sits over the top face, so
        // it's still found.
        let m = build4(
            &[a.clone(), b.clone()],
            &[ex(0, vec![0], 40.0, Operation::NewBody), ex(1, vec![0], 5.0, Operation::Join)],
            &[],
            &[Feature::Sketch(0), Feature::Extrude(0), Feature::Sketch(1), Feature::Extrude(1)],
        );
        assert_eq!(m.sketch_errors, vec![None, None]);
        assert!((m.sketch_frames[1].unwrap().origin.z - 40.0).abs() < 1e-9);
        assert_eq!(m.bodies.len(), 1);
        // If the face is gone (no plate), the sketch reports it and the boss
        // fails instead of floating where the face used to be.
        let m = build4(&[a, b], &[ex(1, vec![0], 5.0, Operation::NewBody)], &[], &[Feature::Sketch(0), Feature::Sketch(1), Feature::Extrude(0)]);
        assert!(m.sketch_errors[1].is_some());
        assert!(m.errors[0].is_some());
        assert!(m.bodies.is_empty(), "no floating body");
    }

    #[test]
    fn face_sketch_picks_the_face_it_sits_on_among_stepped_faces() {
        use crate::sketch::FacePlane;
        // Plate 50×30×10 with a 10×30 block on its right end rising to z = 30.
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let mut step = Sketch::new("Step".into(), OriginPlane::XY);
        rect(&mut step, 40.0, 0.0, 50.0, 30.0);
        // A sketch picked on the plate's top (z = 10), over x 5..15.
        let mut b = Sketch::on_face("B".into(), FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 10.0)));
        rect(&mut b, 5.0, 5.0, 15.0, 15.0);
        // The plate is then made 25 or 28 thick, close to the block's top at
        // z = 30. Both tops face up; the sketch must land on the plate's top,
        // the face it was drawn over.
        for thick in [25.0, 28.0] {
            let m = build4(
                &[a.clone(), step.clone(), b.clone()],
                &[
                    ex(0, vec![0], thick, Operation::NewBody),
                    ex(1, vec![0], 30.0, Operation::Join),
                    ex(2, vec![0], 2.0, Operation::Join),
                ],
                &[],
                &[
                    Feature::Sketch(0),
                    Feature::Extrude(0),
                    Feature::Sketch(1),
                    Feature::Extrude(1),
                    Feature::Sketch(2),
                    Feature::Extrude(2),
                ],
            );
            assert!((m.sketch_frames[2].unwrap().origin.z - thick).abs() < 1e-9, "{thick}");
        }
    }

    fn revolve(sketch: usize, profiles: Vec<usize>, axis: RevolveAxis, angle: f64, op: Operation) -> Revolve {
        Revolve { op, name: "R".into(), sketch, profiles, axis, angle }
    }

    /// A 10 × 20 rectangle from x = 5 to 15 on the XY plane.
    fn ring_profile() -> Sketch {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 5.0, 0.0, 15.0, 20.0);
        sk
    }

    #[test]
    fn revolve_full_half_and_backwards() {
        // Spun around the sketch's Y axis: a tube, inner r 5, outer r 15, 20 tall.
        let tube = PI * (15.0 * 15.0 - 5.0 * 5.0) * 20.0;
        for (angle, want) in [(360.0, tube), (180.0, tube / 2.0), (-90.0, tube / 4.0)] {
            let m = build(
                &[ring_profile()],
                &[],
                &[revolve(0, vec![0], RevolveAxis::SketchY, angle, Operation::NewBody)],
                &[],
                &[Feature::Sketch(0), Feature::Revolve(0)],
            );
            assert_eq!(m.revolve_errors, vec![None], "{angle}");
            assert!(close(total(&m), want), "{angle}°: {} vs {want}", total(&m));
        }
    }

    #[test]
    fn revolve_around_a_sketch_line() {
        // Axis: a vertical line at x = -10. Profile 5..15 → radii 15..25.
        let mut sk = ring_profile();
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, free(-10.0, 0.0));
        ed.click(&mut sk, free(-10.0, 30.0));
        let axis = sk.entities.len() - 1;
        let m = build(
            &[sk],
            &[],
            &[revolve(0, vec![0], RevolveAxis::Line(axis), 360.0, Operation::NewBody)],
            &[],
            &[Feature::Sketch(0), Feature::Revolve(0)],
        );
        assert_eq!(m.revolve_errors, vec![None]);
        assert!(close(total(&m), PI * (25.0 * 25.0 - 15.0 * 15.0) * 20.0));
        assert!(m.bodies[0].shape.faces().count() >= 4);
    }

    #[test]
    fn revolve_refuses_a_profile_across_its_axis() {
        // Profile x = -5..5 straddles the Y axis.
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, -5.0, 0.0, 5.0, 10.0);
        let m = build(
            &[sk],
            &[],
            &[revolve(0, vec![0], RevolveAxis::SketchY, 360.0, Operation::NewBody)],
            &[],
            &[Feature::Sketch(0), Feature::Revolve(0)],
        );
        assert!(m.revolve_errors[0].as_deref().is_some_and(|e| e.contains("crosses")));
        assert!(m.bodies.is_empty());
    }

    #[test]
    fn revolve_touching_the_axis_makes_a_solid_cylinder() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk, 0.0, 0.0, 10.0, 20.0);
        let m = build(
            &[sk],
            &[],
            &[revolve(0, vec![0], RevolveAxis::SketchY, 360.0, Operation::NewBody)],
            &[],
            &[Feature::Sketch(0), Feature::Revolve(0)],
        );
        assert_eq!(m.revolve_errors, vec![None]);
        assert!(close(total(&m), PI * 100.0 * 20.0));
        assert!(closed(&m.bodies[0].mesh));
    }

    #[test]
    fn revolve_can_cut_a_groove() {
        // A cylinder r 20 along Y, then a 2×4 notch profile at its surface spun into a groove.
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 20.0, 40.0);
        let mut b = Sketch::new("B".into(), OriginPlane::XY);
        rect(&mut b, 18.0, 18.0, 22.0, 22.0); // r 18..22, y 18..22
        let m = build(
            &[a, b],
            &[],
            &[
                revolve(0, vec![0], RevolveAxis::SketchY, 360.0, Operation::NewBody),
                revolve(1, vec![0], RevolveAxis::SketchY, 360.0, Operation::Cut),
            ],
            &[],
            &[Feature::Sketch(0), Feature::Revolve(0), Feature::Sketch(1), Feature::Revolve(1)],
        );
        assert_eq!(m.revolve_errors, vec![None, None]);
        let groove = PI * (20.0 * 20.0 - 18.0 * 18.0) * 4.0;
        assert!(close(total(&m), PI * 400.0 * 40.0 - groove));
    }

    /// Revolve the 5..15 × 0..20 ring profile by `angle` around the sketch Y axis.
    fn quarter_tube(angle: f64) -> (Sketch, Revolve) {
        (ring_profile(), revolve(0, vec![0], RevolveAxis::SketchY, angle, Operation::NewBody))
    }

    #[test]
    fn sketch_on_a_revolve_end_swings_with_the_angle() {
        use crate::sketch::FacePlane;
        // Pick the far end face of a 90° revolve, the way the app does.
        let (s0, r0) = quarter_tube(90.0);
        let m = build(&[s0.clone()], &[], &[r0.clone()], &[], &[Feature::Sketch(0), Feature::Revolve(0)]);
        let tol = 1e-6;
        let (n, q) = *m.bodies[0]
            .flat_faces
            .iter()
            .find(|(n, q)| follow_for_plane(&m.caps, *n, *q, tol).is_some() && n.y.abs() < 0.5)
            .expect("the far end face is a cap");
        let mut face = FacePlane::from_face(n, q);
        face.follow = follow_for_plane(&m.caps, n, q, tol);
        assert_eq!(face.follow, Some(Follow::Revolve { feature: 0, angle: 90.0 }));
        // A 4 × 10 rectangle on that end face (in its own coordinates), boss 3 out.
        let mut s1 = Sketch::on_face("B".into(), face);
        let corner = |y: f64, r: f64| {
            // a point on the end face at height y and distance r from the axis
            let p = q + DVec3::Y * (y - q.y) + (q - DVec3::new(0.0, q.y, 0.0)).normalize() * (r - (q.x * q.x + q.z * q.z).sqrt());
            DVec2::new(p.dot(face.u) - face.origin.dot(face.u), p.dot(face.v) - face.origin.dot(face.v))
        };
        let (a, b) = (corner(5.0, 8.0), corner(15.0, 12.0));
        rect(&mut s1, a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
        let boss = ex(1, vec![0], 3.0, Operation::Join);
        let timeline = [Feature::Sketch(0), Feature::Revolve(0), Feature::Sketch(1), Feature::Extrude(0)];
        let tube = |deg: f64| PI * (15.0 * 15.0 - 5.0 * 5.0) * 20.0 * deg / 360.0;
        let at90 = build(&[s0.clone(), s1.clone()], &[boss.clone()], &[r0.clone()], &[], &timeline);
        assert_eq!(at90.bodies.len(), 1);
        assert!(close(total(&at90), tube(90.0) + 4.0 * 10.0 * 3.0), "{}", total(&at90));
        // Now 80°: the end face swings back 10°; the sketch and boss go with it.
        let r80 = Revolve { angle: 80.0, ..r0 };
        let at80 = build(&[s0, s1], &[boss], &[r80], &[], &timeline);
        assert_eq!(at80.sketch_errors, vec![None, None]);
        assert_eq!(at80.errors, vec![None]);
        assert_eq!(at80.bodies.len(), 1, "boss still joined to the tube");
        assert!(close(total(&at80), tube(80.0) + 4.0 * 10.0 * 3.0), "{}", total(&at80));
    }

    #[test]
    fn fillet_on_a_revolve_end_survives_an_angle_change() {
        let (s0, r0) = quarter_tube(90.0);
        let m = build(&[s0.clone()], &[], &[r0.clone()], &[], &[Feature::Sketch(0), Feature::Revolve(0)]);
        // The outer straight edge of the far end face (radius 15, running along Y).
        let mut picked = m.bodies[0]
            .edges
            .iter()
            .map(|e| e.at)
            .find(|r| {
                r.dir.dot(DVec3::Y).abs() > 0.99
                    && follow_for_edge(&m.caps, r.mid, r.dir, 1e-6).is_some()
                    && (r.mid.x * r.mid.x + r.mid.z * r.mid.z).sqrt() > 14.0
            })
            .expect("outer edge of the end face");
        picked.follow = follow_for_edge(&m.caps, picked.mid, picked.dir, 1e-6);
        let fillet = EdgeFeature { name: "F".into(), kind: EdgeKind::Fillet, size: 1.0, size2: None, angle: None, edges: vec![picked] };
        let timeline = [Feature::Sketch(0), Feature::Revolve(0), Feature::Edge(0)];
        // The edge sits between a flat end and the curved outside, so the
        // material removed isn't the straight-edge formula — but it is the same
        // small amount whatever the angle, since the same edge is rounded.
        let mut removed = Vec::new();
        for angle in [90.0, 80.0, 120.0] {
            let rv = Revolve { angle, ..r0.clone() };
            let m = build(&[s0.clone()], &[], &[rv], &[fillet.clone()], &timeline);
            assert_eq!(m.edge_errors, vec![None], "{angle}°");
            let tube = PI * (15.0 * 15.0 - 5.0 * 5.0) * 20.0 * angle / 360.0;
            removed.push(tube - total(&m));
        }
        assert!(removed.iter().all(|&r| r > 0.1 && r < 1.0 * 1.0 * 20.0), "{removed:?}");
        assert!(removed.iter().all(|&r| (r - removed[0]).abs() < 1e-6 * 100.0), "{removed:?}");
    }

    #[test]
    fn follow_indices_stay_right_when_features_are_deleted() {
        let f = Follow::Revolve { feature: 2, angle: 90.0 };
        assert_eq!(f.after_delete(true, 2), None, "its revolve deleted");
        assert_eq!(f.after_delete(true, 0), Some(Follow::Revolve { feature: 1, angle: 90.0 }));
        assert_eq!(f.after_delete(true, 5), Some(f));
        assert_eq!(f.after_delete(false, 0), Some(f), "deleting an extrude doesn't touch it");
    }

    /// A "D": the upper half of a radius-10 circle closed by its diameter.
    fn d_shape() -> Sketch {
        let mut sk = Sketch::new("D".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Arc3Point);
        ed.click(&mut sk, free(10.0, 0.0));
        ed.click(&mut sk, free(-10.0, 0.0));
        ed.click(&mut sk, free(0.0, 10.0));
        ed.set_tool(Tool::Line);
        let (a, b) = match sk.entities[0] {
            crate::sketch::Entity::Arc { a, b, .. } => (a, b),
            _ => unreachable!(),
        };
        ed.click(&mut sk, Anchor::Point(b));
        ed.click(&mut sk, Anchor::Point(a));
        sk
    }

    #[test]
    fn arcs_close_profiles_and_extrude_exactly() {
        let sk = d_shape();
        let regions = find_regions(&sk);
        assert_eq!(regions.len(), 1, "the D is one closed region");
        let m = build4(&[sk], &[ex(0, vec![0], 4.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert_eq!(m.errors, vec![None]);
        assert!(close(total(&m), PI * 100.0 / 2.0 * 4.0), "{}", total(&m));
        assert!(closed(&m.bodies[0].mesh));
    }

    #[test]
    fn sketch_fillet_and_chamfer_give_exact_solids() {
        let base = || {
            let mut sk = Sketch::new("S".into(), OriginPlane::XY);
            rect(&mut sk, 0.0, 0.0, 40.0, 30.0);
            sk
        };
        // Round the corner at (40, 30) with r = 5.
        let mut sk = base();
        let corner = (0..sk.points.len()).find(|&p| sk.points[p] == DVec2::new(40.0, 30.0)).unwrap();
        let cs = sk.fillet_corner(corner, 5.0).unwrap();
        crate::solver::add_constraints(&mut sk, &cs).unwrap();
        let m = build4(&[sk], &[ex(0, vec![0], 2.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert!(close(total(&m), (1200.0 - 25.0 * (1.0 - PI / 4.0)) * 2.0), "{}", total(&m));
        // Bevel it by 5 instead.
        let mut sk = base();
        let cs = sk.chamfer_corner2(corner, 5.0, 5.0, None).unwrap();
        crate::solver::add_constraints(&mut sk, &cs).unwrap();
        let m = build4(&[sk], &[ex(0, vec![0], 2.0, Operation::NewBody)], &[], &timeline(1, 1));
        assert!(close(total(&m), (1200.0 - 12.5) * 2.0), "{}", total(&m));
    }

    #[test]
    fn step_export_is_exact_and_reads_back() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        circle(&mut sk, 0.0, 0.0, 10.0);
        let m = build4(&[sk], &[ex(0, vec![0], 30.0, Operation::NewBody)], &[], &timeline(1, 1));
        let path = std::env::temp_dir().join(format!("opencad-kernel-{}.step", std::process::id()));
        write_step(&path, &m.bodies.iter().collect::<Vec<_>>()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("CYLINDRICAL_SURFACE"), "true cylinder, not facets");
        let back = Shape::read_step(&path).unwrap();
        assert!(close(back.volume(), PI * 100.0 * 30.0));
        std::fs::remove_file(&path).ok();
    }
}
