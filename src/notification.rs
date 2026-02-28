use serde::Serialize;
use serde_repr::Serialize_repr;
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Possible urgency levels for the notification.
#[derive(Clone, Debug, Default, Serialize_repr, Copy, PartialEq)]
#[repr(u8)]
pub enum Urgency {
    /// Urgency - low
    Low,
    /// Urgency - normal
    #[default]
    Normal,
    /// Urgency - high
    Critical,
}

impl Display for Urgency {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", format!("{self:?}").to_lowercase())
    }
}

impl From<u64> for Urgency {
    fn from(value: u64) -> Self {
        match value {
            0 => Self::Low,
            1 => Self::Normal,
            2 => Self::Critical,
            _ => Self::default(),
        }
    }
}

/// Representation of a notification.
///
/// See [D-Bus Notify Parameters](https://specifications.freedesktop.org/notification-spec/latest/ar01s09.html)
#[derive(Clone, Debug, Default, Serialize)]
pub struct Notification {
    /// notification id
    pub id: u32,
    /// summary
    pub summary: String,
    /// body
    pub body: String,
    /// name of app that generated the notification
    pub application: String,
    /// icon name from app that generated the notification
    pub icon: String,
    /// urgency of notification
    pub urgency: Urgency,
    /// possible actions against notification
    pub actions: Vec<String>,
    /// other notification metadata
    pub hints: HashMap<String, String>,
    /// time that notification was received by daemon
    pub timestamp: u64,
}

/// Specifies internal events
#[derive(Debug)]
pub enum Action {
    /// Show a notification event from dbus
    Show(Notification),
    /// Close a notification event from dbus
    Close(Option<u32>),
    /// Close all the notifications event from dbus
    CloseAll,
    /// A fatal problem occurred, exit
    Shutdown(crate::error::Error),
}

/// Notification database
#[derive(Debug)]
pub struct NotificationStore {
    /// Inner type that holds the notifications in thread-safe way.
    inner: Arc<RwLock<Vec<Notification>>>,
}

impl Clone for NotificationStore {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl NotificationStore {
    /// Initializes the notification db
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Returns the number of notifications.
    pub fn count(&self) -> usize {
        self.inner
            .read()
            .expect("failed to retrieve notifications")
            .len()
    }

    /// Adds a new notifications to manage.
    pub fn add(&self, notification: Notification) {
        self.ds_write().push(notification);
    }

    /// Return a copy of all active notifications at time of call
    pub fn items(&self) -> Vec<Notification> {
        self.ds_read().iter().cloned().collect()
    }

    /// Marks the given notification as read.
    pub fn delete(&self, id: u32) {
        let mut ds = self.ds_write();

        ds.retain(|e| e.id != id);
    }
    /// Marks all the notifications as read.
    pub fn delete_all(&self) {
        self.ds_write().clear()
    }

    /// Marks the given notification as read.
    pub fn delete_from_app(&self, app_name: &str) {
        let mut ds = self.ds_write();

        ds.retain(|e| e.application != app_name);
    }

    /// set the urgency of the notification
    pub fn set_urgency(&self, id: u32, target_urgency: Urgency) {
        let mut ds = self.ds_write();

        if let Some(notification) = ds.iter_mut().find(|n| n.id == id) {
            notification.urgency = target_urgency;
        }
    }

    fn ds_read(&self) -> RwLockReadGuard<'_, Vec<Notification>> {
        self.inner.read().expect("can read from db store")
    }

    fn ds_write(&self) -> RwLockWriteGuard<'_, Vec<Notification>> {
        self.inner.write().expect("can write to db store")
    }
}

impl Default for NotificationStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notification::NotificationStore;

    #[test]
    fn notification_store_init() {
        let unit = NotificationStore::new();

        assert_eq!(
            unit.count(),
            0,
            "initialized store contains no notifications"
        );
    }

    #[test]
    fn notification_store_add() {
        let (unit, added_item) = add_single_item();

        assert_eq!(
            unit.count(),
            1,
            "adding one notification has expected count"
        );

        let binding = unit.items();
        let retrieved_item = binding
            .first()
            .expect("Can get added notification from store");

        assert_eq!(added_item.id, retrieved_item.id);
    }

    #[test]
    fn notification_store_delete_one() {
        let (unit, _) = add_single_item();

        unit.delete(0); // invalid id
        assert_eq!(
            unit.count(),
            1,
            "no change after attempt to delete invalid id"
        );

        unit.delete(1);
        assert_eq!(unit.count(), 0, "count down by own after deleting valid id");
    }

    #[test]
    fn notification_store_delete_by_app() {
        let (unit, _) = add_single_item();

        unit.delete_from_app("invalid_app_name"); // invalid app name
        assert_eq!(
            unit.count(),
            1,
            "no change after attempt to delete invalid id"
        );

        unit.delete_from_app("test-app");
        assert_eq!(unit.count(), 0, "count down by own after deleting valid id");
    }

    #[test]
    fn notification_store_delete_all() {
        let (unit, _) = add_single_item();

        unit.delete_all();
        assert_eq!(unit.count(), 0, "count down by own after deleting valid id");
    }

    #[test]
    fn notification_store_change_urgency() {
        let (unit, _) = add_single_item();

        unit.set_urgency(1, Urgency::Low);

        let notifications = unit.items();
        let n = notifications.first().expect("Has added element");

        assert_eq!(n.id, 1);
        assert_eq!(n.urgency, Urgency::Low);
    }

    fn add_single_item() -> (NotificationStore, Notification) {
        let unit = NotificationStore::new();

        let test_notification: Notification = Notification {
            id: 1,
            summary: "test-summary".to_string(),
            body: "test-body".to_string(),
            application: "test-app".to_string(),
            icon: "test-icon".to_string(),
            urgency: Urgency::Critical,
            actions: vec!["test-action-1".to_string()],
            hints: HashMap::from([(
                "test-hint-key-1".to_string(),
                "test-hint-value-1".to_string(),
            )]),
            timestamp: 1234,
        };

        let test_notification_copy: Notification = Notification {
            summary: test_notification.summary.clone(),
            body: test_notification.body.clone(),
            application: test_notification.application.clone(),
            icon: test_notification.icon.clone(),
            actions: test_notification.actions.clone(),
            hints: test_notification.hints.clone(),
            ..test_notification
        };

        unit.add(test_notification);

        (unit, test_notification_copy)
    }
}
