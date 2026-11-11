//! The start page: New / Open / Add Cloud Folder, and a file viewer (as
//! Fusion's Data panel) — Recent, each cloud folder and this computer down
//! the side; projects (folders) and designs as preview tiles or a list, each
//! with its last save and a ⋯ menu (open, rename, move to a project, keep
//! offline, remove from Recent, delete). Folder contents and previews are
//! read in the background: a share can be slow or away.

use crate::*;
use file_browser::Item;
use icons::Icon;
use std::time::SystemTime;

/// A tile's side (its preview is square).
const TILE: f32 = 168.0;

/// One thing shown: a project or a design, wherever it's from.
#[derive(Clone)]
struct Shown {
    path: PathBuf,
    name: String,
    folder: bool,
    when: Option<SystemTime>,
    there: bool,
    cloudy: bool,
    /// Listed under Recent (Remove from Recent offered).
    recent: bool,
    /// Where it is, for Recent ("TrueNAS › keys").
    place: Option<String>,
}

/// What the ⋯ menu or a click asked for.
enum Ask {
    Open(PathBuf),
    Enter(String),
    /// (A project folder if true.)
    Rename(PathBuf, bool),
    MoveTo(PathBuf, PathBuf),
    Keep(PathBuf, bool),
    Forget(PathBuf),
    Delete(PathBuf),
}

impl OpenCad {
    /// Where the viewer is looking now (None: Recent, or a place that can't be reached).
    fn browse_dir(&self, roots: &[save_dialog::Root]) -> Option<PathBuf> {
        let k = self.browse.root?;
        roots.get(k)?.path.as_ref().map(|p| p.join(&self.browse.sub))
    }

    /// Read `dir` in the background (unless it's being read or was just read).
    fn browse_read(&mut self, dir: &std::path::Path, ctx: &egui::Context) {
        if self.browse.listing.as_ref().is_some_and(|(at, _)| at == dir) {
            return;
        }
        self.browse.listing = Some((dir.to_path_buf(), None));
        let (tx, rx) = std::sync::mpsc::channel();
        let (d, ctx) = (dir.to_path_buf(), ctx.clone());
        std::thread::spawn(move || {
            let l = file_browser::list(&d);
            let _ = tx.send((d, l));
            ctx.request_repaint();
        });
        self.browse.listing_rx = Some(rx);
    }

    /// Take in what the background reads found.
    fn browse_poll(&mut self, roots: &[save_dialog::Root], ctx: &egui::Context) {
        if let Some(rx) = &self.browse.listing_rx {
            if let Ok((dir, l)) = rx.try_recv() {
                // A place's top folders are its projects (for Move to).
                if let Ok(items) = &l {
                    if roots.iter().any(|r| r.path.as_ref() == Some(&dir)) {
                        self.browse.projects.insert(dir.clone(), items.iter().filter(|i| i.folder).map(|i| i.name.clone()).collect());
                    }
                }
                if self.browse.listing.as_ref().is_some_and(|(at, _)| *at == dir) {
                    self.browse.listing = Some((dir, Some(l)));
                }
                self.browse.listing_rx = None;
            }
        }
        if let Some(rx) = &self.browse.previews_rx {
            loop {
                match rx.try_recv() {
                    Ok((path, img)) => {
                        let tex = img.map(|(w, h, rgba)| ctx.load_texture(format!("preview {}", path.display()), egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba), egui::TextureOptions::LINEAR));
                        if let Some(slot) = self.browse.previews.get_mut(&path) {
                            slot.1 = tex;
                        }
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.browse.previews_rx = None;
                        break;
                    }
                }
            }
        }
    }

    /// Fetch the pictures of `items` not fetched yet (or saved again since), in the background.
    fn browse_want_previews(&mut self, items: &[Shown], ctx: &egui::Context) {
        if self.browse.previews_rx.is_some() {
            return; // (one batch at a time; the rest next time round)
        }
        let cache = cloud::Cache::default_cache();
        let mut want: Vec<(PathBuf, PathBuf)> = Vec::new();
        for it in items.iter().filter(|i| !i.folder && i.there) {
            if self.browse.previews.get(&it.path).is_some_and(|(t, _)| *t == it.when) {
                continue;
            }
            self.browse.previews.insert(it.path.clone(), (it.when, None));
            // A cloud that's away: the picture from this computer's copy.
            let from = if self.cloud_away(&it.path) { cache.as_ref().and_then(|c| c.copy_of(&it.path)) } else { None }.unwrap_or_else(|| it.path.clone());
            want.push((it.path.clone(), from));
        }
        if want.is_empty() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            for (path, from) in want {
                let img = std::fs::read_to_string(&from).ok().and_then(|t| doc::preview_of(&t)).and_then(|b| preview::decode_png(&b));
                if tx.send((path, img)).is_err() {
                    return;
                }
                ctx.request_repaint();
            }
        });
        self.browse.previews_rx = Some(rx);
    }

    /// Is `path` open in this app (its lock is this program's)? Then it isn't renamed, moved or deleted.
    fn open_here(&self, path: &std::path::Path) -> bool {
        // (From the tabs, not the share: nothing here reads the share on the main thread.)
        self.path.as_deref() == Some(path) || self.open_paths.iter().any(|p| p == path || p.starts_with(path))
    }

    pub(crate) fn start_page_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.cloud_lists.len() != self.cloud_folders.len() && self.cloud_scan.is_none() {
            self.scan_cloud(&ctx);
        }
        self.poll_cloud();
        let roots = self.save_roots();
        self.browse_poll(&roots, &ctx);
        self.browse_job_done(&ctx);
        if self.browse.root.is_some_and(|k| k >= roots.len()) {
            self.browse.root = None;
        }
        let dir = self.browse_dir(&roots);
        if let Some(d) = &dir {
            self.browse_read(d, &ctx);
            // (A place's projects, for Move to, even while looking inside one.)
            if let Some(top) = self.browse.root.and_then(|k| roots[k].path.clone()) {
                if !self.browse.projects.contains_key(&top) && self.browse.listing_rx.is_none() && !self.browse.sub.as_os_str().is_empty() {
                    let (tx, rx) = std::sync::mpsc::channel();
                    let (t, c) = (top.clone(), ctx.clone());
                    std::thread::spawn(move || {
                        let l = file_browser::list(&t);
                        let _ = tx.send((t, l));
                        c.request_repaint();
                    });
                    self.browse.listing_rx = Some(rx);
                }
            }
        }
        // Which designs are kept offline (this computer's list, read now and then).
        let t = ctx.input(|i| i.time);
        if t - self.browse.kept.0 > 2.0 {
            self.browse.kept = (t, cloud::Cache::default_cache().map_or_else(Default::default, |c| c.entries().into_iter().filter(|e| e.keep).map(|e| e.cloud).collect()));
        }
        let items = self.browse_items(&roots, dir.as_deref());
        self.browse_want_previews(&items, &ctx);
        let pal = theme::now();
        ui.painter().rect_filled(ui.max_rect(), 0.0, pal.view);
        let mut ask: Option<Ask> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let width = ui.available_width().min(1200.0);
            let side = ((ui.available_width() - width) / 2.0).max(16.0);
            ui.add_space(28.0);
            ui.horizontal(|ui| {
                ui.add_space(side);
                ui.vertical(|ui| {
                    ui.set_width(width - 32.0);
                    ui.horizontal(|ui| {
                        ui.heading(egui::RichText::new("Oxide360").size(28.0));
                    });
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if big_button(ui, Icon::NewDesign, "New Design", "Start an empty design (Ctrl+N)").clicked() {
                            self.start_page = false;
                        }
                        ui.add_space(12.0);
                        if big_button(ui, Icon::Cloud, "Connect Cloud…", "Your own server (TrueNAS / Samba share, Nextcloud): its designs show here, and new ones save there").clicked() {
                            self.add_cloud_folder(&ctx);
                        }
                    });
                    // No network at all: say what to do.
                    if !self.cloud_folders.is_empty() && self.cloud_status.checked.is_some() && !self.cloud_status.network {
                        ui.add_space(14.0);
                        let names: Vec<&str> = self.cloud_folders.iter().map(|f| f.name.as_str()).collect();
                        egui::Frame::new().fill(egui::Color32::from_rgb(70, 45, 30)).corner_radius(6.0).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
                            ui.label(egui::RichText::new(format!("This computer isn't connected to a network. Turn on Wi-Fi (or plug in a cable) to reach {}. Until then you can work on the copies kept here; saves upload when it's back.", names.join(" and "))).color(egui::Color32::from_rgb(255, 220, 190)));
                        });
                    }
                    ui.add_space(22.0);
                    ui.horizontal_top(|ui| {
                        self.browse_sidebar(ui, &roots);
                        ui.add_space(14.0);
                        ui.vertical(|ui| {
                            ask = self.browse_content(ui, &roots, dir.as_deref(), &items);
                        });
                    });
                    ui.add_space(40.0);
                });
            });
        });
        if let Some(a) = ask {
            self.browse_do(a, &roots, &ctx);
        }
        self.browse_edit_window(&ctx, &roots);
    }

    /// What's shown: Recent's designs, or the folder's projects and designs
    /// (or, for a cloud that's away, this computer's copies of its designs).
    fn browse_items(&self, roots: &[save_dialog::Root], dir: Option<&std::path::Path>) -> Vec<Shown> {
        match self.browse.root {
            None => self
                .recent
                .iter()
                .map(|p| {
                    let seen = self.recent_seen.iter().find(|s| s.0 == *p);
                    Shown {
                        path: p.clone(),
                        name: p.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                        folder: false,
                        when: seen.and_then(|s| s.2),
                        there: seen.is_none_or(|s| s.1),
                        cloudy: cloud::in_cloud(&self.cloud_folders, p),
                        recent: true,
                        place: Some(self.place_text(p)),
                    }
                })
                .collect(),
            Some(k) => {
                let cloudy = roots.get(k).is_some_and(|r| r.cloud);
                match self.browse.listing.as_ref().filter(|(at, _)| Some(at.as_path()) == dir).and_then(|(_, l)| l.as_ref()) {
                    Some(Ok(list)) => list
                        .iter()
                        .map(|i: &Item| Shown { path: i.path.clone(), name: i.name.clone(), folder: i.folder, when: i.modified, there: true, cloudy, recent: self.recent.contains(&i.path), place: None })
                        .collect(),
                    _ if dir.is_none() || self.browse.listing.as_ref().is_some_and(|(_, l)| matches!(l, Some(Err(_)))) => {
                        // Away: the copies on this computer.
                        let folder = self.cloud_folders.get(k).map(|f| f.path.clone());
                        cloud::Cache::default_cache()
                            .map_or_else(Vec::new, |c| c.entries())
                            .into_iter()
                            .filter(|e| folder.as_ref().is_some_and(|f| e.cloud.starts_with(f)))
                            .map(|e| Shown {
                                name: e.cloud.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                                path: e.cloud.clone(),
                                folder: false,
                                when: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(e.used)),
                                there: true,
                                cloudy: true,
                                recent: false,
                                place: Some(if e.pending { "on this computer, not uploaded yet".into() } else { "copy on this computer".into() }),
                            })
                            .collect()
                    }
                    _ => Vec::new(),
                }
            }
        }
    }

    fn browse_sidebar(&mut self, ui: &mut egui::Ui, roots: &[save_dialog::Root]) {
        ui.vertical(|ui| {
            ui.set_width(190.0);
            let pal = theme::now();
            let mut pick: Option<Option<usize>> = None;
            let row = |ui: &mut egui::Ui, icon: Icon, label: &str, on: bool, note: Option<&str>| {
                let (rect, r) = ui.allocate_exact_size(egui::vec2(190.0, 32.0), egui::Sense::click());
                if on || r.hovered() {
                    ui.painter().rect_filled(rect, 5.0, if on { pal.accent.gamma_multiply(0.45) } else { pal.raised });
                }
                icons::paint(ui.painter(), egui::Rect::from_min_size(rect.min + egui::vec2(6.0, 5.0), egui::vec2(22.0, 22.0)), icon, true);
                ui.painter().text(rect.min + egui::vec2(36.0, 16.0), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(15.0), pal.text);
                let r = match note {
                    Some(n) => {
                        ui.painter().circle_filled(rect.right_center() - egui::vec2(12.0, 0.0), 4.0, egui::Color32::from_rgb(230, 120, 90));
                        r.on_hover_text(n.to_string())
                    }
                    None => r,
                };
                r.clicked()
            };
            if row(ui, Icon::File, "Recent", self.browse.root.is_none(), None) {
                pick = Some(None);
            }
            ui.add_space(6.0);
            for (k, r) in roots.iter().enumerate() {
                let away = r.note.as_deref().map(|n| format!("{} {n} — this computer's copies of its designs are shown", r.name));
                if row(ui, if r.cloud { Icon::Cloud } else { Icon::Folder }, &r.name, self.browse.root == Some(k), away.as_deref()) {
                    pick = Some(Some(k));
                }
            }
            if let Some(p) = pick {
                self.browse.root = p;
                self.browse.sub = PathBuf::new();
                self.browse.listing = None;
            }
        });
    }

    fn browse_content(&mut self, ui: &mut egui::Ui, roots: &[save_dialog::Root], dir: Option<&std::path::Path>, items: &[Shown]) -> Option<Ask> {
        let pal = theme::now();
        let mut ask = None;
        // Where: Recent, or the place › its folders (click one to go back up).
        ui.horizontal(|ui| {
            match self.browse.root {
                None => {
                    ui.label(egui::RichText::new("Recent").size(18.0).strong());
                }
                Some(k) => {
                    // The place and the folders down to here, each a box to go back up to.
                    let name = roots.get(k).map_or_else(String::new, |r| r.name.clone());
                    let parts: Vec<String> = self.browse.sub.iter().map(|c| c.to_string_lossy().into_owned()).collect();
                    let crumb = |ui: &mut egui::Ui, text: &str, here: bool| {
                        let t = egui::RichText::new(text).size(17.0).strong();
                        let b = if here { egui::Button::selectable(true, t) } else { egui::Button::new(t).fill(pal.raised) }.corner_radius(6.0).min_size(egui::vec2(0.0, 30.0));
                        let r = ui.add(b);
                        if here { r } else { r.on_hover_text(format!("Back to {text}")) }
                    };
                    if crumb(ui, &name, parts.is_empty()).clicked() {
                        self.browse.sub = PathBuf::new();
                    }
                    for (n, part) in parts.iter().enumerate() {
                        ui.label(egui::RichText::new("›").size(18.0).color(pal.weak));
                        if crumb(ui, part, n + 1 == parts.len()).clicked() {
                            self.browse.sub = parts[..=n].iter().collect();
                        }
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.selectable_value(&mut self.browse.list_view, true, "List").on_hover_text("Designs as a list");
                ui.selectable_value(&mut self.browse.list_view, false, "Tiles").on_hover_text("Designs as pictures");
                if icons::button_sized(ui, Icon::Refresh, "Refresh: look at your cloud folders and this list again", true, false, 28.0).clicked() {
                    self.cloud_refresh(ui.ctx());
                }
                if ui.button("Other File…").on_hover_text("Open a design kept somewhere else (the system's chooser)").clicked() {
                    self.open_dialog();
                }
                if let (Some(_), Some(d)) = (self.browse.root, dir) {
                    if ui.button("New Project").on_hover_text("A folder to keep designs together").clicked() {
                        self.browse.edit = Some(BrowseEdit::NewProject(d.to_path_buf(), String::new()));
                    }
                }
            });
        });
        ui.separator();
        // Why it's empty, or can't be read.
        let state = self.browse.listing.as_ref().filter(|(at, _)| Some(at.as_path()) == dir).map(|(_, l)| l);
        match (self.browse.root, state) {
            (Some(_), Some(None)) => {
                ui.weak("Looking…");
            }
            (Some(k), Some(Some(Err(e)))) => {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 90), e);
                if !items.is_empty() {
                    ui.weak(format!("{} can't be reached: these are the copies on this computer (saves upload when it's back).", roots[k].name));
                }
            }
            (Some(k), None) if dir.is_none() => {
                ui.colored_label(egui::Color32::from_rgb(230, 120, 90), format!("{} can't be reached", roots[k].name));
            }
            _ if items.is_empty() => {
                ui.weak(match self.browse.root {
                    None => "Designs you open or save show here.",
                    Some(_) => "Nothing here yet — save a design here, or make a New Project.",
                });
            }
            _ => {}
        }
        ui.add_space(6.0);
        if self.browse.list_view {
            for it in items {
                if let Some(a) = self.item_row(ui, it, roots) {
                    ask = Some(a);
                }
            }
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(14.0, 14.0);
                for it in items {
                    if let Some(a) = self.item_tile(ui, it, roots) {
                        ask = Some(a);
                    }
                }
            });
        }
        ask
    }

    /// The picture for a design (or a big icon).
    fn paint_preview(&self, ui: &egui::Ui, rect: egui::Rect, it: &Shown) {
        let tex = self.browse.previews.get(&it.path).and_then(|(_, t)| t.clone());
        match (it.folder, tex) {
            (false, Some(t)) => {
                ui.painter().image(t.id(), rect.shrink(rect.width() * 0.04), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), if it.there { egui::Color32::WHITE } else { egui::Color32::from_gray(120) });
            }
            (folder, _) => {
                let r = egui::Rect::from_center_size(rect.center(), rect.size() * 0.42);
                icons::paint(ui.painter(), r, if folder { Icon::Folder } else { Icon::Part }, it.there);
            }
        }
    }

    fn item_tile(&self, ui: &mut egui::Ui, it: &Shown, roots: &[save_dialog::Root]) -> Option<Ask> {
        let pal = theme::now();
        let (rect, r) = ui.allocate_exact_size(egui::vec2(TILE, TILE + 50.0), egui::Sense::click());
        let hot = r.hovered();
        ui.painter().rect_filled(rect, 8.0, if hot { pal.raised } else { pal.panel });
        let pic = egui::Rect::from_min_size(rect.min, egui::vec2(TILE, TILE));
        self.paint_preview(ui, pic, it);
        let ink = if it.there { pal.text } else { pal.weak };
        let name = ui.painter().layout(it.name.clone(), egui::FontId::proportional(14.0), ink, TILE - 16.0);
        let name_h = name.size().y.min(18.0);
        ui.painter().with_clip_rect(egui::Rect::from_min_size(egui::pos2(rect.min.x + 8.0, pic.max.y + 4.0), egui::vec2(TILE - 16.0, name_h))).galley(egui::pos2(rect.min.x + 8.0, pic.max.y + 4.0), name, ink);
        let sub = self.item_note(it);
        ui.painter().text(egui::pos2(rect.min.x + 8.0, pic.max.y + 31.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.0), pal.weak);
        let tip = it.place.clone().unwrap_or_else(|| it.path.display().to_string());
        let r = r.on_hover_text(tip);
        // ⋯ in the corner.
        let dots = egui::Rect::from_min_size(egui::pos2(rect.max.x - 30.0, rect.min.y + 6.0), egui::vec2(24.0, 22.0));
        let menu = self.item_menu(ui, it, roots, dots, hot);
        if menu.is_some() {
            return menu;
        }
        (r.clicked() && it.there).then(|| if it.folder { Ask::Enter(it.name.clone()) } else { Ask::Open(it.path.clone()) })
    }

    fn item_row(&self, ui: &mut egui::Ui, it: &Shown, roots: &[save_dialog::Root]) -> Option<Ask> {
        let pal = theme::now();
        let (rect, r) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), egui::Sense::click());
        let hot = r.hovered();
        if hot {
            ui.painter().rect_filled(rect, 5.0, pal.raised);
        }
        self.paint_preview(ui, egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 3.0), egui::vec2(40.0, 40.0)), it);
        let ink = if it.there { pal.text } else { pal.weak };
        ui.painter().text(rect.min + egui::vec2(56.0, 15.0), egui::Align2::LEFT_CENTER, &it.name, egui::FontId::proportional(15.0), ink);
        let place = it.place.clone().map(|p| format!("{p}   ")).unwrap_or_default();
        ui.painter().text(rect.min + egui::vec2(56.0, 33.0), egui::Align2::LEFT_CENTER, format!("{place}{}", self.item_note(it)), egui::FontId::proportional(12.0), pal.weak);
        let dots = egui::Rect::from_min_size(egui::pos2(rect.max.x - 34.0, rect.center().y - 11.0), egui::vec2(26.0, 22.0));
        let menu = self.item_menu(ui, it, roots, dots, true);
        if menu.is_some() {
            return menu;
        }
        (r.clicked() && it.there).then(|| if it.folder { Ask::Enter(it.name.clone()) } else { Ask::Open(it.path.clone()) })
    }

    /// "Saved 5 min ago", "Project", "moved or deleted".
    fn item_note(&self, it: &Shown) -> String {
        if !it.there {
            return "moved or deleted".into();
        }
        if it.folder {
            return "Project".into();
        }
        match it.when {
            Some(t) => format!("Saved {}", cloud::ago(SystemTime::now(), t)),
            None => String::new(),
        }
    }

    /// The ⋯ button at `at` and its menu.
    fn item_menu(&self, ui: &mut egui::Ui, it: &Shown, roots: &[save_dialog::Root], at: egui::Rect, show: bool) -> Option<Ask> {
        let pal = theme::now();
        let id = ui.id().with(("dots", &it.path));
        let r = ui.interact(at, id, egui::Sense::click()).on_hover_text("More");
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r));
        if show || open || r.hovered() {
            if r.hovered() || open {
                ui.painter().rect_filled(at, 4.0, pal.card);
            }
            for k in 0..3 {
                ui.painter().circle_filled(at.center() + egui::vec2((k as f32 - 1.0) * 5.5, 0.0), 1.8, pal.text);
            }
        }
        let mut ask = None;
        egui::Popup::menu(&r).show(|ui| {
            if it.there && ui.button(if it.folder { "Open Project" } else { "Open" }).clicked() {
                ask = Some(if it.folder { Ask::Enter(it.name.clone()) } else { Ask::Open(it.path.clone()) });
            }
            let busy = self.open_here(&it.path);
            if it.there {
                if ui.add_enabled(!busy, egui::Button::new("Rename…")).on_disabled_hover_text("It's open here: close it first").clicked() {
                    ask = Some(Ask::Rename(it.path.clone(), it.folder));
                }
                // Move to another project of the same place.
                if !it.folder {
                    if let Some(top) = roots.iter().filter_map(|r| r.path.as_ref()).find(|p| it.path.starts_with(p)) {
                        let here = it.path.parent().map(std::path::Path::to_path_buf);
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.menu_button("Move to", |ui| {
                                let mut dests: Vec<(String, PathBuf)> = vec![(format!("{} (top)", roots.iter().find(|r| r.path.as_ref() == Some(top)).map_or("", |r| r.name.as_str())), top.clone())];
                                for p in self.browse.projects.get(top).into_iter().flatten() {
                                    dests.push((p.clone(), top.join(p)));
                                }
                                for (label, d) in dests {
                                    if here.as_ref() != Some(&d) && ui.button(label).clicked() {
                                        ask = Some(Ask::MoveTo(it.path.clone(), d));
                                    }
                                }
                                if self.browse.projects.get(top).is_none_or(|p| p.is_empty()) {
                                    ui.weak("(no projects yet: New Project makes one)");
                                }
                            });
                        });
                    }
                }
            }
            if it.cloudy && !it.folder {
                let kept = self.browse.kept.1.contains(&it.path);
                let mut on = kept;
                if ui.checkbox(&mut on, "Available offline").on_hover_text("Keep it on this computer too, to work on with no connection (kept up to date)").changed() {
                    ask = Some(Ask::Keep(it.path.clone(), on));
                }
            }
            if it.recent && ui.button("Remove from Recent").clicked() {
                ask = Some(Ask::Forget(it.path.clone()));
            }
            if it.there {
                ui.separator();
                if ui.add_enabled(!busy, egui::Button::new("Delete…")).on_disabled_hover_text("It's open here: close it first").clicked() {
                    ask = Some(Ask::Delete(it.path.clone()));
                }
            }
        });
        ask
    }

    /// Do `work` on the files off the main thread (a share can be slow); it
    /// gets Recent to keep up to date, and says what it did.
    fn browse_job(&mut self, ctx: &egui::Context, work: impl FnOnce(&mut Vec<PathBuf>) -> (Result<String, String>, Option<PathBuf>) + Send + 'static) {
        let mut recent = self.recent.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let (r, retry) = work(&mut recent);
            let _ = tx.send((r, recent, retry));
            ctx.request_repaint();
        });
        self.browse.job = Some(rx);
    }

    /// A file job finished: Recent as it left it, and what it said.
    fn browse_job_done(&mut self, ctx: &egui::Context) {
        let Some(rx) = &self.browse.job else { return };
        let Ok((r, recent, retry)) = rx.try_recv() else { return };
        self.browse.job = None;
        self.recent = recent;
        match (r, retry) {
            (_, Some(p)) => self.browse.edit = Some(BrowseEdit::Delete(p, true)),
            (Ok(msg), None) => self.note_saved(msg, true, true),
            (Err(e), None) => self.alert = Some(e),
        }
        self.browse_refresh(ctx);
    }

    fn browse_do(&mut self, a: Ask, roots: &[save_dialog::Root], ctx: &egui::Context) {
        let _ = roots;
        match a {
            Ask::Open(p) => self.open_from_start(p),
            Ask::Enter(name) => {
                self.browse.sub.push(name);
            }
            Ask::Rename(p, folder) => {
                let name = if folder { p.file_name() } else { p.file_stem() }.map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                self.browse.edit = Some(BrowseEdit::Rename(p, name));
            }
            Ask::MoveTo(p, d) => {
                let place = self.place_text(&d.join("x"));
                self.browse_job(ctx, move |recent| {
                    let r = file_browser::move_to(&p, &d, cloud::Cache::default_cache().as_ref(), recent);
                    (r.map(|_| format!("Moved to {place}")).map_err(|e| format!("Couldn't move it: {e}")), None)
                });
            }
            Ask::Keep(p, on) => {
                // (Copying from the share can take a moment: in the background.)
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    if let Some(c) = cloud::Cache::default_cache() {
                        let _ = cloud::set_kept(&c, &p, on);
                    }
                    ctx.request_repaint();
                });
            }
            Ask::Forget(p) => {
                self.recent.retain(|r| *r != p);
                // (The settings file is what other windows read Recent from.)
                if let Some(path) = settings::default_path() {
                    let mut s = settings::load(&path);
                    s.recent.retain(|r| *r != p);
                    let _ = settings::save(&path, &s);
                }
            }
            Ask::Delete(p) => self.browse.edit = Some(BrowseEdit::Delete(p, false)),
        }
    }

    /// Read the folder (and Recent) again after a change.
    fn browse_refresh(&mut self, ctx: &egui::Context) {
        self.browse.listing = None;
        self.browse.projects.clear();
        self.scan_cloud(ctx);
    }

    /// Rename / New Project / Delete, asked in a small window.
    fn browse_edit_window(&mut self, ctx: &egui::Context, roots: &[save_dialog::Root]) {
        let _ = roots;
        let Some(edit) = self.browse.edit.clone() else { return };
        let mut done: Option<Option<BrowseEdit>> = None; // Some(None): close
        let mut job: Option<(BrowseEdit, String)> = None; // what to do, off the main thread
        let (title, keep) = match &edit {
            BrowseEdit::Rename(..) => ("Rename", true),
            BrowseEdit::NewProject(..) => ("New Project", true),
            BrowseEdit::Delete(..) => ("Delete", false),
        };
        let _ = keep;
        egui::Modal::new(egui::Id::new("browse_edit")).show(ctx, |ui| {
            ui.heading(title);
            match &edit {
                BrowseEdit::Rename(p, name) | BrowseEdit::NewProject(p, name) => {
                    let mut text = name.clone();
                    let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(280.0).hint_text("Name"));
                    r.request_focus();
                    if text != *name {
                        done = Some(Some(match &edit {
                            BrowseEdit::Rename(..) => BrowseEdit::Rename(p.clone(), text.clone()),
                            _ => BrowseEdit::NewProject(p.clone(), text.clone()),
                        }));
                    }
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        let ok = save_dialog::clean_name(&text).is_ok();
                        if (ui.add_enabled(ok, egui::Button::new("OK")).clicked() || (enter && ok)) && done.is_none() {
                            job = Some((edit.clone(), text.clone()));
                            done = Some(None);
                        }
                        if ui.button("Cancel").clicked() {
                            done = Some(None);
                        }
                    });
                }
                BrowseEdit::Delete(p, no_trash) => {
                    let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                    if *no_trash {
                        ui.label(format!("{name} can't go to the trash there. Delete it for good? This can't be undone."));
                    } else {
                        ui.label(format!("Delete {name}? It goes to the trash."));
                    }
                    ui.horizontal(|ui| {
                        if ui.button(if *no_trash { "Delete for Good" } else { "Delete" }).clicked() {
                            job = Some((edit.clone(), String::new()));
                            done = Some(None);
                        }
                        if ui.button("Cancel").clicked() {
                            done = Some(None);
                        }
                    });
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(None);
            }
        });
        if let Some((what, text)) = job {
            self.browse_job(ctx, move |recent| {
                let cache = cloud::Cache::default_cache();
                match what {
                    BrowseEdit::Rename(p, _) => (file_browser::rename(&p, &text, cache.as_ref(), recent).map(|to| format!("Renamed to {}", to.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned()))), None),
                    BrowseEdit::NewProject(p, _) => (file_browser::new_project(&p, &text).map(|_| format!("Made the project {}", text.trim())), None),
                    BrowseEdit::Delete(p, for_good) => {
                        let how = if for_good { file_browser::Delete::ForGood } else { file_browser::Delete::Trash };
                        let name = p.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        match file_browser::delete(&p, how, cache.as_ref()) {
                            Ok(()) => {
                                recent.retain(|r| !r.starts_with(&p));
                                (Ok(format!("Deleted {name}")), None)
                            }
                            // The trash didn't take it: ask about for good.
                            Err(_) if !for_good => (Ok(String::new()), Some(p)),
                            Err(e) => (Err(e), None),
                        }
                    }
                }
            });
        }
        match done {
            Some(None) => {
                self.browse.edit = None;
            }
            Some(Some(e)) => self.browse.edit = Some(e),
            None => {}
        }
    }
}

/// A big icon button with its name under it.
fn big_button(ui: &mut egui::Ui, icon: Icon, label: &str, tip: &str) -> egui::Response {
    let size = egui::vec2(150.0, 104.0);
    let (rect, r) = ui.allocate_exact_size(size, egui::Sense::click());
    let pal = theme::now();
    let fill = if r.hovered() { pal.raised } else { pal.panel };
    ui.painter().rect_filled(rect, 8.0, fill);
    let icon_rect = egui::Rect::from_center_size(rect.center() - egui::vec2(0.0, 14.0), egui::vec2(48.0, 48.0));
    icons::paint(ui.painter(), icon_rect, icon, true);
    ui.painter().text(rect.center_bottom() - egui::vec2(0.0, 16.0), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(15.0), pal.text);
    r.on_hover_text(tip)
}
