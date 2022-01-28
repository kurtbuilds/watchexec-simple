use notify::{Watcher, RecursiveMode, watcher, DebouncedEvent};
use std::borrow::Cow;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{channel, RecvTimeoutError};

use std::time::{Duration, Instant};
use clap::{AppSettings, Arg};
use regex::Regex;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = env!("CARGO_PKG_NAME");


struct Error {
    message: String,
}

impl std::fmt::Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {}

macro_rules! err {
    ($($arg:tt)*) => {
        Error {
            message: format!($($arg)*),
        }
    }
}

#[derive(PartialEq, Eq)]
enum Status {
    RestartProcess,
    Waiting,
    RestartTriggered(Instant),
}

fn main() -> Result<(), Error> {
    let args = clap::App::new(NAME)
        .version(VERSION)
        .about("Run command")
        .setting(AppSettings::ArgRequiredElseHelp)
        .arg(Arg::new("verbose")
            .long("verbose")
            .short('v')
            .takes_value(false)
        )
        .arg(Arg::new("debounce")
            .short('d')
            .long("debounce")
        )
        .arg(Arg::new("ignore")
            .long("ignore")
            .short('i')
            .default_value(".*~")
            .multiple_values(true)
            .multiple_occurrences(true)
        )
        .arg(Arg::new("extensions")
            .long("exts")
            .short('e')
            .takes_value(true)
            .multiple_values(true)
            .multiple_occurrences(true)
        )
        .arg(Arg::new("on-busy-update")
            .long("on-busy-update")
            .takes_value(true)
            .possible_values(&["do-nothing", "queue", "restart", "signal"])
            .default_value("restart")
            .help("The command to execute if the command is busy.")
        )
        .arg(Arg::new("paths")
            .multiple_values(true)
        )
        .arg(Arg::new("command")
            .last(true)
            .min_values(1)
            .required(true)
            .help("The command to execute if an update has occurred.")
        )
        .get_matches();

    let command = args.values_of("command").unwrap().collect::<Vec<&str>>();
    let verbose = args.is_present("verbose");
    let debounce = args.value_of("debounce").unwrap_or("100").parse::<u64>().unwrap();
    let ignore_regexes = args.values_of("ignore")
        .unwrap()
        .map(|i| Regex::new(i).unwrap())
        .collect::<Vec<Regex>>();
    let extensions = args.values_of("extensions")
        .unwrap_or_default()
        .collect::<Vec<&str>>();

    let mut status = Status::RestartProcess;

    let (sender, receiver) = channel();
    // Not sure why, but the built-in debouncing seems to cause us to drop tons of events that should
    // be handled. Instead, we implement our own debouncing.
    let mut watcher = watcher(sender, Duration::from_millis(0)).unwrap();
    for p in args.values_of("paths")
        .map(|v| v.collect::<Vec<&str>>())
        .unwrap_or_else(|| vec!["."]) {
        println!("Watching {}", p);
        watcher.watch(Path::new(p), RecursiveMode::Recursive).unwrap();
    }

    loop {
        if status == Status::RestartProcess {
            status = Status::Waiting;
            if verbose {
                eprintln!("{} {}", command[0], command.iter().skip(1)
                    .map(|s| shell_escape::escape(Cow::from(*s)))
                    .collect::<Vec<Cow<'_, str>>>()
                    .join(" "));
            }
            Command::new(command[0])
                .args(command[1..].iter())
                .spawn()
                .map_err(|_| err!("{}: command not found", command[0]))?;
        }
        match receiver.recv_timeout(Duration::from_millis(debounce)) {
            Ok(event) => {
                let w = match event {
                    DebouncedEvent::NoticeWrite(w)
                    | DebouncedEvent::Write(w)
                    | DebouncedEvent::Chmod(w)
                    => {
                        w
                    }
                    _ => continue,
                };
                if !extensions.is_empty() {
                    match w.extension() {
                        None => continue,
                        Some(e) => {
                            if !extensions.contains(&e.to_string_lossy().as_ref()) {
                                continue;
                            }
                        }
                    }
                }
                if ignore_regexes.iter().any(|r| r.is_match(w.to_string_lossy().as_ref())) {
                    continue;
                }
                eprintln!("{}: File modified. Resetting waiting period.", w.to_str().unwrap());
                status = Status::RestartTriggered(Instant::now());
            }
            Err(e) => {
                match e {
                    RecvTimeoutError::Timeout => {
                        if let Status::RestartTriggered(instant) = status {
                            if instant.elapsed() > Duration::from_millis(debounce) {
                                eprintln!("Triggering restart.");
                                status = Status::RestartProcess;
                            }
                        }
                    }
                    RecvTimeoutError::Disconnected => {
                        return Err(err!("watchexec disconected"));
                    }
                }
            }
        }
    }
}