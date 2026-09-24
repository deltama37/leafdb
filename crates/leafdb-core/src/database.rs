//! The [`Database`] facade: the primary entry point of the engine.
//!
//! A `Database` owns an in-memory page image ([`MemoryStorage`]) and the
//! catalog. Callers run SQL through [`Database::execute`]. Persistence is
//! handled by exporting the raw page image ([`Database::export_image`]) and
//! reloading it with [`Database::open_image`]; the browser layer stores that
//! image in OPFS.

use crate::catalog::{Catalog, TableSchema};
use crate::error::Result;
use crate::executor::{self, ExecResult};
use crate::sql;
use crate::storage::{MemoryStorage, PageId, Storage};
use crate::value::Value;

const CATALOG_PAGE: PageId = 0;

/// An embedded leafdb database backed by an in-memory page image.
pub struct Database {
    storage: MemoryStorage,
    catalog: Catalog,
}

impl Database {
    /// Creates a brand-new, empty database.
    pub fn create() -> Result<Database> {
        let mut storage = MemoryStorage::new();
        let page = storage.allocate_page()?;
        debug_assert_eq!(page, CATALOG_PAGE);
        let catalog = Catalog::new();
        storage.write_page(CATALOG_PAGE, &catalog.to_page()?)?;
        Ok(Database { storage, catalog })
    }

    /// Reopens a database from a previously exported page image.
    pub fn open_image(image: Vec<u8>) -> Result<Database> {
        let storage = MemoryStorage::from_image(image)?;
        let mut buf = [0u8; crate::storage::PAGE_SIZE];
        storage.read_page(CATALOG_PAGE, &mut buf)?;
        let catalog = Catalog::from_page(&buf)?;
        Ok(Database { storage, catalog })
    }

    /// Opens from an image if provided and non-empty, otherwise creates fresh.
    pub fn open_or_create(image: Option<Vec<u8>>) -> Result<Database> {
        match image {
            Some(bytes) if !bytes.is_empty() => Database::open_image(bytes),
            _ => Database::create(),
        }
    }

    /// Executes a single SQL statement with positional parameters.
    pub fn execute(&mut self, sql: &str, params: &[Value]) -> Result<ExecResult> {
        let stmt = sql::parse(sql)?;
        let result = executor::execute(&mut self.catalog, &mut self.storage, stmt, params)?;
        if result.is_mutation() {
            self.flush_catalog()?;
        }
        Ok(result)
    }

    /// Executes a statement that takes no parameters.
    pub fn execute_str(&mut self, sql: &str) -> Result<ExecResult> {
        self.execute(sql, &[])
    }

    /// Serializes the full database into a page image for persistence.
    pub fn export_image(&self) -> Vec<u8> {
        self.storage.image()
    }

    /// Returns the schemas of all tables.
    pub fn tables(&self) -> &[TableSchema] {
        self.catalog.tables()
    }

    fn flush_catalog(&mut self) -> Result<()> {
        self.storage
            .write_page(CATALOG_PAGE, &self.catalog.to_page()?)
    }
}
