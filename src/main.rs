use armesto::Config;
use clap::Parser;
use log::{debug, error, LevelFilter};
use std::process;
use syslog::{BasicLogger, Facility, Formatter3164};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct CliConfig {
    /// Local path to file representing domain socket
    #[arg(short, long, default_value = "/tmp/armesto")]
    socket_path: String,

    /// Duration to wait for incoming d-bus messages
    #[arg(short, long, default_value_t = 1000)]
    dbus_poll_timeout: u16,
}

fn main() {
    let formatter = Formatter3164 {
        facility: Facility::LOG_USER,
        hostname: None,
        process: "armesto".into(),
        pid: 0,
    };

    let logger = match syslog::unix(formatter) {
        Err(e) => {
            println!("unable to connect to syslog: {:?}", e);
            return;
        }
        Ok(logger) => logger,
    };

    log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
        .map(|()| log::set_max_level(LevelFilter::Debug))
        .expect("can set logger");

    let cli = CliConfig::parse();
    let config = Config {
        socket_path: cli.socket_path,
        dbus_poll_timeout: cli.dbus_poll_timeout,
    };
    debug!("Starting armesto with {:?}", config);

    match armesto::run(config) {
        Ok(_) => process::exit(0),
        Err(e) => {
            error!("Unable to start armesto, aborting: {:?}", e);
            process::exit(1)
        }
    }
}
