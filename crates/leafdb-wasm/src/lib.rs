//! WebAssembly bindings for leafdb.
//!
//! This crate exposes the synchronous [`leafdb_core::Database`] engine to
//! JavaScript as the [`LeafDb`] class. It converts JavaScript parameter arrays
//! into engine [`Value`]s and query results back into plain JS objects. It
//! does **not** touch OPFS: the raw page image is exchanged via [`LeafDb::export`]
//! and the constructor, and the async OPFS persistence is layered on top in
//! JavaScript (see `web/leafdb.js`).

use js_sys::{Array, Object, Reflect, Uint8Array};
use leafdb_core::{Database, ExecResult, Value};
use wasm_bindgen::prelude::*;

/// A leafdb database instance backed by an in-memory page image.
#[wasm_bindgen]
pub struct LeafDb {
    inner: Database,
}

#[wasm_bindgen]
impl LeafDb {
    /// Opens a database from an existing page image, or creates an empty one
    /// when `image` is `undefined`/`null`/empty.
    #[wasm_bindgen(constructor)]
    pub fn new(image: Option<Uint8Array>) -> Result<LeafDb, JsError> {
        let bytes = image.map(|a| a.to_vec());
        let inner = Database::open_or_create(bytes).map_err(to_js)?;
        Ok(LeafDb { inner })
    }

    /// Executes a DDL/DML statement and returns the number of affected rows.
    pub fn exec(&mut self, sql: &str, params: JsValue) -> Result<u32, JsError> {
        let params = js_to_params(&params)?;
        match self.inner.execute(sql, &params).map_err(to_js)? {
            ExecResult::Affected(n) => Ok(n as u32),
            ExecResult::Rows { rows, .. } => Ok(rows.len() as u32),
        }
    }

    /// Executes a query and returns `{ columns: string[], rows: any[][] }`.
    pub fn query(&mut self, sql: &str, params: JsValue) -> Result<JsValue, JsError> {
        let params = js_to_params(&params)?;
        let (columns, rows) = match self.inner.execute(sql, &params).map_err(to_js)? {
            ExecResult::Rows { columns, rows } => (columns, rows),
            ExecResult::Affected(_) => (Vec::new(), Vec::new()),
        };
        result_to_js(&columns, &rows)
    }

    /// Serializes the database to a page image for persistence in OPFS.
    pub fn export(&self) -> Uint8Array {
        Uint8Array::from(self.inner.export_image().as_slice())
    }

    /// Names of all tables currently defined.
    #[wasm_bindgen(js_name = tableNames)]
    pub fn table_names(&self) -> Array {
        let arr = Array::new();
        for t in self.inner.tables() {
            arr.push(&JsValue::from_str(&t.name));
        }
        arr
    }
}

fn to_js(err: leafdb_core::Error) -> JsError {
    JsError::new(&err.to_string())
}

/// Converts a JS array (or null/undefined) into engine parameter values.
fn js_to_params(params: &JsValue) -> Result<Vec<Value>, JsError> {
    if params.is_null() || params.is_undefined() {
        return Ok(Vec::new());
    }
    if !Array::is_array(params) {
        return Err(JsError::new("params must be an array"));
    }
    let arr = Array::from(params);
    let mut out = Vec::with_capacity(arr.length() as usize);
    for v in arr.iter() {
        out.push(js_to_value(&v)?);
    }
    Ok(out)
}

fn js_to_value(v: &JsValue) -> Result<Value, JsError> {
    if v.is_null() || v.is_undefined() {
        return Ok(Value::Null);
    }
    if let Some(b) = v.as_bool() {
        return Ok(Value::Boolean(b));
    }
    if let Some(n) = v.as_f64() {
        if n.fract() != 0.0 {
            return Err(JsError::new(
                "only integer numbers are supported; use a string for other values",
            ));
        }
        return Ok(Value::Integer(n as i64));
    }
    if let Some(s) = v.as_string() {
        return Ok(Value::Text(s));
    }
    Err(JsError::new("unsupported parameter type"))
}

fn value_to_js(v: &Value) -> JsValue {
    match v {
        Value::Null => JsValue::NULL,
        Value::Integer(i) => JsValue::from_f64(*i as f64),
        Value::Text(s) => JsValue::from_str(s),
        Value::Boolean(b) => JsValue::from_bool(*b),
    }
}

fn result_to_js(columns: &[String], rows: &[Vec<Value>]) -> Result<JsValue, JsError> {
    let cols = Array::new();
    for c in columns {
        cols.push(&JsValue::from_str(c));
    }
    let js_rows = Array::new();
    for row in rows {
        let r = Array::new();
        for value in row {
            r.push(&value_to_js(value));
        }
        js_rows.push(&r);
    }
    let obj = Object::new();
    Reflect::set(&obj, &JsValue::from_str("columns"), &cols)
        .map_err(|_| JsError::new("failed to build result object"))?;
    Reflect::set(&obj, &JsValue::from_str("rows"), &js_rows)
        .map_err(|_| JsError::new("failed to build result object"))?;
    Ok(obj.into())
}
