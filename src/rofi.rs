//! Optional UNIX-socket compatibility layer for rofi integrations.

use crate::model::{ChangeKind, Notification, Urgency};
use crate::repository::SqliteRepository;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Optional rofi-compatible UNIX socket integration.
#[derive(Clone, Debug)]
pub struct RofiServer {
    socket_path: String,
    repository: SqliteRepository,
}

#[derive(Debug)]
enum RofiCommand {
    Count,
    List,
    DeleteOne(u64),
    DeleteSimilar(u64),
    DeleteApps(String),
    MarkSeen(u64),
    Watch,
}

#[derive(Clone, Debug, Serialize)]
struct RofiNotification {
    id: u64,
    summary: String,
    body: String,
    application: String,
    icon: String,
    urgency: u8,
    actions: Vec<String>,
    hints: HashMap<String, String>,
    timestamp: u64,
}

impl RofiServer {
    /// Creates a new rofi socket server.
    pub fn new(socket_path: String, repository: SqliteRepository) -> Self {
        Self {
            socket_path,
            repository,
        }
    }

    /// Starts the rofi compatibility server in a background thread.
    pub fn start_background(self) -> std::io::Result<thread::JoinHandle<()>> {
        if Path::new(&self.socket_path).exists() {
            let _ = std::fs::remove_file(&self.socket_path);
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        let shared = Arc::new(self);
        Ok(thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let server = Arc::clone(&shared);
                        let _ = thread::Builder::new()
                            .name("rofi-client".to_string())
                            .spawn(move || {
                                server.handle_request(stream);
                            });
                    }
                    Err(_) => break,
                }
            }
        }))
    }

    fn handle_request(&self, stream: UnixStream) {
        let mut client_in = BufReader::new(&stream);
        let mut client_out = BufWriter::new(&stream);

        let mut line = String::new();
        if client_in.read_line(&mut line).is_err() {
            return;
        }

        match RofiCommand::parse(line.trim()) {
            Some(command) => {
                let _ = self.execute_command(command, &mut client_out);
            }
            None => {
                let _ = client_out.write_all(b"error:unknown command\n");
                let _ = client_out.flush();
            }
        }
    }

    fn execute_command(
        &self,
        command: RofiCommand,
        out: &mut BufWriter<&UnixStream>,
    ) -> std::io::Result<()> {
        match command {
            RofiCommand::Count => {
                let count = self.repository.count_active().unwrap_or_default();
                out.write_all(count.to_string().as_bytes())?;
                out.flush()?;
            }
            RofiCommand::List => {
                let notifications = self
                    .repository
                    .list_active()
                    .unwrap_or_default()
                    .into_iter()
                    .map(to_rofi_notification)
                    .collect::<Vec<_>>();
                let payload =
                    serde_json::to_string(&notifications).unwrap_or_else(|_| "[]".to_string());
                out.write_all(payload.as_bytes())?;
                out.flush()?;
            }
            RofiCommand::DeleteOne(id) => {
                let _ = self.repository.delete(id);
            }
            RofiCommand::DeleteApps(app_name) => {
                let _ = self.repository.delete_by_app(&app_name);
            }
            RofiCommand::DeleteSimilar(id) => {
                if let Ok(Some(notification)) = self.repository.get(id) {
                    let _ = self.repository.delete_by_app(&notification.app_name);
                }
            }
            RofiCommand::MarkSeen(id) => {
                let _ = self.repository.mark_seen(id);
            }
            RofiCommand::Watch => {
                self.stream_new_events(out)?;
            }
        }

        Ok(())
    }

    fn stream_new_events(&self, out: &mut BufWriter<&UnixStream>) -> std::io::Result<()> {
        let mut cursor = self.repository.latest_change_id().unwrap_or(0);

        loop {
            match self.repository.list_changes_since(cursor) {
                Ok(changes) => {
                    for change in changes {
                        cursor = change.change_id;
                        if change.kind != ChangeKind::Create {
                            continue;
                        }

                        let notification = match change.notification_id {
                            Some(id) => self.repository.get(id).ok().flatten(),
                            None => None,
                        };

                        let payload = notification
                            .map(to_rofi_notification)
                            .and_then(|item| serde_json::to_string(&item).ok())
                            .unwrap_or_else(|| {
                                format!(
                                    r#"{{"change_id":{},"event":"new","id":{}}}"#,
                                    change.change_id,
                                    change.notification_id.unwrap_or(0)
                                )
                            });

                        out.write_all(payload.as_bytes())?;
                        out.write_all(b"\n")?;
                        out.flush()?;
                    }
                }
                Err(_) => {
                    thread::sleep(Duration::from_millis(200));
                }
            }

            thread::sleep(Duration::from_millis(200));
        }
    }
}

impl RofiCommand {
    fn parse(input: &str) -> Option<Self> {
        let mut token_iter = input.split(':');

        match token_iter.next() {
            Some("num") => Some(Self::Count),
            Some("list") => Some(Self::List),
            Some("del") => token_iter.next()?.parse::<u64>().ok().map(Self::DeleteOne),
            Some("dels") => token_iter
                .next()?
                .parse::<u64>()
                .ok()
                .map(Self::DeleteSimilar),
            Some("dela") => Some(Self::DeleteApps(token_iter.next()?.trim().to_string())),
            Some("saw") => token_iter.next()?.parse::<u64>().ok().map(Self::MarkSeen),
            Some("watch") => Some(Self::Watch),
            _ => None,
        }
    }
}

fn to_rofi_notification(notification: Notification) -> RofiNotification {
    RofiNotification {
        id: notification.id,
        summary: notification.summary,
        body: notification.body,
        application: notification.app_name,
        icon: notification.icon,
        urgency: match notification.urgency {
            Urgency::Low => 0,
            Urgency::Normal => 1,
            Urgency::Critical => 2,
        },
        actions: notification.actions,
        hints: notification.hints,
        timestamp: unix_now(),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{RofiCommand, RofiServer};
    use crate::config::DatabaseConfig;
    use crate::model::{NewNotification, Urgency};
    use crate::repository::SqliteRepository;
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn parses_rofi_commands() {
        assert!(matches!(
            RofiCommand::parse("num"),
            Some(RofiCommand::Count)
        ));
        assert!(matches!(
            RofiCommand::parse("list"),
            Some(RofiCommand::List)
        ));
        assert!(matches!(
            RofiCommand::parse("watch"),
            Some(RofiCommand::Watch)
        ));
        assert!(RofiCommand::parse("del:12").is_some());
        assert!(RofiCommand::parse("unknown").is_none());
    }

    #[test]
    fn rofi_num_list_and_watch_work() {
        let db_path = temp_db_path("rofi");
        let socket = temp_socket_path("rofi");

        let repo = SqliteRepository::new(&DatabaseConfig::for_test_path(
            db_path.to_string_lossy().to_string(),
        ))
        .expect("repo should init");
        repo.migrate().expect("migrations should apply");

        let _handle = match RofiServer::new(socket.to_string_lossy().to_string(), repo.clone())
            .start_background()
        {
            Ok(handle) => handle,
            Err(err) => {
                eprintln!("skipping rofi socket test: {err}");
                return;
            }
        };
        thread::sleep(Duration::from_millis(100));

        let num_raw = send_command(&socket, "num\n");
        assert_eq!(num_raw.trim(), "0");

        repo.create(sample_new_notification("first"))
            .expect("create should work");
        let num_after = send_command(&socket, "num\n");
        assert_eq!(num_after.trim(), "1");

        let list_raw = send_command(&socket, "list\n");
        let parsed: serde_json::Value =
            serde_json::from_str(&list_raw).expect("list response should be valid json");
        assert_eq!(parsed.as_array().map(|a| a.len()).unwrap_or(0), 1);

        let mut watch_stream =
            UnixStream::connect(&socket).expect("watch connection should be established");
        watch_stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set_read_timeout should work");
        watch_stream
            .write_all(b"watch\n")
            .expect("watch command should be sent");

        repo.create(sample_new_notification("second"))
            .expect("second create should work");

        let mut reader = BufReader::new(watch_stream);
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("watch stream should produce event line");
        assert!(!line.trim().is_empty());

        let watch_json: serde_json::Value =
            serde_json::from_str(line.trim()).expect("watch line should be json");
        assert_eq!(watch_json["summary"], "second");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(&socket);
    }

    fn send_command(socket: &PathBuf, command: &str) -> String {
        let mut stream = UnixStream::connect(socket).expect("socket should connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("set_read_timeout should work");
        stream
            .write_all(command.as_bytes())
            .expect("command should be written");
        stream.flush().expect("command should flush");

        let mut response = String::new();
        let mut reader = BufReader::new(stream);
        let _ = reader.read_line(&mut response);
        if response.is_empty() {
            let mut bytes = Vec::new();
            let _ = reader.read_to_end(&mut bytes);
            response = String::from_utf8_lossy(&bytes).to_string();
        }
        response
    }

    fn sample_new_notification(summary: &str) -> NewNotification {
        NewNotification {
            app_name: "test-app".to_string(),
            summary: summary.to_string(),
            body: "body".to_string(),
            icon: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            hints: HashMap::new(),
        }
    }

    fn temp_db_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("armesto_notify_backend_{prefix}_{unique}.db"));
        path
    }

    fn temp_socket_path(prefix: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("armesto_notify_backend_{prefix}_{unique}.sock"))
    }
}
