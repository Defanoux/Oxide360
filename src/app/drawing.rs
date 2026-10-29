//! The DRAWING workspace: the sheet (`drawing::sheet`) of the visible bodies
//! on screen, its toolbar, and Export PDF / DXF.

use crate::*;
use crate::drawing::{DrawingSettings, Pen, Sheet, SheetSize, ViewKind};

impl OpenCad {
    /// Into the DRAWING workspace (a sketch finishes, dialogs close).
    pub(crate) fn open_drawing(&mut self) {
        self.finish_sketch();
        self.cancel_all_dialogs();
        self.message = None;
        self.drawing_open = true;
        self.drawing_cam = None;
    }

    /// This design's drawing settings (the defaults until something's changed).
    pub(crate) fn drawing_settings(&self) -> DrawingSettings {
        self.drawing.clone().unwrap_or_default()
    }

    /// The sheet of the visible bodies, as they are now.
    pub(crate) fn drawing_sheet(&mut self) -> Sheet {
        self.ensure_model(Slot::Committed);
        let settings = self.drawing_settings();
        let model = self.model(Slot::Committed);
        let shown: Vec<&kernel::Body> = model.bodies.iter().filter(|b| !self.hidden_bodies.contains(&b.id)).collect();
        let shapes: Vec<_> = shown.iter().map(|b| &b.shape).collect();
        let views: Vec<(ViewKind, kernel::ViewLines)> = settings
            .views
            .iter()
            .filter_map(|&k| {
                let (toward, right) = k.axes();
                Some((k, kernel::hlr_view(&shapes, toward, right)?))
            })
            .collect();
        // The title block's material: the bodies' one, if they share it.
        let mut materials: Vec<&str> = shown.iter().map(|b| self.body_material(b.id).name).collect();
        materials.dedup();
        let material = match materials.as_slice() {
            [one] => one.to_string(),
            [] => String::new(),
            _ => "Various".to_string(),
        };
        let name = self.file_name().trim_end_matches(".ox360").trim_end_matches(".ocad").to_string();
        let title = if settings.title.trim().is_empty() { name } else { settings.title.clone() };
        let date = drawing::today();
        drawing::sheet(&settings, &views, &drawing::TitleInfo { title: &title, drawn_by: &settings.drawn_by, material: &material, date: &date })
    }

    /// The DRAWING toolbar: sheet, scale, views, hidden lines, title block, export.
    pub(crate) fn drawing_toolbar(&mut self, ui: &mut egui::Ui) {
        let mut s = self.drawing_settings();
        let before = s.clone();
        let scale_now = self.drawing_scale_shown;
        ui.horizontal(|ui| {
            ui.label("Sheet");
            egui::ComboBox::from_id_salt("drawing_sheet").selected_text(s.sheet.name()).width(60.0).show_ui(ui, |ui| {
                for size in SheetSize::ALL {
                    ui.selectable_value(&mut s.sheet, size, size.name());
                }
            });
            ui.add_space(8.0);
            ui.label("Scale");
            let shown = match s.scale {
                None => format!("Auto ({})", drawing::scale_text(scale_now)),
                Some(k) => drawing::scale_text(k),
            };
            egui::ComboBox::from_id_salt("drawing_scale").selected_text(shown).width(90.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.scale, None, "Auto").on_hover_text("The largest standard scale that fits the sheet");
                for k in drawing::SCALES {
                    ui.selectable_value(&mut s.scale, Some(k), drawing::scale_text(k));
                }
            });
            ui.add_space(8.0);
            ui.label("Views");
            for k in ViewKind::ALL {
                let mut on = s.views.contains(&k);
                if ui.checkbox(&mut on, k.name()).changed() {
                    if on {
                        s.views.push(k);
                    } else {
                        s.views.retain(|v| *v != k);
                    }
                }
            }
            ui.checkbox(&mut s.hidden_lines, "Hidden lines").on_hover_text("Edges behind, dashed (not in the iso view)");
            ui.add_space(8.0);
            ui.label("Title");
            ui.add(egui::TextEdit::singleline(&mut s.title).hint_text(self.file_name().trim_end_matches(".ox360").trim_end_matches(".ocad")).desired_width(120.0));
            ui.label("Drawn by");
            ui.add(egui::TextEdit::singleline(&mut s.drawn_by).desired_width(90.0));
            ui.add_space(8.0);
            if ui.button("Export PDF…").on_hover_text("The sheet as a PDF, true size, for printing").clicked() {
                self.export_drawing(true);
            }
            if ui.button("Export DXF…").on_hover_text("The sheet as a DXF in millimetres (layers VISIBLE, THIN, HIDDEN, TEXT)").clicked() {
                self.export_drawing(false);
            }
            if ui.button("Fit").on_hover_text("Show the whole sheet (F6)").clicked() {
                self.drawing_cam = None;
            }
        });
        if s != before {
            self.drawing = Some(s);
        }
    }

    /// The sheet on screen: paper white, lines black; drag with the right or
    /// middle button to pan, scroll to zoom, F6 to fit.
    pub(crate) fn drawing_view(&mut self, ui: &mut egui::Ui) {
        let sheet = self.drawing_sheet();
        self.drawing_scale_shown = sheet.scale;
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, Color32::from_gray(70));
        // px per mm and where the paper's bottom-left corner is on screen.
        let fit = {
            let k = ((rect.width() - 40.0) / sheet.size.x as f32).min((rect.height() - 40.0) / sheet.size.y as f32).max(0.05);
            (rect.center() + egui::vec2(-(sheet.size.x as f32) * k * 0.5, sheet.size.y as f32 * k * 0.5), k)
        };
        let (mut corner, mut k) = self.drawing_cam.unwrap_or(fit);
        if ui.input(|i| i.key_pressed(egui::Key::F6)) || response.double_clicked_by(egui::PointerButton::Middle) {
            (corner, k) = fit;
            self.drawing_cam = None;
        }
        if response.dragged_by(egui::PointerButton::Secondary) || response.dragged_by(egui::PointerButton::Middle) {
            corner += response.drag_delta();
            self.drawing_cam = Some((corner, k));
        }
        if let (Some(at), true) = (response.hover_pos(), response.hovered()) {
            let dy = ui.input(|i| i.smooth_scroll_delta.y);
            if dy != 0.0 {
                let f = (dy * 0.002).exp();
                let nk = (k * f).clamp(0.05, 200.0);
                // Zoom about the pointer.
                corner = at + (corner - at) * (nk / k);
                k = nk;
                self.drawing_cam = Some((corner, k));
            }
        }
        let to_screen = |p: glam::DVec2| egui::pos2(corner.x + p.x as f32 * k, corner.y - p.y as f32 * k);
        let paper = egui::Rect::from_two_pos(to_screen(glam::DVec2::ZERO), to_screen(sheet.size));
        painter.rect_filled(paper.translate(egui::vec2(3.0, 3.0)), 0.0, Color32::from_black_alpha(80));
        painter.rect_filled(paper, 0.0, Color32::WHITE);
        let ink = Color32::from_gray(20);
        for (pen, pts) in &sheet.lines {
            let stroke = Stroke::new((pen.width() as f32 * k).max(1.0), if *pen == Pen::Thin { Color32::from_gray(70) } else { ink });
            let screen: Vec<egui::Pos2> = pts.iter().map(|p| to_screen(*p)).collect();
            if *pen == Pen::Dashed {
                painter.extend(egui::Shape::dashed_line(&screen, stroke, 3.0 * k, 1.5 * k));
            } else {
                painter.add(egui::Shape::line(screen, stroke));
            }
        }
        for t in &sheet.texts {
            // egui's font size is about 1.4 × its capital height.
            let size = (t.height as f32 * k * 1.4).max(1.0);
            painter.text(to_screen(t.at), egui::Align2::LEFT_BOTTOM, &t.text, egui::FontId::proportional(size), ink);
        }
        if self.body_count == 0 {
            painter.text(rect.center_top() + egui::vec2(0.0, 30.0), egui::Align2::CENTER_TOP, "No bodies to draw: make one in SOLID first", egui::FontId::proportional(15.0), Color32::from_gray(90));
        }
    }

    /// Export the sheet, asking where.
    pub(crate) fn export_drawing(&mut self, pdf: bool) {
        let ext = if pdf { "pdf" } else { "dxf" };
        let name = self.file_name().trim_end_matches(".ox360").trim_end_matches(".ocad").to_string();
        let picked = self
            .file_dialog()
            .set_title(format!("Export Drawing As {}", ext.to_uppercase()))
            .add_filter(if pdf { "PDF document" } else { "DXF drawing" }, &[ext])
            .set_file_name(format!("{name}.{ext}"))
            .save_file();
        let Some(mut p) = picked else { return };
        if p.extension().is_none_or(|e| e != ext) {
            p.set_extension(ext);
        }
        if let Err(e) = self.export_drawing_to(&p) {
            self.alert = Some(e);
        }
    }

    /// Write the sheet to `path`: PDF or DXF by its extension.
    pub(crate) fn export_drawing_to(&mut self, path: &std::path::Path) -> Result<(), String> {
        let sheet = self.drawing_sheet();
        let bytes = match path.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
            Some("pdf") => drawing_export::pdf(&sheet),
            Some("dxf") => drawing_export::dxf(&sheet).into_bytes(),
            _ => return Err("Export Drawing writes .pdf or .dxf".to_string()),
        };
        std::fs::write(path, bytes).map_err(|e| format!("Couldn't save {}: {e}", path.display()))?;
        self.notify(format!("Drawing saved: {}", path.display()));
        Ok(())
    }
}
