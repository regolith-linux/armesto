CREATE TABLE IF NOT EXISTS notifications (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  app_name TEXT NOT NULL,
  summary TEXT NOT NULL,
  body TEXT NOT NULL,
  icon TEXT NOT NULL DEFAULT '',
  urgency INTEGER NOT NULL DEFAULT 1 CHECK (urgency IN (0, 1, 2)),
  actions_json TEXT NOT NULL DEFAULT '[]',
  hints_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
  updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
  closed_at TEXT,
  status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'closed'))
);

CREATE TABLE IF NOT EXISTS notification_changes (
  change_id INTEGER PRIMARY KEY AUTOINCREMENT,
  notification_id INTEGER,
  change_kind TEXT NOT NULL CHECK (change_kind IN ('create', 'update', 'close', 'delete', 'close_all')),
  created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
  metadata_json TEXT NOT NULL DEFAULT '{}',
  FOREIGN KEY (notification_id)
    REFERENCES notifications(id)
    ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_notifications_status_created
  ON notifications(status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_notifications_app_status
  ON notifications(app_name, status);

CREATE INDEX IF NOT EXISTS idx_notification_changes_change_id
  ON notification_changes(change_id);

CREATE INDEX IF NOT EXISTS idx_notification_changes_created_at
  ON notification_changes(created_at);
