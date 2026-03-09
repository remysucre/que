use crate::bridge::TableData;

/// Result of a SQL query.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct QueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
    #[serde(default)]
    pub row_ids: Vec<i64>,
}

impl QueryResult {
    pub fn into_table_data(self) -> TableData {
        TableData {
            columns: self.columns,
            rows: self.rows,
            row_ids: self.row_ids,
        }
    }
}

pub trait Db {
    fn execute(&self, sql: &str) -> Result<(), String>;
    fn query(&self, sql: &str) -> Result<QueryResult, String>;
    fn is_ready(&self) -> bool;
    fn init_error(&self) -> Option<String>;
    fn load_dropped_file(&self, table_name: &str, filename: &str) -> Result<QueryResult, String>;
    fn batch(&self, stmts: &[String], final_query: Option<&str>) -> Result<QueryResult, String>;
}

// ---------------------------------------------------------------------------
// WASM implementation (duckdb-wasm, async via Worker)
// ---------------------------------------------------------------------------
mod wasm {
    use super::*;
    use wasm_bindgen::prelude::*;
    use wasm_bindgen::JsCast;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    fn call_js_sync(fn_name: &str, arg: &str) -> Result<String, String> {
        let window = web_sys::window().ok_or("no window")?;
        let func = js_sys::Reflect::get(&window, &JsValue::from_str(fn_name))
            .map_err(|_| format!("JS function {fn_name} not found"))?;
        let func: js_sys::Function = func
            .dyn_into()
            .map_err(|_| format!("{fn_name} is not a function"))?;
        let result = func
            .call1(&JsValue::NULL, &JsValue::from_str(arg))
            .map_err(|e| format!("JS call failed: {e:?}"))?;
        result
            .as_string()
            .ok_or_else(|| "JS function did not return a string".to_string())
    }

    fn call_js_async_raw(fn_name: &str, arg: &str) -> Result<js_sys::Promise, String> {
        let window = web_sys::window().ok_or("no window")?;
        let func = js_sys::Reflect::get(&window, &JsValue::from_str(fn_name))
            .map_err(|_| format!("JS function {fn_name} not found"))?;
        let func: js_sys::Function = func
            .dyn_into()
            .map_err(|_| format!("{fn_name} is not a function"))?;
        let result = func
            .call1(&JsValue::NULL, &JsValue::from_str(arg))
            .map_err(|e| format!("JS call failed: {e:?}"))?;
        result
            .dyn_into::<js_sys::Promise>()
            .map_err(|_| "JS function did not return a Promise".to_string())
    }

    type ResultSlot = Rc<RefCell<Option<Result<String, String>>>>;

    fn spawn_js_call(fn_name: &'static str, arg: String, slot: ResultSlot) {
        let promise = match call_js_async_raw(fn_name, &arg) {
            Ok(p) => p,
            Err(e) => {
                *slot.borrow_mut() = Some(Err(e));
                return;
            }
        };
        let future = wasm_bindgen_futures::JsFuture::from(promise);
        wasm_bindgen_futures::spawn_local(async move {
            match future.await {
                Ok(val) => {
                    let s = val.as_string().unwrap_or_default();
                    if s.starts_with("ERROR:") {
                        *slot.borrow_mut() = Some(Err(s[6..].trim().to_string()));
                    } else {
                        *slot.borrow_mut() = Some(Ok(s));
                    }
                }
                Err(e) => {
                    *slot.borrow_mut() = Some(Err(format!("JS promise rejected: {e:?}")));
                }
            }
        });
    }

    pub struct WasmDb {
        query_cache: RefCell<HashMap<String, ResultSlot>>,
    }

    impl WasmDb {
        pub fn new() -> Self {
            Self {
                query_cache: RefCell::new(HashMap::new()),
            }
        }

    }

    impl Db for WasmDb {
        fn is_ready(&self) -> bool {
            call_js_sync("_ddb_is_ready", "")
                .map(|s| s == "true")
                .unwrap_or(false)
        }

        fn init_error(&self) -> Option<String> {
            call_js_sync("_ddb_get_init_error", "")
                .ok()
                .filter(|s| !s.is_empty())
        }

        fn execute(&self, sql: &str) -> Result<(), String> {
            if !self.is_ready() {
                return Err("Database is loading...".to_string());
            }
            let slot: ResultSlot = Rc::new(RefCell::new(None));
            spawn_js_call("_ddb_exec_async", sql.to_string(), slot);
            Ok(())
        }

        fn query(&self, sql: &str) -> Result<QueryResult, String> {
            if !self.is_ready() {
                return Err("Database is loading...".to_string());
            }

            let mut cache = self.query_cache.borrow_mut();

            if let Some(slot) = cache.get(sql) {
                let result = slot.borrow_mut().take();
                if let Some(res) = result {
                    cache.remove(sql);
                    return match res {
                        Ok(json) => serde_json::from_str(&json)
                            .map_err(|e| format!("JSON parse error: {e}")),
                        Err(e) => Err(e),
                    };
                }
                // Still pending
                return Ok(QueryResult::default());
            }

            let slot: ResultSlot = Rc::new(RefCell::new(None));
            cache.insert(sql.to_string(), slot.clone());
            spawn_js_call("_ddb_query_async", sql.to_string(), slot);
            Ok(QueryResult::default())
        }

        fn load_dropped_file(&self, table_name: &str, filename: &str) -> Result<QueryResult, String> {
            if !self.is_ready() {
                return Err("Database is loading...".to_string());
            }

            let input = serde_json::json!({
                "name": table_name,
                "filename": filename,
            });
            let key = format!("__file_load__{table_name}");

            let mut cache = self.query_cache.borrow_mut();

            if let Some(slot) = cache.get(&key) {
                let result = slot.borrow_mut().take();
                if let Some(res) = result {
                    cache.remove(&key);
                    return match res {
                        Ok(json) => serde_json::from_str(&json)
                            .map_err(|e| format!("JSON parse error: {e}")),
                        Err(e) => Err(e),
                    };
                }
                return Ok(QueryResult::default());
            }

            let slot: ResultSlot = Rc::new(RefCell::new(None));
            cache.insert(key, slot.clone());
            spawn_js_call("_ddb_load_dropped_file", input.to_string(), slot);
            Ok(QueryResult::default())
        }

        fn batch(
            &self,
            stmts: &[String],
            final_query: Option<&str>,
        ) -> Result<QueryResult, String> {
            if !self.is_ready() {
                return Err("Database is loading...".to_string());
            }

            // Build the JSON payload for _ddb_batch_async
            let input = serde_json::json!({
                "stmts": stmts,
                "query": final_query.unwrap_or(""),
            });
            let key = format!("__batch__{}", input);

            let mut cache = self.query_cache.borrow_mut();

            if let Some(slot) = cache.get(&key) {
                let result = slot.borrow_mut().take();
                if let Some(res) = result {
                    cache.remove(&key);
                    return match res {
                        Ok(json) if json == "OK" => Ok(QueryResult::default()),
                        Ok(json) => serde_json::from_str(&json)
                            .map_err(|e| format!("JSON parse error: {e}")),
                        Err(e) => Err(e),
                    };
                }
                return Ok(QueryResult::default());
            }

            let slot: ResultSlot = Rc::new(RefCell::new(None));
            cache.insert(key, slot.clone());
            spawn_js_call("_ddb_batch_async", input.to_string(), slot);
            Ok(QueryResult::default())
        }
    }
}

pub use wasm::WasmDb;
