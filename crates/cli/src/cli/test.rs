//! `seal test`: build and run the isolated test harnesses.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use seal_test::{Progress, Report, Status};

use crate::cli::harness::{self, Harness};
use crate::cli::{Cli, HarnessFormat, TestArgs};
use crate::ui;

pub(crate) fn test(cli: &Cli, root: &std::path::Path, args: &TestArgs) -> ExitCode {
    let project = match crate::cli::load_for_selection(cli, root, &args.selection) {
        Ok(project) => project,
        Err(code) => return code,
    };
    for warning in &project.warnings {
        if !cli.quiet {
            ui::warning(&warning.message);
        }
    }
    let jobs = match harness::select(&project, &args.selection, Harness::Test) {
        Ok(jobs) => jobs,
        Err(code) => return code,
    };
    let human = args.format == HarnessFormat::Human
        && cli.message_format == crate::cli::MessageFormat::Human;
    let executables = match harness::build(
        cli,
        &project,
        &jobs,
        &args.selection,
        Harness::Test,
        args.no_cache,
        human,
    ) {
        Ok(executables) => executables,
        Err(code) => return code,
    };
    let tests = match seal_test::discover(&executables) {
        Ok(tests) => tests,
        Err(error) => return harness::test_error(error),
    };
    let partition = match args.partition.as_deref().map(harness::parse_partition) {
        None => None,
        Some(Ok(partition)) => Some(partition),
        Some(Err(code)) => return code,
    };
    if args.stress == 0 {
        return ui::error("E8102", "`--stress` must be at least one");
    }
    let selection = seal_test::Selection {
        filters: &args.filters,
        exact: args.exact,
        skips: &args.skips,
        partition,
        stress: args.stress,
    };
    let scheduled = selection.apply(&tests);
    if args.list {
        for entry in &scheduled {
            println!("{}", entry.label);
        }
        return ExitCode::SUCCESS;
    }
    if scheduled.is_empty() {
        if human {
            println!("no tests to run");
            return ExitCode::SUCCESS;
        }
        return match emit(args, &empty_report(), &project.name, false) {
            Ok(()) => ExitCode::SUCCESS,
            Err(code) => code,
        };
    }
    let progress: Option<Arc<Progress>> = human.then(|| {
        let color = cli.color;
        Arc::new(move |outcome: &seal_test::Outcome| {
            let style = match outcome.status {
                Status::Passed => ui::green(color, "PASS"),
                Status::Failed => ui::red(color, "FAIL"),
                Status::Timeout => ui::yellow(color, "TIME"),
                Status::Skipped => ui::yellow(color, "SKIP"),
            };
            println!(
                "{style} {} ({} ms)",
                outcome.name,
                outcome.duration.as_millis()
            );
            if outcome.is_failure() && !outcome.message.is_empty() {
                for line in outcome.message.lines() {
                    println!("     {line}");
                }
            }
        }) as Arc<Progress>
    });
    let options = seal_test::Options {
        jobs: args.jobs.unwrap_or_else(default_jobs),
        fail_fast: !args.no_fail_fast,
        retries: args.retries,
        timeout: (args.timeout > 0).then(|| Duration::from_secs(args.timeout)),
        iterations: 1,
        directory: Some(project.root.clone()),
        progress,
    };
    if options.jobs == 0 {
        return ui::error("E8102", "`--jobs` must be at least one");
    }
    let report = match seal_test::run(&scheduled, &options) {
        Ok(report) => report,
        Err(error) => return harness::test_error(error),
    };
    if let Err(code) = emit(args, &report, &project.name, human) {
        return code;
    }
    if report.has_failures() {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn emit(args: &TestArgs, report: &Report, suite: &str, human: bool) -> Result<(), ExitCode> {
    match args.format {
        HarnessFormat::Human => {
            if human {
                print_summary(report);
            }
            Ok(())
        }
        HarnessFormat::Json => emit_json(args, report),
        HarnessFormat::Junit => {
            let path = args
                .output
                .clone()
                .unwrap_or_else(|| std::path::PathBuf::from("seal/target/test-junit.xml"));
            seal_test::write_junit(report, suite, &path)
                .map_err(harness::test_error)
                .inspect(|()| {
                    if human {
                        println!("wrote {}", path.display());
                    }
                })
        }
    }
}

fn emit_json(args: &TestArgs, report: &Report) -> Result<(), ExitCode> {
    let text = seal_test::json(report).map_err(harness::test_error)?;
    write_output(args.output.as_deref(), &text)
}

fn write_output(path: Option<&std::path::Path>, text: &str) -> Result<(), ExitCode> {
    match path {
        Some(path) => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent).map_err(|error| {
                    ui::error(
                        "E8108",
                        &format!("cannot create `{}`: {error}", parent.display()),
                    )
                })?;
            }
            std::fs::write(path, text).map_err(|error| {
                ui::error(
                    "E8108",
                    &format!("cannot write `{}`: {error}", path.display()),
                )
            })
        }
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

fn print_summary(report: &Report) {
    let status = if report.has_failures() {
        "FAILED"
    } else {
        "ok"
    };
    println!(
        "test result: {status}. {} passed; {} failed; {} skipped; finished in {:.3}s",
        report.passed(),
        report.failed(),
        report.skipped(),
        report.duration.as_secs_f64()
    );
}

fn empty_report() -> Report {
    Report {
        outcomes: Vec::new(),
        duration: Duration::ZERO,
    }
}

fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
}
