//! A dead simple notification daemon.

#![warn(missing_docs, clippy::unwrap_used)]

/// Error handler.
pub mod error;

/// D-Bus handler.
pub mod dbus;

/// Notification manager.
pub mod notification;

/// Rofi server
pub mod rofi;

use crate::dbus::DbusServer;
use crate::error::Result;
use crate::rofi::RofiServer;
use log::{debug, error};
use notification::Action;
use notification::NotificationStore;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Startup configuration
#[derive(Debug, Clone)]
pub struct Config {
    /// Local path to file representing domain socket
    pub socket_path: String,

    /// Duration to wait for incoming d-bus messages
    pub dbus_poll_timeout: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            socket_path: "/tmp/armesto".to_string(),
            dbus_poll_timeout: 1000,
        }
    }
}

/// Service entry-point
pub fn run(config: Config) -> Result<()> {
    let Config {
        socket_path,
        dbus_poll_timeout,
    } = config;
    let dbus_server = DbusServer::init()?;
    let db = NotificationStore::init();
    let (dbus_sender, receiver) = mpsc::channel();
    let rofi_sender = dbus_sender.clone();

    thread::Builder::new()
        .name("dbus".to_string())
        .spawn(move || {
            debug!("registering D-Bus server");
            let dbus_sender2 = dbus_sender.clone();
            let duration = Duration::from_millis(dbus_poll_timeout.into());
            if let Err(err) = dbus_server.register_notification_handler(dbus_sender, duration) {
                if let Err(send_err) = dbus_sender2.send(Action::Shutdown(err)) {
                    error!("failed to send dbus shutdown action: {}", send_err);
                }
            }
        })?;

    let db_clone = db.clone();
    thread::Builder::new()
        .name("rofication".to_string())
        .spawn(move || {
            debug!("starting rofication server");
            let rofi_server = RofiServer::new(socket_path, db_clone);
            if let Err(err) = rofi_server.start() {
                if let Err(send_err) = rofi_sender.send(Action::Shutdown(err.into())) {
                    error!("failed to send rofi shutdown action: {}", send_err);
                }
            }
        })?;

    loop {
        match receiver.recv()? {
            Action::Show(notification) => {
                db.add(notification);
            }
            Action::ShowLast => {
                debug!("showing the last notification");
            }
            Action::Close(id) => {
                if let Some(id) = id {
                    debug!("closing notification: {}", id);
                    db.delete(id);
                }
            }
            Action::CloseAll => {
                debug!("closing all notifications");
                db.delete_all();
            }
            Action::Shutdown(reason) => break Err(reason),
        }
    }
}
