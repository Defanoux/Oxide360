//! The Save window (as Fusion's): a name, and where — one of the cloud
//! folders or this computer, and a folder inside it — without the system's
//! file chooser. Folder contents are read off the main thread by the app
//! (a share can be slow); this holds the choices and draws the window.

use std::path::{Path, PathBuf};

/// A place designs are saved under: a cloud folder or this computer.
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub name: String,
    /// Where it is (None: a cloud folder that can't be reached now).
    pub path: Option<PathBuf>,
    pub cloud: bool,
    /// Why it can't be used ("offline"), shown greyed.
    pub note: Option<String>,
}

/// What's in the folder being looked at: its subfolders and designs (names).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub folders: Vec<String>,
    pub designs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SaveDialog {
    /// The design's name (without `.ox360`).
    pub name: String,
    /// Which root, and the folder inside it (relative).
    pub root: usize,
    pub sub: PathBuf,
    /// The name being typed for a new folder.
    pub new_folder: String,
    /// The window was opened by "Save changes?" before closing: what to do once saved.
    pub then: Option<crate::Pending>,
    /// The chosen file is there already and isn't this design's: asked once, then replaced.
    pub replace_ok: Option<PathBuf>,
}

/// What the window asks the app to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    None,
    Save(PathBuf),
    Cancel,
    /// The system's file chooser instead.
    Browse,
    /// Make this folder (then look in it).
    MakeFolder(PathBuf),
}

/// What's in `dir`: its subfolders and the designs in it (by name, sorted;
/// hidden ones left out).
pub fn read(dir: &Path) -> Result<Listing, String> {
    let mut l = Listing::default();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("couldn't read {}: {e}", dir.display()))?.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        if file.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            l.folders.push(file);
        } else if path.extension().is_some_and(|e| e == crate::doc::EXTENSION || e == crate::doc::OLD_EXTENSION) {
            l.designs.push(path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()));
        }
    }
    l.folders.sort_by_key(|f| f.to_lowercase());
    l.designs.sort_by_key(|f| f.to_lowercase());
    Ok(l)
}

/// `name` made into a file name: trimmed, any `.ox360` the user typed taken
/// off (it's added back once), and refused when empty or holding a slash.
pub fn clean_name(name: &str) -> Result<String, String> {
    let mut n = name.trim().to_string();
    loop {
        let lower = n.to_lowercase();
        let ext = [crate::doc::EXTENSION, crate::doc::OLD_EXTENSION].iter().find(|e| lower.ends_with(&format!(".{e}"))).copied();
        match ext {
            Some(e) => n.truncate(n.len() - e.len() - 1),
            None => break,
        }
    }
    let n = n.trim().to_string();
    if n.is_empty() {
        return Err("give the design a name".into());
    }
    if n.contains('/') || n.contains('\0') {
        return Err("a name can't have / in it".into());
    }
    if n == "." || n == ".." {
        return Err("that name is taken by the folder system".into());
    }
    Ok(n)
}

impl SaveDialog {
    /// Saving `name`, starting in `root`'s `sub` folder.
    pub fn new(name: &str, root: usize, sub: PathBuf) -> SaveDialog {
        let name = clean_name(name).unwrap_or_else(|_| name.to_string());
        SaveDialog { name, root, sub, new_folder: String::new(), then: None, replace_ok: None }
    }

    /// Start in the folder `current` (the design's file) is in, when it's under
    /// one of `roots`; else the first cloud folder that can be reached; else this computer.
    pub fn place_for(roots: &[Root], current: Option<&Path>) -> (usize, PathBuf) {
        if let Some(dir) = current.and_then(Path::parent) {
            if let Some((k, rest)) = roots.iter().enumerate().find_map(|(k, r)| r.path.as_ref().and_then(|p| dir.strip_prefix(p).ok()).map(|rest| (k, rest.to_path_buf()))) {
                return (k, rest);
            }
        }
        let k = roots.iter().position(|r| r.cloud && r.path.is_some()).or_else(|| roots.iter().position(|r| r.path.is_some())).unwrap_or(0);
        (k, PathBuf::new())
    }

    /// The folder being looked at.
    pub fn folder(&self, roots: &[Root]) -> Option<PathBuf> {
        roots.get(self.root)?.path.as_ref().map(|p| p.join(&self.sub))
    }

    /// The file it would save to.
    pub fn target(&self, roots: &[Root]) -> Result<PathBuf, String> {
        let name = clean_name(&self.name)?;
        let dir = self.folder(roots).ok_or_else(|| "that place can't be reached now".to_string())?;
        Ok(dir.join(format!("{name}.{}", crate::doc::EXTENSION)))
    }

    /// Into subfolder `name`.
    pub fn enter(&mut self, name: &str) {
        self.sub.push(name);
        self.replace_ok = None;
    }

    /// Up one folder (not above the root).
    pub fn up(&mut self) {
        self.sub.pop();
        self.replace_ok = None;
    }

    /// Another root, at its top.
    pub fn pick_root(&mut self, k: usize) {
        self.root = k;
        self.sub = PathBuf::new();
        self.replace_ok = None;
    }

    /// The window. `listing` is the folder's contents (None while it's read,
    /// Err when it can't be); `current` is this design's own file, if any.
    pub fn panel(&mut self, ui: &mut eframe::egui::Ui, roots: &[Root], listing: Option<&Result<Listing, String>>, current: Option<&Path>) -> Action {
        use eframe::egui;
        let pal = crate::theme::now();
        let mut act = Action::None;
        egui::Grid::new("save_name").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label("Name");
            let r = ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(320.0));
            if r.changed() {
                self.replace_ok = None;
            }
            ui.end_row();
        });
        ui.add_space(6.0);
        ui.allocate_ui_with_layout(egui::vec2(530.0, 290.0), egui::Layout::left_to_right(egui::Align::Min), |ui| {
            ui.set_min_height(290.0);
            // Where: the cloud folders and this computer.
            ui.vertical(|ui| {
                ui.set_width(170.0);
                ui.weak("LOCATION");
                for (k, r) in roots.iter().enumerate() {
                    let label = match &r.note {
                        Some(n) => format!("{}  ({n})", r.name),
                        None => r.name.clone(),
                    };
                    let on = k == self.root;
                    let b = ui.add_enabled(r.path.is_some(), egui::Button::selectable(on, label).min_size(egui::vec2(165.0, 0.0)));
                    let b = b.on_hover_text(r.path.as_ref().map_or_else(|| "Can't be reached now".to_string(), |p| p.display().to_string()));
                    if b.clicked() {
                        self.pick_root(k);
                    }
                }
            });
            ui.separator();
            // What's in the folder.
            ui.vertical(|ui| {
                ui.set_width(330.0);
                ui.horizontal(|ui| {
                    let mut crumbs = vec![roots.get(self.root).map_or("?".to_string(), |r| r.name.clone())];
                    crumbs.extend(self.sub.iter().map(|c| c.to_string_lossy().into_owned()));
                    if !self.sub.as_os_str().is_empty() && ui.small_button("Up").on_hover_text("The folder this one is in").clicked() {
                        self.up();
                    }
                    ui.label(egui::RichText::new(crumbs.join(" > ")).strong());
                });
                let offline = roots.get(self.root).is_some_and(|r| r.cloud && r.note.is_some());
                if let Some(r) = roots.get(self.root).filter(|_| offline) {
                    ui.colored_label(egui::Color32::from_rgb(230, 170, 80), format!("{} can't be reached now: it's saved on this computer and uploads when it's back.", r.name));
                }
                egui::ScrollArea::vertical().max_height(220.0).min_scrolled_height(220.0).show(ui, |ui| {
                    ui.set_min_width(320.0);
                    ui.set_min_height(220.0);
                    match listing {
                        None => {
                            ui.weak("Looking…");
                        }
                        // (Offline: said above; the read error is no news.)
                        Some(Err(_)) if offline => {}
                        Some(Err(e)) => {
                            ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
                        }
                        Some(Ok(l)) => {
                            if l.folders.is_empty() && l.designs.is_empty() {
                                ui.weak("(empty)");
                            }
                            let mut enter = None;
                            for f in &l.folders {
                                if row(ui, crate::icons::Icon::Folder, f, pal.text).on_hover_text("Open this folder").clicked() {
                                    enter = Some(f.clone());
                                }
                            }
                            for d in &l.designs {
                                let r = row(ui, crate::icons::Icon::File, d, pal.weak).on_hover_text("Click to use this name (Save then replaces it)");
                                if r.clicked() {
                                    self.name = d.clone();
                                    self.replace_ok = None;
                                }
                            }
                            if let Some(f) = enter {
                                self.enter(&f);
                            }
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_folder).hint_text("New folder name").desired_width(200.0));
                    let ok = clean_name(&self.new_folder).is_ok();
                    if ui.add_enabled(ok, egui::Button::new("New Folder")).clicked() {
                        if let Some(dir) = self.folder(roots) {
                            act = Action::MakeFolder(dir.join(self.new_folder.trim()));
                            self.enter(self.new_folder.trim().to_string().as_str());
                            self.new_folder.clear();
                        }
                    }
                });
            });
        });
        ui.separator();
        let target = self.target(roots);
        let exists = match (&target, listing) {
            (Ok(t), Some(Ok(l))) => {
                let n = t.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                l.designs.contains(&n) && Some(t.as_path()) != current
            }
            _ => false,
        };
        match &target {
            Ok(t) => {
                ui.weak(format!("Saves to {}", t.display()));
            }
            Err(e) => {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
            }
        }
        if exists {
            ui.colored_label(egui::Color32::from_rgb(230, 170, 80), "A design with this name is there already: Save replaces it.");
        }
        ui.horizontal(|ui| {
            if ui.button("Browse This Computer…").on_hover_text("Choose with the system's file chooser").clicked() {
                act = Action::Browse;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if exists { "Replace" } else { "Save" };
                if ui.add_enabled(target.is_ok(), egui::Button::new(label)).clicked() {
                    if let Ok(t) = target.clone() {
                        act = Action::Save(t);
                    }
                }
                if ui.button("Cancel").clicked() {
                    act = Action::Cancel;
                }
            });
        });
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter && act == Action::None {
            if let Ok(t) = target {
                act = Action::Save(t);
            }
        }
        act
    }
}

/// A folder or design in the list: its icon and name, clickable.
fn row(ui: &mut eframe::egui::Ui, icon: crate::icons::Icon, name: &str, color: eframe::egui::Color32) -> eframe::egui::Response {
    use eframe::egui;
    let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());
    if r.hovered() {
        ui.painter().rect_filled(rect, 4.0, crate::theme::now().raised);
    }
    crate::icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 3.0), egui::vec2(18.0, 18.0)), icon, true);
    ui.painter().text(rect.left_center() + egui::vec2(30.0, 0.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(14.0), color);
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<Root> {
        vec![
            Root { name: "TrueNAS".into(), path: Some(PathBuf::from("/nas/cad")), cloud: true, note: None },
            Root { name: "Nextcloud".into(), path: None, cloud: true, note: Some("offline".into()) },
            Root { name: "This computer".into(), path: Some(PathBuf::from("/home/g/Documents")), cloud: false, note: None },
        ]
    }

    #[test]
    fn names_get_one_extension_and_bad_ones_are_refused() {
        assert_eq!(clean_name("key-noelle"), Ok("key-noelle".into()));
        assert_eq!(clean_name(" key-noelle.ox360.ox360 "), Ok("key-noelle".into()), "typed extensions come off");
        assert_eq!(clean_name("old.OCAD"), Ok("old".into()));
        assert_eq!(clean_name("v2.1 bracket"), Ok("v2.1 bracket".into()), "other dots stay");
        assert!(clean_name("  ").is_err());
        assert!(clean_name("a/b").is_err());
        assert!(clean_name(".ox360").is_err());
    }

    #[test]
    fn it_starts_where_the_design_is_or_in_the_cloud() {
        let r = roots();
        assert_eq!(SaveDialog::place_for(&r, Some(Path::new("/nas/cad/keys/key.ox360"))), (0, PathBuf::from("keys")));
        assert_eq!(SaveDialog::place_for(&r, Some(Path::new("/home/g/Documents/a.ox360"))), (2, PathBuf::new()));
        assert_eq!(SaveDialog::place_for(&r, None), (0, PathBuf::new()), "a new design goes to the cloud");
        let offline = vec![r[1].clone(), r[2].clone()];
        assert_eq!(SaveDialog::place_for(&offline, None), (1, PathBuf::new()), "this computer when the cloud is away");
    }

    #[test]
    fn a_folder_reads_as_its_subfolders_and_designs() {
        let d = std::env::temp_dir().join(format!("oxide360-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Prints")).unwrap();
        std::fs::create_dir_all(d.join(".cache")).unwrap();
        for f in ["b.ox360", "A.ox360", "notes.txt", ".~b.ox360.lock"] {
            std::fs::write(d.join(f), "").unwrap();
        }
        assert_eq!(read(&d), Ok(Listing { folders: vec!["Prints".into()], designs: vec!["A".into(), "b".into()] }));
        assert!(read(&d.join("gone")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_target_is_the_folder_and_name() {
        let r = roots();
        let mut d = SaveDialog::new("key-noelle.ox360", 0, PathBuf::new());
        assert_eq!(d.name, "key-noelle");
        assert_eq!(d.target(&r), Ok(PathBuf::from("/nas/cad/key-noelle.ox360")));
        d.enter("keys");
        d.enter("old");
        d.up();
        assert_eq!(d.target(&r), Ok(PathBuf::from("/nas/cad/keys/key-noelle.ox360")));
        d.pick_root(1);
        assert!(d.target(&r).is_err(), "offline");
        d.pick_root(2);
        assert_eq!(d.target(&r), Ok(PathBuf::from("/home/g/Documents/key-noelle.ox360")));
        d.up();
        assert_eq!(d.sub, PathBuf::new(), "not above the root");
    }

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let mut d = SaveDialog::new("key-noelle", 0, PathBuf::from("keys"));
        let l = Ok(Listing { folders: vec!["old".into(), "prints".into()], designs: vec!["key-noelle".into(), "bracket".into()] });
        crate::ui_shot::shot(&format!("{dir}/save.png"), [600, 470], |ui| {
            d.panel(ui, &roots(), Some(&l), None);
        });
    }
}
