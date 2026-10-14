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
fn tool_group(ui: &mut egui::Ui, label: &str, items: &[ToolItem], hidden: &mut Vec<String>, order: &mut Vec<String>) -> Option<&'static str> {
    let mut chosen = None;
    let mut toggle: Option<&'static str> = None;
    // The group's tools in the saved order (new ones where they're listed).
    let rank = |name: &str, k: usize| order.iter().position(|o| o == name).unwrap_or(1000 + k);
    let mut sorted: Vec<(usize, &ToolItem)> = items.iter().enumerate().collect();
    sorted.sort_by_key(|&(k, it)| rank(it.name, k));
    let items: Vec<&ToolItem> = sorted.into_iter().map(|(_, it)| it).collect();
    let drag_id = egui::Id::new(("tool_drag", label));
    let mut dragging: Option<&'static str> = ui.ctx().data(|d| d.get_temp(drag_id)).flatten();
    let mut dropped: Option<(&'static str, usize)> = None;
    ui.vertical(|ui| {
        let row = ui
            .horizontal(|ui| {
                let shown: Vec<&ToolItem> = items.iter().copied().filter(|it| !hidden.iter().any(|h| h == it.name)).collect();
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
                    order.retain(|o| !items.iter().any(|it| it.name == o));
                    order.extend(names.into_iter().chain(rest).map(String::from));
                }
            })
            .response
            .rect
            .width();
        ui.ctx().data_mut(|d| d.insert_temp(drag_id, dragging));
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
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_min_width(230.0);
            let tucked: Vec<&ToolItem> = items.iter().copied().filter(|it| hidden.iter().any(|h| h == it.name)).collect();
            if tucked.is_empty() {
                ui.weak("Every tool here is on the toolbar.");
            }
            for it in tucked {
                // The whole row is the button; ⋯ at its end pins it to the toolbar.
                let (row, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), if it.enabled { egui::Sense::click() } else { egui::Sense::hover() });
                let more = egui::Rect::from_min_size(row.right_top() - egui::vec2(26.0, 0.0), egui::vec2(24.0, 24.0));
                let over_more = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| more.contains(p));
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
    });
    ui.separator();
    if let Some(name) = toggle {
        match hidden.iter().position(|h| h == name) {
            Some(k) => {
                hidden.remove(k);
            }
            None => hidden.push(name.to_string()),
        }
    }
    chosen
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
                    (Cmd::New, "New                Ctrl+N"),
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
                let row = ui
                    .horizontal(|ui| {
                        ui.add_space(pad);
                        let start = ui.cursor().min.x;
                        add(ui);
                        ui.cursor().min.x - start
                    })
                    .inner;
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
                group(ui, "CREATE", |ui| {
                    for (tool, icon, tip) in [(Tool::Select, Icon::Select, "Select (Esc)"), (Tool::Line, Icon::Line, "Line (L)")] {
                        if icons::button(ui, icon, tip, true, self.editor.tool == tool).clicked() {
                            self.editor.set_tool(tool);
                        }
                    }
                    // Rectangle: the last rectangle tool used, ▾ for the others.
                    let rect_tool = self.rect_tool;
                    let tip = format!("{} (R)", rect_tool.name());
                    if icons::button(ui, rect_icon(rect_tool), &tip, true, self.editor.tool == rect_tool).clicked() {
                        self.editor.set_tool(rect_tool);
                    }
                    let more = icons::dropdown(ui, "More rectangle tools");
                    egui::Popup::menu(&more).show(|ui| {
                        for tool in Tool::RECTANGLES {
                            let row = ui.horizontal(|ui| {
                                let icon = icons::button(ui, rect_icon(tool), tool.name(), true, tool == rect_tool);
                                let name = ui.selectable_label(tool == rect_tool, tool.name());
                                icon.clicked() || name.clicked()
                            });
                            if row.inner {
                                self.rect_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    if icons::button(ui, Icon::Point, "Point: a sketch point (on a line if you click one)", true, self.editor.tool == Tool::Point)
                        .clicked()
                    {
                        self.editor.set_tool(Tool::Point);
                    }
                    // The circle button is whichever circle tool was used last;
                    // the arrow beside it picks another.
                    let circle = self.circle_tool;
                    let tip = format!("{} (C)", circle.name());
                    if icons::button(ui, circle_icon(circle), &tip, true, self.editor.tool == circle).clicked() {
                        self.editor.set_tool(circle);
                    }
                    let more_circles = icons::dropdown(ui, "More circle tools");
                    egui::Popup::menu(&more_circles).show(|ui| {
                        for tool in Tool::CIRCLES {
                            let row = ui.horizontal(|ui| {
                                let icon = icons::button(ui, circle_icon(tool), tool.name(), true, tool == circle);
                                let name = ui.selectable_label(tool == circle, tool.name());
                                icon.clicked() || name.clicked()
                            });
                            if row.inner {
                                self.circle_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    // Arc: the last arc tool used, ▾ for the other.
                    let arc_tool = self.arc_tool;
                    let tip = format!("{}", arc_tool.name());
                    let arc_icon = |t: Tool| if t == Tool::ArcCenter { Icon::ArcCenter } else { Icon::Arc3Point };
                    if icons::button(ui, arc_icon(arc_tool), &tip, true, self.editor.tool == arc_tool).clicked() {
                        self.editor.set_tool(arc_tool);
                    }
                    let more_arcs = icons::dropdown(ui, "More arc tools");
                    egui::Popup::menu(&more_arcs).show(|ui| {
                        for tool in Tool::ARCS {
                            let row = ui.horizontal(|ui| {
                                let icon = icons::button(ui, arc_icon(tool), tool.name(), true, tool == arc_tool);
                                let name = ui.selectable_label(tool == arc_tool, tool.name());
                                icon.clicked() || name.clicked()
                            });
                            if row.inner {
                                self.arc_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    // Polygon: the last polygon tool used, ▾ for the others, and the side count.
                    let poly_tool = self.poly_tool;
                    if icons::button(ui, Icon::Polygon, poly_tool.name(), true, Tool::POLYGONS.contains(&self.editor.tool)).clicked() {
                        self.editor.set_tool(poly_tool);
                    }
                    let more_polys = icons::dropdown(ui, "More polygon tools");
                    egui::Popup::menu(&more_polys).show(|ui| {
                        for tool in Tool::POLYGONS {
                            if ui.selectable_label(tool == poly_tool, tool.name()).clicked() {
                                self.poly_tool = tool;
                                self.editor.set_tool(tool);
                                ui.close();
                            }
                        }
                    });
                    if icons::button(ui, Icon::Slot, "Slot: click the two end centres, then the width", true, self.editor.tool == Tool::Slot).clicked() {
                        self.editor.set_tool(Tool::Slot);
                    }
                    if icons::button(ui, Icon::Spline, "Spline: click the points it goes through; Enter (or click the last point again) finishes", true, self.editor.tool == Tool::Spline).clicked() {
                        self.editor.set_tool(Tool::Spline);
                    }
                });
                let _ = i;
                let t = self.editor.tool;
                let pattern_on = |m: SpMode| self.sketch_pattern.as_ref().is_some_and(|d| d.mode == m);
                let items = [
                    ToolItem { name: "Fillet", icon: Icon::Fillet, tip: "Fillet: click a corner (or its two lines), type the radius", enabled: true, on: t == Tool::SketchFillet },
                    ToolItem { name: "Chamfer", icon: Icon::Chamfer, tip: "Chamfer: click a corner (or its two lines), type the distance — or two, like 2, 5 (first = first line)", enabled: true, on: t == Tool::SketchChamfer },
                    ToolItem { name: "Trim", icon: Icon::Trim, tip: "Trim: click the part of a line, circle or arc to remove, back to the nearest crossings", enabled: true, on: t == Tool::Trim },
                    ToolItem { name: "Extend", icon: Icon::Extend, tip: "Extend: click a line near the end to run it on to the next geometry", enabled: true, on: t == Tool::Extend },
                    ToolItem { name: "Offset", icon: Icon::Offset, tip: "Offset: click a line, arc or circle (its joined run follows), then click the side and distance", enabled: true, on: t == Tool::Offset },
                    ToolItem { name: "Rectangular Pattern", icon: Icon::RectPattern, tip: "Rectangular Pattern: select geometry first, then click this", enabled: true, on: pattern_on(SpMode::Rectangular) },
                    ToolItem { name: "Circular Pattern", icon: Icon::CircPattern, tip: "Circular Pattern: select geometry first, then click this", enabled: true, on: pattern_on(SpMode::Circular) },
                    ToolItem { name: "Mirror", icon: Icon::Mirror, tip: "Mirror: select geometry first, then click this and the mirror line", enabled: true, on: t == Tool::Mirror },
                    ToolItem { name: "Project", icon: Icon::Project, tip: "Project: click a body edge to bring it into the sketch (as locked construction geometry)", enabled: self.body_count > 0, on: t == Tool::Project },
                    ToolItem { name: "Construction", icon: Icon::Construction, tip: "Construction: toggle the selected geometry, or what you draw next", enabled: true, on: self.editor.construction },
                ];
                match tool_group(ui, "MODIFY", &items, &mut self.toolbar_hidden, &mut self.toolbar_order) {
                    Some("Fillet") => self.editor.set_tool(Tool::SketchFillet),
                    Some("Chamfer") => self.editor.set_tool(Tool::SketchChamfer),
                    Some("Trim") => self.editor.set_tool(Tool::Trim),
                    Some("Extend") => self.editor.set_tool(Tool::Extend),
                    Some("Offset") => self.editor.set_tool(Tool::Offset),
                    Some("Rectangular Pattern") => self.start_sketch_pattern(SpMode::Rectangular),
                    Some("Circular Pattern") => self.start_sketch_pattern(SpMode::Circular),
                    Some("Mirror") => self.start_mirror(),
                    Some("Project") => self.editor.set_tool(Tool::Project),
                    Some("Construction") => self.toggle_construction(),
                    _ => {}
                }
                let items: Vec<ToolItem> = CONSTRAINT_ACTIONS
                    .into_iter()
                    .map(|label| {
                        let (icon, tip) = match label {
                            "Dimension" => (
                                Icon::Dimension,
                                "Dimension (D): one line → length · circle → diameter · two lines → angle · two points → distance",
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
                            _ => (Icon::Fix, "Fix: lock points where they are"),
                        };
                        // Always clickable: with the right things selected it's added
                        // at once, otherwise it becomes the tool and you pick them.
                        ToolItem { name: label, icon, tip, enabled: true, on: self.editor.tool == Tool::Constrain(label) }
                    })
                    .collect();
                if let Some(label) = tool_group(ui, "CONSTRAINTS", &items, &mut self.toolbar_hidden, &mut self.toolbar_order) {
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
                group(ui, "FINISH SKETCH", |ui| {
                    if icons::button(ui, Icon::FinishSketch, "Finish Sketch", true, false).clicked() {
                        self.finish_sketch();
                    }
                });
            } else {
                // --- solid mode ---
                let can_profile = self.sketches.iter().any(|s| !find_regions(s).is_empty());
                let has_bodies = self.body_count > 0;
                group(ui, "CREATE", |ui| {
                    // Like Fusion: click, then click the plane (or flat face) in the view.
                    let tip = "Create Sketch (N): uses the selected plane, or click a plane or flat face";
                    if icons::button(ui, Icon::Sketch, tip, true, self.face_pick && !self.plane_pick).clicked() {
                        self.new_sketch_tool();
                    }
                    if icons::button(ui, Icon::Extrude, "Extrude (E)", can_profile, self.dialog.is_some()).clicked() {
                        self.open_extrude(None);
                    }
                    if icons::button(ui, Icon::Revolve, "Revolve: spin a profile around an axis", can_profile, self.revolve_dialog.is_some())
                        .clicked()
                    {
                        self.open_revolve(None);
                    }
                });
                group(ui, "CONSTRUCT", |ui| {
                    let tip = "Offset Plane: click an origin plane or a face, then set the distance";
                    if icons::button(ui, Icon::Plane, tip, true, self.plane_pick || self.plane_dialog.is_some()).clicked() {
                        let was = self.plane_pick;
                        self.cancel_all_dialogs();
                        self.plane_pick = !was;
                        self.face_pick = !was;
                    }
                    let tip = "Axis Along Edge: click a straight edge of a body";
                    if icons::button(ui, Icon::Axis, tip, has_bodies, self.axis_pick).clicked() {
                        let was = self.axis_pick;
                        self.cancel_all_dialogs();
                        self.axis_pick = !was;
                    }
                });
                let copy_mode = self.copy_dialog.as_ref().map(|d| d.mode);
                group(ui, "PATTERN", |ui| {
                    for (mode, icon, tip) in [
                        (CopyMode::Mirror, Icon::Mirror, "Mirror: copy bodies across an origin plane"),
                        (CopyMode::Rectangular, Icon::RectPattern, "Rectangular Pattern: copies in a grid"),
                        (CopyMode::Circular, Icon::CircPattern, "Circular Pattern: copies round an axis"),
                    ] {
                        if icons::button(ui, icon, tip, has_bodies, copy_mode == Some(mode)).clicked() {
                            self.open_copy(mode, None);
                        }
                    }
                });
                let edge_kind = self.edge_dialog.as_ref().map(|d| d.kind);
                group(ui, "MODIFY", |ui| {
                    if icons::button(ui, Icon::Fillet, "Fillet (F): round edges", has_bodies, edge_kind == Some(EdgeKind::Fillet))
                        .clicked()
                    {
                        self.open_edge_dialog(None, EdgeKind::Fillet);
                    }
                    if icons::button(ui, Icon::Chamfer, "Chamfer: bevel edges", has_bodies, edge_kind == Some(EdgeKind::Chamfer))
                        .clicked()
                    {
                        self.open_edge_dialog(None, EdgeKind::Chamfer);
                    }
                });
                group(ui, "INSPECT", |ui| {
                    if icons::button(ui, Icon::Measure, "Measure: click a corner, edge, face, body or sketch geometry — one for its size, two for distance and angle", has_bodies || !self.sketches.is_empty(), self.measure.is_some()).clicked() {
                        self.measure = if self.measure.is_some() { None } else { Some(measure::Measure::default()) };
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
        let mut eye_clicked = false;
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), egui::Id::new(id), false);
        state
            .show_header(ui, |ui| {
                browser_row(ui, |ui| {
                    if let Some(shown) = eye {
                        eye_clicked = icons::eye(ui, shown, if shown { "Hide everything in it" } else { "Show everything in it" }).clicked();
                    }
                    icons::small(ui, icon);
                    ui.label(name);
                });
            })
            .body(|ui| body(self, ui));
        eye_clicked
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

    /// Analysis (sections, zebra… later): empty for now, as a placeholder.
    fn browser_analysis(&mut self, ui: &mut egui::Ui) {
        self.folder(ui, "folder_analysis", Icon::Folder, "Analysis", None, |_, ui| {
            ui.weak("Nothing yet");
        });
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
                    let selected = self.selected_bodies.contains(&id);
                    let r = ui
                        .selectable_label(selected, name.clone())
                        .on_hover_text("Click to select (Ctrl/Shift adds) · double-click to rename · right-click for more");
                    if r.double_clicked() {
                        self.renaming = Some((id, name.clone()));
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
        let (mut edit, mut delete, mut toggle) = (None, None, None);
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
        ui.label(format!("Profiles: {} selected", d.profiles.len()));
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
        ui.label(format!("Profiles: {} selected", d.profiles.len()));
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
        ui.label(format!("Edges: {} selected", d.edges.len()));
        ui.weak("Click edges on a body to add or remove them.");
        ui.separator();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(d.kind.size_label());
            changed |= ui
                .add(units::length_drag(&mut d.size, unit, 0.1).range(0.001..=1.0e5))
                .changed();
        });
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

    /// The floating number box (dimension values, extrude distance).
    pub(crate) fn float_editor(&mut self, ctx: &egui::Context) {
        let (Some(anchor), Some(fe)) = (self.float_anchor, self.float.as_mut()) else { return };
        let target = fe.target;
        let degrees = match target {
            EditTarget::RevolveAngle => true,
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
        let mut esc_hit = false;
        let mut commit: Option<Option<f64>> = None; // Some(parsed) when Enter was pressed
        let mut close = false;
        let mut live: Option<f64> = None;
        let mut corner_text: Option<String> = None;
        let mut sides_typed: Option<usize> = None;
        let mut edge_sizes: Option<(f64, Option<f64>)> = None;
        let mut extrude_ok = false;
        let mut extrude_cancel = false;
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
                        let (lost, changed, id) = (out.response.lost_focus(), out.response.changed(), out.response.id);
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
                                    }
                                    close = true; // Esc or clicking away cancels
                                }
                            }
                            EditTarget::ExtrudeDistance | EditTarget::EdgeSize | EditTarget::RevolveAngle => {
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
                        Tool::Line if v > 0.0 => self.line_length = Some(v),
                        Tool::Offset => self.finish_offset_typed(v),
                        _ => self.finish_circle_typed(v),
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
                        // Typed in another unit: the dimension keeps showing in it
                        // (typed in the design's: back to following the design).
                        if let (Some(u), Some(i), false) = (typed_unit, self.active, degrees) {
                            let keep = (u != self.units).then_some(u);
                            if let Some(sk) = self.sketches.get_mut(i) {
                                sk.set_dim_unit(ci, keep);
                            }
                        }
                    }
                    Some(None) => self.message = Some("Type a number".into()),
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
