//! D-Bus notification server implementation and method handlers.

use crate::config::ServerConfig;
use crate::dbus_support::{
    open_connection, CHANGE_INTERFACE, CHANGE_PATH, CHANGE_SIGNAL, NOTIFICATION_INTERFACE,
    NOTIFICATION_PATH,
};
use crate::error::{Error, Result};
use crate::model::{ChangeEvent, NewNotification, UpdateNotification, Urgency};
use crate::repository::SqliteRepository;
use crate::rofi::RofiServer;
use dbus::arg::{PropMap, RefArg};
use dbus::blocking::stdintf::org_freedesktop_dbus::RequestNameReply;
use dbus::blocking::Connection;
use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;
use dbus::Message;
use dbus_crossroads::{Context, Crossroads, MethodErr};
use std::sync::{Arc, Mutex};

const SERVER_INFO_SPEC_VERSION: &str = "1.2";

/// Notification daemon entrypoint.
#[derive(Clone, Debug)]
pub struct NotificationServer {
    config: ServerConfig,
}

impl NotificationServer {
    /// Build a new server from configuration.
    pub fn new(config: ServerConfig) -> Self {
        Self { config }
    }

    /// Start the daemon and serve D-Bus notification methods.
    pub fn run(&self) -> Result<()> {
        let repository = self.initialize_repository()?;
        self.start_rofi_if_configured(repository.clone())?;

        let connection = self.open_and_claim_dbus()?;
        let mut crossroads = Crossroads::new();
        register_notification_interface(&mut crossroads, repository)?;

        connection.start_receive(
            MatchRule::new_method_call(),
            Box::new(move |message, conn| {
                let _ = crossroads.handle_message(message, conn);
                true
            }),
        );

        loop {
            connection.process(self.config.dbus_poll_timeout)?;
        }
    }

    fn initialize_repository(&self) -> Result<SqliteRepository> {
        let repository = SqliteRepository::new(&self.config.database)?;
        repository.migrate()?;
        Ok(repository)
    }

    fn start_rofi_if_configured(&self, repository: SqliteRepository) -> Result<()> {
        if let Some(socket_path) = self.config.rofi_socket_path.clone() {
            RofiServer::new(socket_path, repository)
                .start_background()
                .map_err(Error::from)?;
        }
        Ok(())
    }

    fn open_and_claim_dbus(&self) -> Result<Connection> {
        let connection = open_connection(self.config.dbus_address.as_deref())?;
        request_bus_name(&connection, &self.config.notification_bus_name)?;
        request_bus_name(&connection, &self.config.change_bus_name)?;
        Ok(connection)
    }
}

type NotifyArgs = (
    String,
    u32,
    String,
    String,
    String,
    Vec<String>,
    PropMap,
    i32,
);

fn register_notification_interface(
    crossroads: &mut Crossroads,
    repository: SqliteRepository,
) -> Result<()> {
    let last_emitted = Arc::new(Mutex::new(repository.latest_change_id()?));
    let iface_token = crossroads.register(NOTIFICATION_INTERFACE, move |builder| {
        let repo_notify = repository.clone();
        let emitted_notify = Arc::clone(&last_emitted);
        builder.method(
            "Notify",
            (
                "app_name",
                "replaces_id",
                "icon",
                "summary",
                "body",
                "actions",
                "hints",
                "expire_timeout",
            ),
            ("id",),
            move |ctx, _, args: NotifyArgs| handle_notify(ctx, &repo_notify, &emitted_notify, args),
        );

        let repo_close = repository.clone();
        let emitted_close = Arc::clone(&last_emitted);
        builder.method(
            "CloseNotification",
            ("id",),
            (),
            move |ctx, _, (id,): (u32,)| {
                handle_close_notification(ctx, &repo_close, &emitted_close, id)
            },
        );

        builder.method("GetCapabilities", (), ("caps",), |_, _, ()| {
            Ok((vec!["actions".to_string(), "body".to_string()],))
        });

        builder.method(
            "GetServerInformation",
            (),
            ("name", "vendor", "version", "spec_version"),
            |_, _, ()| {
                Ok((
                    env!("CARGO_PKG_NAME").to_string(),
                    env!("CARGO_PKG_AUTHORS").to_string(),
                    env!("CARGO_PKG_VERSION").to_string(),
                    SERVER_INFO_SPEC_VERSION.to_string(),
                ))
            },
        );
    });

    crossroads.insert(NOTIFICATION_PATH, &[iface_token], ());
    Ok(())
}

fn handle_notify(
    ctx: &mut Context,
    repository: &SqliteRepository,
    last_emitted: &Arc<Mutex<u64>>,
    (app_name, replaces_id, icon, summary, body, actions, hints, _expire_timeout): NotifyArgs,
) -> std::result::Result<(u32,), MethodErr> {
    let urgency = map_urgency_hint(&hints);
    let hints = map_hints(&hints);

    let payload = NotifyPayload {
        app_name,
        icon,
        summary,
        body,
        actions,
        hints,
        urgency,
    };
    let id = upsert_notification(repository, replaces_id, payload).map_err(to_method_err)?;

    emit_pending_change_signals(ctx, repository, last_emitted).map_err(to_method_err)?;

    let reply_id = u32::try_from(id).map_err(|_| {
        MethodErr::failed(&format!(
            "notification id {id} cannot be represented as u32"
        ))
    })?;
    Ok((reply_id,))
}

struct NotifyPayload {
    app_name: String,
    icon: String,
    summary: String,
    body: String,
    actions: Vec<String>,
    hints: std::collections::HashMap<String, String>,
    urgency: Urgency,
}

fn upsert_notification(
    repository: &SqliteRepository,
    replaces_id: u32,
    payload: NotifyPayload,
) -> Result<u64> {
    if replaces_id == 0 {
        return create_notification(repository, payload);
    }

    let replace_id_u64 = u64::from(replaces_id);
    if repository.get(replace_id_u64)?.is_some() {
        repository.update(UpdateNotification {
            id: replace_id_u64,
            summary: Some(payload.summary),
            body: Some(payload.body),
            urgency: Some(payload.urgency),
        })?;
        return Ok(replace_id_u64);
    }

    create_notification(repository, payload)
}

fn create_notification(repository: &SqliteRepository, payload: NotifyPayload) -> Result<u64> {
    Ok(repository
        .create(NewNotification {
            app_name: payload.app_name,
            summary: payload.summary,
            body: payload.body,
            icon: payload.icon,
            urgency: payload.urgency,
            actions: payload.actions,
            hints: payload.hints,
        })?
        .id)
}

fn handle_close_notification(
    ctx: &mut Context,
    repository: &SqliteRepository,
    last_emitted: &Arc<Mutex<u64>>,
    id: u32,
) -> std::result::Result<(), MethodErr> {
    repository.close(u64::from(id)).map_err(to_method_err)?;
    emit_pending_change_signals(ctx, repository, last_emitted).map_err(to_method_err)?;
    Ok(())
}

fn emit_pending_change_signals(
    ctx: &mut Context,
    repository: &SqliteRepository,
    last_emitted: &Arc<Mutex<u64>>,
) -> Result<()> {
    let mut cursor = last_emitted
        .lock()
        .map_err(|_| Error::Initialization("failed to lock change signal cursor".to_string()))?;
    let changes = repository.list_changes_since(*cursor)?;

    for change in changes {
        ctx.push_msg(change_signal_message(&change));
        *cursor = change.change_id;
    }

    Ok(())
}

fn change_signal_message(change: &ChangeEvent) -> Message {
    Message::signal(
        &CHANGE_PATH.into(),
        &CHANGE_INTERFACE.into(),
        &CHANGE_SIGNAL.into(),
    )
    .append3(
        change.change_id,
        change.kind.as_str().to_string(),
        change.notification_id.unwrap_or(0),
    )
}

fn request_bus_name(connection: &dbus::blocking::Connection, name: &str) -> Result<()> {
    let reply = connection.request_name(name, false, true, false)?;
    if matches!(
        reply,
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner
    ) {
        Ok(())
    } else {
        Err(Error::Initialization(format!(
            "unable to acquire D-Bus name {name}"
        )))
    }
}

fn to_method_err(err: Error) -> MethodErr {
    MethodErr::failed(&err.to_string())
}

fn map_urgency_hint(hints: &PropMap) -> Urgency {
    hints
        .get("urgency")
        .and_then(|value| value.as_u64())
        .map(|value| match value {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        })
        .unwrap_or(Urgency::Normal)
}

fn map_hints(hints: &PropMap) -> std::collections::HashMap<String, String> {
    hints
        .iter()
        .map(|(key, value)| {
            let rendered = if let Some(text) = value.as_str() {
                text.to_string()
            } else if let Some(number) = value.as_u64() {
                number.to_string()
            } else if let Some(integer) = value.as_i64() {
                integer.to_string()
            } else {
                format!("{value:?}")
            };
            (key.clone(), rendered)
        })
        .collect()
}
