#![forbid(unsafe_code)]
#![forbid(unused)]

extern crate core;

use std::path::{Path, PathBuf};
use std::process::{Command};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use clap::{Parser, ValueEnum};
use command_group::{CommandGroup, GroupChild, Signal, UnixChildExt};
use glob::{Pattern, PatternError};
use ignore::gitignore::Gitignore;
use notify::{DebouncedEvent, RecursiveMode, watcher, Watcher};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use filter::Filter;
use tracing::{Level, debug};

use crate::error::Error;
use crate::filter::find_project_gitignore;

mod error;
mod filter;


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

#[derive(ValueEnum, Debug, Copy, Clone)]
enum OnBusyUpdate {
    Signal,
    Queue,
    DoNothing,
}

#[derive(ValueEnum, Debug, Copy, Clone)]
#[clap(rename_all = "verbatim")]
enum ChildSignal {
    SIGHUP,
    SIGINT,
    SIGQUIT,
    SIGTERM,
}

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Cli {
    /// Set the timeout between detected change and command execution, defaults to 100ms
    #[clap(long, short, default_value = "100")]
    debounce: u64,
    /// Clear screen before running command
    #[clap(long, short = 'L')]
    clear: bool,

    /// Ignore paths matching the pattern
    #[clap(long, short)]
    ignore: Vec<String>,

    /// Only watch paths with the given file extension
    #[clap(long, short, value_delimiter(','))]
    extensions: Vec<String>,

    /// Select the behaviour to use when receiving events while the command is running
    #[clap(long, default_value = "signal")]
    on_busy_update: OnBusyUpdate,

    /// The signal to send to the command if on-busy-update is set to signal
    #[clap(long, default_value = "SIGTERM")]
    signal: ChildSignal,

    /// Do not use the default ignore globs
    #[clap(long)]
    no_default_ignore: bool,

    #[clap(long)]
    no_global_ignore: bool,

    /// Skip auto-loading of project ignore files (.gitignore, etc)
    #[clap(long)]
    no_project_ignore: bool,

    #[clap(default_value = ".")]
    paths: Vec<String>,

    #[clap(last(true), required(true), num_args(1..))]
    command: Vec<String>,

    #[clap(long, short, global = true)]
    verbose: bool,
}

fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    let level = if cli.verbose { Level::DEBUG } else { Level::INFO };
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().without_time().with_target(false))
        .with(tracing_subscriber::filter::Targets::new()
            .with_target(env!("CARGO_BIN_NAME"), level)
        )
        .init();

    let command = cli.command;
    let signal = match cli.signal {
        ChildSignal::SIGHUP => Signal::SIGHUP,
        ChildSignal::SIGINT => Signal::SIGINT,
        ChildSignal::SIGQUIT => Signal::SIGQUIT,
        ChildSignal::SIGTERM => Signal::SIGTERM,
    };
    let debounce = cli.debounce;

    let extensions = cli.extensions;

    let mut ignore_globs = cli.ignore.iter()
        .map(|s| {
            let mut s = s.to_string();
            s += "*";
            Pattern::new(&s)
        })
        .collect::<Result<Vec<_>, PatternError>>()
        .map_err(|e| err!("Invalid ignore glob: {}", e))?;

    if !cli.no_default_ignore {
        ignore_globs.push(Pattern::new("*~")
            .map_err(|e| err!("Invalid ignore glob: {}", e))?);
        ignore_globs.push(Pattern::new("**/.DS_Store")
            .map_err(|e| err!("Invalid ignore glob: {}", e))?);
        ignore_globs.push(Pattern::new(".git/*")
            .map_err(|e| err!("Invalid ignore glob: {}", e))?);
    }

    let strategy = match cli.on_busy_update {
        OnBusyUpdate::Signal => BusyAction::Restart,
        OnBusyUpdate::Queue => BusyAction::Queue,
        OnBusyUpdate::DoNothing => BusyAction::DoNothing,
    };

    let gitignore = if cli.no_project_ignore {
        None
    } else {
        find_project_gitignore()
    };

    let global_gitignore = if cli.no_global_ignore {
        None
    } else {
        let (g, _) = Gitignore::global();
        Some(g)
    };

    let (sender, receiver) = channel();
    // Not sure why, but the built-in debouncing seems to cause us to drop tons of events that should
    // be handled. Instead, we implement our own debouncing.
    let mut watcher = watcher(sender, Duration::from_millis(0)).unwrap();
    let mut watched_files: Vec<PathBuf> = Vec::new();
    for s in cli.paths.iter() {
        let p = std::fs::canonicalize(&Path::new(s)).unwrap();
        if p.is_dir() {
            debug!("{}: Watching directory", p.display());
            watcher.watch(p, RecursiveMode::Recursive).unwrap();
        } else {
            debug!("{}: Watching file", p.display());
            watcher.watch(&p, RecursiveMode::NonRecursive).unwrap();
            watched_files.push(p);
        }
    }

    let current_dir = std::env::current_dir().unwrap();

    let filter = Filter {
        working_dir: current_dir,
        watched_files,
        extensions,
        gitignore,
        global_gitignore,
        ignore_globs,
    };

    let mut status = Status::RestartProcess;
    let mut child: Option<GroupChild> = None;

    let terminate_signal = Arc::new(AtomicBool::new(false));
    let child_signal = Arc::new(AtomicBool::new(false));

    signal_hook::flag::register(signal_hook::consts::SIGINT, terminate_signal.clone())
        .unwrap();
    signal_hook::flag::register(signal_hook::consts::SIGCHLD, child_signal.clone())
        .unwrap();

    loop {

        // restart the process if necessary
        if status == Status::RestartProcess {
            status = Status::Waiting;

            match strategy {
                BusyAction::Restart => {
                    if let Some(mut child) = child.take() {
                        debug!("Waiting for process to exit...");
                        child.signal(signal)
                            .unwrap_or_else(|e| debug!("Failed to signal children: {}", e));
                        child.wait().unwrap();
                        debug!("Exited");
                    }
                }
                BusyAction::DoNothing => {
                    if let Some(child) = child.as_mut() {
                        match child.try_wait().unwrap() {
                            None => {
                                continue;
                            }
                            Some(_) => {}
                        }
                    }
                }
                BusyAction::Queue => {
                    if let Some(child) = child.as_mut() {
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
            if cli.clear {
                clearscreen::clear().expect("failed to clear screen");
            }
            child = Some(Command::new(&command[0])
                .args(&command[1..])
                .group_spawn()
                .map_err(|_| err!("{}: command not found", command[0]))?);
        }

        // check if we've been asked to terminate
        if terminate_signal.load(Ordering::Relaxed) {
            child.take().map(|mut c| c.signal(Signal::SIGINT));
            std::process::exit(1);
        }

        // check if the child terminated via signal
        // this is a hack to get around the fact that vite
        // swallows SIGTERM and SIGINT
        if let Some(c) = &mut child {
            if let Ok(Some(_)) = c.try_wait() {
                if child_signal.load(Ordering::Relaxed) {
                    std::process::exit(130);
                }
            }
        }

        // check if we should trigger a restart based on a file change
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
                debug!("{}: File modified. Queuing restart.", w.display());
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
    }
}
