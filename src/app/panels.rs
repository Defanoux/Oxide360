//! The toolbar, browser, side panels and the floating number box.

use crate::*;

/// One row of the floating browser: its own small dark box, so the model
/// shows between the rows (as in Fusion).
fn browser_row<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(30, 30, 30, 215))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| ui.horizontal(add).inner)
}

/// A tool in a toolbar group: what the group shows and returns when clicked.
struct ToolItem {
    name: &'static str,
    icon: Icon,
    tip: &'static str,
    enabled: bool,
    on: bool,
}

/// A toolbar group whose tools can be tucked away: the ones on the toolbar
/// as icons, every one of them in the list under the group's name (its ▾).
/// Right-click a tool — on the bar or in the list — to add it to the toolbar
/// or take it off. Returns the tool clicked.
///
/// Tools on the toolbar can be dragged into another order within their group;
/// `order` keeps the arrangement (names, earlier first).
/// `scope` keeps its names apart from another group's tools of the same
/// name in the settings ("Solid " for the model's groups).
/// `families`: tools that come in kinds (Rectangle: 2-point, 3-point,
/// centre). The tool's button is the kind used last, with a ▾ for the
/// others; tucked away, its row in the list opens them. A kind's name is
/// returned when one is picked.
fn tool_group(ui: &mut egui::Ui, label: &str, scope: &str, items: &[ToolItem], families: &[(&'static str, Vec<ToolItem>)], hidden: &mut Vec<String>, order: &mut Vec<String>) -> Option<&'static str> {
    let key = |n: &str| format!("{scope}{n}");
    let mut chosen = None;
    let mut toggle: Option<&'static str> = None;
    // The group's tools in the saved order (new ones where they're listed).
    let rank = |name: &str, k: usize| order.iter().position(|o| *o == key(name)).unwrap_or(1000 + k);
    let mut sorted: Vec<(usize, &ToolItem)> = items.iter().enumerate().collect();
    sorted.sort_by_key(|&(k, it)| rank(it.name, k));
    let items: Vec<&ToolItem> = sorted.into_iter().map(|(_, it)| it).collect();
    let drag_id = egui::Id::new(("tool_drag", label));
    let mut dragging: Option<&'static str> = ui.ctx().data(|d| d.get_temp(drag_id)).flatten();
    let mut dropped: Option<(&'static str, usize)> = None;
    ui.vertical(|ui| {
        let row = ui
            .horizontal(|ui| {
                let shown: Vec<&ToolItem> = items.iter().copied().filter(|it| !hidden.iter().any(|h| *h == key(it.name))).collect();
                let mut rects: Vec<egui::Rect> = Vec::new();
                for it in &shown {
                    let r = icons::button(ui, it.icon, it.tip, it.enabled, it.on);
                    rects.push(r.rect);
                    if r.clicked() {
                        chosen = Some(it.name);
                    }
                    if r.drag_started() {
                        dragging = Some(it.name);
                    }
                    // Greyed out it can't be used, but it can still be moved along or taken off the toolbar.
                    let r = if it.enabled { r } else { ui.interact(r.rect, ui.id().with(("tool_off", it.name)), egui::Sense::click_and_drag()).on_hover_text(it.tip) };
                    if !it.enabled && r.drag_started() {
                        dragging = Some(it.name);
                    }
                    if let Some((_, kinds)) = families.iter().find(|(n, _)| *n == it.name) {
                        let more = icons::dropdown(ui, &format!("More {} tools", it.name.to_lowercase()));
                        egui::Popup::menu(&more).show(|ui| {
                            if let Some(k) = kind_rows(ui, kinds) {
                                chosen = Some(k);
                            }
                        });
                    }
                    r.context_menu(|ui| {
                        if ui.button("Remove from toolbar").clicked() {
                            toggle = Some(it.name);
                            ui.close();
                        }
                    });
                }
                // Dragging a tool: a bar shows where it'll go; let go to put it there.
                if let (Some(name), Some(p)) = (dragging, ui.input(|i| i.pointer.interact_pos())) {
                    let gap = rects.iter().filter(|r| r.center().x < p.x).count();
                    if let (Some(first), Some(last)) = (rects.first(), rects.last()) {
                        let x = if gap == 0 { first.left() - 2.0 } else if gap >= rects.len() { last.right() + 2.0 } else { (rects[gap - 1].right() + rects[gap].left()) / 2.0 };
                        ui.painter().line_segment([egui::pos2(x, first.top()), egui::pos2(x, first.bottom())], Stroke::new(3.0, Color32::from_rgb(70, 145, 230)));
                    }
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    if ui.input(|i| i.pointer.any_released()) {
                        dropped = Some((name, gap));
                        dragging = None;
                    }
                }
                if let Some((name, gap)) = dropped {
                    let mut names: Vec<&'static str> = shown.iter().map(|it| it.name).collect();
                    if let Some(from) = names.iter().position(|n| *n == name) {
                        let to = if gap > from { gap - 1 } else { gap };
                        let n = names.remove(from);
                        names.insert(to.min(names.len()), n);
                    }
                    // The whole group's order: the tools on the toolbar as dropped, then the rest.
                    let rest: Vec<&'static str> = items.iter().map(|it| it.name).filter(|n| !names.contains(n)).collect();
                    order.retain(|o| !items.iter().any(|it| *o == key(it.name)));
                    order.extend(names.into_iter().chain(rest).map(key));
                }
            })
            .response
            .rect
            .width();
        ui.ctx().data_mut(|d| d.insert_temp(drag_id, dragging));
        let tucked: Vec<&ToolItem> = items.iter().copied().filter(|it| hidden.iter().any(|h| *h == key(it.name))).collect();
        let (picked, pin) = group_menu(ui, label, row, &tucked, families, true);
        chosen = chosen.or(picked);
        toggle = toggle.or(pin);
    });
    ui.separator();
    if let Some(name) = toggle {
        match hidden.iter().position(|h| *h == key(name)) {
            Some(k) => {
                hidden.remove(k);
            }
            None => hidden.push(key(name)),
        }
    }
    chosen
}

/// A toolbar group's name with a ▾ under its icons (`row`: their width):
/// it lists `tucked`, the group's tools that aren't on the toolbar. With
/// `pinnable`, each row's ⋯ can pin it to the toolbar. Returns the tool
/// clicked and the one to pin.
/// The kinds of a tool (see `tool_group`), as rows with their icons. Returns the one clicked.
fn kind_rows(ui: &mut egui::Ui, kinds: &[ToolItem]) -> Option<&'static str> {
    for k in kinds {
        let row = ui.horizontal(|ui| {
            let icon = icons::button(ui, k.icon, k.tip, k.enabled, k.on);
            let name = ui.selectable_label(k.on, k.name);
            icon.clicked() || name.clicked()
        });
        if row.inner {
            ui.close();
            return Some(k.name);
        }
    }
    None
}

fn group_menu(ui: &mut egui::Ui, label: &str, row: f32, tucked: &[&ToolItem], families: &[(&'static str, Vec<ToolItem>)], pinnable: bool) -> (Option<&'static str>, Option<&'static str>) {
    let mut chosen = None;
    let mut toggle: Option<&'static str> = None;
    // The group's name with a ▾: every tool, with names.
    let font = egui::FontId::proportional(11.0);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, Color32::from_gray(150));
    let width = row.max(galley.size().x + 18.0);
    let (rect, r) = ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::click());
    let ink = if r.hovered() { Color32::from_gray(230) } else { Color32::from_gray(150) };
    let text_at = rect.center() - egui::vec2((galley.size().x + 12.0) / 2.0, galley.size().y / 2.0);
    ui.painter().galley_with_override_text_color(text_at, galley.clone(), ink);
    let c = egui::pos2(text_at.x + galley.size().x + 8.0, rect.center().y);
    ui.painter().add(egui::Shape::convex_polygon(vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)], ink, Stroke::NONE));
    let r = r.on_hover_text("More tools in this group (right-click a tool on the toolbar to take it off)");
    // As wide as its rows need, not as wide as the whole group above it.
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).width(240.0).show(|ui| {
        ui.set_min_width(230.0);
        if tucked.is_empty() {
            ui.weak("Every tool here is on the toolbar.");
        }
        for &it in tucked {
            // A tool with kinds: its row opens them (and can pin the tool back).
            if let Some((_, kinds)) = families.iter().find(|(n, _)| *n == it.name) {
                ui.horizontal(|ui| {
                    let (spot, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                    icons::paint(ui.painter(), spot.shrink(2.0), it.icon, it.enabled);
                    ui.menu_button(it.name, |ui| {
                        if let Some(k) = kind_rows(ui, kinds) {
                            chosen = Some(k);
                        }
                        if pinnable {
                            ui.separator();
                            if ui.button("Pin to toolbar").clicked() {
                                toggle = Some(it.name);
                                ui.close();
                            }
                        }
                    });
                });
                continue;
            }
            // The whole row is the button; ⋯ at its end pins it to the toolbar.
            let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), if it.enabled { egui::Sense::click() } else { egui::Sense::hover() });
            let more = egui::Rect::from_min_size(row.right_top() - egui::vec2(26.0, 0.0), egui::vec2(24.0, 24.0));
            let over_more = pinnable && ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| more.contains(p));
            if (resp.hovered() && !over_more) || it.on {
                ui.painter().rect_filled(row, 4.0, Color32::from_gray(if it.on { 70 } else { 55 }));
            }
            icons::paint(ui.painter(), egui::Rect::from_min_size(row.min + egui::vec2(4.0, 4.0), egui::vec2(16.0, 16.0)), it.icon, it.enabled);
            let ink = if it.enabled { Color32::from_gray(225) } else { Color32::from_gray(110) };
            ui.painter().text(row.left_center() + egui::vec2(28.0, 0.0), egui::Align2::LEFT_CENTER, it.name, egui::FontId::proportional(14.0), ink);
            if resp.clicked() && !over_more {
                chosen = Some(it.name);
                ui.close();
            }
            if !pinnable {
                resp.on_hover_text(it.tip);
                continue;
            }
            let dots = ui.put(more, egui::Button::new("").frame(over_more)).on_hover_text("More");
            for k in -1..=1 {
                ui.painter().circle_filled(more.center() + egui::vec2(k as f32 * 5.0, 0.0), 1.6, Color32::from_gray(200));
            }
            // ⋯ opens this row's options just under it (in the same list).
            let open_id = egui::Id::new(("tool_more", label));
            let open: Option<&'static str> = ui.ctx().data(|d| d.get_temp(open_id)).flatten();
            if dots.clicked() {
                let next = if open == Some(it.name) { None } else { Some(it.name) };
                ui.ctx().data_mut(|d| d.insert_temp(open_id, next));
            }
            if open == Some(it.name) {
                ui.horizontal(|ui| {
                    ui.add_space(26.0);
                    if ui.button("Pin to toolbar").clicked() {
                        toggle = Some(it.name);
                        ui.ctx().data_mut(|d| d.insert_temp(open_id, None::<&'static str>));
                    }
                });
            }
            if !over_more {
                resp.on_hover_text(it.tip);
            }
        }
    });
    (chosen, toggle)
}

impl OpenCad {
    /// The top bar, like Fusion's: file menu, save, undo / redo (with lists
    /// of steps), the open designs as tabs (× closes, + opens a new one) and
    /// Settings; under it, which workspace you're in. It doesn't scroll.
    pub(crate) fn title_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let file = icons::button(ui, Icon::File, "File: new, open, save, export", true, false);
            egui::Popup::menu(&file).show(|ui| {
                for (cmd, label) in [
                    (Cmd::New, "New Design         Ctrl+N"),
                    (Cmd::Open, "Open…             Ctrl+O"),
                    (Cmd::Save, "Save                Ctrl+S"),
                    (Cmd::SaveAs, "Save As…  Ctrl+Shift+S"),
                ] {
                    if ui.button(label).clicked() {
                        self.cmd = Some(cmd);
                        ui.close();
                    }
                }
                ui.separator();
                ui.menu_button("Export", |ui| {
                    for f in export::Format::ALL {
                        if ui.button(format!("{}…", f.label())).clicked() {
                            self.cmd = Some(Cmd::Export(f));
                            ui.close();
                        }
                    }
                });
            });
            if icons::button(ui, Icon::Save, "Save (Ctrl+S) — each save is a new version", true, false).clicked() {
                self.cmd = Some(Cmd::Save);
            }
            // Undo / redo, each with a list to jump several steps.
            let (back, ahead) = self.history.depth();
            if icons::button(ui, Icon::Undo, "Undo (Ctrl+Z)", self.can_undo(), false).clicked() {
                self.undo();
            }
            let list = ui.add_enabled_ui(back > 0, |ui| icons::dropdown(ui, "Undo several steps")).inner;
            egui::Popup::menu(&list).show(|ui| {
                for n in 1..=back.min(20) {
                    if ui.button(format!("Undo {n} step{}", if n == 1 { "" } else { "s" })).clicked() {
                        for _ in 0..n {
                            self.undo();
                        }
                        ui.close();
                    }
                }
            });
            if icons::button(ui, Icon::Redo, "Redo (Ctrl+Shift+Z or Ctrl+Y)", self.history.can_redo(), false).clicked() {
                self.redo();
            }
            let list = ui.add_enabled_ui(ahead > 0, |ui| icons::dropdown(ui, "Redo several steps")).inner;
            egui::Popup::menu(&list).show(|ui| {
                for n in 1..=ahead.min(20) {
                    if ui.button(format!("Redo {n} step{}", if n == 1 { "" } else { "s" })).clicked() {
                        for _ in 0..n {
                            self.redo();
                        }
                        ui.close();
                    }
                }
            });
            ui.separator();
            // The open designs: name, version, a dot when unsaved; × closes.
            let titles = if self.tab_titles.is_empty() { vec![(self.tab_title(), self.is_dirty())] } else { self.tab_titles.clone() };
            let mut rename_done: Option<Option<String>> = None;
            for (k, (title, _)) in titles.iter().enumerate() {
                let current = k == self.tab_index;
                let text = title.clone();
                egui::Frame::new()
                    .fill(if current { Color32::from_gray(58) } else { Color32::from_gray(36) })
                    .corner_radius(5.0)
                    .inner_margin(egui::Margin::symmetric(8, 3))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            // Right-click → Rename (this tab's name is edited in place).
                            if let (true, Some(t)) = (current, self.renaming_design.as_mut()) {
                                let mut out = egui::TextEdit::singleline(t).desired_width(140.0).show(ui);
                                let edit = out.response.clone();
                                edit.request_focus();
                                // The whole name selected at first, so typing replaces it.
                                if std::mem::take(&mut self.rename_select_all) {
                                    out.state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all(&out.galley)));
                                    out.state.store(ui.ctx(), edit.id);
                                }
                                let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                                if enter {
                                    rename_done = Some(Some(t.clone()));
                                } else if esc || edit.lost_focus() {
                                    rename_done = Some(None);
                                }
                            } else {
                                let label = egui::RichText::new(text).color(if current { Color32::WHITE } else { Color32::from_gray(160) });
                                let r = ui.add(egui::Label::new(label).sense(egui::Sense::click())).on_hover_text("Right-click to rename");
                                if r.clicked() && !current {
                                    self.tab_request = Some(TabRequest::Switch(k));
                                }
                                if current {
                                    r.context_menu(|ui| {
                                        if ui.button("Rename…").clicked() {
                                            self.renaming_design = Some(self.file_name().trim_end_matches(".ocad").to_string());
                                            self.rename_select_all = true;
                                            ui.close();
                                        }
                                    });
                                }
                            }
                            if ui.small_button("×").on_hover_text("Close this design").clicked() {
                                self.tab_request = Some(TabRequest::Close(k));
                            }
                        });
                    });
            }
            match rename_done {
                Some(Some(name)) => {
                    self.renaming_design = None;
                    self.rename_design(&name);
                }
                Some(None) => self.renaming_design = None,
                None => {}
            }
            if ui.button("+").on_hover_text("New design in a new tab").clicked() {
                self.tab_request = Some(TabRequest::New);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("⚙").on_hover_text("Settings").clicked() {
                    self.settings_open = !self.settings_open;
                }
            });
        });
        // Which workspace you're in, like Fusion's SOLID / SKETCH tabs.
        let sketching = self.active.is_some();
        ui.horizontal(|ui| {
            for (name, on) in [("SOLID", !sketching), ("SKETCH", sketching)] {
                let text = egui::RichText::new(name).size(12.0);
                let text = if on { text.color(Color32::from_rgb(110, 170, 255)).strong() } else { text.color(Color32::from_gray(110)) };
                ui.label(text);
                ui.add_space(10.0);
            }
            if let Some(i) = self.active {
                ui.weak(format!("— editing {}", self.sketches[i].name));
            }
        });
    }

    /// This design's tab title: its name and version ("Bracket v3").
    pub(crate) fn tab_title(&self) -> String {
        let name = self.file_name().trim_end_matches(".ocad").to_string();
        if self.version > 0 { format!("{name} v{}", self.version) } else { name }
    }

    /// Global settings (kept between runs, for every design).
    pub(crate) fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        egui::Window::new("Settings").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            egui::Grid::new("settings_grid").num_columns(2).spacing([16.0, 8.0]).show(ui, |ui| {
                ui.label("Input device");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut self.touchpad, false, "Mouse").on_hover_text("Scroll zooms · right/middle-drag pans");
                    ui.radio_value(&mut self.touchpad, true, "Touchpad")
                        .on_hover_text("Two-finger scroll pans · Shift+two-finger orbits · Ctrl+two-finger zooms");
                });
                ui.end_row();
                ui.label("Touchpad zoom");
                ui.checkbox(&mut self.touchpad_zoom_flip, "Fingers up zoom in");
                ui.end_row();
                ui.label("Units for new designs");
                egui::ComboBox::from_id_salt("settings_units").selected_text(self.default_units.long_name()).show_ui(ui, |ui| {
                    for u in units::Unit::ALL {
                        ui.selectable_value(&mut self.default_units, u, u.long_name());
                    }
                });
                ui.end_row();
                ui.label("Camera");
                egui::ComboBox::from_id_salt("settings_camera").selected_text(self.camera.projection.name()).show_ui(ui, |ui| {
                    for p in Projection::ALL {
                        ui.selectable_value(&mut self.camera.projection, p, p.name());
                    }
                });
                ui.end_row();
                ui.label("View cube");
                ui.checkbox(&mut self.cube_axes, "Show X/Y/Z axes");
                ui.end_row();
                ui.label("Measure precision");
                egui::ComboBox::from_id_salt("settings_precision").selected_text(format!("{:.*}", self.measure_precision, 0.0)).show_ui(ui, |ui| {
                    for d in 0..=8 {
                        ui.selectable_value(&mut self.measure_precision, d, format!("{:.*}", d, 0.0));
                    }
                });
                ui.end_row();
            });
            ui.weak("A design's own units are under Document Settings in the browser.");
            if ui.button("Set keybinds…").clicked() {
                self.keybinds_open = true;
            }
        });
        self.settings_open = open;
        self.keybinds_window(ctx);
    }

    /// Which key does what: click a key, then press the new one (Esc: no key).
    fn keybinds_window(&mut self, ctx: &egui::Context) {
        let mut open = self.keybinds_open;
        // A key being set takes the next key pressed.
        if let Some(a) = self.capturing_key {
            let key = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.alt => Some(*key),
                    _ => None,
                })
            });
            if let Some(k) = key {
                // Taken here, so the key doesn't also work as a shortcut this frame.
                ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
                self.capturing_key = None;
                let k = (k != egui::Key::Escape).then_some(k);
                if let Some(other) = self.keymap.set(a, k) {
                    self.notify(format!("{} no longer has a key (it was {})", other.name(), k.map_or("", |k| k.name())));
                }
            }
        }
        egui::Window::new("Keybinds").open(&mut open).collapsible(false).resizable(false).show(ctx, |ui| {
            for (title, sketch) in [("In a sketch", true), ("In the model", false)] {
                ui.strong(title);
                egui::Grid::new(("keybinds", sketch)).num_columns(4).spacing([16.0, 6.0]).striped(true).show(ui, |ui| {
                    for a in keys::Action::ALL.into_iter().filter(|a| a.in_sketch() == sketch) {
                        ui.label(a.name());
                        ui.weak(a.about());
                        let waiting = self.capturing_key == Some(a);
                        let changed = self.keymap.key(a) != a.default_key();
                        let text = if waiting { "press a key…".to_string() } else { self.keymap.key(a).map_or("—".to_string(), |k| k.name().to_string()) };
                        // A key that isn't the default shows orange.
                        let text = if changed && !waiting { egui::RichText::new(text).color(Color32::from_rgb(240, 160, 50)).strong() } else { egui::RichText::new(text) };
                        if ui.add(egui::Button::selectable(waiting, text).min_size(egui::vec2(90.0, 0.0))).on_hover_text("Click, then press the key (Esc: no key)").clicked() {
                            self.capturing_key = if waiting { None } else { Some(a) };
                        }
                        // …with a reset arrow beside it to put its default back.
                        if changed {
                            let default = a.default_key().map_or("no key".to_string(), |k| k.name().to_string());
                            let r = ui.add(egui::Button::new("").min_size(egui::vec2(22.0, 20.0))).on_hover_text(format!("Back to the default ({default})"));
                            let (c, ink) = (r.rect.center(), Color32::from_gray(220));
                            let pts: Vec<egui::Pos2> = (0..=20)
                                .map(|i| {
                                    let t = -0.6 + i as f32 / 20.0 * 4.9;
                                    c + egui::vec2(t.cos(), t.sin()) * 5.5
                                })
                                .collect();
                            let tip = *pts.last().expect("points");
                            ui.painter().add(egui::Shape::line(pts, Stroke::new(1.5, ink)));
                            ui.painter().add(egui::Shape::convex_polygon(vec![tip + egui::vec2(-3.5, -1.0), tip + egui::vec2(2.5, -3.0), tip + egui::vec2(1.0, 3.0)], ink, Stroke::NONE));
                            if r.clicked() {
                                if let Some(other) = self.keymap.set(a, a.default_key()) {
                                    self.notify(format!("{} no longer has a key (it was {default})", other.name()));
                                }
                                self.capturing_key = None;
                            }
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
            }
            if ui.button("Reset to defaults").clicked() {
                self.keymap = keys::Keymap::default();
                self.capturing_key = None;
            }
        });
        if !open {
            self.capturing_key = None;
        }
        self.keybinds_open = open;
    }

    /// FINISH SKETCH, kept at the toolbar's right end (outside its scrolling,
    /// so it never goes off the edge), as in Fusion.
    pub(crate) fn finish_sketch_group(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            if icons::button(ui, Icon::FinishSketch, "Finish Sketch", true, false).clicked() {
                self.finish_sketch();
            }
            ui.add(egui::Label::new(egui::RichText::new("FINISH SKETCH").size(11.0).color(Color32::from_gray(150))).extend());
        });
    }

    /// The ribbon: quick-access icons (file, save, undo, redo), then tool
    /// groups with their names underneath, like Fusion's toolbar.
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        // One group: a row of icons with a small label centred under it. The
        // label area is exactly as wide as the icons (a centred layout would
        // stretch across the whole toolbar and push the other groups away).
        fn group(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
            ui.vertical(|ui| {
                let font = egui::FontId::proportional(11.0);
                let text_w = ui.painter().layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE).size().x;
                // A short group is as wide as its label, so the label never
                // runs into the next group.
                ui.set_min_width(text_w + 6.0);
                // Centre the icons under a wider label: pad by half the
                // difference, using the icons' width measured last frame.
                let id = egui::Id::new(("toolbar_group", label));
                let icons_w: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.0);
                let pad = ((text_w + 6.0 - icons_w) * 0.5).max(0.0);
                // (The icons' own width: the row's rect, without the spacing after the last.)
                let row = ui
                    .horizontal(|ui| {
                        ui.add_space(pad);
                        add(ui);
                    })
                    .response
                    .rect
                    .width()
                    - pad;
                if (row - icons_w).abs() > 0.5 {
                    ui.ctx().data_mut(|d| d.insert_temp(id, row));
                    ui.ctx().request_repaint();
                }
                let width = (row + pad).max(text_w + 6.0);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::hover());
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, label, font, Color32::from_gray(150));
            });
            ui.separator();
        }
        ui.horizontal(|ui| {
            if let Some(i) = self.active {
                // --- sketch mode ---
                // CREATE: rectangle, circle, arc and polygon each come in kinds;
                // their button is the kind used last.
                let t = self.editor.tool;
                let arc_icon = |t: Tool| match t {
                    Tool::ArcCenter => Icon::ArcCenter,
                    Tool::ArcTangent => Icon::ArcTangent,
                    _ => Icon::Arc3Point,
                };
                let kinds = |tools: &[Tool], icon: &dyn Fn(Tool) -> Icon| -> Vec<ToolItem> {
                    tools.iter().map(|&k| ToolItem { name: k.name(), icon: icon(k), tip: k.name(), enabled: true, on: t == k }).collect()
                };
                let families = [
                    ("Rectangle", kinds(&Tool::RECTANGLES, &rect_icon)),
                    ("Circle", kinds(&Tool::CIRCLES, &circle_icon)),
                    ("Arc", kinds(&Tool::ARCS, &arc_icon)),
                    ("Polygon", kinds(&Tool::POLYGONS, &|_| Icon::Polygon)),
                    ("Spline", kinds(&Tool::SPLINES, &spline_icon)),
                ];
                let (rect_tool, circle, arc_tool, poly_tool, spline_tool) = (self.rect_tool, self.circle_tool, self.arc_tool, self.poly_tool, self.spline_tool);
                let items = [
                    ToolItem { name: "Select", icon: Icon::Select, tip: "Select (Esc)", enabled: true, on: t == Tool::Select },
                    ToolItem { name: "Line", icon: Icon::Line, tip: "Line (L)", enabled: true, on: t == Tool::Line },
                    ToolItem { name: "Rectangle", icon: rect_icon(rect_tool), tip: "Rectangle (R) — the kind used last; the arrow beside it for the others", enabled: true, on: Tool::RECTANGLES.contains(&t) },
                    ToolItem { name: "Point", icon: Icon::Point, tip: "Point: a sketch point (on a line if you click one)", enabled: true, on: t == Tool::Point },
                    ToolItem { name: "Circle", icon: circle_icon(circle), tip: "Circle (C) — the kind used last; the arrow beside it for the others", enabled: true, on: Tool::CIRCLES.contains(&t) },
                    ToolItem { name: "Arc", icon: arc_icon(arc_tool), tip: arc_tool.name(), enabled: true, on: Tool::ARCS.contains(&t) },
                    ToolItem { name: "Polygon", icon: Icon::Polygon, tip: poly_tool.name(), enabled: true, on: Tool::POLYGONS.contains(&t) },
                    ToolItem { name: "Slot", icon: Icon::Slot, tip: "Slot: click the two end centres, then the width", enabled: true, on: t == Tool::Slot },
                    ToolItem {
                        name: "Spline",
                        icon: spline_icon(spline_tool),
                        tip: if spline_tool == Tool::Spline {
                            "Fit Point Spline: click the points it goes through; Enter (or click the last point again) finishes. Select it to drag its tangent handles"
                        } else {
                            "Control Point Spline: click its control points (the curve is pulled toward them); the Up/Down arrow keys for degree 3 or 5; Enter finishes"
                        },
                        enabled: true,
                        on: Tool::SPLINES.contains(&t),
                    },
                    ToolItem { name: "Ellipse", icon: Icon::Ellipse, tip: "Ellipse: click the centre, the end of one axis, then how wide", enabled: true, on: t == Tool::Ellipse },
                    ToolItem { name: "Text", icon: Icon::Text, tip: "Text: click where it starts, then type it; its letters are profiles to extrude. Double-click text to edit it", enabled: true, on: t == Tool::Text || self.text_dialog.is_some() },
                ];
                let picked = tool_group(ui, "CREATE", "", &items, &families, &mut self.toolbar_hidden, &mut self.toolbar_order);
                let kind = picked.and_then(|name| Tool::RECTANGLES.into_iter().chain(Tool::CIRCLES).chain(Tool::ARCS).chain(Tool::POLYGONS).chain(Tool::SPLINES).find(|k| k.name() == name));
                match (picked, kind) {
                    // A kind: it becomes its family's button, as Fusion remembers it.
                    (_, Some(k)) => {
                        if Tool::RECTANGLES.contains(&k) {
                            self.rect_tool = k;
                        } else if Tool::CIRCLES.contains(&k) {
                            self.circle_tool = k;
                        } else if Tool::ARCS.contains(&k) {
                            self.arc_tool = k;
                        } else if Tool::SPLINES.contains(&k) {
                            self.spline_tool = k;
                        } else {
                            self.poly_tool = k;
                        }
                        self.editor.set_tool(k);
                    }
                    (Some("Select"), _) => self.editor.set_tool(Tool::Select),
                    (Some("Line"), _) => self.editor.set_tool(Tool::Line),
                    (Some("Rectangle"), _) => self.editor.set_tool(rect_tool),
                    (Some("Point"), _) => self.editor.set_tool(Tool::Point),
                    (Some("Circle"), _) => self.editor.set_tool(circle),
                    (Some("Arc"), _) => self.editor.set_tool(arc_tool),
                    (Some("Polygon"), _) => self.editor.set_tool(poly_tool),
                    (Some("Slot"), _) => self.editor.set_tool(Tool::Slot),
                    (Some("Spline"), _) => self.editor.set_tool(spline_tool),
                    (Some("Ellipse"), _) => self.editor.set_tool(Tool::Ellipse),
                    (Some("Text"), _) => self.editor.set_tool(Tool::Text),
                    _ => {}
                }
                let _ = i;
                let t = self.editor.tool;
                let pattern_on = |m: SpMode| self.sketch_pattern.as_ref().is_some_and(|d| d.mode == m);
                let items = [
                    ToolItem { name: "Fillet", icon: Icon::Fillet, tip: "Fillet: click a corner (or its two lines), type the radius", enabled: true, on: t == Tool::SketchFillet },
                    ToolItem { name: "Chamfer", icon: Icon::Chamfer, tip: "Chamfer: click a corner (or its two lines), type the distance — or two, like 2, 5 (first = first line)", enabled: true, on: t == Tool::SketchChamfer },
                    ToolItem { name: "Trim", icon: Icon::Trim, tip: "Trim: click the part of a line, circle, arc or spline to remove, back to the nearest crossings", enabled: true, on: t == Tool::Trim },
                    ToolItem { name: "Break", icon: Icon::Break, tip: "Break: click a line, circle, arc or spline to split it where things cross it (nothing is removed)", enabled: true, on: t == Tool::Break },
                    ToolItem { name: "Extend", icon: Icon::Extend, tip: "Extend: click a line near the end to run it on to the next geometry", enabled: true, on: t == Tool::Extend },
                    ToolItem { name: "Offset", icon: Icon::Offset, tip: "Offset: click a line, arc, circle or spline (a joined run follows), then click the side and distance", enabled: true, on: t == Tool::Offset },
                    ToolItem { name: "Rectangular Pattern", icon: Icon::RectPattern, tip: "Rectangular Pattern: select geometry first, then click this", enabled: true, on: pattern_on(SpMode::Rectangular) },
                    ToolItem { name: "Circular Pattern", icon: Icon::CircPattern, tip: "Circular Pattern: select geometry first, then click this", enabled: true, on: pattern_on(SpMode::Circular) },
                    ToolItem { name: "Mirror", icon: Icon::Mirror, tip: "Mirror: select geometry first, then click this and the mirror line", enabled: true, on: t == Tool::Mirror },
                    ToolItem { name: "Project", icon: Icon::Project, tip: "Project: click a body edge to bring it into the sketch (as locked construction geometry)", enabled: self.body_count > 0, on: t == Tool::Project },
                    ToolItem { name: "Construction", icon: Icon::Construction, tip: "Construction: toggle the selected geometry, or what you draw next", enabled: true, on: self.editor.construction },
                    ToolItem { name: "Change Parameters", icon: Icon::Parameters, tip: "Change Parameters: named values (width, depth…) that dimensions and boxes can use", enabled: true, on: self.params_window.open },
                ];
                match tool_group(ui, "MODIFY", "", &items, &[], &mut self.toolbar_hidden, &mut self.toolbar_order) {
                    Some("Fillet") => self.editor.set_tool(Tool::SketchFillet),
                    Some("Chamfer") => self.editor.set_tool(Tool::SketchChamfer),
                    Some("Trim") => self.editor.set_tool(Tool::Trim),
                    Some("Break") => self.editor.set_tool(Tool::Break),
                    Some("Extend") => self.editor.set_tool(Tool::Extend),
                    Some("Offset") => self.editor.set_tool(Tool::Offset),
                    Some("Rectangular Pattern") => self.start_sketch_pattern(SpMode::Rectangular),
                    Some("Circular Pattern") => self.start_sketch_pattern(SpMode::Circular),
                    Some("Mirror") => self.start_mirror(),
                    Some("Project") => self.editor.set_tool(Tool::Project),
                    Some("Construction") => self.toggle_construction(),
                    Some("Change Parameters") => self.params_window.open = !self.params_window.open,
                    _ => {}
                }
                let items: Vec<ToolItem> = CONSTRAINT_ACTIONS
                    .into_iter()
                    .map(|label| {
                        let (icon, tip) = match label {
                            "Dimension" => (
                                Icon::Dimension,
                                "Dimension (D): one line: its length · a circle: its diameter · two lines: the angle · two points: the distance",
                            ),
                            "Coincident" => (Icon::Coincident, "Coincident: two points together, or a point onto a line, circle, arc or axis"),
                            "Collinear" => (Icon::Collinear, "Collinear: two lines (or a line and an axis) on one straight line"),
                            "Concentric" => (Icon::Concentric, "Concentric: two circles or arcs round one centre"),
                            "Horizontal" => (Icon::Horizontal, "Horizontal"),
                            "Vertical" => (Icon::Vertical, "Vertical"),
                            "Parallel" => (Icon::Parallel, "Parallel: two lines, or a line and an axis"),
                            "Perpendicular" => (Icon::Perpendicular, "Perpendicular: two lines, or a line and an axis"),
                            "Equal" => (Icon::Equal, "Equal: two lines or two circles"),
                            "Tangent" => (Icon::Tangent, "Tangent: a line and a circle"),
                            "Midpoint" => (Icon::Midpoint, "Midpoint: a point and a line — the point goes to the line's middle"),
                            "Symmetric" => (Icon::Symmetric, "Symmetric: two points, lines or circles, then the line (or axis) they mirror about"),
                            _ => (Icon::Fix, "Fix: lock points where they are"),
                        };
                        // Always clickable: with the right things selected it's added
                        // at once, otherwise it becomes the tool and you pick them.
                        ToolItem { name: label, icon, tip, enabled: true, on: self.editor.tool == Tool::Constrain(label) }
                    })
                    .collect();
                if let Some(label) = tool_group(ui, "CONSTRAINTS", "", &items, &[], &mut self.toolbar_hidden, &mut self.toolbar_order) {
                    self.constraint_button(label);
                }
                group(ui, "OPTIONS", |ui| {
                    if icons::button(ui, Icon::Grid, "Grid snap (G, or Ctrl+G any time)", true, self.grid_snap).clicked() {
                        self.grid_snap = !self.grid_snap;
                    }
                });
                group(ui, "INSPECT", |ui| {
                    if icons::button(ui, Icon::Measure, "Measure: click sketch geometry or the model — one for its size, two for distance and angle", true, self.measure.is_some()).clicked() {
                        self.measure = if self.measure.is_some() { None } else { Some(measure::Measure::default()) };
                        self.editor.set_tool(Tool::Select);
                        self.editor.selection.clear();
                    }
                });
            } else {
                // --- solid mode ---
                let can_profile = self.sketches.iter().any(|s| !find_regions(s).is_empty());
                let has_bodies = self.body_count > 0;
                // Each group's tools can be tucked into its ▾ list, as in sketch mode.
                let (hidden, order) = (&mut self.toolbar_hidden, &mut self.toolbar_order);
                let items = [
                    ToolItem { name: "Create Sketch", icon: Icon::Sketch, tip: "Create Sketch (N): uses the selected plane, or click a plane or flat face", enabled: true, on: self.face_pick && !self.plane_pick },
                    ToolItem { name: "Extrude", icon: Icon::Extrude, tip: "Extrude (E)", enabled: can_profile, on: self.dialog.is_some() },
                    ToolItem { name: "Revolve", icon: Icon::Revolve, tip: "Revolve: spin a profile around an axis", enabled: can_profile, on: self.revolve_dialog.is_some() },
                    ToolItem { name: "Sweep", icon: Icon::Sweep, tip: "Sweep: a profile carried along a path of sketch curves", enabled: can_profile, on: self.sweep_dialog.as_ref().is_some_and(|d| d.mode == sweep_dialog::Mode::Sweep) },
                    ToolItem { name: "Loft", icon: Icon::Loft, tip: "Loft: a shape through profiles on two or more sketches, in order", enabled: can_profile, on: self.sweep_dialog.as_ref().is_some_and(|d| d.mode == sweep_dialog::Mode::Loft) },
                    ToolItem { name: "Pipe", icon: Icon::Pipe, tip: "Pipe: a round section along a path of sketch curves (solid or hollow)", enabled: !self.sketches.is_empty(), on: self.sweep_dialog.as_ref().is_some_and(|d| d.mode == sweep_dialog::Mode::Pipe) },
                    ToolItem { name: "Coil", icon: Icon::Coil, tip: "Coil: a spring — click a plane or face, then set its size and turns", enabled: true, on: self.sweep_dialog.as_ref().is_some_and(|d| d.mode == sweep_dialog::Mode::Coil) },
                    ToolItem { name: "Hole", icon: Icon::Hole, tip: "Hole (H): click sketch points, circles or a face; counterbore, countersink, clearance and tapped holes", enabled: has_bodies, on: self.hole_dialog.is_some() },
                    ToolItem { name: "Thread", icon: Icon::Thread, tip: "Thread: click a shaft or the inside of a hole; shown or modeled, ISO and Unified sizes", enabled: has_bodies, on: self.thread_dialog.is_some() },
                ];
                let create = tool_group(ui, "CREATE", "Solid ", &items, &[], hidden, order);
                let angled = self.plane_dialog.as_ref().is_some_and(|d| d.angle.is_some());
                let items = [
                    ToolItem { name: "Offset Plane", icon: Icon::Plane, tip: "Offset Plane: click an origin plane or a face, then set the distance", enabled: true, on: self.plane_pick || self.plane_dialog.as_ref().is_some_and(|d| d.angle.is_none()) },
                    ToolItem { name: "Plane at Angle", icon: Icon::PlaneAngle, tip: "Plane at Angle: click a sketch line, body edge or axis, then set the angle", enabled: true, on: self.angle_pick || angled },
                    ToolItem { name: "Axis Along Edge", icon: Icon::Axis, tip: "Axis Along Edge: click a straight edge of a body", enabled: has_bodies, on: self.axis_pick },
                    ToolItem { name: "Midplane", icon: Icon::Midplane, tip: "Midplane: click two planes or flat faces — halfway between them (halving the angle if they meet)", enabled: true, on: matches!(self.construct_pick, Some(ConstructPick::Midplane(_))) },
                    ToolItem { name: "Plane Through Three Points", icon: Icon::Plane3Points, tip: "Plane Through Three Points: click three body corners", enabled: has_bodies, on: matches!(self.construct_pick, Some(ConstructPick::ThreePoints(_))) },
                    ToolItem { name: "Tangent Plane", icon: Icon::TangentPlane, tip: "Tangent Plane: click a cylinder where the plane should touch it", enabled: has_bodies, on: self.construct_pick == Some(ConstructPick::Tangent) },
                    ToolItem { name: "Axis Through Cylinder", icon: Icon::AxisCylinder, tip: "Axis Through Cylinder: click a hole or shaft", enabled: has_bodies, on: self.construct_pick == Some(ConstructPick::AxisCylinder) },
                    ToolItem { name: "Point", icon: Icon::ConstructPoint, tip: "Construction Point: click a body corner, or a round edge (its centre)", enabled: has_bodies, on: self.point_pick },
                    ToolItem { name: "Axis Through Two Points", icon: Icon::Axis2Points, tip: "Axis Through Two Points: click two body corners", enabled: has_bodies, on: matches!(self.construct_pick, Some(ConstructPick::AxisTwoPoints(_))) },
                ];
                let construct = tool_group(ui, "CONSTRUCT", "Solid ", &items, &[], hidden, order);
                let copy_mode = self.copy_dialog.as_ref().map(|d| d.mode);
                let items = [
                    ToolItem { name: "Mirror", icon: Icon::Mirror, tip: "Mirror: copy bodies across an origin plane", enabled: has_bodies, on: copy_mode == Some(CopyMode::Mirror) },
                    ToolItem { name: "Rectangular Pattern", icon: Icon::RectPattern, tip: "Rectangular Pattern: copies in a grid", enabled: has_bodies, on: copy_mode == Some(CopyMode::Rectangular) },
                    ToolItem { name: "Circular Pattern", icon: Icon::CircPattern, tip: "Circular Pattern: copies round an axis", enabled: has_bodies, on: copy_mode == Some(CopyMode::Circular) },
                ];
                let pattern = tool_group(ui, "PATTERN", "Solid ", &items, &[], hidden, order);
                let edge_kind = self.edge_dialog.as_ref().map(|d| d.kind);
                let items = [
                    ToolItem { name: "Press Pull", icon: Icon::PressPull, tip: "Press Pull (Q): drag a face to move it (Offset Face), a fillet to change its radius, or a sketch profile to extrude it", enabled: has_bodies || !self.sketches.is_empty(), on: self.offset_dialog.is_some() },
                    ToolItem { name: "Fillet", icon: Icon::Fillet, tip: "Fillet (F): round edges", enabled: has_bodies, on: edge_kind == Some(EdgeKind::Fillet) },
                    ToolItem { name: "Chamfer", icon: Icon::Chamfer, tip: "Chamfer: bevel edges", enabled: has_bodies, on: edge_kind == Some(EdgeKind::Chamfer) },
                    ToolItem { name: "Shell", icon: Icon::Shell, tip: "Shell: hollow a body — click the faces to open, or a body to hollow closed", enabled: has_bodies, on: self.shell_dialog.is_some() },
                    ToolItem { name: "Move/Copy", icon: Icon::Move, tip: "Move/Copy (M): move or turn bodies — drag the arrows and rings, type distances and angles, or turn about an axis", enabled: has_bodies, on: self.move_dialog.is_some() },
                    ToolItem { name: "Change Parameters", icon: Icon::Parameters, tip: "Change Parameters: named values (width, depth…) that dimensions and boxes can use", enabled: true, on: self.params_window.open },
                    ToolItem { name: "Combine", icon: Icon::Combine, tip: "Combine: join, cut or intersect bodies — a target body and tool bodies", enabled: self.body_count > 1, on: self.combine_dialog.is_some() },
                ];
                let modify = tool_group(ui, "MODIFY", "Solid ", &items, &[], hidden, order);
                match create.or(construct).or(pattern).or(modify) {
                    Some("Create Sketch") => self.new_sketch_tool(),
                    Some("Extrude") => self.open_extrude(None),
                    Some("Revolve") => self.open_revolve(None),
                    Some(name @ ("Sweep" | "Loft" | "Pipe" | "Coil")) => {
                        let mode = match name {
                            "Sweep" => sweep_dialog::Mode::Sweep,
                            "Loft" => sweep_dialog::Mode::Loft,
                            "Pipe" => sweep_dialog::Mode::Pipe,
                            _ => sweep_dialog::Mode::Coil,
                        };
                        if self.sweep_dialog.as_ref().is_some_and(|d| d.mode == mode) {
                            self.sweep_dialog = None;
                        } else {
                            self.open_sweep(None, mode);
                        }
                    }
                    Some("Hole") if self.hole_dialog.is_some() => self.cancel_hole(),
                    Some("Hole") => self.open_hole(None),
                    Some("Thread") if self.thread_dialog.is_some() => self.cancel_thread(),
                    Some("Thread") => self.open_thread(None),
                    Some("Offset Plane") => {
                        let was = self.plane_pick;
                        self.cancel_all_dialogs();
                        self.plane_pick = !was;
                        self.face_pick = !was;
                    }
                    Some("Plane at Angle") => {
                        let was = self.angle_pick;
                        self.cancel_all_dialogs();
                        self.angle_pick = !was;
                    }
                    Some("Axis Along Edge") => {
                        let was = self.axis_pick;
                        self.cancel_all_dialogs();
                        self.axis_pick = !was;
                    }
                    Some("Point") => {
                        let was = self.point_pick;
                        self.cancel_all_dialogs();
                        self.point_pick = !was;
                    }
                    Some(name @ ("Midplane" | "Plane Through Three Points" | "Tangent Plane" | "Axis Through Cylinder" | "Axis Through Two Points")) => {
                        let pick = match name {
                            "Midplane" => ConstructPick::Midplane(Vec::new()),
                            "Plane Through Three Points" => ConstructPick::ThreePoints(Vec::new()),
                            "Tangent Plane" => ConstructPick::Tangent,
                            "Axis Through Cylinder" => ConstructPick::AxisCylinder,
                            _ => ConstructPick::AxisTwoPoints(Vec::new()),
                        };
                        let same = self.construct_pick.as_ref().is_some_and(|p| std::mem::discriminant(p) == std::mem::discriminant(&pick));
                        self.cancel_all_dialogs();
                        self.construct_pick = if same { None } else { Some(pick) };
                    }
                    Some("Mirror") => self.open_copy(CopyMode::Mirror, None),
                    Some("Rectangular Pattern") => self.open_copy(CopyMode::Rectangular, None),
                    Some("Circular Pattern") => self.open_copy(CopyMode::Circular, None),
                    Some("Press Pull") if self.offset_dialog.is_some() => self.cancel_offset(),
                    Some("Press Pull") => self.open_offset(None, true),
                    Some("Fillet") => self.open_edge_dialog(None, EdgeKind::Fillet),
                    Some("Chamfer") => self.open_edge_dialog(None, EdgeKind::Chamfer),
                    Some("Shell") if self.shell_dialog.is_some() => self.cancel_shell(),
                    Some("Shell") => self.open_shell(None),
                    Some("Move/Copy") if self.move_dialog.is_some() => self.cancel_move(),
                    Some("Move/Copy") => self.open_move(None),
                    Some("Change Parameters") => self.params_window.open = !self.params_window.open,
                    Some("Combine") if self.combine_dialog.is_some() => self.cancel_combine(),
                    Some("Combine") => self.open_combine(None),
                    _ => {}
                }
                group(ui, "INSPECT", |ui| {
                    if icons::button(ui, Icon::Measure, "Measure: click a corner, edge, face, body or sketch geometry — one for its size, two for distance and angle", has_bodies || !self.sketches.is_empty(), self.measure.is_some()).clicked() {
                        self.measure = if self.measure.is_some() { None } else { Some(measure::Measure::default()) };
                    }
                    let tip = "Section Analysis: see inside — click a plane or flat face to cut the view along (the design isn't changed)";
                    if icons::button(ui, Icon::Section, tip, has_bodies, self.section_dialog.is_some()).clicked() {
                        self.section_dialog = if self.section_dialog.is_some() { None } else { Some(section_dialog::SectionDialog::new()) };
                    }
                });
            }
        });
    }

    pub(crate) fn browser(&mut self, ui: &mut egui::Ui) {
        browser_row(ui, |ui| {
            ui.strong("BROWSER");
            if ui.small_button("«").on_hover_text("Fold the browser away").clicked() {
                self.browser_open = false;
            }
        });
        // Down to the bottom of the space it's given; it only scrolls when
        // there's more than fits.
        let room = ui.available_height().max(80.0);
        egui::ScrollArea::vertical().max_height(room).auto_shrink([true, true]).show(ui, |ui| {
            // The design itself (a single part for now; assemblies come later).
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), egui::Id::new("browser_root"), true)
                .show_header(ui, |ui| {
                    browser_row(ui, |ui| {
                        icons::small(ui, Icon::Part);
                        ui.strong(self.file_name()).on_hover_text("This design (one part)");
                    });
                })
                .body(|ui| {
                    self.browser_settings(ui);
                    self.browser_named_views(ui);
                    self.browser_origin(ui);
                    self.browser_analysis(ui);
                    self.browser_construction(ui);
                    self.browser_bodies(ui);
                    self.browser_sketches(ui);
                });
        });
    }

    /// A folder row in the browser: an eye (when it has one), the folder
    /// icon and its name; returns the folder's open/closed state to show its
    /// rows in. `eye`: is anything in it shown, and set when clicked.
    fn folder<R>(&mut self, ui: &mut egui::Ui, id: &str, icon: Icon, name: &str, eye: Option<bool>, body: impl FnOnce(&mut Self, &mut egui::Ui) -> R) -> bool {
        self.folder_header(ui, id, icon, name, eye, body).0
    }

    /// As `folder`, also returning its name's row (for a right-click menu on it).
    fn folder_header<R>(&mut self, ui: &mut egui::Ui, id: &str, icon: Icon, name: &str, eye: Option<bool>, body: impl FnOnce(&mut Self, &mut egui::Ui) -> R) -> (bool, egui::Response) {
        let mut eye_clicked = false;
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), egui::Id::new(id), false);
        let (_, header, _) = state
            .show_header(ui, |ui| {
                browser_row(ui, |ui| {
                    if let Some(shown) = eye {
                        eye_clicked = icons::eye(ui, shown, if shown { "Hide everything in it" } else { "Show everything in it" }).clicked();
                    }
                    icons::small(ui, icon);
                    ui.label(name).interact(egui::Sense::click())
                })
                .inner
            })
            .body(|ui| body(self, ui));
        (eye_clicked, header.inner)
    }

    /// Document Settings: the design's units (saved with it).
    fn browser_settings(&mut self, ui: &mut egui::Ui) {
        self.folder(ui, "folder_settings", Icon::Gear, "Document Settings", None, |app, ui| {
            browser_row(ui, |ui| {
                ui.label("Units");
                egui::ComboBox::from_id_salt("design_units").selected_text(app.units.name()).show_ui(ui, |ui| {
                    for u in units::Unit::ALL {
                        ui.selectable_value(&mut app.units, u, u.long_name());
                    }
                });
            });
            ui.weak(format!("New designs start in {}.", app.default_units.name()));
            if app.default_units != app.units && ui.small_button(format!("Make {} the default", app.units.name())).clicked() {
                app.default_units = app.units;
            }
            ui.weak("Values are kept exactly; only how they're shown and typed changes. Type \"22 mm\" or \"1 in\" in any box.");
            // Threads: model them all (say, for an export to print) or show them all.
            let (on, all) = model::thread_count(&app.holes, &app.thread_features);
            if all > 0 {
                ui.separator();
                browser_row(ui, |ui| {
                    ui.label(format!("Threads: {on} of {all} modeled"));
                });
                browser_row(ui, |ui| {
                    if ui.add_enabled(on < all, egui::Button::new("Model all")).on_hover_text("Cut every thread into the part (slow to rebuild)").clicked() {
                        model::set_all_modeled(&mut app.holes, &mut app.thread_features, true);
                    }
                    if ui.add_enabled(on > 0, egui::Button::new("All cosmetic")).on_hover_text("Only draw the threads (fast)").clicked() {
                        model::set_all_modeled(&mut app.holes, &mut app.thread_features, false);
                    }
                });
            }
        });
    }

    /// The origin planes and axes.
    fn browser_origin(&mut self, ui: &mut egui::Ui) {
        let shown = self.origin_shown.all;
        let clicked = self.folder(ui, "origin_folder", Icon::Folder, "Origin", Some(shown), |app, ui| {
            for op in OriginPlane::ALL {
                let k = op as usize;
                browser_row(ui, |ui| {
                    if icons::eye(ui, app.origin_shown.planes[k], "Show or hide").clicked() {
                        app.origin_shown.planes[k] = !app.origin_shown.planes[k];
                    }
                    icons::small(ui, Icon::Plane);
                    ui.label(format!("{} plane", op.name()));
                });
            }
            for (k, name) in ["X axis", "Y axis", "Z axis"].into_iter().enumerate() {
                browser_row(ui, |ui| {
                    if icons::eye(ui, app.origin_shown.axes[k], "Show or hide").clicked() {
                        app.origin_shown.axes[k] = !app.origin_shown.axes[k];
                    }
                    icons::small(ui, Icon::Axis);
                    ui.label(name);
                });
            }
        });
        if clicked {
            self.origin_shown.all = !shown;
        }
    }

    /// Named Views, as in Fusion: TOP, FRONT, RIGHT and HOME, then the views
    /// saved by name (right-click the folder to save the current view;
    /// right-click one to rename or delete it). Click one to go there.
    fn browser_named_views(&mut self, ui: &mut egui::Ui) {
        let now = ui.input(|i| i.time);
        let mut go: Option<Pose> = None;
        let mut delete: Option<usize> = None;
        let (_, header) = self.folder_header(ui, "folder_named_views", Icon::Folder, "Named Views", None, |app, ui| {
            for (name, dir) in [("TOP", glam::Vec3::Z), ("FRONT", glam::Vec3::NEG_Y), ("RIGHT", glam::Vec3::X)] {
                if browser_row(ui, |ui| ui.selectable_label(false, name)).inner.clicked() {
                    let (yaw, pitch) = angles_for_dir(dir);
                    go = Some(Pose { yaw: yaw.to_radians(), pitch: pitch.to_radians(), roll: 0.0, ..app.camera.pose() });
                }
            }
            if browser_row(ui, |ui| ui.selectable_label(false, "HOME")).inner.clicked() {
                go = Some(app.home);
            }
            for k in 0..app.named_views.len() {
                let r = browser_row(ui, |ui| ui.selectable_label(false, app.named_views[k].0.as_str())).inner;
                if r.clicked() {
                    go = Some(app.named_views[k].1);
                }
                r.context_menu(|ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut app.named_views[k].0);
                    if ui.button("Update to the current view").clicked() {
                        app.named_views[k].1 = app.camera.pose();
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        delete = Some(k);
                        ui.close();
                    }
                });
            }
        });
        // Right-click the folder: save the current view under a new name.
        header.on_hover_text("Click a view to go there; right-click here to save the current view").context_menu(|ui| {
            if ui.button("New Named View").clicked() {
                let n = (1..).find(|n| !self.named_views.iter().any(|(name, _)| *name == format!("Named View{n}"))).unwrap_or(1);
                self.named_views.push((format!("Named View{n}"), self.camera.pose()));
                ui.close();
            }
        });
        if let Some(k) = delete {
            self.named_views.remove(k);
        }
        if let Some(to) = go {
            self.move_camera(to, now);
        }
    }

    /// Analysis (sections, zebra… later): empty for now, as a placeholder.
    fn browser_analysis(&mut self, ui: &mut egui::Ui) {
        let mut edit: Option<usize> = None;
        let mut delete: Option<usize> = None;
        self.folder(ui, "folder_analysis", Icon::Folder, "Analysis", None, |app, ui| {
            if app.sections.is_empty() {
                ui.weak("Nothing yet (Inspect → Section Analysis)");
            }
            for k in 0..app.sections.len() {
                let r = browser_row(ui, |ui| {
                    if icons::eye(ui, app.sections[k].shown, "Show or hide this section").clicked() {
                        let on = !app.sections[k].shown;
                        // One section at a time, as the view can show one.
                        for s in app.sections.iter_mut() {
                            s.shown = false;
                        }
                        app.sections[k].shown = on;
                    }
                    icons::small(ui, Icon::Section);
                    ui.selectable_label(false, app.sections[k].name.as_str())
                })
                .inner;
                if r.double_clicked() {
                    edit = Some(k);
                }
                r.context_menu(|ui| {
                    if ui.button("Edit Section Analysis").clicked() {
                        edit = Some(k);
                        ui.close();
                    }
                    if ui.button("Delete").clicked() {
                        delete = Some(k);
                        ui.close();
                    }
                });
            }
        });
        if let Some(k) = edit {
            self.section_dialog = Some(section_dialog::SectionDialog::edit(k, &self.sections[k]));
        }
        if let Some(k) = delete {
            self.sections.remove(k);
            self.section_dialog = None;
        }
    }

    /// Construction planes and axes.
    fn browser_construction(&mut self, ui: &mut egui::Ui) {
        if self.axes.is_empty() && self.planes.is_empty() {
            return;
        }
        let any = (0..self.axes.len()).any(|k| !self.axis_hidden.contains(&k)) || (0..self.planes.len()).any(|k| !self.plane_hidden.contains(&k));
        let clicked = self.folder(ui, "folder_construction", Icon::Folder, "Construction", Some(any), |app, ui| {
            for k in 0..app.planes.len() {
                browser_row(ui, |ui| {
                    let shown = !app.plane_hidden.contains(&k);
                    if icons::eye(ui, shown, "Show or hide this plane").clicked() {
                        if shown {
                            app.plane_hidden.insert(k);
                        } else {
                            app.plane_hidden.remove(&k);
                        }
                    }
                    icons::small(ui, Icon::Plane);
                    ui.label(app.planes[k].name.as_str());
                });
            }
            for k in 0..app.axes.len() {
                browser_row(ui, |ui| {
                    let shown = !app.axis_hidden.contains(&k);
                    if icons::eye(ui, shown, "Show or hide this axis").clicked() {
                        if shown {
                            app.axis_hidden.insert(k);
                        } else {
                            app.axis_hidden.remove(&k);
                        }
                    }
                    icons::small(ui, Icon::Axis);
                    ui.label(app.axes[k].name.as_str());
                });
            }
        });
        if clicked {
            if any {
                self.plane_hidden = (0..self.planes.len()).collect();
                self.axis_hidden = (0..self.axes.len()).collect();
            } else {
                self.plane_hidden.clear();
                self.axis_hidden.clear();
            }
        }
    }

    fn browser_bodies(&mut self, ui: &mut egui::Ui) {
        let any = self.body_ids.iter().any(|id| !self.hidden_bodies.contains(id));
        let clicked = self.folder(ui, "folder_bodies", Icon::Folder, "Bodies", Some(any), |app, ui| app.body_rows(ui));
        if clicked {
            if any {
                self.hidden_bodies.extend(self.body_ids.iter().copied());
            } else {
                for id in &self.body_ids {
                    self.hidden_bodies.remove(id);
                }
            }
        }
    }

    fn browser_sketches(&mut self, ui: &mut egui::Ui) {
        let any = (0..self.sketches.len()).any(|i| self.sketch_visible(i));
        let clicked = self.folder(ui, "folder_sketches", Icon::Folder, "Sketches", Some(any), |app, ui| app.sketch_rows(ui));
        if clicked {
            for i in 0..self.sketches.len() {
                self.sketch_shown.insert(i, !any);
            }
        }
    }

    fn body_rows(&mut self, ui: &mut egui::Ui) {
        let mut rename_done: Option<(BodyId, Option<String>)> = None;
        let (mut remove, mut delete_maker): (Option<Vec<BodyId>>, Option<Feature>) = (None, None);
        self.selected_bodies.retain(|id| self.body_ids.contains(id));
        for id in self.body_ids.clone() {
            browser_row(ui, |ui| {
                let shown = !self.hidden_bodies.contains(&id);
                if icons::eye(ui, shown, "Show or hide this body").clicked() {
                    if shown {
                        self.hidden_bodies.insert(id);
                    } else {
                        self.hidden_bodies.remove(&id);
                    }
                }
                // The body's colour, as a small swatch.
                let (swatch, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                ui.painter().rect_filled(swatch, 2.0, self.body_color(id));
                if let Some((rid, text)) = self.renaming.as_mut().filter(|(rid, _)| *rid == id) {
                    let edit = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                    edit.request_focus();
                    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                    if enter {
                        rename_done = Some((*rid, Some(text.trim().to_string())));
                    } else if esc || edit.lost_focus() {
                        rename_done = Some((*rid, None));
                    }
                } else {
                    let name = self.body_name(id);
                    let picked = self.dialog_bodies().map(|b| b.contains(&id));
                    let selected = picked.unwrap_or_else(|| self.selected_bodies.contains(&id));
                    let r = ui
                        .selectable_label(selected, name.clone())
                        .on_hover_text("Click to select (Ctrl/Shift adds) · double-click to rename · right-click for more");
                    if r.double_clicked() {
                        self.renaming = Some((id, name.clone()));
                    } else if r.clicked() && picked.is_some() {
                        // A dialog is picking bodies: the browser picks for it, as the view does.
                        self.dialog_pick_body(id);
                    } else if r.clicked() {
                        let add = ui.input(|i| i.modifiers.ctrl || i.modifiers.shift || i.modifiers.command);
                        if add {
                            if selected {
                                self.selected_bodies.retain(|b| *b != id);
                            } else {
                                self.selected_bodies.push(id);
                            }
                        } else if selected && self.selected_bodies.len() == 1 {
                            self.selected_bodies.clear();
                        } else {
                            self.selected_bodies = vec![id];
                        }
                    }
                    if r.secondary_clicked() && !selected {
                        self.selected_bodies = vec![id];
                    }
                    r.context_menu(|ui| {
                        // Fusion's two ways to get rid of a body.
                        let n = self.selected_bodies.len().max(1);
                        let what = if n == 1 { "this body".to_string() } else { format!("the {n} selected bodies") };
                        if ui.button("Remove").on_hover_text(format!("Take {what} out from here on (a Remove feature in the timeline; Delete key)")).clicked() {
                            remove = Some(if self.selected_bodies.is_empty() { vec![id] } else { self.selected_bodies.clone() });
                            ui.close();
                        }
                        let maker = id.feature;
                        let maker_name = self.feature_name_mut(maker).map(|s| s.clone()).unwrap_or_default();
                        let siblings = self.body_ids.iter().filter(|b| b.feature == maker).count();
                        let tip = if siblings > 1 {
                            format!("Deletes {maker_name}, which made this body — and its other {} bod{}", siblings - 1, if siblings == 2 { "y" } else { "ies" })
                        } else {
                            format!("Deletes {maker_name}, the feature that made this body")
                        };
                        if ui.button(format!("Delete ({maker_name})")).on_hover_text(tip).clicked() {
                            delete_maker = Some(maker);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Rename").clicked() {
                            self.renaming = Some((id, name));
                            ui.close();
                        }
                        ui.menu_button("Color…", |ui| {
                            let c = self.body_color(id);
                            let mut c = c;
                            if egui::color_picker::color_picker_color32(ui, &mut c, egui::color_picker::Alpha::Opaque) {
                                self.body_colors.insert(id, [c.r(), c.g(), c.b()]);
                            }
                        });
                        if self.body_colors.contains_key(&id) && ui.button("Use the default color").clicked() {
                            self.body_colors.remove(&id);
                            ui.close();
                        }
                        if self.body_names.contains_key(&id) && ui.button("Use the feature's name").clicked() {
                            self.body_names.remove(&id);
                            ui.close();
                        }
                    });
                }
            });
        }
        // Delete key with bodies selected: Remove them (the safe one; the
        // right-click menu has Delete, which deletes the making feature).
        let typing = ui.ctx().text_edit_focused();
        if self.active.is_none() && !typing && !self.selected_bodies.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Delete)) {
            remove = Some(self.selected_bodies.clone());
        }
        if let Some(bodies) = remove {
            self.remove_bodies(bodies);
        }
        match delete_maker {
            Some(Feature::Extrude(k)) => self.delete_extrude(k),
            Some(Feature::Revolve(k)) => self.delete_revolve(k),
            Some(Feature::Copy(k)) => self.delete_copy(k),
            _ => {}
        }
        if let Some((id, name)) = rename_done {
            self.renaming = None;
            match name {
                Some(n) if !n.is_empty() => {
                    self.body_names.insert(id, n);
                }
                Some(_) => {
                    self.body_names.remove(&id); // emptied: back to the feature's name
                }
                None => {}
            }
        }
    }

    fn sketch_rows(&mut self, ui: &mut egui::Ui) {
        let (mut edit, mut delete, mut toggle, mut redefine) = (None, None, None, None);
        for (i, s) in self.sketches.iter().enumerate() {
            browser_row(ui, |ui| {
                let shown = self.sketch_visible(i);
                if icons::eye(ui, shown, "Show or hide this sketch").clicked() {
                    toggle = Some((i, !shown));
                }
                icons::small(ui, Icon::Sketch);
                let r = ui
                    .selectable_label(self.active == Some(i), format!("{} ({})", s.name, s.plane_name()))
                    .on_hover_text("Click to edit · right-click for more");
                if r.clicked() {
                    edit = Some(i);
                }
                r.context_menu(|ui| {
                    let uses = self.extrudes.iter().filter(|e| e.sketch == i).count();
                    let label = if uses == 0 {
                        "Delete".to_string()
                    } else {
                        format!("Delete (and the {uses} extrude{} using it)", if uses == 1 { "" } else { "s" })
                    };
                    if ui.button("Redefine Sketch Plane").on_hover_text("Move this sketch onto another plane or face").clicked() {
                        redefine = Some(i);
                        ui.close();
                    }
                    if ui.button(label).clicked() {
                        delete = Some(i);
                        ui.close();
                    }
                });
            });
        }
        if let Some((i, shown)) = toggle {
            self.sketch_shown.insert(i, shown);
        }
        if let Some(i) = redefine {
            self.start_redefine(i);
        }
        if let Some(i) = delete {
            self.delete_sketch(i);
        } else if let Some(i) = edit {
            self.edit_sketch(i);
        }
    }

    /// Right panel while extruding.
    /// The flat face of a visible body under a ray: (distance, body index, its plane).
    pub(crate) fn pick_face(&mut self, ray: &Ray) -> Option<(f32, usize, FacePlane)> {
        self.ensure_model(Slot::Committed);
        let tol = self.camera.distance as f64 * 1e-3;
        let m = self.model(Slot::Committed);
        m.bodies
            .iter()
            .enumerate()
            .filter(|(_, body)| !self.hidden_bodies.contains(&body.id))
            .filter_map(|(b, body)| model::ray_hit_tri(&body.mesh, ray.origin, ray.dir).map(|(t, tri)| (t, tri, b)))
            .min_by(|x, y| x.0.total_cmp(&y.0))
            .and_then(|(t, tri, b)| {
                let body = &m.bodies[b];
                let n = body.mesh.normal(tri).as_dvec3();
                let p = ray.at(t).as_dvec3();
                // Only flat faces: the triangle must lie on one of the body's planar faces.
                body.flat_faces.iter().find(|(fnormal, q)| fnormal.dot(n) > 0.9999 && fnormal.dot(p - *q).abs() < tol).map(
                    |&(fnormal, q)| {
                        // Remember if this face is the far end of an extrude/revolve,
                        // so the sketch moves with it.
                        let mut face = FacePlane::from_face(fnormal, q);
                        face.follow = kernel::follow_for_plane(&m.caps, fnormal, q, FOLLOW_TOL);
                        (t, b, face)
                    },
                )
            })
    }

    /// Which way (+1 / -1 along the sketch's normal) an extrude goes into the
    /// existing bodies: toward where they are. -1 when there's nothing to go into.
    pub(crate) fn cut_direction(&self, sketch: usize) -> f64 {
        let Some(cache) = self.caches[Slot::Committed as usize].as_ref() else { return -1.0 };
        let (mut sum, mut n) = (Vec3::ZERO, 0usize);
        for b in &cache.model.bodies {
            for p in &b.mesh.positions {
                sum += *p;
                n += 1;
            }
        }
        if n == 0 || sketch >= self.sketches.len() {
            return -1.0;
        }
        let plane = self.plane_of(sketch);
        if ((sum / n as f32) - plane.origin).dot(plane.normal()) > 1e-4 { 1.0 } else { -1.0 }
    }

    /// Does the extrude being set up reach into any existing body? None until
    /// there is a profile to judge.
    pub(crate) fn extrude_touches_bodies(&self) -> Option<bool> {
        let d = self.dialog.as_ref()?;
        let s = d.sketch?;
        let sk = self.sketches.get(s)?;
        let regions = find_regions(sk);
        let plane = self.plane_of(s);
        let points: Vec<Vec3> = d.profiles.iter().filter_map(|&p| regions.get(p)).flat_map(|r| r.outer.iter().map(|&q| plane.to_world(q))).collect();
        if points.is_empty() {
            return None;
        }
        let cache = self.caches[Slot::Committed as usize].as_ref()?;
        let bounds: Vec<(Vec3, Vec3)> = cache.model.bodies.iter().filter_map(|b| model::mesh_bounds(&b.mesh)).collect();
        let ex = Extrude { op: d.op, name: String::new(), sketch: s, profiles: Vec::new(), distance: d.distance, extent: d.extent, distance2: d.distance2, taper: d.taper, target: d.target };
        let (from, to) = ex.span();
        let touches = |len: f64| model::prism_touches(&points, plane.normal(), len as f32, &bounds, 1e-3);
        Some(touches(to) || (from != 0.0 && touches(from)))
    }

    pub(crate) fn extrude_panel(&mut self, ui: &mut egui::Ui) {
        let unit = self.units;
        // Until the user picks an operation: Join what it touches, else a new body.
        if let (Some(touches), true) = (self.extrude_touches_bodies(), self.dialog.as_ref().is_some_and(|d| !d.op_chosen)) {
            let toward = self.dialog.as_ref().and_then(|d| d.sketch).map(|s| self.cut_direction(s));
            if let Some(d) = self.dialog.as_mut() {
                // Pulled into the body: Cut. Away from it: Join. Clear of every body: New Body.
                let into = d.extent == model::Extent::OneSide && toward.is_some_and(|t| d.distance * t > 0.0);
                d.op = match (touches, into) {
                    (true, true) => Operation::Cut,
                    (true, false) => Operation::Join,
                    _ => Operation::NewBody,
                };
            }
        }
        let dialog_error = self.dialog_error.clone();
        let toward = self.dialog.as_ref().and_then(|d| d.sketch).map(|s| self.cut_direction(s));
        let Some(d) = self.dialog.as_mut() else { return };
        // Cut chosen before the profile was: aim it into the body once the sketch is known.
        if let (Operation::Cut, Some(s), true) = (d.op, d.sketch, d.cut_aimed_for != d.sketch) {
            d.cut_aimed_for = Some(s);
            d.distance = d.distance.abs() * toward.unwrap_or(-1.0);
            self.set_distance_text_pending = true;
        }
        let plane_names: Vec<String> = self.planes.iter().map(|p| p.name.clone()).collect();
        let Some(d) = self.dialog.as_mut() else { return };
        ui.heading(if d.editing.is_some() { "Edit Extrude" } else { "Extrude" });
        ui.horizontal(|ui| {
            ui.label(format!("Profiles: {} selected", d.profiles.len()));
            if !d.profiles.is_empty() && ui.small_button("×").on_hover_text("Clear the profiles").clicked() {
                d.profiles.clear();
            }
        });
        ui.weak("Click shaded regions to add or remove them.");
        ui.separator();
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("Extent");
            for ex in model::Extent::ALL {
                if ui.selectable_label(d.extent == ex, ex.name()).clicked() && d.extent != ex {
                    d.extent = ex;
                    // To Object: pick what it runs up to straight away (if nothing yet).
                    d.picking_target = ex == model::Extent::ToObject && d.target.is_none();
                    changed = true;
                }
            }
        });
        if d.extent == model::Extent::ToObject {
            // What it runs up to: click the field, then a plane or flat face in the view.
            ui.horizontal(|ui| {
                ui.label("Object");
                let name = match d.target {
                    _ if d.picking_target => "Click a plane or flat face…".to_string(),
                    Some(model::ExtrudeTarget::Origin(op)) => format!("{} plane", op.name()),
                    Some(model::ExtrudeTarget::Plane(k)) => plane_names.get(k).cloned().unwrap_or_else(|| "a plane".into()),
                    Some(model::ExtrudeTarget::Face { .. }) => "a body face".into(),
                    None => "Click to select".into(),
                };
                let text = if d.picking_target { egui::RichText::new(name).strong() } else { egui::RichText::new(name) };
                if ui.add(egui::Button::selectable(d.picking_target, text).min_size(egui::vec2(150.0, 0.0))).on_hover_text("Click, then click an origin or construction plane, or a flat body face").clicked() {
                    d.picking_target = !d.picking_target;
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.label(match d.extent {
                    model::Extent::OneSide => "Distance",
                    model::Extent::Symmetric => "Length",
                    _ => "Side 1",
                });
                changed |= ui
                    .add(units::length_drag(&mut d.distance, unit, 0.5))
                    .changed();
                if ui.button("Flip").clicked() {
                    d.distance = -d.distance;
                    changed = true;
                }
            });
        }
        if d.extent == model::Extent::OneSide {
            ui.horizontal(|ui| {
                ui.label("Taper");
                changed |= ui.add(egui::DragValue::new(&mut d.taper).speed(0.2).range(-89.0..=89.0).suffix("°")).changed();
            });
        }
        if d.extent == model::Extent::TwoSides {
            ui.horizontal(|ui| {
                ui.label("Side 2");
                changed |= ui.add(units::length_drag(&mut d.distance2, unit, 0.5).range(0.0..=f64::MAX)).changed();
            });
        }
        ui.horizontal(|ui| {
            ui.label("Operation");
            for op in Operation::ALL {
                if ui.selectable_label(d.op == op, op.name()).clicked() && d.op != op {
                    d.op_chosen = true;
                    // A cut goes into the body by itself; leaving Cut goes back out.
                    if op == Operation::Cut {
                        d.distance = d.distance.abs() * toward.unwrap_or(-1.0);
                    } else if d.op == Operation::Cut {
                        d.distance = d.distance.abs();
                    }
                    d.op = op;
                    changed = true;
                }
            }
        });
        let editing = d.editing;
        let has_profiles = !d.profiles.is_empty();
        if changed | std::mem::take(&mut self.set_distance_text_pending) {
            self.set_distance_text();
        }
        if let (Some(e), true) = (&dialog_error, has_profiles) {
            ui.colored_label(COL_FIXED, e.as_str());
        }
        if let Some(msg) = &self.message {
            ui.colored_label(COL_FIXED, msg.as_str());
        }
        ui.separator();
        let ok = has_profiles && dialog_error.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                self.commit_extrude();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_extrude();
            }
            if let Some(k) = editing {
                if ui.button("Delete").clicked() {
                    self.delete_extrude(k);
                }
            }
        });
    }

    /// Right panel while revolving.
    pub(crate) fn revolve_panel(&mut self, ui: &mut egui::Ui) {
        let dialog_error = self.dialog_error.clone();
        let axis_names: Vec<String> = self.axes.iter().map(|a| a.name.clone()).collect();
        let Some(d) = self.revolve_dialog.as_mut() else { return };
        ui.heading(if d.editing.is_some() { "Edit Revolve" } else { "Revolve" });
        ui.horizontal(|ui| {
            ui.label(format!("Profiles: {} selected", d.profiles.len()));
            if !d.profiles.is_empty() && ui.small_button("×").on_hover_text("Clear the profiles").clicked() {
                d.profiles.clear();
            }
        });
        ui.weak("Click shaded regions to add or remove them.");
        ui.separator();
        ui.label("Axis");
        ui.horizontal(|ui| {
            if ui.selectable_label(d.axis == RevolveAxis::SketchX && !d.picking_axis, "Sketch X").clicked() {
                d.axis = RevolveAxis::SketchX;
                d.picking_axis = false;
            }
            if ui.selectable_label(d.axis == RevolveAxis::SketchY && !d.picking_axis, "Sketch Y").clicked() {
                d.axis = RevolveAxis::SketchY;
                d.picking_axis = false;
            }
            let on_line = matches!(d.axis, RevolveAxis::Line(_));
            if ui.selectable_label(d.picking_axis || on_line, "A sketch line…").clicked() {
                d.picking_axis = true;
            }
            for (k, name) in axis_names.iter().enumerate() {
                if ui.selectable_label(d.axis == RevolveAxis::Construction(k) && !d.picking_axis, name.as_str()).clicked() {
                    d.axis = RevolveAxis::Construction(k);
                    d.picking_axis = false;
                }
            }
        });
        if d.picking_axis {
            ui.weak("Click a line in the profile's sketch.");
        }
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Angle");
            changed |= ui
                .add(egui::DragValue::new(&mut d.angle).speed(1.0).range(-360.0..=360.0).min_decimals(1).suffix("°"))
                .changed();
            if ui.button("Full").clicked() {
                d.angle = 360.0;
                changed = true;
            }
            if ui.button("Flip").clicked() {
                d.angle = -d.angle;
                changed = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Operation");
            for op in Operation::ALL {
                if ui.selectable_label(d.op == op, op.name()).clicked() {
                    d.op = op;
                }
            }
        });
        let editing = d.editing;
        let has_profiles = !d.profiles.is_empty();
        if changed {
            self.set_angle_text();
        }
        if let (Some(e), true) = (&dialog_error, has_profiles) {
            ui.colored_label(COL_FIXED, e.as_str());
        }
        if let Some(msg) = &self.message {
            ui.colored_label(COL_FIXED, msg.as_str());
        }
        ui.separator();
        let ok = has_profiles && dialog_error.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                self.commit_revolve();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_revolve();
            }
            if let Some(k) = editing {
                if ui.button("Delete").clicked() {
                    self.delete_revolve(k);
                }
            }
        });
    }

    /// Right panel while filleting / chamfering.
    pub(crate) fn edge_panel(&mut self, ui: &mut egui::Ui) {
        let unit = self.units;
        let dialog_error = self.dialog_error.clone();
        let Some(d) = self.edge_dialog.as_mut() else { return };
        ui.heading(if d.editing.is_some() { format!("Edit {}", d.kind.name()) } else { d.kind.name().to_string() });
        ui.horizontal(|ui| {
            for kind in [EdgeKind::Fillet, EdgeKind::Chamfer] {
                if ui.selectable_label(d.kind == kind, kind.name()).clicked() {
                    d.kind = kind;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(format!("Edges: {} selected", d.edges.len()));
            if !d.edges.is_empty() && ui.small_button("×").on_hover_text("Clear the edges").clicked() {
                d.edges.clear();
            }
        });
        ui.weak("Click edges (or a face, for all its edges) to add or remove them.");
        ui.checkbox(&mut d.tangent_chain, "Tangent Chain").on_hover_text("Picking an edge picks the edges running smoothly on from it");
        ui.separator();
        let mut changed = false;
        if d.kind == EdgeKind::Chamfer {
            // Fusion's Chamfer Type.
            let kind = if d.angle.is_some() { 2 } else if d.size2.is_some() { 1 } else { 0 };
            ui.horizontal(|ui| {
                ui.label("Type");
                for (k, name) in [(0, "Equal distance"), (1, "Two distances"), (2, "Distance and angle")] {
                    if ui.selectable_label(kind == k, name).clicked() && kind != k {
                        (d.size2, d.angle) = match k {
                            1 => (Some(d.size), None),
                            2 => (None, Some(45.0)),
                            _ => (None, None),
                        };
                        changed = true;
                    }
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(if d.kind == EdgeKind::Chamfer && (d.size2.is_some() || d.angle.is_some()) { "Distance" } else { d.kind.size_label() });
            changed |= ui
                .add(units::length_drag(&mut d.size, unit, 0.1).range(0.001..=1.0e5))
                .changed();
        });
        if d.kind == EdgeKind::Chamfer {
            if let Some(d2) = d.size2.as_mut().filter(|_| d.angle.is_none()) {
                ui.horizontal(|ui| {
                    ui.label("Distance 2");
                    changed |= ui.add(units::length_drag(d2, unit, 0.1).range(0.001..=1.0e5)).changed();
                });
            }
            if let Some(a) = d.angle.as_mut() {
                ui.horizontal(|ui| {
                    ui.label("Angle");
                    changed |= ui.add(egui::DragValue::new(a).speed(0.5).range(0.1..=89.9).suffix("°")).changed();
                });
            }
        }
        let editing = d.editing;
        let has_edges = !d.edges.is_empty();
        if changed {
            self.set_edge_size_text();
        }
        if let (Some(e), true) = (&dialog_error, has_edges) {
            ui.colored_label(COL_FIXED, e.as_str());
        }
        if let Some(msg) = &self.message {
            ui.colored_label(COL_FIXED, msg.as_str());
        }
        ui.separator();
        let ok = has_edges && dialog_error.is_none();
        ui.horizontal(|ui| {
            if ui.add_enabled(ok, egui::Button::new("OK")).clicked() {
                self.commit_edge_feature();
            }
            if ui.button("Cancel").clicked() {
                self.cancel_edge_dialog();
            }
            if let Some(k) = editing {
                if ui.button("Delete").clicked() {
                    self.delete_edge_feature(k);
                }
            }
        });
    }

    /// The width and height boxes beside a rectangle being drawn. Empty boxes
    /// show the size the cursor makes; Tab swaps boxes, Enter places it, Esc
    /// lets go of what was typed.
    /// Fusion's name list under a box being typed in: the parameters
    /// holding the letters at the end of `text` (up to 6, with values), drawn
    /// at `at`. ↑/↓ choose; Tab, Enter or a click puts the name in (returned;
    /// the caller keeps the box focused). Call before the box is drawn, so
    /// the keys are taken first.
    pub(crate) fn name_suggestions(params: &[expr::UserParameter], ctx: &egui::Context, id: egui::Id, text: &mut String, at: egui::Pos2) -> Option<String> {
        let names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let Some((start, found)) = expr::suggestions(text, &names, 6) else {
            ctx.data_mut(|d| d.remove::<(usize, Vec<egui::Rect>)>(id));
            return None;
        };
        let (mut pick, rows): (usize, Vec<egui::Rect>) = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
        pick = pick.min(found.len() - 1);
        let (take, up, down, release) = ctx.input_mut(|i| {
            let tab = i.consume_key(egui::Modifiers::NONE, egui::Key::Tab);
            let enter = i.consume_key(egui::Modifiers::NONE, egui::Key::Enter);
            let up = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            let down = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            (tab || enter, up, down, i.pointer.primary_released().then(|| i.pointer.interact_pos()).flatten())
        });
        if up {
            pick = pick.saturating_sub(1);
        }
        if down {
            pick = (pick + 1).min(found.len() - 1);
        }
        // (A click on a row counts when the button comes up: that's when the box would lose focus.)
        let clicked = release.and_then(|p| rows.iter().position(|r| r.contains(p))).filter(|k| *k < found.len());
        let chosen = clicked.or(take.then_some(pick)).map(|k| found[k].clone());
        if let Some(name) = &chosen {
            text.truncate(start);
            text.push_str(name);
            ctx.data_mut(|d| d.remove::<(usize, Vec<egui::Rect>)>(id));
            return chosen;
        }
        let values = expr::evaluate(params);
        let mut new_rows = Vec::new();
        egui::Area::new(id.with("list")).order(egui::Order::Tooltip).fixed_pos(at).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                for (k, name) in found.iter().enumerate() {
                    let value = params.iter().zip(&values).find(|(p, _)| &p.name == name).and_then(|(p, v)| v.as_ref().ok().map(|v| p.unit.show(*v))).unwrap_or_default();
                    let r = ui.add(egui::Button::selectable(k == pick, format!("{name}   {value}")).min_size(egui::vec2(160.0, 0.0)).sense(egui::Sense::hover()));
                    new_rows.push(r.rect);
                }
            });
        });
        ctx.data_mut(|d| d.insert_temp(id, (pick, new_rows)));
        None
    }

    pub(crate) fn rect_size_boxes(&mut self, ctx: &egui::Context) {
        let sizing = self.active.is_some() && matches!(self.editor.tool, Tool::Rectangle | Tool::RectangleCenter) && self.editor.start.is_some();
        let Some((at, live)) = self.rect_boxes.filter(|_| sizing) else { return };
        let unit = self.units;
        let ids = [egui::Id::new("rect_width"), egui::Id::new("rect_height")];
        let (tab, enter, esc) = ctx.input(|i| (i.key_pressed(egui::Key::Tab), i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        let ours = ctx.memory(|m| m.focused()).and_then(|f| ids.iter().position(|&id| id == f));
        let mut input = self.rect_input.clone().unwrap_or_default();
        if let (Some(k), true) = (ours, tab) {
            input.field = k;
            input.tab();
        }
        let mut place = false;
        let mut drop = false;
        // Parameter names for the box being typed in (Enter then fills the name, not places).
        let mut named = false;
        if let Some(k) = ours {
            if Self::name_suggestions(&self.parameters, ctx, egui::Id::new("rect_suggest"), &mut input.text[k], at[k] + egui::vec2(-40.0, 16.0)).is_some() {
                (input.field, input.focus, named) = (k, true, true);
            }
        }
        for k in 0..2 {
            egui::Area::new(ids[k].with("area")).order(egui::Order::Foreground).fixed_pos(at[k]).pivot(egui::Align2::CENTER_CENTER).show(ctx, |ui| {
                egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(4, 2)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut out = egui::TextEdit::singleline(&mut input.text[k])
                            .id(ids[k])
                            .desired_width(56.0)
                            .lock_focus(true)
                            .hint_text(unit.plain(live[k]))
                            .show(ui);
                        ui.label(unit.name());
                        if input.focus && input.field == k {
                            // Focus with the cursor after what was typed.
                            out.response.request_focus();
                            let end = egui::text::CCursor::new(input.text[k].chars().count());
                            out.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                            out.state.store(ui.ctx(), ids[k]);
                            input.focus = false;
                        }
                        if out.response.lost_focus() && !named {
                            place |= enter;
                            drop |= esc;
                        }
                    });
                });
            });
        }
        if drop {
            self.rect_input = None; // free size again; another Esc leaves the tool
        } else {
            let any = input.text.iter().any(|t| !t.is_empty()) || ours.is_some() || input.focus;
            self.rect_input = any.then_some(input);
            if place {
                self.rect_place = true;
            }
        }
    }

    /// The floating number box (dimension values, extrude distance).
    pub(crate) fn float_editor(&mut self, ctx: &egui::Context) {
        let (Some(anchor), Some(fe)) = (self.float_anchor, self.float.as_mut()) else { return };
        let target = fe.target;
        let degrees = match target {
            EditTarget::RevolveAngle | EditTarget::PlaneAngle => true,
            EditTarget::Dimension(ci) => self
                .active
                .and_then(|i| self.sketches.get(i))
                .and_then(|sk| sk.constraints.get(ci))
                .is_some_and(|c| matches!(c, Constraint::Angle(..))),
            _ => false,
        };
        // A dimension is shown and typed in its own unit (if it was typed in one).
        let unit = match (target, self.active) {
            (EditTarget::Dimension(ci), Some(i)) => self.sketches.get(i).and_then(|sk| sk.dim_unit(ci)).unwrap_or(self.units),
            _ => self.units,
        };
        let mut typed_unit = None; // the unit written in the box, when Enter was pressed
        let mut typed_text = None; // and what was written
        let mut esc_hit = false;
        let mut commit: Option<Option<f64>> = None; // Some(parsed) when Enter was pressed
        let mut close = false;
        let mut live: Option<f64> = None;
        let mut corner_text: Option<String> = None;
        let mut sides_typed: Option<usize> = None;
        let mut edge_sizes: Option<(f64, Option<f64>)> = None;
        let mut extrude_ok = false;
        let mut extrude_cancel = false;
        // Parameter names for what's being typed: a list under the box.
        let takes_names = !matches!(target, EditTarget::PolygonSides | EditTarget::Corner(_));
        let chosen = if takes_names { Self::name_suggestions(&self.parameters, ctx, egui::Id::new("float_suggest"), &mut fe.text, anchor + egui::vec2(0.0, 38.0)) } else { None };
        if chosen.is_some() {
            fe.focus = true;
            fe.keep_text = true;
        }
        egui::Area::new(egui::Id::new("float_edit")).order(egui::Order::Foreground).fixed_pos(anchor).show(
            ctx,
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let mut out = egui::TextEdit::singleline(&mut fe.text)
                            .id(egui::Id::new("float_edit_text"))
                            .desired_width(70.0)
                            .show(ui);
                        ui.label(if degrees {
                            "°"
                        } else if target == EditTarget::PolygonSides {
                            "sides"
                        } else {
                            unit.name()
                        });
                        // (A click on a suggested name isn't leaving the box.)
                        let (lost, changed, id) = (out.response.lost_focus() && chosen.is_none(), out.response.changed() || chosen.is_some(), out.response.id);
                        if fe.focus {
                            // Focus and select everything, so typing replaces the value.
                            out.response.request_focus();
                            let range = if fe.keep_text {
                                egui::text::CCursorRange::one(egui::text::CCursor::new(fe.text.chars().count()))
                            } else {
                                egui::text::CCursorRange::select_all(&out.galley)
                            };
                            out.state.cursor.set_char_range(Some(range));
                            out.state.store(ui.ctx(), id);
                            fe.focus = false;
                        }
                        // Lengths in the design's units (or as typed: "22 mm", "1 in");
                        // angles in degrees whatever the units.
                        let parsed = if degrees { units::parse_angle(&fe.text) } else { units::parse_length(&fe.text, unit) };
                        let (enter, esc) =
                            ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                        match target {
                            // Applies as you type; Enter just puts the cursor back on the canvas.
                            EditTarget::PolygonSides => {
                                if changed {
                                    sides_typed = fe.text.trim().parse::<usize>().ok();
                                }
                                if lost && (enter || esc) {
                                    close = true;
                                }
                            }
                            // Stays open until Enter / Esc, so its arrow can be dragged.
                            EditTarget::Corner(_) => {
                                if lost {
                                    if enter {
                                        corner_text = Some(fe.text.clone());
                                    }
                                    if enter || esc {
                                        close = true;
                                    }
                                }
                            }
                            EditTarget::Dimension(_) | EditTarget::NewCircle => {
                                if changed {
                                    live = parsed;
                                }
                                if lost {
                                    esc_hit = esc;
                                    if enter {
                                        commit = Some(parsed);
                                        typed_unit = units::typed_unit(&fe.text);
                                        typed_text = Some(fe.text.clone());
                                    }
                                    close = true; // Esc or clicking away cancels
                                }
                            }
                            EditTarget::ExtrudeDistance | EditTarget::EdgeSize | EditTarget::RevolveAngle | EditTarget::PlaneOffset | EditTarget::PlaneAngle | EditTarget::OffsetDistance => {
                                if changed {
                                    live = parsed;
                                }
                                if lost && enter {
                                    live = parsed;
                                    extrude_ok = true;
                                }
                                if target == EditTarget::EdgeSize && (changed || (lost && enter)) {
                                    edge_sizes = sketch::parse_sizes(&fe.text, unit);
                                }
                                if lost && esc {
                                    extrude_cancel = true;
                                }
                            }
                        }
                    });
                });
            },
        );
        // A feature's box typed as an expression: kept with the feature at OK.
        let field = match target {
            EditTarget::ExtrudeDistance | EditTarget::OffsetDistance => Some(doc::Field::Distance),
            EditTarget::EdgeSize => Some(doc::Field::Size),
            EditTarget::RevolveAngle | EditTarget::PlaneAngle => Some(doc::Field::Angle),
            EditTarget::PlaneOffset => Some(doc::Field::Offset),
            _ => None,
        };
        if let (Some(field), Some(text)) = (field, self.float.as_ref().map(|f| f.text.clone())) {
            self.pending_exprs.retain(|(f, _)| *f != field);
            if units::is_expression(&text) {
                self.pending_exprs.push((field, text.trim().to_string()));
            }
        }
        match target {
            EditTarget::Corner(p) => {
                if let Some(text) = corner_text {
                    match sketch::parse_sizes(&text, unit) {
                        Some((d1, d2)) => self.finish_corner(p, d1, d2),
                        None => self.message = Some("Type a number (a chamfer can take two: 2, 5)".into()),
                    }
                }
                if close {
                    self.float = None;
                }
            }
            EditTarget::PolygonSides => {
                if let Some(n) = sides_typed.filter(|n| (3..=Editor::MAX_SIDES).contains(n)) {
                    self.editor.polygon_sides = n;
                }
                if close {
                    self.float = None;
                }
            }
            EditTarget::NewCircle => {
                // A line follows the length as it's typed (and keeps it when the
                // box is left by clicking in the view: that click sets the angle).
                if self.editor.tool == Tool::Line {
                    if let Some(v) = live.filter(|v| *v > 0.0) {
                        self.line_length = Some(v);
                    }
                    if esc_hit {
                        self.line_length = None;
                    }
                }
                match commit {
                    Some(Some(v)) => match self.editor.tool {
                        Tool::Slot => self.finish_slot_typed(v),
                        // A line keeps the length; the next click sets its angle.
                        Tool::Line if v > 0.0 => {
                            self.line_length = Some(v);
                            self.line_length_text = typed_text.clone();
                        }
                        Tool::Offset => self.finish_offset_typed(v),
                        _ => {
                            self.finish_circle_typed(v);
                            if let Some(t) = &typed_text {
                                self.link_new_dimension(v, t);
                            }
                        }
                    },
                    Some(None) => self.message = Some("Type a number".into()),
                    None => {}
                }
                if close {
                    self.float = None;
                }
            }
            EditTarget::Dimension(ci) => {
                match commit {
                    Some(Some(v)) => {
                        self.set_dimension(ci, v);
                        // An expression is kept: the dimension follows the parameters it uses.
                        let text = typed_text.filter(|t| units::is_expression(t));
                        if let Some(sk) = self.active.and_then(|i| self.sketches.get_mut(i)) {
                            sk.set_dim_expr(ci, text);
                        }
                        // Typed in another unit: the dimension keeps showing in it
                        // (typed in the design's: back to following the design).
                        if let (Some(u), Some(i), false) = (typed_unit, self.active, degrees) {
                            let keep = (u != self.units).then_some(u);
                            if let Some(sk) = self.sketches.get_mut(i) {
                                sk.set_dim_unit(ci, keep);
                            }
                        }
                    }
                    Some(None) => {
                        // Say what's wrong with an expression ("no parameter called widht").
                        let kind = if degrees { expr::Kind::Angle } else { expr::Kind::Length };
                        let why = typed_text.as_deref().and_then(|t| expr::eval(t, kind, unit).err());
                        self.message = Some(why.map_or("Type a number".into(), |w| format!("Can't use that: {w}")));
                    }
                    None => {}
                }
                if close {
                    self.float = None;
                }
            }
            EditTarget::ExtrudeDistance => {
                if let (Some(v), Some(d)) = (live, self.dialog.as_mut()) {
                    d.distance = v;
                }
                if extrude_cancel {
                    self.cancel_extrude();
                } else if extrude_ok {
                    self.commit_extrude();
                    if self.dialog.is_some() {
                        self.focus_distance(); // not valid yet (e.g. no profile): stay in the box
                    }
                }
            }
            EditTarget::PlaneOffset | EditTarget::PlaneAngle => {
                if let (Some(v), Some(d)) = (live, self.plane_dialog.as_mut()) {
                    match d.angle.as_mut() {
                        Some(a) => a.degrees = v.clamp(-360.0, 360.0),
                        None => d.distance = v,
                    }
                }
                if extrude_cancel {
                    self.close_plane_dialog();
                } else if extrude_ok {
                    self.commit_plane();
                    if self.plane_dialog.is_some() {
                        self.focus_distance();
                    }
                }
            }
            EditTarget::OffsetDistance => {
                if let (Some(v), Some(d)) = (live, self.offset_dialog.as_mut()) {
                    d.offset.distance = v;
                }
                if extrude_cancel {
                    self.cancel_offset();
                } else if extrude_ok {
                    self.commit_offset();
                    if self.offset_dialog.is_some() {
                        self.focus_distance();
                    }
                }
            }
            EditTarget::RevolveAngle => {
                if let (Some(v), Some(d)) = (live, self.revolve_dialog.as_mut()) {
                    d.angle = v.clamp(-360.0, 360.0);
                }
                if extrude_cancel {
                    self.cancel_revolve();
                } else if extrude_ok {
                    self.commit_revolve();
                    if self.revolve_dialog.is_some() {
                        self.focus_distance();
                    }
                }
            }
            EditTarget::EdgeSize => {
                if let (Some((a, b)), Some(d)) = (edge_sizes, self.edge_dialog.as_mut()) {
                    if a > 0.0 {
                        d.size = a;
                        // (A fillet only has a radius.)
                        d.size2 = b.filter(|v| *v > 0.0 && d.kind == EdgeKind::Chamfer);
                    }
                }
                if extrude_cancel {
                    self.cancel_edge_dialog();
                } else if extrude_ok {
                    self.commit_edge_feature();
                    if self.edge_dialog.is_some() {
                        self.focus_distance();
                    }
                }
            }
        }
    }

    // ---------- viewport ----------

}

#[cfg(test)]
mod shots {
    use super::*;

    #[test]
    #[ignore]
    fn shots() {
        let dir = std::env::var("SHOT_DIR").unwrap_or_else(|_| "/tmp".into());
        let kinds = |tools: &[Tool], icon: fn(Tool) -> Icon| tools.iter().map(|&k| ToolItem { name: k.name(), icon: icon(k), tip: k.name(), enabled: true, on: false }).collect::<Vec<_>>();
        let families = [("Rectangle", kinds(&Tool::RECTANGLES, rect_icon)), ("Circle", kinds(&Tool::CIRCLES, circle_icon))];
        let items = [
            ToolItem { name: "Line", icon: Icon::Line, tip: "", enabled: true, on: false },
            ToolItem { name: "Rectangle", icon: Icon::Rectangle, tip: "", enabled: true, on: false },
            ToolItem { name: "Circle", icon: circle_icon(Tool::Circle), tip: "", enabled: true, on: false },
            ToolItem { name: "Combine", icon: Icon::Combine, tip: "", enabled: false, on: false },
            ToolItem { name: "Move", icon: Icon::Move, tip: "", enabled: false, on: false },
        ];
        for (name, hidden) in [("create_row", vec![]), ("create_row_tucked", vec!["Rectangle".to_string()])] {
            let (mut hidden, mut order) = (hidden, Vec::new());
            crate::ui_shot::shot(&format!("{dir}/{name}.png"), [360, 70], |ui| {
                ui.horizontal(|ui| {
                    tool_group(ui, "CREATE", "", &items, &families, &mut hidden, &mut order);
                });
            });
        }
    }
}
