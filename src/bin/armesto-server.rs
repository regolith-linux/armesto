//! Command-line entrypoint for running the notification daemon.

use armesto_notify_backend::{DatabaseConfig, NotificationServer, ServerConfig};
use clap::Parser;
use std::process;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(author, version, about = "SQLite-backed notification daemon")]
struct Cli {
    /// Optional explicit D-Bus bus address.
    #[arg(long)]
    dbus_address: Option<String>,

    /// Optional rofi-compatible UNIX socket path.
    #[arg(long)]
    rofi_socket: Option<String>,

    /// D-Bus poll interval in milliseconds.
    #[arg(long, default_value = "1000")]
    dbus_poll_timeout_ms: u64,
}

fn main() {
    let cli = Cli::parse();
    let config = ServerConfig {
        database: DatabaseConfig::default(),
        dbus_address: cli.dbus_address,
        rofi_socket_path: cli.rofi_socket,
        dbus_poll_timeout: Duration::from_millis(cli.dbus_poll_timeout_ms),
        ..ServerConfig::default()
    };

    let server = NotificationServer::new(config);
    if let Err(err) = server.run() {
        eprintln!("armesto-server failed: {err}");
        process::exit(1);
    }
}
