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

impl OpenCad {
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
        ui.horizontal(|ui| {
            // --- quick access ---
            group(ui, "FILE", |ui| {
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
                    ui.menu_button("Input device", |ui| {
                        ui.radio_value(&mut self.touchpad, false, "Mouse")
                            .on_hover_text("Scroll zooms · right/middle-drag pans · drag a dimension to move it");
                        ui.radio_value(&mut self.touchpad, true, "Touchpad").on_hover_text(
                            "Two-finger scroll pans · Shift+two-finger orbits · Ctrl+two-finger zooms · \
                             click a selected dimension to pick it up, click again to drop it",
                        );
                    });
                    ui.menu_button("Export", |ui| {
                        for f in export::Format::ALL {
                            if ui.button(format!("{}…", f.label())).clicked() {
                                self.cmd = Some(Cmd::Export(f));
                                ui.close();
                            }
                        }
                    });
                });
                if icons::button(ui, Icon::Save, "Save (Ctrl+S)", true, false).clicked() {
                    self.cmd = Some(Cmd::Save);
                }
                if icons::button(ui, Icon::Undo, "Undo (Ctrl+Z)", self.can_undo(), false).clicked() {
                    self.undo();
                }
                if icons::button(ui, Icon::Redo, "Redo (Ctrl+Shift+Z or Ctrl+Y)", self.history.can_redo(), false).clicked() {
                    self.redo();
                }
            });

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
                });
                let _ = i;
                group(ui, "MODIFY", |ui| {
                    for (tool, icon, tip) in [
                        (Tool::SketchFillet, Icon::Fillet, "Fillet: click a corner (or its two lines), type the radius"),
                        (Tool::SketchChamfer, Icon::Chamfer, "Chamfer: click a corner (or its two lines), type the distance — or two, like 2, 5 (first = first line)"),
                    ] {
                        if icons::button(ui, icon, tip, true, self.editor.tool == tool).clicked() {
                            self.editor.set_tool(tool);
                        }
                    }
                    if icons::button(ui, Icon::Trim, "Trim: click the part of a line to remove, back to the nearest crossing", true, self.editor.tool == Tool::Trim).clicked() {
                        self.editor.set_tool(Tool::Trim);
                    }
                    for (tool, icon, tip) in [
                        (Tool::Extend, Icon::Extend, "Extend: click a line near the end to run it on to the next geometry"),
                        (Tool::Offset, Icon::Offset, "Offset: click a line, arc or circle (its joined run follows), then click the side and distance"),
                    ] {
                        if icons::button(ui, icon, tip, true, self.editor.tool == tool).clicked() {
                            self.editor.set_tool(tool);
                        }
                    }
                    for (mode, icon, tip) in [
                        (SpMode::Rectangular, Icon::RectPattern, "Rectangular Pattern: select geometry first, then click this"),
                        (SpMode::Circular, Icon::CircPattern, "Circular Pattern: select geometry first, then click this"),
                    ] {
                        let on = self.sketch_pattern.as_ref().is_some_and(|d| d.mode == mode);
                        if icons::button(ui, icon, tip, true, on).clicked() {
                            self.start_sketch_pattern(mode);
                        }
                    }
                    if icons::button(ui, Icon::Mirror, "Mirror: select geometry first, then click this and the mirror line", true, self.editor.tool == Tool::Mirror).clicked() {
                        self.start_mirror();
                    }
                    let tip = "Project: click a body edge to bring it into the sketch (as locked construction geometry)";
                    if icons::button(ui, Icon::Project, tip, self.body_count > 0, self.editor.tool == Tool::Project).clicked() {
                        self.editor.set_tool(Tool::Project);
                    }
                    let tip = "Construction (X): toggle the selected geometry, or what you draw next";
                    if icons::button(ui, Icon::Construction, tip, true, self.editor.construction).clicked() {
                        self.toggle_construction();
                    }
                });
                let mut chosen = None;
                group(ui, "CONSTRAINTS", |ui| {
                    for label in CONSTRAINT_ACTIONS {
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
                        if icons::button(ui, icon, tip, true, self.editor.tool == Tool::Constrain(label)).clicked() {
                            chosen = Some(label);
                        }
                    }
                });
                if let Some(label) = chosen {
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
        egui::ScrollArea::vertical().max_height(ui.ctx().content_rect().height() * 0.7).show(ui, |ui| {
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
        let ex = Extrude { op: d.op, name: String::new(), sketch: s, profiles: Vec::new(), distance: d.distance, extent: d.extent, distance2: d.distance2, taper: d.taper };
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
        let Some(d) = self.dialog.as_mut() else { return };
        ui.heading(if d.editing.is_some() { "Edit Extrude" } else { "Extrude" });
        ui.label(format!("Profiles: {} selected", d.profiles.len()));
        ui.weak("Click shaded regions to add or remove them.");
        ui.separator();
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("Extent");
            for ex in model::Extent::ALL {
                if ui.selectable_label(d.extent == ex, ex.name()).clicked() && d.extent != ex {
                    d.extent = ex;
                    changed = true;
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(match d.extent {
                model::Extent::OneSide => "Distance",
                model::Extent::Symmetric => "Length",
                model::Extent::TwoSides => "Side 1",
            });
            changed |= ui
                .add(units::length_drag(&mut d.distance, unit, 0.5))
                .changed();
            if ui.button("Flip").clicked() {
                d.distance = -d.distance;
                changed = true;
            }
        });
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
                                if lost {
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
                match commit {
                    Some(Some(v)) => match self.editor.tool {
                        Tool::Slot => self.finish_slot_typed(v),
                        Tool::Line => self.finish_line_typed(v),
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
