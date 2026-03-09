use crate::bridge::TableData;
use crate::db::Db;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

pub struct QueryWindow {
    id: usize,
    pub name: String,
    pub renaming: bool,
    query: String,
    result: Option<Result<TableData, String>>,
    pub open: bool,
    /// Pending async query being polled
    pending_query: Option<String>,
}

impl QueryWindow {
    pub fn new(id: usize) -> Self {
        Self {
            id,
            name: format!("Query {id}"),
            renaming: false,
            query: String::new(),
            result: None,
            open: true,
            pending_query: None,
        }
    }

    /// Render the query window. Returns whether a query was run.
    pub fn show(&mut self, ctx: &egui::Context, db: &dyn Db) -> bool {
        if !self.open {
            return false;
        }

        // Poll pending async query
        let mut ran_from_poll = false;
        if let Some(sql) = self.pending_query.clone() {
            match db.query(&sql) {
                Ok(result) if !result.columns.is_empty() => {
                    self.result = Some(Ok(result.into_table_data()));
                    self.pending_query = None;
                    ran_from_poll = true;
                }
                Ok(_) => { ctx.request_repaint_after(std::time::Duration::from_millis(16)); }
                Err(e) => {
                    self.result = Some(Err(e));
                    self.pending_query = None;
                }
            }
        }

        let mut open = self.open;
        let ran = std::cell::Cell::new(ran_from_poll);
        let window_frame = egui::Frame::window(&ctx.style())
            .inner_margin(egui::Margin::same(2));
        egui::Window::new(&self.name)
            .id(egui::Id::new(("query_window", self.id)))
            .open(&mut open)
            .default_size([300.0, 400.0])
            .resizable(true)
            .frame(window_frame)
            .show(ctx, |ui| {
                ui.label("SQL:");
                let layouter = |ui: &egui::Ui, text: &str, wrap_width: f32| {
                    let layout_job = highlight_sql(ui, text, wrap_width);
                    ui.fonts(|f| f.layout_job(layout_job))
                };
                ui.add(
                    egui::TextEdit::multiline(&mut self.query)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .layouter(&mut layouter.clone()),
                );

                if ui.button("Run").clicked() {
                    match run_query(db, &self.query) {
                        Ok(data) if !data.columns.is_empty() => {
                            self.result = Some(Ok(data));
                            ran.set(true);
                        }
                        Ok(_) => {
                            // WASM: result pending, poll next frame
                            self.pending_query = Some(self.query.clone());
                            self.result = None;
                        }
                        Err(e) => {
                            self.result = Some(Err(e));
                        }
                    }
                }

                ui.separator();

                match &self.result {
                    Some(Ok(data)) => {
                        ui.label(format!(
                            "{} columns, {} rows",
                            data.columns.len(),
                            data.rows.len()
                        ));
                        show_table(ui, data);
                    }
                    Some(Err(e)) => {
                        ui.colored_label(
                            egui::Color32::from_rgb(255, 100, 100),
                            format!("Error: {e}"),
                        );
                    }
                    None => {}
                }
            });
        self.open = open;
        ran.get()
    }
}

fn run_query(db: &dyn Db, sql: &str) -> Result<TableData, String> {
    let result = db.query(sql)?;
    Ok(result.into_table_data())
}

const SQL_KEYWORDS: &[&str] = &[
    "SELECT", "FROM", "WHERE", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER", "TABLE",
    "INTO", "VALUES", "SET", "JOIN", "LEFT", "RIGHT", "INNER", "OUTER", "FULL", "CROSS", "ON",
    "AND", "OR", "NOT", "IN", "IS", "NULL", "AS", "ORDER", "BY", "GROUP", "HAVING", "LIMIT",
    "OFFSET", "UNION", "ALL", "DISTINCT", "BETWEEN", "LIKE", "ILIKE", "EXISTS", "CASE", "WHEN",
    "THEN", "ELSE", "END", "ASC", "DESC", "WITH", "RECURSIVE", "CAST", "TRUE", "FALSE", "COUNT",
    "SUM", "AVG", "MIN", "MAX", "OVER", "PARTITION", "WINDOW", "FILTER", "USING", "NATURAL",
    "EXCEPT", "INTERSECT", "PRIMARY", "KEY", "FOREIGN", "REFERENCES", "INDEX", "VIEW",
    "REPLACE", "IF", "BEGIN", "COMMIT", "ROLLBACK", "PRAGMA", "DESCRIBE", "EXPLAIN", "ANALYZE",
];

fn highlight_sql(ui: &egui::Ui, text: &str, wrap_width: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap_width;

    let mono = egui::TextFormat {
        font_id: egui::TextStyle::Monospace.resolve(ui.style()),
        ..Default::default()
    };

    let keyword_fmt = egui::TextFormat {
        color: egui::Color32::from_rgb(86, 156, 214), // blue
        ..mono.clone()
    };
    let string_fmt = egui::TextFormat {
        color: egui::Color32::from_rgb(206, 145, 120), // orange
        ..mono.clone()
    };
    let number_fmt = egui::TextFormat {
        color: egui::Color32::from_rgb(181, 206, 168), // green
        ..mono.clone()
    };
    let comment_fmt = egui::TextFormat {
        color: egui::Color32::from_rgb(106, 153, 85), // dim green
        ..mono.clone()
    };

    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Line comment: --
        if i + 1 < len && chars[i] == '-' && chars[i + 1] == '-' {
            let start = i;
            while i < len && chars[i] != '\n' {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            job.append(&s, 0.0, comment_fmt.clone());
            continue;
        }

        // Block comment: /* ... */
        if i + 1 < len && chars[i] == '/' && chars[i + 1] == '*' {
            let start = i;
            i += 2;
            while i + 1 < len && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            if i + 1 < len {
                i += 2;
            }
            let s: String = chars[start..i].iter().collect();
            job.append(&s, 0.0, comment_fmt.clone());
            continue;
        }

        // String literal: '...'
        if chars[i] == '\'' {
            let start = i;
            i += 1;
            while i < len {
                if chars[i] == '\'' {
                    i += 1;
                    if i < len && chars[i] == '\'' {
                        i += 1; // escaped quote
                    } else {
                        break;
                    }
                } else {
                    i += 1;
                }
            }
            let s: String = chars[start..i].iter().collect();
            job.append(&s, 0.0, string_fmt.clone());
            continue;
        }

        // Number
        if chars[i].is_ascii_digit()
            || (chars[i] == '.' && i + 1 < len && chars[i + 1].is_ascii_digit())
        {
            let start = i;
            while i < len && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // Only highlight if not part of an identifier
            if start == 0
                || !(chars[start - 1].is_alphanumeric() || chars[start - 1] == '_')
            {
                let s: String = chars[start..i].iter().collect();
                job.append(&s, 0.0, number_fmt.clone());
            } else {
                let s: String = chars[start..i].iter().collect();
                job.append(&s, 0.0, mono.clone());
            }
            continue;
        }

        // Word (identifier or keyword)
        if chars[i].is_alphanumeric() || chars[i] == '_' {
            let start = i;
            while i < len && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let upper = word.to_uppercase();
            if SQL_KEYWORDS.contains(&upper.as_str()) {
                job.append(&word, 0.0, keyword_fmt.clone());
            } else {
                job.append(&word, 0.0, mono.clone());
            }
            continue;
        }

        // Everything else (operators, whitespace, punctuation)
        let s: String = chars[i..i + 1].iter().collect();
        job.append(&s, 0.0, mono.clone());
        i += 1;
    }

    job
}

fn show_table(ui: &mut egui::Ui, data: &TableData) {
    let row_count = data.rows.len();
    let text_height = egui::TextStyle::Body
        .resolve(ui.style())
        .size
        .max(ui.spacing().interact_size.y);

    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .min_scrolled_height(0.0);

    for _ in &data.columns {
        table = table.column(Column::auto().at_least(40.0).resizable(true));
    }

    table
        .header(20.0, |mut header| {
            for col_name in &data.columns {
                header.col(|ui| {
                    ui.strong(col_name);
                });
            }
        })
        .body(|body| {
            body.rows(text_height, row_count, |mut row| {
                let row_data = &data.rows[row.index()];
                for cell in row_data {
                    row.col(|ui| {
                        ui.label(cell);
                    });
                }
            });
        });
}
