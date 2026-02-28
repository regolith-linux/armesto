use log::{debug, error, warn};
use std::{
    io::BufRead,
    io::{BufReader, BufWriter, Error, ErrorKind, Write},
    os::unix::net::{UnixListener, UnixStream},
};

use crate::notification::{NotificationStore, Urgency};

/// Provides service to roficiation clients. See https://github.com/DaveDavenport/Rofication
pub struct RofiServer {
    socket_path: String,
    db: NotificationStore,
}

/// See https://github.com/DaveDavenport/Rofication/blob/master/rofication-daemon.py#LL155C1-L170C87
pub enum RofiCommand {
    /// Retrieve count of notifications
    Count,
    /// Retrieve all notifications
    List,
    /// Delete notification by id
    DeleteOne(u32),
    /// Delete all notifications with same app as id
    DeleteSimilar(u32),
    /// Delete all notifications with app name
    DeleteApps(String),
    /// Reduce urgency to 'normal'
    MarkSeen(u32),
}

impl RofiCommand {
    fn parse(client_request: &str) -> Option<RofiCommand> {
        let mut token_iter = client_request.split(':');

        match token_iter.next() {
            Some(command) => match command {
                "num" => Some(Self::Count),
                "list" => Some(Self::List),
                "del" => {
                    let id = token_iter.next()?.parse::<u32>().ok()?;

                    Some(Self::DeleteOne(id))
                }
                "dels" => {
                    let id = token_iter.next()?.parse::<u32>().ok()?;

                    Some(Self::DeleteSimilar(id))
                }
                "dela" => {
                    let app_name = token_iter.next()?.trim().to_string();

                    Some(Self::DeleteApps(app_name))
                }
                "saw" => {
                    let id = token_iter.next()?.parse::<u32>().ok()?;

                    Some(Self::MarkSeen(id))
                }
                unrecognized_cmd => {
                    warn!("unknown command: '{}'", unrecognized_cmd);
                    None
                }
            },
            None => None,
        }
    }
}

impl RofiServer {
    /// Create a new server instance
    pub fn new(socket_path: String, db: NotificationStore) -> RofiServer {
        RofiServer { socket_path, db }
    }

    /// Server listens for incoming requests, blocks
    pub fn start(&self) -> std::io::Result<()> {
        debug!("Rofication server binding to path {}", &self.socket_path);
        let listener = UnixListener::bind(&self.socket_path)?;

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    self.handle_request(stream);
                }
                Err(err) => {
                    println!("Failed to initialize socket listener: {}", err);
                    break;
                }
            }
        }
        Ok(())
    }

    fn handle_request(&self, stream: UnixStream) {
        let mut client_in = BufReader::new(&stream);
        let mut client_out = BufWriter::new(&stream);

        let mut line = String::new();
        let _ = client_in.read_line(&mut line).expect("unable to read");

        let line = line.trim();
        debug!("Rofication client request: '{}'", line);

        match RofiCommand::parse(line) {
            Some(command) => {
                if let Err(err) = self.execute_command(command, &mut client_out) {
                    error!("Failed to execute command: {}", err);
                }
            }
            None => error!("Unable to parse message, no action taken: {}", &line),
        }
    }

    fn execute_command(
        &self,
        cmd: RofiCommand,
        client_out: &mut BufWriter<&UnixStream>,
    ) -> std::io::Result<()> {
        match cmd {
            RofiCommand::Count => {
                client_out.write_all(self.db.count().to_string().as_bytes())?;
                client_out.flush()?;
            }
            RofiCommand::List => {
                let elems = self.db.items();
                let response =
                    serde_json::to_string(&elems).map_err(|err| Error::new(ErrorKind::Other, err))?;
                client_out.write_all(response.as_bytes())?;
                client_out.flush()?;
            }
            RofiCommand::DeleteOne(id) => {
                self.db.delete(id);
            }
            RofiCommand::DeleteApps(app_name) => {
                self.db.delete_from_app(app_name);
            }
            RofiCommand::DeleteSimilar(id) => {
                let notifications = self.db.items();
                let source_notification = notifications.iter().find(|n| n.id == id);

                if let Some(source_notification) = source_notification {
                    let app_name = source_notification.application.clone();

                    if !app_name.is_empty() {
                        self.db.delete_from_app(app_name);
                    }
                }
            }
            RofiCommand::MarkSeen(id) => {
                self.db.set_urgency(id, Urgency::Normal);
            }
        }
        Ok(())
    }
}
