use crate::db::{Db, QueryResult};

/// Parsed table data from a file.
#[derive(Debug, Clone)]
pub struct TableData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_ids: Vec<i64>,
}

/// Parse a query result that includes rowid as the first column.
pub fn parse_rowid_result(result: QueryResult) -> TableData {
    if result.columns.is_empty() {
        return TableData { columns: vec![], rows: vec![], row_ids: vec![] };
    }
    let columns = result.columns[1..].to_vec();
    let mut rows = Vec::new();
    let mut row_ids = Vec::new();
    for row in &result.rows {
        if let Some(rid_str) = row.first() {
            row_ids.push(rid_str.parse().unwrap_or(0));
            rows.push(row[1..].to_vec());
        }
    }
    TableData { columns, rows, row_ids }
}

/// Read all data from a table (columns + rows with rowid).
pub fn read_table(db: &dyn Db, table_name: &str) -> Result<TableData, String> {
    let col_result = db.query(&format!("PRAGMA table_info('{table_name}')"))?;
    let columns: Vec<String> = col_result
        .rows
        .iter()
        .filter_map(|row| row.get(1).cloned())
        .collect();

    if columns.is_empty() {
        // On WASM, the query might not have resolved yet — return empty
        return Ok(TableData {
            columns: Vec::new(),
            rows: Vec::new(),
            row_ids: Vec::new(),
        });
    }

    let col_list: String = columns.iter().map(|c| format!("\"{c}\"")).collect::<Vec<_>>().join(", ");
    let data_result = db.query(&format!(
        "SELECT rowid, {col_list} FROM \"{table_name}\" LIMIT 10000"
    ))?;

    let mut rows = Vec::new();
    let mut row_ids = Vec::new();
    for row in &data_result.rows {
        if let Some(rid_str) = row.first() {
            let rid: i64 = rid_str.parse().unwrap_or(0);
            row_ids.push(rid);
            rows.push(row[1..].to_vec());
        }
    }

    Ok(TableData { columns, rows, row_ids })
}

/// Drop a table from DuckDB.
pub fn drop_table(db: &dyn Db, name: &str) -> Result<(), String> {
    db.execute(&format!("DROP TABLE IF EXISTS \"{name}\""))
        .map_err(|e| format!("Failed to drop table: {e}"))
}
