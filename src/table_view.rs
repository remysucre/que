use crate::bridge::TableData;
use duckdb::Connection;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

/// Structural mutation collected during a UI pass.
enum TableAction {
    AddColumn,
    DropColumn(usize),
    RenameColumn(usize, String, String), // (col_index, old_name, new_name)
    AddRow,
    DeleteRow(i64),
}

/// State for a single table window.
pub struct TableWindow {
    id: usize,
    pub name: String,
    /// Stored when renaming starts so we can ALTER TABLE from old → new
    pub rename_old: Option<String>,
    pub data: TableData,
    pub open: bool,
    pub renaming: bool,
    /// Which cell is currently being edited: (row, col)
    editing_cell: Option<(usize, usize)>,
    /// Column being renamed: (col_index, old_name)
    editing_col: Option<(usize, String)>,
}

static NEXT_TABLE_WINDOW_ID: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);

impl TableWindow {
    pub fn new(name: String, data: TableData) -> Self {
        Self {
            id: NEXT_TABLE_WINDOW_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name,
            rename_old: None,
            data,
            open: true,
            renaming: false,
            editing_cell: None,
            editing_col: None,
        }
    }

    /// Start renaming — saves old name for the ALTER TABLE.
    pub fn start_rename(&mut self) {
        self.rename_old = Some(self.name.clone());
        self.renaming = true;
    }

    /// Finish renaming — issues ALTER TABLE if name changed.
    pub fn finish_rename(&mut self, conn: &Connection) -> Result<(), String> {
        self.renaming = false;
        if let Some(old) = self.rename_old.take() {
            if old != self.name {
                let sql = format!("ALTER TABLE \"{}\" RENAME TO \"{}\"", old, self.name);
                conn.execute_batch(&sql)
                    .map_err(|e| format!("Rename failed: {e}"))?;
            }
        }
        Ok(())
    }

    pub fn refresh(&mut self, conn: &Connection) {
        let sql = format!("SELECT rowid, * FROM \"{}\" LIMIT 10000", self.name);
        if let Ok(mut stmt) = conn.prepare(&sql) {
            if let Ok(mut result) = stmt.query([]) {
                // column_count includes rowid, so actual data columns = count - 1
                let total_cols = result.as_ref().unwrap().column_count();
                let col_count = total_cols - 1;
                let columns: Vec<String> = (0..col_count)
                    .map(|i| {
                        result
                            .as_ref()
                            .unwrap()
                            .column_name(i + 1)
                            .map_or("?".to_string(), |v| v.to_string())
                    })
                    .collect();

                let mut rows = Vec::new();
                let mut row_ids = Vec::new();
                while let Ok(Some(row)) = result.next() {
                    let rid: i64 = row.get::<_, i64>(0).unwrap_or(0);
                    row_ids.push(rid);
                    let mut vals = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        let val: String = row
                            .get::<_, duckdb::types::Value>(i + 1)
                            .map(|v| crate::bridge::format_value(&v))
                            .unwrap_or_default();
                        vals.push(val);
                    }
                    rows.push(vals);
                }

                self.data = TableData { columns, rows, row_ids };
            }
        }
    }

    fn estimate_width(&self, ctx: &egui::Context) -> f32 {
        let font_id = egui::TextStyle::Body.resolve(&ctx.style());
        let char_width = ctx.fonts(|f| f.glyph_width(&font_id, '0'));
        let padding_per_col = 16.0;
        let scroll_bar = 20.0;

        let total: f32 = self
            .data
            .columns
            .iter()
            .enumerate()
            .map(|(ci, header)| {
                let mut max_len = header.len();
                for row in self.data.rows.iter().take(100) {
                    if let Some(cell) = row.get(ci) {
                        max_len = max_len.max(cell.len());
                    }
                }
                (max_len as f32 * char_width + padding_per_col).min(300.0).max(40.0)
            })
            .sum();

        total + scroll_bar + 20.0 // window margins
    }

    /// Render this table as a floating egui::Window. Returns false if closed.
    pub fn show(&mut self, ctx: &egui::Context, conn: &Connection) -> bool {
        let mut open = self.open;
        let width = self.estimate_width(ctx);

        // Collect pending edits and structural actions outside the UI closure
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        let mut actions: Vec<TableAction> = Vec::new();
        let mut new_editing: Option<(usize, usize)> = self.editing_cell;
        let editing = self.editing_cell;
        let mut new_editing_col: Option<(usize, String)> = self.editing_col.clone();
        let editing_col = self.editing_col.clone();

        let window_frame = egui::Frame::window(&ctx.style())
            .inner_margin(egui::Margin::same(2));
        egui::Window::new(&self.name)
            .id(egui::Id::new(("table_window", self.id)))
            .open(&mut open)
            .default_width(width)
            .resizable(true)
            .collapsible(true)
            .frame(window_frame)
            .show(ctx, |ui| {
                let row_count = self.data.rows.len();
                ui.label(format!("{} rows", row_count));
                ui.separator();

                let text_height = egui::TextStyle::Body
                    .resolve(ui.style())
                    .size
                    .max(ui.spacing().interact_size.y);

                let mut table = TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .min_scrolled_height(0.0);

                for _ in &self.data.columns {
                    table = table.column(Column::auto().at_least(40.0).resizable(true));
                }
                table = table.column(Column::auto().resizable(false));

                table
                    .header(20.0, |mut header| {
                        for ci in 0..self.data.columns.len() {
                            header.col(|ui| {
                                if editing_col.as_ref().map_or(false, |(idx, _)| *idx == ci) {
                                    // Inline rename editor
                                    let col = &mut self.data.columns[ci];
                                    let resp = ui.text_edit_singleline(col);
                                    if !resp.has_focus() && !resp.lost_focus() {
                                        resp.request_focus();
                                    }
                                    if resp.lost_focus() {
                                        let old = editing_col.as_ref().unwrap().1.clone();
                                        let new = col.clone();
                                        if old != new {
                                            actions.push(TableAction::RenameColumn(ci, old, new));
                                        }
                                        new_editing_col = None;
                                    }
                                } else {
                                    let col_name = &self.data.columns[ci];
                                    let label_resp = ui.strong(col_name);
                                    let header_resp = ui.interact(ui.max_rect(), ui.id().with(("col_header", ci)), egui::Sense::click());
                                    if label_resp.double_clicked() || header_resp.double_clicked() {
                                        new_editing_col = Some((ci, col_name.clone()));
                                    }
                                    let hovered = ui.rect_contains_pointer(ui.max_rect());
                                    let x_resp = ui.add_visible(hovered, egui::Button::new(egui_phosphor::regular::X).small().fill(egui::Color32::from_rgb(255, 180, 180)));
                                    if x_resp.clicked() {
                                        actions.push(TableAction::DropColumn(ci));
                                    }
                                    header_resp.context_menu(|ui| {
                                        if ui.button("Rename column").clicked() {
                                            new_editing_col = Some((ci, self.data.columns[ci].clone()));
                                            ui.close_menu();
                                        }
                                        if ui.button("Delete column").clicked() {
                                            actions.push(TableAction::DropColumn(ci));
                                            ui.close_menu();
                                        }
                                        if ui.button("Add column").clicked() {
                                            actions.push(TableAction::AddColumn);
                                            ui.close_menu();
                                        }
                                    });
                                }
                            });
                        }
                        header.col(|ui| {
                            if ui.small_button(format!("{} col", egui_phosphor::regular::PLUS)).clicked() {
                                actions.push(TableAction::AddColumn);
                            }
                        });
                    })
                    .body(|body| {
                        body.rows(text_height, row_count, |mut row| {
                            let ri = row.index();
                            let col_count = self.data.columns.len();
                            let mut row_hovered = false;
                            for ci in 0..col_count {
                                row.col(|ui| {
                                    if ui.rect_contains_pointer(ui.max_rect()) {
                                        row_hovered = true;
                                    }
                                    if editing == Some((ri, ci)) {
                                        let cell = &mut self.data.rows[ri][ci];
                                        let resp = ui.text_edit_singleline(cell);
                                        if !resp.has_focus() && !resp.lost_focus() {
                                            resp.request_focus();
                                        }
                                        if resp.changed() {
                                            edits.push((ri, ci, cell.clone()));
                                        }
                                        if resp.lost_focus() {
                                            new_editing = None;
                                        }
                                    } else {
                                        let cell = &self.data.rows[ri][ci];
                                        ui.label(cell);
                                        let cell_resp = ui.interact(ui.max_rect(), ui.id().with(("cell", ri, ci)), egui::Sense::click());
                                        if cell_resp.double_clicked() {
                                            new_editing = Some((ri, ci));
                                        }
                                        cell_resp.context_menu(|ui| {
                                            if ui.button("Add row").clicked() {
                                                actions.push(TableAction::AddRow);
                                                ui.close_menu();
                                            }
                                            if let Some(&rid) = self.data.row_ids.get(ri) {
                                                if ui.button("Delete row").clicked() {
                                                    actions.push(TableAction::DeleteRow(rid));
                                                    ui.close_menu();
                                                }
                                            }
                                        });
                                    }
                                });
                            }
                            row.col(|ui| {
                                if ui.rect_contains_pointer(ui.max_rect()) {
                                    row_hovered = true;
                                }
                                if let Some(&rid) = self.data.row_ids.get(ri) {
                                    let x_resp = ui.add_visible(row_hovered, egui::Button::new(egui_phosphor::regular::X).small().fill(egui::Color32::from_rgb(255, 180, 180)));
                                    if x_resp.clicked() {
                                        actions.push(TableAction::DeleteRow(rid));
                                    }
                                }
                            });
                        });
                    });
                if ui.small_button(format!("{} row", egui_phosphor::regular::PLUS)).clicked() {
                    actions.push(TableAction::AddRow);
                }
            });

        self.editing_cell = new_editing;
        self.editing_col = new_editing_col;

        // Apply cell edits to DuckDB
        for (ri, ci, new_val) in edits {
            if let Some(&rid) = self.data.row_ids.get(ri) {
                let col = &self.data.columns[ci];
                let sql = format!(
                    "UPDATE \"{}\" SET \"{}\" = $1 WHERE rowid = $2",
                    self.name, col
                );
                if let Ok(mut stmt) = conn.prepare(&sql) {
                    let _ = stmt.execute(duckdb::params![new_val, rid]);
                }
            }
        }

        // Execute structural actions
        for action in actions {
            match action {
                TableAction::AddColumn => {
                    let new_col = format!("col_{}", self.data.columns.len());
                    let sql = format!(
                        "ALTER TABLE \"{}\" ADD COLUMN \"{}\" VARCHAR",
                        self.name, new_col
                    );
                    let _ = conn.execute_batch(&sql);
                    self.refresh(conn);
                }
                TableAction::DropColumn(ci) => {
                    if let Some(col) = self.data.columns.get(ci) {
                        let sql = format!(
                            "ALTER TABLE \"{}\" DROP COLUMN \"{}\"",
                            self.name, col
                        );
                        let _ = conn.execute_batch(&sql);
                        self.refresh(conn);
                    }
                }
                TableAction::RenameColumn(_ci, old, new) => {
                    let sql = format!(
                        "ALTER TABLE \"{}\" RENAME COLUMN \"{}\" TO \"{}\"",
                        self.name, old, new
                    );
                    let _ = conn.execute_batch(&sql);
                    self.refresh(conn);
                }
                TableAction::AddRow => {
                    let sql = format!("INSERT INTO \"{}\" DEFAULT VALUES", self.name);
                    let _ = conn.execute_batch(&sql);
                    self.refresh(conn);
                }
                TableAction::DeleteRow(rid) => {
                    let sql = format!(
                        "DELETE FROM \"{}\" WHERE rowid = {}",
                        self.name, rid
                    );
                    let _ = conn.execute_batch(&sql);
                    self.refresh(conn);
                }
            }
        }

        self.open = open;
        open
    }
}
