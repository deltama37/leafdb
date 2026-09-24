//! System catalog: the set of tables and their schemas.
//!
//! The catalog is serialized onto page 0 of the database image. It is small
//! (a handful of tables) and is rewritten whenever the schema or a table's
//! page chain changes.

use crate::bytes::{ByteReader, ByteWriter};
use crate::error::{Error, Result};
use crate::storage::{PageId, PAGE_SIZE};
use crate::value::DataType;

/// A single column definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    pub data_type: DataType,
    pub primary_key: bool,
}

/// A table's schema plus the location of its heap pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableSchema {
    pub name: String,
    pub columns: Vec<Column>,
    /// First page of the heap chain (0 == not yet allocated).
    pub first_page: PageId,
    /// Last page of the heap chain, cached for fast appends (0 == none).
    pub last_page: PageId,
}

impl TableSchema {
    /// Index of a column by name (case-insensitive).
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Index of the single primary-key column, if any.
    pub fn primary_key_index(&self) -> Option<usize> {
        self.columns.iter().position(|c| c.primary_key)
    }
}

/// The in-memory catalog, loaded from and flushed to page 0.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    tables: Vec<TableSchema>,
}

const MAGIC: u32 = 0x4C_45_41_46; // "LEAF"

impl Catalog {
    pub fn new() -> Self {
        Self { tables: Vec::new() }
    }

    pub fn tables(&self) -> &[TableSchema] {
        &self.tables
    }

    pub fn get(&self, name: &str) -> Option<&TableSchema> {
        self.tables
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut TableSchema> {
        self.tables
            .iter_mut()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    pub fn add_table(&mut self, table: TableSchema) -> Result<()> {
        if self.contains(&table.name) {
            return Err(Error::Catalog(format!(
                "table '{}' already exists",
                table.name
            )));
        }
        self.tables.push(table);
        Ok(())
    }

    /// Serializes the whole catalog into a single page buffer.
    pub fn to_page(&self) -> Result<[u8; PAGE_SIZE]> {
        let mut w = ByteWriter::new();
        w.u32(MAGIC);
        w.u32(self.tables.len() as u32);
        for t in &self.tables {
            w.string(&t.name);
            w.u32(t.first_page);
            w.u32(t.last_page);
            w.u32(t.columns.len() as u32);
            for c in &t.columns {
                w.string(&c.name);
                c.data_type.write(&mut w);
                w.u8(if c.primary_key { 1 } else { 0 });
            }
        }
        let bytes = w.into_vec();
        if bytes.len() > PAGE_SIZE {
            return Err(Error::Catalog(format!(
                "catalog is too large to fit in one page ({} > {PAGE_SIZE})",
                bytes.len()
            )));
        }
        let mut page = [0u8; PAGE_SIZE];
        page[..bytes.len()].copy_from_slice(&bytes);
        Ok(page)
    }

    /// Parses a catalog from page 0.
    pub fn from_page(page: &[u8]) -> Result<Catalog> {
        let mut r = ByteReader::new(page);
        let magic = r.u32()?;
        if magic != MAGIC {
            return Err(Error::Storage(
                "database image has an invalid magic header".into(),
            ));
        }
        let n = r.u32()? as usize;
        let mut tables = Vec::with_capacity(n);
        for _ in 0..n {
            let name = r.string()?;
            let first_page = r.u32()?;
            let last_page = r.u32()?;
            let col_count = r.u32()? as usize;
            let mut columns = Vec::with_capacity(col_count);
            for _ in 0..col_count {
                let cname = r.string()?;
                let data_type = DataType::read(&mut r)?;
                let primary_key = r.u8()? != 0;
                columns.push(Column {
                    name: cname,
                    data_type,
                    primary_key,
                });
            }
            tables.push(TableSchema {
                name,
                columns,
                first_page,
                last_page,
            });
        }
        Ok(Catalog { tables })
    }
}
