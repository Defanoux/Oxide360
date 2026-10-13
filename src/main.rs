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
mod colors;
mod copy_dialog;
mod doc;
mod export;
mod gpu;
mod handles;
mod icons;
mod inertia;
mod kernel;
mod app;
mod measure;
mod model;
mod offset;
mod origin;
mod plane_dialog;
mod pattern;
mod profile;
mod project;
mod settings;
mod sketch;
mod sketch_pattern;
mod solver;
mod view;

use camera::{
    Ray,
    FOV_Y, OrbitCamera, Pose, Projection, angles_for_dir, closest_param_on_line, cube_cells, cube_click_target,
    ndc_from_pixel,
};
use doc::{BodyId, Document, Feature, History, HomeView, ViewState};
use handles::{LineHandle, TurnHandle};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use eframe::egui;
use egui::{Color32, Stroke};
use glam::{DVec2, DVec3, Vec2, Vec3};
use copy_dialog::{CopyDialog, Mode as CopyMode};
use plane_dialog::PlaneDialog;
use sketch_pattern::{Mode as SpMode, SketchPattern};
use model::{PlaneFeature, AxisFeature, CopyFeature, EdgeFeature, EdgeKind, EdgeRef, Extrude, Mesh, Operation, Revolve, RevolveAxis};
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
const COL_PROJECTED: Color32 = Color32::from_rgb(190, 120, 255); // projected from a body
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
const SKETCH_TURN_SECONDS: f64 = 0.6; // slower, gentler glide when a sketch starts
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

/// A size box's text: "5", or "2, 5" with a second distance.
fn sizes_text(a: f64, b: Option<f64>) -> String {
    match b {
        Some(b) => format!("{a:.2}, {b:.2}"),
        None => format!("{a:.2}"),
    }
}

/// A typed number, with or without a unit ("12.7", "0.5 in", "90°"), in mm or degrees.
fn parse_mm(text: &str) -> Option<f64> {
    measure::parse_value(text)
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
        Constraint::Offset(o, e, d) => {
            // From the copy across to the original, square to both.
            let (from, to) = match (line(o), line(e)) {
                (Some((a, b)), Some((p, q))) => {
                    let mid = (p + q) * 0.5;
                    let u = (b - a).normalize_or_zero();
                    (a + u * (mid - a).dot(u), mid)
                }
                _ => {
                    let (centre, ang) = match *sk.entities.get(e)? {
                        Entity::Circle { center, .. } => (sk.points[center], std::f64::consts::FRAC_PI_4),
                        Entity::Arc { .. } => sk.arc_geometry(e).map(|(c, _, a0, sweep)| (c, a0 + sweep * 0.5))?,
                        _ => return None,
                    };
                    let dir = DVec2::from_angle(ang);
                    (centre + dir * sk.round_radius(o)?, centre + dir * sk.round_radius(e)?)
                }
            };
            view.linear_dimension(plane, from, to, away_from, shift, text(format!("{:.2}", d_of(d).abs())), color)
        }
        _ => None,
    }
}

/// The digits typed this frame, if it starts like a number.
fn typed_number(ui: &egui::Ui) -> Option<String> {
    let typed: String = ui.input(|inp| {
        inp.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    });
    typed.chars().next().filter(|ch| ch.is_ascii_digit() || *ch == '.').map(|_| typed)
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
    PolygonSides,    // number of sides, asked after the polygon's first click
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
    /// The user picked an operation themselves (else it follows what the new body touches).
    op_chosen: bool,
    /// The sketch a Cut was last pointed into the body for (so it's done once).
    cut_aimed_for: Option<usize>,
    editing: Option<usize>,
    sketch: Option<usize>,
    profiles: Vec<usize>,
    distance: f64,
    extent: model::Extent,
    distance2: f64,
    taper: f64,
    op: Operation,
}

/// A smooth camera move (view cube clicks, Home).
struct CameraTurn {
    from: Pose,
    to: Pose,
    start: f64,
    seconds: f64,
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

/// An axis a pattern can turn about.
#[derive(Clone, Copy, PartialEq)]
enum AxisChoice {
    World(model::WorldAxis),
    Custom(usize),
}

/// Where the bodies of a pattern are, for placing its arrows.
struct CopyGeo {
    centroid: DVec3,
    axis_point: DVec3,
    axis_dir: DVec3,
    /// The point of the axis nearest the bodies, and how far they are from it.
    pivot: DVec3,
    base_radius: f64,
    out_dir: Option<DVec3>,
    /// How far the bodies reach from their middle along `out_dir` (half their size that way).
    reach: f64,
}

/// A drag arrow on the canvas and the value it controls (see `handles`).
#[derive(Clone, Copy)]
enum Handle {
    Line { line: LineHandle, value: f64, target: HandleTarget },
    Turn { turn: TurnHandle, value: f64, target: HandleTarget },
}

impl Handle {
    fn value(&self) -> f64 {
        match *self {
            Handle::Line { value, .. } | Handle::Turn { value, .. } => value,
        }
    }

    fn target(&self) -> HandleTarget {
        match *self {
            Handle::Line { target, .. } | Handle::Turn { target, .. } => target,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum HandleTarget {
    Extrude,
    EdgeSize,
    RevolveAngle,
    Corner,
    /// Rectangular pattern spacing along direction 1 / 2.
    PatternSpacing1,
    PatternSpacing2,
    /// Circular pattern angle, and distance out from the axis.
    PatternAngle,
    PatternRadial,
    PlaneOffset,
    /// A sketch pattern's spacing along X / Y, and its angle.
    SketchSpacing1,
    SketchSpacing2,
    SketchAngle,
    SketchRadial,
}

struct EdgeDialog {
    editing: Option<usize>,
    kind: EdgeKind,
    size: f64,
    /// A chamfer's second distance (type `2, 5`).
    size2: Option<f64>,
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
    copies: Vec<CopyFeature>,
    axes: Vec<AxisFeature>,
    planes: Vec<PlaneFeature>,
    removes: Vec<model::RemoveFeature>,
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
    poly_asked: bool, // the sides box was shown for the polygon being drawn
    corner_first: Option<usize>, // chamfer: the line the first distance is for (when picked as two lines)
    feature_errors: Vec<Option<String>>, // per extrude, from the last committed build
    body_count: usize,
    prev_body_count: usize, // (to notice the first body being made)
    dialog_error: Option<String>,
    caches: [Option<KernelCache>; 3], // indexed by Slot
    edge_features: Vec<EdgeFeature>,
    edge_dialog: Option<EdgeDialog>,
    revolves: Vec<Revolve>,
    revolve_dialog: Option<RevolveDialog>,
    copies: Vec<CopyFeature>,                // mirrors and patterns
    copy_errors: Vec<Option<String>>,
    removes: Vec<model::RemoveFeature>,     // Remove features (bodies taken out)
    remove_errors: Vec<Option<String>>,
    selected_bodies: Vec<BodyId>,           // bodies selected in the browser
    copy_body_hover: Option<BodyId>,        // Mirror: the body under the cursor (a click adds / drops it)
    copy_dialog: Option<CopyDialog>,
    axes: Vec<AxisFeature>,                  // construction axes
    axis_errors: Vec<Option<String>>,
    axis_hidden: HashSet<usize>,             // construction axes switched off in the browser
    planes: Vec<PlaneFeature>,               // construction planes
    plane_errors: Vec<Option<String>>,
    plane_hidden: HashSet<usize>,            // construction planes switched off in the browser
    plane_dialog: Option<PlaneDialog>,
    plane_pick: bool,                        // Offset Plane: waiting for a click on the plane or face to measure from
    selected_cplane: Option<usize>,          // the construction plane clicked in the 3D view
    cplane_hover: Option<usize>,             // the construction plane under the mouse
    ctx_cplane: Option<usize>,               // the one a right-click was on
    axis_pick: bool,                         // waiting for a click on a straight edge to put an axis on
    revolve_errors: Vec<Option<String>>,
    sketch_pattern: Option<SketchPattern>,   // an open Rectangular / Circular Pattern in a sketch
    offset_preview: Option<Sketch>,          // the sketch with the offset the cursor would make
    cut_preview: Option<(Tool, DVec2, DVec2)>, // what Trim would cut / Extend would add, under the cursor
    sp_applied: Option<pattern::Layout>,     // the layout the sketch is showing now
    project_body_hover: Option<usize>,       // the body under the mouse when no edge is (Project tool: takes all its edges)
    project_hover: Option<EdgeRef>,          // the body edge under the mouse (Project tool)
    copy_axis_hover: Option<AxisChoice>, // the axis under the mouse while a pattern's axis is being picked
    axis_hover: Option<(usize, usize)>, // sketch, line entity under the mouse (revolve axis picking)
    edge_hover: Option<EdgeRef>,
    edge_feature_errors: Vec<Option<String>>,
    sketch_errors: Vec<Option<String>>,
    history: History, // undo / redo for the whole design
    hidden_bodies: HashSet<BodyId>,         // bodies switched off in the browser
    body_names: HashMap<BodyId, String>,    // names the user gave bodies
    body_colors: HashMap<BodyId, [u8; 3]>,  // colours the user gave bodies
    body_ids: Vec<BodyId>,                  // the bodies of the last build, in order
    measure: Option<measure::Measure>,      // the Measure tool while it's on
    measure_unit: measure::Unit,            // the Measure window's unit
    measure_unit2: Option<measure::Unit>,   // and the one shown under it, if any
    measure_precision: usize,               // decimal places in the Measure window
    measure_snap: bool,                     // Measure shows / snaps to snap points
    measure_xyz: bool,                      // Measure shows the X/Y/Z delta between corners
    sketch_cursor: Option<DVec2>,           // where the cursor last was on the sketch plane
    timeline_drag: Option<usize>,           // the timeline feature being dragged to a new place
    timeline_height: f32,                   // last frame's, so floating windows sit above it
    suppressed: Vec<Feature>,               // timeline features switched off
    renaming_feature: Option<(Feature, String)>, // a timeline feature being renamed, and the text so far
    renaming: Option<(BodyId, String)>,     // a body being renamed in the browser, and the text so far
    set_distance_text_pending: bool,        // the extrude distance changed outside its box
    marker: Option<usize>,                  // timeline rollback: features before this many are built (None = all)
    sketch_shown: HashMap<usize, bool>,     // per-sketch visibility the user chose
    label_drag: Option<(usize, DVec2, DVec2)>, // dimension being moved: constraint, press point, its shift then
    circle_tool: Tool,                         // which circle tool the toolbar's circle button is
    toast: Option<(String, f64)>,              // a notification in the viewport's corner, and when it appeared
    touchpad: bool,                            // touchpad controls instead of mouse
    label_carry: bool,                         // touchpad: a dimension label picked up, following the cursor
    dim_preview: Option<(Constraint, DVec2)>,  // Dimension tool: what a click would place, and its label shift
    face_pick_point: Option<DVec3>,          // where the cursor ray met the face being hovered while picking
    face_pick: bool,                         // waiting for a click on an origin plane or a flat face to sketch on
    mirror_objects: Vec<Sel>,                // what the Mirror tool will copy
    origin_shown: origin::Shown,             // the browser's Origin folder: what the 3D view shows
    origin_hover: Option<OriginPlane>,       // the origin plane under the mouse
    selected_plane: Option<OriginPlane>,     // the origin plane clicked in the 3D view
    ctx_plane: Option<OriginPlane>,          // the origin plane a right-click landed on
    ctx_face: Option<(usize, FacePlane)>,    // the body face a right-click landed on
    drag_line: Option<(usize, DVec2, DVec2, [DVec2; 2])>, // a line being slid sideways: entity, its normal, where the drag began, its ends then
    marquee: Option<(DVec2, DVec2)>,         // sketch drag-select: where it started and the cursor now
    face_hover: Option<(usize, FacePlane)>, // body index + the face plane under the mouse
    turn: Option<CameraTurn>,
    pending_turn: Option<Pose>, // a camera move to start next frame (when the time is known)
    toolbar_x: f32,             // the ribbon's scroll position
    toolbar_user_delta: f32,    // how far the user scrolled it last frame
    toolbar_inertia: inertia::Inertia,
    home: Pose,      // where Home goes (set it from the view cube menu)
    cube_axes: bool, // show the XYZ triad on the view cube
    settings_saved: settings::Settings, // what's on disk, to notice a change
    arrows: Vec<Option<(Vec2, Vec2)>>,  // the drag arrows on screen, last frame (one per handle)
    arrow_drag: Option<(usize, f64)>,   // which arrow is being dragged, and its grab offset
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
            poly_asked: false,
            corner_first: None,
            feature_errors: Vec::new(),
            body_count: 0,
            prev_body_count: 0,
            dialog_error: None,
            caches: [None, None, None],
            edge_features: Vec::new(),
            edge_dialog: None,
            revolves: Vec::new(),
            revolve_dialog: None,
            copies: Vec::new(),
            copy_errors: Vec::new(),
            removes: Vec::new(),
            remove_errors: Vec::new(),
            selected_bodies: Vec::new(),
            copy_body_hover: None,
            copy_dialog: None,
            axes: Vec::new(),
            axis_errors: Vec::new(),
            axis_hidden: HashSet::new(),
            planes: Vec::new(),
            plane_errors: Vec::new(),
            plane_hidden: HashSet::new(),
            plane_dialog: None,
            plane_pick: false,
            selected_cplane: None,
            cplane_hover: None,
            ctx_cplane: None,
            axis_pick: false,
            revolve_errors: Vec::new(),
            sketch_pattern: None,
            offset_preview: None,
            cut_preview: None,
            sp_applied: None,
            project_body_hover: None,
            project_hover: None,
            copy_axis_hover: None,
            axis_hover: None,
            edge_hover: None,
            edge_feature_errors: Vec::new(),
            sketch_errors: Vec::new(),
            history: History::default(),
            hidden_bodies: HashSet::new(),
            body_names: HashMap::new(),
            body_colors: HashMap::new(),
            body_ids: Vec::new(),
            measure: None,
            measure_unit: measure::Unit::Mm,
            measure_unit2: None,
            measure_precision: 3,
            measure_snap: true,
            measure_xyz: true,
            sketch_cursor: None,
            timeline_drag: None,
            timeline_height: 40.0,
            suppressed: Vec::new(),
            renaming_feature: None,
            renaming: None,
            marker: None,
            set_distance_text_pending: false,
            sketch_shown: HashMap::new(),
            label_drag: None,
            circle_tool: Tool::Circle,
            toast: None,
            touchpad: false,
            label_carry: false,
            dim_preview: None,
            face_pick_point: None,
            face_pick: false,
            mirror_objects: Vec::new(),
            origin_shown: origin::Shown::default(),
            origin_hover: None,
            selected_plane: None,
            ctx_plane: None,
            ctx_face: None,
            drag_line: None,
            marquee: None,
            face_hover: None,
            turn: None,
            pending_turn: None,
            toolbar_x: 0.0,
            toolbar_user_delta: 0.0,
            toolbar_inertia: inertia::Inertia::default(),
            home: OrbitCamera::default().pose(),
            cube_axes: true,
            settings_saved: settings::Settings::default(),
            arrows: Vec::new(),
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
        if !ctx.text_edit_focused() && self.confirm.is_none() && self.sketch_pattern.is_none() {
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

        // Side-scrollable, so a narrow window doesn't cut the ribbon off.
        egui::Panel::top("toolbar").show(ui, |ui| {
            // Carry-on scrolling: a flick keeps gliding a little (see `inertia`).
            let dt = ui.input(|i| i.stable_dt);
            let glide = self.toolbar_inertia.step(self.toolbar_user_delta, dt);
            let offset_in = (self.toolbar_x + glide.unwrap_or(0.0)).max(0.0);
            let out = egui::ScrollArea::horizontal()
                .auto_shrink([false, true])
                .horizontal_scroll_offset(offset_in)
                .show(ui, |ui| self.toolbar(ui));
            let offset_out = out.state.offset.x;
            self.toolbar_user_delta = offset_out - offset_in;
            if glide.is_some() && self.toolbar_user_delta != 0.0 && (offset_out <= 0.0 || offset_out >= out.content_size.x - out.inner_rect.width() - 0.5) {
                self.toolbar_inertia.stop(); // hit an end of the bar
                self.toolbar_user_delta = 0.0;
            }
            self.toolbar_x = offset_out;
            if self.toolbar_inertia.is_gliding() || self.toolbar_user_delta != 0.0 {
                ui.ctx().request_repaint();
            }
        });
        egui::Panel::left("browser").resizable(true).default_size(200.0).show(ui, |ui| self.browser(ui));
        // Tool dialogs float over the view's right side, like Fusion's.
        // Bottom right, above the timeline, so they cover no controls.
        let lift = self.timeline_height + 12.0;
        let float = |id: &'static str| {
            egui::Window::new(id)
                .title_bar(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -lift))
                .resizable(false)
                .collapsible(false)
                .default_width(240.0)
        };
        if self.sketch_pattern.is_some() && self.active.is_some() {
            float("sketch_pattern").show(&ctx, |ui| {
                let error = self.sketch_pattern.as_ref().and_then(|d| d.preview().err());
                if let Some(d) = self.sketch_pattern.as_mut() {
                    match d.panel(ui, error.as_deref()) {
                        copy_dialog::Action::Ok => self.commit_sketch_pattern(),
                        copy_dialog::Action::Cancel => self.cancel_sketch_pattern(),
                        copy_dialog::Action::None => {}
                    }
                }
            });
        } else if self.dialog.is_some() && self.active.is_none() {
            float("extrude").show(&ctx, |ui| self.extrude_panel(ui));
        } else if self.edge_dialog.is_some() && self.active.is_none() {
            float("edges").show(&ctx, |ui| self.edge_panel(ui));
        } else if self.copy_dialog.is_some() && self.active.is_none() {
            float("copy").show(&ctx, |ui| self.copy_panel(ui));
        } else if self.plane_dialog.is_some() && self.active.is_none() {
            float("plane").show(&ctx, |ui| {
                let error = self.dialog_error.clone();
                if let Some(d) = self.plane_dialog.as_mut() {
                    match d.panel(ui, error.as_deref()) {
                        copy_dialog::Action::Ok => self.commit_plane(),
                        copy_dialog::Action::Cancel => self.plane_dialog = None,
                        copy_dialog::Action::None => {}
                    }
                }
            });
        } else if self.revolve_dialog.is_some() && self.active.is_none() {
            float("revolve").show(&ctx, |ui| self.revolve_panel(ui));
        } else if self.measure.is_some() && self.active.is_none() {
            float("measure").show(&ctx, |ui| self.measure_panel(ui));
        }
        self.timeline_height = egui::Panel::bottom("timeline").show(ui, |ui| self.timeline_bar(ui)).response.rect.height();
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui));
        self.float_editor(&ctx);
        self.copy_quantity_boxes(&ctx);

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

        self.save_settings_if_changed();
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
            if let Some(path) = settings::default_path() {
                let s = settings::load(&path);
                app.touchpad = s.touchpad;
                app.cube_axes = s.cube_axes;
                app.camera.projection = s.projection;
                app.measure_unit = s.measure_unit;
                app.measure_unit2 = s.measure_unit2;
                app.measure_precision = s.measure_precision.min(6);
                app.measure_snap = s.measure_snap;
                app.measure_xyz = s.measure_xyz;
                app.settings_saved = s;
            }
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
