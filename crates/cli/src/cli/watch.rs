//! `seal watch`: poll source files and repeat one build-like command.
//!
//! Polling keeps the watcher dependency-free and portable; the project tree is
//! small enough that a metadata pass is cheaper than a filesystem notification
//! backend that behaves differently on every platform. A change is reported only
//! after the tree has been quiet for the debounce interval, so editors that save
//! several files in sequence trigger one rebuild.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::cli::build;
use crate::cli::{BuildArgs, CheckArgs, Cli, TestArgs, WatchAction, WatchArgs};
use crate::ui;

/// How often the tree is polled while waiting for a change.
const POLL: Duration = Duration::from_millis(100);

/// One observed file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Stamp {
    modified: Option<SystemTime>,
    length: u64,
}

type Snapshot = BTreeMap<PathBuf, Stamp>;

pub(crate) fn watch(cli: &Cli, root: &Path, args: &WatchArgs) -> ExitCode {
    let root = match root.canonicalize() {
        Ok(root) => root,
        Err(error) => {
            return ui::error(
                "E8109",
                &format!("cannot watch `{}`: {error}", root.display()),
            );
        }
    };
    let mut last = run_action(cli, &root, args);
    if args.once {
        return last;
    }
    // Take the baseline after the first run: a build can legitimately rewrite
    // `seal.lock`, and that write must not trigger a second run.
    let mut snapshot = match take(&root) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return ui::error("E8109", &format!("cannot scan the project: {error}"));
        }
    };
    let interrupted = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&interrupted);
    let _ = ctrlc::set_handler(move || flag.store(true, Ordering::SeqCst));
    let debounce = Duration::from_millis(args.debounce.max(1));
    loop {
        std::thread::sleep(POLL);
        if interrupted.load(Ordering::SeqCst) {
            return stop(cli);
        }
        let next = match take(&root) {
            Ok(next) => next,
            Err(error) => {
                ui::warning(&format!("cannot scan the project: {error}"));
                continue;
            }
        };
        if next == snapshot {
            continue;
        }
        // Editors often write a temporary file and rename it, so wait for the
        // tree to settle before rebuilding.
        let mut settled = next;
        let mut quiet_since = Instant::now();
        loop {
            std::thread::sleep(POLL);
            if interrupted.load(Ordering::SeqCst) {
                return stop(cli);
            }
            match take(&root) {
                Ok(current) if current == settled => {
                    if quiet_since.elapsed() >= debounce {
                        break;
                    }
                }
                Ok(current) => {
                    settled = current;
                    quiet_since = Instant::now();
                }
                Err(error) => {
                    ui::warning(&format!("cannot scan the project: {error}"));
                    quiet_since = Instant::now();
                }
            }
        }
        snapshot = settled;
        clear(cli);
        ui::status(
            cli,
            &format!(
                "[watch] change detected; running {}",
                action_name(args.action)
            ),
        );
        let started = Instant::now();
        last = run_action(cli, &root, args);
        let result = if last == ExitCode::SUCCESS {
            ui::green(cli.color, "ok")
        } else {
            ui::red(cli.color, "failed")
        };
        ui::status(
            cli,
            &format!(
                "[watch] {}: {result} in {:.2}s; waiting for changes",
                action_name(args.action),
                started.elapsed().as_secs_f64()
            ),
        );
    }
}

/// Clears the terminal and returns the conventional interrupt status.
fn stop(cli: &Cli) -> ExitCode {
    clear(cli);
    ui::status(cli, "[watch] interrupted");
    ExitCode::from(130)
}

fn run_action(cli: &Cli, root: &Path, args: &WatchArgs) -> ExitCode {
    match args.action {
        WatchAction::Build => {
            let build = BuildArgs {
                selection: args.selection.clone(),
                link_args: Vec::new(),
                jobs: None,
                timings: false,
                no_cache: false,
            };
            build::build(cli, root, &build)
        }
        WatchAction::Check => {
            let check = CheckArgs {
                selection: args.selection.clone(),
            };
            crate::cli::check::check(cli, root, &check)
        }
        WatchAction::Test => {
            let test = TestArgs {
                selection: args.selection.clone(),
                filters: args.filters.clone(),
                exact: false,
                skips: Vec::new(),
                partition: None,
                stress: 1,
                retries: 0,
                timeout: 60,
                no_fail_fast: true,
                jobs: None,
                list: false,
                format: crate::cli::HarnessFormat::Human,
                output: None,
                no_cache: false,
            };
            crate::cli::test::test(cli, root, &test)
        }
    }
}

fn action_name(action: WatchAction) -> &'static str {
    match action {
        WatchAction::Build => "build",
        WatchAction::Check => "check",
        WatchAction::Test => "test",
    }
}

fn take(root: &Path) -> std::io::Result<Snapshot> {
    let mut snapshot = Snapshot::new();
    visit(root, &mut snapshot)?;
    Ok(snapshot)
}

fn visit(directory: &Path, snapshot: &mut Snapshot) -> std::io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(directory)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if entry.file_type()?.is_dir() {
            if matches!(
                name.to_string_lossy().as_ref(),
                ".git" | "seal" | "target" | "vendor" | "node_modules"
            ) {
                continue;
            }
            visit(&path, snapshot)?;
            continue;
        }
        if !interesting(&path) {
            continue;
        }
        let metadata = entry.metadata()?;
        snapshot.insert(
            path,
            Stamp {
                modified: metadata.modified().ok(),
                length: metadata.len(),
            },
        );
    }
    Ok(())
}

fn interesting(path: &Path) -> bool {
    matches!(
        path.extension().and_then(std::ffi::OsStr::to_str),
        Some("sl")
    ) || matches!(
        path.file_name().and_then(std::ffi::OsStr::to_str),
        Some("seal.toml" | "seal.lock")
    )
}

/// Clears the terminal only when stdout is a terminal and output is human.
fn clear(cli: &Cli) {
    if cli.quiet || cli.message_format != crate::cli::MessageFormat::Human {
        return;
    }
    if std::io::stdout().is_terminal() {
        print!("\x1b[2J\x1b[H");
        let _ = std::io::stdout().flush();
    }
}
