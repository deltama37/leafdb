//! Error and result types shared across the engine.

use core::fmt;

/// A specialized [`Result`] type for leafdb operations.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors that can occur while parsing or executing SQL, or while managing
/// pages and storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The SQL text could not be tokenized or parsed.
    Parse(String),
    /// A statement referenced something that does not exist or is invalid.
    Catalog(String),
    /// A value did not match the expected column type, or a constraint failed.
    Type(String),
    /// A primary-key or uniqueness constraint was violated.
    Constraint(String),
    /// The number of bound parameters did not match the placeholders.
    Parameter(String),
    /// The underlying storage/page layer failed or is corrupt.
    Storage(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(m) => write!(f, "parse error: {m}"),
            Error::Catalog(m) => write!(f, "catalog error: {m}"),
            Error::Type(m) => write!(f, "type error: {m}"),
            Error::Constraint(m) => write!(f, "constraint error: {m}"),
            Error::Parameter(m) => write!(f, "parameter error: {m}"),
            Error::Storage(m) => write!(f, "storage error: {m}"),
        }
    }
}

impl std::error::Error for Error {}
