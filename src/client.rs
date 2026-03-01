//! Client API and change-stream subscription for notifications.

use crate::config::ClientConfig;
use crate::dbus_support::{open_connection, CHANGE_INTERFACE, CHANGE_PATH, CHANGE_SIGNAL};
use crate::error::Result;
use crate::model::{
    ChangeEvent, ChangeKind, ListQuery, NewNotification, Notification, UpdateNotification,
};
use crate::repository::SqliteRepository;
use dbus::blocking::Connection;
use dbus::message::MatchRule;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

/// Stream of notification change events.
#[derive(Debug)]
pub struct ChangeStream {
    receiver: Receiver<ChangeEvent>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ChangeStream {
    /// Waits for the next change event up to the provided timeout.
    pub fn next_timeout(&self, timeout: Duration) -> Option<ChangeEvent> {
        self.receiver.recv_timeout(timeout).ok()
    }
}

impl Iterator for ChangeStream {
    type Item = ChangeEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.recv().ok()
    }
}

impl Drop for ChangeStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

/// API for interacting with the SQLite-backed notification store.
#[derive(Clone, Debug)]
pub struct NotifyClient {
    config: ClientConfig,
    repository: SqliteRepository,
}

impl NotifyClient {
    /// Create a new API client.
    pub fn new(config: ClientConfig) -> Result<Self> {
        let repository = SqliteRepository::new(&config.database)?;
        Ok(Self { config, repository })
    }

    /// Applies migrations before serving API requests.
    pub fn migrate(&self) -> Result<()> {
        self.repository.migrate()
    }

    /// Create a new notification.
    pub fn create(&self, req: NewNotification) -> Result<Notification> {
        let _ = &self.config;
        self.repository.create(req)
    }

    /// Retrieve one notification by id.
    pub fn get(&self, id: u64) -> Result<Option<Notification>> {
        let _ = &self.config;
        self.repository.get(id)
    }

    /// List notifications matching query filters.
    pub fn list(&self, query: ListQuery) -> Result<Vec<Notification>> {
        let _ = &self.config;
        self.repository.list(&query)
    }

    /// Update a notification.
    pub fn update(&self, req: UpdateNotification) -> Result<Notification> {
        let _ = &self.config;
        self.repository.update(req)
    }

    /// Close a notification.
    pub fn close(&self, id: u64) -> Result<()> {
        let _ = &self.config;
        self.repository.close(id)
    }

    /// Delete a notification.
    pub fn delete(&self, id: u64) -> Result<()> {
        let _ = &self.config;
        self.repository.delete(id)
    }

    /// Close all active notifications.
    pub fn close_all(&self, app_name: Option<&str>) -> Result<u64> {
        let _ = &self.config;
        self.repository.close_all(app_name)
    }

    /// Subscribe to live change events.
    ///
    /// The stream starts from the current latest change id and yields new events as they arrive.
    pub fn subscribe_changes(&self) -> Result<ChangeStream> {
        let repository = self.repository.clone();
        let start_cursor = repository.latest_change_id()?;
        let dbus_address = self.config.dbus_address.clone();

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_worker = Arc::clone(&stop);

        let worker = thread::Builder::new()
            .name("notify-change-subscriber".to_string())
            .spawn(move || {
                let connection = match open_connection(dbus_address.as_deref()) {
                    Ok(connection) => connection,
                    Err(_) => return,
                };

                if install_change_match(
                    &connection,
                    repository,
                    tx,
                    stop_worker.clone(),
                    start_cursor,
                )
                .is_err()
                {
                    return;
                }

                while !stop_worker.load(Ordering::Relaxed) {
                    if connection.process(Duration::from_millis(200)).is_err() {
                        break;
                    }
                }
            })
            .map_err(crate::Error::from)?;

        Ok(ChangeStream {
            receiver: rx,
            stop,
            worker: Some(worker),
        })
    }
}

fn install_change_match(
    connection: &Connection,
    repository: SqliteRepository,
    tx: mpsc::Sender<ChangeEvent>,
    stop: Arc<AtomicBool>,
    start_cursor: u64,
) -> Result<()> {
    let mut rule = MatchRule::new_signal(CHANGE_INTERFACE, CHANGE_SIGNAL);
    rule.path = Some(CHANGE_PATH.into());

    let mut last_seen = start_cursor;
    connection.add_match(rule, move |signal: (u64, String, u64), _conn, _message| {
        if stop.load(Ordering::Relaxed) {
            return false;
        }

        let (signal_change_id, signal_kind, signal_notification_id) = signal;

        if signal_change_id <= last_seen {
            return true;
        }

        match repository.list_changes_since(last_seen) {
            Ok(changes) => {
                for change in changes {
                    last_seen = change.change_id;
                    if tx.send(change).is_err() {
                        stop.store(true, Ordering::Relaxed);
                        return false;
                    }
                }
            }
            Err(_) => {
                let parsed_kind = signal_kind.parse::<ChangeKind>().ok().map(|kind| ChangeEvent {
                    change_id: signal_change_id,
                    kind,
                    notification_id: if signal_notification_id == 0 {
                        None
                    } else {
                        Some(signal_notification_id)
                    },
                });

                if let Some(change) = parsed_kind {
                    last_seen = change.change_id;
                    if tx.send(change).is_err() {
                        stop.store(true, Ordering::Relaxed);
                        return false;
                    }
                }
            }
        }

        true
    })?;

    Ok(())
}
