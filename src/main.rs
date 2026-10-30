//! Oxide360: a parametric CAD modeller in Rust.
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

mod appearance;
mod appearance_dialog;
mod camera;
mod colors;
mod control;
mod copy_dialog;
mod doc;
mod drawing;
mod drawing_export;
mod export;
mod gizmo;
mod gpu;
mod handles;
mod icons;
mod inertia;
mod kernel;
mod app;
mod assembly;
mod materials;
mod measure;
mod units;
mod keys;
mod spline;
mod model;
mod offset;
mod origin;
mod plane_dialog;
mod pattern;
mod profile;
mod help;
mod hole_dialog;
mod import_dialog;
mod joint_dialog;
mod shell_dialog;
mod shortcuts;
mod offset_dialog;
mod combine_dialog;
mod scale_dialog;
mod insert_drawing_dialog;
mod print3d;
mod print_check_dialog;
mod sketch_import;
mod sketch_palette;
mod split_dialog;
mod move_dialog;
mod expr;
mod spline_edit;
mod params_window;
mod thread_dialog;
mod rect_input;
mod project;
mod settings;
mod sketch;
mod sketch_export;
mod sketch_pattern;
mod solver;
mod threads;
mod text;
mod text_dialog;
mod section_dialog;
mod sweep_dialog;
mod recovery;
#[cfg(test)]
mod ui_shot;
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
use hole_dialog::HoleDialog;
use shell_dialog::ShellDialog;
use offset_dialog::OffsetDialog;
use appearance_dialog::AppearanceDialog;
use combine_dialog::CombineDialog;
use split_dialog::SplitDialog;
use scale_dialog::ScaleDialog;
use move_dialog::MoveDialog;
use thread_dialog::ThreadDialog;
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
/// The app's id: the Flatpak's name, and what KDE matches the window to its
/// menu entry and icon by.
const APP_ID: &str = "io.github.Defanoux.Oxide360";
const COL_SELECT: Color32 = Color32::from_rgb(255, 170, 40);
const COL_INTERFERENCE: Color32 = Color32::from_rgb(230, 60, 60); // where bodies overlap
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
impl OpenCad {
    /// The tool in use, for the top of the view (None when just looking).
    fn tool_name(&self) -> Option<String> {
        if self.measure.is_some() {
            return Some("Measure".into());
        }
        if self.active.is_some() {
            return Some(match self.editor.tool {
                Tool::Constrain(label) => label.to_string(),
                t => t.name().to_string(),
            });
        }
        Some(
            if self.dialog.is_some() {
                "Extrude"
            } else if self.revolve_dialog.is_some() {
                "Revolve"
            } else if let Some(d) = &self.edge_dialog {
                d.kind.name()
            } else if let Some(d) = &self.copy_dialog {
                d.mode.title()
            } else if self.shell_dialog.is_some() {
                "Shell"
            } else if self.combine_dialog.is_some() {
                "Combine"
            } else if self.split_dialog.is_some() {
                "Split Body"
            } else if self.scale_dialog.is_some() {
                "Scale"
            } else if self.appearance_dialog.is_some() {
                "Appearance"
            } else if self.move_dialog.is_some() {
                "Move/Copy"
            } else if let Some(d) = &self.offset_dialog {
                if d.press_pull && d.editing.is_none() { "Press Pull" } else { "Offset Face" }
            } else if self.thread_dialog.is_some() {
                "Thread"
            } else if self.hole_dialog.is_some() {
                "Hole"
            } else if self.plane_dialog.as_ref().is_some_and(|d| d.angle.is_some()) {
                "Plane at Angle"
            } else if self.angle_pick {
                "Plane at Angle — click a line, edge or axis"
            } else if self.plane_dialog.is_some() || self.plane_pick {
                "Offset Plane"
            } else if self.face_pick && self.redefine.is_some() {
                "Redefine Sketch Plane — click a plane or face"
            } else if self.face_pick {
                "Create Sketch — click a plane or face"
            } else if self.axis_pick {
                "Construction Axis — click a straight edge"
            } else {
                return None;
            }
            .to_string(),
        )
    }

    /// The unit sketch `i`'s dimension `ci` is shown and typed in: the one
    /// it was typed in, else the design's.
    fn dim_unit(&self, i: usize, ci: usize) -> units::Unit {
        self.sketches.get(i).and_then(|sk| sk.dim_unit(ci)).unwrap_or(self.units)
    }

    /// A dimension's value as typed in its box: lengths in its unit, angles
    /// in degrees.
    fn value_text(&self, i: usize, ci: usize) -> String {
        let Some(c) = self.sketches.get(i).and_then(|sk| sk.constraints.get(ci)) else { return String::new() };
        // Typed as an expression: edited as one ("width / 2"), not its number.
        if let Some(e) = self.sketches[i].dim_expr(ci) {
            return e.to_string();
        }
        match (c, c.value()) {
            (Constraint::Angle(..), Some(d)) => format!("{d:.2}"),
            (_, Some(d)) => self.dim_unit(i, ci).plain(d),
            (_, None) => String::new(),
        }
    }
}

/// A chamfer's size box text, in the design's units ("2.00" or "2.00, 5.00").
fn sizes_text(a: f64, b: Option<f64>, unit: units::Unit) -> String {
    match b {
        Some(b) => format!("{}, {}", unit.plain(a), unit.plain(b)),
        None => unit.plain(a),
    }
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
    unit: units::Unit,
    fx: bool,
) -> Option<egui::Rect> {
    let line = |e: usize| match sk.entities.get(e)? {
        Entity::Line { a, b } => Some((sk.points[*a], sk.points[*b])),
        _ => None,
    };
    // A driven dimension shows what the geometry measures now, in brackets.
    let d_of = |stored: f64| if driven { sk.measure(c).unwrap_or(stored) } else { stored };
    // Driven: in brackets; following a parameter expression: "fx: " first, as Fusion shows it.
    let text = |s: String| if driven { format!("({s})") } else if fx { format!("fx: {s}") } else { s };
    match *c {
        Constraint::Length(e, d) => {
            let (a, b) = line(e)?;
            view.linear_dimension(plane, a, b, away_from, shift, text(unit.show(d_of(d))), color)
        }
        Constraint::Distance(p, q, d) => view.linear_dimension(
            plane,
            *sk.points.get(p)?,
            *sk.points.get(q)?,
            away_from,
            shift,
            text(unit.show(d_of(d))),
            color,
        ),
        Constraint::Diameter(e, d) => match sk.entities.get(e)? {
            Entity::Circle { center, radius } => {
                view.diameter_dimension(plane, sk.points[*center], *radius, shift, text(format!("Ø{}", unit.show(d_of(d)))), color)
            }
            _ => None,
        },
        Constraint::Angle(e1, e2, d) => {
            view.angle_dimension(plane, line(e1)?, line(e2)?, shift, text(format!("{:.1}°", d_of(d))), color)
        }
        Constraint::Radius(e, d) => {
            let (c, r, a0, sweep) = sk.arc_geometry(e)?;
            view.radius_dimension(plane, c, r, a0 + sweep * 0.5, shift, text(format!("R{}", unit.show(d_of(d)))), color)
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
            view.linear_dimension(plane, from, to, away_from, shift, text(unit.show(d_of(d).abs())), color)
        }
        _ => None,
    }
}

/// The digits typed this frame, if it starts like a number.
/// The text typed this frame when it starts a number — or, with `names`
/// (the design has parameters), a name.
fn typed_number(ui: &egui::Ui, names: bool) -> Option<String> {
    let typed: String = ui.input(|inp| {
        inp.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect()
    });
    typed.chars().next().filter(|ch| ch.is_ascii_digit() || *ch == '.' || (names && (ch.is_alphabetic() || *ch == '_'))).map(|_| typed)
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

fn spline_icon(tool: Tool) -> Icon {
    match tool {
        Tool::ControlSpline => Icon::ControlSpline,
        _ => Icon::Spline,
    }
}

fn rgb(c: Color32) -> [f32; 3] {
    [c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0]
}

/// The Help window's pages.
#[derive(Clone, Copy, Debug, PartialEq)]
enum HelpPage {
    GettingStarted,
    About,
}

/// File commands, run after the frame's UI so dialogs never interrupt a widget.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    New,
    Open,
    Save,
    SaveAs,
    Export(export::Format),
    /// Insert a STEP / STL file's bodies into this design.
    Import,
    /// Insert an SVG / DXF drawing's curves into the open sketch.
    InsertDrawing,
    /// File → Recent: open the design listed `k`th.
    OpenRecent(usize),
}

/// What to do once the user has answered "save changes?".
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    Quit,
    /// Close this tab (asked by the tab strip's ×).
    CloseTab,
}

/// What the tab strip asks the tab holder to do.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TabRequest {
    New,
    /// Open the design in `OpenCad::open_in_tab` in a tab of its own (or
    /// this one, when it's an empty new design), as Fusion does.
    Open,
    Switch(usize),
    Close(usize),
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
    PlaneOffset,     // Offset Plane distance
    PlaneAngle,      // Plane at Angle, degrees
    OffsetDistance,  // Offset Face / Press Pull distance
}

/// What a click does while Offset Face / Press Pull picks.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PressPullHover {
    /// A flat face to move.
    Face(model::PickedFace),
    /// Press Pull: a fillet face — its Fillet feature opens.
    Fillet(usize),
    /// Press Pull: a sketch profile (sketch, region) — Extrude starts.
    Profile(usize, usize),
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
    /// To Object: what it runs up to, and whether clicks in the view pick it.
    target: Option<model::ExtrudeTarget>,
    picking_target: bool,
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
    PlaneAngle,
    HoleDepth,
    ShellThickness,
    OffsetDistance,
    /// Move/Copy: the X / Y / Z distance, the X / Y / Z angle; Rotate's angle.
    MoveX,
    MoveY,
    MoveZ,
    MoveRX,
    MoveRY,
    MoveRZ,
    MoveAngle,
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
    /// A chamfer by distance and angle: the angle in degrees.
    angle: Option<f64>,
    /// Picking an edge picks the edges running smoothly on from it.
    tangent_chain: bool,
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
impl KernelCache {
    /// Built from nothing (stands in until the first build is done).
    fn empty() -> Self {
        KernelCache {
            sketches: Vec::new(),
            extrudes: Vec::new(),
            revolves: Vec::new(),
            edge_features: Vec::new(),
            copies: Vec::new(),
            axes: Vec::new(),
            planes: Vec::new(),
            removes: Vec::new(),
            holes: Vec::new(),
            thread_features: Vec::new(),
            shells: Vec::new(),
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
            timeline: Vec::new(),
            model: kernel::Model::default(),
            stand_in: true,
        }
    }
}

struct KernelCache {
    sketches: Vec<Sketch>,
    extrudes: Vec<Extrude>,
    revolves: Vec<Revolve>,
    edge_features: Vec<EdgeFeature>,
    copies: Vec<CopyFeature>,
    axes: Vec<AxisFeature>,
    planes: Vec<PlaneFeature>,
    removes: Vec<model::RemoveFeature>,
    holes: Vec<model::HoleFeature>,
    thread_features: Vec<model::ThreadFeature>,
    shells: Vec<model::ShellFeature>,
    offset_faces: Vec<model::OffsetFaceFeature>,
    combines: Vec<model::CombineFeature>,
    splits: Vec<model::SplitFeature>,
    scales: Vec<model::ScaleFeature>,
    moves: Vec<model::MoveFeature>,
    points: Vec<model::ConstructionPoint>,
    sweeps: Vec<model::SweepFeature>,
    imports: Vec<model::ImportFeature>,
    joints: Vec<model::JointFeature>,
    components: Vec<assembly::Component>,
    timeline: Vec<Feature>,
    model: kernel::Model,
    /// Nothing built yet (an empty model while the first build runs).
    stand_in: bool,
}

struct OpenCad {
    camera: OrbitCamera,
    sketches: Vec<Sketch>,
    extrudes: Vec<Extrude>,
    timeline: Vec<Feature>,
    active: Option<usize>,
    editor: Editor,
    grid_snap: bool,
    sketch_options: sketch_palette::Options, // the Sketch Palette's show / slice switches
    insert_drawing: Option<insert_drawing_dialog::InsertDrawingDialog>, // Insert SVG / DXF, being placed
    print_check: bool,                      // the Print Check window is open
    print_found: print_check_dialog::Findings, // what it found (kept, the orientations are slow-ish)
    print_key: String,                      // what `print_found.best` was worked out for
    printer: String,                        // Print Check settings (kept between runs)
    printer_custom: [f64; 3],
    overhang_limit: f64,
    undo: Vec<Sketch>,
    dof: Option<usize>,
    message: Option<String>,
    drag: Option<PointId>,
    drag_offset: DVec2, // where the point is from the cursor while dragging it (text is grabbed anywhere)
    dialog: Option<ExtrudeDialog>,
    float: Option<FloatEdit>,
    float_anchor: Option<egui::Pos2>,
    dim_labels: Vec<(usize, egui::Rect, Vec<[Vec2; 2]>)>, // dimension, its label, and its lines on screen
    dim_selection: Vec<usize>,                                 // dimensions selected (Delete removes them)
    dim_hover: Option<usize>,
    rect_tool: Tool, // which rectangle tool the toolbar's rectangle button is
    arc_tool: Tool,  // which arc tool the toolbar's arc button is
    poly_tool: Tool, // which polygon tool the toolbar's polygon button is
    spline_tool: Tool, // which spline tool the toolbar's spline button is
    corner_size: f64, // last sketch fillet radius / chamfer distance
    poly_asked: bool, // the sides box was shown for the polygon being drawn
    corner_first: Option<usize>, // chamfer: the line the first distance is for (when picked as two lines)
    feature_errors: Vec<Option<String>>, // per extrude, from the last committed build
    body_count: usize,
    prev_body_count: usize, // (to notice the first body being made)
    dialog_error: Option<String>,
    caches: [Option<KernelCache>; 3], // indexed by Slot
    builds: [Option<kernel::Build>; 3], // a rebuild under way, run a little each frame (indexed by Slot)
    stepped: [u64; 3],                  // the frame each slot's build last ran in
    edge_features: Vec<EdgeFeature>,
    edge_dialog: Option<EdgeDialog>,
    revolves: Vec<Revolve>,
    revolve_dialog: Option<RevolveDialog>,
    copies: Vec<CopyFeature>,                // mirrors and patterns
    copy_errors: Vec<Option<String>>,
    removes: Vec<model::RemoveFeature>,     // Remove features (bodies taken out)
    holes: Vec<model::HoleFeature>,         // Hole features
    hole_dialog: Option<hole_dialog::HoleDialog>,
    frame_no: u64,                           // frames drawn (a build runs once a frame)
    hole_errors: Vec<Option<String>>,
    hole_hover: Option<(model::HolePlace, DVec3)>, // what a click would add, and where its centre is
    edge_group_hover: Vec<EdgeRef>,          // fillet/chamfer: the edges a click would pick (a tangent chain, or a face's edges)
    thread_features: Vec<model::ThreadFeature>, // Thread features
    shells: Vec<model::ShellFeature>,       // Shell features
    shell_dialog: Option<ShellDialog>,
    shell_errors: Vec<Option<String>>,
    shell_hover: Option<(Option<model::PickedFace>, Option<BodyId>)>, // a face or body a click would pick
    offset_faces: Vec<model::OffsetFaceFeature>, // Offset Face features
    offset_dialog: Option<OffsetDialog>,
    offset_face_errors: Vec<Option<String>>,
    offset_hover: Option<PressPullHover>,      // what a click would pick (Offset Face / Press Pull)
    press_pull_go: Option<PressPullHover>,     // a fillet / profile Press Pull hands on to (next frame)
    combines: Vec<model::CombineFeature>,      // Combine features
    combine_dialog: Option<CombineDialog>,
    appearance_dialog: Option<AppearanceDialog>, // Appearance (A), while open
    appearance_hover: Option<BodyId>,            // the body a click would give it to
    appearance_face_hover: Option<(usize, u32, glam::DVec3, glam::DVec3)>, // Faces: body index, face, point, normal
    combine_errors: Vec<Option<String>>,
    combine_hover: Option<BodyId>,             // the body a click would pick (Combine)
    splits: Vec<model::SplitFeature>,          // Split Body features
    split_dialog: Option<SplitDialog>,
    split_errors: Vec<Option<String>>,
    split_hover: Option<BodyId>,               // the body a click would pick (Split Body)
    scales: Vec<model::ScaleFeature>,          // Scale features
    scale_dialog: Option<ScaleDialog>,
    scale_errors: Vec<Option<String>>,
    scale_hover: Option<BodyId>,               // the body a click would pick (Scale)
    moves: Vec<model::MoveFeature>,            // Move/Copy features
    parameters: Vec<expr::UserParameter>,      // the design's user parameters
    params_window: params_window::ParamsWindow,
    params_seen: Option<Vec<expr::UserParameter>>, // what the dimensions last followed
    feature_exprs: Vec<doc::FeatureExpr>,      // feature values typed as expressions
    pending_exprs: Vec<(doc::Field, String)>,  // typed in the open dialog's box, kept at OK
    move_dialog: Option<MoveDialog>,
    move_errors: Vec<Option<String>>,
    points: Vec<model::ConstructionPoint>,  // construction points
    sweeps: Vec<model::SweepFeature>,       // sweeps and lofts
    sweep_errors: Vec<Option<String>>,
    imports: Vec<model::ImportFeature>,    // STEP / STL bodies brought in
    components: Vec<assembly::Component>,  // named groups of bodies (parts)
    joints: Vec<model::JointFeature>,      // joints moving components onto each other
    joint_errors: Vec<Option<String>>,
    joint_dialog: Option<joint_dialog::JointDialog>,
    joint_hover: Option<(model::JointSnap, DVec3, DVec3)>, // the snap under the mouse: point, Z
    snap_preview: Vec<DVec3>,              // the hovered face's snap points, shown as dots
    renaming_component: Option<(usize, String)>,
    import_errors: Vec<Option<String>>,
    sweep_dialog: Option<sweep_dialog::SweepDialog>, // the Sweep / Loft dialog
    import_dialog: Option<import_dialog::ImportDialog>, // placing an inserted STEP / STL
    path_hover: Option<(usize, Vec<usize>)>, // the sketch curve chain under the cursor while picking a sweep's path
    point_errors: Vec<Option<String>>,
    point_pick: bool,                       // Construction Point: click a corner or a circle
    move_hover: Option<BodyId>,                // the body a click would pick (Move/Copy)
    thread_dialog: Option<thread_dialog::ThreadDialog>,
    thread_errors: Vec<Option<String>>,
    thread_hover: Option<(kernel::Cylinder, DVec3)>, // the cylindrical face under the cursor, and where
    last_thread: Option<model::ThreadSpec>,  // Thread's "Remember Size"
    units: units::Unit,                     // the design's units (shown and typed in)
    touchpad_zoom_flip: bool,               // touchpad Ctrl+scroll: fingers up zoom in
    version: u32,                           // saves so far (shown by the name)
    settings_open: bool,                    // the Settings window is open
    tab_titles: Vec<(String, bool)>,        // every open design's title and unsaved mark (set by the tab holder)
    tab_index: usize,                       // which of them this is
    tab_request: Option<TabRequest>,        // what the tab strip asked for this frame
    open_in_tab: Option<PathBuf>,           // a design to open in a new tab (with TabRequest::Open)
    close_me: bool,                         // this tab is done (its × was confirmed)
    toolbar_hidden: Vec<String>,            // tools tucked into their group's ▾ list (settings)
    keymap: keys::Keymap,                   // tool keys (settings)
    toolbar_order: Vec<String>,             // tools' order within their groups, as dragged (settings)
    style: gpu::Style,                      // how bodies are drawn (view cube menu → Visual style; settings)
    keybinds_open: bool,                    // the Keybinds window is open
    capturing_key: Option<keys::Action>,    // waiting for the key to give this action
    browser_open: bool,                     // the browser is shown (else folded to a button)
    view_rect: egui::Rect,                  // the 3D view's area last frame (the browser floats in it)
    default_units: units::Unit,             // units new designs start in (settings)
    remove_errors: Vec<Option<String>>,
    selected_bodies: Vec<BodyId>,           // bodies selected in the browser
    solid_sel: Vec<measure::Picked>,        // edges and faces clicked in solid mode (their stats show bottom right)
    solid_hover: Option<measure::Picked>,   // the edge or face under the cursor there
    solid_box: Option<(Vec2, Vec2)>,        // Shift+drag selection box in solid mode (screen corners)
    construct_pick: Option<ConstructPick>,  // Midplane / three-point / tangent plane, axis through cylinder / two points: picking
    corner_hover: Option<(model::PointRef, DVec3)>, // the corner (or construction point) under the cursor while picking points
    cylinder_hover: Option<(kernel::Cylinder, DVec3)>, // the cylinder under the cursor while picking one
    copy_body_hover: Option<BodyId>,        // Mirror: the body under the cursor (a click adds / drops it)
    copy_dialog: Option<CopyDialog>,
    axes: Vec<AxisFeature>,                  // construction axes
    axis_errors: Vec<Option<String>>,
    axis_hidden: HashSet<usize>,             // construction axes switched off in the browser
    planes: Vec<PlaneFeature>,               // construction planes
    plane_errors: Vec<Option<String>>,
    plane_hidden: HashSet<usize>,            // construction planes switched off in the browser
    point_hidden: HashSet<usize>,            // construction points switched off in the browser
    plane_dialog: Option<PlaneDialog>,
    plane_pick: bool,                        // Offset Plane: waiting for a click on the plane or face to measure from
    selected_cplane: Option<usize>,          // the construction plane clicked in the 3D view
    cplane_hover: Option<usize>,             // the construction plane under the mouse
    ctx_cplane: Option<usize>,               // the one a right-click was on
    axis_pick: bool,                         // waiting for a click on a straight edge to put an axis on
    angle_pick: bool,                        // Plane at Angle: waiting for a click on a line, edge or axis
    hinge_hover: Option<(model::Hinge, DVec3, DVec3, DVec3)>, // the line under the cursor: what, its ends, the 0° normal
    revolve_errors: Vec<Option<String>>,
    sketch_pattern: Option<SketchPattern>,   // an open Rectangular / Circular Pattern in a sketch
    text_dialog: Option<text_dialog::TextDialog>, // the Text dialog, on a text in the open sketch
    sections: Vec<model::SectionAnalysis>,  // Section Analyses (browser → Analysis)
    section_dialog: Option<section_dialog::SectionDialog>, // the Section Analysis dialog
    section_error: Option<String>,          // why the section shown couldn't be cut
    section_on: bool,                       // a section is drawn this frame (the real faces aren't where they look)
    offset_preview: Option<Sketch>,          // the sketch with the offset the cursor would make
    cut_preview: Option<(Tool, sketch::TrimPiece)>, // what Trim would cut / Extend would add, under the cursor
    break_marks: Vec<DVec2>,                  // where Break would split what's under the cursor
    line_up: Option<(Option<DVec2>, Option<DVec2>, DVec2)>, // inference: lined up above/beside these, at this point
    spline_check: Option<egui::Rect>,         // the ✓ that finishes the spline being drawn (where it was drawn)
    handle_drag: Option<(sketch::PointId, f64, usize)>, // a spline tangent handle being dragged: fit point, which end (±1), spline
    line_arc: Option<sketch::PointId>,        // Line tool: dragging a tangent arc off this end (Fusion's)
    line_length_text: Option<String>,         // the line length as typed (an expression links its dimension)
    acquired: (Option<usize>, Vec<sketch::EntityId>), // line middles the cursor rested on (sketch, lines; newest last)
    mid_guide: Option<(DVec2, Vec<DVec2>)>,   // the cursor on the perpendicular(s) through those middles
    line_length: Option<f64>,               // Line tool: a typed length the next segment keeps (the click sets its angle)
    rect_input: Option<rect_input::RectInput>, // 2-point / centre rectangle: typed width and height
    rect_boxes: Option<([egui::Pos2; 2], [f64; 2])>, // where the width / height boxes go, and the sizes shown now
    rect_place: bool,                        // Enter in a size box: place the rectangle as a click would
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
    body_colors: HashMap<BodyId, [u8; 3]>,
    body_materials: HashMap<BodyId, String>, // physical materials the user gave bodies
    body_appearances: HashMap<BodyId, String>, // appearances the user gave bodies (names)
    face_appearances: Vec<appearance::FaceAppearance>, // faces with their own appearance
    drawing: Option<drawing::DrawingSettings>, // the 2-D drawing's settings, once changed
    drawing_open: bool,                         // in the DRAWING workspace
    drawing_cam: Option<(egui::Pos2, f32)>,     // the sheet's corner on screen, px per mm (None: fit)
    drawing_scale_shown: f64,                   // the scale Auto picked last frame
    properties_of: Option<BodyId>,           // the body whose Properties window is open
    interference: Option<Interference>,      // Inspect → Interference, while open  // colours the user gave bodies
    body_ids: Vec<BodyId>,                  // the bodies of the last build, in order
    measure: Option<measure::Measure>,      // the Measure tool while it's on
    measure_unit: Option<measure::Unit>,    // the Measure window's unit (None: the design's)
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
    redefine: Option<usize>,                 // …to move this sketch onto (Redefine Sketch Plane)
    mirror_objects: Vec<Sel>,                // what the Mirror tool will copy
    origin_shown: origin::Shown,             // the browser's Origin folder: what the 3D view shows
    origin_hover: Option<OriginPlane>,       // the origin plane under the mouse
    selected_plane: Option<OriginPlane>,     // the origin plane clicked in the 3D view
    ctx_plane: Option<OriginPlane>,          // the origin plane a right-click landed on
    ctx_face: Option<(usize, FacePlane)>,    // the body face a right-click landed on
    drag_line: Option<(usize, DVec2, [DVec2; 2])>, // a line being dragged: entity, where the drag began, its ends then
    marquee: Option<(DVec2, DVec2)>,         // sketch drag-select: where it started and the cursor now
    face_hover: Option<(usize, FacePlane)>, // body index + the face plane under the mouse
    turn: Option<CameraTurn>,
    pending_turn: Option<Pose>, // a camera move to start next frame (when the time is known)
    toolbar_x: f32,             // the ribbon's scroll position
    toolbar_user_delta: f32,    // how far the user scrolled it last frame
    toolbar_inertia: inertia::Inertia,
    home: Pose,      // where Home goes (set it from the view cube menu)
    named_views: Vec<(String, Pose)>, // views saved by name (browser → Named Views), with the design
    cube_axes: bool, // show the XYZ triad on the view cube
    settings_saved: settings::Settings, // what's on disk, to notice a change
    recent: Vec<PathBuf>,               // designs opened or saved lately (File → Recent)
    help_open: Option<HelpPage>,        // the Help window, on which page
    last_tool: Option<&'static str>,    // the last tool started (the right-click menu's Repeat)
    recent_tools: Vec<&'static str>,    // tools used lately (the S box's top row)
    marking: Option<egui::Pos2>,        // the right-click marking menu, open where clicked
    shortcut_box: Option<(egui::Pos2, String)>, // the S key's Design Shortcuts box: where, what's typed
    arrows: Vec<Option<(Vec2, Vec2)>>,  // the drag arrows on screen, last frame (one per handle)
    arrow_drag: Option<(usize, f64)>,   // which arrow is being dragged, and its grab offset
    arrow_turn: Option<(f64, f64)>,     // revolve arrow: cursor angle so far (unwrapped, last raw)
    path: Option<PathBuf>,              // where this design is saved
    name: Option<String>,               // a name given before the first save
    renaming_design: Option<String>,    // the tab's name being edited
    rename_select_all: bool,            // select the whole name when the box opens
    saved: Document,                    // what's on disk, to detect unsaved changes
    tab_id: u64,                        // names this tab's autosave (see `recovery`)
    last_autosave: f64,                 // when it was last autosaved (or found clean)
    autosaved: bool,                    // an autosave of it is on disk
    cmd: Option<Cmd>,
    confirm: Option<Pending>,
    alert: Option<String>,
    allow_close: bool,
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
            sketch_options: sketch_palette::Options::default(),
            insert_drawing: None,
            print_check: false,
            print_found: Default::default(),
            print_key: String::new(),
            printer: print3d::PRINTERS[0].name.to_string(),
            printer_custom: [200.0, 200.0, 200.0],
            overhang_limit: 45.0,
            undo: Vec::new(),
            dof: None,
            message: None,
            drag: None,
            drag_offset: DVec2::ZERO,
            dialog: None,
            float: None,
            float_anchor: None,
            dim_labels: Vec::new(),
            dim_selection: Vec::new(),
            dim_hover: None,
            rect_tool: Tool::Rectangle,
            arc_tool: Tool::Arc3Point,
            poly_tool: Tool::PolygonCircumscribed,
            spline_tool: Tool::Spline,
            corner_size: 2.0,
            poly_asked: false,
            corner_first: None,
            feature_errors: Vec::new(),
            body_count: 0,
            prev_body_count: 0,
            dialog_error: None,
            caches: [None, None, None],
            builds: [None, None, None],
            stepped: [0; 3],
            edge_features: Vec::new(),
            edge_dialog: None,
            revolves: Vec::new(),
            revolve_dialog: None,
            copies: Vec::new(),
            copy_errors: Vec::new(),
            removes: Vec::new(),
            holes: Vec::new(),
            hole_dialog: None,
            frame_no: 0,
            hole_errors: Vec::new(),
            hole_hover: None,
            edge_group_hover: Vec::new(),
            thread_features: Vec::new(),
            shells: Vec::new(),
            shell_dialog: None,
            shell_errors: Vec::new(),
            shell_hover: None,
            offset_faces: Vec::new(),
            offset_dialog: None,
            offset_face_errors: Vec::new(),
            offset_hover: None,
            press_pull_go: None,
            combines: Vec::new(),
            combine_dialog: None,
            appearance_dialog: None,
            appearance_hover: None,
            appearance_face_hover: None,
            combine_errors: Vec::new(),
            combine_hover: None,
            splits: Vec::new(),
            split_dialog: None,
            split_errors: Vec::new(),
            split_hover: None,
            scales: Vec::new(),
            scale_dialog: None,
            scale_errors: Vec::new(),
            scale_hover: None,
            moves: Vec::new(),
            parameters: Vec::new(),
            params_window: Default::default(),
            params_seen: None,
            feature_exprs: Vec::new(),
            pending_exprs: Vec::new(),
            move_dialog: None,
            move_errors: Vec::new(),
            points: Vec::new(),
            point_errors: Vec::new(),
            sweeps: Vec::new(),
            sweep_errors: Vec::new(),
            imports: Vec::new(),
            import_errors: Vec::new(),
            components: Vec::new(),
            renaming_component: None,
            joints: Vec::new(),
            joint_errors: Vec::new(),
            joint_dialog: None,
            joint_hover: None,
            snap_preview: Vec::new(),
            sweep_dialog: None,
            import_dialog: None,
            path_hover: None,
            point_pick: false,
            move_hover: None,
            thread_dialog: None,
            thread_errors: Vec::new(),
            thread_hover: None,
            last_thread: None,
            units: units::Unit::Mm,
            touchpad_zoom_flip: true,
            version: 0,
            settings_open: false,
            tab_titles: Vec::new(),
            tab_index: 0,
            tab_request: None,
            open_in_tab: None,
            name: None,
            renaming_design: None,
            rename_select_all: false,
            close_me: false,
            toolbar_hidden: settings::Settings::default().toolbar_hidden,
            keymap: keys::Keymap::default(),
            toolbar_order: Vec::new(),
            style: gpu::Style::default(),
            keybinds_open: false,
            capturing_key: None,
            browser_open: true,
            view_rect: egui::Rect::NOTHING,
            default_units: units::Unit::Mm,
            remove_errors: Vec::new(),
            selected_bodies: Vec::new(),
            solid_sel: Vec::new(),
            solid_hover: None,
            solid_box: None,
            construct_pick: None,
            corner_hover: None,
            cylinder_hover: None,
            copy_body_hover: None,
            copy_dialog: None,
            axes: Vec::new(),
            axis_errors: Vec::new(),
            axis_hidden: HashSet::new(),
            planes: Vec::new(),
            plane_errors: Vec::new(),
            plane_hidden: HashSet::new(),
            point_hidden: HashSet::new(),
            plane_dialog: None,
            plane_pick: false,
            selected_cplane: None,
            cplane_hover: None,
            ctx_cplane: None,
            axis_pick: false,
            angle_pick: false,
            hinge_hover: None,
            revolve_errors: Vec::new(),
            sketch_pattern: None,
            text_dialog: None,
            sections: Vec::new(),
            section_dialog: None,
            section_error: None,
            section_on: false,
            offset_preview: None,
            break_marks: Vec::new(),
            cut_preview: None,
            line_up: None,
            spline_check: None,
            handle_drag: None,
            line_arc: None,
            line_length_text: None,
            acquired: (None, Vec::new()),
            mid_guide: None,
            line_length: None,
            rect_input: None,
            rect_boxes: None,
            rect_place: false,
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
            body_materials: HashMap::new(),
            body_appearances: HashMap::new(),
            face_appearances: Vec::new(),
            drawing: None,
            drawing_open: false,
            drawing_cam: None,
            drawing_scale_shown: 1.0,
            properties_of: None,
            interference: None,
            body_ids: Vec::new(),
            measure: None,
            measure_unit: None,
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
            redefine: None,
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
            named_views: Vec::new(),
            cube_axes: true,
            settings_saved: settings::Settings::default(),
            recent: Vec::new(),
            help_open: None,
            last_tool: None,
            recent_tools: Vec::new(),
            marking: None,
            shortcut_box: None,
            arrows: Vec::new(),
            arrow_drag: None,
            arrow_turn: None,
            path: None,
            saved: Document::default(),
            tab_id: {
                static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            last_autosave: 0.0,
            autosaved: false,
            cmd: None,
            confirm: None,
            alert: None,
            allow_close: false,
            gpu: false,
        }
    }
}


thread_local! {
    /// OPENCAD_TIMING: where this frame's time went (section, when it started).
    static MARKS: std::cell::RefCell<Vec<(&'static str, std::time::Instant)>> = const { std::cell::RefCell::new(Vec::new()) };
    static TIMING: bool = std::env::var_os("OPENCAD_TIMING").is_some();
}

/// A section of the frame starts here (for OPENCAD_TIMING's slow-frame report).
fn mark(name: &'static str) {
    if TIMING.with(|t| *t) {
        MARKS.with(|m| m.borrow_mut().push((name, std::time::Instant::now())));
    }
}

/// Prints the frame's time when it drops, if OPENCAD_TIMING is set and it was
/// slow, with the sections that took over a millisecond.
struct SlowFrame(std::time::Instant);

impl Drop for SlowFrame {
    fn drop(&mut self) {
        let marks = MARKS.with(|m| std::mem::take(&mut *m.borrow_mut()));
        let ms = self.0.elapsed().as_secs_f64() * 1e3;
        if ms > 30.0 && TIMING.with(|t| *t) {
            let mut parts = Vec::new();
            let mut from = ("frame start", self.0);
            for &(name, at) in marks.iter().chain([("frame end", std::time::Instant::now())].iter()) {
                let d = (at - from.1).as_secs_f64() * 1e3;
                if d >= 1.0 {
                    parts.push(format!("{} {d:.0}", from.0));
                }
                from = (name, at);
            }
            eprintln!("slow frame: {ms:.0} ms  [{}]", parts.join(", "));
        }
    }
}

impl OpenCad {
    /// One frame of this design (the tab that's showing).
    fn tab_ui(&mut self, ui: &mut egui::Ui) {
        // OPENCAD_TIMING=1: report frames slow enough to feel (and the kernel prints its steps).
        let frame_start = std::time::Instant::now();
        let _slow = SlowFrame(frame_start);
        self.frame_no += 1;
        // Previews of modeled threads wait until a value is finished.
        let busy = ui.ctx().text_edit_focused() || ui.ctx().egui_is_using_pointer() || self.arrow_drag.is_some();
        if let Some(d) = self.hole_dialog.as_mut() {
            d.catch_up(busy);
        }
        if let Some(d) = self.thread_dialog.as_mut() {
            d.catch_up(busy);
        }
        let ctx = ui.ctx().clone();
        self.follow_parameters(&ctx);
        // Enter finishes the open tool window (OK) when nothing else takes it:
        // no text box focused and no number box open (that has its own Enter).
        if self.active.is_none() && self.float.is_none() && !ctx.text_edit_focused() && ctx.input(|i| i.key_pressed(egui::Key::Enter)) {
            if self.shell_dialog.is_some() {
                self.commit_shell();
            } else if self.offset_dialog.is_some() {
                self.commit_offset();
            } else if self.combine_dialog.is_some() {
                self.commit_combine();
            } else if self.split_dialog.is_some() {
                self.commit_split();
            } else if self.scale_dialog.is_some() {
                self.commit_scale();
            } else if self.move_dialog.is_some() {
                self.commit_move();
            } else if self.thread_dialog.is_some() {
                self.commit_thread();
            } else if self.hole_dialog.is_some() {
                self.commit_hole();
            } else if self.plane_dialog.is_some() {
                self.commit_plane();
            } else if self.copy_dialog.is_some() {
                self.commit_copy();
            } else if self.edge_dialog.is_some() {
                self.commit_edge_feature();
            } else if self.revolve_dialog.is_some() {
                self.commit_revolve();
            } else if self.dialog.is_some() {
                self.commit_extrude();
            }
        }
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

        egui::Panel::top("titlebar").show(ui, |ui| self.title_bar(ui));
        self.settings_window(&ctx);
        self.help_window(&ctx);
        // Side-scrollable, so a narrow window doesn't cut the ribbon off.
        egui::Panel::top("toolbar").show(ui, |ui| {
            if self.active.is_some() {
                // Only as wide as its label.
                let label_w = ui.painter().layout_no_wrap("FINISH SKETCH".into(), egui::FontId::proportional(11.0), Color32::WHITE).size().x;
                egui::Panel::right("finish_sketch").resizable(false).exact_size(label_w + 20.0).show(ui, |ui| self.finish_sketch_group(ui));
            }
            // Carry-on scrolling: a flick keeps gliding a little (see `inertia`).
            let dt = ui.input(|i| i.stable_dt);
            let glide = self.toolbar_inertia.step(self.toolbar_user_delta, dt);
            let offset_in = (self.toolbar_x + glide.unwrap_or(0.0)).max(0.0);
            // A solid scrollbar in its own strip under the group names (a
            // floating one sat over them and caught clicks meant for the ▾s).
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle::solid();
            let out = egui::ScrollArea::horizontal()
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
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

        self.properties_window(&ctx);
        // Tool dialogs float over the view's right side, like Fusion's.
        // Bottom right, above the timeline, so they cover no controls.
        let lift = self.timeline_height + 12.0;
        let design_units = self.units;
        let float = |id: &'static str| {
            egui::Window::new(id)
                .title_bar(false)
                .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-12.0, -lift))
                .resizable(false)
                .collapsible(false)
                .default_width(240.0)
        };
        if self.insert_drawing.is_some() && self.active.is_some() {
            float("insert_drawing").show(&ctx, |ui| {
                let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                let act = self.insert_drawing.as_mut().map_or(copy_dialog::Action::None, |d| d.panel(ui, design_units));
                match act {
                    copy_dialog::Action::Ok => self.commit_insert_drawing(),
                    copy_dialog::Action::Cancel => self.insert_drawing = None,
                    copy_dialog::Action::None if esc => self.insert_drawing = None,
                    copy_dialog::Action::None => {}
                }
            });
        } else if self.text_dialog.is_some() && self.active.is_some() {
            float("text").show(&ctx, |ui| {
                let act = self.text_dialog.as_mut().map_or(copy_dialog::Action::None, |d| d.panel(ui, design_units));
                self.text_dialog_step(act);
            });
        } else if self.sweep_dialog.is_some() && self.active.is_none() {
            float("sweep").show(&ctx, |ui| self.sweep_panel(ui));
        } else if self.section_dialog.is_some() && self.active.is_none() {
            float("section").show(&ctx, |ui| self.section_panel(ui));
        } else if self.joint_dialog.is_some() && self.active.is_none() {
            float("joint").show(&ctx, |ui| self.joint_panel(ui));
        } else if self.import_dialog.is_some() && self.active.is_none() {
            float("import").show(&ctx, |ui| self.import_panel(ui));
        } else if self.interference.is_some() && self.active.is_none() {
            float("interference").show(&ctx, |ui| self.interference_panel(ui));
        } else if self.sketch_pattern.is_some() && self.active.is_some() {
            float("sketch_pattern").show(&ctx, |ui| {
                let error = self.sketch_pattern.as_ref().and_then(|d| d.preview().err());
                if let Some(d) = self.sketch_pattern.as_mut() {
                    match d.panel(ui, design_units, error.as_deref()) {
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
        } else if self.shell_dialog.is_some() && self.active.is_none() {
            float("shell").show(&ctx, |ui| {
                let error = self.dialog_error.clone();
                if let Some(d) = self.shell_dialog.as_mut() {
                    match d.panel(ui, design_units, error.as_deref()) {
                        copy_dialog::Action::Ok => self.commit_shell(),
                        copy_dialog::Action::Cancel => self.cancel_shell(),
                        copy_dialog::Action::None => {}
                    }
                }
            });
        } else if self.move_dialog.is_some() && self.active.is_none() {
            float("move").show(&ctx, |ui| self.move_panel(ui));
        } else if self.combine_dialog.is_some() && self.active.is_none() {
            float("combine").show(&ctx, |ui| self.combine_panel(ui));
        } else if self.split_dialog.is_some() && self.active.is_none() {
            float("split").show(&ctx, |ui| self.split_panel(ui));
        } else if self.scale_dialog.is_some() && self.active.is_none() {
            float("scale").show(&ctx, |ui| self.scale_panel(ui));
        } else if self.appearance_dialog.is_some() && self.active.is_none() {
            float("appearance").show(&ctx, |ui| self.appearance_panel(ui));
        } else if self.offset_dialog.is_some() && self.active.is_none() {
            float("offset").show(&ctx, |ui| {
                let error = self.dialog_error.clone();
                if let Some(d) = self.offset_dialog.as_mut() {
                    let before = d.offset.distance;
                    let action = d.panel(ui, design_units, error.as_deref());
                    if d.offset.distance != before {
                        self.set_distance_text();
                    }
                    match action {
                        copy_dialog::Action::Ok => self.commit_offset(),
                        copy_dialog::Action::Cancel => self.cancel_offset(),
                        copy_dialog::Action::None => {}
                    }
                }
            });
        } else if self.thread_dialog.is_some() && self.active.is_none() {
            float("thread").show(&ctx, |ui| {
                let error = self.dialog_error.clone();
                if let Some(d) = self.thread_dialog.as_mut() {
                    match d.panel(ui, design_units, error.as_deref()) {
                        copy_dialog::Action::Ok => self.commit_thread(),
                        copy_dialog::Action::Cancel => self.cancel_thread(),
                        copy_dialog::Action::None => {}
                    }
                }
            });
        } else if self.hole_dialog.is_some() && self.active.is_none() {
            float("hole").show(&ctx, |ui| {
                let error = self.dialog_error.clone();
                let plane_names: Vec<String> = self.planes.iter().map(|p| p.name.clone()).collect();
                if let Some(d) = self.hole_dialog.as_mut() {
                    let circles = d.hole.places.iter().filter(|p| matches!(p, model::HolePlace::SketchCircle { .. })).count();
                    match d.panel(ui, design_units, error.as_deref(), &plane_names, circles) {
                        copy_dialog::Action::Ok => self.commit_hole(),
                        copy_dialog::Action::Cancel => self.cancel_hole(),
                        copy_dialog::Action::None => {}
                    }
                }
            });
        } else if self.plane_dialog.is_some() && self.active.is_none() {
            float("plane").show(&ctx, |ui| {
                let error = self.dialog_error.clone();
                if let Some(d) = self.plane_dialog.as_mut() {
                    let before = (d.distance, d.angle.map(|a| a.degrees));
                    let action = d.panel(ui, design_units, error.as_deref());
                    if (d.distance, d.angle.map(|a| a.degrees)) != before {
                        self.set_distance_text();
                    }
                    match action {
                        copy_dialog::Action::Ok => self.commit_plane(),
                        copy_dialog::Action::Cancel => self.close_plane_dialog(),
                        copy_dialog::Action::None => {}
                    }
                }
            });
        } else if self.revolve_dialog.is_some() && self.active.is_none() {
            float("revolve").show(&ctx, |ui| self.revolve_panel(ui));
        } else if self.measure.is_some() {
            float("measure").show(&ctx, |ui| self.measure_panel(ui));
        } else if self.print_check && self.active.is_none() {
            float("print_check").show(&ctx, |ui| {
                let found = self.print_found.clone();
                match print_check_dialog::panel(ui, &mut self.printer, &mut self.printer_custom, &mut self.overhang_limit, &found) {
                    print_check_dialog::Action::Close => self.print_check = false,
                    print_check_dialog::Action::LayFlat(k) => self.lay_flat(k),
                    print_check_dialog::Action::None => {}
                }
            });
        }
        // The Sketch Palette: the view's right, under the view cube.
        // (Out of the way while a sketch dialog is open there.)
        let sketch_dialog = self.insert_drawing.is_some() || self.text_dialog.is_some() || self.sketch_pattern.is_some();
        if self.active.is_some() && !sketch_dialog && self.view_rect.width() > 0.0 {
            let at = self.view_rect.right_top() + egui::vec2(-12.0, CUBE_TOP + CUBE_MARGIN);
            let construction = self.editor.construction;
            let mut action = sketch_palette::Action::None;
            egui::Window::new("SKETCH PALETTE").id(egui::Id::new("sketch_palette")).default_width(60.0).pivot(egui::Align2::RIGHT_TOP).fixed_pos(at).resizable(false).collapsible(true).show(&ctx, |ui| {
                action = sketch_palette::panel(ui, &mut self.sketch_options, &mut self.grid_snap, construction);
            });
            match action {
                sketch_palette::Action::Construction => self.toggle_construction(),
                sketch_palette::Action::LookAt => {
                    if let Some(i) = self.active {
                        self.look_at(self.sketches[i].view_angles(), None);
                    }
                }
                sketch_palette::Action::Finish => {
                    self.finish_sketch();
                }
                sketch_palette::Action::None => {}
            }
        }
        mark("timeline bar");
        self.timeline_height = egui::Panel::bottom("timeline").show(ui, |ui| self.timeline_bar(ui)).response.rect.height();
        mark("view");
        egui::CentralPanel::no_frame().show(ui, |ui| self.viewport(ui));
        self.marking_menu(&ctx);
        self.shortcut_box(&ctx);
        mark("after the view");
        self.float_editor(&ctx);
        self.rect_size_boxes(&ctx);
        // A rebuild under way: the busy cursor and a progress bar, and keep drawing.
        if self.builds.iter().any(Option::is_some) {
            ctx.set_cursor_icon(egui::CursorIcon::Progress);
            ctx.request_repaint();
            self.build_progress(&ctx);
        }
        self.copy_quantity_boxes(&ctx);

        if let Some(cmd) = self.cmd.take() {
            self.run_cmd(cmd);
        }
        self.modals(&ctx);
        self.record_history(&ctx);
        self.save_settings_if_changed();
    }

    /// Take on the global settings (at start, and when this tab comes up).
    fn apply_settings(&mut self, s: &settings::Settings) {
        self.touchpad = s.touchpad;
        self.cube_axes = s.cube_axes;
        self.camera.projection = s.projection;
        self.measure_unit = s.measure_unit_own;
        self.measure_unit2 = s.measure_unit2;
        self.measure_precision = s.measure_precision.min(8);
        self.measure_snap = s.measure_snap;
        self.measure_xyz = s.measure_xyz;
        self.default_units = s.default_units;
        self.touchpad_zoom_flip = s.touchpad_zoom_flip;
        self.toolbar_hidden = s.toolbar_hidden.clone();
        self.keymap = keys::Keymap::from_pairs(&s.keybinds);
        self.toolbar_order = s.toolbar_order.clone();
        self.style = s.style;
        self.recent = s.recent.clone();
        self.printer = s.printer.clone();
        self.printer_custom = s.printer_custom;
        self.overhang_limit = if s.overhang_limit.is_finite() { s.overhang_limit.clamp(10.0, 85.0) } else { 45.0 };
        self.settings_saved = s.clone();
    }

    /// A fresh design in a new tab: the settings, starting in the default units.
    fn fresh(s: &settings::Settings, gpu: bool) -> Self {
        let mut app = OpenCad::default();
        app.apply_settings(s);
        // (Starting in the default units isn't a change to the design.)
        app.units = s.default_units;
        app.saved.units = s.default_units;
        app.gpu = gpu;
        app
    }
}

/// The construction tools that are picks only (no dialog): what's been picked so far.
#[derive(Clone, Debug, PartialEq)]
enum ConstructPick {
    Midplane(Vec<model::ExtrudeTarget>),
    /// The points picked so far, and where they are.
    ThreePoints(Vec<(model::PointRef, DVec3)>),
    Tangent,
    AxisCylinder,
    AxisTwoPoints(Vec<(model::PointRef, DVec3)>),
}

impl ConstructPick {
    /// What to click next, for the top of the view.
    fn hint(&self) -> String {
        match self {
            ConstructPick::Midplane(t) => format!("Midplane {} of 2: click a plane or flat face", t.len() + 1),
            ConstructPick::ThreePoints(p) => format!("Plane Through Three Points {} of 3: click a body corner or construction point", p.len() + 1),
            ConstructPick::Tangent => "Tangent Plane: click a cylinder where the plane should touch it".into(),
            ConstructPick::AxisCylinder => "Axis Through Cylinder: click a hole or shaft".into(),
            ConstructPick::AxisTwoPoints(p) => format!("Axis Through Two Points {} of 2: click a body corner or construction point", p.len() + 1),
        }
    }
}

/// Inspect → Interference: the bodies that overlap and where, for the
/// bodies as they were when it was worked out (`shapes`).
struct Interference {
    shapes: Vec<opencascade::primitives::Shape>,
    hits: Vec<(BodyId, BodyId, f64, model::Mesh)>,
}

/// The open designs, one per tab (like Fusion's document tabs).
struct Tabs {
    tabs: Vec<OpenCad>,
    current: usize,
    gpu: bool,
    title: String,
    /// Driven from outside (`--control PORT`): see `control`.
    control: Option<Control>,
    /// Autosaves left by a run that didn't close properly: offered at start.
    recovered: Vec<recovery::Recovered>,
}

/// The control link's side in the app: requests come in one at a time;
/// input ones are fed in a frame per step, and each is answered once the
/// view has settled (and any rebuild finished).
struct Control {
    rx: std::sync::mpsc::Receiver<control::Request>,
    busy: Option<control::Request>,
    queue: std::collections::VecDeque<control::Step>,
    /// Frames still to draw before answering.
    settle: u32,
    /// When the request being worked on came in (to give up on one stuck).
    since: f64,
    asked_shot: bool,
    shot: Option<std::sync::Arc<egui::ColorImage>>,
}

impl eframe::App for Tabs {
    /// The control link's made-up input goes in here, a step per frame, as
    /// if from the mouse and keyboard; a screenshot it asked for comes back here.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        let Some(c) = self.control.as_mut() else { return };
        for e in &raw.events {
            if let egui::Event::Screenshot { image, .. } = e {
                if std::mem::take(&mut c.asked_shot) {
                    c.shot = Some(image.clone());
                }
            }
        }
        if let Some(step) = c.queue.pop_front() {
            raw.events.push(egui::Event::ModifiersChanged(step.mods));
            raw.events.extend(step.events);
            raw.focused = true;
            // Keys held for it are let go afterwards.
            if c.queue.is_empty() && !step.mods.is_none() {
                c.queue.push_back(control::Step { events: Vec::new(), mods: egui::Modifiers::NONE });
            }
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let titles: Vec<(String, bool)> = self.tabs.iter().map(|t| (t.tab_title(), t.is_dirty())).collect();
        let current = self.current;
        let tab = &mut self.tabs[current];
        (tab.tab_titles, tab.tab_index) = (titles, current);
        tab.tab_ui(ui);
        let settings = tab.current_settings();
        let close_me = std::mem::take(&mut tab.close_me);
        match tab.tab_request.take() {
            Some(TabRequest::New) => {
                self.tabs.push(OpenCad::fresh(&settings, self.gpu));
                self.current = self.tabs.len() - 1;
            }
            Some(TabRequest::Open) => {
                if let Some(path) = tab.open_in_tab.take() {
                    // An untouched new design gives its tab up; otherwise a tab of its own.
                    let here = tab.path.is_none() && tab.timeline.is_empty() && !tab.is_dirty();
                    if !here {
                        self.tabs.push(OpenCad::fresh(&settings, self.gpu));
                        self.current = self.tabs.len() - 1;
                    }
                    let t = &mut self.tabs[self.current];
                    t.open_path(path);
                    // Couldn't be read: the new tab goes again (the alert stays in the old one).
                    if t.path.is_none() && !here {
                        let alert = t.alert.take();
                        self.tabs.pop();
                        self.current = current;
                        self.tabs[current].alert = alert;
                    }
                }
            }
            Some(TabRequest::Switch(k)) if k < self.tabs.len() => self.show(k, &settings),
            // Closing a tab with unsaved work asks first (in that tab).
            Some(TabRequest::Close(k)) if k < self.tabs.len() => {
                if self.tabs[k].is_dirty() {
                    self.show(k, &settings);
                    self.tabs[k].confirm = Some(Pending::CloseTab);
                } else {
                    self.close(k);
                }
            }
            _ => {}
        }
        if close_me {
            self.close(self.current.min(self.tabs.len() - 1));
        }
        // Closing the window: each tab with unsaved work asks in turn.
        if ctx.input(|i| i.viewport().close_requested()) {
            if let Some(k) = self.tabs.iter().position(|t| t.is_dirty() && !t.allow_close) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                let settings = self.tabs[self.current].current_settings();
                self.show(k, &settings);
                self.tabs[k].confirm = Some(Pending::Quit);
            }
        }
        let t = &self.tabs[self.current];
        let title = format!("{}{} — Oxide360", t.tab_title(), if t.is_dirty() { "*" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        self.control_frame(&ctx);
        // Autosave every design with changes now and then (see `recovery`).
        let now = ctx.input(|i| i.time);
        for t in &mut self.tabs {
            t.autosave(now);
        }
        self.recovery_prompt(&ctx);
    }

    /// A normal close: this run's autosaves aren't needed.
    fn on_exit(&mut self) {
        if let Some(dir) = recovery::default_dir() {
            for t in &self.tabs {
                recovery::remove(&dir, std::process::id(), t.tab_id);
            }
        }
    }
}

impl Tabs {
    /// After a run that didn't close properly: offer its autosaves back.
    fn recovery_prompt(&mut self, ctx: &egui::Context) {
        if self.recovered.is_empty() {
            return;
        }
        let mut choice: Option<bool> = None;
        egui::Window::new("Recover designs?").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)).show(ctx, |ui| {
            ui.label("Oxide360 didn't close properly last time. These designs had unsaved changes:");
            for r in &self.recovered {
                let from = r.original.as_ref().map_or_else(|| "never saved".to_string(), |p| p.display().to_string());
                ui.label(format!("  {}   ({from})", r.title));
            }
            ui.horizontal(|ui| {
                if ui.button("Recover").clicked() {
                    choice = Some(true);
                }
                if ui.button("Discard").clicked() {
                    choice = Some(false);
                }
            });
        });
        let Some(recover) = choice else { return };
        let settings = self.tabs[self.current].current_settings();
        for r in std::mem::take(&mut self.recovered) {
            if recover {
                // Into the empty first tab, else a new one.
                let blank = self.tabs.len() == 1 && !self.tabs[0].is_dirty() && self.tabs[0].path.is_none();
                if !blank {
                    self.tabs.push(OpenCad::fresh(&settings, self.gpu));
                }
                let k = self.tabs.len() - 1;
                if self.tabs[k].open_recovered(&r).is_err() {
                    if !blank {
                        self.tabs.pop();
                    }
                    continue; // (unreadable: dropped)
                }
                self.current = k;
            }
            recovery::forget(&r);
        }
    }

    /// The control link, once a frame: take the next request, and answer the
    /// one in hand when it's done.
    fn control_frame(&mut self, ctx: &egui::Context) {
        use serde_json::{json, Value};
        let Tabs { control, tabs, current, .. } = self;
        let Some(c) = control.as_mut() else { return };
        let tab = &mut tabs[*current];
        let now = ctx.input(|i| i.time);
        let ppp = ctx.pixels_per_point();
        if c.busy.is_none() {
            if let Ok((req, reply)) = c.rx.try_recv() {
                let path = || req["path"].as_str().map(str::to_string).ok_or_else(|| "needs \"path\"".to_string());
                let done = |r: Result<(), String>| r.map_or_else(|e| json!({"error": e}), |_| json!({"ok": true}));
                let now_answer: Option<Value> = match req["cmd"].as_str().unwrap_or("") {
                    "state" => Some(tab.control_state()),
                    "open" => Some(done(path().and_then(|p| tab.control_open(&p)))),
                    "save" => Some(done(path().and_then(|p| tab.control_save(&p)))),
                    "import" => Some(done(path().and_then(|p| {
                        let p = std::path::Path::new(&p);
                        // A drawing goes into the open sketch; a part into the design.
                        match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
                            Some("svg" | "dxf") => tab.insert_drawing_path(p),
                            _ => tab.import_path(p),
                        }
                    }))),
                    "export_drawing" => Some(done(path().and_then(|p| tab.export_drawing_to(std::path::Path::new(&p))))),
                    "new" => {
                        tab.control_new();
                        Some(json!({"ok": true}))
                    }
                    "screenshot" => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                        c.asked_shot = true;
                        c.shot = None;
                        None
                    }
                    "wait" => None,
                    _ => match control::steps(&req, 1.0 / ppp) {
                        Ok(s) => {
                            c.queue.extend(s);
                            None
                        }
                        Err(e) => Some(json!({"error": e})),
                    },
                };
                match now_answer {
                    Some(a) => {
                        let _ = reply.send(a);
                    }
                    None => {
                        c.settle = if req["cmd"] == "wait" { req["frames"].as_u64().unwrap_or(30).min(6000) as u32 } else { 3 };
                        c.since = now;
                        c.busy = Some((req, reply));
                    }
                }
            }
        }
        let Some((req, _)) = &c.busy else { return };
        ctx.request_repaint();
        let waited = now - c.since;
        let answer = if req["cmd"] == "screenshot" {
            match c.shot.take() {
                Some(img) => {
                    let rgba: Vec<u8> = img.pixels.iter().flat_map(|p| p.to_array()).collect();
                    let bytes = control::png(img.size[0], img.size[1], &rgba);
                    let (w, h) = (img.size[0], img.size[1]);
                    Some(match req["path"].as_str() {
                        Some(p) => match std::fs::write(p, &bytes) {
                            Ok(()) => json!({"path": p, "width": w, "height": h, "pixels_per_point": ppp}),
                            Err(e) => json!({"error": format!("couldn't write {p}: {e}")}),
                        },
                        None => json!({"png_base64": control::base64(&bytes), "width": w, "height": h, "pixels_per_point": ppp}),
                    })
                }
                None if waited > 10.0 => Some(json!({"error": "no screenshot came back (is the window minimised?)"})),
                None => None,
            }
        } else if !c.queue.is_empty() {
            None
        } else if c.settle > 0 || waited < 0.2 {
            // (A few frames, and long enough for egui's little animations to end.)
            c.settle = c.settle.saturating_sub(1);
            None
        } else if tab.control_busy() && waited < 30.0 {
            None // (a rebuild still running)
        } else {
            Some(json!({"ok": true, "notice": tab.toast.as_ref().map(|t| t.0.clone()), "tool": tab.tool_name(), "dialogs_open": tab.control_state()["dialogs"].clone()}))
        };
        if let Some(a) = answer {
            if let Some((_, reply)) = c.busy.take() {
                let _ = reply.send(a);
            }
        }
    }

    /// Bring tab `k` up, carrying the global settings over to it.
    fn show(&mut self, k: usize, settings: &settings::Settings) {
        self.current = k;
        self.tabs[k].apply_settings(settings);
    }

    /// Close tab `k` (an empty window gets a fresh design).
    fn close(&mut self, k: usize) {
        let settings = self.tabs[self.current].current_settings();
        if let Some(dir) = recovery::default_dir() {
            recovery::remove(&dir, std::process::id(), self.tabs[k].tab_id);
        }
        self.tabs.remove(k);
        if self.tabs.is_empty() {
            self.tabs.push(OpenCad::fresh(&settings, self.gpu));
        }
        if self.current >= k && self.current > 0 {
            self.current -= 1;
        }
        self.current = self.current.min(self.tabs.len() - 1);
        let k = self.current;
        self.show(k, &settings);
    }
}

/// Snap distance for arc centres, in mm at the current zoom.
fn tol_px(mm_per_px: f64) -> f64 {
    SNAP_PX as f64 * mm_per_px
}

/// Entry point for the Oxide360 application.
/// Handles command-line arguments, sets up the GUI, and initializes the application state.
fn main() -> eframe::Result {
    // `oxide360 ctl [--port N] '<json>'`: one request to a running app.
    // `oxide360 mcp [--port N]`: the MCP server (for Claude Code).
    // `oxide360 --control [N]`: the app, listening for those.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let port_after = |flag: &str| args.iter().position(|a| a == flag).and_then(|k| args.get(k + 1)).and_then(|p| p.parse().ok());
    match args.first().map(String::as_str) {
        Some("ctl") => {
            let port = port_after("--port").unwrap_or(control::DEFAULT_PORT);
            let Some(req) = args.iter().skip(1).filter(|a| a.starts_with('{')).last() else {
                eprintln!("usage: oxide360 ctl [--port N] '{{\"cmd\": \"state\"}}'");
                std::process::exit(2);
            };
            let req: serde_json::Value = serde_json::from_str(req).unwrap_or_else(|e| {
                eprintln!("not JSON: {e}");
                std::process::exit(2)
            });
            match control::call(port, &req) {
                Ok(a) => println!("{}", serde_json::to_string_pretty(&a).unwrap_or_default()),
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
        Some("mcp") => {
            control::mcp(port_after("--port").unwrap_or(control::DEFAULT_PORT));
            return Ok(());
        }
        _ => {}
    }
    let control_port: Option<u16> = args.iter().position(|a| a == "--control").map(|k| args.get(k + 1).and_then(|p| p.parse().ok()).unwrap_or(control::DEFAULT_PORT));
    // `oxide360 part.ox360`: open that design (a double-clicked file, from the app menu entry).
    let open_at_start: Option<PathBuf> = args.iter().find(|a| !a.starts_with('-') && a.parse::<u16>().is_err()).map(PathBuf::from).filter(|p| p.is_file());
    opencascade::catch_crashes();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Oxide360")
            .with_app_id(APP_ID)
            .with_icon(help::app_icon().unwrap_or_default())
            .with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Oxide360",
        options,
        Box::new(|cc| {
            let s = settings::default_path().map(|p| settings::load(&p)).unwrap_or_default();
            // Set up the GPU renderer once and keep it with egui's renderer (all tabs share it).
            let gpu = if let Some(rs) = cc.wgpu_render_state.as_ref() {
                let g = gpu::Gpu::new(&rs.device, rs.target_format);
                rs.renderer.write().callback_resources.insert(g);
                true
            } else {
                false
            };
            let control = match control_port {
                Some(port) => {
                    let ctx = cc.egui_ctx.clone();
                    match control::serve(port, move || ctx.request_repaint()) {
                        Ok(rx) => Some(Control { rx, busy: None, queue: Default::default(), settle: 0, since: 0.0, asked_shot: false, shot: None }),
                        Err(e) => {
                            eprintln!("oxide360: can't listen on port {port}: {e}");
                            None
                        }
                    }
                }
                None => None,
            };
            let recovered = recovery::default_dir().map_or_else(Vec::new, |d| recovery::left_behind(&d, recovery::process_alive));
            let mut first = OpenCad::fresh(&s, gpu);
            if let Some(p) = open_at_start {
                first.open_path(p);
            }
            Ok(Box::new(Tabs { tabs: vec![first], current: 0, gpu, title: String::new(), control, recovered }))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tol_px() {
        assert_eq!(tol_px(0.5), 5.0);
    }
}
