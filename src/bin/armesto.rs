//! Command-line client for interacting with the notification backend.

use armesto_notify_backend::{
    ClientConfig, DatabaseConfig, Error, ListQuery, NewNotification, NotificationStatus,
    NotifyClient, UpdateNotification, Urgency,
};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::process;

#[derive(Debug, Parser)]
#[command(author, version, about = "CLI for SQLite-backed notification store")]
struct Cli {
    /// Optional explicit D-Bus bus address.
    #[arg(long)]
    dbus_address: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List notifications.
    List {
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        app: Option<String>,
        #[arg(long)]
        limit: Option<u32>,
    },
    /// Get one notification by id.
    Get { id: u64 },
    /// Create a notification.
    Create {
        #[arg(long)]
        app: String,
        #[arg(long)]
        summary: String,
        #[arg(long)]
        body: String,
        #[arg(long, default_value = "1")]
        urgency: u8,
    },
    /// Update a notification.
    Update {
        id: u64,
        #[arg(long)]
        summary: Option<String>,
        #[arg(long)]
        body: Option<String>,
        #[arg(long)]
        urgency: Option<u8>,
    },
    /// Close a notification.
    Close { id: u64 },
    /// Delete a notification.
    Delete { id: u64 },
    /// Close all active notifications.
    CloseAll {
        #[arg(long)]
        app: Option<String>,
    },
    /// Stream change events live.
    Watch,
}

fn main() {
    let cli = Cli::parse();

    let client = match NotifyClient::new(ClientConfig {
        database: DatabaseConfig::default(),
        dbus_address: cli.dbus_address,
        ..ClientConfig::default()
    }) {
        Ok(client) => client,
        Err(err) => {
            eprintln!("unable to initialize client: {err}");
            process::exit(1);
        }
    };

    if let Err(err) = client.migrate() {
        eprintln!("unable to apply migrations: {err}");
        process::exit(1);
    }

    if let Err(err) = run_command(&client, cli.command) {
        eprintln!("command failed: {err}");
        process::exit(1);
    }
}

fn run_command(client: &NotifyClient, command: Command) -> armesto_notify_backend::Result<()> {
    match command {
        Command::List { status, app, limit } => {
            let query = ListQuery {
                status: parse_status(status.as_deref())?,
                app_name: app,
                limit,
            };
            let notifications = client.list(query)?;
            print_json(&notifications);
        }
        Command::Get { id } => {
            let notification = client.get(id)?;
            print_json(&notification);
        }
        Command::Create {
            app,
            summary,
            body,
            urgency,
        } => {
            let notification = client.create(NewNotification {
                app_name: app,
                summary,
                body,
                icon: String::new(),
                urgency: parse_urgency(urgency)?,
                actions: Vec::new(),
                hints: HashMap::new(),
            })?;
            print_json(&notification);
        }
        Command::Update {
            id,
            summary,
            body,
            urgency,
        } => {
            let notification = client.update(UpdateNotification {
                id,
                summary,
                body,
                urgency: urgency.map(parse_urgency).transpose()?,
            })?;
            print_json(&notification);
        }
        Command::Close { id } => {
            client.close(id)?;
            println!("closed {id}");
        }
        Command::Delete { id } => {
            client.delete(id)?;
            println!("deleted {id}");
        }
        Command::CloseAll { app } => {
            let count = client.close_all(app.as_deref())?;
            println!("closed {count} notification(s)");
        }
        Command::Watch => {
            let events = client.subscribe_changes()?;
            for event in events {
                print_json(&event);
            }
        }
    }

    Ok(())
}

fn parse_status(input: Option<&str>) -> armesto_notify_backend::Result<Option<NotificationStatus>> {
    match input {
        None => Ok(None),
        Some("active") => Ok(Some(NotificationStatus::Active)),
        Some("closed") => Ok(Some(NotificationStatus::Closed)),
        Some(other) => Err(Error::Validation(format!(
            "invalid status '{other}', expected active|closed"
        ))),
    }
}

fn parse_urgency(level: u8) -> armesto_notify_backend::Result<Urgency> {
    match level {
        0 => Ok(Urgency::Low),
        1 => Ok(Urgency::Normal),
        2 => Ok(Urgency::Critical),
        other => Err(Error::Validation(format!(
            "invalid urgency '{other}', expected 0|1|2"
        ))),
    }
}

fn print_json<T: serde::Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(output) => println!("{output}"),
        Err(_) => println!("{}", serde_json::to_string(value).unwrap_or_default()),
    }
}
