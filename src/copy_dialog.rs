//! The Mirror / Rectangular Pattern / Circular Pattern / Pattern on Path dialogs: what the user
//! has set so far, turned into a `CopyFeature`, and the panel that edits it.

use crate::doc::BodyId;
use crate::model::{CopyFeature, CopyKind, MAX_COPIES, WorldAxis};
use crate::sketch::OriginPlane;
use eframe::egui;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Mirror,
    Rectangular,
    Circular,
    Path,
}

impl Mode {
    pub fn title(self) -> &'static str {
        match self {
            Mode::Mirror => "Mirror",
            Mode::Rectangular => "Rectangular Pattern",
            Mode::Circular => "Circular Pattern",
            Mode::Path => "Pattern on Path",
        }
    }
}

/// Which selection field is waiting for clicks in the view (Fusion's
/// blue "Select" boxes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Bodies,
    Plane,
    /// Pattern on Path: the sketch curves to follow.
    Path,
}

pub enum Action {
    None,
    Ok,
    Cancel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CopyDialog {
    pub editing: Option<usize>,
    pub mode: Mode,
    pub bodies: Vec<BodyId>,
    pub plane: OriginPlane,
    pub dir: WorldAxis,
    pub count: u32,
    pub spacing: f64,
    pub dir2: WorldAxis,
    pub count2: u32,
    pub spacing2: f64,
    pub axis: WorldAxis,
    /// A construction axis instead of `axis`.
    pub custom: Option<usize>,
    pub count_round: u32,
    pub angle: f64,
    /// Extra distance from the axis for the copies (mm).
    pub radial: f64,
    /// Waiting for a click on an axis in the 3D view.
    pub picking_axis: bool,
    /// Mirror in a construction plane instead of `plane`.
    pub mirror_custom: Option<usize>,
    /// The field clicks in the view go to.
    pub picking: Option<Field>,
    /// A mirror plane has been chosen (a new Mirror starts with none).
    pub plane_set: bool,
    /// Pattern on Path: the sketch and its curves to follow, how many
    /// copies, the distance (between copies when `by_spacing`, else first to
    /// last), where the original is on the path (0 to 1), both ways, turned
    /// with the path, and from the path's other end.
    pub path_sketch: Option<usize>,
    pub path: Vec<usize>,
    pub count_path: u32,
    pub distance: f64,
    pub by_spacing: bool,
    pub start: f64,
    pub symmetric: bool,
    pub align: bool,
    pub flip: bool,
}

impl CopyDialog {
    pub fn new(mode: Mode, bodies: Vec<BodyId>) -> Self {
        CopyDialog {
            editing: None,
            mode,
            bodies,
            plane: OriginPlane::YZ,
            dir: WorldAxis::X,
            count: 3,
            spacing: 20.0,
            dir2: WorldAxis::Y,
            count2: 1,
            spacing2: 20.0,
            axis: WorldAxis::Z,
            custom: None,
            count_round: 4,
            angle: 360.0,
            radial: 0.0,
            picking_axis: false,
            mirror_custom: None,
            picking: None,
            plane_set: false,
            path_sketch: None,
            path: Vec::new(),
            count_path: 3,
            distance: 20.0,
            by_spacing: true,
            start: 0.0,
            symmetric: false,
            align: false,
            flip: false,
        }
    }

    /// The dialog for an existing feature, to edit it.
    pub fn from_feature(k: usize, f: &CopyFeature) -> Self {
        let mut d = CopyDialog::new(Mode::Mirror, f.bodies.clone());
        d.editing = Some(k);
        d.plane_set = true;
        match f.kind {
            CopyKind::Mirror(p) => d.plane = p,
            CopyKind::MirrorPlane(k) => d.mirror_custom = Some(k),
            CopyKind::Rectangular { dir, count, spacing, dir2, count2, spacing2 } => {
                (d.mode, d.dir, d.count, d.spacing, d.dir2, d.count2, d.spacing2) = (Mode::Rectangular, dir, count, spacing, dir2, count2, spacing2);
            }
            CopyKind::Circular { axis, custom, count, angle, radial } => {
                (d.mode, d.axis, d.custom, d.count_round, d.angle, d.radial) = (Mode::Circular, axis, custom, count, angle, radial)
            }
            CopyKind::Path { sketch, count, distance, spacing, start, symmetric, align, flip } => {
                (d.mode, d.path_sketch, d.path) = (Mode::Path, Some(sketch), f.path.clone());
                (d.count_path, d.distance, d.by_spacing, d.start, d.symmetric, d.align, d.flip) = (count, distance, spacing, start, symmetric, align, flip);
            }
        }
        d
    }

    pub fn kind(&self) -> CopyKind {
        match self.mode {
            Mode::Mirror => match self.mirror_custom {
                Some(k) => CopyKind::MirrorPlane(k),
                None => CopyKind::Mirror(self.plane),
            },
            Mode::Rectangular => CopyKind::Rectangular {
                dir: self.dir,
                count: self.count,
                spacing: self.spacing,
                dir2: self.dir2,
                count2: self.count2,
                spacing2: self.spacing2,
            },
            Mode::Circular => CopyKind::Circular { axis: self.axis, custom: self.custom, count: self.count_round, angle: self.angle, radial: self.radial },
            Mode::Path => CopyKind::Path {
                sketch: self.path_sketch.unwrap_or(0),
                count: self.count_path,
                distance: self.distance,
                spacing: self.by_spacing,
                start: self.start,
                symmetric: self.symmetric,
                align: self.align,
                flip: self.flip,
            },
        }
    }

    /// Everything it needs is picked: a body, and a mirror's plane or a
    /// path pattern's path.
    pub fn ready(&self) -> bool {
        !self.bodies.is_empty() && (self.mode != Mode::Mirror || self.plane_set) && (self.mode != Mode::Path || (self.path_sketch.is_some() && !self.path.is_empty()))
    }

    /// The feature this dialog would make; None until a body (and, for a
    /// mirror, a plane; for a pattern on a path, the path) is picked.
    pub fn feature(&self, name: String) -> Option<CopyFeature> {
        let ready = self.ready();
        ready.then(|| CopyFeature { name, bodies: self.bodies.clone(), kind: self.kind(), path: if self.mode == Mode::Path { self.path.clone() } else { Vec::new() } })
    }

    /// The right-hand panel. `bodies` are the bodies that can be picked.
    pub fn panel(
        &mut self,
        ui: &mut egui::Ui,
        bodies: &[(BodyId, String)],
        axes: &[String],
        planes: &[String],
        unit: crate::units::Unit,
        error: Option<&str>,
        message: Option<&str>,
    ) -> Action {
        ui.heading(if self.editing.is_some() { format!("Edit {}", self.mode.title()) } else { self.mode.title().to_string() });
        // Selection fields: click one, then click in the view.
        let field = |ui: &mut egui::Ui, active: bool, text: String| {
            let text = if active { egui::RichText::new(text).strong() } else { egui::RichText::new(text) };
            ui.add(egui::Button::selectable(active, text).min_size(egui::vec2(150.0, 0.0)))
        };
        ui.horizontal(|ui| {
            ui.label("Bodies");
            let names: Vec<&str> = bodies.iter().filter(|(id, _)| self.bodies.contains(id)).map(|(_, n)| n.as_str()).collect();
            let picking = self.picking == Some(Field::Bodies);
            let text = match (picking, names.len()) {
                (true, 0) => "Click bodies in the view…".to_string(),
                (_, 0) => "Click to select".to_string(),
                (_, n) if n <= 2 => names.join(", "),
                (_, n) => format!("{n} bodies"),
            };
            if field(ui, picking, text).on_hover_text("Click, then click bodies in the view (click again to drop one)").clicked() {
                self.picking = if picking { None } else { Some(Field::Bodies) };
                self.picking_axis = false;
            }
            if !self.bodies.is_empty() && ui.small_button("×").on_hover_text("Clear").clicked() {
                self.bodies.clear();
            }
        });
        if bodies.is_empty() {
            ui.weak("Make a body first.");
        }
        ui.separator();
        let axis_pick = |ui: &mut egui::Ui, label: &str, axis: &mut WorldAxis| {
            ui.horizontal(|ui| {
                ui.label(label);
                for a in WorldAxis::ALL {
                    ui.selectable_value(axis, a, a.name());
                }
            });
        };
        let count_box = |ui: &mut egui::Ui, n: &mut u32| {
            ui.horizontal(|ui| {
                ui.label("Quantity");
                ui.add(egui::DragValue::new(n).range(1..=MAX_COPIES).speed(0.2));
            });
        };
        match self.mode {
            Mode::Mirror => {
                ui.horizontal(|ui| {
                    ui.label("Mirror plane");
                    let picking = self.picking == Some(Field::Plane);
                    let name = match self.mirror_custom.and_then(|k| planes.get(k)) {
                        Some(n) => n.clone(),
                        None if self.plane_set => format!("{} plane", self.plane.name()),
                        None => "Click to select".to_string(),
                    };
                    let text = if picking { "Click a plane in the view…".to_string() } else { name };
                    if field(ui, picking, text).on_hover_text("Click, then click an origin or construction plane in the view").clicked() {
                        self.picking = if picking { None } else { Some(Field::Plane) };
                    }
                });
            }
            Mode::Rectangular => {
                ui.strong("Direction 1");
                axis_pick(ui, "Axis", &mut self.dir);
                count_box(ui, &mut self.count);
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    ui.add(crate::units::length_drag(&mut self.spacing, unit, 0.5));
                });
                ui.strong("Direction 2");
                axis_pick(ui, "Axis", &mut self.dir2);
                count_box(ui, &mut self.count2);
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    ui.add(crate::units::length_drag(&mut self.spacing2, unit, 0.5));
                });
                ui.weak("Distance is the gap from one copy to the next. Quantity counts the original.");
            }
            Mode::Circular => {
                // Click to select: press the button, then click an axis in the view.
                ui.horizontal(|ui| {
                    ui.label("Axis");
                    let name = match self.custom.and_then(|k| axes.get(k)) {
                        Some(n) => n.clone(),
                        None => format!("{} axis", self.axis.name()),
                    };
                    let text = if self.picking_axis { "Click an axis…".to_string() } else { name };
                    if ui.selectable_label(self.picking_axis, text).clicked() {
                        self.picking_axis = !self.picking_axis;
                        self.picking = None;
                    }
                });
                if self.picking_axis {
                    ui.weak("Click X, Y or Z, or a construction axis, in the view.");
                }
                count_box(ui, &mut self.count_round);
                ui.horizontal(|ui| {
                    ui.label("Angle");
                    ui.add(egui::DragValue::new(&mut self.angle).speed(1.0).range(-360.0..=360.0).suffix("°"));
                    if ui.button("Full").clicked() {
                        self.angle = 360.0;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    ui.add(crate::units::length_drag(&mut self.radial, unit, 0.5));
                });
                ui.weak("Distance moves the copies out from the axis. Drag the blue arrows, or type.");
            }
            Mode::Path => {
                ui.horizontal(|ui| {
                    ui.label("Path");
                    let picking = self.picking == Some(Field::Path);
                    let text = match (picking, self.path.len()) {
                        (true, _) => "Click a sketch curve…".to_string(),
                        (_, 0) => "Click to select".to_string(),
                        (_, 1) => "1 curve".to_string(),
                        (_, n) => format!("{n} curves"),
                    };
                    if field(ui, picking, text).on_hover_text("Click, then click a line, arc, spline or circle of a sketch; the curves joined to it come too").clicked() {
                        self.picking = if picking { None } else { Some(Field::Path) };
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Distance type");
                    ui.selectable_value(&mut self.by_spacing, false, "Extent").on_hover_text("Distance is from the first copy to the last");
                    ui.selectable_value(&mut self.by_spacing, true, "Spacing").on_hover_text("Distance is from one copy to the next");
                });
                count_box(ui, &mut self.count_path);
                ui.horizontal(|ui| {
                    ui.label("Distance");
                    ui.add(crate::units::length_drag(&mut self.distance, unit, 0.5));
                });
                ui.horizontal(|ui| {
                    ui.label("Start point");
                    ui.add(egui::DragValue::new(&mut self.start).speed(0.01).range(0.0..=1.0).fixed_decimals(2)).on_hover_text("Where on the path the original counts as being: 0 its start, 1 its end");
                    if ui.button("Flip").on_hover_text("Go along the path from its other end").clicked() {
                        self.flip = !self.flip;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Direction");
                    ui.selectable_value(&mut self.symmetric, false, "One direction");
                    ui.selectable_value(&mut self.symmetric, true, "Symmetric").on_hover_text("The same copies both ways from the original");
                });
                ui.horizontal(|ui| {
                    ui.label("Orientation");
                    ui.selectable_value(&mut self.align, false, "Identical").on_hover_text("Every copy faces the way the original does");
                    ui.selectable_value(&mut self.align, true, "Path direction").on_hover_text("Each copy turns with the path");
                });
                ui.weak("Each copy moves the way the path goes from the start point. Quantity counts the original.");
            }
        }
        if let (Some(e), false) = (error, self.bodies.is_empty()) {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
        }
        if let Some(m) = message {
            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), m);
        }
        ui.separator();
        let mut action = Action::None;
        ui.horizontal(|ui| {
            let ready = self.ready();
            if ui.add_enabled(ready && error.is_none(), egui::Button::new("OK")).clicked() {
                action = Action::Ok;
            }
            if ui.button("Cancel").clicked() {
                action = Action::Cancel;
            }
        });
        action
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Feature;

    fn id(k: usize) -> BodyId {
        BodyId { feature: Feature::Extrude(k), piece: 0 }
    }

    #[test]
    fn a_dialog_makes_the_feature_and_an_existing_one_reopens_the_same() {
        let mut d = CopyDialog::new(Mode::Circular, vec![]);
        assert!(d.feature("C".into()).is_none(), "no body picked yet");
        d.bodies = vec![id(0), id(2)];
        d.count_round = 6;
        d.angle = 180.0;
        d.radial = 3.5;
        d.axis = WorldAxis::X;
        d.custom = Some(2);
        let f = d.feature("Circular Pattern1".into()).unwrap();
        assert_eq!(f.kind, CopyKind::Circular { axis: WorldAxis::X, custom: Some(2), count: 6, angle: 180.0, radial: 3.5 });
        assert_eq!(CopyDialog::from_feature(3, &f).feature(f.name.clone()), Some(f.clone()));
        assert_eq!(CopyDialog::from_feature(3, &f).editing, Some(3));

        let mut r = CopyDialog::new(Mode::Rectangular, vec![id(1)]);
        r.count2 = 4;
        r.spacing2 = 7.5;
        let f = r.feature("R".into()).unwrap();
        assert_eq!(CopyDialog::from_feature(0, &f).feature("R".into()), Some(f));

        // A new mirror has no plane until one is picked: no feature yet.
        let mut mp = CopyDialog::new(Mode::Mirror, vec![id(1)]);
        assert!(mp.feature("M".into()).is_none(), "no mirror plane picked");
        // Mirror in a construction plane, and back.
        (mp.mirror_custom, mp.plane_set) = (Some(2), true);
        let f = mp.feature("M".into()).unwrap();
        assert_eq!(f.kind, CopyKind::MirrorPlane(2));
        assert_eq!(CopyDialog::from_feature(0, &f).feature("M".into()), Some(f));

        let mut o = CopyDialog::new(Mode::Mirror, vec![id(1)]);
        (o.plane, o.plane_set) = (OriginPlane::XZ, true);
        let m = o.feature("M".into()).unwrap();
        assert_eq!(m.kind, CopyKind::Mirror(OriginPlane::XZ));
        assert_eq!(CopyDialog::from_feature(0, &m).mode, Mode::Mirror);
    }

    #[test]
    fn a_pattern_on_a_path_waits_for_its_path_and_reopens_the_same() {
        let mut d = CopyDialog::new(Mode::Path, vec![id(0)]);
        assert!(!d.ready() && d.feature("P".into()).is_none(), "no path picked yet");
        (d.path_sketch, d.path) = (Some(2), vec![4, 5]);
        (d.count_path, d.distance, d.by_spacing, d.start, d.symmetric, d.align, d.flip) = (5, 12.5, false, 0.25, true, true, true);
        let f = d.feature("P".into()).unwrap();
        assert_eq!(f.kind, CopyKind::Path { sketch: 2, count: 5, distance: 12.5, spacing: false, start: 0.25, symmetric: true, align: true, flip: true });
        assert_eq!(f.path, vec![4, 5]);
        let again = CopyDialog::from_feature(1, &f);
        assert_eq!((again.mode, again.editing), (Mode::Path, Some(1)));
        assert_eq!(again.feature("P".into()), Some(f.clone()));
        // Saved and read back (and files from before it have no path list).
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(serde_json::from_str::<CopyFeature>(&json).unwrap(), f);
        // Other patterns never carry a path.
        let mut r = CopyDialog::new(Mode::Rectangular, vec![id(0)]);
        r.path = vec![1];
        assert!(r.feature("R".into()).unwrap().path.is_empty());
    }
}
