//! The document: everything that gets saved, plus file I/O and STL export.
//!
//! Files are JSON (`.ocad`), versioned, in millimetres. Loading validates every
//! index and number so a damaged or hand-edited file gives a clear error
//! instead of crashing the app.

use crate::model::{EdgeFeature, Extrude, Revolve};
use crate::sketch::{Constraint, Entity, Sketch};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const FORMAT: &str = "opencad";
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "ocad";

/// One entry in the timeline, in the order features were created.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Feature {
    Sketch(usize),
    Extrude(usize),
    /// A fillet or chamfer (index into `edge_features`).
    Edge(usize),
    Revolve(usize),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub sketches: Vec<Sketch>,
    pub extrudes: Vec<Extrude>,
    /// Older files have none.
    #[serde(default)]
    pub edge_features: Vec<EdgeFeature>,
    #[serde(default)]
    pub revolves: Vec<Revolve>,
    pub timeline: Vec<Feature>,
    /// What's shown and where Home goes. Saved with the file, but never part
    /// of undo or the "unsaved changes" test (see `ViewState`'s equality).
    #[serde(default)]
    pub view: ViewState,
}

/// Viewing choices saved with a design: hidden bodies, sketch visibility
/// overrides and the Home pose. Older files have none.
///
/// All of these compare equal, so changing them is neither an undo step nor
/// an unsaved change — the same way a dimension label's position doesn't
/// rebuild the model.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ViewState {
    pub hidden_bodies: Vec<usize>,
    /// Sketches the user switched on or off in the browser, by index.
    pub sketch_shown: Vec<(usize, bool)>,
    pub home: Option<HomeView>,
}

impl PartialEq for ViewState {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// The Home camera pose (radians, mm).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HomeView {
    pub yaw: f32,
    pub pitch: f32,
    pub target: [f32; 3],
    pub distance: f32,
}

#[derive(Serialize, Deserialize)]
struct FileHeader {
    format: String,
    version: u32,
    units: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    #[serde(flatten)]
    header: FileHeader,
    document: Document,
}

pub fn to_json(doc: &Document) -> String {
    let file = File {
        header: FileHeader { format: FORMAT.into(), version: VERSION, units: "mm".into() },
        document: doc.clone(),
    };
    serde_json::to_string_pretty(&file).expect("document always serializes")
}

pub fn from_json(text: &str) -> Result<Document, String> {
    let header: FileHeader = serde_json::from_str(text).map_err(|e| format!("not an opencad file ({e})"))?;
    if header.format != FORMAT {
        return Err(format!("not an opencad file (format \"{}\")", header.format));
    }
    if header.version > VERSION {
        return Err(format!("made by a newer opencad (file version {}, this app reads {VERSION})", header.version));
    }
    if header.units != "mm" {
        return Err(format!("unsupported units \"{}\"", header.units));
    }
    let file: File = serde_json::from_str(text).map_err(|e| format!("damaged file: {e}"))?;
    validate(&file.document)?;
    Ok(file.document)
}

/// Check every reference and number so nothing later can index out of bounds.
pub fn validate(doc: &Document) -> Result<(), String> {
    let finite = |v: f64| v.is_finite();
    if let Some(h) = doc.view.home {
        let nums = [h.yaw, h.pitch, h.distance, h.target[0], h.target[1], h.target[2]];
        if nums.iter().any(|v| !v.is_finite()) || h.distance <= 0.0 {
            return Err("the saved Home view is damaged".into());
        }
    }
    for (si, sk) in doc.sketches.iter().enumerate() {
        let err = |m: String| format!("{}: {m}", if sk.name.is_empty() { format!("sketch {si}") } else { sk.name.clone() });
        if sk.points.is_empty() {
            return Err(err("missing origin point".into()));
        }
        if let Some(f) = sk.face {
            let unit = |v: glam::DVec3| v.is_finite() && (v.length() - 1.0).abs() < 1e-6;
            if !f.origin.is_finite() || !unit(f.u) || !unit(f.v) || f.u.dot(f.v).abs() > 1e-6 {
                return Err(err("invalid face plane".into()));
            }
        }
        if sk.points.iter().any(|p| !finite(p.x) || !finite(p.y)) {
            return Err(err("point with invalid coordinates".into()));
        }
        let np = sk.points.len();
        for (ei, e) in sk.entities.iter().enumerate() {
            let ok = match *e {
                Entity::Line { a, b } => a < np && b < np && a != b,
                Entity::Circle { center, radius } => center < np && finite(radius) && radius > 0.0,
                Entity::Point { p } => p < np,
                Entity::Arc { center, a, b } => center < np && a < np && b < np && a != b && a != center && b != center,
            };
            if !ok {
                return Err(err(format!("entity {ei} is invalid")));
            }
        }
        for (ci, c) in sk.constraints.iter().enumerate() {
            use Constraint::*;
            let (line, circle) = (|e: usize| sk.is_line(e), |e: usize| sk.is_round(e));
            let ok = match *c {
                Horizontal(e) | Vertical(e) => line(e),
                Length(e, d) => line(e) && finite(d) && d > 0.0,
                Diameter(e, d) => circle(e) && finite(d) && d > 0.0,
                Parallel(a, b) | Perpendicular(a, b) => line(a) && line(b) && a != b,
                Equal(a, b) => a != b && ((line(a) && line(b)) || (circle(a) && circle(b))),
                Tangent(l, c) => line(l) && circle(c),
                Angle(a, b, deg) => line(a) && line(b) && a != b && finite(deg),
                Distance(p, q, dist) => p < np && q < np && p != q && finite(dist) && dist > 0.0,
                OnCircle(p, c) => p < np && circle(c),
                OnLine(p, l) => p < np && line(l),
                OnAxis(p, axis) => p < np && axis < 2,
                Radius(e, r) => sk.is_arc(e) && finite(r) && r > 0.0,
                ArcEnds(_) => false, // internal to the solver, never saved
                Midpoint(p, a, b) => p < np && a < np && b < np && p != a && p != b && a != b,
                Fix(p) => p < np,
            };
            if !ok {
                return Err(err(format!("constraint {ci} is invalid")));
            }
        }
    }
    for (k, ex) in doc.extrudes.iter().enumerate() {
        if ex.sketch >= doc.sketches.len() || !finite(ex.distance) {
            return Err(format!("{}: invalid sketch reference or distance", ex.name.clone() + &format!(" (#{k})")));
        }
    }
    for (k, r) in doc.revolves.iter().enumerate() {
        if r.sketch >= doc.sketches.len() || !r.angle.is_finite() {
            return Err(format!("{} (#{k}): invalid sketch reference or angle", r.name));
        }
    }
    for f in &doc.edge_features {
        let ok = f.size.is_finite()
            && f.size > 0.0
            && f.edges.iter().all(|e| e.mid.is_finite() && e.dir.is_finite());
        if !ok {
            return Err(format!("{}: invalid size or edge", f.name));
        }
    }
    // The timeline must list every feature exactly once.
    let mut seen_s = vec![false; doc.sketches.len()];
    let mut seen_e = vec![false; doc.extrudes.len()];
    let mut seen_f = vec![false; doc.edge_features.len()];
    let mut seen_r = vec![false; doc.revolves.len()];
    for f in &doc.timeline {
        let slot = match *f {
            Feature::Sketch(i) => seen_s.get_mut(i),
            Feature::Extrude(i) => seen_e.get_mut(i),
            Feature::Edge(i) => seen_f.get_mut(i),
            Feature::Revolve(i) => seen_r.get_mut(i),
        };
        match slot {
            Some(s) if !*s => *s = true,
            _ => return Err(format!("timeline entry {f:?} is invalid or repeated")),
        }
    }
    if seen_s.iter().chain(&seen_e).chain(&seen_f).chain(&seen_r).any(|s| !s) {
        return Err("timeline is missing a feature".into());
    }
    Ok(())
}

/// Undo/redo for the whole design: snapshots of the document.
#[derive(Default)]
pub struct History {
    past: Vec<Document>,
    future: Vec<Document>,
    /// The design as of the last recorded step.
    base: Document,
}

impl History {
    pub const LIMIT: usize = 300;

    /// Start over from `doc` (new or opened file).
    pub fn reset(&mut self, doc: Document) {
        self.past.clear();
        self.future.clear();
        self.base = doc;
    }

    /// If `doc` differs from the last recorded version, that's a new step.
    /// A new step after undoing throws away the redo list (as everywhere).
    pub fn record(&mut self, doc: Document) {
        if doc != self.base {
            self.past.push(std::mem::replace(&mut self.base, doc));
            if self.past.len() > Self::LIMIT {
                self.past.remove(0);
            }
            self.future.clear();
        }
    }

    pub fn can_undo(&self, current: &Document) -> bool {
        !self.past.is_empty() || *current != self.base
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// The version to go back to. `current` is the design right now (it may
    /// have a change not recorded yet; that counts as its own step).
    pub fn undo(&mut self, current: Document) -> Option<Document> {
        self.record(current);
        let prev = self.past.pop()?;
        self.future.push(std::mem::replace(&mut self.base, prev.clone()));
        Some(prev)
    }

    pub fn redo(&mut self) -> Option<Document> {
        let next = self.future.pop()?;
        self.past.push(std::mem::replace(&mut self.base, next.clone()));
        Some(next)
    }
}

pub fn save(path: &Path, doc: &Document) -> Result<(), String> {
    // Write to a temporary file first, then rename: a crash mid-save can't
    // leave a half-written file in place of the good one.
    let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
    std::fs::write(&tmp, to_json(doc)).map_err(|e| format!("couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("couldn't save {}: {e}", path.display()))
}

pub fn load(path: &Path) -> Result<Document, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("couldn't read {}: {e}", path.display()))?;
    from_json(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::build_extrude;
    use crate::sketch::{Anchor, Editor, OriginPlane, Tool};
    use crate::solver;
    use glam::DVec2;

    fn sample() -> Document {
        let mut sk = Sketch::new("Sketch1".into(), OriginPlane::XY);
        let mut ed = Editor::default();
        ed.set_tool(Tool::Line);
        ed.click(&mut sk, Anchor::Point(0));
        for p in [(50.1, 0.2), (49.7, 30.3), (0.4, 29.6)] {
            ed.click(&mut sk, Anchor::Free(DVec2::new(p.0, p.1)));
        }
        ed.click(&mut sk, Anchor::Point(0));
        ed.set_tool(Tool::Circle);
        ed.click(&mut sk, Anchor::Free(DVec2::new(25.0, 15.0)));
        ed.click(&mut sk, Anchor::Free(DVec2::new(29.0, 15.0)));
        use Constraint::*;
        solver::add_constraints(&mut sk, &[Horizontal(0), Vertical(1), Horizontal(2), Vertical(3)]).unwrap();
        solver::add_constraints(&mut sk, &[Length(0, 50.0), Length(1, 30.0), Diameter(4, 8.0)]).unwrap();
        Document {
            sketches: vec![sk],
            extrudes: vec![Extrude { op: Default::default(), name: "Extrude1".into(), sketch: 0, profiles: vec![0], distance: 12.5 }],
            edge_features: vec![crate::model::EdgeFeature {
                name: "Fillet1".into(),
                kind: crate::model::EdgeKind::Fillet,
                size: 2.5,
                size2: None,
                edges: vec![crate::model::EdgeRef {
                    mid: glam::DVec3::new(25.0, 0.0, 12.5),
                    dir: glam::DVec3::X,
                    faces: [glam::DVec3::Z, glam::DVec3::NEG_Y],
                    follow: None,
                }],
            }],
            revolves: Vec::new(),
            timeline: vec![Feature::Sketch(0), Feature::Extrude(0), Feature::Edge(0)],
            view: ViewState::default(),
        }
    }

    #[test]
    fn undo_and_redo_walk_through_every_step() {
        let mut h = History::default();
        let v0 = Document::default();
        let mut v1 = v0.clone();
        v1.sketches.push(Sketch::new("Sketch1".into(), OriginPlane::XY));
        let v2 = sample();
        h.record(v0.clone()); // unchanged: no step
        assert!(!h.can_undo(&v0));
        h.record(v1.clone());
        h.record(v2.clone());
        assert_eq!(h.undo(v2.clone()), Some(v1.clone()));
        assert_eq!(h.undo(v1.clone()), Some(v0.clone()));
        assert_eq!(h.undo(v0.clone()), None, "nothing before the start");
        assert_eq!(h.redo(), Some(v1.clone()));
        assert_eq!(h.redo(), Some(v2.clone()));
        assert_eq!(h.redo(), None);
        // Undo, then make a different change: the redo list is gone.
        assert_eq!(h.undo(v2.clone()), Some(v1.clone()));
        let mut v3 = v1.clone();
        v3.sketches[0].name = "Other".into();
        h.record(v3.clone());
        assert!(!h.can_redo());
        assert_eq!(h.undo(v3), Some(v1));
    }

    #[test]
    fn an_unrecorded_change_is_its_own_undo_step() {
        let mut h = History::default();
        let base = Document::default();
        h.reset(base.clone());
        let changed = sample(); // e.g. mid-drag, not recorded yet
        assert!(h.can_undo(&changed));
        assert_eq!(h.undo(changed.clone()), Some(base));
        assert_eq!(h.redo(), Some(changed), "and redo brings it back");
    }

    #[test]
    fn history_keeps_a_bounded_number_of_steps() {
        let mut h = History::default();
        for i in 0..History::LIMIT + 50 {
            let mut d = Document::default();
            d.sketches.push(Sketch::new(format!("S{i}"), OriginPlane::XY));
            h.record(d);
        }
        let mut n = 0;
        let mut cur = h.base.clone();
        while let Some(prev) = h.undo(cur) {
            cur = prev;
            n += 1;
        }
        assert_eq!(n, History::LIMIT);
    }

    #[test]
    fn json_round_trip_is_exact() {
        let doc = sample();
        let text = to_json(&doc);
        assert!(text.contains("\"format\": \"opencad\"") && text.contains("\"units\": \"mm\""));
        assert_eq!(from_json(&text).unwrap(), doc, "every f64 survives exactly");
    }

    #[test]
    fn sketch_on_a_face_round_trips_and_is_checked() {
        let mut doc = sample();
        let face = crate::sketch::FacePlane::from_face(glam::DVec3::Z, glam::DVec3::new(0.0, 0.0, 12.5));
        doc.sketches.push(Sketch::on_face("Sketch2".into(), face));
        doc.timeline.push(Feature::Sketch(1));
        assert_eq!(from_json(&to_json(&doc)).unwrap(), doc);
        let mut bad = doc.clone();
        bad.sketches[1].face.as_mut().unwrap().u = glam::DVec3::new(2.0, 0.0, 0.0);
        assert!(validate(&bad).is_err(), "axes must be unit length");
    }

    #[test]
    fn save_and_load_through_a_real_file() {
        let dir = std::env::temp_dir().join(format!("opencad-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("part.ocad");
        let doc = sample();
        save(&path, &doc).unwrap();
        assert_eq!(load(&path).unwrap(), doc);
        assert!(!path.with_extension("ocad.tmp").exists(), "temp file renamed away");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn opens_files_saved_before_extrude_operations_existed() {
        // A real file saved by the milestone-5 version (no "op" on the extrude).
        let old = include_str!("../tests/milestone5_cylinder.ocad");
        let doc = from_json(old).expect("old files still open");
        assert_eq!(doc.extrudes[0].op, crate::model::Operation::NewBody);
        assert_eq!(doc.extrudes[0].distance, 30.0);
        assert!(doc.edge_features.is_empty(), "no fillets in old files");
    }

    #[test]
    fn rejects_wrong_or_newer_files() {
        assert!(from_json("hello").is_err());
        assert!(from_json(r#"{"format":"other","version":1,"units":"mm","document":{}}"#).is_err());
        let newer = to_json(&sample()).replace("\"version\": 1", "\"version\": 99");
        assert!(from_json(&newer).unwrap_err().contains("newer"));
    }

    #[test]
    fn rejects_broken_references_instead_of_crashing() {
        let mut d = sample();
        d.sketches[0].entities.push(Entity::Line { a: 1, b: 999 });
        assert!(validate(&d).is_err());

        let mut d = sample();
        d.sketches[0].constraints.push(Constraint::Tangent(4, 0)); // circle, line: wrong order
        assert!(validate(&d).is_err());

        let mut d = sample();
        d.extrudes[0].sketch = 5;
        assert!(validate(&d).is_err());

        let mut d = sample();
        d.timeline.push(Feature::Extrude(0)); // repeated
        assert!(validate(&d).is_err());

        let mut d = sample();
        d.timeline.pop(); // missing
        assert!(validate(&d).is_err());

        let mut d = sample();
        d.extrudes[0].distance = f64::NAN;
        assert!(validate(&d).is_err());

        let mut d = sample();
        d.edge_features[0].size = -1.0;
        assert!(validate(&d).is_err());

        let mut d = sample();
        d.timeline.retain(|f| !matches!(f, Feature::Edge(_)));
        assert!(validate(&d).is_err(), "fillet missing from the timeline");
    }

    #[test]
    fn loaded_document_rebuilds_the_same_body() {
        let doc = sample();
        let loaded = from_json(&to_json(&doc)).unwrap();
        let a = build_extrude(&doc.sketches, &doc.extrudes[0]).unwrap();
        let b = build_extrude(&loaded.sketches, &loaded.extrudes[0]).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn view_state_is_saved_but_never_an_undo_step_or_a_change() {
        let mut doc = sample();
        let plain = doc.clone();
        doc.view = ViewState {
            hidden_bodies: vec![1, 3],
            sketch_shown: vec![(0, true)],
            home: Some(HomeView { yaw: 0.5, pitch: 0.25, target: [1.0, 2.0, 3.0], distance: 150.0 }),
        };
        assert_eq!(doc, plain, "view choices don't count as a change");
        let back = from_json(&to_json(&doc)).unwrap();
        assert_eq!(back.view.hidden_bodies, vec![1, 3]);
        assert_eq!(back.view.sketch_shown, vec![(0, true)]);
        assert_eq!(back.view.home, doc.view.home);
        // Not an undo step.
        let mut h = History::default();
        h.reset(plain);
        h.record(doc);
        assert!(h.past.is_empty());
    }

    #[test]
    fn files_without_view_state_still_open_and_a_damaged_home_is_refused() {
        let json = to_json(&sample());
        // Remove the `view` key entirely, like a file saved by an older version.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["document"].as_object_mut().unwrap().remove("view");
        let old = from_json(&v.to_string()).unwrap();
        assert!(old.view.hidden_bodies.is_empty() && old.view.home.is_none());
        let mut bad = sample();
        bad.view.home = Some(HomeView { yaw: 0.0, pitch: 0.0, target: [0.0; 3], distance: -1.0 });
        assert!(validate(&bad).is_err());
        bad.view.home = Some(HomeView { yaw: f32::NAN, pitch: 0.0, target: [0.0; 3], distance: 10.0 });
        assert!(validate(&bad).is_err());
    }
}
