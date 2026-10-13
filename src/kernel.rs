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
use crate::model::{PlaneFeature, AxisFeature, CopyFeature, Placement, EdgeFeature, EdgeKind, EdgeRef, Extrude, Follow, Mesh, Operation, Revolve, RevolveAxis, mesh_edges};
use crate::profile::{Curve, Region, find_regions, signed_area, union};
use crate::sketch::{FacePlane, Sketch};
use glam::{DQuat, DVec2, DVec3, Vec3};
use glam24::DVec3 as KVec;
use opencascade::primitives::{Edge, Face, Shape, Wire};
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
}

impl Model {
    /// How many features failed to build.
    pub fn error_count(&self) -> usize {
        let n = |v: &Vec<Option<String>>| v.iter().flatten().count();
        n(&self.sketch_errors) + n(&self.errors) + n(&self.edge_errors) + n(&self.revolve_errors) + n(&self.copy_errors) + n(&self.axis_errors) + n(&self.plane_errors)
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
    /// Each construction axis as built: a point on it and its direction.
    pub axes: Vec<Option<(DVec3, DVec3)>>,
    /// One entry per construction axis.
    pub axis_errors: Vec<Option<String>>,
    /// Each construction plane as built, offset included.
    pub planes: Vec<Option<FacePlane>>,
    /// One entry per construction plane.
    pub plane_errors: Vec<Option<String>>,
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
fn tool(sketches: &[Sketch], frames: &[Option<FacePlane>], ex: &Extrude) -> Result<(Shape, End), String> {
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
    let (from, to) = ex.span();
    if (to - from).abs() < 1e-6 {
        return Err("distance is zero".into());
    }
    let dir = kv(plane.normal() * (to - from));
    let mut out: Option<Shape> = None;
    for r in union(&picked) {
        let mut solid: Shape = region_face(&plane, &r)?.extrude(dir).into();
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
        .map(|(_, at, pts)| BodyEdge { points: pts.iter().map(|p| p.as_vec3()).collect(), at })
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
pub fn mesh(shape: &Shape) -> Result<Mesh, String> {
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
    for t in m.indices.chunks_exact(3) {
        let tri = [remap[t[0]], remap[t[1]], remap[t[2]]];
        if tri[0] != tri[1] && tri[1] != tri[2] && tri[0] != tri[2] {
            out.tris.push(tri);
        }
    }
    Ok(out)
}

/// The flat faces of a shape as (outward normal, point on it).
fn flat_faces(shape: &Shape) -> Vec<(DVec3, DVec3)> {
    flat_faces_with_outline(shape).into_iter().map(|(n, q, _)| (n, q)).collect()
}

/// The flat faces of a shape, with points around each face's boundary. A face
/// is flat when the surface normal is the same everywhere along its edges and
/// those edges all lie in one plane.
fn flat_faces_with_outline(shape: &Shape) -> Vec<(DVec3, DVec3, Vec<DVec3>)> {
    let bb = opencascade::bounding_box::aabb(shape);
    let tol = ((bb.max() - bb.min()).length() * 1e-7).max(1e-9);
    let mut out = Vec::new();
    for face in shape.faces() {
        let pts: Vec<DVec3> = face
            .edges()
            .flat_map(|e| e.approximation_segments().collect::<Vec<_>>())
            .map(|p| DVec3::new(p.x, p.y, p.z))
            .collect();
        if pts.len() < 3 {
            continue;
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
        if flat {
            out.push((n0, sample[0], pts));
        }
    }
    out
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
fn apply_copy(bodies: Tagged, cf: &CopyFeature, k: usize, axes: &[Option<(DVec3, DVec3)>]) -> Result<(Tagged, Option<BodyId>), String> {
    let sources: Vec<usize> = (0..bodies.len()).filter(|&i| cf.bodies.contains(&bodies[i].1)).collect();
    if sources.is_empty() {
        return Err("the bodies it copies are gone — edit it and pick again".into());
    }
    let placements = cf.kind.placements(axes)?;
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
                Placement::Reflect { normal } => src.mirrored_in_plane(kv(DVec3::ZERO), kv(normal)),
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
                if let Some(pf) = plane_features.get(k) {
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
                let r = catch_unwind(AssertUnwindSafe(|| {
                    let (shape, end) = tool(sketches, &frames, ex)?;
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
            Feature::Copy(k) => {
                let Some(cf) = copies.get(k) else {
                    shapes = current;
                    continue;
                };
                let axes_now = &axes;
                let r = catch_unwind(AssertUnwindSafe(|| apply_copy(current, cf, k, axes_now)));
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
            let mesh = catch_unwind(AssertUnwindSafe(|| mesh(&shape))).ok()?.ok()?;
            let edges = catch_unwind(AssertUnwindSafe(|| pickable_edges(&shape, &mesh))).unwrap_or_default();
            let flat_faces = catch_unwind(AssertUnwindSafe(|| flat_faces(&shape))).unwrap_or_default();
            Some(Body { id, shape, mesh, edges, flat_faces })
        })
        .collect();
    let caps = caps_of(&extrude_ends, &revolve_ends);
    Model { bodies, sketch_frames: frames, sketch_errors, errors, edge_errors, revolve_errors, copy_errors, axes, axis_errors, planes, plane_errors, caps, touched: touched_by }
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
        Extrude { op, name: "E".into(), sketch, profiles, distance, extent: Default::default(), distance2: 0.0 }
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
    fn construction_planes_offset_from_origin_planes_and_faces_carry_sketches() {
        use crate::model::PlaneFeature;
        // A 4×3×5 box at x 2..6. Plane 0: 2 mm above the XY plane. Plane 1: 3 mm above the box's top face.
        let mut base = Sketch::new("S".into(), OriginPlane::XY);
        rect(&mut base, 2.0, 0.0, 6.0, 3.0);
        let xy = FacePlane::from_face(DVec3::Z, DVec3::ZERO);
        let top = FacePlane::from_face(DVec3::Z, DVec3::new(4.0, 1.5, 5.0));
        let planes = [
            PlaneFeature { name: "P0".into(), base: xy, on_origin: true, offset: 2.0, anchor: [0.0, 0.0], extent: [0.0; 4] },
            PlaneFeature { name: "P1".into(), base: top, on_origin: false, offset: 3.0, anchor: [4.0, 1.5], extent: [0.0; 4] },
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
        let swapped = crate::doc::swapped(&tl, 2, false).unwrap();
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
