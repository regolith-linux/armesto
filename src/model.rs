//! Data model types for notifications and change events.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;

/// Notification urgency.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(u8)]
pub enum Urgency {
    /// Low urgency.
    Low = 0,
    /// Normal urgency.
    #[default]
    Normal = 1,
    /// Critical urgency.
    Critical = 2,
}

/// Notification lifecycle status.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum NotificationStatus {
    /// Notification is currently active.
    #[default]
    Active,
    /// Notification has been closed.
    Closed,
}

/// Canonical notification record.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Notification {
    /// Persistent notification id.
    pub id: u64,
    /// Source application name.
    pub app_name: String,
    /// Summary/title.
    pub summary: String,
    /// Body text.
    pub body: String,
    /// App-provided icon token.
    pub icon: String,
    /// Notification urgency.
    pub urgency: Urgency,
    /// Optional action definitions.
    pub actions: Vec<String>,
    /// Optional metadata hints.
    pub hints: HashMap<String, String>,
    /// Current status.
    pub status: NotificationStatus,
    /// UTC timestamp (RFC3339) for creation.
    pub created_at: String,
    /// UTC timestamp (RFC3339) for last update.
    pub updated_at: String,
    /// Optional UTC timestamp (RFC3339) for closure.
    pub closed_at: Option<String>,
}

/// Input for creating a notification.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NewNotification {
    /// Source application name.
    pub app_name: String,
    /// Summary/title.
    pub summary: String,
    /// Body text.
    pub body: String,
    /// Icon token.
    pub icon: String,
    /// Urgency level.
    pub urgency: Urgency,
    /// Optional action definitions.
    pub actions: Vec<String>,
    /// Optional metadata hints.
    pub hints: HashMap<String, String>,
}

/// Input for updating a notification.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UpdateNotification {
    /// Target notification id.
    pub id: u64,
    /// Optional updated summary.
    pub summary: Option<String>,
    /// Optional updated body.
    pub body: Option<String>,
    /// Optional updated urgency.
    pub urgency: Option<Urgency>,
}

/// Query options for list operations.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ListQuery {
    /// Optional status filter.
    pub status: Option<NotificationStatus>,
    /// Optional application filter.
    pub app_name: Option<String>,
    /// Optional result limit.
    pub limit: Option<u32>,
}

/// Change event type.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ChangeKind {
    /// Notification created.
    Create,
    /// Notification updated.
    Update,
    /// Notification closed.
    Close,
    /// Notification deleted.
    Delete,
    /// Bulk close performed.
    CloseAll,
}

impl ChangeKind {
    /// Returns canonical lower-case representation used in storage and wire signaling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Close => "close",
            Self::Delete => "delete",
            Self::CloseAll => "close_all",
        }
    }

}

impl FromStr for ChangeKind {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "create" => Ok(Self::Create),
            "update" => Ok(Self::Update),
            "close" => Ok(Self::Close),
            "delete" => Ok(Self::Delete),
            "close_all" => Ok(Self::CloseAll),
            _ => Err(()),
        }
    }
}

/// Storage mutation event for stream consumers.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ChangeEvent {
    /// Monotonic change id.
    pub change_id: u64,
    /// Mutation type.
    pub kind: ChangeKind,
    /// Optional target notification id.
    pub notification_id: Option<u64>,
}
