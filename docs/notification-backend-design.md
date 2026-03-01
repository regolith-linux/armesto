# SQLite-Backed Linux Notification Backend Design

## 1. Overview

This document proposes a new Rust crate, independent from the current `armesto` implementation, to provide a Linux desktop notification backend backed by embedded SQLite.

The crate includes:
1. A low-memory notification server daemon that accepts D-Bus calls and persists all state in SQLite.
2. A CLI tool to read and manage notifications.
3. A client API intended for use from `grelier` with:
   - CRUD operations on notifications.
   - A way to subscribe to "new notification available" events.

## 2. Goals

1. Keep notification state in SQLite, not in long-lived in-memory collections.
2. React to notification changes through D-Bus events.
3. Support server-side mutation operations (create/update/delete/close/close-all).
4. Provide operational CLI for querying and managing stored notifications.
5. Provide a reusable Rust client API for other applications (including `grelier`).
6. Keep RAM usage low and predictable.
7. Eliminate unmanaged host dependencies like the `mysql` CLI or an external database server.

## 3. Non-Goals

1. Full parity with every third-party rofi daemon extension.
2. Reuse of current in-memory `NotificationStore` internals.
3. UI rendering responsibilities.

## 4. Proposed Crate

### 4.2 Crate Outputs

1. `armesto-server` (binary): notification daemon/server.
2. `armesto` (binary): CLI management client.
3. `armesto_notify_backend` (library): data models, DB repo, client API, shared protocol types.

## 5. Architecture

### 5.1 Components

1. D-Bus ingress adapter
   - Implements `org.freedesktop.Notifications` methods (`Notify`, `CloseNotification`, etc.).
   - Converts incoming messages to storage mutations.

2. Notification service core
   - Stateless service layer; each operation maps to one DB transaction.
   - No in-memory long-term notification cache.

3. SQLite repository layer
   - Owns SQL operations and transactional boundaries.
   - Uses direct embedded DB access via `rusqlite`.

4. D-Bus egress notifier
   - Emits internal change signals on notification mutations so clients can react quickly.
   - Proposed interface: `org.armesto.NotifyStore1` signal `NotificationChanged(change_id, kind, notification_id)`.

5. CLI (`armesto`)
   - Calls repository operations directly via embedded SQLite access.

6. Rust client API
   - Exposes typed CRUD methods.
   - Exposes a live subscription API for new change events, with DB recovery semantics.
7. Optional rofi UNIX socket adapter
   - Supports legacy commands (`num`, `list`, `del`, `dels`, `dela`, `saw`).
   - Supports `watch` for new-notification events.

### 5.2 Event Flow

1. App sends `Notify` via D-Bus.
2. `armesto-server` receives D-Bus call and writes notification row in SQLite transactionally.
3. Server appends a change event row in the same store.
4. `grelier` (via client API) fetches latest data from SQLite.

All durable state remains in SQLite.

## 6. Data Model

Database file location policy:
1. Fixed runtime path: `$HOME/.cache/armesto/armesto-<major>.db`
2. `<major>` comes from the backend crate major version.
3. Runtime DB path is not user-configurable from daemon/CLI flags.
4. Path override exists only for automated tests.

### 6.1 Tables

1. `notifications`
   - `id INTEGER PRIMARY KEY AUTOINCREMENT`
   - `app_name TEXT NOT NULL`
   - `summary TEXT NOT NULL`
   - `body TEXT NOT NULL`
   - `icon TEXT NOT NULL DEFAULT ''`
   - `urgency INTEGER NOT NULL DEFAULT 1 CHECK (urgency IN (0,1,2))`
   - `actions_json TEXT NOT NULL DEFAULT '[]'`
   - `hints_json TEXT NOT NULL DEFAULT '{}'`
   - `created_at TEXT NOT NULL DEFAULT (STRFTIME(...))`
   - `updated_at TEXT NOT NULL DEFAULT (STRFTIME(...))`
   - `closed_at TEXT NULL`
   - `status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','closed'))`

2. `notification_changes`
   - `change_id INTEGER PRIMARY KEY AUTOINCREMENT`
   - `notification_id INTEGER NULL`
   - `change_kind TEXT NOT NULL CHECK (...)`
   - `created_at TEXT NOT NULL DEFAULT (STRFTIME(...))`
   - `metadata_json TEXT NOT NULL DEFAULT '{}'`

### 6.2 Indexes

1. `notifications(status, created_at DESC)`
2. `notifications(app_name, status)`
3. `notification_changes(change_id)`
4. `notification_changes(created_at)`

## 7. D-Bus Interfaces

### 7.1 Ingress (Desktop Notification Spec)

Implemented:
1. `Notify(...) -> u32`
2. `CloseNotification(id)`
3. `GetCapabilities()`
4. `GetServerInformation()`

### 7.2 Internal Change Signal

Implemented:
1. Bus name: `org.armesto.NotifyStore1`
2. Object path: `/org/armesto/NotifyStore1`
3. Signal: `NotificationChanged(change_id: t, kind: s, notification_id: t)`

## 8. CLI Design (`armesto`)

Implemented commands:
1. `list [--status active|closed] [--limit N] [--app APP]`
2. `get ID`
3. `create --app APP --summary TEXT --body TEXT [--urgency 0|1|2]`
4. `update ID [--summary TEXT] [--body TEXT] [--urgency 0|1|2]`
5. `close ID`
6. `delete ID`
7. `close-all [--app APP]`
8. `watch` (streams live change events)

## 8.1 Optional Rofi Integration

Implemented via `armesto-server --rofi-socket <path>`:
1. `num`
2. `list`
3. `del:<id>`
4. `dels:<id>`
5. `dela:<app_name>`
6. `saw:<id>`
7. `watch` (line-delimited JSON for new-notification events)

## 9. Client API for `grelier`

### 9.1 Rust API Surface

```rust
pub struct NotifyClient { /* sqlite repo + optional dbus connection */ }

impl NotifyClient {
    pub fn new(cfg: ClientConfig) -> Result<Self>;
    pub fn migrate(&self) -> Result<()>;

    pub fn create(&self, req: NewNotification) -> Result<Notification>;
    pub fn get(&self, id: u64) -> Result<Option<Notification>>;
    pub fn list(&self, q: ListQuery) -> Result<Vec<Notification>>;
    pub fn update(&self, req: UpdateNotification) -> Result<Notification>;
    pub fn close(&self, id: u64) -> Result<()>;
    pub fn delete(&self, id: u64) -> Result<()>;
    pub fn close_all(&self, app: Option<&str>) -> Result<u64>;

    pub fn subscribe_changes(&self) -> Result<ChangeStream>;
}
```

### 9.2 Subscription Strategy

1. Uses D-Bus `NotificationChanged` for low-latency wakeups.
2. Uses DB recovery from last seen `change_id` to avoid missed updates.

## 10. Low-RAM Strategy

1. No in-memory notification cache.
2. SQLite file as source of truth.
3. Blocking D-Bus processing loop.
4. Per-request short-lived allocations.

## 11. Error Handling and Reliability

1. Each mutation operation is transactional.
2. If DB write fails, D-Bus method returns error.
3. Structured error propagation through typed crate errors.

## 12. Security and Operations

1. DB file permissions control data access.
2. Systemd hardening should be used for the daemon.
3. SQLite file can be placed under user-controlled runtime directories.
4. Optional explicit D-Bus bus address can be provided for constrained/containerized environments.

## 13. Implementation Phases

1. Phase 1: Crate skeleton + models + migrations + repository CRUD. Done.
2. Phase 2: `armesto` CLI on top of repository. Done.
3. Phase 3: D-Bus ingress server to persist `Notify`/`Close` operations. Done.
4. Phase 4: D-Bus change signal + live client API subscription. Done.
5. Phase 5: integration tests with ephemeral D-Bus session. Done.

## 14. Testing Plan

1. Unit tests
   - URL/path parsing.
   - SQL mapping and validation.

2. Integration tests
   - CRUD against SQLite file.
   - D-Bus `Notify` creates row.
   - Live change signal subscription path.

## 15. Open Questions

1. Retention policy for closed notifications.
2. Whether to keep this crate standalone or promote it into a top-level Cargo workspace.
3. Whether to add retention cleanup commands to `armesto`.
