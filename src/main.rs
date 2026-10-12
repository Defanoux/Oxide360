//! opencad — revolve, rectangle tool, icon toolbar.
//! Units: millimetres. World is Z-up.
//!
//! Controls
//!   Orbit: left-drag (outside a sketch) or Shift + middle-drag
//!   Pan:   right-drag or middle-drag        Zoom: scroll
//!   F = fillet: click edges on a body, type a radius; Enter = OK, Esc = cancel
//!   E = extrude: click shaded profiles, then type a distance (the box is
//!       already selected) or drag the blue arrow; Enter = OK, Esc = cancel
//!   Camera menu: Perspective / Orthographic / Perspective with Ortho Faces
//!   File: Ctrl+N new, Ctrl+O open, Ctrl+S save, Ctrl+Shift+S save as
//!   In a sketch:
//!     Esc = stop tool / back to Select / clear selection
//!     L = line, R = rectangle, C = circle, G = grid snap, D = dimension selection
//!     Select tool: click to add/remove from selection, click empty to clear,
//!                  drag a point to move it, click a dimension to edit it
//!     Delete = delete selection
//!   Ctrl+Z = undo anything, Ctrl+Shift+Z / Ctrl+Y = redo

mod camera;
mod doc;
mod export;
mod gpu;
mod handles;
mod icons;
mod kernel;
mod model;
mod profile;
mod sketch;
mod solver;
mod view;

use camera::{
    FOV_Y, OrbitCamera, Pose, Projection, angles_for_dir, closest_param_on_line, cube_cells, cube_click_target,
    ndc_from_pixel,
};
use doc::{Document, Feature, History};
use handles::{LineHandle, TurnHandle};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use eframe::egui;
use egui::{Color32, Stroke};
use glam::{DVec2, Vec2, Vec3};
use model::{EdgeFeature, EdgeKind, EdgeRef, Extrude, Mesh, Operation, Revolve, RevolveAxis};
use icons::Icon;
use profile::{Region, find_regions};
use sketch::{
    Anchor, CONSTRAINT_ACTIONS, Constraint, Editor, Entity, FacePlane, GridSpec, OriginPlane, PointId, Sel, Sketch,
    Tool,
};
use solver::{SolveError, Solved};
use view::{GRID_HALF_LINES, View, axis_color, dist_to_segment_px, to_pos, to_vec};

const SNAP_PX: f32 = 10.0;

const COL_UNDER: Color32 = Color32::from_rgb(110, 165, 255); // under-constrained
const COL_FULL: Color32 = Color32::from_rgb(235, 235, 235); // fully constrained
const COL_INACTIVE: Color32 = Color32::from_rgb(110, 125, 150);
const COL_PREVIEW: Color32 = Color32::from_rgb(90, 170, 255);
const COL_POINT: Color32 = Color32::from_rgb(200, 200, 200);
const COL_FIXED: Color32 = Color32::from_rgb(230, 80, 80);
const COL_ORIGIN: Color32 = Color32::from_rgb(240, 180, 60);
const COL_SNAP: Color32 = Color32::from_rgb(255, 120, 60);
const COL_SELECT: Color32 = Color32::from_rgb(255, 170, 40);
const COL_HOVER: Color32 = Color32::from_rgb(255, 220, 140);
const COL_GLYPH: Color32 = Color32::from_rgb(170, 200, 170);
const COL_DIM: Color32 = Color32::from_rgb(230, 230, 120);
const COL_DRIVEN: Color32 = Color32::from_rgb(140, 140, 130);
const COL_BODY: Color32 = Color32::from_rgb(170, 180, 195);
const COL_BODY_PREVIEW: Color32 = Color32::from_rgb(120, 170, 240);
const COL_ARROW: Color32 = Color32::from_rgb(60, 140, 255);
const COL_ARROW_HOT: Color32 = Color32::from_rgb(140, 200, 255);
const ARROW_PX: f32 = 60.0; // on-screen arrow length
const CUBE_SCALE: f32 = 34.0; // view cube half-size in pixels
const CUBE_MARGIN: f32 = 80.0; // view cube centre from the viewport's right edge
const CUBE_TOP: f32 = 100.0; // ...and from its top (room for the Z label)
const TURN_SECONDS: f64 = 0.35; // camera animation to a view-cube view
const FOLLOW_TOL: f64 = 1e-5; // mm: is a picked face/edge on a feature's far end?

fn profile_fill(alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(90, 150, 255, alpha)
}

/// Try an edit on a copy of the sketch, solve, and only commit if it solves.
/// Returns Ok(None) if the edit changed nothing.
fn apply_edit(
    sk: &mut Sketch,
    undo: Option<&mut Vec<Sketch>>,
    f: impl FnOnce(&mut Sketch),
) -> Result<Option<Solved>, SolveError> {
    let mut trial = sk.clone();
    f(&mut trial);
    if trial == *sk {
        return Ok(None);
    }
    let solved = solver::solve(&mut trial, &[])?;
    if let Some(undo) = undo {
        undo.push(sk.clone());
    }
    *sk = trial;
    Ok(Some(solved))
}

fn parse_mm(text: &str) -> Option<f64> {
    text.trim().trim_end_matches("mm").trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Hands one frame of solids to the GPU renderer inside egui's own frame.
/// `prepare` draws into the offscreen depth-buffered image; `paint` copies it
/// into the viewport between the grid (below) and the sketch overlays (above).
struct SceneCallback {
    scene: gpu::Scene,
    size_points: [f32; 2],
}

impl eframe::egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &eframe::wgpu::Device,
        queue: &eframe::wgpu::Queue,
        screen: &eframe::egui_wgpu::ScreenDescriptor,
        encoder: &mut eframe::wgpu::CommandEncoder,
        resources: &mut eframe::egui_wgpu::CallbackResources,
    ) -> Vec<eframe::wgpu::CommandBuffer> {
        if let Some(g) = resources.get_mut::<gpu::Gpu>() {
            let ppp = screen.pixels_per_point;
            let size = [
                (self.size_points[0] * ppp).round() as u32,
                (self.size_points[1] * ppp).round() as u32,
            ];
            g.render(device, queue, encoder, size, &self.scene);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut eframe::wgpu::RenderPass<'static>,
        resources: &eframe::egui_wgpu::CallbackResources,
    ) {
        if let Some(g) = resources.get::<gpu::Gpu>() {
            g.blit(pass);
        }
    }
}

/// Draw a dimension constraint with its label moved by `shift`; returns the
/// label's rectangle. Used for real dimensions, the one being placed, and (on an
/// invisible painter) to find where a label would sit by default.
fn draw_dimension(
    view: &View,
    sk: &Sketch,
    plane: &sketch::Plane,
    c: &Constraint,
    driven: bool,
    shift: DVec2,
    away_from: Vec2,
    color: Color32,
) -> Option<egui::Rect> {
    let line = |e: usize| match sk.entities.get(e)? {
        Entity::Line { a, b } => Some((sk.points[*a], sk.points[*b])),
        _ => None,
    };
    // A driven dimension shows what the geometry measures now, in brackets.
    let d_of = |stored: f64| if driven { sk.measure(c).unwrap_or(stored) } else { stored };
    let text = |s: String| if driven { format!("({s})") } else { s };
    match *c {
        Constraint::Length(e, d) => {
            let (a, b) = line(e)?;
            view.linear_dimension(plane, a, b, away_from, shift, text(format!("{:.2}", d_of(d))), color)
        }
        Constraint::Distance(p, q, d) => view.linear_dimension(
            plane,
            *sk.points.get(p)?,
            *sk.points.get(q)?,
            away_from,
            shift,
            text(format!("{:.2}", d_of(d))),
            color,
        ),
        Constraint::Diameter(e, d) => match sk.entities.get(e)? {
            Entity::Circle { center, radius } => {
                view.diameter_dimension(plane, sk.points[*center], *radius, shift, text(format!("Ø{:.2}", d_of(d))), color)
            }
            _ => None,
        },
        Constraint::Angle(e1, e2, d) => {
            view.angle_dimension(plane, line(e1)?, line(e2)?, shift, text(format!("{:.1}°", d_of(d))), color)
        }
        Constraint::Radius(e, d) => {
            let (c, r, a0, sweep) = sk.arc_geometry(e)?;
            view.radius_dimension(plane, c, r, a0 + sweep * 0.5, shift, text(format!("R{:.2}", d_of(d))), color)
        }
        _ => None,
    }
}

/// The point dimensions are drawn away from: the middle of the sketch's
/// geometry on screen (so they sit outside the shape).
fn sketch_centroid(view: &View, sk: &Sketch, plane: &sketch::Plane, fallback: Vec2) -> Vec2 {
    let visible: Vec<Vec2> = sk.points.iter().skip(1).filter_map(|&p| view.project(plane.to_world(p))).collect();
    if visible.is_empty() { fallback } else { visible.iter().copied().sum::<Vec2>() / visible.len() as f32 }
}

fn rect_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Rectangle3Point => Icon::Rectangle3Point,
        Tool::RectangleCenter => Icon::RectangleCenter,
        _ => Icon::Rectangle,
    }
}

fn circle_icon(tool: Tool) -> Icon {
    match tool {
        Tool::Circle2Point => Icon::Circle2Point,
        Tool::Circle3Point => Icon::Circle3Point,
        Tool::Circle2Tangent => Icon::Circle2Tangent,
        Tool::Circle3Tangent => Icon::Circle3Tangent,
        _ => Icon::Circle,
    }
}

fn rgb(c: Color32) -> [f32; 3] {
    [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0]
}

/// File commands, run after the frame's UI so dialogs never interrupt a widget.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    New,
    Open,
    Save,
    SaveAs,
    Export(export::Format),
}

/// What to do once the user has answered "save changes?".
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    New,
    Open,
    Quit,
}

/// What the floating number box is editing.
#[derive(Clone, Copy, Debug, PartialEq)]
enum EditTarget {
    Dimension(usize), // constraint index in the active sketch
    ExtrudeDistance,
    EdgeSize,     // fillet radius / chamfer distance
    RevolveAngle, // degrees
    NewCircle,    // diameter typed for the circle being drawn
    Corner(PointId), // sketch fillet radius / chamfer distance for this corner
}

struct FloatEdit {
    target: EditTarget,
    text: String,
    /// Focus the box and select all its text next frame, so typing replaces it.
    focus: bool,
    /// With `focus`: false puts the cursor at the end instead (the user
    /// already started typing the value).
    keep_text: bool,
}

struct ExtrudeDialog {
    editing: Option<usize>,
    sketch: Option<usize>,
    profiles: Vec<usize>,
    distance: f64,
    op: Operation,
}

/// A smooth camera move (view cube clicks, Home).
struct CameraTurn {
    from: Pose,
    to: Pose,
    start: f64,
}

struct RevolveDialog {
    editing: Option<usize>,
    sketch: Option<usize>,
    profiles: Vec<usize>,
    axis: RevolveAxis,
    angle: f64,
    op: Operation,
    /// Waiting for a click on a sketch line to use as the axis.
    picking_axis: bool,
}

/// A drag arrow on the canvas and the value it controls (see `handles`).
#[derive(Clone, Copy)]
enum Handle {
    Line { line: LineHandle, value: f64, target: HandleTarget },
    Turn { turn: TurnHandle, value: f64 },
}

#[derive(Clone, Copy, PartialEq)]
enum HandleTarget {
    Extrude,
    EdgeSize,
    RevolveAngle,
    Corner,
}

struct EdgeDialog {
    editing: Option<usize>,
    kind: EdgeKind,
    size: f64,
    edges: Vec<EdgeRef>,
}

/// Which version of the design a kernel model is built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// What's saved in the document.
    Committed = 0,
    /// With the open extrude / fillet dialog applied.
    Preview = 1,
    /// Before the fillet being edited — its edges are the ones to pick.
    Base = 2,
}

/// The last kernel evaluation and the inputs it was built from. The kernel
/// only runs again when those inputs change.
struct KernelCache {
    sketches: Vec<Sketch>,
    extrudes: Vec<Extrude>,
    revolves: Vec<Revolve>,
    edge_features: Vec<EdgeFeature>,
    timeline: Vec<Feature>,
    model: kernel::Model,
}

struct OpenCad {
    camera: OrbitCamera,
    sketches: Vec<Sketch>,
    extrudes: Vec<Extrude>,
    timeline: Vec<Feature>,
    active: Option<usize>,
    editor: Editor,
    grid_snap: bool,
    undo: Vec<Sketch>,
    dof: Option<usize>,
    message: Option<String>,
    drag: Option<PointId>,
    dialog: Option<ExtrudeDialog>,
    float: Option<FloatEdit>,
    float_anchor: Option<egui::Pos2>,
    dim_labels: Vec<(usize, egui::Rect, Vec<[Vec2; 2]>)>, // dimension, its label, and its lines on screen
    dim_selection: Vec<usize>,                                 // dimensions selected (Delete removes them)
    dim_hover: Option<usize>,
    rect_tool: Tool, // which rectangle tool the toolbar's rectangle button is
    arc_tool: Tool,  // which arc tool the toolbar's arc button is
    poly_tool: Tool, // which polygon tool the toolbar's polygon button is
    corner_size: f64, // last sketch fillet radius / chamfer distance
    corner_first: Option<usize>, // chamfer: the line the first distance is for (when picked as two lines)
    feature_errors: Vec<Option<String>>, // per extrude, from the last committed build
    body_count: usize,
    dialog_error: Option<String>,
    caches: [Option<KernelCache>; 3], // indexed by Slot
    edge_features: Vec<EdgeFeature>,
    edge_dialog: Option<EdgeDialog>,
    revolves: Vec<Revolve>,
    revolve_dialog: Option<RevolveDialog>,
    revolve_errors: Vec<Option<String>>,
    axis_hover: Option<(usize, usize)>, // sketch, line entity under the mouse (revolve axis picking)
    edge_hover: Option<EdgeRef>,
    edge_feature_errors: Vec<Option<String>>,
    sketch_errors: Vec<Option<String>>,
    history: History, // undo / redo for the whole design
    hidden_bodies: HashSet<usize>,          // bodies switched off in the browser
    sketch_shown: HashMap<usize, bool>,     // per-sketch visibility the user chose
    label_drag: Option<(usize, DVec2, DVec2)>, // dimension being moved: constraint, press point, its shift then
    circle_tool: Tool,                         // which circle tool the toolbar's circle button is
    toast: Option<(String, f64)>,              // a notification in the viewport's corner, and when it appeared
    touchpad: bool,                            // touchpad controls instead of mouse
    label_carry: bool,                         // touchpad: a dimension label picked up, following the cursor
    dim_preview: Option<(Constraint, DVec2)>,  // Dimension tool: what a click would place, and its label shift
    face_pick: bool,                         // waiting for a click on a flat face to sketch on
    face_hover: Option<(usize, FacePlane)>, // body index + the face plane under the mouse
    turn: Option<CameraTurn>,
    home: Pose,      // where Home goes (set it from the view cube menu)
    cube_axes: bool, // show the XYZ triad on the view cube
    arrow_screen: Option<(Vec2, Vec2)>, // extrude arrow on screen, last frame
    arrow_drag: Option<f64>,            // grab offset while dragging the arrow
    arrow_turn: Option<(f64, f64)>,     // revolve arrow: cursor angle so far (unwrapped, last raw)
    path: Option<PathBuf>,              // where this design is saved
    saved: Document,                    // what's on disk, to detect unsaved changes
    cmd: Option<Cmd>,
    confirm: Option<Pending>,
    alert: Option<String>,
    allow_close: bool,
    title: String,
    gpu: bool, // GPU renderer available (otherwise the CPU fallback draws)
}

impl Default for OpenCad {
    fn default() -> Self {
        Self {
            camera: OrbitCamera::default(),
            sketches: Vec::new(),
            extrudes: Vec::new(),
            timeline: Vec::new(),
            active: None,
            editor: Editor::default(),
            grid_snap: true,
            undo: Vec::new(),
            dof: None,
            message: None,
            drag: None,
            dialog: None,
            float: None,
            float_anchor: None,
            dim_labels: Vec::new(),
            dim_selection: Vec::new(),
            dim_hover: None,
            rect_tool: Tool::Rectangle,
            arc_tool: Tool::Arc3Point,
            poly_tool: Tool::PolygonCircumscribed,
            corner_size: 2.0,
            corner_first: None,
            feature_errors: Vec::new(),
            body_count: 0,
            dialog_error: None,
            caches: [None, None, None],
            edge_features: Vec::new(),
            edge_dialog: None,
            revolves: Vec::new(),
            revolve_dialog: None,
            revolve_errors: Vec::new(),
            axis_hover: None,
            edge_hover: None,
            edge_feature_errors: Vec::new(),
            sketch_errors: Vec::new(),
            history: History::default(),
            hidden_bodies: HashSet::new(),
            sketch_shown: HashMap::new(),
            label_drag: None,
            circle_tool: Tool::Circle,
            toast: None,
            touchpad: false,
            label_carry: false,
            dim_preview: None,
            face_pick: false,
            face_hover: None,
            turn: None,
            home: OrbitCamera::default().pose(),
            cube_axes: true,
            arrow_screen: None,
            arrow_drag: None,
            arrow_turn: None,
            path: None,
            saved: Document::default(),
            cmd: None,
            confirm: None,
            alert: None,
            allow_close: false,
            title: String::new(),
            gpu: false,
        }
    }
}

impl OpenCad {
    // ---------- documents and files ----------

    fn document(&self) -> Document {
        Document {
            sketches: self.sketches.clone(),
            extrudes: self.extrudes.clone(),
            edge_features: self.edge_features.clone(),
            revolves: self.revolves.clone(),
            timeline: self.timeline.clone(),
        }
    }

    fn is_dirty(&self) -> bool {
        self.sketches != self.saved.sketches
            || self.extrudes != self.saved.extrudes
            || self.edge_features != self.saved.edge_features
            || self.revolves != self.saved.revolves
            || self.timeline != self.saved.timeline
    }

    fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("Untitled".into(), |n| n.to_string_lossy().into_owned())
    }

    /// Replace the whole design (New / Open) and reset every editing state.
    fn reset_with(&mut self, doc: Document, path: Option<PathBuf>) {
        self.active = None;
        self.editor = Editor::default();
        self.undo.clear();
        self.dof = None;
        self.message = None;
        self.drag = None;
        self.dialog = None;
        self.float = None;
        self.arrow_screen = None;
        self.arrow_drag = None;
        self.edge_dialog = None;
        self.sketches = doc.sketches.clone();
        self.extrudes = doc.extrudes.clone();
        self.edge_features = doc.edge_features.clone();
        self.revolves = doc.revolves.clone();
        self.revolve_dialog = None;
        self.timeline = doc.timeline.clone();
        self.history.reset(doc.clone());
        self.saved = doc;
        self.path = path;
    }

    // ---------- undo / redo (the whole design) ----------

    /// Record a finished change. Runs once per frame after the UI; waits while
    /// the mouse is held or a number is being typed, so a whole drag or a
    /// typed value becomes one step.
    fn record_history(&mut self, ctx: &egui::Context) {
        // (A label being carried on a touchpad is one step too: wait for the drop.)
        if ctx.input(|i| i.pointer.any_down()) || ctx.text_edit_focused() || self.label_carry {
            return;
        }
        self.history.record(self.document());
    }

    fn can_undo(&self) -> bool {
        self.editor.busy() || self.history.can_undo(&self.document())
    }

    /// Undo the last change, whatever it was: a sketch line, a constraint, an
    /// extrude, a fillet, a delete…
    fn undo(&mut self) {
        if self.editor.busy() {
            self.editor.cancel(); // first just drop a half-drawn line or circle
            return;
        }
        if let Some(prev) = self.history.undo(self.document()) {
            self.restore(prev);
        }
    }

    fn redo(&mut self) {
        if let Some(next) = self.history.redo() {
            self.restore(next);
        }
    }

    /// Put a version of the design back, closing anything that points into
    /// the old one (dialogs, selections, drags).
    fn restore(&mut self, doc: Document) {
        let active = self.active.filter(|&i| i < doc.sketches.len());
        self.sketches = doc.sketches;
        self.extrudes = doc.extrudes;
        self.edge_features = doc.edge_features;
        self.revolves = doc.revolves;
        self.timeline = doc.timeline;
        self.dialog = None;
        self.revolve_dialog = None;
        self.edge_dialog = None;
        self.edge_hover = None;
        self.float = None;
        self.drag = None;
        self.arrow_drag = None;
        self.arrow_screen = None;
        self.face_pick = false;
        self.message = None;
        self.editor.selection.clear();
        self.editor.cancel();
        self.dim_selection.clear();
        self.undo.clear();
        self.active = active;
        self.dof = active.and_then(|i| solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof));
    }

    fn file_dialog(&self) -> rfd::FileDialog {
        let mut d = rfd::FileDialog::new();
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            d = d.set_directory(dir);
        }
        d
    }

    fn save_to(&mut self, path: PathBuf) -> bool {
        let doc = self.document();
        match doc::save(&path, &doc) {
            Ok(()) => {
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

    fn save_as(&mut self) -> bool {
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

    fn save(&mut self) -> bool {
        match self.path.clone() {
            Some(p) => self.save_to(p),
            None => self.save_as(),
        }
    }

    fn open_dialog(&mut self) {
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
    fn move_camera(&mut self, to: Pose, now: f64) {
        self.turn = Some(CameraTurn { from: self.camera.pose(), to, start: now });
    }

    /// The feature lists a slot is built from.
    fn inputs(&self, slot: Slot) -> (Vec<Extrude>, Vec<Revolve>, Vec<EdgeFeature>, Vec<Feature>) {
        let (mut extrudes, mut revolves, mut edge_features, mut timeline) =
            (self.extrudes.clone(), self.revolves.clone(), self.edge_features.clone(), self.timeline.clone());
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
        (extrudes, revolves, edge_features, timeline)
    }

    /// Make sure the kernel model for `slot` matches the current design.
    /// Rebuilds only when its inputs changed.
    fn ensure_model(&mut self, slot: Slot) {
        let (extrudes, revolves, edge_features, timeline) = self.inputs(slot);
        let fresh = self.caches[slot as usize].as_ref().is_some_and(|c| {
            c.sketches.len() == self.sketches.len()
                && c.sketches.iter().zip(&self.sketches).all(|(a, b)| a.same_shape(b))
                && c.extrudes == extrudes
                && c.revolves == revolves
                && c.edge_features == edge_features
                && c.timeline == timeline
        });
        if !fresh {
            let model = kernel::build(&self.sketches, &extrudes, &revolves, &edge_features, &timeline);
            self.caches[slot as usize] =
                Some(KernelCache { sketches: self.sketches.clone(), extrudes, revolves, edge_features, timeline, model });
        }
    }

    fn model(&self, slot: Slot) -> &kernel::Model {
        &self.caches[slot as usize].as_ref().expect("ensure_model runs first").model
    }

    fn export(&mut self, format: export::Format) {
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
                let bodies: Vec<export::Body> = model
                    .bodies
                    .iter()
                    .enumerate()
                    .map(|(k, b)| export::Body { name: format!("Body{}", k + 1), mesh: &b.mesh })
                    .collect();
                export::write(&p, format, &bodies)
            }
        };
        if let Err(e) = result {
            self.alert = Some(e);
        }
    }

    /// New / Open / Quit go through here so unsaved work is never lost silently.
    fn request(&mut self, action: Pending, ctx: &egui::Context) {
        if self.is_dirty() {
            self.confirm = Some(action);
        } else {
            self.run(action, ctx);
        }
    }

    fn run(&mut self, action: Pending, ctx: &egui::Context) {
        match action {
            Pending::New => self.reset_with(Document::default(), None),
            Pending::Open => self.open_dialog(),
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn run_cmd(&mut self, cmd: Cmd, ctx: &egui::Context) {
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
    fn modals(&mut self, ctx: &egui::Context) {
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

    fn start_sketch(&mut self, plane: OriginPlane) {
        self.dialog = None;
        self.finish_sketch();
        let name = format!("Sketch{}", self.sketches.len() + 1);
        self.sketches.push(Sketch::new(name, plane));
        let i = self.sketches.len() - 1;
        self.timeline.push(Feature::Sketch(i));
        self.enter_sketch(i);
    }

    /// The plane sketch `i` is on right now. A sketch on a face follows that
    /// face: the committed build says where the face is (the sketch itself
    /// keeps the plane it was picked on, so tracking always starts from there).
    fn plane_of(&self, i: usize) -> sketch::Plane {
        let sk = &self.sketches[i];
        let now = self.caches[Slot::Committed as usize]
            .as_ref()
            .filter(|c| c.sketches.get(i).is_some_and(|s| s.same_shape(sk)))
            .and_then(|c| c.model.sketch_frames.get(i).copied().flatten());
        match (sk.face, now) {
            (Some(_), Some(face)) => face.plane(),
            _ => sk.sketch_plane(),
        }
    }

    /// Is a sketch drawn? By default a sketch used by a feature hides itself
    /// (like Fusion); the browser's checkbox overrides that either way.
    fn sketch_visible(&self, i: usize) -> bool {
        let used = self.extrudes.iter().any(|e| e.sketch == i) || self.revolves.iter().any(|r| r.sketch == i);
        self.sketch_shown.get(&i).copied().unwrap_or(!used)
    }

    /// Delete a sketch and the extrudes built from it.
    fn delete_sketch(&mut self, i: usize) {
        if i >= self.sketches.len() {
            return;
        }
        // Leave sketch mode without finish_sketch (it may drop an empty sketch
        // and shift the indices under us).
        self.active = None;
        self.editor = Editor::default();
        self.undo.clear();
        self.drag = None;
        self.float = None;
        self.cancel_extrude();
        self.cancel_edge_dialog();
        let mut uses: Vec<usize> = (0..self.extrudes.len()).filter(|&k| self.extrudes[k].sketch == i).collect();
        uses.sort_unstable_by(|a, b| b.cmp(a)); // highest first, so indices stay valid
        for k in uses {
            self.delete_extrude(k);
        }
        let mut spun: Vec<usize> = (0..self.revolves.len()).filter(|&k| self.revolves[k].sketch == i).collect();
        spun.sort_unstable_by(|a, b| b.cmp(a));
        for k in spun {
            self.delete_revolve(k);
        }
        self.sketches.remove(i);
        self.timeline.retain(|f| *f != Feature::Sketch(i));
        for f in &mut self.timeline {
            if let Feature::Sketch(k) = f {
                if *k > i {
                    *k -= 1;
                }
            }
        }
        for e in &mut self.extrudes {
            if e.sketch > i {
                e.sketch -= 1;
            }
        }
        for r in &mut self.revolves {
            if r.sketch > i {
                r.sketch -= 1;
            }
        }
        self.sketch_shown =
            self.sketch_shown.drain().filter(|&(k, _)| k != i).map(|(k, v)| (if k > i { k - 1 } else { k }, v)).collect();
    }

    /// Start a new sketch on a flat face of a body.
    fn start_sketch_on_face(&mut self, face: FacePlane) {
        self.dialog = None;
        self.finish_sketch();
        let name = format!("Sketch{}", self.sketches.len() + 1);
        self.sketches.push(Sketch::on_face(name, face));
        let i = self.sketches.len() - 1;
        self.timeline.push(Feature::Sketch(i));
        self.enter_sketch(i);
        // Centre the view on the face, like the origin-plane sketches.
        self.camera.target = face.origin.as_vec3();
    }

    fn enter_sketch(&mut self, i: usize) {
        self.revolve_dialog = None;
        self.dim_selection.clear();
        self.face_pick = false;
        self.face_hover = None;
        self.dialog = None;
        self.edge_dialog = None;
        self.float = None;
        self.active = Some(i);
        self.editor = Editor::default();
        self.undo.clear();
        self.message = None;
        self.drag = None;
        self.dof = solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof);
        let (yaw, pitch) = self.sketches[i].view_angles();
        self.camera.look_from(yaw, pitch);
    }

    /// Leave sketch mode. An empty sketch nothing depends on is discarded.
    /// Returns the index of a removed sketch so callers can fix up indices.
    fn finish_sketch(&mut self) -> Option<usize> {
        let i = self.active.take()?;
        self.editor = Editor::default();
        self.undo.clear();
        self.message = None;
        self.drag = None;
        self.float = None;
        let used = self.extrudes.iter().any(|e| e.sketch == i) || self.revolves.iter().any(|r| r.sketch == i);
        if !self.sketches[i].entities.is_empty() || used {
            return None;
        }
        self.sketches.remove(i);
        self.timeline.retain(|f| *f != Feature::Sketch(i));
        for f in &mut self.timeline {
            if let Feature::Sketch(k) = f {
                if *k > i {
                    *k -= 1;
                }
            }
        }
        for e in &mut self.extrudes {
            if e.sketch > i {
                e.sketch -= 1;
            }
        }
        for r in &mut self.revolves {
            if r.sketch > i {
                r.sketch -= 1;
            }
        }
        Some(i)
    }

    fn edit_sketch(&mut self, mut i: usize) {
        if self.active == Some(i) {
            return;
        }
        if let Some(removed) = self.finish_sketch() {
            if removed < i {
                i -= 1;
            }
        }
        self.enter_sketch(i);
    }

    fn report(&mut self, r: Result<Option<Solved>, SolveError>) {
        match r {
            Ok(Some(s)) => {
                self.dof = Some(s.dof);
                self.message = None;
            }
            Ok(None) => {}
            Err(e) => {
                let text = format!("Can't do that: {e}");
                self.notify(text.clone());
                self.message = Some(text);
            }
        }
    }

    /// Show a short notice in the viewport's bottom-left corner.
    fn notify(&mut self, text: String) {
        self.toast = Some((text, f64::NAN)); // timestamped when drawn
    }

    // ---------- sketch editing ----------

    /// Apply the constraint action `label` to the current selection.
    fn constrain(&mut self, label: &str) {
        let Some(i) = self.active else { return };
        let opts = self.sketches[i].constraint_options(&self.editor.selection);
        let Some((_, cs)) = opts.into_iter().find(|(l, _)| *l == label) else { return };
        let first_new = self.sketches[i].constraints.len();
        let mut trial = self.sketches[i].clone();
        let mut driven = false;
        let r = match (label, cs.as_slice()) {
            // A dimension that would over-constrain becomes a reference one.
            ("Dimension", &[c]) => solver::add_dimension(&mut trial, c).map(|(s, d)| {
                driven = d;
                Some(s)
            }),
            _ => solver::add_constraints(&mut trial, &cs).map(Some),
        };
        let ok = r.is_ok();
        if ok {
            self.undo.push(std::mem::replace(&mut self.sketches[i], trial));
        }
        self.report(r);
        if ok && driven {
            self.notify("Already fixed by other constraints — added as a reference (driven) dimension".into());
        }
        if ok {
            self.editor.selection.clear();
            if driven {
                return;
            }
            if let Some(Constraint::Length(_, d) | Constraint::Diameter(_, d) | Constraint::Angle(_, _, d) | Constraint::Distance(_, _, d) | Constraint::Radius(_, d)) = cs.first() {
                // Open the number box on the new dimension, ready to type.
                self.float =
                    Some(FloatEdit { target: EditTarget::Dimension(first_new), text: format!("{d:.2}"), focus: true, keep_text: false });
            }
        }
    }

    /// A constraint button or key. With suitable geometry selected it's added
    /// right away; otherwise the constraint becomes the tool and you pick the
    /// geometry next (like Fusion).
    fn constraint_button(&mut self, label: &'static str) {
        let Some(i) = self.active else { return };
        let ready = self.sketches[i]
            .constraint_options(&self.editor.selection)
            .iter()
            .any(|(l, cs)| *l == label && !cs.is_empty());
        if ready {
            self.constrain(label);
        } else {
            self.editor.selection.clear();
            self.editor.set_tool(Tool::Constrain(label));
        }
    }

    /// How many picks a constraint needs before it can be added.
    fn picks_needed(label: &str) -> usize {
        match label {
            "Horizontal" | "Vertical" | "Fix" => 1,
            _ => 2,
        }
    }

    /// A pick with a constraint tool (not Dimension): add it to the picks and
    /// apply the constraint as soon as they're enough. Picks that can't work
    /// together start over from the new one.
    fn constraint_pick(&mut self, label: &'static str, sel: Sel) {
        let Some(i) = self.active else { return };
        if self.editor.selection.contains(&sel) {
            self.editor.selection.retain(|s| *s != sel);
            return;
        }
        self.editor.selection.push(sel);
        let applies = |app: &Self| {
            app.sketches[i].constraint_options(&app.editor.selection).iter().any(|(l, cs)| *l == label && !cs.is_empty())
        };
        if !applies(self) && self.editor.selection.len() >= Self::picks_needed(label) {
            self.editor.selection = vec![sel];
        }
        if applies(self) {
            self.constrain(label);
            self.editor.set_tool(Tool::Constrain(label)); // stay in the tool for the next one
        }
    }

    /// Dimension tool: what the current picks would dimension.
    fn dimension_candidate(&self) -> Option<Constraint> {
        let i = self.active?;
        self.sketches[i]
            .constraint_options(&self.editor.selection)
            .into_iter()
            .find(|(l, _)| *l == "Dimension")
            .and_then(|(_, cs)| cs.first().copied())
    }

    /// Dimension tool: add the dimension with its label where it was placed,
    /// then open its value for typing (`typed`: what was already typed).
    fn place_dimension(&mut self, c: Constraint, shift: DVec2, typed: Option<String>) {
        let Some(i) = self.active else { return };
        let mut trial = self.sketches[i].clone();
        match solver::add_dimension(&mut trial, c) {
            Ok((s, driven)) => {
                let ci = trial.constraints.len() - 1;
                trial.set_label_shift(ci, shift);
                self.undo.push(std::mem::replace(&mut self.sketches[i], trial));
                self.dof = Some(s.dof);
                self.message = None;
                if driven {
                    self.notify("Already fixed by other constraints — added as a reference (driven) dimension".into());
                    self.editor.selection.clear();
                    return;
                }
                let value = match c {
                    Constraint::Length(_, d)
                    | Constraint::Diameter(_, d)
                    | Constraint::Angle(_, _, d)
                    | Constraint::Distance(_, _, d) | Constraint::Radius(_, d) => d,
                    _ => 0.0,
                };
                let keep_text = typed.is_some();
                let text = typed.unwrap_or_else(|| format!("{value:.2}"));
                self.float = Some(FloatEdit { target: EditTarget::Dimension(ci), text, focus: true, keep_text });
            }
            Err(e) => self.report(Err(e)),
        }
        self.editor.selection.clear();
    }

    /// The fillet / chamfer the open size box would make right now.
    fn corner_preview(&self, i: usize, p: PointId) -> Option<sketch::CornerCut> {
        let text = &self.float.as_ref()?.text;
        let (d1, d2) = sketch::parse_sizes(text).unwrap_or((self.corner_size, None));
        let fillet = self.editor.tool == Tool::SketchFillet;
        self.sketches.get(i)?.corner_cut(p, fillet, d1, d2.unwrap_or(d1), self.corner_first)
    }

    /// Sketch fillet / chamfer: round or bevel the corner at point `p`.
    /// A chamfer can take two distances (`size2`); a fillet only has a radius.
    fn finish_corner(&mut self, p: PointId, size: f64, size2: Option<f64>) {
        let Some(i) = self.active else { return };
        let fillet = self.editor.tool == Tool::SketchFillet;
        let first = self.corner_first;
        let mut why = None;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let mut trial = sk.clone();
            let made = if fillet {
                trial.fillet_corner(p, size)
            } else {
                trial.chamfer_corner2(p, size, size2.unwrap_or(size), first)
            };
            match made {
                Ok(cs) => match solver::add_constraints(&mut trial, &cs) {
                    Ok(_) => *sk = trial,
                    Err(e) => why = Some(format!("{e}")),
                },
                Err(e) => why = Some(e),
            }
        });
        match why {
            Some(e) => self.notify(format!("Can't {}: {e}", if fillet { "fillet" } else { "chamfer" })),
            None => {
                self.corner_size = size;
                self.report(r);
            }
        }
    }

    /// Centre circle: a diameter was typed after placing the centre. Make the
    /// circle that size, with a diameter dimension holding it.
    fn finish_circle_typed(&mut self, d: f64) {
        let (Some(i), Some(start)) = (self.active, self.editor.start) else { return };
        if !(d > 0.0) || !self.sketches[i].is_valid_anchor(start) {
            self.notify("The diameter must be greater than zero".into());
            return;
        }
        let at = Anchor::Free(self.sketches[i].pos(start) + DVec2::new(d / 2.0, 0.0));
        let ed = &mut self.editor;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let before = sk.entities.len();
            let mut add = vec![];
            add.extend(ed.click(sk, at));
            if sk.entities.len() > before {
                add.push(Constraint::Diameter(before, d));
            }
            for c in add {
                let mut t = sk.clone();
                if solver::add_constraints(&mut t, &[c]).is_ok() {
                    *sk = t;
                }
            }
        });
        self.report(r);
    }

    /// Fusion's X: with geometry selected, switch it between normal and
    /// construction; with nothing selected, switch construction mode for what
    /// you draw next.
    fn toggle_construction(&mut self) {
        let Some(i) = self.active else { return };
        let sel = self.editor.selection.clone();
        if sel.iter().any(|s| matches!(s, Sel::Entity(_))) {
            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| sk.toggle_construction(&sel));
            self.report(r);
        } else {
            self.editor.construction = !self.editor.construction;
            self.notify(if self.editor.construction { "Construction mode on: new geometry is construction" } else { "Construction mode off" }.into());
        }
    }

    /// Trim tool: cut line `e` back at the click `p`.
    fn trim_at(&mut self, e: usize, p: DVec2) {
        let Some(i) = self.active else { return };
        let mut why = None;
        self.dim_selection.clear();
        self.editor.selection.clear();
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            let mut trial = sk.clone();
            match trial.trim_line(e, p) {
                Ok(cs) => {
                    // One at a time: a tie that turns out redundant is just skipped.
                    for c in cs {
                        let mut t = trial.clone();
                        if solver::add_constraints(&mut t, &[c]).is_ok() {
                            trial = t;
                        }
                    }
                    *sk = trial;
                }
                Err(e) => why = Some(e),
            }
        });
        match why {
            Some(e) => self.notify(format!("Can't trim: {e}")),
            None => self.report(r),
        }
    }

    /// Switch dimension `ci` between driving and driven (reference).
    fn toggle_driven(&mut self, ci: usize) {
        let Some(i) = self.active else { return };
        self.float = None;
        let was_driven = self.sketches[i].is_driven(ci);
        let mut why = None;
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            if was_driven {
                if let Err(e) = solver::make_driving(sk, ci) {
                    why = Some(format!("{e}"));
                }
            } else {
                solver::make_driven(sk, ci);
            }
        });
        match why {
            Some(e) => self.notify(format!("Can't make it drive: {e}")),
            None => {
                self.notify(if was_driven { "Dimension now drives" } else { "Dimension is now reference only" }.into());
                self.report(r);
            }
        }
    }

    fn set_dimension(&mut self, ci: usize, v: f64) {
        let Some(i) = self.active else { return };
        if self.sketches[i].is_driven(ci) {
            self.message = Some("That's a reference dimension — right-click it to make it drive".into());
            return;
        }
        let is_angle = matches!(self.sketches[i].constraints.get(ci), Some(Constraint::Angle(..)));
        if is_angle && !(0.0..180.0).contains(&v) {
            self.message = Some("Angles go from 0° up to (not including) 180°".into());
            return;
        }
        if !is_angle && v <= 0.0 {
            self.message = Some("Dimensions must be greater than zero".into());
            return;
        }
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            if let Some(Constraint::Length(_, d) | Constraint::Diameter(_, d) | Constraint::Angle(_, _, d) | Constraint::Distance(_, _, d) | Constraint::Radius(_, d)) =
                sk.constraints.get_mut(ci)
            {
                *d = v;
            }
        });
        self.report(r);
    }

    fn delete_selection(&mut self) {
        let Some(i) = self.active else { return };
        let sel = std::mem::take(&mut self.editor.selection);
        let dims = std::mem::take(&mut self.dim_selection);
        self.float = None; // constraint indices may shift
        let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
            // Selected dimensions first (by index, before anything renumbers).
            let mut k = 0;
            sk.retain_constraints(|_| {
                let keep = !dims.contains(&k);
                k += 1;
                keep
            });
            sk.delete(&sel);
        });
        self.report(r);
    }

    fn undo_last(&mut self) {
        self.undo();
    }

    // ---------- extrude ----------

    fn open_extrude(&mut self, editing: Option<usize>) {
        self.finish_sketch();
        self.cancel_edge_dialog();
        self.cancel_revolve();
        self.message = None;
        let d = match editing.and_then(|k| self.extrudes.get(k)) {
            Some(e) => ExtrudeDialog {
                editing,
                sketch: Some(e.sketch),
                profiles: e.profiles.clone(),
                distance: e.distance,
                op: e.op,
            },
            None => ExtrudeDialog {
                editing: None,
                sketch: None,
                profiles: Vec::new(),
                distance: 10.0,
                // Like Fusion: the first extrude makes a body, later ones join by default.
                op: if self.body_count > 0 { Operation::Join } else { Operation::NewBody },
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

    fn dialog_extrude(&self) -> Option<Extrude> {
        let d = self.dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.extrudes[k].name.clone(),
            None => format!("Extrude{}", self.extrudes.len() + 1),
        };
        Some(Extrude { op: d.op, name, sketch: d.sketch?, profiles: d.profiles.clone(), distance: d.distance })
    }

    fn commit_extrude(&mut self) {
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
                self.timeline.push(Feature::Extrude(self.extrudes.len() - 1));
            }
        }
        self.dialog = None;
        self.float = None;
        self.message = None;
        self.arrow_screen = None;
    }

    fn cancel_extrude(&mut self) {
        self.dialog = None;
        self.float = None;
        self.message = None;
        self.arrow_drag = None;
        self.arrow_screen = None;
    }

    /// Put the cursor back in the number box with its text selected.
    fn focus_distance(&mut self) {
        if let Some(fe) = self.float.as_mut() {
            if matches!(fe.target, EditTarget::ExtrudeDistance | EditTarget::EdgeSize | EditTarget::RevolveAngle | EditTarget::Corner(_)) {
                fe.focus = true;
            }
        }
    }

    /// The drag arrow for whichever box is open (extrude distance, fillet /
    /// chamfer size, revolve angle), with the value it shows now.
    fn current_handle(&self, regions: &[Vec<Region>]) -> Option<Handle> {
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
            return Some(Handle::Line { line, value: d.distance, target: HandleTarget::Extrude });
        }
        if let Some(d) = &self.edge_dialog {
            let line = handles::edge_handle(d.edges.first()?)?;
            return Some(Handle::Line { line, value: d.size, target: HandleTarget::EdgeSize });
        }
        if let Some(d) = &self.revolve_dialog {
            let s = d.sketch?;
            let r = regions.get(s)?.get(*d.profiles.first()?)?;
            let spot = self.plane_of(s).to_world(r.centroid());
            let (a, b) = self.revolve_axis_line(s, d.axis, 1.0)?;
            let turn = TurnHandle::new(a.as_dvec3(), (b - a).as_dvec3(), spot.as_dvec3())?;
            return Some(Handle::Turn { turn, value: d.angle });
        }
        None
    }

    /// Put a dragged arrow's value into its box.
    fn set_handle_value(&mut self, target: HandleTarget, v: f64) {
        match target {
            HandleTarget::Extrude => {
                if let Some(d) = self.dialog.as_mut() {
                    d.distance = v;
                }
            }
            HandleTarget::EdgeSize => {
                if let Some(d) = self.edge_dialog.as_mut() {
                    d.size = v.max(0.01);
                }
            }
            HandleTarget::RevolveAngle => {
                if let Some(d) = self.revolve_dialog.as_mut() {
                    d.angle = v.clamp(-360.0, 360.0);
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
    fn revolve_axis_line(&self, s: usize, which: RevolveAxis, reach: f64) -> Option<(Vec3, Vec3)> {
        let sk = self.sketches.get(s)?;
        let plane = self.plane_of(s);
        let reach = (self.camera.distance as f64) * 2.0 * reach;
        let (a, b) = match which {
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
    fn extrude_axis(&self, regions: &[Vec<Region>]) -> Option<(Vec3, Vec3)> {
        let d = self.dialog.as_ref()?;
        let s = d.sketch?;
        let r = regions.get(s)?.get(*d.profiles.first()?)?;
        let plane = self.plane_of(s);
        Some((plane.to_world(r.centroid()), plane.normal()))
    }

    /// Faces and edges picked on the far end of a deleted extrude/revolve
    /// stop following it; later feature numbers shift down.
    fn fix_follows(&mut self, revolve: bool, k: usize) {
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

    fn delete_extrude(&mut self, k: usize) {
        self.fix_follows(false, k);
        self.extrudes.remove(k);
        self.timeline.retain(|f| *f != Feature::Extrude(k));
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

    fn open_revolve(&mut self, editing: Option<usize>) {
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

    fn dialog_revolve(&self) -> Option<Revolve> {
        let d = self.revolve_dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.revolves[k].name.clone(),
            None => format!("Revolve{}", self.revolves.len() + 1),
        };
        Some(Revolve { op: d.op, name, sketch: d.sketch?, profiles: d.profiles.clone(), axis: d.axis, angle: d.angle })
    }

    fn commit_revolve(&mut self) {
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
                self.timeline.push(Feature::Revolve(self.revolves.len() - 1));
            }
        }
        self.cancel_revolve();
    }

    fn cancel_revolve(&mut self) {
        self.revolve_dialog = None;
        self.axis_hover = None;
        if self.float.as_ref().is_some_and(|f| f.target == EditTarget::RevolveAngle) {
            self.float = None;
        }
    }

    fn delete_revolve(&mut self, k: usize) {
        self.fix_follows(true, k);
        self.revolves.remove(k);
        self.timeline.retain(|f| *f != Feature::Revolve(k));
        for f in &mut self.timeline {
            if let Feature::Revolve(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_revolve();
    }

    fn set_angle_text(&mut self) {
        if let (Some(fe), Some(d)) = (self.float.as_mut(), self.revolve_dialog.as_ref()) {
            if fe.target == EditTarget::RevolveAngle {
                fe.text = format!("{:.1}", d.angle);
            }
        }
    }

    /// The profile selection of whichever dialog is open (extrude or revolve).
    fn profile_pick(&self) -> Option<(Option<usize>, &[usize])> {
        if let Some(d) = &self.dialog {
            Some((d.sketch, d.profiles.as_slice()))
        } else {
            self.revolve_dialog.as_ref().map(|d| (d.sketch, d.profiles.as_slice()))
        }
    }

    fn profile_pick_mut(&mut self) -> Option<(&mut Option<usize>, &mut Vec<usize>)> {
        if self.dialog.is_some() {
            self.dialog.as_mut().map(|d| (&mut d.sketch, &mut d.profiles))
        } else {
            self.revolve_dialog.as_mut().map(|d| (&mut d.sketch, &mut d.profiles))
        }
    }

    // ---------- fillet / chamfer ----------

    fn open_edge_dialog(&mut self, editing: Option<usize>, kind: EdgeKind) {
        self.finish_sketch();
        self.cancel_extrude();
        self.cancel_revolve();
        self.message = None;
        let d = match editing.and_then(|k| self.edge_features.get(k)) {
            Some(f) => EdgeDialog { editing, kind: f.kind, size: f.size, edges: f.edges.clone() },
            None => EdgeDialog { editing: None, kind, size: 2.0, edges: Vec::new() },
        };
        self.float = Some(FloatEdit { target: EditTarget::EdgeSize, text: format!("{:.2}", d.size), focus: true, keep_text: false });
        self.edge_dialog = Some(d);
    }

    fn dialog_edge_feature(&self) -> Option<EdgeFeature> {
        let d = self.edge_dialog.as_ref()?;
        let name = match d.editing {
            Some(k) => self.edge_features[k].name.clone(),
            None => {
                let n = self.edge_features.iter().filter(|f| f.kind == d.kind).count() + 1;
                format!("{}{n}", d.kind.name())
            }
        };
        Some(EdgeFeature { name, kind: d.kind, size: d.size, edges: d.edges.clone() })
    }

    fn commit_edge_feature(&mut self) {
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
                self.timeline.push(Feature::Edge(self.edge_features.len() - 1));
            }
        }
        self.cancel_edge_dialog();
    }

    fn cancel_edge_dialog(&mut self) {
        self.edge_dialog = None;
        self.edge_hover = None;
        if self.float.as_ref().is_some_and(|f| f.target == EditTarget::EdgeSize) {
            self.float = None;
        }
        self.message = None;
    }

    fn delete_edge_feature(&mut self, k: usize) {
        self.edge_features.remove(k);
        self.timeline.retain(|f| *f != Feature::Edge(k));
        for f in &mut self.timeline {
            if let Feature::Edge(j) = f {
                if *j > k {
                    *j -= 1;
                }
            }
        }
        self.cancel_edge_dialog();
    }

    fn set_edge_size_text(&mut self) {
        if let (Some(fe), Some(d)) = (self.float.as_mut(), self.edge_dialog.as_ref()) {
            if fe.target == EditTarget::EdgeSize {
                fe.text = format!("{:.2}", d.size);
            }
        }
    }

    fn set_distance_text(&mut self) {
        let Some(fe) = self.float.as_mut() else { return };
        match fe.target {
            EditTarget::ExtrudeDistance => {
                if let Some(d) = self.dialog.as_ref() {
                    fe.text = format!("{:.2}", d.distance);
                }
            }
            EditTarget::EdgeSize => {
                if let Some(d) = self.edge_dialog.as_ref() {
                    fe.text = format!("{:.2}", d.size);
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

    /// The ribbon: quick-access icons (file, save, undo, redo), then tool
    /// groups with their names underneath, like Fusion's toolbar.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        // One group: a row of icons with a small label centred under it. The
        // label area is exactly as wide as the icons (a centred layout would
        // stretch across the whole toolbar and push the other groups away).
        fn group(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
            ui.vertical(|ui| {
                let font = egui::FontId::proportional(11.0);
                let text_w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x;
                // A short group is as wide as its label, so the label never
                // runs into the next group.
                ui.set_min_width(text_w + 6.0);
                // Centre the icons under a wider label: pad by half the
                // difference, using the icons' width measured last frame.
                let id = egui::Id::new(("toolbar_group", label));
                let icons_w: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.0);
                let pad = ((text_w + 6.0 - icons_w) * 0.5).max(0.0);
                let row = ui
                    .horizontal(|ui| {
                        ui.add_space(pad);
                        let start = ui.cursor().min.x;
                        add(ui);
                        ui.cursor().min.x - start
                    })
                    .inner;
                if (row - icons_w).abs() > 0.5 {
                    ui.ctx().data_mut(|d| d.insert_temp(id, row));
                    ui.ctx().request_repaint();
                }
                let width = (row + pad).max(text_w + 6.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::hover());
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, Color32::from_gray(150));
            });
            ui.separator();
        }
        // Which workspace you're in, like Fusion's SOLID / SKETCH tabs.
        let sketching = self.active.is_some();
        ui.horizontal(|ui| {
            for (name, on) in [("SOLID", !sketching), ("SKETCH", sketching)] {
                let text = egui::RichText::new(name).size(12.0);
                let text = if on { text.color(Color32::from_rgb(110, 170, 255)).strong() } else { text.color(Color32::from_gray(110)) };
                ui.label(text);
                ui.add_space(10.0);
            }
            if let Some(i) = self.active {
                ui.weak(format!("— editing {}", self.sketches[i].name));
            }
        });
        ui.horizontal(|ui| {
            // --- quick access ---
            group(ui, "FILE", |ui| {
                let file = icons::button(ui, Icon::File, "File: new, open, save, export", true, false);
                egui::Popup::menu(&file).show(|ui| {
                    for (cmd, label) in [
                        (Cmd::New, "New                Ctrl+N"),
                        (Cmd::Open, "Open…             Ctrl+O"),
                        (Cmd::Save, "Save                Ctrl+S"),
                        (Cmd::SaveAs, "Save As…  Ctrl+Shift+S"),
                    ] {
                        if ui.button(label).clicked() {
                            self.cmd = Some(cmd);
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.menu_button("Input device", |ui| {
                        ui.radio_value(&mut self.touchpad, false, "Mouse")
                            .on_hover_text("Scroll zooms · right/middle-drag pans · drag a dimension to move it");
                        ui.radio_value(&mut self.touchpad, true, "Touchpad").on_hover_text(
                            "Two-finger scroll pans · Shift+two-finger orbits · Ctrl+two-finger zooms · \
                             click a selected dimension to pick it up, click again to drop it",
                        );
                    });
                    ui.menu_button("Export", |ui| {
                        for f in export::Format::ALL {
                            if ui.button(format!("{}…", f.label())).clicked() {
                                self.cmd = Some(Cmd::Export(f));
                                ui.close();
                            }
                        }
                    });
                });
                if icons::button(ui, Icon::Save, "Save (Ctrl+S)", true, false).clicked() {
                    self.cmd = Some(Cmd::Save);
                }
                if icons::button(ui, Icon::Undo, "Undo (Ctrl+Z)", self.can_undo(), false).clicked() {
                    self.undo();
                }
                if icons::button(ui, Icon::Redo, "Redo (Ctrl+Shift+Z or Ctrl+Y)", self.history.can_redo(), false).clicked() {
                    self.redo();
                }
            });

            if let Some(i) = self.active {
                // --- sketch mode ---
                group(ui, "CREATE", |ui| {
                    for (tool, icon, tip) in [(Tool::Select, Icon::Select, "Select (Esc)"), (Tool::Line, Icon::Line, "Line (L)")] {
                        if icons::button(ui, icon, tip, true, self.editor.tool == tool).clicked() {
                            self.editor.set_tool(tool);
                        }
                    }
                    // Rectangle: the last rectangle tool used, ▾ for the others.
                    let rect_tool = self.rect_tool;
                    let tip = format!("{} (R)", rect_tool.name());
                    if icons::button(ui, rect_icon(rect_tool), &tip, true, self.editor.tool == rect_tool).clicked() {
                        self.editor.set_tool(rect_tool);
                    }
                    let more = icons::dropdown(ui, "More rectangle tools");
                    egui::Popup::menu(&more).show(|ui| {
                        for tool in Tool::RECTANGLES {
                            let row = ui.horizontal(|ui| {
                                let icon = icons::button(ui, rect_icon(tool), tool.name(), true, tool == rect_tool);
                                let name = ui.selectable_label(tool == rect_tool, tool.name());
                                icon.clicked() || name.clicked()
                            });
                            if row.inner {
                                self.rect_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    if icons::button(ui, Icon::Point, "Point: a sketch point (on a line if you click one)", true, self.editor.tool == Tool::Point)
                        .clicked()
                    {
                        self.editor.set_tool(Tool::Point);
                    }
                    // The circle button is whichever circle tool was used last;
                    // the arrow beside it picks another.
                    let circle = self.circle_tool;
                    let tip = format!("{} (C)", circle.name());
                    if icons::button(ui, circle_icon(circle), &tip, true, self.editor.tool == circle).clicked() {
                        self.editor.set_tool(circle);
                    }
                    let more_circles = icons::dropdown(ui, "More circle tools");
                    egui::Popup::menu(&more_circles).show(|ui| {
                        for tool in Tool::CIRCLES {
                            let row = ui.horizontal(|ui| {
                                let icon = icons::button(ui, circle_icon(tool), tool.name(), true, tool == circle);
                                let name = ui.selectable_label(tool == circle, tool.name());
                                icon.clicked() || name.clicked()
                            });
                            if row.inner {
                                self.circle_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    // Arc: the last arc tool used, ▾ for the other.
                    let arc_tool = self.arc_tool;
                    let tip = format!("{}", arc_tool.name());
                    let arc_icon = |t: Tool| if t == Tool::ArcCenter { Icon::ArcCenter } else { Icon::Arc3Point };
                    if icons::button(ui, arc_icon(arc_tool), &tip, true, self.editor.tool == arc_tool).clicked() {
                        self.editor.set_tool(arc_tool);
                    }
                    let more_arcs = icons::dropdown(ui, "More arc tools");
                    egui::Popup::menu(&more_arcs).show(|ui| {
                        for tool in Tool::ARCS {
                            let row = ui.horizontal(|ui| {
                                let icon = icons::button(ui, arc_icon(tool), tool.name(), true, tool == arc_tool);
                                let name = ui.selectable_label(tool == arc_tool, tool.name());
                                icon.clicked() || name.clicked()
                            });
                            if row.inner {
                                self.arc_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    // Polygon: the last polygon tool used, ▾ for the others, and the side count.
                    let poly_tool = self.poly_tool;
                    if icons::button(ui, Icon::Polygon, poly_tool.name(), true, Tool::POLYGONS.contains(&self.editor.tool)).clicked() {
                        self.editor.set_tool(poly_tool);
                    }
                    let more_polys = icons::dropdown(ui, "More polygon tools");
                    egui::Popup::menu(&more_polys).show(|ui| {
                        for tool in Tool::POLYGONS {
                            if ui.selectable_label(tool == poly_tool, tool.name()).clicked() {
                                self.poly_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    let mut sides = self.editor.sides();
                    let r = ui
                        .add(egui::DragValue::new(&mut sides).range(3..=Editor::MAX_SIDES).prefix("sides "))
                        .on_hover_text("Number of sides (also ↑ / ↓ while a polygon tool is active)");
                    if r.changed() {
                        self.editor.polygon_sides = sides;
                    }
                });
                let _ = i;
                group(ui, "MODIFY", |ui| {
                    for (tool, icon, tip) in [
                        (Tool::SketchFillet, Icon::Fillet, "Fillet: click a corner (or its two lines), type the radius"),
                        (Tool::SketchChamfer, Icon::Chamfer, "Chamfer: click a corner (or its two lines), type the distance — or two, like 2, 5 (first = first line)"),
                    ] {
                        if icons::button(ui, icon, tip, true, self.editor.tool == tool).clicked() {
                            self.editor.set_tool(tool);
                        }
                    }
                    if icons::button(ui, Icon::Trim, "Trim: click the part of a line to remove, back to the nearest crossing", true, self.editor.tool == Tool::Trim).clicked() {
                        self.editor.set_tool(Tool::Trim);
                    }
                    let tip = "Construction (X): toggle the selected geometry, or what you draw next";
                    if icons::button(ui, Icon::Construction, tip, true, self.editor.construction).clicked() {
                        self.toggle_construction();
                    }
                });
                let mut chosen = None;
                group(ui, "CONSTRAINTS", |ui| {
                    for label in CONSTRAINT_ACTIONS {
                        let (icon, tip) = match label {
                            "Dimension" => (
                                Icon::Dimension,
                                "Dimension (D): one line → length · circle → diameter · two lines → angle · two points → distance",
                            ),
                            "Horizontal" => (Icon::Horizontal, "Horizontal"),
                            "Vertical" => (Icon::Vertical, "Vertical"),
                            "Parallel" => (Icon::Parallel, "Parallel: two lines"),
                            "Perpendicular" => (Icon::Perpendicular, "Perpendicular: two lines"),
                            "Equal" => (Icon::Equal, "Equal: two lines or two circles"),
                            "Tangent" => (Icon::Tangent, "Tangent: a line and a circle"),
                            _ => (Icon::Fix, "Fix: lock points where they are"),
                        };
                        // Always clickable: with the right things selected it's added
                        // at once, otherwise it becomes the tool and you pick them.
                        if icons::button(ui, icon, tip, true, self.editor.tool == Tool::Constrain(label)).clicked() {
                            chosen = Some(label);
                        }
                    }
                });
                if let Some(label) = chosen {
                    self.constraint_button(label);
                }
                group(ui, "OPTIONS", |ui| {
                    if icons::button(ui, Icon::Grid, "Grid snap (G)", true, self.grid_snap).clicked() {
                        self.grid_snap = !self.grid_snap;
                    }
                });
                group(ui, "FINISH SKETCH", |ui| {
                    if icons::button(ui, Icon::FinishSketch, "Finish Sketch", true, false).clicked() {
                        self.finish_sketch();
                    }
                });
            } else {
                // --- solid mode ---
                let can_profile = self.sketches.iter().any(|s| !find_regions(s).is_empty());
                let has_bodies = self.body_count > 0;
                group(ui, "CREATE", |ui| {
                    let sketch = icons::button(ui, Icon::Sketch, "Create Sketch: pick a plane or a face", true, false);
                    egui::Popup::menu(&sketch).show(|ui| {
                        for plane in OriginPlane::ALL {
                            if ui.button(format!("{} plane", plane.name())).clicked() {
                                self.start_sketch(plane);
                                ui.close();
                            }
                        }
                        ui.separator();
                        let on_face = ui
                            .add_enabled(has_bodies, egui::Button::new("On a face…"))
                            .on_hover_text("Then click a flat face of a body");
                        if on_face.clicked() {
                            self.cancel_extrude();
                            self.cancel_revolve();
                            self.cancel_edge_dialog();
                            self.face_pick = true;
                            ui.close();
                        }
                    });
                    if icons::button(ui, Icon::Extrude, "Extrude (E)", can_profile, self.dialog.is_some()).clicked() {
                        self.open_extrude(None);
                    }
                    if icons::button(ui, Icon::Revolve, "Revolve: spin a profile around an axis", can_profile, self.revolve_dialog.is_some())
                        .clicked()
                    {
                        self.open_revolve(None);
                    }
                });
                let edge_kind = self.edge_dialog.as_ref().map(|d| d.kind);
                group(ui, "MODIFY", |ui| {
                    if icons::button(ui, Icon::Fillet, "Fillet (F): round edges", has_bodies, edge_kind == Some(EdgeKind::Fillet))
                        .clicked()
                    {
                        self.open_edge_dialog(None, EdgeKind::Fillet);
                    }
                    if icons::button(ui, Icon::Chamfer, "Chamfer: bevel edges", has_bodies, edge_kind == Some(EdgeKind::Chamfer))
                        .clicked()
                    {
                        self.open_edge_dialog(None, EdgeKind::Chamfer);
                    }
                });
            }
        });
    }

    fn browser(&mut self, ui: &mut egui::Ui) {
        ui.heading("Browser");
        ui.label("Origin");
        ui.label("Bodies");
        for k in 0..self.body_count {
            ui.horizontal(|ui| {
                let mut shown = !self.hidden_bodies.contains(&k);
                if ui.checkbox(&mut shown, format!("Body{}", k + 1)).on_hover_text("Show or hide this body").changed() {
                    if shown {
                        self.hidden_bodies.remove(&k);
                    } else {
                        self.hidden_bodies.insert(k);
                    }
                }
            });
        }
        ui.label("Sketches");
        let (mut edit, mut delete, mut toggle) = (None, None, None);
        for (i, s) in self.sketches.iter().enumerate() {
            ui.horizontal(|ui| {
                let mut shown = self.sketch_visible(i);
                if ui.checkbox(&mut shown, "").on_hover_text("Show or hide this sketch").changed() {
                    toggle = Some((i, shown));
                }
                let r = ui
                    .selectable_label(self.active == Some(i), format!("{} ({})", s.name, s.plane_name()))
                    .on_hover_text("Click to edit · right-click for more");
                if r.clicked() {
                    edit = Some(i);
                }
                r.context_menu(|ui| {
                    let uses = self.extrudes.iter().filter(|e| e.sketch == i).count();
                    let label = if uses == 0 {
                        "Delete".to_string()
                    } else {
                        format!("Delete (and the {uses} extrude{} using it)", if uses == 1 { "" } else { "s" })
                    };
                    if ui.button(label).clicked() {
                        delete = Some(i);
                        ui.close();
                    }
                });
            });
        }
        if let Some((i, shown)) = toggle {
            self.sketch_shown.insert(i, shown);
        }
        if let Some(i) = delete {
            self.delete_sketch(i);
        } else if let Some(i) = edit {
            self.edit_sketch(i);
        }
    }

    /// Right panel in sketch mode: solve status and the constraint list.
    fn constraints_panel(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.active else { return };
        ui.heading(self.sketches[i].name.as_str());
        match self.dof {
            Some(0) => ui.colored_label(Color32::from_rgb(120, 220, 120), "Fully constrained"),
            Some(n) => ui.colored_label(COL_UNDER, format!("{n} degrees of freedom left")),
            None => ui.colored_label(COL_FIXED, "Not solvable"),
        };
        if let Some(msg) = &self.message {
            ui.colored_label(COL_FIXED, msg.as_str());
        }
        ui.label(format!("{} selected", self.editor.selection.len()));
        ui.separator();
        ui.strong("Constraints");

        let mut remove = None;
        let mut new_value: Option<(usize, f64, bool)> = None; // (index, value, start of an edit)
        let sk = &self.sketches[i];
        for (ci, c) in sk.constraints.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("x").clicked() {
                    remove = Some(ci);
                }
                let what = match *c {
                    Constraint::Fix(p) => format!("Point{p}"),
                    Constraint::Distance(p, q, _) => format!("Point{p}, Point{q}"),
                    Constraint::OnAxis(p, _) => format!("Point{p}"),
                    Constraint::Midpoint(p, a, b) => format!("Point{p} between Point{a}, Point{b}"),
                    _ => c.entities().iter().map(|&e| sk.entity_name(e)).collect::<Vec<_>>().join(", "),
                };
                ui.label(format!("{} {}", c.glyph(), what));
                if let Constraint::Length(_, d) | Constraint::Diameter(_, d) | Constraint::Angle(_, _, d) | Constraint::Distance(_, _, d) | Constraint::Radius(_, d) = *c {
                    let mut v = d;
                    let field = if matches!(c, Constraint::Angle(..)) {
                        egui::DragValue::new(&mut v).speed(0.5).range(0.0..=179.999).min_decimals(1).suffix("°")
                    } else {
                        egui::DragValue::new(&mut v).speed(0.1).range(0.001..=1.0e6).min_decimals(2).suffix(" mm")
                    };
                    let r = ui.add(field);
                    if r.changed() {
                        new_value = Some((ci, v, false));
                    }
                    if r.drag_started() || r.gained_focus() {
                        // Snapshot once at the start of an edit, not every frame of it.
                        new_value = Some((ci, new_value.map_or(d, |n| n.1), true));
                    }
                }
            });
        }

        if let Some(ci) = remove {
            self.float = None; // constraint indices shift
            self.dim_selection.clear();
            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
                sk.remove_constraint(ci);
            });
            self.report(r);
        } else if let Some((ci, v, start)) = new_value {
            if start && self.undo.last() != Some(&self.sketches[i]) {
                self.undo.push(self.sketches[i].clone());
            }
            let r = apply_edit(&mut self.sketches[i], None, |sk| match &mut sk.constraints[ci] {
                Constraint::Length(_, d) | Constraint::Diameter(_, d) | Constraint::Angle(_, _, d) | Constraint::Distance(_, _, d) | Constraint::Radius(_, d) => *d = v,
                _ => {}
            });
            self.report(r);
        }
    }

    /// Right panel while extruding.
    fn extrude_panel(&mut self, ui: &mut egui::Ui) {
        let dialog_error = self.dialog_error.clone();
        let Some(d) = self.dialog.as_mut() else { return };
        ui.heading(if d.editing.is_some() { "Edit Extrude" } else { "Extrude" });
        ui.label(format!("Profiles: {} selected", d.profiles.len()));
        ui.weak("Click shaded regions to add or remove them.");
        ui.separator();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Distance");
            changed |= ui
                .add(egui::DragValue::new(&mut d.distance).speed(0.5).min_decimals(2).suffix(" mm"))
                .changed();
            if ui.button("Flip").clicked() {
                d.distance = -d.distance;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Operation");
            for op in Operation::ALL {
                if ui.selectable_label(d.op == op, op.name()).clicked() {
                    d.op = op;
                }
            }
        });
        let editing = d.editing;
        let has_profiles = !d.profiles.is_empty();
        if changed {
            self.set_distance_text();
        }
        if let (Some(e), true) = (&dialog_error, has_profiles) {
            ui.colored_label(COL_FIXED, e.as_str());
        }
        if let Some(msg) = &self.message {
            ui.colored_label(COL_FIXED, msg.as_str());
        }
        ui.separator();
        let ok = has_profiles && dialog_error.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                self.commit_extrude();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_extrude();
            }
            if let Some(k) = editing {
                if ui.button("Delete").clicked() {
                    self.delete_extrude(k);
                }
            }
        });
    }

    /// Right panel while revolving.
    fn revolve_panel(&mut self, ui: &mut egui::Ui) {
        let dialog_error = self.dialog_error.clone();
        let Some(d) = self.revolve_dialog.as_mut() else { return };
        ui.heading(if d.editing.is_some() { "Edit Revolve" } else { "Revolve" });
        ui.label(format!("Profiles: {} selected", d.profiles.len()));
        ui.weak("Click shaded regions to add or remove them.");
        ui.separator();
        ui.label("Axis");
        ui.horizontal(|ui| {
            if ui.selectable_label(d.axis == RevolveAxis::SketchX && !d.picking_axis, "Sketch X").clicked() {
                d.axis = RevolveAxis::SketchX;
                d.picking_axis = false;
            }
            if ui.selectable_label(d.axis == RevolveAxis::SketchY && !d.picking_axis, "Sketch Y").clicked() {
                d.axis = RevolveAxis::SketchY;
                d.picking_axis = false;
            }
            let on_line = matches!(d.axis, RevolveAxis::Line(_));
            if ui.selectable_label(d.picking_axis || on_line, "A sketch line…").clicked() {
                d.picking_axis = true;
            }
        });
        if d.picking_axis {
            ui.weak("Click a line in the profile's sketch.");
        }
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Angle");
            changed |= ui
                .add(egui::DragValue::new(&mut d.angle).speed(1.0).range(-360.0..=360.0).min_decimals(1).suffix("°"))
                .changed();
            if ui.button("Full").clicked() {
                d.angle = 360.0;
                changed = true;
            }
            if ui.button("Flip").clicked() {
                d.angle = -d.angle;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Operation");
            for op in Operation::ALL {
                if ui.selectable_label(d.op == op, op.name()).clicked() {
                    d.op = op;
                }
            }
        });
        let editing = d.editing;
        let has_profiles = !d.profiles.is_empty();
        if changed {
            self.set_angle_text();
        }
        if let (Some(e), true) = (&dialog_error, has_profiles) {
            ui.colored_label(COL_FIXED, e.as_str());
        }
        if let Some(msg) = &self.message {
            ui.colored_label(COL_FIXED, msg.as_str());
        }
        ui.separator();
        let ok = has_profiles && dialog_error.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                self.commit_revolve();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_revolve();
            }
            if let Some(k) = editing {
                if ui.button("Delete").clicked() {
                    self.delete_revolve(k);
                }
            }
        });
    }

    /// Right panel while filleting / chamfering.
    fn edge_panel(&mut self, ui: &mut egui::Ui) {
        let dialog_error = self.dialog_error.clone();
        let Some(d) = self.edge_dialog.as_mut() else { return };
        ui.heading(if d.editing.is_some() { format!("Edit {}", d.kind.name()) } else { d.kind.name().to_string() });
        ui.horizontal(|ui| {
            for kind in [EdgeKind::Fillet, EdgeKind::Chamfer] {
                if ui.selectable_label(d.kind == kind, kind.name()).clicked() {
                    d.kind = kind;
                }
            }
        });
        ui.label(format!("Edges: {} selected", d.edges.len()));
        ui.weak("Click edges on a body to add or remove them.");
        ui.separator();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(d.kind.size_label());
            changed |= ui
                .add(egui::DragValue::new(&mut d.size).speed(0.1).range(0.001..=1.0e5).min_decimals(2).suffix(" mm"))
                .changed();
        });
        let editing = d.editing;
        let has_edges = !d.edges.is_empty();
        if changed {
            self.set_edge_size_text();
        }
        if let (Some(e), true) = (&dialog_error, has_edges) {
            ui.colored_label(COL_FIXED, e.as_str());
        }
        if let Some(msg) = &self.message {
            ui.colored_label(COL_FIXED, msg.as_str());
        }
        ui.separator();
        let ok = has_edges && dialog_error.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                self.commit_edge_feature();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_edge_dialog();
            }
            if let Some(k) = editing {
                if ui.button("Delete").clicked() {
                    self.delete_edge_feature(k);
                }
            }
        });
    }

    fn timeline_bar(&mut self, ui: &mut egui::Ui) {
        let mut action = None;
        let mut delete = None;
        ui.horizontal(|ui| {
            ui.label("Timeline:");
            if self.timeline.is_empty() {
                ui.weak("(empty)");
            }
            for f in &self.timeline {
                let (name, selected) = match *f {
                    Feature::Sketch(i) => (self.sketches[i].name.clone(), self.active == Some(i)),
                    Feature::Extrude(k) => (
                        self.extrudes[k].name.clone(),
                        self.dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                    Feature::Edge(k) => (
                        self.edge_features[k].name.clone(),
                        self.edge_dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                    Feature::Revolve(k) => (
                        self.revolves[k].name.clone(),
                        self.revolve_dialog.as_ref().is_some_and(|d| d.editing == Some(k)),
                    ),
                };
                let error = match *f {
                    Feature::Extrude(k) => self.feature_errors.get(k).cloned().flatten(),
                    Feature::Edge(k) => self.edge_feature_errors.get(k).cloned().flatten(),
                    Feature::Revolve(k) => self.revolve_errors.get(k).cloned().flatten(),
                    Feature::Sketch(k) => self.sketch_errors.get(k).cloned().flatten(),
                };
                let text = match &error {
                    Some(_) => egui::RichText::new(format!("{name} ⚠")).color(COL_FIXED),
                    None => egui::RichText::new(name),
                };
                let r = ui.selectable_label(selected, text);
                let r = match error {
                    Some(e) => r.on_hover_text(e),
                    None => r,
                };
                if r.clicked() {
                    action = Some(*f);
                }
                r.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        delete = Some(*f);
                        ui.close();
                    }
                });
            }
        });
        match delete {
            Some(Feature::Sketch(i)) => self.delete_sketch(i),
            Some(Feature::Extrude(k)) => self.delete_extrude(k),
            Some(Feature::Edge(k)) => self.delete_edge_feature(k),
            Some(Feature::Revolve(k)) => self.delete_revolve(k),
            None => {}
        }
        if delete.is_some() {
            return;
        }
        match action {
            Some(Feature::Sketch(i)) => self.edit_sketch(i),
            Some(Feature::Extrude(k)) => self.open_extrude(Some(k)),
            Some(Feature::Revolve(k)) => self.open_revolve(Some(k)),
            Some(Feature::Edge(k)) => {
                let kind = self.edge_features[k].kind;
                self.open_edge_dialog(Some(k), kind);
            }
            None => {}
        }
    }

    /// The floating number box (dimension values, extrude distance).
    fn float_editor(&mut self, ctx: &egui::Context) {
        let (Some(anchor), Some(fe)) = (self.float_anchor, self.float.as_mut()) else { return };
        let target = fe.target;
        let degrees = match target {
            EditTarget::RevolveAngle => true,
            EditTarget::Dimension(ci) => self
                .active
                .and_then(|i| self.sketches.get(i))
                .and_then(|sk| sk.constraints.get(ci))
                .is_some_and(|c| matches!(c, Constraint::Angle(..))),
            _ => false,
        };
        let mut commit: Option<Option<f64>> = None; // Some(parsed) when Enter was pressed
        let mut close = false;
        let mut live: Option<f64> = None;
        let mut corner_text: Option<String> = None;
        let mut extrude_ok = false;
        let mut extrude_cancel = false;
        egui::Area::new(egui::Id::new("float_edit")).order(egui::Order::Foreground).fixed_pos(anchor).show(
            ctx,
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut out = egui::TextEdit::singleline(&mut fe.text)
                            .id(egui::Id::new("float_edit_text"))
                            .desired_width(70.0)
                            .show(ui);
                        ui.label(if degrees { "°" } else { "mm" });
                        let (lost, changed, id) = (out.response.lost_focus(), out.response.changed(), out.response.id);
                        if fe.focus {
                            // Focus and select everything, so typing replaces the value.
                            out.response.request_focus();
                            let range = if fe.keep_text {
                                egui::text::CCursorRange::one(egui::text::CCursor::new(fe.text.chars().count()))
                            } else {
                                egui::text::CCursorRange::select_all(&out.galley)
                            };
                            out.state.cursor.set_char_range(Some(range));
                            out.state.store(ui.ctx(), id);
                            fe.focus = false;
                        }
                        let parsed = parse_mm(&fe.text);
                        let (enter, esc) =
                            ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                        match target {
                            // Stays open until Enter / Esc, so its arrow can be dragged.
                            EditTarget::Corner(_) => {
                                if lost {
                                    if enter {
                                        corner_text = Some(fe.text.clone());
                                    }
                                    if enter || esc {
                                        close = true;
                                    }
                                }
                            }
                            EditTarget::Dimension(_) | EditTarget::NewCircle => {
                                if lost {
                                    if enter {
                                        commit = Some(parsed);
                                    }
                                    close = true; // Esc or clicking away cancels
                                }
                            }
                            EditTarget::ExtrudeDistance | EditTarget::EdgeSize | EditTarget::RevolveAngle => {
                                if changed {
                                    live = parsed;
                                }
                                if lost && enter {
                                    live = parsed;
                                    extrude_ok = true;
                                }
                                if lost && esc {
                                    extrude_cancel = true;
                                }
                            }
                        }
                    });
                });
            },
        );
        match target {
            EditTarget::Corner(p) => {
                if let Some(text) = corner_text {
                    match sketch::parse_sizes(&text) {
                        Some((d1, d2)) => self.finish_corner(p, d1, d2),
                        None => self.message = Some("Type a number (a chamfer can take two: 2, 5)".into()),
                    }
                }
                if close {
                    self.float = None;
                }
            }
            EditTarget::NewCircle => {
                match commit {
                    Some(Some(v)) => self.finish_circle_typed(v),
                    Some(None) => self.message = Some("Type a number".into()),
                    None => {}
                }
                if close {
                    self.float = None;
                }
            }
            EditTarget::Dimension(ci) => {
                match commit {
                    Some(Some(v)) => self.set_dimension(ci, v),
                    Some(None) => self.message = Some("Type a number".into()),
                    None => {}
                }
                if close {
                    self.float = None;
                }
            }
            EditTarget::ExtrudeDistance => {
                if let (Some(v), Some(d)) = (live, self.dialog.as_mut()) {
                    d.distance = v;
                }
                if extrude_cancel {
                    self.cancel_extrude();
                } else if extrude_ok {
                    self.commit_extrude();
                    if self.dialog.is_some() {
                        self.focus_distance(); // not valid yet (e.g. no profile): stay in the box
                    }
                }
            }
            EditTarget::RevolveAngle => {
                if let (Some(v), Some(d)) = (live, self.revolve_dialog.as_mut()) {
                    d.angle = v.clamp(-360.0, 360.0);
                }
                if extrude_cancel {
                    self.cancel_revolve();
                } else if extrude_ok {
                    self.commit_revolve();
                    if self.revolve_dialog.is_some() {
                        self.focus_distance();
                    }
                }
            }
            EditTarget::EdgeSize => {
                if let (Some(v), Some(d)) = (live, self.edge_dialog.as_mut()) {
                    if v > 0.0 {
                        d.size = v;
                    }
                }
                if extrude_cancel {
                    self.cancel_edge_dialog();
                } else if extrude_ok {
                    self.commit_edge_feature();
                    if self.edge_dialog.is_some() {
                        self.focus_distance();
                    }
                }
            }
        }
    }

    // ---------- viewport ----------

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let sketching = self.active.is_some();

        // --- view cube: sits on top of the viewport, so it gets clicks first ---
        let cube_center = rect.right_top() + egui::vec2(-CUBE_MARGIN, CUBE_TOP);
        let cube_rect = egui::Rect::from_center_size(cube_center, egui::vec2(CUBE_MARGIN * 1.8, CUBE_MARGIN * 1.8));
        let cube_resp = ui.interact(cube_rect, ui.id().with("view_cube"), egui::Sense::click());
        let cells = cube_cells();
        let now = ui.input(|inp| inp.time);
        let cube_hover = cube_resp
            .hover_pos()
            .and_then(|p| self.camera.cube_hit(&cells, (to_vec(p) - to_vec(cube_center)) / CUBE_SCALE))
            .map(|i| cells[i].dir);
        if cube_resp.clicked() {
            if let Some(dir) = cube_hover {
                // Already looking straight at it? Then flip to the opposite side.
                let dir = cube_click_target(-self.camera.basis().0, dir);
                let (yaw, pitch) = angles_for_dir(dir);
                let to = Pose { yaw: yaw.to_radians(), pitch: pitch.to_radians(), ..self.camera.pose() };
                self.move_camera(to, now);
            }
        }
        if let Some(t) = &self.turn {
            let k = ((now - t.start) / TURN_SECONDS).clamp(0.0, 1.0) as f32;
            let e = k * k * (3.0 - 2.0 * k); // ease in and out
            self.camera.set_pose(Pose::lerp(t.from, t.to, e));
            if k >= 1.0 {
                self.turn = None;
            } else {
                ui.ctx().request_repaint();
            }
        }
        let typing = ui.ctx().text_edit_focused();
        let press_px = ui.input(|i| i.pointer.press_origin()).map(to_vec);
        let hover_px = response.hover_pos().map(to_vec);
        let near_arrow = |p: Option<Vec2>, arrow: Option<(Vec2, Vec2)>| match (p, arrow) {
            (Some(p), Some((a, b))) => dist_to_segment_px(p, a, b) < 10.0,
            _ => false,
        };
        // Grabbing the extrude arrow (decided before the camera, so it doesn't orbit).
        let grab_arrow = self.arrow_screen.is_some()
            && response.drag_started_by(egui::PointerButton::Primary)
            && near_arrow(press_px, self.arrow_screen);
        let arrow_active = grab_arrow || self.arrow_drag.is_some();

        let min = to_vec(rect.min);
        let size = Vec2::new(rect.width().max(1.0), rect.height().max(1.0));
        let aspect = size.x / size.y;

        // --- camera input ---
        let shift = ui.input(|i| i.modifiers.shift);
        let d = response.drag_delta();
        let middle = response.dragged_by(egui::PointerButton::Middle);
        if (!sketching && !arrow_active && response.dragged_by(egui::PointerButton::Primary)) || (shift && middle) {
            self.turn = None;
            self.camera.orbit(d.x, d.y);
        } else if response.dragged_by(egui::PointerButton::Secondary) || middle {
            self.camera.pan(d.x, d.y);
        }
        if response.hovered() {
            // Zoom toward whatever is under the cursor.
            let at = hover_px.map_or(Vec2::ZERO, |p| ndc_from_pixel(p, min, size));
            let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta, i.zoom_delta()));
            if pinch != 1.0 {
                // Pinch on a touchpad, or Ctrl+scroll.
                self.camera.zoom_at(pinch.ln() / 0.002, aspect, at);
            }
            if scroll != egui::Vec2::ZERO {
                if self.touchpad {
                    // Two fingers: pan; with Shift: orbit.
                    self.turn = None;
                    if shift {
                        self.camera.orbit(scroll.x, scroll.y);
                    } else {
                        self.camera.pan(scroll.x, scroll.y);
                    }
                } else if scroll.y != 0.0 {
                    self.camera.zoom_at(scroll.y, aspect, at);
                }
            }
        }
        let painter = ui.painter_at(rect);
        let view = View { painter: &painter, vp: self.camera.view_proj(aspect), min, size, trace: None };
        let grid = GridSpec::for_distance(self.camera.distance);
        let dplane = self.active.map_or(OriginPlane::XY.plane(), |i| self.plane_of(i));
        let mm_per_px = (self.camera.distance * 2.0 * (FOV_Y * 0.5).tan() / size.y) as f64;
        let regions: Vec<Vec<Region>> = self.sketches.iter().map(find_regions).collect();

        // The corner size box only lives while its tool does.
        if matches!(self.float.as_ref().map(|f| f.target), Some(EditTarget::Corner(_)))
            && !(matches!(self.editor.tool, Tool::SketchFillet | Tool::SketchChamfer) && self.active.is_some())
        {
            self.float = None;
        }
        // --- arrow drag (extrude distance, fillet / chamfer size, revolve angle) ---
        if let Some(h) = self.current_handle(&regions) {
            // What the cursor at `px` means for this arrow: a distance along
            // it, or the angle it has been swung to.
            let sample = |px: Vec2| {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                match h {
                    Handle::Line { line, .. } => {
                        closest_param_on_line(&ray, line.origin.as_vec3(), line.dir.as_vec3()).map(|s| s as f64)
                    }
                    Handle::Turn { turn, .. } => turn.angle_at(ray.origin.as_dvec3(), ray.dir.as_dvec3()),
                }
            };
            let (value, target) = match h {
                Handle::Line { value, target, .. } => (value, target),
                Handle::Turn { value, .. } => (value, HandleTarget::RevolveAngle),
            };
            if grab_arrow {
                if let Some(c) = press_px.and_then(sample) {
                    self.arrow_drag = Some(c - value); // keep the grab point under the cursor
                    self.arrow_turn = Some((c, c));
                }
            }
            if let (Some(offset), Some(px)) = (self.arrow_drag, hover_px) {
                if response.dragged_by(egui::PointerButton::Primary) {
                    if let Some(c) = sample(px) {
                        let mut v = match (h, self.arrow_turn) {
                            (Handle::Turn { .. }, Some((acc, raw))) => {
                                let acc = acc + handles::angle_step(raw, c);
                                self.arrow_turn = Some((acc, c));
                                acc - offset
                            }
                            _ => c - offset,
                        };
                        if self.grid_snap {
                            let step = if matches!(h, Handle::Turn { .. }) { 1.0 } else { grid.step / 10.0 };
                            v = (v / step).round() * step;
                        }
                        self.set_handle_value(target, v);
                    }
                }
            }
        }
        if self.arrow_drag.is_some() && response.drag_stopped() {
            self.arrow_drag = None;
            self.arrow_turn = None;
            self.focus_distance();
        }

        let key_x = ui.input(|i| i.key_pressed(egui::Key::X) && !i.modifiers.command);
        let (key_l, key_r, key_c, key_g, key_d, key_e, key_f, esc, del, undo) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::L),
                i.key_pressed(egui::Key::R),
                i.key_pressed(egui::Key::C),
                i.key_pressed(egui::Key::G),
                i.key_pressed(egui::Key::D),
                i.key_pressed(egui::Key::E),
                i.key_pressed(egui::Key::F),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.key_pressed(egui::Key::Z) && i.modifiers.command,
            )
        });

        // --- create sketch on a face: hover and click a flat face ---
        self.face_hover = None;
        if self.face_pick {
            self.ensure_model(Slot::Committed);
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let tol = self.camera.distance as f64 * 1e-3;
                let found = {
                    let m = self.model(Slot::Committed);
                    m.bodies
                        .iter()
                        .enumerate()
                        .filter(|(b, _)| !self.hidden_bodies.contains(b))
                        .filter_map(|(b, body)| {
                            model::ray_hit_tri(&body.mesh, ray.origin, ray.dir).map(|(t, tri)| (t, tri, b))
                        })
                        .min_by(|x, y| x.0.total_cmp(&y.0))
                        .and_then(|(t, tri, b)| {
                            let body = &m.bodies[b];
                            let n = body.mesh.normal(tri).as_dvec3();
                            let p = ray.at(t).as_dvec3();
                            // Only flat faces: the triangle must lie on one of the body's planar faces.
                            body.flat_faces
                                .iter()
                                .find(|(fnormal, q)| fnormal.dot(n) > 0.9999 && fnormal.dot(p - *q).abs() < tol)
                                .map(|&(fnormal, q)| {
                                    // Remember if this face is the far end of an
                                    // extrude/revolve, so the sketch moves with it.
                                    let mut face = FacePlane::from_face(fnormal, q);
                                    face.follow = kernel::follow_for_plane(&m.caps, fnormal, q, FOLLOW_TOL);
                                    (b, face)
                                })
                        })
                };
                self.face_hover = found;
            }
            if response.clicked() {
                if let Some((_, face)) = self.face_hover {
                    self.start_sketch_on_face(face);
                }
            }
            if esc && !typing {
                self.face_pick = false;
            }
        }

        // --- extrude / revolve input: pick profiles (and a revolve axis) ---
        let mut hover_profile: Option<(usize, usize)> = None;
        self.axis_hover = None;
        let picking_axis = self.revolve_dialog.as_ref().is_some_and(|d| d.picking_axis);
        if picking_axis {
            // A line in the profile's sketch (or any sketch, before a profile is picked).
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let want = self.revolve_dialog.as_ref().and_then(|d| d.sketch);
                let tol = SNAP_PX as f64 * mm_per_px;
                for s in 0..self.sketches.len() {
                    if want.is_some_and(|w| w != s) {
                        continue;
                    }
                    let Some(p) = self.plane_of(s).intersect(&ray) else { continue };
                    let sk = &self.sketches[s];
                    let line = sk.entities.iter().enumerate().find(|(_, e)| match **e {
                        Entity::Line { a, b } => sketch::dist_to_segment(p, sk.points[a], sk.points[b]) <= tol,
                        _ => false,
                    });
                    if let Some((e, _)) = line {
                        self.axis_hover = Some((s, e));
                        break;
                    }
                }
            }
            if response.clicked() {
                if let (Some((s, e)), Some(d)) = (self.axis_hover, self.revolve_dialog.as_mut()) {
                    if d.sketch.is_none() {
                        d.sketch = Some(s);
                    }
                    d.axis = RevolveAxis::Line(e);
                    d.picking_axis = false;
                }
                self.focus_distance();
            }
            if esc && !typing {
                if let Some(d) = self.revolve_dialog.as_mut() {
                    d.picking_axis = false;
                }
            }
        } else if self.profile_pick().is_some() {
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let mut best: Option<(f32, f64)> = None;
                for s in 0..self.sketches.len() {
                    let Some((t, p)) = self.plane_of(s).intersect_t(&ray) else { continue };
                    for (ri, r) in regions.get(s).into_iter().flatten().enumerate() {
                        if !r.contains(p) {
                            continue;
                        }
                        let area = r.area();
                        let better = match best {
                            None => true,
                            Some((bt, ba)) => t < bt - 1e-3 || ((t - bt).abs() <= 1e-3 && area < ba),
                        };
                        if better {
                            best = Some((t, area));
                            hover_profile = Some((s, ri));
                        }
                    }
                }
            }
            if response.clicked() && !near_arrow(hover_px, self.arrow_screen) {
                self.focus_distance();
                if let (Some((s, ri)), Some((sketch, profiles))) = (hover_profile, self.profile_pick_mut()) {
                    if *sketch != Some(s) {
                        *sketch = Some(s);
                        *profiles = vec![ri];
                    } else if let Some(pos) = profiles.iter().position(|&p| p == ri) {
                        profiles.remove(pos);
                    } else {
                        profiles.push(ri);
                    }
                }
            }
            if esc && !typing {
                self.cancel_extrude();
                self.cancel_revolve();
            }
        } else if self.edge_dialog.is_some() {
            // --- fillet / chamfer input: pick edges on the model before this feature ---
            self.ensure_model(Slot::Base);
            self.edge_hover = None;
            if let Some(px) = hover_px {
                let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size));
                let base = self.model(Slot::Base);
                // How far the ray goes before it hits a body: edges further than
                // that are hidden behind something and can't be picked.
                let hit = base.bodies.iter().filter_map(|b| model::ray_hit(&b.mesh, ray.origin, ray.dir)).reduce(f32::min);
                let slack = self.camera.distance * 0.01;
                let mut best: Option<(f32, EdgeRef)> = None;
                for body in &base.bodies {
                    for e in &body.edges {
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
                self.edge_hover = best.map(|(_, r)| r);
            }
            if response.clicked() {
                // Remember if the edge lies on the far end of an extrude/revolve.
                let tagged = self.edge_hover.map(|mut r| {
                    r.follow = kernel::follow_for_edge(&self.model(Slot::Base).caps, r.mid, r.dir, FOLLOW_TOL);
                    r
                });
                if let (Some(r), Some(d)) = (tagged, self.edge_dialog.as_mut()) {
                    match d.edges.iter().position(|e| e.mid.distance(r.mid) < 1e-6) {
                        Some(i) => {
                            d.edges.remove(i);
                        }
                        None => d.edges.push(r),
                    }
                }
                self.focus_distance();
            }
            if esc && !typing {
                self.cancel_edge_dialog();
            }
        } else if !sketching && key_e && !typing && regions.iter().any(|r| !r.is_empty()) {
            self.open_extrude(None);
        } else if !sketching && key_f && !typing && self.body_count > 0 {
            self.open_edge_dialog(None, EdgeKind::Fillet);
        }

        // --- sketch input ---
        let mut cursor: Option<Anchor> = None; // drawing target (snapped)
        let mut hovered: Option<Sel> = None; // select-tool hover
        if let Some(i) = self.active {
            let plane = self.plane_of(i);
            let raw = |px: Vec2| plane.intersect(&self.camera.ray_from_ndc(aspect, ndc_from_pixel(px, min, size)));
            let raw_hover = hover_px.and_then(raw);
            let raw_press = ui.input(|i| i.pointer.press_origin()).map(to_vec).and_then(raw);
            // Over a dimension: its label, or near any of its lines.
            let over_label = hover_px.and_then(|px| {
                self.dim_labels
                    .iter()
                    .find(|(_, r, segs)| r.contains(to_pos(px)) || segs.iter().any(|s| dist_to_segment_px(px, s[0], s[1]) < 5.0))
                    .map(|(ci, ..)| *ci)
            });
            self.dim_hover = over_label;

            {
                let sk = &self.sketches[i];
                let tool = self.editor.tool;
                // Tools that pick existing geometry get a hover target…
                if matches!(
                    tool,
                    Tool::Select
                        | Tool::Constrain(_)
                        | Tool::Circle2Tangent
                        | Tool::Circle3Tangent
                        | Tool::SketchFillet
                        | Tool::SketchChamfer
                        | Tool::Trim
                ) && over_label.is_none()
                {
                    hovered = raw_hover.and_then(|p| sk.hit(p, SNAP_PX as f64 * mm_per_px));
                    if tool.tangent_lines() > 0 {
                        hovered = hovered.filter(|h| matches!(h, Sel::Entity(e) if sk.is_line(*e)));
                    }
                    if tool == Tool::Trim {
                        hovered = hovered.filter(|h| matches!(h, Sel::Entity(e) if sk.is_line(*e)));
                    }
                    if matches!(tool, Tool::SketchFillet | Tool::SketchChamfer) {
                        // Corners where exactly two lines meet, or lines to pick in pairs.
                        hovered = hovered.filter(|h| match h {
                            Sel::Point(p) => sk.corner_lines(*p).is_some(),
                            // Or a line: pick two lines instead of their corner.
                            Sel::Entity(e) => sk.is_line(*e),
                        });
                    }
                }
                // …drawing tools a snapped cursor.
                if matches!(tool, Tool::Select | Tool::Constrain(_) | Tool::SketchFillet | Tool::SketchChamfer | Tool::Trim) {
                } else if let Some(px) = hover_px {
                    let nearest = sk
                        .points
                        .iter()
                        .enumerate()
                        .filter_map(|(id, &p)| Some((id, view.project(plane.to_world(p))?.distance(px))))
                        .filter(|&(_, d)| d < SNAP_PX)
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    // Near a line (not at a point)? The new point will stay on it.
                    let on_line = raw_hover.and_then(|p| {
                        let tol = SNAP_PX as f64 * mm_per_px;
                        sk.entities
                            .iter()
                            .enumerate()
                            .filter_map(|(e, ent)| match *ent {
                                Entity::Line { a, b } => {
                                    let (a, b) = (sk.points[a], sk.points[b]);
                                    let ab = b - a;
                                    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-30)).clamp(0.0, 1.0);
                                    let q = a + ab * t;
                                    let d = q.distance(p);
                                    (d <= tol).then_some((e, q, d))
                                }
                                _ => None,
                            })
                            .min_by(|x, y| x.2.total_cmp(&y.2))
                    });
                    // Centre circle, edge near where it would touch a line: snap
                    // there exactly, so the circle is made tangent to it.
                    let tangent_spot = match (tool, self.editor.start, on_line) {
                        (Tool::Circle, Some(start), Some((e, q, _))) if sk.is_valid_anchor(start) => sk
                            .line_foot(e, sk.pos(start))
                            .filter(|f| f.distance(q) <= 1.5 * SNAP_PX as f64 * mm_per_px)
                            .map(|f| Anchor::OnLine(e, f)),
                        _ => None,
                    };
                    let arc3_third = tool == Tool::Arc3Point && self.editor.second.is_some();
                    cursor = match (nearest, on_line) {
                        (Some((id, _)), _) => Some(Anchor::Point(id)),
                        _ if tangent_spot.is_some() => tangent_spot,
                        // Tangent circles sit where the cursor really is, never on the grid.
                        _ if tool.tangent_lines() > 0 => raw_hover.map(Anchor::Free),
                        // 3-point arc, third click: the raw cursor (never the grid),
                        // with the centre snaps below.
                        _ if arc3_third => raw_hover.map(Anchor::Free),
                        (None, Some((e, q, _))) => Some(Anchor::OnLine(e, q)),
                        (None, None) => raw_hover.map(|p| {
                            // Near an origin axis: locked onto it (nearer axis wins).
                            let tol = SNAP_PX as f64 * mm_per_px;
                            match (p.y.abs() <= tol, p.x.abs() <= tol) {
                                (true, false) => Anchor::OnAxis(0, DVec2::new(if self.grid_snap { grid.snap(p).x } else { p.x }, 0.0)),
                                (false, true) => Anchor::OnAxis(1, DVec2::new(0.0, if self.grid_snap { grid.snap(p).y } else { p.y })),
                                (true, true) if p.y.abs() <= p.x.abs() => Anchor::OnAxis(0, DVec2::new(p.x, 0.0)),
                                (true, true) => Anchor::OnAxis(1, DVec2::new(0.0, p.y)),
                                _ => Anchor::Free(if self.grid_snap { grid.snap(p) } else { p }),
                            }
                        }),
                    };
                    if let (true, Some(Anchor::Free(p))) = (arc3_third, cursor) {
                        let g = self.grid_snap.then_some(grid);
                        cursor = Some(Anchor::Free(self.editor.arc3_snap(sk, p, g, tol_px(mm_per_px)).0));
                    }
                }
            }

            if !typing {
                if key_l {
                    self.editor.set_tool(Tool::Line);
                }
                if key_r {
                    self.editor.set_tool(self.rect_tool);
                }
                if key_c {
                    self.editor.set_tool(self.circle_tool);
                }
                if key_g {
                    self.grid_snap = !self.grid_snap;
                }
                if key_x {
                    self.toggle_construction();
                }
                if Tool::POLYGONS.contains(&self.editor.tool) {
                    let step = ui.input(|i| i.key_pressed(egui::Key::ArrowUp) as i32 - i.key_pressed(egui::Key::ArrowDown) as i32);
                    if step != 0 {
                        let n = (self.editor.sides() as i32 + step).clamp(3, Editor::MAX_SIDES as i32);
                        self.editor.polygon_sides = n as usize;
                    }
                }
                if key_d {
                    self.constraint_button("Dimension");
                }
                if del {
                    self.delete_selection();
                }
                if esc {
                    if self.editor.busy() {
                        self.editor.cancel();
                    } else if matches!(self.editor.tool, Tool::Constrain(_)) && !self.editor.selection.is_empty() {
                        self.editor.selection.clear(); // drop the picks, stay in the tool
                    } else if self.editor.tool != Tool::Select {
                        self.editor.set_tool(Tool::Select);
                    } else {
                        self.editor.selection.clear();
                    }
                }
                if undo {
                    self.undo_last();
                }
            }
            if response.secondary_clicked() {
                match self.dim_hover.filter(|&ci| ci < self.sketches[i].constraints.len()) {
                    // Right-click a dimension: driving ⇄ driven.
                    Some(ci) => self.toggle_driven(ci),
                    None => self.editor.cancel(),
                }
            }

            // Undo or delete above may have removed points that the cursor, the
            // tool or a drag still refer to. Drop those references before using them.
            {
                let sk = &self.sketches[i];
                if cursor.is_some_and(|a| !sk.is_valid_anchor(a)) {
                    cursor = None;
                }
                if self.editor.start.is_some_and(|a| !sk.is_valid_anchor(a)) {
                    self.editor.start = None;
                }
                if hovered.is_some_and(|s| !sk.is_valid_sel(s)) {
                    hovered = None;
                }
                if self.drag.is_some_and(|p| p >= sk.points.len()) {
                    self.drag = None;
                }
            }

            match self.editor.tool {
                Tool::Select => {
                    if response.drag_started_by(egui::PointerButton::Primary) {
                        let sk = &self.sketches[i];
                        if let (Some(ci), Some(q)) = (over_label, raw_press) {
                            // Grab a dimension's label to move it out of the way.
                            self.label_drag = Some((ci, q, sk.label_shift(ci)));
                        } else if let Some(Sel::Point(p)) = raw_press.and_then(|q| sk.hit(q, SNAP_PX as f64 * mm_per_px)) {
                            if !sk.is_fixed(p) {
                                self.undo.push(sk.clone());
                                self.drag = Some(p);
                            }
                        }
                    }
                    if let (Some((ci, start, before)), Some(now)) = (self.label_drag, raw_hover) {
                        if response.dragged_by(egui::PointerButton::Primary) {
                            self.sketches[i].set_label_shift(ci, before + (now - start));
                        }
                    }
                    if self.label_drag.is_some() && response.drag_stopped() {
                        self.label_drag = None;
                    }
                    if let (Some(p), Some(target)) = (self.drag, raw_hover) {
                        if response.dragged_by(egui::PointerButton::Primary) {
                            let target = if self.grid_snap { grid.snap(target) } else { target };
                            // Follows the cursor as far as the constraints allow.
                            let _ = solver::drag(&mut self.sketches[i], p, target);
                        }
                    }
                    if self.drag.is_some() && response.drag_stopped() {
                        self.drag = None;
                        self.dof = solver::solve(&mut self.sketches[i].clone(), &[]).ok().map(|s| s.dof);
                        if self.undo.last() == Some(&self.sketches[i]) {
                            self.undo.pop(); // drag didn't move anything
                        }
                    }
                    // Clicking dimensions: one click selects (Delete removes it),
                    // a double-click edits its value. On a touchpad (no easy
                    // dragging) a click on an already-selected dimension picks
                    // its label up; it follows the cursor until the next click.
                    let edit_value = |app: &mut Self, ci: usize| {
                        if app.sketches[i].is_driven(ci) {
                            app.notify("Reference dimension: right-click it to make it drive".into());
                            return;
                        }
                        if let Some(
                            Constraint::Length(_, d)
                            | Constraint::Diameter(_, d)
                            | Constraint::Angle(_, _, d)
                            | Constraint::Distance(_, _, d) | Constraint::Radius(_, d),
                        ) = app.sketches[i].constraints.get(ci)
                        {
                            app.float = Some(FloatEdit {
                                target: EditTarget::Dimension(ci),
                                text: format!("{d:.2}"),
                                focus: true,
                                keep_text: false,
                            });
                        }
                    };
                    if response.double_clicked() && over_label.is_some() {
                        // (Put back a label the first click of a touchpad
                        // double-click picked up.)
                        if let (true, Some((ci, _, before))) = (self.label_carry, self.label_drag) {
                            self.sketches[i].set_label_shift(ci, before);
                        }
                        self.label_carry = false;
                        self.label_drag = None;
                        if let Some(ci) = over_label {
                            edit_value(self, ci);
                        }
                    } else if self.label_carry {
                        if let (Some((ci, start, before)), Some(now)) = (self.label_drag, raw_hover) {
                            self.sketches[i].set_label_shift(ci, before + (now - start));
                        }
                        if esc {
                            if let Some((ci, _, before)) = self.label_drag {
                                self.sketches[i].set_label_shift(ci, before);
                            }
                        }
                        if response.clicked() || esc {
                            self.label_carry = false;
                            self.label_drag = None;
                        }
                    } else if response.clicked() {
                        match over_label {
                            Some(ci) if self.touchpad && self.dim_selection.contains(&ci) => {
                                if let Some(p) = raw_hover {
                                    self.label_drag = Some((ci, p, self.sketches[i].label_shift(ci)));
                                    self.label_carry = true;
                                }
                            }
                            Some(ci) => {
                                if let Some(k) = self.dim_selection.iter().position(|&c| c == ci) {
                                    self.dim_selection.remove(k);
                                } else {
                                    self.dim_selection.push(ci);
                                }
                            }
                            None => {
                                if hovered.is_none() {
                                    self.dim_selection.clear();
                                }
                                self.editor.select_click(hovered);
                            }
                        }
                    }
                }
                Tool::Constrain(label) => {
                    if label == "Dimension" {
                        // Picks so far → what would be dimensioned, with its label
                        // following the cursor until a click places it.
                        self.dim_preview = None;
                        if let (Some(c), Some(p)) = (self.dimension_candidate(), raw_hover) {
                            let sk = &self.sketches[i];
                            let hidden_painter = painter.with_clip_rect(egui::Rect::NOTHING);
                            let hidden = View { painter: &hidden_painter, ..view };
                            let centroid = sketch_centroid(&view, sk, &plane, to_vec(rect.center()));
                            let at_default = draw_dimension(&hidden, sk, &plane, &c, false, DVec2::ZERO, centroid, COL_DIM)
                                .and_then(|r| {
                                    let ray = self.camera.ray_from_ndc(aspect, ndc_from_pixel(to_vec(r.center()), min, size));
                                    plane.intersect(&ray)
                                });
                            if let Some(d0) = at_default {
                                self.dim_preview = Some((c, p - d0));
                            }
                        }
                        if response.clicked() {
                            match (hovered, self.dim_preview) {
                                (Some(sel), _) => {
                                    // Another pick: a second line (angle) or point (distance).
                                    if !self.editor.selection.contains(&sel) {
                                        self.editor.selection.push(sel);
                                        if self.editor.selection.len() > 2 || self.dimension_candidate().is_none() && self.editor.selection.len() == 2 {
                                            self.editor.selection = vec![sel];
                                        }
                                    }
                                }
                                (None, _) if over_label.is_some() => {} // clicked an existing dimension
                                (None, Some((c, shift))) => self.place_dimension(c, shift, None),
                                (None, None) => self.editor.selection.clear(),
                            }
                        }
                        // Or just start typing the value: placed where the label is now.
                        if !typing && self.float.is_none() {
                            let typed: String = ui.input(|inp| {
                                inp.events
                                    .iter()
                                    .filter_map(|e| match e {
                                        egui::Event::Text(t) => Some(t.clone()),
                                        _ => None,
                                    })
                                    .collect()
                            });
                            if typed.chars().next().is_some_and(|ch| ch.is_ascii_digit() || ch == '.') {
                                let place = self.dim_preview.or_else(|| self.dimension_candidate().map(|c| (c, DVec2::ZERO)));
                                if let Some((c, shift)) = place {
                                    self.place_dimension(c, shift, Some(typed));
                                }
                            }
                        }
                    } else if response.clicked() {
                        if let Some(sel) = hovered {
                            self.constraint_pick(label, sel);
                        } else {
                            self.editor.selection.clear();
                        }
                    }
                }
                Tool::Trim => {
                    // Click the stretch of a line to remove.
                    if response.clicked() {
                        match (hovered, raw_hover) {
                            (Some(Sel::Entity(e)), Some(p)) => self.trim_at(e, p),
                            _ => self.notify("Click a line to trim it back to the nearest crossing geometry".into()),
                        }
                    }
                }
                Tool::SketchFillet | Tool::SketchChamfer => {
                    // Click a corner, then type the size (Enter applies).
                    if response.clicked() && !near_arrow(hover_px, self.arrow_screen) {
                        match hovered {
                            Some(Sel::Point(p)) => {
                                self.corner_first = None;
                                self.float = Some(FloatEdit {
                                    target: EditTarget::Corner(p),
                                    text: format!("{:.2}", self.corner_size),
                                    focus: true,
                                    keep_text: false,
                                });
                            }
                            Some(Sel::Entity(e)) => {
                                // Two lines that meet at a corner work like the corner.
                                let first = match self.editor.selection.as_slice() {
                                    [Sel::Entity(f)] if *f != e => Some(*f),
                                    _ => None,
                                };
                                match first.map(|f| self.sketches[i].corner_between(f, e)) {
                                    Some(Some(p)) => {
                                        self.editor.selection.clear();
                                        self.corner_first = first; // a chamfer's first distance is on the first line picked
                                        self.float = Some(FloatEdit {
                                            target: EditTarget::Corner(p),
                                            text: format!("{:.2}", self.corner_size),
                                            focus: true,
                                            keep_text: false,
                                        });
                                    }
                                    Some(None) => {
                                        self.editor.selection = vec![Sel::Entity(e)];
                                        self.notify("Those two lines don't meet in a corner of just two lines".into());
                                    }
                                    None => self.editor.selection = vec![Sel::Entity(e)],
                                }
                            }
                            _ => {
                                self.editor.selection.clear();
                                self.float = None;
                                self.notify("Click a corner, or two lines that meet".into());
                            }
                        }
                    }
                }
                Tool::Circle2Tangent | Tool::Circle3Tangent => {
                    if response.clicked() {
                        let lines_needed = self.editor.tool.tangent_lines();
                        if let Some(Sel::Entity(e)) = hovered {
                            self.editor.pick_tangent_line(&self.sketches[i], e);
                        }
                        let ready = self.editor.tangent_lines.len() == lines_needed;
                        // Once the lines are picked, the preview follows the cursor
                        // (which side / region it sits in); a click away from the
                        // lines places it.
                        let go = ready && hovered.is_none();
                        if let (true, Some(p)) = (go, raw_hover) {
                            let ed = &mut self.editor;
                            let mut why = None;
                            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| match ed.tangent_click(sk, p) {
                                Ok(cs) => {
                                    for c in cs {
                                        let mut t = sk.clone();
                                        if solver::add_constraints(&mut t, &[c]).is_ok() {
                                            *sk = t;
                                        }
                                    }
                                }
                                Err(e) => why = Some(e),
                            });
                            match why {
                                Some(e) => self.notify(format!("Can't make the circle: {e}")),
                                None => self.report(r),
                            }
                        }
                    }
                }
                Tool::Line
                | Tool::Rectangle
                | Tool::Rectangle3Point
                | Tool::RectangleCenter
                | Tool::Point
                | Tool::Circle
                | Tool::Circle2Point
                | Tool::Circle3Point
                | Tool::Arc3Point
                | Tool::ArcCenter
                | Tool::PolygonCircumscribed
                | Tool::PolygonInscribed
                | Tool::PolygonEdge => {
                    // Centre circle: after the centre, typing a number sets the diameter.
                    if self.editor.tool == Tool::Circle && self.editor.start.is_some() && !typing && self.float.is_none() {
                        let typed: String = ui.input(|inp| {
                            inp.events
                                .iter()
                                .filter_map(|e| match e {
                                    egui::Event::Text(t) => Some(t.clone()),
                                    _ => None,
                                })
                                .collect()
                        });
                        if typed.chars().next().is_some_and(|ch| ch.is_ascii_digit() || ch == '.') {
                            self.float = Some(FloatEdit { target: EditTarget::NewCircle, text: typed, focus: true, keep_text: true });
                        }
                    }
                    if response.clicked() {
                        if let Some(at) = cursor {
                            let ed = &mut self.editor;
                            let r = apply_edit(&mut self.sketches[i], Some(&mut self.undo), |sk| {
                                // A line drawn nearly horizontal/vertical gets that constraint,
                                // unless it would over-constrain the sketch (then it's skipped).
                                for c in ed.click(sk, at) {
                                    let mut t = sk.clone();
                                    if solver::add_constraints(&mut t, &[c]).is_ok() {
                                        *sk = t;
                                    }
                                }
                            });
                            self.report(r);
                        }
                    }
                }
            }
        }

        // Input above can add or remove sketches (a click on a face starts a new
        // one; finishing an empty sketch drops it), so refresh everything that
        // was computed per sketch before drawing with it.
        let regions: Vec<Vec<Region>> = self.sketches.iter().map(find_regions).collect();

        // --- draw: background, grid, axes ---
        painter.rect_filled(rect, 0.0, Color32::from_gray(38));
        let rel = self.camera.target - dplane.origin;
        let centre = DVec2::new(rel.dot(dplane.u) as f64, rel.dot(dplane.v) as f64);
        let extent = (GRID_HALF_LINES as f64 * grid.step + centre.abs().max_element()) as f32;
        // Grid and axes as coloured world segments. With the GPU they're drawn
        // with depth, so a part hides the grid under it from above but not from
        // below; the CPU fallback just paints them first.
        let mut grid_segments: Vec<(Vec3, Vec3, [f32; 4])> = sketch::grid_lines(&dplane, centre, grid, GRID_HALF_LINES)
            .into_iter()
            .map(|(a, b, major)| {
                let g = if major { 85.0 / 255.0 } else { 60.0 / 255.0 };
                (a, b, [g, g, g, 1.0])
            })
            .collect();
        let mut axes = vec![(dplane.u * -extent, dplane.u * extent, dplane.u), (dplane.v * -extent, dplane.v * extent, dplane.v)];
        if self.active.is_none() {
            axes.push((Vec3::ZERO, Vec3::Z * extent * 0.5, Vec3::Z));
        }
        for &(a, b, axis) in &axes {
            let c = rgb(axis_color(axis));
            grid_segments.push((a, b, [c[0], c[1], c[2], 1.0]));
        }
        if !self.gpu {
            view.grid(&dplane, centre, grid);
            for &(a, b, axis) in &axes {
                view.line(a, b, Stroke::new(2.0, axis_color(axis)));
            }
        }

        // --- draw: bodies (from the kernel; with an open dialog previewed) ---
        let previewing = self.dialog.is_some() || self.edge_dialog.is_some() || self.revolve_dialog.is_some();
        // The committed design always drives the timeline errors and body list.
        self.ensure_model(Slot::Committed);
        self.feature_errors = self.model(Slot::Committed).errors.clone();
        self.edge_feature_errors = self.model(Slot::Committed).edge_errors.clone();
        self.sketch_errors = self.model(Slot::Committed).sketch_errors.clone();
        self.revolve_errors = self.model(Slot::Committed).revolve_errors.clone();
        self.body_count = self.model(Slot::Committed).bodies.len();
        let slot = if previewing { Slot::Preview } else { Slot::Committed };
        self.ensure_model(slot);
        let model = &self.caches[slot as usize].as_ref().expect("built above").model;
        self.dialog_error = if let Some(d) = &self.dialog {
            let k = d.editing.unwrap_or(self.extrudes.len());
            model.errors.get(k).cloned().flatten()
        } else if let Some(d) = &self.edge_dialog {
            let k = d.editing.unwrap_or(self.edge_features.len());
            model.edge_errors.get(k).cloned().flatten()
        } else if let Some(d) = &self.revolve_dialog {
            let k = d.editing.unwrap_or(self.revolves.len());
            model.revolve_errors.get(k).cloned().flatten()
        } else {
            None
        };
        let body_color = if previewing { COL_BODY_PREVIEW } else { COL_BODY };

        let hidden = if previewing { HashSet::new() } else { self.hidden_bodies.clone() };
        let meshes: Vec<(&Mesh, Color32)> = model
            .bodies
            .iter()
            .enumerate()
            .filter(|(k, _)| !hidden.contains(k))
            .map(|(_, b)| (&b.mesh, body_color))
            .collect();
        if self.gpu {
            let list: Vec<(&Mesh, [f32; 3])> = meshes.iter().map(|(m, c)| (*m, rgb(*c))).collect();
            let mut scene = gpu::Scene::build(&list, &self.camera, aspect);
            scene.add_lines(&grid_segments, &self.camera);
            painter.add(egui::Shape::Callback(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                SceneCallback { scene, size_points: [rect.width(), rect.height()] },
            )));
        } else {
            view.solids(&meshes, &self.camera);
        }

        // --- draw: the face a sketch would start on ---
        if let Some((b, face)) = self.face_hover {
            let model = self.model(Slot::Committed);
            if let Some(body) = model.bodies.get(b) {
                let n = face.normal();
                let fill = profile_fill(110);
                let mut overlay = egui::Mesh::default();
                for (t, tri) in body.mesh.tris.iter().enumerate() {
                    let [a, bb, c] = tri.map(|i| body.mesh.positions[i as usize]);
                    let on_face = body.mesh.normal(t).as_dvec3().dot(n) > 0.9999
                        && n.dot(a.as_dvec3() - face.origin).abs() < self.camera.distance as f64 * 1e-3;
                    if let (true, Some(pa), Some(pb), Some(pc)) = (on_face, view.project(a), view.project(bb), view.project(c)) {
                        let base = overlay.vertices.len() as u32;
                        for q in [pa, pb, pc] {
                            overlay.colored_vertex(to_pos(q), fill);
                        }
                        overlay.add_triangle(base, base + 1, base + 2);
                    }
                }
                painter.add(egui::Shape::mesh(overlay));
            }
        }
        if self.face_pick {
            painter.text(
                rect.center_top() + egui::vec2(0.0, 14.0),
                egui::Align2::CENTER_TOP,
                "Click a flat face to sketch on it   (Esc to cancel)",
                egui::FontId::proportional(15.0),
                Color32::from_gray(220),
            );
        }

        // --- draw: picked and hovered edges (fillet / chamfer) ---
        if self.edge_dialog.is_some() {
            self.ensure_model(Slot::Base); // the dialog may have opened this frame
        }
        if let Some(d) = &self.edge_dialog {
            let base = self.model(Slot::Base);
            for body in &base.bodies {
                for e in &body.edges {
                    let selected = d.edges.iter().any(|r| r.mid.distance(e.at.mid) < 1e-6);
                    let hovered = self.edge_hover.is_some_and(|r| r.mid.distance(e.at.mid) < 1e-6);
                    if selected || hovered {
                        let stroke = Stroke::new(3.0, if selected { COL_SELECT } else { COL_HOVER });
                        for w in e.points.windows(2) {
                            view.line(w[0], w[1], stroke);
                        }
                    }
                }
            }
        }


        // --- draw: the drag arrow ---
        let prev_arrow = self.arrow_screen.take();
        if let Some(h) = self.current_handle(&regions) {
            let len = ARROW_PX * mm_per_px as f32;
            let (base, dir, value) = match h {
                Handle::Line { line, value, .. } => (line.origin + line.dir * value, line.dir, value),
                Handle::Turn { turn, value } => (turn.position(value), turn.direction(value), value),
            };
            let sign = if value < 0.0 { -1.0 } else { 1.0 };
            let (base, tip) = (base.as_vec3(), base.as_vec3() + dir.as_vec3() * (sign * len));
            let hot = self.arrow_drag.is_some() || near_arrow(hover_px, prev_arrow);
            self.arrow_screen = view.arrow3d(base, tip, if hot { COL_ARROW_HOT } else { COL_ARROW });
        }

        // --- draw: the fillet / chamfer the size box would make ---
        if let (Some(EditTarget::Corner(p)), Some(i)) = (self.float.as_ref().map(|f| f.target), self.active) {
            if let Some(cut) = self.corner_preview(i, p) {
                let plane = self.plane_of(i);
                let stroke = Stroke::new(2.0, COL_PREVIEW);
                match cut.arc {
                    Some((c, r, a0, sweep)) => view.arc(&plane, c, r, a0, sweep, stroke),
                    None => view.line(plane.to_world(cut.a), plane.to_world(cut.b), stroke),
                }
            }
        }

        // --- draw: profile fills ---
        if let Some((pick_sketch, pick_profiles)) = self.profile_pick() {
            for s in 0..self.sketches.len() {
                let plane = self.plane_of(s);
                for (ri, r) in regions.get(s).into_iter().flatten().enumerate() {
                    let selected = pick_sketch == Some(s) && pick_profiles.contains(&ri);
                    let alpha = if selected {
                        140
                    } else if hover_profile == Some((s, ri)) {
                        90
                    } else {
                        35
                    };
                    view.fill_region(&plane, r, profile_fill(alpha));
                }
            }
        } else if let Some(i) = self.active {
            let plane = self.plane_of(i);
            for r in regions.get(i).into_iter().flatten() {
                view.fill_region(&plane, r, profile_fill(28)); // shows which loops are closed
            }
        }

        // --- draw: the revolve axis (and a line it could be) ---
        if let Some(d) = &self.revolve_dialog {
            let axis_line = |s: usize, which: RevolveAxis| self.revolve_axis_line(s, which, 1.0);
            if let Some((a, b)) = d.sketch.and_then(|s| axis_line(s, d.axis)) {
                view.line(a, b, Stroke::new(2.0, COL_SELECT));
            }
            if let Some((s, e)) = self.axis_hover {
                if let Some((a, b)) = axis_line(s, RevolveAxis::Line(e)) {
                    view.line(a, b, Stroke::new(2.0, COL_HOVER));
                }
            }
        }

        // --- draw: sketches ---
        self.dim_labels.clear();
        for (j, sk) in self.sketches.iter().enumerate() {
            let plane = self.plane_of(j);
            let active = self.active == Some(j);
            // Like Fusion: a sketch used by a feature hides itself (shown again while
            // editing it or picking profiles).
            // Shown: while editing it, while picking extrude profiles, or when
            // its visibility is on. Sketch lines draw over bodies, so a visible
            // sketch shows through the part it's on.
            if !active && self.profile_pick().is_none() && !self.sketch_visible(j) {
                continue;
            }
            let base = if !active {
                COL_INACTIVE
            } else if self.dof == Some(0) {
                COL_FULL
            } else {
                COL_UNDER
            };
            let colour_of = |s: Sel| {
                if !active {
                    base
                } else if self.editor.selection.contains(&s)
                    || matches!(s, Sel::Entity(e) if self.editor.tangent_lines.contains(&e))
                {
                    COL_SELECT
                } else if hovered == Some(s) {
                    COL_HOVER
                } else {
                    base
                }
            };
            for (e, ent) in sk.entities.iter().enumerate() {
                let stroke = Stroke::new(if active { 2.0 } else { 1.5 }, colour_of(Sel::Entity(e)));
                let dashed = sk.is_construction(e);
                match *ent {
                    Entity::Line { a, b } if dashed => {
                        view.dashed(plane.to_world(sk.points[a]), plane.to_world(sk.points[b]), stroke)
                    }
                    Entity::Line { a, b } => view.line(plane.to_world(sk.points[a]), plane.to_world(sk.points[b]), stroke),
                    Entity::Circle { center, radius } if dashed => view.dashed_circle(&plane, sk.points[center], radius, stroke),
                    Entity::Circle { center, radius } => view.circle(&plane, sk.points[center], radius, stroke),
                    Entity::Point { .. } => {} // drawn with the points
                    Entity::Arc { .. } => {
                        if let Some((c, r, a0, sweep)) = sk.arc_geometry(e) {
                            if dashed {
                                view.dashed_arc(&plane, c, r, a0, sweep, stroke);
                            } else {
                                view.arc(&plane, c, r, a0, sweep, stroke);
                            }
                        }
                    }
                }
            }
            if !active {
                continue;
            }
            let projected: Vec<Option<Vec2>> = sk.points.iter().map(|&p| view.project(plane.to_world(p))).collect();
            let uses = sk.point_uses();
            for (id, px) in projected.iter().enumerate() {
                if let Some(px) = *px {
                    let sel = Sel::Point(id);
                    let col = if self.editor.selection.contains(&sel) || hovered == Some(sel) {
                        colour_of(sel)
                    } else if id == Sketch::ORIGIN {
                        COL_ORIGIN
                    } else if sk.is_fixed(id) {
                        COL_FIXED
                    } else {
                        COL_POINT
                    };
                    if uses[id] >= 2 && id != Sketch::ORIGIN {
                        // A joint: lines that share this point stay connected.
                        painter.circle_filled(to_pos(px), 4.5, col);
                        painter.circle_filled(to_pos(px), 1.8, Color32::from_gray(20));
                    } else {
                        painter.circle_filled(to_pos(px), 3.5, col);
                    }
                }
            }

            // Dimensions sit outside the shape, other glyphs inside it.
            let visible: Vec<Vec2> = projected.iter().skip(1).flatten().copied().collect();
            let centroid = if visible.is_empty() {
                to_vec(rect.center())
            } else {
                visible.iter().copied().sum::<Vec2>() / visible.len() as f32
            };
            let mut glyphs: Vec<Vec<&str>> = vec![Vec::new(); sk.entities.len()];
            for (ci, c) in sk.constraints.iter().enumerate() {
                let label = match *c {
                    Constraint::Length(..)
                    | Constraint::Distance(..)
                    | Constraint::Diameter(..)
                    | Constraint::Radius(..)
                    | Constraint::Angle(..) => {
                        // Drawn with its lines recorded, so they can be clicked.
                        let colour = if self.dim_selection.contains(&ci) {
                            COL_SELECT
                        } else if self.dim_hover == Some(ci) {
                            COL_HOVER
                        } else if sk.is_driven(ci) {
                            COL_DRIVEN
                        } else {
                            COL_DIM
                        };
                        let trace = std::cell::RefCell::new(Vec::new());
                        let traced = View { trace: Some(&trace), ..view };
                        let r = draw_dimension(&traced, sk, &plane, c, sk.is_driven(ci), sk.label_shift(ci), centroid, colour);
                        r.map(|r| (r, trace.into_inner()))
                    }
                    Constraint::Fix(p) => {
                        if let Some(px) = projected[p] {
                            view.text(px + Vec2::new(0.0, -12.0), "Fix".into(), COL_FIXED);
                        }
                        None
                    }
                    _ => {
                        for e in c.entities() {
                            glyphs[e].push(c.glyph());
                        }
                        None
                    }
                };
                if let Some((r, segs)) = label {
                    self.dim_labels.push((ci, r, segs));
                }
            }
            // The dimension being placed (Dimension tool), following the cursor.
            if let (Tool::Constrain("Dimension"), Some((c, shift))) = (self.editor.tool, self.dim_preview) {
                if !self.editor.selection.is_empty() {
                    draw_dimension(&view, sk, &plane, &c, false, shift, centroid, COL_DIM.gamma_multiply(0.7));
                }
            }
            for (e, g) in glyphs.iter().enumerate() {
                if g.is_empty() {
                    continue;
                }
                let text = g.join("  ");
                match sk.entities[e] {
                    Entity::Line { a, b } => {
                        if let (Some(pa), Some(pb)) = (projected[a], projected[b]) {
                            let mid = (pa + pb) * 0.5;
                            let mut n = (pb - pa).normalize_or_zero().perp();
                            if (centroid - mid).dot(n) < 0.0 {
                                n = -n;
                            }
                            view.text_beside(mid, n, 8.0, text, COL_GLYPH);
                        }
                    }
                    Entity::Circle { center, radius } => {
                        let at = sk.points[center] + DVec2::new(-radius, radius) * std::f64::consts::FRAC_1_SQRT_2;
                        if let (Some(pc), Some(pa)) = (projected[center], view.project(plane.to_world(at))) {
                            view.text_beside(pa, (pa - pc).normalize_or_zero(), 6.0, text, COL_GLYPH);
                        }
                    }
                    Entity::Point { p } => {
                        if let Some(px) = projected[p] {
                            view.text_beside(px, Vec2::new(0.7, -0.7), 8.0, text, COL_GLYPH);
                        }
                    }
                    Entity::Arc { center, .. } => {
                        if let Some((c, r, a0, sweep)) = sk.arc_geometry(e) {
                            let mid = c + DVec2::from_angle(a0 + sweep * 0.5) * r;
                            if let (Some(pc), Some(pm)) = (projected[center], view.project(plane.to_world(mid))) {
                                view.text_beside(pm, (pm - pc).normalize_or_zero(), 6.0, text, COL_GLYPH);
                            }
                        }
                    }
                }
            }
        }

        // --- floating editor position ---
        self.float_anchor = match self.float.as_ref().map(|f| f.target) {
            Some(EditTarget::Dimension(ci)) => {
                self.dim_labels.iter().find(|(c, ..)| *c == ci).map(|(_, r, _)| r.center() - egui::vec2(52.0, 14.0))
            }
            Some(EditTarget::Corner(p)) => self.active.and_then(|i| {
                let at = *self.sketches[i].points.get(p)?;
                view.project(self.plane_of(i).to_world(at)).map(|q| to_pos(q) + egui::vec2(14.0, 10.0))
            }),
            Some(EditTarget::NewCircle) => self.active.and_then(|i| {
                let sk = &self.sketches[i];
                let start = self.editor.start.filter(|s| sk.is_valid_anchor(*s))?;
                view.project(self.plane_of(i).to_world(sk.pos(start))).map(|p| to_pos(p) + egui::vec2(16.0, 12.0))
            }),
            Some(EditTarget::RevolveAngle) => self.revolve_dialog.as_ref().and_then(|d| {
                let s = d.sketch?;
                let r = regions.get(s)?.get(*d.profiles.first()?)?;
                view.project(self.plane_of(s).to_world(r.centroid())).map(|p| to_pos(p) + egui::vec2(16.0, -16.0))
            }),
            Some(EditTarget::EdgeSize) => self.edge_dialog.as_ref().and_then(|d| {
                // Next to the first picked edge.
                let r = d.edges.first()?;
                view.project(r.mid.as_vec3()).map(|p| to_pos(p) + egui::vec2(18.0, -22.0))
            }),
            Some(EditTarget::ExtrudeDistance) => self.dialog.as_ref().and_then(|d| {
                let s = d.sketch?;
                let r = regions.get(s)?.get(*d.profiles.first()?)?;
                let plane = self.plane_of(s);
                let top = plane.to_world(r.centroid()) + plane.normal() * d.distance as f32;
                view.project(top).map(|p| to_pos(p) + egui::vec2(16.0, -16.0))
            }),
            None => None,
        };

        // --- draw: snap marker, tool preview, readouts ---
        let mut status = String::new();
        if let Some(i) = self.active {
            let sk = &self.sketches[i];
            let plane = self.plane_of(i);
            if let Some(at) = cursor {
                let here = sk.pos(at);
                if let Some(px) = view.project(plane.to_world(here)) {
                    match at {
                        Anchor::Point(_) => {
                            painter.circle_stroke(to_pos(px), 7.0, Stroke::new(2.0, COL_SNAP));
                        }
                        Anchor::Free(_) if self.grid_snap => {
                            painter.circle_filled(to_pos(px), 3.0, COL_PREVIEW);
                        }
                        Anchor::Free(_) => {}
                        Anchor::OnLine(..) | Anchor::OnAxis(..) => {
                            // A small diamond: "this point will stay on that line".
                            let c = to_pos(px);
                            painter.add(egui::Shape::closed_line(
                                vec![c + egui::vec2(0.0, -6.0), c + egui::vec2(6.0, 0.0), c + egui::vec2(0.0, 6.0), c + egui::vec2(-6.0, 0.0)],
                                Stroke::new(2.0, COL_SNAP),
                            ));
                        }
                    }
                }
                status = format!("x {:.2}  y {:.2} mm", here.x, here.y);
                if let Some(start) = self.editor.start {
                    let from = sk.pos(start);
                    let preview = Stroke::new(1.5, COL_PREVIEW);
                    let label = match self.editor.tool {
                        Tool::Rectangle | Tool::Rectangle3Point | Tool::RectangleCenter => {
                            match self.editor.rectangle_preview(sk, here) {
                                Some(c) => {
                                    for k in 0..4 {
                                        view.line(plane.to_world(c[k]), plane.to_world(c[(k + 1) % 4]), preview);
                                    }
                                    format!("{:.2} × {:.2} mm", c[0].distance(c[1]), c[1].distance(c[2]))
                                }
                                None => {
                                    // 3-point, first side: just that side so far.
                                    view.line(plane.to_world(from), plane.to_world(here), preview);
                                    format!("{:.2} mm", from.distance(here))
                                }
                            }
                        }
                        Tool::PolygonCircumscribed | Tool::PolygonInscribed | Tool::PolygonEdge => {
                            match self.editor.polygon_preview(sk, here) {
                                Some(p) => {
                                    let n = p.verts.len();
                                    for k in 0..n {
                                        view.line(plane.to_world(p.verts[k]), plane.to_world(p.verts[(k + 1) % n]), preview);
                                    }
                                    format!("{n} sides   side {:.2} mm   (↑/↓ changes the sides)", p.verts[0].distance(p.verts[1]))
                                }
                                None => {
                                    // Edge polygon, first edge so far.
                                    view.line(plane.to_world(from), plane.to_world(here), preview);
                                    format!("{:.2} mm", from.distance(here))
                                }
                            }
                        }
                        Tool::Circle => {
                            let r = from.distance(here);
                            view.circle(&plane, from, r, preview);
                            let tangent = matches!(at, Anchor::OnLine(e, q) if sk.line_foot(e, from).is_some_and(|f| f.distance(q) < 1e-9 * (1.0 + r)));
                            format!("Ø {:.2} mm{}   (or type the diameter)", 2.0 * r, if tangent { "  tangent" } else { "" })
                        }
                        Tool::Arc3Point | Tool::ArcCenter => match self.editor.arc_preview(sk, here) {
                            Some(a) => {
                                view.arc(&plane, a.center, a.radius, a.start, a.sweep, preview);
                                // Centre snapped? Mark it (idempotent on the snapped point).
                                let g = self.grid_snap.then_some(grid);
                                let hint = match self.editor.arc3_snap(sk, here, g, tol_px(mm_per_px)).1 {
                                    sketch::Arc3Snap::HalfCircle => Some(("½", a.center)),
                                    sketch::Arc3Snap::GridCenter(c) => Some(("grid", c)),
                                    sketch::Arc3Snap::None => None,
                                };
                                if let Some((label, c)) = hint {
                                    if let Some(px) = view.project(plane.to_world(c)) {
                                        painter.circle_stroke(to_pos(px), 5.0, Stroke::new(2.0, COL_SNAP));
                                        painter.text(
                                            to_pos(px) + egui::vec2(9.0, -9.0),
                                            egui::Align2::LEFT_BOTTOM,
                                            label,
                                            egui::FontId::proportional(13.0),
                                            COL_SNAP,
                                        );
                                    }
                                }
                                format!("R {:.2} mm   {:.1}°", a.radius, a.sweep.to_degrees())
                            }
                            None => {
                                // First two clicks: the chord (3-point) or radius (centre).
                                view.line(plane.to_world(from), plane.to_world(here), preview);
                                format!("{:.2} mm", from.distance(here))
                            }
                        },
                        Tool::Circle2Point | Tool::Circle3Point => match self.editor.circle_preview(sk, here) {
                            Some((c, r)) => {
                                view.circle(&plane, c, r, preview);
                                format!("Ø {:.2} mm", 2.0 * r)
                            }
                            None => {
                                // 3-point, first two clicks: just the chord so far.
                                view.line(plane.to_world(from), plane.to_world(here), preview);
                                String::new()
                            }
                        },
                        _ => {
                            // Show the line exactly where the click will put it,
                            // including the horizontal/vertical snap.
                            let (end, align) = self.editor.line_end(sk, at);
                            let to = sk.pos(end);
                            view.line(plane.to_world(from), plane.to_world(to), preview);
                            let tag = match align {
                                Some(sketch::Align::Horizontal) => "  H",
                                Some(sketch::Align::Vertical) => "  V",
                                None => "",
                            };
                            format!("{:.2} mm{tag}", from.distance(to))
                        }
                    };
                    if let Some(px) = response.hover_pos() {
                        painter.text(
                            px + egui::vec2(14.0, -14.0),
                            egui::Align2::LEFT_BOTTOM,
                            label,
                            egui::FontId::monospace(13.0),
                            COL_PREVIEW,
                        );
                    }
                }
            }
            // Tangent circles: once the lines are picked, show the circle.
            if self.editor.tool.tangent_lines() > 0 {
                let left = self.editor.tool.tangent_lines() - self.editor.tangent_lines.len();
                let hint = if left > 0 {
                    format!("pick {left} more line{}", if left == 1 { "" } else { "s" })
                } else {
                    let at = cursor.map(|a| sk.pos(a)).unwrap_or_default();
                    match self.editor.tangent_circle(sk, at) {
                        Ok((c, r)) => {
                            view.circle(&plane, c, r, Stroke::new(1.5, COL_PREVIEW));
                            format!("Ø {:.2} mm — click to place", 2.0 * r)
                        }
                        Err(e) => e,
                    }
                };
                if let Some(px) = response.hover_pos() {
                    painter.text(px + egui::vec2(14.0, -14.0), egui::Align2::LEFT_BOTTOM, hint, egui::FontId::monospace(13.0), COL_PREVIEW);
                }
            }
            status = format!(
                "{}  {}   grid {} mm{}",
                sk.plane_name(),
                status,
                (grid.step * 1000.0).round() / 1000.0,
                if self.grid_snap { " (snap)" } else { "" }
            );
        }

        // --- notification (bottom-left, fades after a few seconds) ---
        let mut expired = false;
        if let Some((text, since)) = &mut self.toast {
            if since.is_nan() {
                *since = now;
            }
            let age = now - *since;
            if age > 5.0 {
                expired = true;
            } else {
                let alpha = ((5.0 - age).min(1.0) * 235.0) as u8;
                let galley = painter.layout(text.clone(), egui::FontId::proportional(14.0), Color32::from_rgba_unmultiplied(255, 225, 200, alpha), 420.0);
                let at = rect.left_bottom() + egui::vec2(14.0, -40.0 - galley.size().y);
                let bg = egui::Rect::from_min_size(at, galley.size()).expand(8.0);
                painter.rect_filled(bg, 6.0, Color32::from_rgba_unmultiplied(70, 30, 30, alpha));
                painter.galley(at, galley, Color32::WHITE);
                ui.ctx().request_repaint();
            }
        }
        if expired {
            self.toast = None;
        }

        view.view_cube(&self.camera, &cells, to_vec(cube_center), CUBE_SCALE, cube_hover, self.cube_axes);

        // View cube controls, placed after everything else so they draw on top:
        // Home at the top-left, the view options menu at the bottom-right.
        let icon = Color32::from_gray(215);
        let home_rect = egui::Rect::from_center_size(cube_center + egui::vec2(-CUBE_MARGIN * 0.82, -CUBE_MARGIN * 0.82), egui::vec2(26.0, 24.0));
        let home = ui.put(home_rect, egui::Button::new("")).on_hover_text("Home view");
        View::house_icon(ui.painter(), home_rect.shrink2(egui::vec2(7.0, 6.0)), icon);
        if home.clicked() {
            self.move_camera(self.home, now);
        }
        let menu_rect = egui::Rect::from_center_size(cube_center + egui::vec2(CUBE_MARGIN * 0.82, CUBE_MARGIN * 0.82), egui::vec2(26.0, 24.0));
        let menu = ui.put(menu_rect, egui::Button::new("")).on_hover_text("View options");
        View::menu_icon(ui.painter(), menu_rect, icon);
        egui::Popup::menu(&menu).show(|ui| {
            if ui.button("Go Home").clicked() {
                self.move_camera(self.home, now);
                ui.close();
            }
            if ui.button("Set current view as Home").clicked() {
                self.home = self.camera.pose();
                ui.close();
            }
            if ui.button("Reset Home").clicked() {
                self.home = OrbitCamera::default().pose();
                ui.close();
            }
            ui.separator();
            ui.checkbox(&mut self.cube_axes, "Show XYZ axes on cube");
            ui.separator();
            ui.weak("Camera");
            for p in Projection::ALL {
                if ui.radio(self.camera.projection == p, p.name()).clicked() {
                    self.camera.projection = p;
                    ui.close();
                }
            }
        });

        painter.text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            format!(
                "{} {}  dist {:.0} mm   {}",
                if self.gpu { "gpu" } else { "cpu" },
                if self.camera.is_ortho() { "ortho" } else { "persp" },
                self.camera.distance,
                status
            ),
            egui::FontId::monospace(12.0),
            Color32::from_gray(160),
        );
    }
}

impl eframe::App for OpenCad {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Shortcuts are consumed first so the viewport never sees them as plain keys.
        let shortcut = ctx.input_mut(|i| {
            use egui::{Key, Modifiers};
            if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S) {
                Some(Cmd::SaveAs)
            } else if i.consume_key(Modifiers::COMMAND, Key::S) {
                Some(Cmd::Save)
            } else if i.consume_key(Modifiers::COMMAND, Key::O) {
                Some(Cmd::Open)
            } else if i.consume_key(Modifiers::COMMAND, Key::N) {
                Some(Cmd::New)
            } else {
                None
            }
        });
        if shortcut.is_some() && self.confirm.is_none() {
            self.cmd = shortcut;
        }
        // Undo / redo for the whole design (text boxes keep their own Ctrl+Z).
        if !ctx.text_edit_focused() && self.confirm.is_none() {
            let (redo, undo) = ctx.input_mut(|i| {
                use egui::{Key, Modifiers};
                let redo = i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
                (redo, i.consume_key(Modifiers::COMMAND, Key::Z))
            });
            if redo {
                self.redo();
            } else if undo {
                self.undo();
            }
        }

        egui::Panel::top("toolbar").show(ui, |ui| self.toolbar(ui));
        egui::Panel::left("browser").resizable(true).default_size(200.0).show(ui, |ui| self.browser(ui));
        if self.active.is_some() {
            egui::Panel::right("constraints").resizable(true).default_size(260.0).show(ui, |ui| self.constraints_panel(ui));
        } else if self.dialog.is_some() {
            egui::Panel::right("extrude").resizable(true).default_size(260.0).show(ui, |ui| self.extrude_panel(ui));
        } else if self.edge_dialog.is_some() {
            egui::Panel::right("edges").resizable(true).default_size(260.0).show(ui, |ui| self.edge_panel(ui));
        } else if self.revolve_dialog.is_some() {
            egui::Panel::right("revolve").resizable(true).default_size(260.0).show(ui, |ui| self.revolve_panel(ui));
        }
        egui::Panel::bottom("timeline").show(ui, |ui| self.timeline_bar(ui));
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui));
        self.float_editor(&ctx);

        if let Some(cmd) = self.cmd.take() {
            self.run_cmd(cmd, &ctx);
        }
        // Closing the window with unsaved work asks first.
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close && self.is_dirty() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm = Some(Pending::Quit);
        }
        self.modals(&ctx);
        self.record_history(&ctx);

        let title = format!("{}{} — opencad", self.file_name(), if self.is_dirty() { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

/// Snap distance for arc centres, in mm at the current zoom.
fn tol_px(mm_per_px: f64) -> f64 {
    SNAP_PX as f64 * mm_per_px
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("opencad")
            .with_app_id("opencad")
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "opencad",
        options,
        Box::new(|cc| {
            let mut app = OpenCad::default();
            // Set up the GPU renderer once and keep it with egui's renderer.
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                let g = gpu::Gpu::new(&rs.device, rs.target_format);
                rs.renderer.write().callback_resources.insert(g);
                app.gpu = true;
            }
            Ok(Box::new(app))
        }),
    )
}
