//! The start page: shown on an empty new design when the app opens (and from
//! File → Start Page). New and Open, the designs opened lately, and the
//! designs in the user's cloud folders (their own TrueNAS / Samba share or
//! Nextcloud folder — see `cloud`).

use crate::*;
use icons::Icon;

/// How many designs a cloud folder shows before "and N more".
const SHOWN: usize = 12;

impl OpenCad {
    /// Is the start page up? Only over an empty, untitled design with nothing going on.
    pub(crate) fn showing_start(&self) -> bool {
        self.start_page && self.path.is_none() && self.timeline.is_empty() && self.active.is_none() && self.tool_name().is_none()
    }

    /// Look through every cloud folder again, and check the Recent designs
    /// (in the background: a share that's gone away can take a while to say so).
    pub(crate) fn scan_cloud(&mut self, ctx: &egui::Context) {
        let folders = self.cloud_folders.clone();
        let recent = self.recent.clone();
        self.cloud_lists = vec![None; folders.len()];
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            cloud::look(&folders, &recent, |found| {
                let ok = tx.send(found).is_ok();
                ctx.request_repaint();
                ok
            });
        });
        self.cloud_scan = Some(rx);
    }

    /// Take in what the background look has found so far.
    fn poll_cloud(&mut self) {
        let Some(rx) = &self.cloud_scan else { return };
        let mut got = Vec::new();
        let done = loop {
            match rx.try_recv() {
                Ok(f) => got.push(f),
                Err(std::sync::mpsc::TryRecvError::Empty) => break false,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break true,
            }
        };
        if done {
            self.cloud_scan = None;
        }
        for found in got {
            match found {
                cloud::Found::Folder(k, r) => {
                    let r = r.map(|(at, list)| {
                        // Found somewhere new (a new login's mount): remember where.
                        if let Some(f) = self.cloud_folders.get_mut(k) {
                            f.path = at;
                        }
                        list
                    });
                    if let Some(slot) = self.cloud_lists.get_mut(k) {
                        *slot = Some(r);
                    }
                }
                cloud::Found::Recent(seen) => {
                    for (was, now, _, _) in &seen {
                        if was != now {
                            for p in &mut self.recent {
                                if p == was {
                                    *p = now.clone();
                                }
                            }
                        }
                    }
                    self.recent_seen = seen.into_iter().map(|(_, now, there, at)| (now, there, at)).collect();
                }
            }
        }
    }

    /// Add a cloud folder (picked in the folder chooser).
    pub(crate) fn add_cloud_folder(&mut self, ctx: &egui::Context) {
        if let Some(dir) = rfd::FileDialog::new().set_title("Add a cloud folder (your share or Nextcloud folder)").pick_folder() {
            if !self.cloud_folders.iter().any(|f| f.path == dir) {
                self.cloud_folders.push(cloud::CloudFolder::at(dir));
            }
            self.scan_cloud(ctx);
        }
    }

    /// Open `p` from the start page (in this tab: it's an empty design).
    fn open_from_start(&mut self, p: PathBuf) {
        self.open_in_tab = Some(p);
        self.tab_request = Some(TabRequest::Open);
    }

    pub(crate) fn start_page_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.cloud_lists.len() != self.cloud_folders.len() && self.cloud_scan.is_none() {
            self.scan_cloud(&ctx);
        }
        self.poll_cloud();
        let pal = theme::now();
        ui.painter().rect_filled(ui.max_rect(), 0.0, pal.view);
        let now = std::time::SystemTime::now();
        let mut open: Option<PathBuf> = None;
        let mut remove: Option<usize> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width().min(980.0);
            let side = ((ui.available_width() - width) / 2.0).max(16.0);
            ui.add_space(36.0);
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width - 32.0);
                    ui.heading(egui::RichText::new("Oxide360").size(30.0));
                    ui.add_space(18.0);
                    ui.horizontal(|ui| {
                        if big_button(ui, Icon::NewDesign, "New Design", "Start an empty design (Ctrl+N)").clicked() {
                            self.start_page = false;
                        }
                        ui.add_space(12.0);
                        if big_button(ui, Icon::OpenDesign, "Open…", "Open a design from a file (Ctrl+O)").clicked() {
                            self.open_dialog();
                        }
                        ui.add_space(12.0);
                        if big_button(ui, Icon::Cloud, "Add Cloud Folder…", "A folder on your own share (TrueNAS / Samba, mounted here) or your Nextcloud folder: its designs show here").clicked() {
                            self.add_cloud_folder(&ctx);
                        }
                    });
                    ui.add_space(26.0);
                    ui.label(egui::RichText::new("RECENT").strong().color(pal.weak));
                    ui.separator();
                    if self.recent.is_empty() {
                        ui.weak("Designs you open or save show here.");
                    }
                    for p in self.recent.clone() {
                        let name = p.file_stem().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
                        let place = p.parent().map(|d| d.display().to_string()).unwrap_or_default();
                        // (Looked up in the background; until then, taken to be there.)
                        let seen = self.recent_seen.iter().find(|s| s.0 == p);
                        let when = seen.and_then(|s| s.2).map(|t| cloud::ago(now, t));
                        let there = seen.is_none_or(|s| s.1);
                        let cloudy = cloud::in_cloud(&self.cloud_folders, &p);
                        if design_row(ui, &name, &place, when.as_deref(), there, cloudy).clicked() {
                            open = Some(p.clone());
                        }
                    }
                    ui.add_space(26.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("CLOUD FOLDERS").strong().color(pal.weak));
                        if !self.cloud_folders.is_empty() && ui.small_button("Refresh").on_hover_text("Look through the folders again").clicked() {
                            self.scan_cloud(&ctx);
                        }
                    });
                    ui.separator();
                    if self.cloud_folders.is_empty() {
                        ui.weak("Keep designs on your own server and open them from any computer:");
                        ui.weak("  · TrueNAS or another Samba share: connect to it (in Dolphin: Network, or mount it in /etc/fstab), then Add Cloud Folder and pick a folder on it.");
                        ui.weak("  · Nextcloud: let the Nextcloud app sync a folder, then Add Cloud Folder and pick that folder.");
                        ui.weak("While a design from a cloud folder is open, other computers are told who has it; a save never quietly replaces one saved elsewhere.");
                    }
                    for (k, f) in self.cloud_folders.clone().iter().enumerate() {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
                            icons::paint(ui.painter(), r, Icon::Cloud, true);
                            ui.label(egui::RichText::new(&f.name).strong().size(15.0));
                            ui.weak(f.path.display().to_string());
                            if ui.small_button("×").on_hover_text("Stop listing this folder (its files stay where they are)").clicked() {
                                remove = Some(k);
                            }
                        });
                        match self.cloud_lists.get(k).cloned().flatten() {
                            None => {
                                ui.weak("    Looking…");
                            }
                            Some(Err(e)) => {
                                ui.colored_label(egui::Color32::from_rgb(230, 120, 90), format!("    {e}"));
                                // Away: what this computer has copies of, to work on until it's back.
                                let copies: Vec<cloud::Entry> = cloud::Cache::default_cache().map_or_else(Vec::new, |c| c.entries()).into_iter().filter(|e| e.cloud.starts_with(&f.path)).collect();
                                if !copies.is_empty() {
                                    ui.weak("    Copies on this computer (saves upload when it's back):");
                                }
                                for c in copies {
                                    let name = c.cloud.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
                                    let when = cloud::ago(now, std::time::UNIX_EPOCH + std::time::Duration::from_secs(c.used));
                                    let note = if c.pending { "not uploaded yet" } else { "" };
                                    if design_row(ui, &name, note, Some(&when), true, true).clicked() {
                                        open = Some(c.cloud.clone());
                                    }
                                }
                            }
                            Some(Ok(list)) if list.is_empty() => {
                                ui.weak("    No designs here yet — save one into this folder (File > Save to Cloud Folder).");
                            }
                            Some(Ok(list)) => {
                                for d in list.iter().take(SHOWN) {
                                    let when = d.modified.map(|t| cloud::ago(now, t));
                                    if design_row(ui, &d.name, &d.folder, when.as_deref(), true, false).clicked() {
                                        open = Some(d.path.clone());
                                    }
                                }
                                if list.len() > SHOWN {
                                    ui.weak(format!("    and {} more (Open… to find them)", list.len() - SHOWN));
                                }
                            }
                        }
                    }
                    ui.add_space(40.0);
                });
            });
        });
        if let Some(k) = remove {
            self.cloud_folders.remove(k);
            self.scan_cloud(&ctx);
        }
        if let Some(p) = open {
            self.open_from_start(p);
        }
    }
}

/// A big icon button with its name under it.
fn big_button(ui: &mut egui::Ui, icon: Icon, label: &str, tip: &str) -> egui::Response {
    let size = egui::vec2(150.0, 110.0);
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    let pal = theme::now();
    let fill = if r.hovered() { pal.raised } else { pal.panel };
    ui.painter().rect_filled(rect, 8.0, fill);
    let icon_rect = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 14.0), egui::vec2(52.0, 52.0));
    icons::paint(ui.painter(), icon_rect, icon, true);
    ui.painter().text(rect.center_bottom() - egui::vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(15.0), pal.text);
    r.on_hover_text(tip)
}

/// One design in a list: its name, where it is and when it was saved.
fn design_row(ui: &mut egui::Ui, name: &str, place: &str, when: Option<&str>, there: bool, cloudy: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, r) = ui.allocate_exact_size(egui::vec2(width, 30.0), if there { egui::Sense::click() } else { egui::Sense::hover() });
    let pal = theme::now();
    if there && r.hovered() {
        ui.painter().rect_filled(rect, 4.0, pal.raised);
    }
    let icon = egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 4.0), egui::vec2(22.0, 22.0));
    icons::paint(ui.painter(), icon, if cloudy { Icon::Cloud } else { Icon::File }, there);
    let ink = if there { pal.text } else { pal.weak };
    let y = rect.center().y;
    let galley = ui.painter().layout_no_wrap(name.to_string(), egui::FontId::proportional(15.0), ink);
    let name_w = galley.size().x;
    ui.painter().galley(egui::pos2(rect.min.x + 38.0, y - galley.size().y / 2.0), galley, ink);
    let place = if there { place.to_string() } else { format!("{place}  (moved or deleted)") };
    ui.painter().text(egui::pos2(rect.min.x + 52.0 + name_w, y), egui::Align2::LEFT_CENTER, place, egui::FontId::proportional(13.0), pal.weak);
    if let Some(w) = when {
        ui.painter().text(egui::pos2(rect.max.x - 8.0, y), egui::Align2::RIGHT_CENTER, w, egui::FontId::proportional(13.0), pal.weak);
    }
    r
}

// ---------- the Save window ----------

impl OpenCad {
    /// Where designs can be saved: the cloud folders (greyed when the start
    /// page found one can't be reached), then this computer.
    pub(crate) fn save_roots(&self) -> Vec<save_dialog::Root> {
        let mut roots: Vec<save_dialog::Root> = self
            .cloud_folders
            .iter()
            .enumerate()
            .map(|(k, f)| match self.cloud_lists.get(k) {
                Some(Some(Err(_))) => save_dialog::Root { name: f.name.clone(), path: None, cloud: true, note: Some("can't be reached".into()) },
                _ => save_dialog::Root { name: f.name.clone(), path: Some(f.path.clone()), cloud: true, note: None },
            })
            .collect();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let docs = home.as_ref().map(|h| h.join("Documents")).filter(|d| d.is_dir()).or(home);
        roots.push(save_dialog::Root { name: "This computer".into(), path: docs, cloud: false, note: None });
        // A design saved somewhere else on this computer: its own folder too.
        if let Some(dir) = self.path.as_deref().and_then(std::path::Path::parent) {
            if !roots.iter().any(|r| r.path.as_deref().is_some_and(|p| dir.starts_with(p))) {
                let name = dir.file_name().map_or_else(|| dir.display().to_string(), |n| n.to_string_lossy().into_owned());
                roots.push(save_dialog::Root { name, path: Some(dir.to_path_buf()), cloud: false, note: None });
            }
        }
        roots
    }

    /// Open the Save window (in cloud folder `root` if given), `then` to do once saved.
    pub(crate) fn open_save_dialog(&mut self, then: Option<Pending>, root: Option<usize>) {
        let roots = self.save_roots();
        let (k, sub) = match root {
            Some(k) => (k, PathBuf::new()),
            None => save_dialog::SaveDialog::place_for(&roots, self.path.as_deref()),
        };
        let mut d = save_dialog::SaveDialog::new(&self.design_name(), k, sub);
        d.then = then;
        self.save_dialog = Some(d);
        self.save_listing = None;
    }

    pub(crate) fn save_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.save_dialog.take() else { return };
        let roots = self.save_roots();
        // What's in the folder shown: read in the background when it changes.
        let folder = d.folder(&roots);
        if let Some(rx) = &self.save_listing_rx {
            if let Ok((dir, l)) = rx.try_recv() {
                if self.save_listing.as_ref().is_some_and(|(at, _)| *at == dir) {
                    self.save_listing = Some((dir, Some(l)));
                }
                self.save_listing_rx = None;
            }
        }
        match &folder {
            Some(dir) if self.save_listing.as_ref().is_none_or(|(at, _)| at != dir) => {
                self.save_listing = Some((dir.clone(), None));
                let (tx, rx) = std::sync::mpsc::channel();
                let (dir, ctx) = (dir.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let l = save_dialog::read(&dir);
                    let _ = tx.send((dir, l));
                    ctx.request_repaint();
                });
                self.save_listing_rx = Some(rx);
            }
            None => self.save_listing = None,
            _ => {}
        }
        let listing = self.save_listing.as_ref().and_then(|(_, l)| l.as_ref());
        let current = self.path.clone();
        let mut act = save_dialog::Action::None;
        let mut open = true;
        egui::Window::new("Save")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ctx, |ui| act = d.panel(ui, &roots, listing, current.as_deref()));
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            act = save_dialog::Action::Cancel;
        }
        match act {
            save_dialog::Action::None => self.save_dialog = Some(d),
            save_dialog::Action::Cancel => {}
            save_dialog::Action::Browse => {
                self.save_as_in(folder);
            }
            save_dialog::Action::MakeFolder(p) => {
                if let Err(e) = std::fs::create_dir_all(&p) {
                    self.alert = Some(format!("Couldn't make the folder {}: {e}", p.display()));
                    d.up();
                }
                self.save_listing = None;
                self.save_dialog = Some(d);
            }
            save_dialog::Action::Save(target) => {
                if self.write_to(target, false) {
                    if let Some(then) = d.then {
                        self.run(then, ctx);
                    }
                }
            }
        }
    }
}
