//! Shared D-Bus constants and connection helpers.

use crate::{Error, Result};
use dbus::blocking::Connection;
use dbus::channel::Channel;

/// Freedesktop notification interface.
pub const NOTIFICATION_INTERFACE: &str = "org.freedesktop.Notifications";
/// Freedesktop notification object path.
pub const NOTIFICATION_PATH: &str = "/org/freedesktop/Notifications";
/// Internal change signal interface.
pub const CHANGE_INTERFACE: &str = "org.armesto.NotifyStore1";
/// Internal change signal object path.
pub const CHANGE_PATH: &str = "/org/armesto/NotifyStore1";
/// Internal change signal member name.
pub const CHANGE_SIGNAL: &str = "NotificationChanged";

/// Opens a blocking D-Bus connection from either explicit address or session bus.
pub fn open_connection(address: Option<&str>) -> Result<Connection> {
    match address {
        Some(address) => {
            let mut channel = Channel::open_private(address)?;
            channel.register()?;
            Ok(Connection::from(channel))
        }
        None => Connection::new_session().map_err(Error::from),
    }
}
