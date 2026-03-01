#![warn(missing_docs, clippy::unwrap_used)]

//! SQLite-backed Linux desktop notification backend primitives.

mod client;
mod config;
mod dbus_support;
mod error;
mod model;
mod repository;
mod rofi;
mod server;

pub use crate::client::{ChangeStream, NotifyClient};
pub use crate::config::{ClientConfig, DatabaseConfig, ServerConfig};
pub use crate::error::{Error, Result};
pub use crate::model::{
    ChangeEvent, ChangeKind, ListQuery, NewNotification, Notification, NotificationStatus,
    UpdateNotification, Urgency,
};
pub use crate::repository::SqliteRepository;
pub use crate::server::NotificationServer;
