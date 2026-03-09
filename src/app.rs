use crate::bridge;
use crate::db::Db;
use crate::query_window::QueryWindow;
use crate::table_view::TableWindow;
use eframe::egui;

pub struct QueApp {
    db: Box<dyn Db>,
    tables: Vec<TableWindow>,
    query_windows: Vec<QueryWindow>,
    error: Option<String>,
    /// Pending file loads waiting for async results (WASM): (table_name, filename)
    pending_loads: Vec<(String, String)>,
}

impl QueApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        cc.egui_ctx.set_fonts(fonts);

        let db: Box<dyn Db> = Box::new(crate::db::WasmDb::new());

        Self {
            db,
            tables: Vec::new(),
            query_windows: Vec::new(),
            error: None,
            pending_loads: Vec::new(),
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        if !self.db.is_ready() {
            return;
        }
        let dropped_files: Vec<egui::DroppedFile> =
            ctx.input(|i| i.raw.dropped_files.clone());

        for file in dropped_files {
            let filename = file.name.clone();
            let name_hint = std::path::Path::new(&filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("table");
            let table_name: String = name_hint
                .chars()
                .map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' })
                .collect();
            let table_name = if table_name.is_empty() { "table".to_string() } else { table_name };

            match self.db.load_dropped_file(&table_name, &filename) {
                Ok(result) if !result.columns.is_empty() => {
                    let data = bridge::parse_rowid_result(result);
                    self.tables.push(TableWindow::new(table_name, data));
                }
                Ok(_) => {
                    // Result pending, poll next frame
                    self.pending_loads.push((table_name, filename));
                }
                Err(e) => {
                    self.error = Some(e);
                }
            }
        }
    }
}

impl eframe::App for QueApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_dropped_files(ctx);

        // Poll pending file loads (WASM async)
        let mut completed_loads = Vec::new();
        for (idx, (table_name, filename)) in self.pending_loads.iter().enumerate() {
            match self.db.load_dropped_file(table_name, filename) {
                Ok(result) if !result.columns.is_empty() => {
                    let data = bridge::parse_rowid_result(result);
                    self.tables.push(TableWindow::new(table_name.clone(), data));
                    completed_loads.push(idx);
                }
                Ok(_) => { ctx.request_repaint_after(std::time::Duration::from_millis(16)); }
                Err(e) => {
                    self.error = Some(e);
                    completed_loads.push(idx);
                }
            }
        }
        for idx in completed_loads.into_iter().rev() {
            self.pending_loads.remove(idx);
        }

        // Render all table windows (keep closed ones for the side panel)
        for tw in &mut self.tables {
            tw.show(ctx, self.db.as_ref());
        }

        // Render all query windows
        let mut any_query_ran = false;
        for qw in &mut self.query_windows {
            let ran = qw.show(ctx, self.db.as_ref());
            if ran {
                any_query_ran = true;
            }
        }

        // Refresh table data after a query modifies the database
        if any_query_ran {
            for tw in &mut self.tables {
                tw.refresh(self.db.as_ref());
            }
        }

        // Right side panel
        egui::SidePanel::right("tables_pane")
            .default_width(160.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Tables");
                ui.separator();
                let mut remove_table_idx = None;
                let mut finish_rename_idx = None;
                for (idx, tw) in self.tables.iter_mut().enumerate() {
                    if tw.renaming {
                        let resp = ui.text_edit_singleline(&mut tw.name);
                        if resp.lost_focus() {
                            finish_rename_idx = Some(idx);
                        } else {
                            resp.request_focus();
                        }
                    } else {
                        ui.horizontal(|ui| {
                            let label = ui.selectable_label(tw.open, &tw.name);
                            if label.clicked() {
                                tw.open = !tw.open;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button(egui_phosphor::regular::X).clicked() {
                                    remove_table_idx = Some(idx);
                                }
                                if ui.small_button(egui_phosphor::regular::PENCIL_SIMPLE).clicked() {
                                    tw.start_rename();
                                }
                            });
                        });
                    }
                }
                if let Some(idx) = finish_rename_idx {
                    if let Err(e) = self.tables[idx].finish_rename(self.db.as_ref()) {
                        self.error = Some(e);
                    }
                }
                if let Some(idx) = remove_table_idx {
                    let name = &self.tables[idx].name;
                    if let Err(e) = bridge::drop_table(self.db.as_ref(), name) {
                        self.error = Some(e);
                    }
                    self.tables.remove(idx);
                }
                if self.tables.is_empty() {
                    ui.weak("No tables loaded");
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.heading("Queries");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(egui_phosphor::regular::PLUS).clicked() {
                            let id = self.query_windows.len() + 1;
                            self.query_windows.push(QueryWindow::new(id));
                        }
                    });
                });
                ui.separator();
                let mut remove_idx = None;
                for (idx, qw) in self.query_windows.iter_mut().enumerate() {
                    if qw.renaming {
                        let resp = ui.text_edit_singleline(&mut qw.name);
                        if resp.lost_focus() {
                            qw.renaming = false;
                        } else {
                            resp.request_focus();
                        }
                    } else {
                        ui.horizontal(|ui| {
                            let label = ui.selectable_label(qw.open, &qw.name);
                            if label.clicked() {
                                qw.open = !qw.open;
                            }
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.small_button(egui_phosphor::regular::X).clicked() {
                                    remove_idx = Some(idx);
                                }
                                if ui.small_button(egui_phosphor::regular::PENCIL_SIMPLE).clicked() {
                                    qw.renaming = true;
                                }
                            });
                        });
                    }
                }
                if let Some(idx) = remove_idx {
                    self.query_windows.remove(idx);
                }
                if self.query_windows.is_empty() {
                    ui.weak("No queries");
                }
            });

        let has_tables = self.tables.iter().any(|tw| tw.open);

        // Central panel: drop zone hint when no tables are open
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error.clone() {
                ui.colored_label(egui::Color32::from_rgb(255, 100, 100), format!("Error: {err}"));
                if ui.button("Dismiss").clicked() {
                    self.error = None;
                }
                ui.separator();
            }

            if !has_tables {
                ui.vertical_centered(|ui| {
                    ui.add_space(ui.available_height() / 3.0);
                    ui.heading("¿Qué?");
                    ui.add_space(8.0);
                    if !self.db.is_ready() {
                        if let Some(err) = self.db.init_error() {
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 100, 100),
                                format!("Database init failed: {err}"),
                            );
                        } else {
                            ui.label("Initializing database...");
                            ctx.request_repaint_after(std::time::Duration::from_millis(16));
                        }
                    } else {
                        ui.label("Drop a data file here");
                    }
                });
            }
        });

        preview_files_being_dropped(ctx);
    }
}

/// Paints a semi-transparent overlay when files are being dragged over the window.
fn preview_files_being_dropped(ctx: &egui::Context) {
    use egui::{Align2, Color32, Id, LayerId, Order, TextStyle};

    if !ctx.input(|i| i.raw.hovered_files.is_empty()) {
        let painter =
            ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("file_drop_target")));

        let screen_rect = ctx.screen_rect();
        painter.rect_filled(screen_rect, 0.0, Color32::from_black_alpha(160));
        painter.text(
            screen_rect.center(),
            Align2::CENTER_CENTER,
            "Drop file to load",
            TextStyle::Heading.resolve(&ctx.style()),
            Color32::WHITE,
        );
    }
}
