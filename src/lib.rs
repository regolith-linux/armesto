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
use clap::Parser;
use log::{debug, error};
use notification::Action;
use notification::NotificationStore;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Startup configuration
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Config {
    /// Local path to file representing domain socket
    #[arg(short, long, default_value = "/tmp/armesto")]
    pub socket_path: String,

    /// Duration to wait for incoming d-bus messages
    #[arg(short, long, default_value_t = 1000)]
    pub dbus_poll_timeout: u16,
}

/// Service entry-point
pub fn run(config: Config) -> Result<()> {
    let dbus_server = DbusServer::init()?;
    let db = NotificationStore::init();
    let (dbus_sender, receiver) = mpsc::channel();
    let rofi_sender = dbus_sender.clone();

    thread::Builder::new()
        .name("dbus".to_string())
        .spawn(move || {
            debug!("registering D-Bus server");
            let dbus_sender2 = dbus_sender.clone();
            let duration = Duration::from_millis(config.dbus_poll_timeout.into());
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
            let rofi_server =
                RofiServer::new("/tmp/rofi_notification_daemon".to_string(), db_clone);
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
