//! SQLite repository for notification persistence and queries.

use crate::config::DatabaseConfig;
use crate::error::{Error, Result};
use crate::model::{
    ChangeEvent, ChangeKind, ListQuery, NewNotification, Notification, NotificationStatus,
    UpdateNotification, Urgency,
};
use rusqlite::types::Value;
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

const INIT_SQL: &str = include_str!("../migrations/0001_init.sql");
const NOW_SQL: &str = "STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')";

/// SQLite-backed repository.
#[derive(Clone, Debug)]
pub struct SqliteRepository {
    db_path: String,
}

impl SqliteRepository {
    /// Creates a new repository instance.
    pub fn new(config: &DatabaseConfig) -> Result<Self> {
        let db_path = config.resolved_path()?;
        Ok(Self { db_path })
    }

    /// Applies schema migration scripts.
    pub fn migrate(&self) -> Result<()> {
        let conn = self.open_connection()?;
        conn.execute_batch(INIT_SQL)?;
        Ok(())
    }

    /// Inserts a new notification and returns the stored row.
    pub fn create(&self, req: NewNotification) -> Result<Notification> {
        validate_new_notification(&req)?;

        let actions_json = serde_json::to_string(&req.actions)?;
        let hints_json = serde_json::to_string(&req.hints)?;

        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        tx.execute(
            "INSERT INTO notifications (
                app_name, summary, body, icon, urgency, actions_json, hints_json, status, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'active', STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'), STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                req.app_name,
                req.summary,
                req.body,
                req.icon,
                urgency_to_i64(req.urgency),
                actions_json,
                hints_json,
            ],
        )?;

        let id = u64::try_from(tx.last_insert_rowid())
            .map_err(|_| Error::Parse("failed to convert inserted row id".to_string()))?;

        tx.execute(
            "INSERT INTO notification_changes (notification_id, change_kind, metadata_json)
             VALUES (?1, 'create', ?2)",
            params![id, r#"{"source":"api"}"#],
        )?;

        tx.commit()?;

        match self.get(id)? {
            Some(notification) => Ok(notification),
            None => Err(Error::Parse(
                "notification inserted but not found".to_string(),
            )),
        }
    }

    /// Fetches one notification by id.
    pub fn get(&self, id: u64) -> Result<Option<Notification>> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT id, app_name, summary, body, icon, urgency, actions_json, hints_json,
                    status, created_at, updated_at, closed_at
             FROM notifications
             WHERE id = ?1
             LIMIT 1",
        )?;

        let maybe = stmt
            .query_row(params![id], map_notification_row)
            .optional()?;

        Ok(maybe)
    }

    /// Lists notifications matching query filters.
    pub fn list(&self, query: &ListQuery) -> Result<Vec<Notification>> {
        if let Some(limit) = query.limit {
            if limit == 0 {
                return Err(Error::Validation(
                    "limit must be greater than 0".to_string(),
                ));
            }
        }

        let conn = self.open_connection()?;
        let mut sql = String::from(
            "SELECT id, app_name, summary, body, icon, urgency, actions_json, hints_json,
                    status, created_at, updated_at, closed_at
             FROM notifications",
        );
        let mut filters: Vec<String> = Vec::new();
        let mut values: Vec<Value> = Vec::new();

        if let Some(status) = &query.status {
            filters.push("status = ?".to_string());
            values.push(Value::Text(status_to_sql(status).to_string()));
        }

        if let Some(app_name) = &query.app_name {
            filters.push("app_name = ?".to_string());
            values.push(Value::Text(app_name.clone()));
        }

        if !filters.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&filters.join(" AND "));
        }

        sql.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = query.limit {
            sql.push_str(" LIMIT ?");
            values.push(Value::Integer(i64::from(limit)));
        }

        let mut stmt = conn.prepare(&sql)?;
        let iter = stmt.query_map(params_from_iter(values.iter()), map_notification_row)?;
        let mut notifications = Vec::new();
        for row in iter {
            notifications.push(row?);
        }

        Ok(notifications)
    }

    /// Updates a notification and returns the updated row.
    pub fn update(&self, req: UpdateNotification) -> Result<Notification> {
        let mut sets: Vec<String> = Vec::new();
        let mut values: Vec<Value> = Vec::new();

        if let Some(summary) = req.summary {
            sets.push("summary = ?".to_string());
            values.push(Value::Text(summary));
        }
        if let Some(body) = req.body {
            sets.push("body = ?".to_string());
            values.push(Value::Text(body));
        }
        if let Some(urgency) = req.urgency {
            sets.push("urgency = ?".to_string());
            values.push(Value::Integer(urgency_to_i64(urgency)));
        }

        if sets.is_empty() {
            return Err(Error::Validation(
                "update requires at least one mutable field".to_string(),
            ));
        }

        sets.push(format!("updated_at = {NOW_SQL}"));

        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        let sql = format!("UPDATE notifications SET {} WHERE id = ?", sets.join(", "));
        values.push(Value::Integer(i64::try_from(req.id).map_err(|_| {
            Error::Validation("notification id exceeds supported range".to_string())
        })?));

        let changed = tx.execute(&sql, params_from_iter(values.iter()))?;
        if changed == 0 {
            return Err(Error::NotFound(format!(
                "notification id {} not found",
                req.id
            )));
        }

        tx.execute(
            "INSERT INTO notification_changes (notification_id, change_kind, metadata_json)
             VALUES (?1, 'update', ?2)",
            params![req.id, r#"{"source":"api"}"#],
        )?;

        tx.commit()?;

        match self.get(req.id)? {
            Some(notification) => Ok(notification),
            None => Err(Error::Parse(
                "notification updated but not found".to_string(),
            )),
        }
    }

    /// Marks a notification as closed.
    pub fn close(&self, id: u64) -> Result<()> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        let changed = tx.execute(
            "UPDATE notifications
             SET status = 'closed',
                 closed_at = COALESCE(closed_at, STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
                 updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
             WHERE id = ?1 AND status != 'closed'",
            params![id],
        )?;

        if changed == 0 {
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT 1 FROM notifications WHERE id = ?1 LIMIT 1",
                    params![id],
                    |row| row.get(0),
                )
                .optional()?;

            if exists.is_none() {
                return Err(Error::NotFound(format!("notification id {id} not found")));
            }
        } else {
            tx.execute(
                "INSERT INTO notification_changes (notification_id, change_kind, metadata_json)
                 VALUES (?1, 'close', ?2)",
                params![id, r#"{"source":"api"}"#],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    /// Deletes a notification.
    pub fn delete(&self, id: u64) -> Result<()> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        let changed = tx.execute("DELETE FROM notifications WHERE id = ?1", params![id])?;

        if changed == 0 {
            return Err(Error::NotFound(format!("notification id {id} not found")));
        }

        tx.execute(
            "INSERT INTO notification_changes (notification_id, change_kind, metadata_json)
             VALUES (NULL, 'delete', ?1)",
            params![format!(r#"{{"source":"api","notification_id":{id}}}"#)],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Closes all active notifications, optionally filtered by application.
    pub fn close_all(&self, app_name: Option<&str>) -> Result<u64> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;

        let changed = match app_name {
            Some(app_name) => tx.execute(
                "UPDATE notifications
                 SET status = 'closed',
                     closed_at = COALESCE(closed_at, STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
                     updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE status = 'active' AND app_name = ?1",
                params![app_name],
            )?,
            None => tx.execute(
                "UPDATE notifications
                 SET status = 'closed',
                     closed_at = COALESCE(closed_at, STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
                     updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
                 WHERE status = 'active'",
                [],
            )?,
        };

        let metadata = match app_name {
            Some(app_name) => format!(
                r#"{{"source":"api","bulk":true,"app_name":"{}"}}"#,
                app_name.replace('"', "\\\"")
            ),
            None => r#"{"source":"api","bulk":true,"app_name":null}"#.to_string(),
        };

        tx.execute(
            "INSERT INTO notification_changes (notification_id, change_kind, metadata_json)
             VALUES (NULL, 'close_all', ?1)",
            params![metadata],
        )?;

        tx.commit()?;

        u64::try_from(changed)
            .map_err(|_| Error::Parse("close_all changed count overflow".to_string()))
    }

    /// Lists changes after a change id.
    pub fn list_changes_since(&self, change_id: u64) -> Result<Vec<ChangeEvent>> {
        let conn = self.open_connection()?;
        let mut stmt = conn.prepare(
            "SELECT change_id, change_kind, notification_id
             FROM notification_changes
             WHERE change_id > ?1
             ORDER BY change_id ASC",
        )?;

        let mut changes = Vec::new();
        let iter = stmt.query_map(params![change_id], |row| {
            let kind: String = row.get(1)?;
            Ok((row.get::<_, i64>(0)?, kind, row.get::<_, Option<i64>>(2)?))
        })?;

        for row in iter {
            let (change_id, kind, notification_id) = row?;
            changes.push(ChangeEvent {
                change_id: u64::try_from(change_id)
                    .map_err(|_| Error::Parse("change_id overflow".to_string()))?,
                kind: change_kind_from_sql(&kind)?,
                notification_id: notification_id
                    .map(|id| {
                        u64::try_from(id)
                            .map_err(|_| Error::Parse("notification_id overflow".to_string()))
                    })
                    .transpose()?,
            });
        }

        Ok(changes)
    }

    /// Returns count of active notifications.
    pub fn count_active(&self) -> Result<u64> {
        let conn = self.open_connection()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM notifications WHERE status = 'active'",
            [],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| Error::Parse("active count overflow".to_string()))
    }

    /// Returns active notifications only, newest first.
    pub fn list_active(&self) -> Result<Vec<Notification>> {
        self.list(&ListQuery {
            status: Some(NotificationStatus::Active),
            ..ListQuery::default()
        })
    }

    /// Deletes notifications from the given application, returning affected rows.
    pub fn delete_by_app(&self, app_name: &str) -> Result<u64> {
        let mut conn = self.open_connection()?;
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "DELETE FROM notifications WHERE app_name = ?1",
            params![app_name],
        )?;

        if changed > 0 {
            tx.execute(
                "INSERT INTO notification_changes (notification_id, change_kind, metadata_json)
                 VALUES (NULL, 'delete', ?1)",
                params![format!(
                    r#"{{"source":"rofi","app_name":"{}"}}"#,
                    app_name.replace('"', "\\\"")
                )],
            )?;
        }
        tx.commit()?;
        u64::try_from(changed).map_err(|_| Error::Parse("delete_by_app overflow".to_string()))
    }

    /// Sets notification urgency to normal.
    pub fn mark_seen(&self, id: u64) -> Result<()> {
        self.update(UpdateNotification {
            id,
            summary: None,
            body: None,
            urgency: Some(Urgency::Normal),
        })?;
        Ok(())
    }

    /// Returns the most recent change id in storage.
    pub fn latest_change_id(&self) -> Result<u64> {
        let conn = self.open_connection()?;
        let latest: i64 = conn.query_row(
            "SELECT COALESCE(MAX(change_id), 0) FROM notification_changes",
            [],
            |row| row.get(0),
        )?;
        u64::try_from(latest).map_err(|_| Error::Parse("change_id overflow".to_string()))
    }

    fn open_connection(&self) -> Result<Connection> {
        if let Some(parent) = Path::new(&self.db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(&self.db_path)?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;",
        )?;
        Ok(conn)
    }
}

fn map_notification_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Notification> {
    let id: i64 = row.get(0)?;
    let urgency: i64 = row.get(5)?;
    let actions_json: String = row.get(6)?;
    let hints_json: String = row.get(7)?;
    let status: String = row.get(8)?;

    let parsed_actions = serde_json::from_str::<Vec<String>>(&actions_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(err))
    })?;

    let parsed_hints =
        serde_json::from_str::<HashMap<String, String>>(&hints_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(err))
        })?;

    let urgency = urgency_from_i64(urgency).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            5,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;

    let status = status_from_sql(&status).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            8,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                err.to_string(),
            )),
        )
    })?;

    Ok(Notification {
        id: u64::try_from(id).map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Integer,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "id overflow",
                )),
            )
        })?,
        app_name: row.get(1)?,
        summary: row.get(2)?,
        body: row.get(3)?,
        icon: row.get(4)?,
        urgency,
        actions: parsed_actions,
        hints: parsed_hints,
        status,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        closed_at: row.get(11)?,
    })
}

fn validate_new_notification(req: &NewNotification) -> Result<()> {
    if req.app_name.trim().is_empty() {
        return Err(Error::Validation("app_name must not be empty".to_string()));
    }
    if req.summary.trim().is_empty() {
        return Err(Error::Validation("summary must not be empty".to_string()));
    }
    Ok(())
}

fn status_to_sql(status: &NotificationStatus) -> &'static str {
    match status {
        NotificationStatus::Active => "active",
        NotificationStatus::Closed => "closed",
    }
}

fn status_from_sql(value: &str) -> Result<NotificationStatus> {
    match value {
        "active" => Ok(NotificationStatus::Active),
        "closed" => Ok(NotificationStatus::Closed),
        other => Err(Error::Parse(format!("invalid status value '{other}'"))),
    }
}

fn urgency_to_i64(urgency: Urgency) -> i64 {
    match urgency {
        Urgency::Low => 0,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    }
}

fn urgency_from_i64(value: i64) -> Result<Urgency> {
    match value {
        0 => Ok(Urgency::Low),
        1 => Ok(Urgency::Normal),
        2 => Ok(Urgency::Critical),
        other => Err(Error::Parse(format!("invalid urgency value '{other}'"))),
    }
}

fn change_kind_from_sql(value: &str) -> Result<ChangeKind> {
    value
        .parse::<ChangeKind>()
        .map_err(|_| Error::Parse(format!("invalid change kind '{value}'")))
}

#[cfg(test)]
mod tests {
    use crate::config::DatabaseConfig;
    use crate::repository::SqliteRepository;

    #[test]
    fn resolves_default_path() {
        let config = DatabaseConfig::default();
        let path = config.resolved_path().expect("default path should resolve");
        assert!(path.contains(".cache/armesto/armesto-"));
        assert!(path.ends_with(".db"));
    }

    #[test]
    fn accepts_override_path_for_test() {
        let config = DatabaseConfig::for_test_path("/tmp/test-notify.db");
        let repo = SqliteRepository::new(&config).expect("repo should init");
        assert!(format!("{repo:?}").contains("/tmp/test-notify.db"));
    }
}
