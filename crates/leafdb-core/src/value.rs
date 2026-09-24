//! Value and data-type model.
//!
//! The initial scope (ADR-0001) supports a small set of practical types:
//! `INTEGER`, `TEXT`, and `BOOLEAN`, plus `NULL`.

use crate::bytes::{ByteReader, ByteWriter};
use crate::error::{Error, Result};

/// The declared type of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    Integer,
    Text,
    Boolean,
}

impl DataType {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataType::Integer => "INTEGER",
            DataType::Text => "TEXT",
            DataType::Boolean => "BOOLEAN",
        }
    }

    pub fn from_keyword(kw: &str) -> Option<DataType> {
        match kw.to_ascii_uppercase().as_str() {
            "INTEGER" | "INT" => Some(DataType::Integer),
            "TEXT" | "STRING" | "VARCHAR" => Some(DataType::Text),
            "BOOLEAN" | "BOOL" => Some(DataType::Boolean),
            _ => None,
        }
    }

    fn tag(&self) -> u8 {
        match self {
            DataType::Integer => 1,
            DataType::Text => 2,
            DataType::Boolean => 3,
        }
    }

    fn from_tag(tag: u8) -> Result<DataType> {
        match tag {
            1 => Ok(DataType::Integer),
            2 => Ok(DataType::Text),
            3 => Ok(DataType::Boolean),
            other => Err(Error::Storage(format!("unknown data type tag {other}"))),
        }
    }

    pub fn write(&self, w: &mut ByteWriter) {
        w.u8(self.tag());
    }

    pub fn read(r: &mut ByteReader) -> Result<DataType> {
        DataType::from_tag(r.u8()?)
    }
}

/// A concrete runtime value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Integer(i64),
    Text(String),
    Boolean(bool),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// Returns the type of a non-null value, or `None` for `NULL`.
    pub fn data_type(&self) -> Option<DataType> {
        match self {
            Value::Null => None,
            Value::Integer(_) => Some(DataType::Integer),
            Value::Text(_) => Some(DataType::Text),
            Value::Boolean(_) => Some(DataType::Boolean),
        }
    }

    /// Coerces/validates a value against a declared column type.
    ///
    /// A limited amount of coercion is allowed to make the SQL surface
    /// ergonomic (e.g. integers `0`/`1` into booleans).
    pub fn coerce_to(self, ty: DataType) -> Result<Value> {
        match (ty, self) {
            (_, Value::Null) => Ok(Value::Null),
            (DataType::Integer, Value::Integer(i)) => Ok(Value::Integer(i)),
            (DataType::Text, Value::Text(s)) => Ok(Value::Text(s)),
            (DataType::Boolean, Value::Boolean(b)) => Ok(Value::Boolean(b)),
            (DataType::Boolean, Value::Integer(0)) => Ok(Value::Boolean(false)),
            (DataType::Boolean, Value::Integer(1)) => Ok(Value::Boolean(true)),
            (ty, v) => Err(Error::Type(format!(
                "cannot store {v:?} in a column of type {}",
                ty.as_str()
            ))),
        }
    }

    fn tag(&self) -> u8 {
        match self {
            Value::Null => 0,
            Value::Integer(_) => 1,
            Value::Text(_) => 2,
            Value::Boolean(_) => 3,
        }
    }

    pub fn write(&self, w: &mut ByteWriter) {
        w.u8(self.tag());
        match self {
            Value::Null => {}
            Value::Integer(i) => w.i64(*i),
            Value::Text(s) => w.string(s),
            Value::Boolean(b) => w.u8(if *b { 1 } else { 0 }),
        }
    }

    pub fn read(r: &mut ByteReader) -> Result<Value> {
        let tag = r.u8()?;
        match tag {
            0 => Ok(Value::Null),
            1 => Ok(Value::Integer(r.i64()?)),
            2 => Ok(Value::Text(r.string()?)),
            3 => Ok(Value::Boolean(r.u8()? != 0)),
            other => Err(Error::Storage(format!("unknown value tag {other}"))),
        }
    }
}
