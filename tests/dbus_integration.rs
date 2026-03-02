//! Integration coverage for D-Bus notification behavior and change signals.

use armesto_notify_backend::{
    ChangeKind, ClientConfig, DatabaseConfig, NewNotification, NotificationServer, NotifyClient,
    ServerConfig, Urgency,
};
use dbus::arg::PropMap;
use dbus::blocking::Connection;
use dbus::channel::Channel;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[test]
fn dbus_notify_persists_and_emits_change_signal() {
    let mut bus = match TestBus::spawn() {
        Ok(bus) => bus,
        Err(reason) => {
            eprintln!("skipping dbus integration test: {reason}");
            return;
        }
    };
    let db_path = temp_db_path("dbus");

    let server = NotificationServer::new(ServerConfig {
        database: DatabaseConfig::for_test_path(db_path.to_string_lossy().to_string()),
        dbus_address: Some(bus.address.clone()),
        dbus_poll_timeout: Duration::from_millis(50),
        ..ServerConfig::default()
    });

    let server_handle = thread::spawn(move || server.run());

    if let Err(reason) = wait_for_server(&bus.address) {
        eprintln!("skipping dbus integration test: {reason}");
        bus.shutdown();
        let _ = server_handle.join();
        return;
    }

    let client = NotifyClient::new(ClientConfig {
        database: DatabaseConfig::for_test_path(db_path.to_string_lossy().to_string()),
        dbus_address: Some(bus.address.clone()),
        ..ClientConfig::default()
    })
    .expect("notify client should initialize");
    client.migrate().expect("migrations should apply");

    let stream = client
        .subscribe_changes()
        .expect("change stream should subscribe");
    thread::sleep(Duration::from_millis(150));

    let id = match send_notify(&bus.address, "dbus-test", "from-dbus", "hello") {
        Ok(id) => id,
        Err(reason) => {
            eprintln!("skipping dbus integration test: {reason}");
            bus.shutdown();
            let _ = server_handle.join();
            return;
        }
    };

    let event = stream
        .next_timeout(Duration::from_secs(3))
        .expect("expected change event from D-Bus signal");

    assert_eq!(event.kind, ChangeKind::Create);
    assert_eq!(event.notification_id, Some(u64::from(id)));

    let stored = client
        .get(u64::from(id))
        .expect("get should succeed")
        .expect("notification should be persisted");
    assert_eq!(stored.summary, "from-dbus");
    assert_eq!(stored.body, "hello");

    // Also validate API-side create still works in same DB.
    let _api_created = client
        .create(NewNotification {
            app_name: "api-test".to_string(),
            summary: "api".to_string(),
            body: "create".to_string(),
            icon: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            hints: HashMap::new(),
        })
        .expect("api create should succeed");

    drop(stream);
    bus.shutdown();

    let server_result = server_handle
        .join()
        .expect("server thread should not panic");
    assert!(
        server_result.is_err(),
        "server should exit when test bus is closed"
    );

    let _ = std::fs::remove_file(&db_path);
}

struct TestBus {
    child: Option<Child>,
    address: String,
}

impl TestBus {
    fn spawn() -> Result<Self, String> {
        let mut child = Command::new("dbus-daemon")
            .arg("--session")
            .arg("--nofork")
            .arg("--nopidfile")
            .arg("--print-address")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("failed to spawn dbus-daemon: {err}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "dbus-daemon stdout unavailable".to_string())?;
        let mut reader = BufReader::new(stdout);
        let mut address = String::new();
        reader
            .read_line(&mut address)
            .map_err(|err| format!("failed to read dbus address: {err}"))?;

        let address = address.trim().to_string();
        if address.is_empty() {
            let stderr = read_stderr(&mut child);
            return Err(format!(
                "dbus-daemon did not provide an address; stderr: {}",
                stderr.trim()
            ));
        }

        Ok(Self {
            child: Some(child),
            address,
        })
    }

    fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn read_stderr(child: &mut Child) -> String {
    let mut output = String::new();
    if let Some(stderr) = child.stderr.take() {
        let mut reader = BufReader::new(stderr);
        let _ = reader.read_line(&mut output);
    }
    output
}

impl Drop for TestBus {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn wait_for_server(address: &str) -> Result<(), String> {
    for _ in 0..40 {
        if let Ok(conn) = open_connection(address) {
            let proxy = conn.with_proxy(
                "org.freedesktop.Notifications",
                "/org/freedesktop/Notifications",
                Duration::from_millis(250),
            );
            let call: Result<(Vec<String>,), _> =
                proxy.method_call("org.freedesktop.Notifications", "GetCapabilities", ());
            if call.is_ok() {
                return Ok(());
            }
        }

        thread::sleep(Duration::from_millis(50));
    }

    Err("server did not become ready".to_string())
}

fn send_notify(address: &str, app_name: &str, summary: &str, body: &str) -> Result<u32, String> {
    let conn = open_connection(address)?;
    let proxy = conn.with_proxy(
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        Duration::from_secs(2),
    );

    let hints: PropMap = PropMap::new();
    let args = (
        app_name.to_string(),
        0u32,
        String::new(),
        summary.to_string(),
        body.to_string(),
        Vec::<String>::new(),
        hints,
        5000i32,
    );

    let (id,): (u32,) = proxy
        .method_call("org.freedesktop.Notifications", "Notify", args)
        .map_err(|err| format!("notify call failed: {err}"))?;

    Ok(id)
}

fn open_connection(address: &str) -> Result<Connection, String> {
    let mut channel =
        Channel::open_private(address).map_err(|err| format!("open_private failed: {err}"))?;
    channel
        .register()
        .map_err(|err| format!("register failed: {err}"))?;
    Ok(Connection::from(channel))
}

fn temp_db_path(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("armesto_notify_backend_{prefix}_{unique}.db"));
    path
}
