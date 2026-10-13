//! Preferences kept between runs (not part of any design): a small JSON file
//! in the user's config directory. A missing or damaged file just means the
//! defaults, and a failed save is silent — losing a preference is never worth
//! interrupting work.

use crate::camera::Projection;
use crate::measure::Unit;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Touchpad controls instead of mouse.
    pub touchpad: bool,
    /// Show the XYZ triad on the view cube.
    pub cube_axes: bool,
    /// Perspective, orthographic, or perspective with ortho faces.
    pub projection: Projection,
    /// The Measure window's unit, and a second one shown under it.
    /// The Measure window's own unit; None follows the design's units.
    pub measure_unit_own: Option<Unit>,
    pub measure_unit2: Option<Unit>,
    /// Measure: decimal places, snap points on, X/Y/Z delta shown.
    pub measure_precision: usize,
    pub measure_snap: bool,
    pub measure_xyz: bool,
    /// Units new designs start in (each design then keeps its own).
    pub default_units: Unit,
    /// Touchpad Ctrl+two-finger scroll: fingers up zoom in.
    pub touchpad_zoom_flip: bool,
    /// Tools kept off the toolbar (still in their group's ▾ list), by name.
    pub toolbar_hidden: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self { touchpad: false, cube_axes: true, projection: Projection::default(), measure_unit_own: None, measure_unit2: None, measure_precision: 3, measure_snap: true, measure_xyz: true, default_units: Unit::Mm,
            touchpad_zoom_flip: true,
            toolbar_hidden: ["Rectangular Pattern", "Circular Pattern", "Collinear", "Concentric", "Midpoint", "Fix"].map(String::from).to_vec(),
        }
    }
}

/// `$XDG_CONFIG_HOME/opencad/settings.json`, or `~/.config/opencad/settings.json`.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".config")))?;
    Some(base.join("opencad").join("settings.json"))
}

pub fn load(path: &Path) -> Settings {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Write the settings (creating the folder). Errors are returned for tests;
/// the app ignores them.
pub fn save(path: &Path, s: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Write beside it and rename, so a crash never leaves half a file.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(s).expect("settings serialize"))?;
    std::fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("opencad-settings-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn settings_round_trip_through_a_file_that_may_not_exist_yet() {
        let d = dir("roundtrip");
        let path = d.join("nested").join("settings.json");
        assert_eq!(load(&path), Settings::default(), "nothing saved yet");
        let s = Settings { touchpad: true, cube_axes: false, projection: Projection::Orthographic, measure_unit_own: Some(Unit::In), measure_unit2: Some(Unit::Mm), measure_precision: 5, measure_snap: false, measure_xyz: false, default_units: Unit::In, touchpad_zoom_flip: false, toolbar_hidden: vec!["Trim".into()] };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn damaged_or_partial_files_fall_back_to_defaults() {
        let d = dir("damaged");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        // A file from an older version with only some settings keeps the rest at default.
        std::fs::write(&path, r#"{"touchpad": true}"#).unwrap();
        assert_eq!(load(&path), Settings { touchpad: true, ..Settings::default() });
        let _ = std::fs::remove_dir_all(&d);
    }
}
