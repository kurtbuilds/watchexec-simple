#![forbid(unsafe_code)]

extern crate core;

mod error;
mod filter;
use notify::{DebouncedEvent, RecursiveMode, Watcher, watcher};
use std::borrow::Cow;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, RecvTimeoutError, TryRecvError};
use std::thread;
use crate::error::Error;

use std::time::{Duration, Instant};
use clap::{AppSettings, Arg, ArgMatches};
use command_group::{CommandGroup, GroupChild, Signal, UnixChildExt};
use glob::{Pattern, PatternError};
use ignore::gitignore::{Gitignore, Glob};
use ignore::Match;
use nix::libc::exit;
use nix::sys;
use nix::unistd::Pid;
use regex::Regex;
use filter::Filter;
use crate::filter::find_project_gitignore;


macro_rules! cond_eprintln {
    ($cond:expr, $($arg:tt)*) => {
        if $cond {
            eprintln!($($arg)*);
        }
    }
}
const VERSION: &str = env!("CARGO_PKG_VERSION");
const NAME: &str = env!("CARGO_PKG_NAME");

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


#[derive(PartialEq, Eq)]
enum BusyAction {
    Restart,
    DoNothing,
    Queue,
}


fn build_app() -> clap::App<'static> {
    clap::App::new(NAME)
        .version(VERSION)
        .about("Run command")
        .setting(AppSettings::ArgRequiredElseHelp)
        .arg(Arg::new("verbose")
            .long("verbose")
            .short('v')
        )
        .arg(Arg::new("debounce")
            .short('d')
            .long("debounce")
            .default_value("100")
            .help("Set the timeout between detected change and command execution, defaults to 100ms")
        )
        .arg(Arg::new("ignore")
            .long("ignore")
            .short('i')
            .help("Ignore paths matching the pattern")
            .takes_value(true)
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
            .possible_values(&["do-nothing", "queue", "signal"])
            .default_value("signal")
            .help("Select the behaviour to use when receiving events while the command is running")
        )
        .arg(Arg::new("signal")
            .long("signal")
            .takes_value(true)
            .possible_values(&["SIGHUP", "SIGINT", "SIGQUIT", "SIGTERM"])
            .default_value("SIGTERM")
            .help("The signal to send to the command if on-busy-update is set to signal")
        )
        .arg(Arg::new("no-default-ignore")
            .long("no-default-ignore")
            .help("Do not use the default ignore globs")
        )
        .arg(Arg::new("no-global-ignore")
            .long("no-global-ignore")
        )
        .arg(Arg::new("no-project-ignore")
            .long("no-project-ignore")
            .help("Skip auto-loading of project ignore files (.gitignore, etc)")
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
}

fn signal_with_kill_fallback(mut child: GroupChild, signal: Signal) -> Result<(), Error> {
    if child.try_wait().is_ok() {
            return Ok(());
    }
    child.signal(signal).unwrap();
    let start = Instant::now();
    let mut has_printed_failure = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => { break; }
            Ok(None) => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                return Err(err!("Failed to wait for child process to exit"));
            }
        }
        if start.elapsed() > Duration::from_millis(200) && !has_printed_failure {
            has_printed_failure = true;
            println!("Child didn't immediately exit. Giving it 5 seconds to exit before forcibly killing it, which might prevent any cleanup operations it performs.");
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().unwrap();
            return Err(err!("Child failed to exit gracefully within 5 seconds. Killing it."));
        }
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let args = build_app().get_matches();

    let command = args.values_of("command").unwrap().collect::<Vec<&str>>();
    let signal = match args.value_of("signal").unwrap().to_uppercase().as_ref() {
        "SIGHUP" => Signal::SIGHUP,
        "SIGINT" => Signal::SIGINT,
        "SIGQUIT" => Signal::SIGQUIT,
        "SIGTERM" => Signal::SIGTERM,
        _ => return Err(err!("Invalid signal. Choices are: SIGHUP, SIGINT, SIGQUIT, SIGTERM"))
    };
    let verbose = args.is_present("verbose");
    let debounce = args.value_of("debounce").unwrap().parse::<u64>().unwrap();

    let extensions = args.values_of("extensions")
        .unwrap_or_default()
        .collect::<Vec<&str>>();

    let mut ignore_globs = args.values_of("ignore")
        .unwrap_or_default()
        .map(|i| Pattern::new(i))
        .collect::<Result<Vec<_>, PatternError>>()
        .map_err(|e| err!("Invalid ignore glob: {}", e))?;

    if !args.is_present("no-default-ignore") {
        ignore_globs.push(Pattern::new("*~")
            .map_err(|e| err!("Invalid ignore glob: {}", e))?);
        ignore_globs.push(Pattern::new(".DS_Store")
            .map_err(|e| err!("Invalid ignore glob: {}", e))?);
        ignore_globs.push(Pattern::new(".git")
            .map_err(|e| err!("Invalid ignore glob: {}", e))?);
    }

    let strategy = match args.value_of("on-busy-update").unwrap() {
        "signal" => BusyAction::Restart,
        "queue" => BusyAction::Queue,
        "do-nothing" => BusyAction::DoNothing,
        _ => return Err(err!("Invalid on-busy-update. Choices are: signal, queue, do-nothing"))
    };

    let gitignore = if args.is_present("no-project-ignore") {
        None
    } else {
        find_project_gitignore()
    };

    let global_gitignore = if args.is_present("no-global-ignore") {
        None
    } else {
        let (g, _) = Gitignore::global();
        Some(g)
    };

    let (sender, receiver) = channel();
    // Not sure why, but the built-in debouncing seems to cause us to drop tons of events that should
    // be handled. Instead, we implement our own debouncing.
    let mut watcher = watcher(sender, Duration::from_millis(0)).unwrap();
    let mut watched_files = Vec::new();
    for s in args.values_of("paths")
        .map(|v| v.collect::<Vec<&str>>())
        .unwrap_or_else(|| vec!["."]) {
        let p = Path::new(s);
        if p.is_dir() {
            cond_eprintln!(verbose, "{}: Watching directory", p.to_string_lossy());
            watcher.watch(p, RecursiveMode::Recursive).unwrap();
        } else {
            cond_eprintln!(verbose, "{}: Watching file", p.to_string_lossy());
            watcher.watch(p, RecursiveMode::NonRecursive).unwrap();
            watched_files.push(s);
        }
    }

    let filter = Filter {
        watched_files,
        extensions,
        gitignore,
        global_gitignore,
        ignore_globs,
    };

    let (signal_sender, signal_receiver) = channel::<Signal>();

    let mut status = Status::RestartProcess;
    let mut child: Option<GroupChild> = None;

    ctrlc::set_handler(move || {
        signal_sender.send(Signal::SIGINT).unwrap();
    }).unwrap();

    loop {
        if status == Status::RestartProcess {
            status = Status::Waiting;

            match strategy {
                BusyAction::Restart => {
                    if let Some(mut child) = child.take() {
                        cond_eprintln!(verbose, "Waiting for process to exit...");
                        child.signal(signal).unwrap();
                        child.wait().unwrap();
                        cond_eprintln!(verbose, "Exited");
                    }
                }
                BusyAction::DoNothing => {
                    if let Some(mut child) = child.as_mut() {
                        match child.try_wait().unwrap() {
                            None => {
                                continue;
                            }
                            Some(_) => {}
                        }
                    }
                }
                BusyAction::Queue => {
                    if let Some(mut child) = child.as_mut() {
                        match child.try_wait().unwrap() {
                            None => {
                                status = Status::RestartProcess;
                                thread::sleep(Duration::from_millis(50));
                                continue;
                            }
                            Some(_) => {}
                        }
                    }
                }
            }

            cond_eprintln!(verbose, "{}", command.iter()
                .map(|s| shell_escape::escape(Cow::from(*s)))
                .collect::<Vec<Cow<'_, str>>>()
                .join(" "));
            child = Some(Command::new(&command[0])
                .args(&command[1..])
                .group_spawn()
                .map_err(|_| err!("{}: command not found", command[0]))?);
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
                if !filter::handle_event(&w, &filter) {
                    continue;
                }
                cond_eprintln!(verbose, "{}: File modified. Queuing restart.", w.to_str().unwrap());
                status = Status::RestartTriggered(Instant::now());
            }
            Err(e) => {
                match e {
                    RecvTimeoutError::Timeout => {
                        if let Status::RestartTriggered(instant) = status {
                            if instant.elapsed() > Duration::from_millis(debounce) {
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
        match signal_receiver.try_recv() {
            Ok(signal) => {
                if let Some(mut child) = child {
                    return signal_with_kill_fallback(child, signal);
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                return Err(err!("watchexec disconected"));
            }
        }
    }
}
