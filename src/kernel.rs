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
use crate::model::{CylinderRef, ThreadFeature, HoleExtent, HoleFeature, HolePlace, TapType, Hinge, PlaneAngle, PlaneFeature, AxisFeature, CopyFeature, ExtrudeTarget, RemoveFeature, Placement, EdgeFeature, EdgeKind, EdgeRef, Extrude, Follow, Mesh, Operation, Revolve, RevolveAxis, mesh_edges};
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
    }
}

/// The evaluated design.
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
    /// Threads shown on holes (not cut): drawn as a helix.
    pub threads: Vec<CosmeticThread>,
    /// Far-end faces of the extrudes/revolves, to tag what's picked there.
    pub caps: Vec<Cap>,
    /// For each extrude / revolve / fillet that was built: the body it made or
    /// changed (the first, if several). The timeline colours its icon by it.
    pub touched: HashMap<Feature, BodyId>,
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
fn loop_wire(plane: &Frame, pts: &[DVec2], curves: &[Curve]) -> Result<Wire, String> {
    let n = pts.len();
    if n < 2 || curves.len() != n {
        return Err("malformed profile".into());
    }
    let p3 = |p: DVec2| kv(world(plane, p));
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
    for e in shape.edges() {
        let pts: Vec<DVec3> = e.approximation_segments().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
        let Some(r) = polyline_mid(&pts) else { continue };
        // The explorer visits shared edges once per face; keep the first.
        if out.iter().any(|(_, o, _)| o.mid.distance(r.mid) < 1e-9) {
            continue;
        }
        out.push((e, r, pts));
    }
    // Which faces each edge belongs to: walk every face's edges and match
    // them to the list above by their middle point.
    let mut normals: Vec<Vec<DVec3>> = vec![Vec::new(); out.len()];
    for face in shape.faces() {
        for e in face.edges() {
            let pts: Vec<DVec3> = e.approximation_segments().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
            let Some(r) = polyline_mid(&pts) else { continue };
            if let Some(i) = out.iter().position(|(_, o, _)| o.mid.distance(r.mid) < 1e-9) {
                let n = face.normal_at(kv(r.mid));
                let n = DVec3::new(n.x, n.y, n.z).normalize();
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
    let near_sharp = |p: DVec3| {
        sharp.iter().any(|&(a, b)| {
            let ab = b - a;
            let t = ((p - a).dot(ab) / ab.length_squared().max(1e-30)).clamp(0.0, 1.0);
            p.distance(a + ab * t) < tol
        })
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
        let done = match (f.kind, two) {
            (EdgeKind::Fillet, _) => b.try_fillet_edges(f.size, edges),
            (EdgeKind::Chamfer, None) => b.try_chamfer_edges(f.size, edges),
            (EdgeKind::Chamfer, Some(d2)) => {
                if !d2.is_finite() || d2 <= 0.0 {
                    return Err("chamfer distance must be greater than zero".into());
                }
                let with_faces: Option<Vec<(&Edge, Face)>> =
                    picked[i].iter().map(|&(j, want)| edge_face(&b, &all[j].2, want).map(|face| (&all[j].1, face))).collect();
                let with_faces = with_faces.ok_or("couldn't find the face a chamfer distance is measured on")?;
                b.try_chamfer_edges_two(f.size, d2, with_faces)
            }
        }
        .filter(|s| s.volume() > MIN_VOLUME)
        .ok_or_else(|| format!("{} {} is too large for these edges", f.kind.size_label().to_lowercase(), f.size))?;
        out.extend(done.solids().into_iter().map(|s| (s, ids[i])));
    }
    let touched = (0..ids.len()).find(|&i| !picked[i].is_empty()).map(|i| ids[i]);
    Ok((uniquify(out), touched))
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
    shape
        .faces()
        .map(|face| {
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
            if let Some((normal, _, _)) = face_flat(&face, tol) {
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
        })
        .collect()
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
    shape.faces().filter_map(|face| face_flat(&face, tol)).collect()
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

/// Where a construction axis is now: along the straight edge it was picked on.
fn resolve_axis(bodies: &Tagged, af: &AxisFeature) -> Result<(DVec3, DVec3), String> {
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
            let (o, d) = resolve_axis(bodies, &AxisFeature { name: String::new(), edge: r })?;
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
    /// Its major radius.
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

/// The face as a whole cylinder (both ends full circles), if it is one.
fn face_cylinder(face: &Face) -> Option<Cylinder> {
    let mut circles: Vec<(DVec3, DVec3, f64, DVec3)> = Vec::new();
    for e in face.edges() {
        let pts: Vec<DVec3> = e.approximation_segments().map(|p| DVec3::new(p.x, p.y, p.z)).collect();
        match edge_geom(&e, &pts) {
            Geom::Circle { center, normal, radius, sweep } if sweep > std::f64::consts::TAU - 1e-6 => circles.push((center, normal, radius, pts[0])),
            Geom::Line { .. } => {} // the seam
            _ => return None,
        }
    }
    let [(c0, n0, r0, p0), (c1, n1, r1, _)] = circles[..] else { return None };
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
            shown.push(CosmeticThread { origin: start + dir * from, axis: dir, radius: c.radius, pitch: t.pitch, length: to - from });
            continue;
        }
        let ((r_in, r_out), widths) = thread_profile(c.radius, c.internal, t.pitch);
        if r_in <= 0.0 {
            return Err("the cylinder is too thin for this thread".into());
        }
        // Swept past both ends, then trimmed to the span (so a shoulder isn't notched).
        let lead = 1.5 * t.pitch;
        let x = crate::model::zero_normal(dir, DVec3::Z);
        let swept = Shape::try_thread(kv(start + dir * (from - lead)), kv(dir), kv(x), (r_in, r_out), widths, t.pitch, to - from + 2.0 * lead, tf.spec.right_hand, 0.0)
            .ok_or("the kernel couldn't model this thread — try it cosmetic (Modeled off)")?;
        let span = Shape::cylinder(kv(start + dir * from), r_out + 1.0, kv(dir), to - from);
        let groove = swept.try_intersect(&span).ok_or("the kernel couldn't trim this thread")?;
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

/// What a hole cuts at each place: the bore, and a modeled thread's groove.
type HoleCuts = Vec<(Shape, Option<Shape>)>;

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
                let end = if h.extent == HoleExtent::All { depth } else { depth - 0.5 * t.pitch };
                if end - start < t.pitch {
                    return Err("the hole is too shallow for a modeled thread".into());
                }
                groove = Some(thread_groove(&t, p, dir, start, end, h.thread.right_hand)?);
            } else {
                threads.push(CosmeticThread { origin: p, axis: dir.normalize(), radius: t.major / 2.0, pitch: t.pitch, length: depth });
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
    build_all(sketches, extrudes, revolves, edge_features, copies, axis_features, plane_features, &[], &[], &[], timeline)
}

/// `build_with`, plus Remove features.
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
    timeline: &[Feature],
) -> Model {
    let mut hole_errors: Vec<Option<String>> = vec![None; holes.len()];
    let mut thread_errors: Vec<Option<String>> = vec![None; thread_features.len()];
    let mut threads: Vec<CosmeticThread> = Vec::new();
    let mut remove_errors: Vec<Option<String>> = vec![None; removes.len()];
    let mut planes: Vec<Option<FacePlane>> = vec![None; plane_features.len()];
    let mut plane_errors: Vec<Option<String>> = vec![None; plane_features.len()];
    let mut axes: Vec<Option<(DVec3, DVec3)>> = vec![None; axis_features.len()];
    let mut axis_errors: Vec<Option<String>> = vec![None; axis_features.len()];
    let mut copy_errors: Vec<Option<String>> = vec![None; copies.len()];
    let mut shapes: Tagged = Vec::new();
    let mut errors: Vec<Option<String>> = vec![None; extrudes.len()];
    let mut edge_errors: Vec<Option<String>> = vec![None; edge_features.len()];
    let mut revolve_errors: Vec<Option<String>> = vec![None; revolves.len()];
    let mut extrude_ends: Vec<Option<End>> = vec![None; extrudes.len()];
    let mut revolve_ends: Vec<Option<End>> = vec![None; revolves.len()];
    let mut frames: Vec<Option<FacePlane>> = sketches.iter().map(|s| s.face).collect();
    let mut sketch_errors: Vec<Option<String>> = vec![None; sketches.len()];
    let mut touched_by: HashMap<Feature, BodyId> = HashMap::new();
    for f in timeline {
        let current = std::mem::take(&mut shapes);
        // Keep a copy so a failed operation leaves the previous bodies intact.
        let backup: Tagged = current.iter().flat_map(|(s, id)| s.solids().into_iter().map(move |x| (x, *id))).collect();
        let (result, slot) = match *f {
            Feature::Plane(k) => {
                if let Some(a) = plane_features.get(k).and_then(|pf| pf.angle) {
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
                shapes = current;
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
                shapes = current;
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
                shapes = current;
                continue;
            }
            Feature::Extrude(k) => {
                let Some(ex) = extrudes.get(k) else {
                    shapes = current;
                    continue;
                };
                let ends = &mut extrude_ends;
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
                    shapes = current;
                    continue;
                };
                let ends = &mut revolve_ends;
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
                    match catch_unwind(AssertUnwindSafe(|| resolve_axis(&current, af))) {
                        Ok(Ok(a)) => axes[k] = Some(a),
                        Ok(Err(e)) => axis_errors[k] = Some(e),
                        Err(_) => axis_errors[k] = Some("the geometry kernel couldn't find its edge".into()),
                    }
                }
                shapes = current;
                continue;
            }
            Feature::Hole(k) => {
                let Some(h) = holes.get(k) else {
                    shapes = current;
                    continue;
                };
                let (fr, pl) = (&frames, &planes);
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
                        let Some(groove) = groove else {
                            let (next, t) = apply(acc, bore, Operation::Cut, Feature::Hole(k))?;
                            (acc, touched) = (next, touched.or(t));
                            continue;
                        };
                        // A modeled thread: what the bore and groove should take out
                        // together, measured on the part as it is (OCCT can fail quietly
                        // beside a helix; better an error than a wrong part).
                        let overlap = |a: &Shape| acc.iter().filter_map(|(s, _)| s.try_intersect(a)).map(|c| c.volume()).sum::<f64>();
                        let (ob, og) = (overlap(&bore), overlap(&groove));
                        let ogb: f64 = acc.iter().filter_map(|(s, _)| s.try_intersect(&groove)?.try_intersect(&bore)).map(|c| c.volume()).sum();
                        let want = ob + og - ogb;
                        let before = volume(&acc);
                        let (next, t1) = apply(acc, groove, Operation::Cut, Feature::Hole(k))?;
                        let (next, t2) = apply(next, bore, Operation::Cut, Feature::Hole(k))?;
                        let removed = before - volume(&next);
                        if (removed - want).abs() > 0.02 * want.max(1e-9) {
                            return Err("the kernel couldn't cut this thread — try it cosmetic (Modeled off)".into());
                        }
                        (acc, touched) = (next, touched.or(t1).or(t2));
                    }
                    Ok((acc, touched))
                }));
                if matches!(r, Ok(Ok(_))) {
                    threads.extend(shown);
                }
                (r, &mut hole_errors[k])
            }
            Feature::Thread(k) => {
                let Some(tf) = thread_features.get(k) else {
                    shapes = current;
                    continue;
                };
                let mut shown = Vec::new();
                let r = catch_unwind(AssertUnwindSafe(|| apply_threads(current, tf, Feature::Thread(k), &mut shown)));
                if matches!(r, Ok(Ok(_))) {
                    threads.extend(shown);
                }
                (r, &mut thread_errors[k])
            }
            Feature::Remove(k) => {
                // Take the bodies out; the rest carry on.
                if let Some(rf) = removes.get(k) {
                    let before = current.len();
                    let kept: Tagged = current.into_iter().filter(|(_, id)| !rf.bodies.contains(id)).collect();
                    if kept.len() == before {
                        remove_errors[k] = Some("the body it removes is gone".into());
                    }
                    shapes = kept;
                } else {
                    shapes = current;
                }
                continue;
            }
            Feature::Copy(k) => {
                let Some(cf) = copies.get(k) else {
                    shapes = current;
                    continue;
                };
                let (axes_now, planes_now) = (&axes, &planes);
                let r = catch_unwind(AssertUnwindSafe(|| apply_copy(current, cf, k, axes_now, planes_now)));
                (r, &mut copy_errors[k])
            }
            Feature::Edge(k) => {
                let Some(ef) = edge_features.get(k) else {
                    shapes = current;
                    continue;
                };
                let (ext, rev) = (&extrude_ends, &revolve_ends);
                let r = catch_unwind(AssertUnwindSafe(|| apply_edges(current, ef, ext, rev)));
                (r, &mut edge_errors[k])
            }
        };
        shapes = match result {
            Ok(Ok((next, touched))) => {
                if let Some(id) = touched {
                    touched_by.insert(*f, id);
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
    let bodies = shapes
        .into_iter()
        .filter_map(|(shape, id)| {
            let (mesh, tri_face) = catch_unwind(AssertUnwindSafe(|| mesh_tagged(&shape))).ok()?.ok()?;
            let edges = catch_unwind(AssertUnwindSafe(|| pickable_edges(&shape, &mesh))).unwrap_or_default();
            let flat_faces = catch_unwind(AssertUnwindSafe(|| flat_faces(&shape))).unwrap_or_default();
            let faces = catch_unwind(AssertUnwindSafe(|| face_geoms(&shape))).unwrap_or_default();
            Some(Body { id, shape, mesh, edges, flat_faces, faces, tri_face })
        })
        .collect();
    let caps = caps_of(&extrude_ends, &revolve_ends);
    Model { bodies, sketch_frames: frames, sketch_errors, errors, edge_errors, revolve_errors, copy_errors, remove_errors, axes, axis_errors, planes, plane_errors, hole_errors, thread_errors, threads, caps, touched: touched_by }
}

/// Exact STEP export of all bodies (true planes, cylinders, arcs).
pub fn write_step(path: &Path, bodies: &[Body]) -> Result<(), String> {
    Shape::write_all_step(bodies.iter().map(|b| &b.shape), path).map_err(|e| format!("STEP export failed: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};
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
        let axis = AxisFeature { name: "Axis1".into(), edge };
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
        let m = build_with(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[], &[AxisFeature { name: "A".into(), edge: nowhere }], &[], &t);
        // (No box edge runs along X near there… the nearest X edge is taken, so it resolves; a wrong direction can't.)
        let wrong = EdgeRef { mid: DVec3::new(6.0, 3.0, 2.5), dir: DVec3::new(0.0, 0.6, 0.8), faces: [DVec3::ZERO; 2], follow: None };
        assert!(m.axes.len() == 1);
        let mut sk2 = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut sk2, 2.0, 0.0, 6.0, 3.0);
        let m2 = build_with(&[sk2], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[], &[AxisFeature { name: "A".into(), edge: wrong }], &[], &t);
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
        let axis = AxisFeature { name: "Axis1".into(), edge };
        let rv = |axis: RevolveAxis| Revolve { op: Operation::NewBody, name: "R".into(), sketch: 1, profiles: vec![0], axis, angle: 360.0 };
        let t = vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Axis(0), Feature::Sketch(1), Feature::Revolve(0)];
        let m = build_with(&[sk.clone(), sk2.clone()], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[rv(RevolveAxis::Construction(0))], &[], &[], &[axis.clone()], &[], &t);
        assert_eq!(m.revolve_errors, vec![None], "{:?}", m.revolve_errors);
        // A ring, outer radius 4, inner 2, 2 mm long.
        assert!(close(total(&m), 60.0 + PI * (16.0 - 4.0) * 2.0), "{}", total(&m));
        // An axis standing up through the sketch plane can't be revolved about.
        let up = AxisFeature { name: "Up".into(), edge: EdgeRef { mid: DVec3::new(6.0, 3.0, 2.5), dir: DVec3::Z, faces: [DVec3::ZERO; 2], follow: None } };
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
        let plane = PlaneFeature { name: "P".into(), base: yz, on_origin: true, offset: 10.0, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None };
        let c = CopyFeature { name: "M".into(), bodies: vec![BodyId { feature: Feature::Extrude(0), piece: 0 }], kind: crate::model::CopyKind::MirrorPlane(0) };
        let t = [Feature::Sketch(0), Feature::Extrude(0), Feature::Plane(0), Feature::Copy(0)];
        let m = build_all(&[sk], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[c], &[], &[plane], &[], &[], &[], &t);
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
        let early = build_all(&[sk2], &[ex(0, vec![0], 5.0, Operation::NewBody)], &[], &[], &[c2], &[], &[PlaneFeature { name: "P".into(), base: yz, on_origin: true, offset: 10.0, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None }], &[], &[], &[], &[Feature::Sketch(0), Feature::Extrude(0), Feature::Copy(0), Feature::Plane(0)]);
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
            PlaneFeature { name: "P0".into(), base: xy, on_origin: true, offset: 2.0, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None },
            PlaneFeature { name: "P1".into(), base: top, on_origin: false, offset: 3.0, anchor: [4.0, 1.5], extent: [0.0; 4], angle: None },
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
            .map(|(kind, size, r)| EdgeFeature { name: "F".into(), kind, size, size2: None, edges: vec![r] })
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
        let f = EdgeFeature { name: "C".into(), kind: EdgeKind::Chamfer, size: d1, size2: Some(d2), edges: vec![r] };
        let mut tl = timeline(1, 1);
        tl.push(Feature::Edge(0));
        build4(&[a], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[f], &tl)
    }

    #[test]
    fn a_fillet_moved_before_its_body_is_an_error_the_app_can_refuse() {
        let mut a = Sketch::new("A".into(), OriginPlane::XY);
        rect(&mut a, 0.0, 0.0, 50.0, 30.0);
        let f = EdgeFeature { name: "F".into(), kind: EdgeKind::Fillet, size: 2.0, size2: None, edges: vec![picked_top_front(10.0)] };
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
        build_all(&[sk], &[ex(0, vec![0, 1], 5.0, Operation::NewBody)], &[], &[], &[], &[], &[], removes, &[], &[], tl)
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
        build_all(&sketches, &extrudes, &[], &[], &[], &[], planes, &[], &[], &[], &tl)
    }

    fn offset_plane(z: f64) -> crate::model::PlaneFeature {
        crate::model::PlaneFeature { name: "P".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: true, offset: z, anchor: [0.0, 0.0], extent: [0.0; 4], angle: None }
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
        PlaneFeature { name: "P".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: false, offset: 0.0, anchor: [0.0; 2], extent: [0.0; 4], angle: Some(PlaneAngle { hinge, degrees }) }
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
        let m = build_all(&[sk], &[e], &[], &[], &[], &[], &planes, &[], &[], &[], &[Feature::Plane(0), Feature::Sketch(0), Feature::Extrude(0)]);
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
        let m = build_all(&[sk], &[], &[], &[], &[], &[], &planes, &[], &[], &[], &[Feature::Sketch(0), Feature::Plane(0)]);
        assert_eq!(m.plane_errors, vec![None]);
        let f = m.planes[0].unwrap();
        assert!(f.normal().cross(DVec3::X).length() < 1e-12 && (f.origin - DVec3::new(4.0, 5.0, 0.0)).length() < 1e-9, "{f:?}");

        // A 50 × 30 × h plate; its top front edge (along X at y 0, z h), between
        // the top (0°) and front faces. At 45° the plane leans out over the front.
        let edge = EdgeRef { mid: DVec3::new(25.0, 0.0, 10.0), dir: DVec3::X, faces: [DVec3::Z, DVec3::NEG_Y], follow: None };
        let axes = [AxisFeature { name: "A".into(), edge }];
        let planes = [angled(Hinge::Edge(edge), 45.0), angled(Hinge::Axis(0), 0.0)];
        for h in [10.0, 20.0] {
            let mut b = Sketch::new("B".into(), OriginPlane::XY);
            rect(&mut b, 0.0, 0.0, 50.0, 30.0);
            let t = [Feature::Sketch(0), Feature::Extrude(0), Feature::Axis(0), Feature::Plane(0), Feature::Plane(1)];
            let m = build_all(&[b], &[ex(0, vec![0], h, Operation::NewBody)], &[], &[], &[], &axes, &planes, &[], &[], &[], &t);
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
        let m = build_all(&[Sketch::new("E".into(), OriginPlane::XY)], &[], &[], &[], &[], &[], &[angled(Hinge::SketchLine { sketch: 0, line: 0 }, 10.0)], &[], &[], &[], &[Feature::Sketch(0), Feature::Plane(0)]);
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
        build_all(&sketches, &[ex(0, vec![0], h, Operation::NewBody)], &[], &[], &[], &[], planes, &[], holes, &[], &tl)
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
        build_all(&[sk], &[ex(0, vec![0], len, Operation::NewBody)], &[], &[], &[], &[], &[], &[], &[], &tf, &tl)
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
        let m = build_all(&[sk], &[ex(0, vec![0], 10.0, Operation::NewBody)], &[], &[], &[], &[], &[], &[], &[hole], &[thread_on(face, "M6x1", true)], &tl);
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
    fn a_face_knows_its_middle() {
        let m = plate_with(10.0, None);
        let b = &m.bodies[0];
        let top = (0..b.faces.len()).find(|&f| b.face_tris(f).all(|t| t.iter().all(|p| (p.z - 10.0).abs() < 1e-9))).unwrap();
        assert!(b.face_centroid(top).unwrap().distance(DVec3::new(25.0, 15.0, 10.0)) < 1e-9);
    }

    #[test]
    fn holes_run_to_a_plane_and_take_screw_sizes() {
        // From the top (z 10) down to a plane at z 6: 4 deep.
        let plane = PlaneFeature { name: "P".into(), base: FacePlane::from_face(DVec3::Z, DVec3::ZERO), on_origin: true, offset: 6.0, anchor: [0.0; 2], extent: [0.0; 4], angle: None };
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
        assert!((m.threads[0].radius - 3.0).abs() < 1e-12 && m.threads[0].axis.distance(DVec3::NEG_Z) < 1e-12);
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
        let fillet = EdgeFeature { name: "F".into(), kind: EdgeKind::Fillet, size: 1.0, size2: None, edges: vec![picked] };
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
        write_step(&path, &m.bodies).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("CYLINDRICAL_SURFACE"), "true cylinder, not facets");
        let back = Shape::read_step(&path).unwrap();
        assert!(close(back.volume(), PI * 100.0 * 30.0));
        std::fs::remove_file(&path).ok();
    }
}
