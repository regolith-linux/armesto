//! Integration coverage for SQLite repository CRUD and query behavior.

use armesto_notify_backend::{
    ClientConfig, DatabaseConfig, ListQuery, NewNotification, NotificationStatus, NotifyClient,
    UpdateNotification, Urgency,
};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn sqlite_crud_lifecycle() {
    let db_path = temp_db_path("crud");
    let client = NotifyClient::new(ClientConfig {
        database: DatabaseConfig::for_test_path(db_path.to_string_lossy().to_string()),
        ..ClientConfig::default()
    })
    .expect("client should initialize");

    client.migrate().expect("migrations should apply");

    let created = client
        .create(NewNotification {
            app_name: "integration-test".to_string(),
            summary: "hello".to_string(),
            body: "body-text".to_string(),
            icon: "icon-name".to_string(),
            urgency: Urgency::Critical,
            actions: vec!["open".to_string()],
            hints: HashMap::new(),
        })
        .expect("create should succeed");

    assert!(created.id > 0);
    assert_eq!(created.status, NotificationStatus::Active);

    let listed = client
        .list(ListQuery {
            status: Some(NotificationStatus::Active),
            ..ListQuery::default()
        })
        .expect("list should succeed");
    assert_eq!(listed.len(), 1);

    let updated = client
        .update(UpdateNotification {
            id: created.id,
            summary: Some("updated".to_string()),
            body: None,
            urgency: Some(Urgency::Low),
        })
        .expect("update should succeed");
    assert_eq!(updated.summary, "updated");
    assert_eq!(updated.urgency, Urgency::Low);

    client
        .close(created.id)
        .expect("close should succeed for existing notification");

    let closed = client
        .get(created.id)
        .expect("get should succeed")
        .expect("notification should exist");
    assert_eq!(closed.status, NotificationStatus::Closed);

    client
        .delete(created.id)
        .expect("delete should succeed for existing notification");

    let deleted = client
        .get(created.id)
        .expect("get should succeed after delete");
    assert!(deleted.is_none());

    let first = client
        .create(NewNotification {
            app_name: "app-a".to_string(),
            summary: "one".to_string(),
            body: "x".to_string(),
            icon: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            hints: HashMap::new(),
        })
        .expect("first create should succeed");

    let _second = client
        .create(NewNotification {
            app_name: "app-a".to_string(),
            summary: "two".to_string(),
            body: "y".to_string(),
            icon: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            hints: HashMap::new(),
        })
        .expect("second create should succeed");

    let closed_count = client
        .close_all(Some("app-a"))
        .expect("close_all should succeed");
    assert!(closed_count >= 2);

    let first_after = client
        .get(first.id)
        .expect("get should succeed")
        .expect("notification should still exist");
    assert_eq!(first_after.status, NotificationStatus::Closed);

    let _ = std::fs::remove_file(&db_path);
}

fn temp_db_path(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("armesto_notify_backend_{prefix}_{unique}.db"));
    path
}
