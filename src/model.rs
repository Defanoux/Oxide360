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
    /// To Object: what it runs up to. Older files have none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ExtrudeTarget>,
}

/// What a To Object extrude runs up to: a plane, met wherever the extrude
/// reaches it (a slanted one cuts the end at its slant).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ExtrudeTarget {
    Origin(OriginPlane),
    /// A construction plane (index into the design's planes).
    Plane(usize),
    /// A flat face of a body, as picked, and where on it (in its plane's
    /// coordinates) — found again on every rebuild, so it follows the part.
    Face { face: FacePlane, anchor: [f64; 2] },
}

/// How far an extrude reaches either side of its sketch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Extent {
    #[default]
    OneSide,
    Symmetric,
    TwoSides,
    /// Up to a picked plane or flat face (Fusion's To Object).
    ToObject,
}

impl Extent {
    pub const ALL: [Extent; 4] = [Extent::OneSide, Extent::Symmetric, Extent::TwoSides, Extent::ToObject];

    pub fn name(self) -> &'static str {
        match self {
            Extent::OneSide => "One Side",
            Extent::Symmetric => "Symmetric",
            Extent::TwoSides => "Two Sides",
            Extent::ToObject => "To Object",
        }
    }
}

impl Extrude {
    /// The construction plane it runs up to, if any (used up: Fusion hides it).
    pub fn plane_used(&self) -> Option<usize> {
        match (self.extent, self.target) {
            (Extent::ToObject, Some(ExtrudeTarget::Plane(k))) => Some(k),
            _ => None,
        }
    }

    /// Where the solid starts and ends along the sketch normal: (from, to), signed.
    pub fn span(&self) -> (f64, f64) {
        let n = if self.distance < 0.0 { -1.0 } else { 1.0 };
        match self.extent {
            Extent::OneSide => (0.0, self.distance),
            Extent::Symmetric => (-self.distance / 2.0, self.distance / 2.0),
            Extent::TwoSides => (-n * self.distance2.abs(), self.distance),
            Extent::ToObject => (0.0, self.distance), // (the kernel works out where it stops)
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
    /// Reflect in a construction plane (index into the design's planes).
    MirrorPlane(usize),
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
    /// In the plane through `origin` with this normal.
    Reflect { origin: DVec3, normal: DVec3 },
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

/// A flat face as picked: its plane and where on it (found again every build).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickedFace {
    pub face: FacePlane,
    pub anchor: [f64; 2],
}

/// Which way a shell's wall goes from the part's surface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellDirection {
    #[default]
    Inside,
    Outside,
    Both,
}

/// Fusion's Shell: hollow a body, opened at the faces picked (or, for the
/// bodies listed, closed all round).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShellFeature {
    pub name: String,
    /// Faces taken away to make the opening.
    pub faces: Vec<PickedFace>,
    /// Bodies hollowed with no opening.
    pub bodies: Vec<BodyId>,
    pub direction: ShellDirection,
    pub inside: f64,
    pub outside: f64,
    /// Rounded offset corners (else sharp).
    pub rounded: bool,
}

impl Default for ShellFeature {
    fn default() -> Self {
        ShellFeature { name: "Shell1".into(), faces: Vec::new(), bodies: Vec::new(), direction: ShellDirection::Inside, inside: 2.0, outside: 2.0, rounded: false }
    }
}

impl ShellFeature {
    /// The wall thicknesses that apply: (inside, outside).
    pub fn walls(&self) -> Result<(f64, f64), String> {
        let (i, o) = match self.direction {
            ShellDirection::Inside => (self.inside, 0.0),
            ShellDirection::Outside => (0.0, self.outside),
            ShellDirection::Both => (self.inside, self.outside),
        };
        if !(i.is_finite() && o.is_finite() && i >= 0.0 && o >= 0.0 && i + o > 0.0) {
            return Err("the wall must be thicker than zero".into());
        }
        Ok((i, o))
    }
}

/// Fusion's Offset Face (and Press Pull on a flat face): the faces picked
/// move out along their normals by `distance` (negative: in), the faces
/// round them stretching to meet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OffsetFaceFeature {
    pub name: String,
    pub faces: Vec<PickedFace>,
    pub distance: f64,
}

impl Default for OffsetFaceFeature {
    fn default() -> Self {
        OffsetFaceFeature { name: "Offset Face1".into(), faces: Vec::new(), distance: 0.0 }
    }
}

/// How Move/Copy moves its bodies.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum MoveKind {
    /// Fusion's Free Move: turned `angles` degrees about X, then Y, then Z
    /// (through the middle of the bodies' bounding box), then moved `offset`.
    Free { offset: [f64; 3], angles: [f64; 3] },
    /// Turned `angle` degrees about an origin axis, or a construction axis
    /// (index into the design's axes) when `custom` is set.
    Rotate { axis: WorldAxis, custom: Option<usize>, angle: f64 },
}

/// Fusion's Move/Copy: bodies moved (they keep their identity), or moved
/// copies made (new bodies) when `copy` is set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MoveFeature {
    pub name: String,
    pub bodies: Vec<BodyId>,
    pub kind: MoveKind,
    pub copy: bool,
}

impl Default for MoveFeature {
    fn default() -> Self {
        MoveFeature { name: "Move1".into(), bodies: Vec::new(), kind: MoveKind::Free { offset: [0.0; 3], angles: [0.0; 3] }, copy: false }
    }
}

impl MoveKind {
    pub fn is_finite(&self) -> bool {
        match *self {
            MoveKind::Free { offset, angles } => offset.iter().chain(&angles).all(|v| v.is_finite()),
            MoveKind::Rotate { angle, .. } => angle.is_finite(),
        }
    }
}

/// Fusion's Combine: the tool bodies joined to, cut from or intersected with
/// the target body. The target keeps its identity; the tools go, unless kept.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CombineFeature {
    pub name: String,
    pub target: Option<crate::doc::BodyId>,
    pub tools: Vec<crate::doc::BodyId>,
    /// Join, Cut or Intersect (never New Body).
    pub op: Operation,
    pub keep_tools: bool,
}

impl Default for CombineFeature {
    fn default() -> Self {
        CombineFeature { name: "Combine1".into(), target: None, tools: Vec::new(), op: Operation::Join, keep_tools: false }
    }
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
    /// Plane at Angle: set, the plane runs through a line at an angle instead
    /// (and `base` / `offset` only remember how it looked when made).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<PlaneAngle>,
}

/// The line a Plane at Angle turns about.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Hinge {
    /// An origin axis.
    World(WorldAxis),
    /// A construction axis.
    Axis(usize),
    /// A straight body edge, found again every build.
    Edge(EdgeRef),
    /// A line of a sketch (follows the sketch).
    SketchLine { sketch: usize, line: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaneAngle {
    pub hinge: Hinge,
    pub degrees: f64,
}

impl Hinge {
    pub fn uses_axis(&self, k: usize) -> bool {
        *self == Hinge::Axis(k)
    }

    pub fn uses_sketch(&self, i: usize) -> bool {
        matches!(*self, Hinge::SketchLine { sketch, .. } if sketch == i)
    }

    /// Renumber after construction axis `k` (not this one's) is deleted.
    pub fn axis_deleted(&mut self, k: usize) {
        if let Hinge::Axis(j) = self
            && *j > k
        {
            *j -= 1;
        }
    }

    /// Renumber after sketch `i` (not this one's) is deleted.
    pub fn sketch_deleted(&mut self, i: usize) {
        if let Hinge::SketchLine { sketch, .. } = self
            && *sketch > i
        {
            *sketch -= 1;
        }
    }

    /// Which way the plane faces at 0° when nothing better is known: an
    /// origin axis lies in the XY plane (Z in XZ).
    pub fn world_normal(axis: WorldAxis) -> DVec3 {
        match axis {
            WorldAxis::X | WorldAxis::Y => DVec3::Z,
            WorldAxis::Z => DVec3::NEG_Y,
        }
    }
}

/// The plane's normal at 0°: `hint` made square to the line `d` (any square
/// direction if the hint runs along it).
pub fn zero_normal(d: DVec3, hint: DVec3) -> DVec3 {
    let d = d.normalize_or_zero();
    [hint, DVec3::Z, DVec3::X]
        .into_iter()
        .find_map(|h| (h - d * h.dot(d)).try_normalize().filter(|_| h.length() > 1e-9))
        .unwrap_or(DVec3::Z)
}

/// Plane at Angle: the plane through the line (`o`, `d`) that faces `n0` at
/// 0°, turned right-handed about `d` by `degrees`. Its u runs along the line,
/// its origin is `o`.
pub fn angle_frame(o: DVec3, d: DVec3, n0: DVec3, degrees: f64) -> Option<crate::sketch::FacePlane> {
    let u = d.try_normalize()?;
    let n0 = zero_normal(u, n0);
    let a = degrees.to_radians();
    let n = n0 * a.cos() + u.cross(n0) * a.sin();
    Some(crate::sketch::FacePlane { origin: o, u, v: n.cross(u), follow: None })
}

/// Where a hole goes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum HolePlace {
    /// A sketch point (a line end, a point, an arc or circle centre).
    SketchPoint { sketch: usize, point: usize },
    /// A sketch circle's centre; a Simple hole takes the circle's diameter.
    SketchCircle { sketch: usize, entity: usize },
    /// A point on a flat body face, kept as picked (found again every build,
    /// like a face sketch). `centre`: the middle of the face instead.
    Face { face: FacePlane, anchor: [f64; 2], centre: bool },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoleKind {
    #[default]
    Simple,
    Counterbore,
    Countersink,
}

/// Fusion's "Hole Tap Type".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TapType {
    /// The diameter as typed.
    #[default]
    Simple,
    /// The clearance diameter for a screw size and fit.
    Clearance,
    /// Drilled at the tap size for a thread (modeled, or shown cosmetically).
    Tapped,
    /// NPT: a 1:16 tapered tapped hole.
    TaperTapped,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HoleExtent {
    #[default]
    Distance,
    /// Up to a plane or face (`HoleFeature.target`).
    To,
    /// Right through everything.
    All,
}

/// A thread's choice from the library (holes; the Thread tool later).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadSpec {
    pub standard: crate::threads::Standard,
    /// e.g. "M6x1", "1/4-20 UNC".
    pub designation: String,
    pub class: String,
    pub right_hand: bool,
    /// Cut the thread into the part (else only drawn).
    pub modeled: bool,
    /// Clearance holes: how snug.
    #[serde(default)]
    pub fit: crate::threads::Fit,
}

impl Default for ThreadSpec {
    fn default() -> Self {
        ThreadSpec { standard: Default::default(), designation: "M6x1".into(), class: "6H".into(), right_hand: true, modeled: false, fit: Default::default() }
    }
}

impl ThreadSpec {
    pub fn thread(&self) -> Option<crate::threads::Thread> {
        crate::threads::Thread::find(&self.designation)
    }
}

/// A cylindrical face as picked: a point on its axis, the axis, its radius,
/// and where it was clicked (the thread starts at the end nearer that).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CylinderRef {
    pub axis_point: DVec3,
    pub axis: DVec3,
    pub radius: f64,
    pub at: DVec3,
    /// As picked: the inside of a hole (else a shaft), and the face's length
    /// (for drawing it; the kernel finds the face again every build).
    #[serde(default)]
    pub internal: bool,
    #[serde(default)]
    pub length: f64,
}

/// Fusion's Thread: threads on cylindrical faces (shafts or holes), drawn or
/// modeled. A timeline feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadFeature {
    pub name: String,
    pub faces: Vec<CylinderRef>,
    pub spec: ThreadSpec,
    /// The whole face; else `length` from `offset` (from the end nearer the click).
    pub full_length: bool,
    pub offset: f64,
    pub length: f64,
}

impl ThreadFeature {
    /// Where the thread runs along a face whose ends are at `z0` and `z1`
    /// along its axis, the click being nearer `z0`: (from, to).
    pub fn span(&self, z0: f64, z1: f64) -> Result<(f64, f64), String> {
        let s = if z1 >= z0 { 1.0 } else { -1.0 };
        let full = (z1 - z0).abs();
        if self.full_length {
            return Ok((z0, z1));
        }
        if !(self.offset >= 0.0 && self.length > 0.0 && self.offset < full) {
            return Err("the offset and length must fit on the face".into());
        }
        Ok((z0 + s * self.offset, z0 + s * (self.offset + self.length).min(full)))
    }
}

/// Every thread in the design that can be modeled: Tapped holes' threads and
/// Thread features (taper tapped holes are only ever shown, as in Fusion).
/// Returns (modeled, all).
pub fn thread_count(holes: &[HoleFeature], threads: &[ThreadFeature]) -> (usize, usize) {
    let tapped = holes.iter().filter(|h| h.tap == TapType::Tapped);
    let all = tapped.clone().count() + threads.len();
    let on = tapped.filter(|h| h.thread.modeled).count() + threads.iter().filter(|t| t.spec.modeled).count();
    (on, all)
}

/// Model every thread, or show them all cosmetically (e.g. model them all
/// just for an export, then switch back).
pub fn set_all_modeled(holes: &mut [HoleFeature], threads: &mut [ThreadFeature], on: bool) {
    for h in holes.iter_mut().filter(|h| h.tap == TapType::Tapped) {
        h.thread.modeled = on;
    }
    for t in threads {
        t.spec.modeled = on;
    }
}

/// Fusion's Hole: drilled holes (simple, counterbore, countersink; plain,
/// clearance or tapped) at one or more places. A timeline feature that cuts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HoleFeature {
    pub name: String,
    pub places: Vec<HolePlace>,
    pub kind: HoleKind,
    pub tap: TapType,
    pub extent: HoleExtent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<ExtrudeTarget>,
    /// From the face to the end of the full diameter (the drill tip is extra).
    pub depth: f64,
    /// Simple holes (circle places use the circle's own).
    pub diameter: f64,
    /// Drill point: None flat, else the tip's included angle in degrees.
    pub tip_angle: Option<f64>,
    pub cbore_diameter: f64,
    pub cbore_depth: f64,
    pub csink_diameter: f64,
    /// Included angle in degrees.
    pub csink_angle: f64,
    /// Drill out of the face instead of into it.
    pub flip: bool,
    #[serde(default)]
    pub thread: ThreadSpec,
    /// Tapped: threaded the whole depth (Fusion's Thread Offset: full), else
    /// `thread_length` in from the face.
    #[serde(default = "full_thread")]
    pub thread_full: bool,
    #[serde(default)]
    pub thread_length: f64,
}

fn full_thread() -> bool {
    true
}

impl Default for HoleFeature {
    fn default() -> Self {
        HoleFeature {
            name: "Hole1".into(),
            places: Vec::new(),
            kind: HoleKind::Simple,
            tap: TapType::Simple,
            extent: HoleExtent::Distance,
            target: None,
            depth: 10.0,
            diameter: 5.0,
            tip_angle: Some(118.0),
            cbore_diameter: 10.0,
            cbore_depth: 5.0,
            csink_diameter: 10.0,
            csink_angle: 90.0,
            flip: false,
            thread: ThreadSpec::default(),
            thread_full: true,
            thread_length: 8.0,
        }
    }
}

impl HoleFeature {
    /// How far in the thread runs for a hole `depth` deep.
    pub fn thread_depth(&self, depth: f64) -> f64 {
        if self.thread_full { depth } else { self.thread_length.clamp(0.0, depth) }
    }

    /// The drilled diameter at a place (None: the circle it was put on, which
    /// only the sketch knows). Tapped holes are drilled at the thread's minor
    /// (tap-drill) diameter, the thread cut beyond it.
    pub fn bore(&self, circle: Option<f64>) -> Result<f64, String> {
        let thread = || self.thread.thread().ok_or_else(|| format!("unknown thread {}", self.thread.designation));
        let d = match self.tap {
            TapType::Simple => circle.unwrap_or(self.diameter),
            TapType::Clearance => thread()?.clearance(self.thread.fit),
            TapType::Tapped | TapType::TaperTapped => thread()?.minor(),
        };
        if !(d.is_finite() && d > 0.0) {
            return Err("the diameter must be more than zero".into());
        }
        Ok(d)
    }

    /// The hole's half cross-section as (radius, depth) points, depth growing
    /// into the material, starting `lead` above the face (so the cut doesn't
    /// share a face with the part) and running `depth` (plus the drill tip).
    /// Closed along the axis (radius 0) at both ends.
    pub fn profile(&self, bore: f64, depth: f64, lead: f64) -> Result<Vec<glam::DVec2>, String> {
        use glam::DVec2;
        let r = bore / 2.0;
        if !(depth.is_finite() && depth > 0.0) {
            return Err("the depth must be more than zero".into());
        }
        let taper = if self.tap == TapType::TaperTapped { crate::threads::NPT_TAPER / 2.0 } else { 0.0 };
        // The bore's radius at depth z (a taper narrows it going in).
        let at = |z: f64| r - taper * z;
        let mut pts = vec![DVec2::new(0.0, -lead)];
        match self.kind {
            HoleKind::Simple => pts.push(DVec2::new(at(-lead), -lead)),
            HoleKind::Counterbore => {
                let (rc, dc) = (self.cbore_diameter / 2.0, self.cbore_depth);
                if rc <= r + 1e-9 {
                    return Err("the counterbore must be wider than the hole".into());
                }
                if !(dc > 0.0 && dc < depth) {
                    return Err("the counterbore must be shallower than the hole".into());
                }
                pts.extend([DVec2::new(rc, -lead), DVec2::new(rc, dc), DVec2::new(at(dc), dc)]);
            }
            HoleKind::Countersink => {
                let (rs, half) = (self.csink_diameter / 2.0, (self.csink_angle / 2.0).to_radians());
                if rs <= r + 1e-9 {
                    return Err("the countersink must be wider than the hole".into());
                }
                if !(half > 0.0 && half < std::f64::consts::FRAC_PI_2) {
                    return Err("the countersink angle must be between 0° and 180°".into());
                }
                // The cone meets the bore this deep (a taper's bore, near enough
                // in one step: solve rs − z tan = r − taper z).
                let hs = (rs - r) / (half.tan() - taper);
                if hs >= depth {
                    return Err("the countersink is deeper than the hole".into());
                }
                pts.extend([DVec2::new(rs + lead * half.tan(), -lead), DVec2::new(at(hs), hs)]);
            }
        }
        let end = at(depth);
        pts.push(DVec2::new(end, depth));
        if let Some(tip) = self.tip_angle {
            let half = (tip / 2.0).to_radians();
            if !(half > 0.0 && half < std::f64::consts::FRAC_PI_2) {
                return Err("the tip angle must be between 0° and 180°".into());
            }
            pts.push(DVec2::new(0.0, depth + end / half.tan()));
        } else {
            pts.push(DVec2::new(0.0, depth));
        }
        Ok(pts)
    }
}

/// Most copies one feature may make (keeps a typo from hanging the app).
pub const MAX_COPIES: u32 = 200;

impl CopyKind {
    /// The construction plane it mirrors in, if any.
    pub fn plane_used(&self) -> Option<usize> {
        match *self {
            CopyKind::MirrorPlane(k) => Some(k),
            _ => None,
        }
    }

    /// One placement per new copy (the original isn't one).
    /// `axes` are the construction axes as built: (point on it, direction);
    /// `planes` the construction planes as built.
    pub fn placements(&self, axes: &[Option<(DVec3, DVec3)>], planes: &[Option<FacePlane>]) -> Result<Vec<Placement>, String> {
        match *self {
            CopyKind::Mirror(p) => {
                let n = p.plane().normal().as_dvec3();
                Ok(vec![Placement::Reflect { origin: DVec3::ZERO, normal: n }])
            }
            CopyKind::MirrorPlane(k) => {
                let p = planes.get(k).copied().flatten().ok_or("its mirror plane couldn't be built")?;
                Ok(vec![Placement::Reflect { origin: p.origin, normal: p.normal() }])
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
    /// A chamfer by distance and angle: the angle (degrees) from the first
    /// face (`size` is then measured along that face). Overrides `size2`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle: Option<f64>,
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

    #[test]
    fn a_hole_profile_has_its_bore_counterbore_countersink_and_tip() {
        use glam::DVec2;
        let mut h = HoleFeature { tip_angle: None, ..Default::default() };
        assert_eq!(h.profile(6.0, 10.0, 1.0).unwrap(), vec![DVec2::new(0.0, -1.0), DVec2::new(3.0, -1.0), DVec2::new(3.0, 10.0), DVec2::new(0.0, 10.0)]);
        h.tip_angle = Some(90.0);
        let p = h.profile(6.0, 10.0, 1.0).unwrap();
        assert!(p.last().unwrap().distance(DVec2::new(0.0, 13.0)) < 1e-12, "a 90° tip is as deep as the radius");
        h.kind = HoleKind::Counterbore;
        (h.cbore_diameter, h.cbore_depth) = (11.0, 4.0);
        assert_eq!(h.profile(6.0, 10.0, 1.0).unwrap()[1..4], [DVec2::new(5.5, -1.0), DVec2::new(5.5, 4.0), DVec2::new(3.0, 4.0)]);
        h.cbore_diameter = 5.0;
        assert!(h.profile(6.0, 10.0, 1.0).is_err(), "a counterbore narrower than the hole");
        h.kind = HoleKind::Countersink;
        (h.csink_diameter, h.csink_angle) = (12.0, 90.0);
        let p = h.profile(6.0, 10.0, 1.0).unwrap();
        assert!(p[1].distance(DVec2::new(7.0, -1.0)) < 1e-12 && p[2].distance(DVec2::new(3.0, 3.0)) < 1e-12, "{p:?}");
        // The drilled size by tap type.
        h.tap = TapType::Clearance;
        assert_eq!(h.bore(None).unwrap(), 6.6);
        h.tap = TapType::Tapped;
        assert!((h.bore(None).unwrap() - 4.917).abs() < 1e-3);
        h.tap = TapType::Simple;
        assert_eq!(h.bore(Some(8.0)).unwrap(), 8.0, "a circle place keeps its circle's size");
        assert_eq!(h.bore(None).unwrap(), 5.0);
    }

    #[test]
    fn a_plane_at_angle_turns_about_its_line() {
        // About X from the XY plane: 0° is XY itself, 90° stands up as XZ (facing -Y).
        let f = angle_frame(DVec3::new(0.0, 0.0, 5.0), DVec3::X, Hinge::world_normal(WorldAxis::X), 0.0).unwrap();
        assert!(f.normal().distance(DVec3::Z) < 1e-12 && f.origin.z == 5.0);
        let f = angle_frame(DVec3::ZERO, DVec3::X, DVec3::Z, 90.0).unwrap();
        assert!(f.normal().distance(DVec3::NEG_Y) < 1e-12, "{}", f.normal());
        let f = angle_frame(DVec3::ZERO, DVec3::X, DVec3::Z, 30.0).unwrap();
        assert!((f.normal().dot(DVec3::Z) - 30f64.to_radians().cos()).abs() < 1e-12);
        assert!(f.u.distance(DVec3::X) < 1e-12, "the line lies in the plane");
        // A hint along the line still gives a plane through it.
        let n = zero_normal(DVec3::Z, DVec3::Z);
        assert!(n.dot(DVec3::Z).abs() < 1e-12 && (n.length() - 1.0).abs() < 1e-12);
        assert!(angle_frame(DVec3::ZERO, DVec3::ZERO, DVec3::Z, 0.0).is_none());
    }

    #[test]
    fn a_hinge_renumbers_when_earlier_axes_and_sketches_go() {
        let mut h = Hinge::Axis(3);
        assert!(h.uses_axis(3) && !h.uses_axis(1));
        h.axis_deleted(1);
        assert_eq!(h, Hinge::Axis(2));
        h.axis_deleted(5);
        assert_eq!(h, Hinge::Axis(2));
        let mut s = Hinge::SketchLine { sketch: 2, line: 4 };
        assert!(s.uses_sketch(2));
        s.sketch_deleted(0);
        assert_eq!(s, Hinge::SketchLine { sketch: 1, line: 4 });
        s.axis_deleted(0);
        assert_eq!(s, Hinge::SketchLine { sketch: 1, line: 4 });
    }

    #[test]
    fn all_threads_switch_together_but_taper_threads_stay_shown() {
        let tapped = HoleFeature { tap: TapType::Tapped, ..Default::default() };
        let taper = HoleFeature { tap: TapType::TaperTapped, ..Default::default() };
        let plain = HoleFeature::default();
        let mut holes = vec![tapped, taper, plain];
        let mut threads = vec![ThreadFeature { name: "T".into(), faces: Vec::new(), spec: ThreadSpec::default(), full_length: true, offset: 0.0, length: 1.0 }];
        assert_eq!(thread_count(&holes, &threads), (0, 2));
        set_all_modeled(&mut holes, &mut threads, true);
        assert_eq!(thread_count(&holes, &threads), (2, 2));
        assert!(!holes[1].thread.modeled, "taper tapped: only shown");
        set_all_modeled(&mut holes, &mut threads, false);
        assert_eq!(thread_count(&holes, &threads), (0, 2));
    }

    #[test]
    fn a_plane_is_used_up_only_by_what_runs_to_or_mirrors_in_it() {
        let mut e = Extrude { op: Operation::NewBody, name: "E".into(), sketch: 0, profiles: vec![0], distance: 5.0, extent: Extent::OneSide, distance2: 0.0, taper: 0.0, target: Some(ExtrudeTarget::Plane(2)) };
        assert_eq!(e.plane_used(), None, "a leftover target with a distance extent doesn't count");
        e.extent = Extent::ToObject;
        assert_eq!(e.plane_used(), Some(2));
        e.target = Some(ExtrudeTarget::Origin(OriginPlane::XY));
        assert_eq!(e.plane_used(), None);
        assert_eq!(CopyKind::MirrorPlane(1).plane_used(), Some(1));
    }

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
            let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
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
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
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
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: -10.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
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
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 5.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
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
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0, 1, 2], distance: 15.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
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
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 7.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
        let m = build_extrude(&[sk], &ex).unwrap();
        let caps: Vec<Vec3> = (0..m.tris.len()).map(|t| m.normal(t)).filter(|n| n.z.abs() > 0.5).collect();
        assert!(caps.len() > 4);
        // Within float rounding (1e-6 is far below one shade step of 1/255).
        assert!(caps.iter().all(|n| (n.z.abs() - 1.0).abs() < 1e-6 && n.x.abs() < 1e-6 && n.y.abs() < 1e-6));
    }

    #[test]
    fn ray_hits_the_nearest_face() {
        let sk = rect_sketch(OriginPlane::XY);
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
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
        let ex = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 20.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
        let m = build_extrude(&[sk], &ex).unwrap();
        let sharp = mesh_edges(&m).iter().filter(|e| e.sharp).count();
        assert_eq!(sharp, 12, "a box has 12 edges; face diagonals are not sharp");
    }

    #[test]
    fn missing_profile_or_zero_distance_is_an_error() {
        let sk = rect_sketch(OriginPlane::XY);
        let bad = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![3], distance: 5.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
        assert!(build_extrude(&[sk.clone()], &bad).is_err());
        let zero = Extrude { op: Default::default(), name: "E".into(), sketch: 0, profiles: vec![0], distance: 0.0, extent: Default::default(), distance2: 0.0, taper: 0.0, target: None };
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
