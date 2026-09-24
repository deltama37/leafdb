//! Row (record) serialization.
//!
//! A row is stored as a self-describing sequence of tagged values in column
//! order. Keeping the type tag with each value keeps decoding simple and makes
//! `NULL` handling uniform.

use crate::bytes::{ByteReader, ByteWriter};
use crate::error::Result;
use crate::value::Value;

/// Serializes a row into a cell payload.
pub fn encode_row(values: &[Value]) -> Vec<u8> {
    let mut w = ByteWriter::new();
    w.u32(values.len() as u32);
    for v in values {
        v.write(&mut w);
    }
    w.into_vec()
}

/// Deserializes a row from a cell payload.
pub fn decode_row(cell: &[u8]) -> Result<Vec<Value>> {
    let mut r = ByteReader::new(cell);
    let n = r.u32()? as usize;
    let mut values = Vec::with_capacity(n);
    for _ in 0..n {
        values.push(Value::read(&mut r)?);
    }
    Ok(values)
}
