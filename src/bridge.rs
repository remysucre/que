/// Parsed table data from a CSV file.
#[derive(Debug, Clone)]
pub struct TableData {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl TableData {
    pub fn from_csv(bytes: &[u8]) -> Result<Self, String> {
        let mut reader = csv::Reader::from_reader(bytes);

        let columns: Vec<String> = reader
            .headers()
            .map_err(|e| format!("Failed to read CSV headers: {e}"))?
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut rows = Vec::new();
        for result in reader.records() {
            let record = result.map_err(|e| format!("Failed to read CSV row: {e}"))?;
            rows.push(record.iter().map(|s| s.to_string()).collect());
        }

        Ok(TableData { columns, rows })
    }
}
