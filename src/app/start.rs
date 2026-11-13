//! The start page: shown on an empty new design when the app opens (and from
//! File → Start Page). New and Open, the designs opened lately, and the
//! designs in the user's cloud folders (their own TrueNAS / Samba share or
//! Nextcloud folder — see `cloud`).

use crate::*;
use icons::Icon;


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
            // Designs kept on this computer on purpose: brought up to date.
            if let Some(c) = cloud::Cache::default_cache() {
                c.refresh_kept();
            }
        });
        self.cloud_scan = Some(rx);
    }

    /// Take in what the background look has found so far.
    pub(crate) fn poll_cloud(&mut self) {
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

    /// Add a cloud folder: the Connect your cloud window.
    pub(crate) fn add_cloud_folder(&mut self, _ctx: &egui::Context) {
        if self.connect.is_none() {
            self.connect = Some(Connect::default());
        }
    }

    /// The Connect your cloud window: a server's address (mounted through
    /// KDE or GNOME, in the background), or a folder on this computer (a
    /// Nextcloud sync folder).
    pub(crate) fn connect_window(&mut self, ctx: &egui::Context) {
        let Some(mut c) = self.connect.take() else { return };
        // A try under way: done?
        if let Some((url, rx)) = &c.busy {
            match rx.try_recv() {
                Ok(Ok(path)) => {
                    let name = if c.name.trim().is_empty() { cloud::url_name(url) } else { c.name.trim().to_string() };
                    if !self.cloud_folders.iter().any(|f| f.path == path) {
                        self.cloud_folders.push(cloud::CloudFolder { name: name.clone(), path, url: Some(url.clone()) });
                    }
                    self.scan_cloud(ctx);
                    self.note_saved(format!("Connected to {name}"), true, true);
                    return; // (closed)
                }
                Ok(Err(e)) => {
                    c.error = Some(e);
                    c.busy = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => c.busy = None,
            }
        }
        let mut close = false;
        let mut local = false;
        egui::Modal::new(egui::Id::new("connect_cloud")).show(ctx, |ui| {
            ui.set_width(480.0);
            ui.heading("Connect your cloud");
            ui.label("Keep your designs on your own server: a TrueNAS or other Samba share, or Nextcloud.");
            ui.add_space(6.0);
            egui::Grid::new("connect_grid").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
                ui.label("Address");
                ui.add(egui::TextEdit::singleline(&mut c.address).hint_text("smb://192.168.1.20/share/folder").desired_width(340.0));
                ui.end_row();
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut c.name).hint_text(if c.address.is_empty() { "Server name".to_string() } else { normalize_or(&c.address) }).desired_width(340.0));
                ui.end_row();
            });
            ui.weak("TrueNAS / Samba: smb://server/share/folder (the server's name or IP).");
            ui.weak("Nextcloud: webdavs://your.cloud/remote.php/dav/files/USER/folder");
            ui.weak("Your password is asked for (and remembered) by the desktop the first time.");
            if let Some(e) = &c.error {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let busy = c.busy.is_some();
                if busy {
                    ui.spinner();
                    ui.label("Connecting…");
                } else if ui.button("Connect").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    match cloud::normalize_url(&c.address) {
                        Ok(url) => {
                            let (tx, rx) = std::sync::mpsc::channel();
                            let (u, ctx) = (url.clone(), ctx.clone());
                            std::thread::spawn(move || {
                                let _ = tx.send(cloud::connect(&u));
                                ctx.request_repaint();
                            });
                            c.busy = Some((url, rx));
                            c.error = None;
                        }
                        Err(e) => c.error = Some(e),
                    }
                }
                if ui.button("Use a Folder on This Computer…").on_hover_text("A folder the Nextcloud app (or another sync app) keeps in step, or a share mounted already").clicked() {
                    local = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
        if local {
            // (The system's chooser, off the main thread: `add_local_cloud_folder` when picked.)
            let d = rfd::FileDialog::new().set_title("A folder for your designs (synced, or on a share)");
            self.ask_path(d, PickKind::Folder, PickFor::CloudFolder, c.name.trim().to_string());
            return;
        }
        if !close {
            self.connect = Some(c);
        }
    }

    /// Open `p` from the start page (in this tab: it's an empty design).
    pub(crate) fn open_from_start(&mut self, p: PathBuf) {
        self.open_in_tab = Some(p);
        self.tab_request = Some(TabRequest::Open);
    }

}

impl OpenCad {
    /// A folder on this computer picked as a cloud folder (`name` if given).
    pub(crate) fn add_local_cloud_folder(&mut self, dir: PathBuf, name: &str) {
        let mut f = cloud::CloudFolder::at(dir);
        if !name.is_empty() {
            f.name = name.to_string();
        }
        if !self.cloud_folders.iter().any(|x| x.path == f.path) {
            let n = f.name.clone();
            self.cloud_folders.push(f);
            self.note_saved(format!("Added {n}"), true, true);
        }
        self.cloud_status.checked = None;
        self.cloud_lists.clear();
    }
}

/// The name an address would give (for the Name box's hint).
fn normalize_or(address: &str) -> String {
    cloud::normalize_url(address).map(|u| cloud::url_name(&u)).unwrap_or_default()
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
            .map(|(k, f)| match self.folder_away(k) {
                // (Still a place to save: kept on this computer, uploaded when it's back.)
                true => save_dialog::Root { name: f.name.clone(), path: Some(f.path.clone()), cloud: true, note: Some("offline".into()) },
                false => save_dialog::Root { name: f.name.clone(), path: Some(f.path.clone()), cloud: true, note: None },
            })
            .collect();
        // This computer: Documents/Oxide360 (made the first time it's needed).
        let local = cloud::local_dir();
        if let Some(d) = &local {
            let _ = std::fs::create_dir_all(d);
        }
        roots.push(save_dialog::Root { name: settings::THIS_COMPUTER.into(), path: local, cloud: false, note: None });
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
            None => {
                // A new design: where Settings says (by name), else the cloud first.
                let chosen = (self.path.is_none() && !self.save_place.is_empty()).then(|| roots.iter().position(|r| r.name == self.save_place && r.path.is_some())).flatten();
                match chosen {
                    Some(k) => (k, PathBuf::new()),
                    None => save_dialog::SaveDialog::place_for(&roots, self.path.as_deref()),
                }
            }
        };
        let mut d = save_dialog::SaveDialog::new(&self.design_name(), k, sub);
        d.then = then;
        self.save_dialog = Some(d);
        self.save_listing = None;
    }

    pub(crate) fn save_window(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.save_dialog.take() else { return };
        let mut roots = self.save_roots();
        let now = ctx.input(|i| i.time);
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
                self.save_listing_since = now;
                let (tx, rx) = std::sync::mpsc::channel();
                let (dir, c) = (dir.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let l = save_dialog::read(&dir);
                    let _ = tx.send((dir, l));
                    c.request_repaint();
                });
                self.save_listing_rx = Some(rx);
                ctx.request_repaint_after(std::time::Duration::from_secs(4));
            }
            None => self.save_listing = None,
            _ => {}
        }
        // A share that hasn't answered in 4 s (Wi-Fi off, the network drive
        // stuck): treated as away — the save is kept here and uploaded later.
        let silent = self.save_listing.as_ref().is_some_and(|(_, l)| l.is_none()) && now - self.save_listing_since > 4.0;
        if silent {
            if let Some(r) = roots.get_mut(d.root).filter(|r| r.cloud) {
                r.note = Some("not answering".into());
                // (The cloud folders come first in `roots`: the same number.)
                if let Some(ok) = self.cloud_status.reachable.get_mut(d.root) {
                    *ok = false;
                }
            }
        }
        let no_answer: Result<save_dialog::Listing, String> = Err("not answering".into());
        let listing = if silent { Some(&no_answer) } else { self.save_listing.as_ref().and_then(|(_, l)| l.as_ref()) };
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
                // (On a share: off the main thread; the list is read again after.)
                let c = ctx.clone();
                std::thread::spawn(move || {
                    let _ = std::fs::create_dir_all(&p);
                    c.request_repaint();
                });
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

// ---------- "Saved to …" ----------

impl OpenCad {
    /// Where `path` is, in words: "TrueNAS › keys" or "This computer › Documents/Oxide360".
    pub(crate) fn place_text(&self, path: &std::path::Path) -> String {
        let dir = path.parent().unwrap_or(path);
        if let Some(f) = self.cloud_folders.iter().find(|f| path.starts_with(&f.path)) {
            let rest = dir.strip_prefix(&f.path).map(|r| r.display().to_string()).unwrap_or_default();
            return if rest.is_empty() { f.name.clone() } else { format!("{} › {rest}", f.name) };
        }
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let shown = match home.as_ref().and_then(|h| dir.strip_prefix(h).ok()) {
            Some(r) => r.display().to_string(),
            None => dir.display().to_string(),
        };
        format!("This computer › {shown}")
    }

    /// Say where a save went (`done`: it's there; else it's on its way).
    pub(crate) fn note_saved(&mut self, text: String, cloud: bool, done: bool) {
        self.save_note = Some((text, cloud, done, f64::NAN));
    }

    /// The "Saved to …" card, bottom left over whatever's showing; it fades
    /// a few seconds after the save is done.
    pub(crate) fn save_note_ui(&mut self, ctx: &egui::Context) {
        let Some((text, cloud, done, since)) = self.save_note.as_mut() else { return };
        let now = ctx.input(|i| i.time);
        if since.is_nan() {
            *since = now;
        }
        let age = now - *since;
        let life = if *done { 5.0 } else { 60.0 };
        if age > life {
            self.save_note = None;
            return;
        }
        let alpha = ((life - age).min(1.0) as f32).clamp(0.0, 1.0);
        let (text, cloud, done) = (text.clone(), *cloud, *done);
        let pal = theme::now();
        egui::Area::new(egui::Id::new("save_note"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(14.0, -self.timeline_height - 14.0))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_opacity(alpha);
                egui::Frame::new().fill(pal.window).corner_radius(8.0).stroke(egui::Stroke::new(1.0, pal.raised)).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
                        icons::paint(ui.painter(), r, if cloud { Icon::Cloud } else { Icon::Folder }, true);
                        if done {
                            // A tick (the UI font has none), bottom right of the icon.
                            let c = r.right_bottom() + egui::vec2(-2.0, -2.0);
                            let s = egui::Stroke::new(2.5, egui::Color32::from_rgb(90, 200, 110));
                            ui.painter().line_segment([c + egui::vec2(-8.0, -4.0), c + egui::vec2(-4.0, 0.0)], s);
                            ui.painter().line_segment([c + egui::vec2(-4.0, 0.0), c + egui::vec2(3.0, -9.0)], s);
                        }
                        ui.label(egui::RichText::new(text).color(pal.text).size(14.0));
                    });
                });
            });
        ctx.request_repaint();
    }
}

impl OpenCad {
    /// Bottom right, while a cloud design is open from the copy on this
    /// computer: says so and why, until the connection is back (looked at
    /// once a second) or it's closed.
    pub(crate) fn local_open_ui(&mut self, ctx: &egui::Context) {
        let Some((p, why)) = self.local_open.clone() else { return };
        if self.path.as_ref() != Some(&p) {
            self.local_open = None;
            return;
        }
        let now = ctx.input(|i| i.time);
        let name = self.cloud_name(&p);
        if now - self.local_checked > 1.0 {
            self.local_checked = now;
            let pending = cloud::Cache::default_cache().and_then(|c| c.entry(&p)).is_some_and(|e| e.pending);
            if cloud::on_network() && !self.cloud_away(&p) && !pending {
                self.local_open = None;
                self.notify(format!("Back online: {} is up to date on {name}", p.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned())));
                return;
            }
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let pal = theme::now();
        let mut close = false;
        egui::Area::new(egui::Id::new("local_open"))
            .order(egui::Order::Middle)
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-14.0, -self.timeline_height - 14.0))
            .show(ctx, |ui| {
                egui::Frame::new().fill(pal.window).corner_radius(8.0).stroke(egui::Stroke::new(1.5, egui::Color32::from_rgb(235, 150, 60))).inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
                    ui.set_max_width(330.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::hover());
                        icons::place_badge(ui.painter(), r.center(), 22.0, crate::file_browser::Stored::Waiting);
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("Opened from this computer").strong().color(pal.text).size(14.0));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.small_button("×").on_hover_text("Hide").clicked() {
                                        close = true;
                                    }
                                });
                            });
                            ui.label(egui::RichText::new(format!("This design is on {name}. {why} You're working on the copy kept here; saves upload to {name} when it's back.")).color(pal.weak).size(13.0));
                        });
                    });
                });
            });
        if close {
            self.local_open = None;
        }
    }
}

// ---------- previews ----------

impl OpenCad {
    /// The design's preview (PNG): its shown bodies in their colours, fitted.
    pub(crate) fn make_preview(&self) -> Option<Vec<u8>> {
        let model = &self.caches[Slot::Committed as usize].as_ref()?.model;
        let bodies: Vec<(&model::Mesh, [u8; 3])> = model
            .bodies
            .iter()
            .filter(|b| !self.hidden_bodies.contains(&b.id))
            .map(|b| {
                let c = self.body_color(b.id);
                (&b.mesh, [c.r(), c.g(), c.b()])
            })
            .collect();
        (!bodies.is_empty()).then(|| preview::png(&preview::render(&bodies)))
    }
}

// ---------- the cloud icon ----------

/// How the cloud is, for the icon.
#[derive(Clone, Copy, PartialEq)]
enum Health {
    NotSetUp,
    NoNetwork,
    Away,
    Uploading,
    Good,
    Unknown,
}

impl OpenCad {
    /// Check the cloud folders in the background (every half minute, or now if `now`).
    fn cloud_check(&mut self, ctx: &egui::Context, now: bool) {
        let t = ctx.input(|i| i.time);
        let s = &mut self.cloud_status;
        if let Some(rx) = &s.rx {
            if let Ok((reachable, network, waiting)) = rx.try_recv() {
                // A folder came back (or went): the start page looks again, so both agree.
                let changed = reachable.iter().enumerate().any(|(k, &ok)| matches!(self.cloud_lists.get(k), Some(Some(Err(_)))) == ok || s.reachable.get(k).is_some_and(|&was| was != ok));
                (s.reachable, s.network, s.waiting) = (reachable, network, waiting);
                s.rx = None;
                if changed && self.cloud_scan.is_none() {
                    self.scan_cloud(ctx);
                    self.browse.listing = None;
                }
            }
        }
        let s = &mut self.cloud_status;
        let due = now || s.checked.is_none_or(|c| t - c > 30.0);
        if !due || s.rx.is_some() || self.cloud_folders.is_empty() {
            return;
        }
        s.checked = Some(t);
        let folders = self.cloud_folders.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let network = cloud::on_network();
            let reachable = folders.iter().map(|f| f.resolve().is_ok()).collect();
            let waiting = cloud::Cache::default_cache().map_or(0, |c| c.to_upload().len());
            let _ = tx.send((reachable, network, waiting));
            c.request_repaint();
        });
        s.rx = Some(rx);
        // (Keeps checking while the app is open.)
        ctx.request_repaint_after(std::time::Duration::from_secs(31));
    }

    fn health(&self) -> Health {
        let s = &self.cloud_status;
        if self.cloud_folders.is_empty() {
            Health::NotSetUp
        } else if s.checked.is_none() || s.reachable.len() != self.cloud_folders.len() {
            Health::Unknown
        } else if !s.network {
            Health::NoNetwork
        } else if s.reachable.iter().any(|r| !r) {
            Health::Away
        } else if s.waiting > 0 || self.save_note.as_ref().is_some_and(|n| n.1 && !n.2) {
            Health::Uploading
        } else {
            Health::Good
        }
    }

    /// Look at the cloud again now: the servers, the start page's lists, the ping.
    pub(crate) fn cloud_refresh(&mut self, ctx: &egui::Context) {
        self.cloud_status.pinged = None;
        self.cloud_check(ctx, true);
        self.scan_cloud(ctx);
        self.browse.listing = None;
        self.browse.projects.clear();
    }

    /// Ping the servers every 10 s while the menu is open (only then).
    fn cloud_pings(&mut self, ctx: &egui::Context) {
        let t = ctx.input(|i| i.time);
        let s = &mut self.cloud_status;
        if let Some(rx) = &s.ping_rx {
            loop {
                match rx.try_recv() {
                    Ok((host, p)) => {
                        s.pings.insert(host, p);
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        s.ping_rx = None;
                        break;
                    }
                }
            }
        }
        if s.ping_rx.is_some() || s.pinged.is_some_and(|p| t - p < 10.0) {
            return;
        }
        s.pinged = Some(t);
        let hosts: Vec<String> = self.cloud_folders.iter().filter_map(|f| f.url.as_deref().and_then(cloud::host_of)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let (tx, rx) = std::sync::mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            for h in hosts {
                let p = cloud::ping(&h);
                if tx.send((h, p)).is_err() {
                    return;
                }
                c.request_repaint();
            }
        });
        s.ping_rx = Some(rx);
        ctx.request_repaint_after(std::time::Duration::from_secs(10));
    }

    /// The cloud icon (top right, by Settings) and its menu.
    pub(crate) fn cloud_button(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.cloud_check(&ctx, false);
        let health = self.health();
        let pal = theme::now();
        let (rect, r) = ui.allocate_exact_size(egui::vec2(32.0, 26.0), egui::Sense::click());
        if r.hovered() {
            ui.painter().rect_filled(rect, 4.0, pal.raised);
        }
        let spot = egui::Rect::from_center_size(rect.center(), egui::vec2(22.0, 22.0));
        if let Some(frac) = self.upload_progress.or((health == Health::Uploading).then_some(0.0)) {
            // Uploading: a ring filling clockwise as it goes, until it's all up.
            icons::progress_ring(ui.painter(), spot, frac, egui::Color32::from_rgb(70, 145, 230), pal.raised);
        } else {
            icons::paint(ui.painter(), spot, Icon::Cloud, health != Health::NotSetUp);
        }
        let dot = match health {
            Health::Good => Some(egui::Color32::from_rgb(90, 200, 110)),
            Health::Uploading => None, // (the arrow says it)
            Health::Away => Some(egui::Color32::from_rgb(230, 90, 80)),
            Health::NoNetwork => Some(egui::Color32::from_gray(150)),
            Health::NotSetUp | Health::Unknown => None,
        };
        if let Some(c) = dot {
            let at = rect.right_bottom() - egui::vec2(8.0, 7.0);
            ui.painter().circle_filled(at, 4.5, pal.window);
            ui.painter().circle_filled(at, 3.5, c);
        }
        let tip = match health {
            Health::NotSetUp => "Cloud: not connected — click to connect your own server",
            Health::NoNetwork => "Cloud: not connected to a network — turn on Wi-Fi to reach your cloud",
            Health::Away => "Cloud: a server can't be reached",
            Health::Uploading => "Cloud: saves uploading",
            Health::Good => "Cloud: connected, everything uploaded",
            Health::Unknown => "Cloud: checking…",
        };
        let r = r.on_hover_text(tip);
        let mut connect = false;
        let mut again = false;
        let mut upload = false;
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(320.0);
            if health == Health::NotSetUp {
                ui.strong("No cloud connected");
                ui.weak("Designs save on this computer, in Documents/Oxide360.");
                ui.add_space(4.0);
                if ui.button("Connect your cloud…").clicked() {
                    connect = true;
                }
                return;
            }
            self.cloud_pings(&ctx);
            let s = &self.cloud_status;
            if !s.network && s.checked.is_some() {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 90), "Not connected to a network: turn on Wi-Fi (or plug in) to reach your cloud. Saves stay on this computer and upload when it's back.");
            }
            for (k, f) in self.cloud_folders.iter().enumerate() {
                let ok = s.reachable.get(k).copied();
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                    let c = match ok {
                        Some(true) => egui::Color32::from_rgb(90, 200, 110),
                        Some(false) => egui::Color32::from_rgb(230, 90, 80),
                        None => egui::Color32::from_gray(130),
                    };
                    ui.painter().circle_filled(dot.center(), 4.0, c);
                    ui.strong(&f.name);
                    ui.weak(match ok {
                        Some(true) => "connected",
                        Some(false) => "can't be reached",
                        None => "checking…",
                    });
                });
                let host = f.url.as_deref().and_then(cloud::host_of);
                ui.horizontal(|ui| {
                    ui.add_space(18.0);
                    ui.weak(f.url.clone().unwrap_or_else(|| f.path.display().to_string()));
                });
                if let Some(h) = host {
                    let line = match s.pings.get(&h) {
                        None => "Ping: measuring…".to_string(),
                        Some(None) => "Ping: couldn't run ping".to_string(),
                        Some(Some(p)) => match p.avg_ms {
                            Some(ms) => format!("Ping {ms:.0} ms · {:.0}% of packets lost", p.loss_pct),
                            None => format!("No answer from {h} ({:.0}% lost)", p.loss_pct),
                        },
                    };
                    ui.horizontal(|ui| {
                        ui.add_space(18.0);
                        ui.label(line);
                    });
                }
                ui.add_space(4.0);
            }
            ui.separator();
            match s.waiting {
                0 => {
                    ui.weak("Everything is uploaded.");
                }
                n => {
                    ui.horizontal(|ui| {
                        ui.label(format!("{n} save{} on this computer waiting to upload", if n == 1 { "" } else { "s" }));
                        if ui.button("Upload Now").clicked() {
                            upload = true;
                        }
                    });
                }
            }
            ui.weak("(Ping is measured every 10 s while this is open.)");
            ui.horizontal(|ui| {
                if ui.button("Check Again").clicked() {
                    again = true;
                }
                if ui.button("Connect Cloud…").clicked() {
                    connect = true;
                }
            });
        });
        if connect {
            self.add_cloud_folder(&ctx);
        }
        if again {
            self.cloud_refresh(&ctx);
        }
        if upload {
            self.sync_now = true;
            self.cloud_check(&ctx, true);
        }
    }
}
