//! Solid features and their triangle meshes.
//! Bodies are rebuilt from sketches every time, so editing a sketch updates
//! every extrude that uses it — that's what makes the model parametric.

#[cfg(test)]
use crate::profile::{Region, find_regions, union};
#[cfg(test)]
use crate::sketch::{Plane, Sketch};
use crate::doc::BodyId;
use crate::sketch::{FacePlane, OriginPlane};
use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Indexed triangle mesh. Triangles are counter-clockwise seen from outside.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
}

impl Mesh {
    #[cfg(test)]
    pub fn append(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.tris.extend(other.tris.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
    }

    /// Unit normal of triangle `t` (outward).
    pub fn normal(&self, t: usize) -> Vec3 {
        let [a, b, c] = self.tris[t].map(|i| self.positions[i as usize]);
        (b - a).cross(c - a).normalize_or_zero()
    }

    /// Signed volume (positive when triangles face outward).
    #[cfg(test)]
    pub fn volume(&self) -> f64 {
        self.tris
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|i| self.positions[i as usize].as_dvec3());
                a.dot(b.cross(c)) / 6.0
            })
            .sum()
    }
}

/// Extrude one region along the plane normal by `distance` mm (negative = other side).
/// Mesh-only version, kept as a simple reference for tests; the app builds
/// bodies with the OpenCASCADE kernel (see kernel.rs).
#[cfg(test)]
pub fn extrude_region(region: &Region, plane: &Plane, distance: f64) -> Mesh {
    let (verts, cap) = region.triangulate();
    let n = verts.len() as u32;
    let offset = plane.normal() * distance as f32;
    let mut m = Mesh::default();
    m.positions.extend(verts.iter().map(|&p| plane.to_world(p))); // bottom: 0..n
    m.positions.extend(verts.iter().map(|&p| plane.to_world(p) + offset)); // top: n..2n

    for t in &cap {
        let [a, b, c] = t.map(|i| i as u32);
        m.tris.push([a + n, b + n, c + n]); // top faces +normal
        m.tris.push([a, c, b]); // bottom faces -normal
    }
    // Side walls. Outer loop is CCW and holes are CW, so the solid is always on
    // the left of travel and the wall's outward normal is to the right.
    let mut start = 0u32;
    for l in region.loops() {
        let len = l.len() as u32;
        for k in 0..len {
            let (i, j) = (start + k, start + (k + 1) % len);
            m.tris.push([i, j, j + n]);
            m.tris.push([i, j + n, i + n]);
        }
        start += len;
    }
    if distance < 0.0 {
        // Extruding backwards mirrors the solid, so flip every triangle.
        for t in &mut m.tris {
            t.swap(1, 2);
        }
    }
    m
}

/// What an extrude does to the bodies that already exist.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    /// Make a separate body.
    #[default]
    NewBody,
    /// Merge into the bodies it touches.
    Join,
    /// Remove its volume from every body.
    Cut,
    /// Keep only where it overlaps the bodies it touches.
    Intersect,
}

impl Operation {
    pub const ALL: [Operation; 4] = [Operation::NewBody, Operation::Join, Operation::Cut, Operation::Intersect];

    pub fn name(self) -> &'static str {
        match self {
            Operation::NewBody => "New Body",
            Operation::Join => "Join",
            Operation::Cut => "Cut",
            Operation::Intersect => "Intersect",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Extrude {
    /// Older files have no operation; they were all new bodies.
    #[serde(default)]
    pub op: Operation,
    pub name: String,
    pub sketch: usize,
    pub profiles: Vec<usize>,
    /// One side: `distance` from the sketch. Symmetric: `distance` in all, half each way.
    /// Two sides: `distance` one way and `distance2` the other.
    pub distance: f64,
    #[serde(default)]
    pub extent: Extent,
    #[serde(default)]
    pub distance2: f64,
    /// Degrees the sides lean in (positive) or out (negative). One side only.
    #[serde(default)]
    pub taper: f64,
}

/// How far an extrude reaches either side of its sketch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Extent {
    #[default]
    OneSide,
    Symmetric,
    TwoSides,
}

impl Extent {
    pub const ALL: [Extent; 3] = [Extent::OneSide, Extent::Symmetric, Extent::TwoSides];

    pub fn name(self) -> &'static str {
        match self {
            Extent::OneSide => "One Side",
            Extent::Symmetric => "Symmetric",
            Extent::TwoSides => "Two Sides",
        }
    }
}

impl Extrude {
    /// Where the solid starts and ends along the sketch normal: (from, to), signed.
    pub fn span(&self) -> (f64, f64) {
        let n = if self.distance < 0.0 { -1.0 } else { 1.0 };
        match self.extent {
            Extent::OneSide => (0.0, self.distance),
            Extent::Symmetric => (-self.distance / 2.0, self.distance / 2.0),
            Extent::TwoSides => (-n * self.distance2.abs(), self.distance),
        }
    }
}

/// Build the body for an extrude from the current sketch geometry (mesh-only, tests).
#[cfg(test)]
pub fn build_extrude(sketches: &[Sketch], ex: &Extrude) -> Result<Mesh, String> {
    let sk = sketches.get(ex.sketch).ok_or("its sketch was deleted")?;
    if ex.distance.abs() < 1e-6 {
        return Err("distance is zero".into());
    }
    let regions = find_regions(sk);
    let plane = sk.sketch_plane();
    let picked = ex
        .profiles
        .iter()
        .map(|&p| regions.get(p).ok_or("a selected profile no longer exists — the sketch changed"))
        .collect::<Result<Vec<&Region>, _>>()?;
    // Adjacent profiles merge into one outline, so the body has no internal walls.
    let mut mesh = Mesh::default();
    for r in union(&picked) {
        mesh.append(&extrude_region(&r, &plane, ex.distance));
    }
    if mesh.tris.is_empty() {
        return Err("no profiles selected".into());
    }
    Ok(mesh)
}

/// What a revolve spins around.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevolveAxis {
    /// The sketch's own X axis (through its origin).
    SketchX,
    /// The sketch's own Y axis (through its origin).
    #[default]
    SketchY,
    /// A line of the sketch (entity index).
    Line(usize),
    /// A construction axis (index into the design's axes); it must run parallel to the sketch.
    Construction(usize),
}

/// Spin sketch profiles around an axis.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Revolve {
    pub op: Operation,
    pub name: String,
    pub sketch: usize,
    pub profiles: Vec<usize>,
    pub axis: RevolveAxis,
    /// Degrees; 360 is a full turn, negative turns the other way.
    pub angle: f64,
}

/// A world axis, for patterns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldAxis {
    #[default]
    X,
    Y,
    Z,
}

impl WorldAxis {
    pub const ALL: [WorldAxis; 3] = [WorldAxis::X, WorldAxis::Y, WorldAxis::Z];

    pub fn dir(self) -> DVec3 {
        match self {
            WorldAxis::X => DVec3::X,
            WorldAxis::Y => DVec3::Y,
            WorldAxis::Z => DVec3::Z,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            WorldAxis::X => "X",
            WorldAxis::Y => "Y",
            WorldAxis::Z => "Z",
        }
    }
}

/// How a Mirror / Pattern feature copies its bodies.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum CopyKind {
    /// Reflect in an origin plane.
    Mirror(OriginPlane),
    /// `count` copies (the original included) every `spacing` mm along `dir`,
    /// and the same again along `dir2` (`count2` = 1 for a single row).
    Rectangular { dir: WorldAxis, count: u32, spacing: f64, dir2: WorldAxis, count2: u32, spacing2: f64 },
    /// `count` copies (the original included) around an axis through the
    /// origin, spread over `angle` degrees: 360 spaces them evenly all the
    /// way round, less puts the last copy at the end of the angle.
    Circular {
        axis: WorldAxis,
        /// A construction axis (index into the design's axes) instead of `axis`.
        #[serde(default)]
        custom: Option<usize>,
        count: u32,
        angle: f64,
        /// How much farther from the axis the copies sit than the original (mm).
        #[serde(default)]
        radial: f64,
    },
}

/// Where one copy goes relative to the original.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Placement {
    Translate(DVec3),
    /// About an axis through the origin, in radians.
    Rotate { origin: DVec3, axis: DVec3, radians: f64, radial: f64 },
    /// In the plane through the origin with this normal.
    Reflect { normal: DVec3 },
}

/// Copy bodies: mirrored in a plane, or repeated in a pattern. The copies
/// are new bodies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CopyFeature {
    pub name: String,
    pub bodies: Vec<BodyId>,
    pub kind: CopyKind,
}

/// Remove bodies from the design from this point in the timeline on
/// (Fusion's Remove: the features that made them stay).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoveFeature {
    pub name: String,
    pub bodies: Vec<BodyId>,
}

/// A construction axis: along a straight edge of a body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AxisFeature {
    pub name: String,
    pub edge: EdgeRef,
}

/// The area of a flat face in its plane's coordinates: (low x, low y, high x, high y).
pub fn face_extent(mesh: &Mesh, face: &FacePlane) -> [f64; 4] {
    let n = face.normal();
    let tol = 1e-4 * (1.0 + face.origin.length());
    let (mut lo, mut hi) = (glam::DVec2::splat(f64::MAX), glam::DVec2::splat(f64::MIN));
    for (t, tri) in mesh.tris.iter().enumerate() {
        if mesh.normal(t).as_dvec3().dot(n) < 0.9999 {
            continue;
        }
        for &i in tri {
            let p = mesh.positions[i as usize].as_dvec3();
            if (p - face.origin).dot(n).abs() > tol.max(1e-3) {
                continue;
            }
            let c = glam::DVec2::new(p.dot(face.u), p.dot(face.v));
            lo = lo.min(c);
            hi = hi.max(c);
        }
    }
    if lo.x > hi.x { [0.0; 4] } else { [lo.x, lo.y, hi.x, hi.y] }
}

/// A construction plane: a plane parallel to an origin plane or a body face,
/// `offset` mm along its normal.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaneFeature {
    pub name: String,
    /// The plane it is measured from, as picked.
    pub base: FacePlane,
    /// An origin plane stays put; a face's plane follows the face as the part changes.
    pub on_origin: bool,
    pub offset: f64,
    /// Where on the face it was picked, in the base plane's (u, v) — how the
    /// right face is found again after the part changes.
    #[serde(default)]
    pub anchor: [f64; 2],
    /// The face's area as shown (low x, low y, high x, high y in the base
    /// plane's coordinates): only what the picked plane is drawn over.
    #[serde(default)]
    pub extent: [f64; 4],
}

/// Most copies one feature may make (keeps a typo from hanging the app).
pub const MAX_COPIES: u32 = 200;

impl CopyKind {
    /// One placement per new copy (the original isn't one).
    /// `axes` are the construction axes as built: (point on it, direction).
    pub fn placements(&self, axes: &[Option<(DVec3, DVec3)>]) -> Result<Vec<Placement>, String> {
        match *self {
            CopyKind::Mirror(p) => {
                let n = p.plane().normal().as_dvec3();
                Ok(vec![Placement::Reflect { normal: n }])
            }
            CopyKind::Rectangular { dir, count, spacing, dir2, count2, spacing2 } => {
                let mut out = Vec::new();
                for j in 0..count2.clamp(1, MAX_COPIES) {
                    for i in 0..count.clamp(1, MAX_COPIES) {
                        if i == 0 && j == 0 {
                            continue;
                        }
                        out.push(Placement::Translate(dir.dir() * spacing * i as f64 + dir2.dir() * spacing2 * j as f64));
                    }
                }
                Ok(out)
            }
            CopyKind::Circular { axis, custom, count, angle, radial } => {
                let count = count.clamp(1, MAX_COPIES);
                if count < 2 {
                    return Ok(Vec::new());
                }
                let (origin, dir) = match custom {
                    Some(k) => axes.get(k).copied().flatten().ok_or("its construction axis couldn't be built")?,
                    None => (DVec3::ZERO, axis.dir()),
                };
                let full = angle.abs() >= 360.0 - 1e-9;
                let step = if full { angle / count as f64 } else { angle / (count - 1) as f64 };
                Ok((1..count).map(|i| Placement::Rotate { origin, axis: dir, radians: (step * i as f64).to_radians(), radial }).collect())
            }
        }
    }
}

/// Round (fillet) or bevel (chamfer) edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeKind {
    #[default]
    Fillet,
    Chamfer,
}

impl EdgeKind {
    pub fn name(self) -> &'static str {
        match self {
            EdgeKind::Fillet => "Fillet",
            EdgeKind::Chamfer => "Chamfer",
        }
    }

    /// What the size means for this kind.
    pub fn size_label(self) -> &'static str {
        match self {
            EdgeKind::Fillet => "Radius",
            EdgeKind::Chamfer => "Distance",
        }
    }
}

/// A face or edge on the far end of an extrude or revolve moves when that
/// feature's distance or angle changes (the cap slides or swings). A reference
/// picked there remembers which feature's end it was on and the value at that
/// time, so after a change it can be moved along by the same amount before it
/// is matched again.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Follow {
    Extrude { feature: usize, distance: f64 },
    Revolve { feature: usize, angle: f64 },
}

impl Follow {
    /// Keep indices right when an extrude (`revolve == false`) or revolve is
    /// deleted: a reference to it is dropped, later ones shift down.
    pub fn after_delete(self, revolve: bool, k: usize) -> Option<Follow> {
        match self {
            Follow::Extrude { feature, distance } if !revolve => match feature.cmp(&k) {
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(Follow::Extrude { feature: feature - 1, distance }),
                std::cmp::Ordering::Less => Some(self),
            },
            Follow::Revolve { feature, angle } if revolve => match feature.cmp(&k) {
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(Follow::Revolve { feature: feature - 1, angle }),
                std::cmp::Ordering::Less => Some(self),
            },
            other => Some(other),
        }
    }
}

/// How an edge is remembered between rebuilds: the point halfway along it and
/// its direction there. After the model rebuilds, the edge whose midpoint is
/// nearest (and that runs the same way) is taken to be the same edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EdgeRef {
    pub mid: DVec3,
    pub dir: DVec3,
    /// Which way the two faces meeting at the edge point (at its middle). A
    /// box's top-front and bottom-front edges run the same way but differ
    /// here, so the right one is found however far the part changes.
    /// Zero in older files (then only position and direction are used).
    #[serde(default)]
    pub faces: [DVec3; 2],
    /// Set when the edge was on the far end of an extrude or revolve.
    #[serde(default)]
    pub follow: Option<Follow>,
}

/// A fillet or chamfer on some edges.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EdgeFeature {
    pub name: String,
    pub kind: EdgeKind,
    /// Fillet radius or chamfer distance (mm).
    pub size: f64,
    /// A chamfer's second distance. `size` is measured on the first face of
    /// each edge reference (`EdgeRef::faces[0]`), this one on the other.
    /// None: the same distance both ways.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size2: Option<f64>,
    pub edges: Vec<EdgeRef>,
}

/// Smallest box around a mesh: (min, max).
pub fn mesh_bounds(m: &Mesh) -> Option<(Vec3, Vec3)> {
    let first = *m.positions.first()?;
    Some(m.positions.iter().fold((first, first), |(lo, hi), p| (lo.min(*p), hi.max(*p))))
}

/// Might an extruded profile (its outline `points`, pushed `distance` along
/// `normal`) touch any of the bodies, given as bounding boxes? A cheap test:
/// boxes that overlap or touch (within `eps`). New bodies that can't touch an
/// existing one should start as separate bodies, not joins.
pub fn prism_touches(points: &[Vec3], normal: Vec3, distance: f32, bodies: &[(Vec3, Vec3)], eps: f32) -> bool {
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for &p in points {
        for q in [p, p + normal * distance] {
            lo = lo.min(q);
            hi = hi.max(q);
        }
    }
    bodies.iter().any(|&(blo, bhi)| {
        lo.x <= bhi.x + eps && hi.x >= blo.x - eps && lo.y <= bhi.y + eps && hi.y >= blo.y - eps && lo.z <= bhi.z + eps && hi.z >= blo.z - eps
    })
}

/// Is `p` inside a closed mesh? Counts crossings along a ray in an odd
/// direction (so it doesn't graze edges of axis-aligned parts).
pub fn inside(mesh: &Mesh, p: Vec3) -> bool {
    let dir = Vec3::new(0.5773, 0.6131, 0.5392).normalize();
    let mut crossings = 0;
    for t in &mesh.tris {
        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
        let (e1, e2) = (b - a, c - a);
        let q = dir.cross(e2);
        let det = e1.dot(q);
        if det.abs() < 1e-12 {
            continue;
        }
        let s = p - a;
        let u = s.dot(q) / det;
        let r = s.cross(e1);
        let v = dir.dot(r) / det;
        if (0.0..=1.0).contains(&u) && v >= 0.0 && u + v <= 1.0 && e2.dot(r) / det > 0.0 {
            crossings += 1;
        }
    }
    crossings % 2 == 1
}

/// Distance along a ray to the first triangle it hits (Möller–Trumbore).
pub fn ray_hit(mesh: &Mesh, origin: Vec3, dir: Vec3) -> Option<f32> {
    ray_hit_tri(mesh, origin, dir).map(|(t, _)| t)
}

/// Like `ray_hit`, also saying which triangle was hit.
pub fn ray_hit_tri(mesh: &Mesh, origin: Vec3, dir: Vec3) -> Option<(f32, usize)> {
    let mut best: Option<(f32, usize)> = None;
    for (index, t) in mesh.tris.iter().enumerate() {
        let [a, b, c] = t.map(|i| mesh.positions[i as usize]);
        let (e1, e2) = (b - a, c - a);
        let p = dir.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-12 {
            continue;
        }
        let inv = 1.0 / det;
        let s = origin - a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            continue;
        }
        let q = s.cross(e1);
        let v = dir.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            continue;
        }
        let dist = e2.dot(q) * inv;
        if dist > 0.0 && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, index));
        }
    }
    best
}

/// A mesh edge with its one or two neighbouring triangles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshEdge {
    pub a: u32,
    pub b: u32,
    pub tris: [usize; 2],
    /// Neighbouring faces meet at a real corner (not a smooth curve facet).
    pub sharp: bool,
}

/// Edges for drawing outlines: sharp corners always, smooth facets only on the silhouette.
pub fn mesh_edges(m: &Mesh) -> Vec<MeshEdge> {
    let mut map: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
    for (t, tri) in m.tris.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (tri[k], tri[(k + 1) % 3]);
            map.entry((a.min(b), a.max(b))).or_default().push(t);
        }
    }
    let cos_sharp = 30f32.to_radians().cos();
    let mut out: Vec<MeshEdge> = map
        .into_iter()
        .map(|((a, b), ts)| {
            let t0 = ts[0];
            let t1 = *ts.get(1).unwrap_or(&t0);
            let sharp = t0 == t1 || m.normal(t0).dot(m.normal(t1)) < cos_sharp;
            MeshEdge { a, b, tris: [t0, t1], sharp }
        })
        .collect();
    out.sort_by_key(|e| (e.a, e.b)); // deterministic
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};
    use glam::DVec2;

    fn rect_sketch(plane: OriginPlane) -> Sketch {
        let mut sk = Sketch::new("S".into(), plane);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        for p in [(0.0, 0.0), (50.0, 0.0), (50.0, 30.0), (0.0, 30.0)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(p.0, p.1)));
        }
        ed.click(&mut sk, Anchor::Point(1));
        sk
    }

    /// Every directed edge must be matched by its reverse exactly once.
    fn is_closed(m: &Mesh) -> bool {
        let mut count: HashMap<(u32, u32), i32> = HashMap::new();
        for t in &m.tris {
            for k in 0..3 {
                *count.entry((t[k], t[(k + 1) % 3])).or_default() += 1;
            }
        }
        count.iter().all(|(&(a, b), &c)| c == 1 && count.get(&(b, a)) == Some(&1))
    }

    #[test]
    fn box_has_right_volume_and_faces_outward() {
        for plane in OriginPlane::ALL {
            let sk = rect_sketch(plane);
            let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
            let m = build_extrude(&[sk], &ex).unwrap();
            assert!((m.volume() - 50.0 * 30.0 * 20.0).abs() < 1e-2, "{plane:?}: {}", m.volume());
            assert!(is_closed(&m));
            assert_eq!(m.tris.len(), 12);
        }
    }

    #[test]
    fn a_faces_extent_is_where_the_face_lies_in_its_planes_coordinates() {
        // The box is 50 × 30 × 20 from the origin; its top face is z = 20.
        let sk = rect_sketch(OriginPlane::XY);
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        let m = build_extrude(&[sk], &ex).unwrap();
        let top = FacePlane::from_face(DVec3::Z, DVec3::new(10.0, 10.0, 20.0));
        let e = face_extent(&m, &top);
        assert_eq!(e, [0.0, 0.0, 50.0, 30.0]);
        // A side face (x = 50, normal +X) is 30 wide and 20 tall.
        let side = FacePlane::from_face(DVec3::X, DVec3::new(50.0, 10.0, 10.0));
        let e = face_extent(&m, &side);
        assert!(((e[2] - e[0]) - 30.0).abs() < 1e-9 || ((e[2] - e[0]) - 20.0).abs() < 1e-9, "{e:?}");
        // Nothing there: empty.
        assert_eq!(face_extent(&m, &FacePlane::from_face(DVec3::Z, DVec3::new(0.0, 0.0, 99.0))), [0.0; 4]);
    }

    #[test]
    fn negative_distance_goes_the_other_way_and_stays_outward() {
        let sk = rect_sketch(OriginPlane::XY);
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: -10.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        let m = build_extrude(&[sk], &ex).unwrap();
        assert!((m.volume() - 15000.0).abs() < 1e-2);
        assert!(m.positions.iter().all(|p| p.z <= 0.0));
        assert!(is_closed(&m));
    }

    #[test]
    fn plate_with_hole_is_closed_and_hollow() {
        let mut sk = rect_sketch(OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(25.0, 15.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(30.0, 15.0)));
        let regions = find_regions(&sk);
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 5.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        let m = build_extrude(&[sk], &ex).unwrap();
        assert!(is_closed(&m));
        assert!((m.volume() - regions[0].area() * 5.0).abs() < 1e-2);
    }

    #[test]
    fn belt_extrudes_as_one_closed_body_without_seams() {
        let mut sk = Sketch::new("S".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        for (cx, r) in [(0.0, 10.0), (40.0, 10.0)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(cx, 0.0)));
            ed.click(&mut sk, Anchor::Free(DVec2::new(cx + r, 0.0)));
        }
        ed.set_tool(Tool::Line);
        for y in [10.0, -10.0] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(-20.0, y)));
            ed.click(&mut sk, Anchor::Free(DVec2::new(60.0, y)));
            ed.cancel();
        }
        let regions = find_regions(&sk);
        assert_eq!(regions.len(), 3);
        let total: f64 = regions.iter().map(|r| r.area()).sum();
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0, 1, 2], distance: 15.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        let m = build_extrude(&[sk], &ex).unwrap();
        assert!(is_closed(&m), "one watertight body");
        assert!((m.volume() - total * 15.0).abs() < 1e-2);
        // Sharp edges are only the top and bottom outlines: no vertical seams.
        let vertical_sharp = mesh_edges(&m)
            .iter()
            .filter(|e| e.sharp && (m.positions[e.a as usize].z - m.positions[e.b as usize].z).abs() > 1e-3)
            .count();
        assert_eq!(vertical_sharp, 0);
    }

    #[test]
    fn cap_triangles_share_one_normal() {
        // Flat shading gives one colour per face only if every cap triangle's
        // normal is identical, however the cap was triangulated.
        let mut sk = rect_sketch(OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(25.0, 15.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(30.0, 15.0)));
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 7.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        let m = build_extrude(&[sk], &ex).unwrap();
        let caps: Vec<Vec3> = (0..m.tris.len()).map(|t| m.normal(t)).filter(|n| n.z.abs() > 0.5).collect();
        assert!(caps.len() > 4);
        // Within float rounding (1e-6 is far below one shade step of 1/255).
        assert!(caps.iter().all(|n| (n.z.abs() - 1.0).abs() < 1e-6 && n.x.abs() < 1e-6 && n.y.abs() < 1e-6));
    }

    #[test]
    fn ray_hits_the_nearest_face() {
        let sk = rect_sketch(OriginPlane::XY);
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        let m = build_extrude(&[sk], &ex).unwrap();
        // Straight down onto the top face (z = 20) from z = 100.
        let t = ray_hit(&m, Vec3::new(10.0, 10.0, 100.0), Vec3::NEG_Z).unwrap();
        assert!((t - 80.0).abs() < 1e-4);
        // Missing the box entirely.
        assert!(ray_hit(&m, Vec3::new(100.0, 10.0, 100.0), Vec3::NEG_Z).is_none());
        // Pointing away from it.
        assert!(ray_hit(&m, Vec3::new(10.0, 10.0, 100.0), Vec3::Z).is_none());
    }

    #[test]
    fn edges_find_box_corners_only() {
        let sk = rect_sketch(OriginPlane::XY);
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        let m = build_extrude(&[sk], &ex).unwrap();
        let sharp = mesh_edges(&m).iter().filter(|e| e.sharp).count();
        assert_eq!(sharp, 12, "a box has 12 edges; face diagonals are not sharp");
    }

    #[test]
    fn missing_profile_or_zero_distance_is_an_error() {
        let sk = rect_sketch(OriginPlane::XY);
        let bad = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![3], distance: 5.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        assert!(build_extrude(&[sk.clone()], &bad).is_err());
        let zero = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 0.0, extent: Default::default(), distance2: 0.0, taper: 0.0 };
        assert!(build_extrude(&[sk], &zero).is_err());
    }

    #[test]
    fn a_prism_far_from_every_body_does_not_touch_it() {
        let body = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(50.0, 30.0, 10.0));
        let square = |x: f32, y: f32| [Vec3::new(x, y, 0.0), Vec3::new(x + 5.0, y, 0.0), Vec3::new(x + 5.0, y + 5.0, 0.0), Vec3::new(x, y + 5.0, 0.0)];
        // Overlapping, and sitting on top (extruded up from the top face).
        assert!(prism_touches(&square(10.0, 10.0), Vec3::Z, 10.0, &[body], 1e-3));
        let on_top: Vec<Vec3> = square(10.0, 10.0).iter().map(|p| *p + Vec3::Z * 10.0).collect();
        assert!(prism_touches(&on_top, Vec3::Z, 10.0, &[body], 1e-3), "starts on the top face");
        // Off to the side, clear of it.
        assert!(!prism_touches(&square(100.0, 0.0), Vec3::Z, 10.0, &[body], 1e-3));
        // Extruding down from the face goes the other way: still touches.
        assert!(prism_touches(&on_top, Vec3::Z, -10.0, &[body], 1e-3));
        // No bodies: nothing to touch.
        assert!(!prism_touches(&square(0.0, 0.0), Vec3::Z, 10.0, &[], 1e-3));
    }

    #[test]
    fn mesh_bounds_cover_every_vertex() {
        let m = Mesh { positions: vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(-1.0, 5.0, 0.0)], tris: vec![] };
        assert_eq!(mesh_bounds(&m), Some((Vec3::new(-1.0, 2.0, 0.0), Vec3::new(1.0, 5.0, 3.0))));
        assert_eq!(mesh_bounds(&Mesh::default()), None);
    }
}
