//! # leafdb-core
//!
//! The storage and SQL engine for **leafdb**, a lightweight, browser-first
//! relational database (see the ADRs in `docs/adr/`).
//!
//! The crate is intentionally free of external dependencies and of any
//! browser/OPFS coupling: it operates purely on a page-based
//! [`storage::Storage`] abstraction (ADR-0002). Persistence in the browser is
//! achieved by exporting the raw page image and storing it in OPFS from
//! JavaScript.
//!
//! ## Example
//!
//! ```
//! use leafdb_core::{Database, ExecResult, Value};
//!
//! let mut db = Database::create().unwrap();
//! db.execute_str(
//!     "CREATE TABLE todos (id INTEGER PRIMARY KEY, title TEXT, done BOOLEAN)",
//! )
//! .unwrap();
//! db.execute(
//!     "INSERT INTO todos VALUES (?, ?, ?)",
//!     &[Value::Integer(1), Value::Text("Build a database".into()), Value::Boolean(false)],
//! )
//! .unwrap();
//!
//! let result = db
//!     .execute("SELECT * FROM todos WHERE done = ?", &[Value::Boolean(false)])
//!     .unwrap();
//! match result {
//!     ExecResult::Rows { rows, .. } => assert_eq!(rows.len(), 1),
//!     _ => panic!("expected rows"),
//! }
//! ```

pub mod bytes;
pub mod catalog;
pub mod database;
pub mod error;
pub mod executor;
pub mod heap;
pub mod page;
pub mod record;
pub mod sql;
pub mod storage;
pub mod value;

pub use database::Database;
pub use error::{Error, Result};
pub use executor::ExecResult;
pub use storage::{MemoryStorage, Storage, PAGE_SIZE};
pub use value::{DataType, Value};
