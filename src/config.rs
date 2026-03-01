//! Runtime configuration types for server and client components.

use crate::error::{Error, Result};
use std::env;
use std::path::PathBuf;
use std::time::Duration;

/// SQLite database settings.
#[derive(Clone, Debug, Default)]
pub struct DatabaseConfig {
    /// Internal override path used only for tests.
    override_path: Option<String>,
}

impl DatabaseConfig {
    /// Resolves database file path.
    ///
    /// Runtime path is fixed to `$HOME/.cache/armesto/armesto-<major>.db`.
    pub fn resolved_path(&self) -> Result<String> {
        if let Some(path) = &self.override_path {
            return Ok(path.clone());
        }

        let home = env::var("HOME").map_err(|_| {
            Error::Initialization("HOME is not set; cannot resolve DB path".to_string())
        })?;
        let major = env!("CARGO_PKG_VERSION").split('.').next().unwrap_or("0");
        let path = PathBuf::from(home)
            .join(".cache")
            .join("armesto")
            .join(format!("armesto-{major}.db"));
        Ok(path.to_string_lossy().to_string())
    }

    /// Creates a config with explicit DB path override for tests.
    #[doc(hidden)]
    pub fn for_test_path(path: impl Into<String>) -> Self {
        Self {
            override_path: Some(path.into()),
        }
    }
}

/// Runtime configuration for the notification daemon.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// D-Bus service name for freedesktop notification compatibility.
    pub notification_bus_name: String,
    /// D-Bus service name for internal change notifications.
    pub change_bus_name: String,
    /// Optional explicit D-Bus bus address. When unset, session bus is used.
    pub dbus_address: Option<String>,
    /// Optional rofi-compatible UNIX socket path.
    pub rofi_socket_path: Option<String>,
    /// Poll interval for D-Bus process loop.
    pub dbus_poll_timeout: Duration,
    /// Database settings.
    pub database: DatabaseConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            notification_bus_name: "org.freedesktop.Notifications".to_string(),
            change_bus_name: "org.armesto.NotifyStore1".to_string(),
            dbus_address: None,
            rofi_socket_path: None,
            dbus_poll_timeout: Duration::from_millis(1000),
            database: DatabaseConfig::default(),
        }
    }
}

/// Runtime configuration for API/CLI clients.
#[derive(Clone, Debug)]
pub struct ClientConfig {
    /// Database settings.
    pub database: DatabaseConfig,
    /// Optional D-Bus service name for change subscriptions.
    pub change_bus_name: String,
    /// Optional explicit D-Bus bus address. When unset, session bus is used.
    pub dbus_address: Option<String>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            database: DatabaseConfig::default(),
            change_bus_name: "org.armesto.NotifyStore1".to_string(),
            dbus_address: None,
        }
    }
}
