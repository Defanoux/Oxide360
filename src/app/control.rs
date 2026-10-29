//! What the control link (`crate::control`) asks of a design: a summary of
//! its state, and opening / saving / starting over without dialogs.

use crate::*;
use serde_json::{json, Value};

impl OpenCad {
    /// The design and what's on screen, for `state`.
    pub(crate) fn control_state(&self) -> Value {
        let timeline: Vec<Value> = self
            .timeline
            .iter()
            .enumerate()
            .map(|(k, &f)| {
                json!({
                    "name": self.feature_label(f),
                    "kind": format!("{f:?}"),
                    "error": self.feature_error(f),
                    "suppressed": self.suppressed.contains(&f),
                    "rolled_back": k >= self.applied(),
                })
            })
            .collect();
        let bodies: Vec<Value> = self.caches[Slot::Committed as usize].as_ref().map_or(Vec::new(), |c| {
            c.model
                .bodies
                .iter()
                .map(|b| {
                    let (volume, area, center) = kernel::body_properties(&b.shape);
                    let m = self.body_material(b.id);
                    json!({"name": self.body_name(b.id), "material": m.name, "appearance": self.body_appearance(b.id).map(|a| a.name), "face_appearances": self.body_face_looks(b).iter().map(|(_, a)| a.name).collect::<Vec<_>>(), "mass_g": materials::mass_g(volume, m.density), "made_by": self.feature_label(b.id.feature), "volume_mm3": volume, "area_mm2": area, "center": [center.x, center.y, center.z], "hidden": self.hidden_bodies.contains(&b.id)})
                })
                .collect()
        });
        let sketch = self.active.map(|i| {
            let sk = &self.sketches[i];
            json!({
                "name": sk.name,
                "plane": sk.plane_name(),
                "entities": sk.entities.len(),
                "constraints": sk.constraints.len(),
                "texts": sk.texts.iter().map(|t| t.text.clone()).collect::<Vec<_>>(),
                "degrees_of_freedom": self.dof,
                "profiles": find_regions(sk).len(),
                "selection": format!("{:?}", self.editor.selection),
                "message": self.message,
            })
        });
        let open: Vec<&str> = [
            ("Extrude", self.dialog.is_some()),
            ("Fillet/Chamfer", self.edge_dialog.is_some()),
            ("Revolve", self.revolve_dialog.is_some()),
            ("Pattern/Mirror", self.copy_dialog.is_some()),
            ("Plane", self.plane_dialog.is_some()),
            ("Hole", self.hole_dialog.is_some()),
            ("Thread", self.thread_dialog.is_some()),
            ("Shell", self.shell_dialog.is_some()),
            ("Press Pull", self.offset_dialog.is_some()),
            ("Combine", self.combine_dialog.is_some()),
            ("Move/Copy", self.move_dialog.is_some()),
            ("Sketch Pattern", self.sketch_pattern.is_some()),
            ("Text", self.text_dialog.is_some()),
            ("Measure", self.measure.is_some()),
            ("Interference", self.interference.is_some()),
            ("Insert", self.import_dialog.is_some()),
            ("Joint", self.joint_dialog.is_some()),
            ("Properties", self.properties_of.is_some()),
            ("Change Parameters", self.params_window.open),
        ]
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| n)
        .collect();
        json!({
            "title": self.tab_title(),
            "file": self.path.as_ref().map(|p| p.display().to_string()),
            "units": format!("{:?}", self.units),
            "timeline": timeline,
            "bodies": bodies,
            "sketch": sketch,
            "tool": self.tool_name(),
            "workspace": if self.drawing_open { "DRAWING" } else if self.active.is_some() { "SKETCH" } else { "SOLID" },
            "dialogs": open,
            "notice": self.toast.as_ref().map(|t| t.0.clone()),
            "alert": self.alert,
            "solid_selection": self.solid_sel.iter().map(|p| format!("{:?}", p.pick)).collect::<Vec<_>>(),
            "camera": {"yaw_deg": self.camera.yaw.to_degrees(), "pitch_deg": self.camera.pitch.to_degrees(), "roll_deg": self.camera.roll.to_degrees(), "distance": self.camera.distance},
            "building": self.builds.iter().any(|b| b.is_some()),
        })
    }

    /// Open a design file (no dialog, no questions).
    pub(crate) fn control_open(&mut self, path: &str) -> Result<(), String> {
        let d = doc::load(std::path::Path::new(path))?;
        self.reset_with(d, Some(path.into()));
        Ok(())
    }

    /// Save the design to `path`.
    pub(crate) fn control_save(&mut self, path: &str) -> Result<(), String> {
        if self.save_to(path.into()) { Ok(()) } else { Err(self.alert.take().unwrap_or_else(|| "couldn't save".into())) }
    }

    /// Start over with an empty design (unsaved work is dropped).
    pub(crate) fn control_new(&mut self) {
        self.reset_with(Document::default(), None);
    }

    /// Is a rebuild still under way?
    pub(crate) fn control_busy(&self) -> bool {
        self.builds.iter().any(|b| b.is_some())
    }
}
