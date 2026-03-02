//! Error and result types used across the notification backend.

use thiserror::Error as ThisError;

/// Error type for SQLite-backed notification operations.
#[derive(Debug, ThisError)]
pub enum Error {
    /// IO operation failed.
    #[error("IO error: `{0}`")]
    Io(#[from] std::io::Error),
    /// JSON operation failed.
    #[error("JSON error: `{0}`")]
    Json(#[from] serde_json::Error),
    /// SQLite operation failed.
    #[error("SQLite error: `{0}`")]
    Sqlite(#[from] rusqlite::Error),
    /// D-Bus operation failed.
    #[error("D-Bus error: `{0}`")]
    Dbus(#[from] dbus::Error),
    /// Validation failed on user input.
    #[error("Validation error: `{0}`")]
    Validation(String),
    /// Data parse failed.
    #[error("Parse error: `{0}`")]
    Parse(String),
    /// Requested resource was not found.
    #[error("Not found: `{0}`")]
    NotFound(String),
    /// Initialization failed.
    #[error("Initialization failed: `{0}`")]
    Initialization(String),
    /// Feature has not been implemented yet in the scaffold.
    #[error("Not implemented: `{0}`")]
    NotImplemented(&'static str),
}

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;
