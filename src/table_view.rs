use crate::bridge::TableData;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

/// State for a single table window.
pub struct TableWindow {
    pub name: String,
    pub data: TableData,
    pub open: bool,
}

impl TableWindow {
    pub fn new(name: String, data: TableData) -> Self {
        Self {
            name,
            data,
            open: true,
        }
    }

    /// Render this table as a floating egui::Window. Returns false if closed.
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        let mut open = self.open;
        egui::Window::new(&self.name)
            .open(&mut open)
            .default_size([600.0, 400.0])
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                let row_count = self.data.rows.len();
                ui.label(format!(
                    "{} columns, {} rows",
                    self.data.columns.len(),
                    row_count
                ));
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

                table
                    .header(20.0, |mut header| {
                        for col_name in &self.data.columns {
                            header.col(|ui| {
                                ui.strong(col_name);
                            });
                        }
                    })
                    .body(|body| {
                        body.rows(text_height, row_count, |mut row| {
                            let row_data = &self.data.rows[row.index()];
                            for cell in row_data {
                                row.col(|ui| {
                                    ui.label(cell);
                                });
                            }
                        });
                    });
            });
        self.open = open;
        open
    }
}
