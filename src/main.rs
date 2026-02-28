use armesto::Config;
use clap::Parser;
use log::{debug, error, LevelFilter};
use std::process;
use syslog::{BasicLogger, Facility, Formatter3164};

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

    let config = Config::parse();
    debug!("Starting armesto with {:?}", config);

    match armesto::run(config) {
        Ok(_) => process::exit(0),
        Err(e) => {
            error!("Unable to start armesto, aborting: {:?}", e);
            process::exit(1)
        }
    }
}
