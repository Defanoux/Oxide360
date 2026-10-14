//! Keyboard shortcuts for tools: which key does what (set in Settings →
//! Keybinds, kept with the settings). A key works in its own place only: the
//! sketch keys while sketching, the model keys in the 3D view.

use eframe::egui::Key;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    // Sketch
    Line,
    Rectangle,
    Circle,
    Dimension,
    Trim,
    Extend,
    Offset,
    Construction,
    Project,
    GridSnap,
    // Model
    NewSketch,
    Extrude,
    Fillet,
    Measure,
}

impl Action {
    pub const ALL: [Action; 14] = [
        Action::Line,
        Action::Rectangle,
        Action::Circle,
        Action::Dimension,
        Action::Trim,
        Action::Extend,
        Action::Offset,
        Action::Construction,
        Action::Project,
        Action::GridSnap,
        Action::NewSketch,
        Action::Extrude,
        Action::Fillet,
        Action::Measure,
    ];

    /// Its name (also how it's saved).
    pub fn name(self) -> &'static str {
        match self {
            Action::Line => "Line",
            Action::Rectangle => "Rectangle",
            Action::Circle => "Circle",
            Action::Dimension => "Dimension",
            Action::Trim => "Trim",
            Action::Extend => "Extend",
            Action::Offset => "Offset",
            Action::Construction => "Construction",
            Action::Project => "Project",
            Action::GridSnap => "Grid snap",
            Action::NewSketch => "New sketch",
            Action::Extrude => "Extrude",
            Action::Fillet => "Fillet",
            Action::Measure => "Measure",
        }
    }

    /// What it does, for the keybinds window.
    pub fn about(self) -> &'static str {
        match self {
            Action::Line => "Draw lines (type a length after the first point)",
            Action::Rectangle => "Draw a rectangle (the last kind used)",
            Action::Circle => "Draw a circle (the last kind used)",
            Action::Dimension => "Dimension what's selected, or pick things to dimension",
            Action::Trim => "Click the part of a line to cut away",
            Action::Extend => "Run a line on to the next geometry",
            Action::Offset => "Copy a line, arc or circle run at a distance",
            Action::Construction => "Make the selection (or what you draw next) construction",
            Action::Project => "Bring a body edge into the sketch",
            Action::GridSnap => "Snap to the grid on or off",
            Action::NewSketch => "Start a sketch (on the selected plane, or pick one)",
            Action::Extrude => "Extrude a profile",
            Action::Fillet => "Round body edges",
            Action::Measure => "Measure distances, sizes and angles",
        }
    }

    /// Does it work while sketching (else in the 3D view)?
    pub fn in_sketch(self) -> bool {
        !matches!(self, Action::NewSketch | Action::Extrude | Action::Fillet | Action::Measure)
    }

    pub fn default_key(self) -> Option<Key> {
        Some(match self {
            Action::Line => Key::L,
            Action::Rectangle => Key::R,
            Action::Circle => Key::C,
            Action::Dimension => Key::D,
            Action::Trim => Key::X,
            Action::Extend => return None,
            Action::Offset => Key::O,
            Action::Construction => Key::K,
            Action::Project => Key::P,
            Action::GridSnap => Key::G,
            Action::NewSketch => Key::N,
            Action::Extrude => Key::E,
            Action::Fillet => Key::F,
            Action::Measure => Key::I,
        })
    }

    fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// Which key each action has.
#[derive(Clone, Debug, PartialEq)]
pub struct Keymap(Vec<(Action, Option<Key>)>);

impl Default for Keymap {
    fn default() -> Self {
        Keymap(Action::ALL.into_iter().map(|a| (a, a.default_key())).collect())
    }
}

impl Keymap {
    pub fn key(&self, a: Action) -> Option<Key> {
        self.0.iter().find(|(x, _)| *x == a).and_then(|(_, k)| *k)
    }

    /// Give `a` key `k` (None: no key). Another action of the same place
    /// (sketch or model) that had that key loses it; it's returned.
    pub fn set(&mut self, a: Action, k: Option<Key>) -> Option<Action> {
        let mut displaced = None;
        if let Some(k) = k {
            for (x, key) in &mut self.0 {
                if *x != a && x.in_sketch() == a.in_sketch() && *key == Some(k) {
                    *key = None;
                    displaced = Some(*x);
                }
            }
        }
        if let Some(slot) = self.0.iter_mut().find(|(x, _)| *x == a) {
            slot.1 = k;
        }
        displaced
    }

    /// As saved: (action name, key name or "").
    pub fn to_pairs(&self) -> Vec<(String, String)> {
        self.0.iter().map(|(a, k)| (a.name().to_string(), k.map_or(String::new(), |k| k.name().to_string()))).collect()
    }

    /// From what was saved; anything missing or unknown keeps its default.
    pub fn from_pairs(pairs: &[(String, String)]) -> Self {
        let mut map = Keymap::default();
        for (a, k) in pairs {
            if let Some(a) = Action::from_name(a) {
                let key = if k.is_empty() { None } else { Key::from_name(k) };
                if let Some(slot) = map.0.iter_mut().find(|(x, _)| *x == a) {
                    slot.1 = key;
                }
            }
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_is_x_and_keys_are_unique_where_they_work() {
        let m = Keymap::default();
        assert_eq!(m.key(Action::Trim), Some(Key::X));
        for place in [true, false] {
            let keys: Vec<&str> = Action::ALL.into_iter().filter(|a| a.in_sketch() == place).filter_map(|a| m.key(a)).map(|k| k.name()).collect();
            let unique: std::collections::HashSet<&str> = keys.iter().copied().collect();
            assert_eq!(unique.len(), keys.len(), "a key used twice");
        }
    }

    #[test]
    fn a_taken_key_moves_and_bindings_save_and_load() {
        let mut m = Keymap::default();
        // L for Trim takes it from Line (both sketch keys)…
        assert_eq!(m.set(Action::Trim, Some(Key::L)), Some(Action::Line));
        assert_eq!((m.key(Action::Trim), m.key(Action::Line)), (Some(Key::L), None));
        // …but E for Extend (sketch) leaves Extrude (model) alone.
        assert_eq!(m.set(Action::Extend, Some(Key::E)), None);
        assert_eq!(m.key(Action::Extrude), Some(Key::E));
        let back = Keymap::from_pairs(&m.to_pairs());
        assert_eq!(back, m);
        // Old or partial saves: the rest keep their defaults.
        let partial = Keymap::from_pairs(&[("Trim".into(), "T".into()), ("Nonsense".into(), "Q".into())]);
        assert_eq!((partial.key(Action::Trim), partial.key(Action::Line)), (Some(Key::T), Some(Key::L)));
    }
}
