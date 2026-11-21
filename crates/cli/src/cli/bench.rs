//! `seal bench`: build benchmarks and measure wall time per iteration.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use seal_test::{Scheduled, Status};

use crate::cli::harness::{self, Harness};
use crate::cli::{BenchArgs, Cli};
use crate::ui;

/// One benchmark's measurements across samples.
struct Measurement {
    name: String,
    samples: Vec<Duration>,
}

impl Measurement {
    fn fastest(&self) -> Duration {
        self.samples.iter().copied().min().unwrap_or_default()
    }

    fn mean(&self) -> Duration {
        if self.samples.is_empty() {
            return Duration::ZERO;
        }
        let total: u128 = self.samples.iter().map(Duration::as_nanos).sum();
        Duration::from_nanos((total / self.samples.len() as u128) as u64)
    }

    fn nanoseconds(&self, duration: Duration, iterations: u32) -> f64 {
        duration.as_nanos() as f64 / f64::from(iterations.max(1))
    }
}

pub(crate) fn bench(cli: &Cli, root: &Path, args: &BenchArgs) -> ExitCode {
    if args.iterations == 0 {
        return ui::error("E8102", "`--iterations` must be at least one");
    }
    if args.samples == 0 {
        return ui::error("E8102", "`--samples` must be at least one");
    }
    if !(args.threshold.is_finite() && args.threshold >= 0.0) {
        return ui::error("E8102", "`--threshold` must be a non-negative number");
    }
    let project = match crate::cli::load_for_selection(cli, root, &args.selection) {
        Ok(project) => project,
        Err(code) => return code,
    };
    for warning in &project.warnings {
        if !cli.quiet {
            ui::warning(&warning.message);
        }
    }
    let jobs = match harness::select(&project, &args.selection, Harness::Bench) {
        Ok(jobs) => jobs,
        Err(code) => return code,
    };
    let human = !args.json && cli.message_format == crate::cli::MessageFormat::Human;
    let executables = match harness::build(
        cli,
        &project,
        &jobs,
        &args.selection,
        Harness::Bench,
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
    let selection = seal_test::Selection {
        filters: &args.filters,
        exact: args.exact,
        skips: &args.skips,
        partition,
        stress: 1,
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
            println!("no benchmarks to run");
        }
        return ExitCode::SUCCESS;
    }
    let measurements = match measure(&scheduled, args) {
        Ok(measurements) => measurements,
        Err(code) => return code,
    };
    report(args, &measurements, human)
}

fn measure(scheduled: &[Scheduled], args: &BenchArgs) -> Result<Vec<Measurement>, ExitCode> {
    let jobs = args.jobs.unwrap_or(1);
    if jobs == 0 {
        return Err(ui::error("E8102", "`--jobs` must be at least one"));
    }
    let options = seal_test::Options {
        jobs,
        fail_fast: true,
        retries: 0,
        timeout: (args.timeout > 0).then(|| Duration::from_secs(args.timeout)),
        iterations: args.iterations,
        directory: None,
        progress: None,
    };
    let mut samples: Vec<Vec<Duration>> = vec![Vec::new(); scheduled.len()];
    for _ in 0..args.samples {
        let report = seal_test::run(scheduled, &options).map_err(harness::test_error)?;
        if report.has_failures() {
            for outcome in &report.outcomes {
                if outcome.is_failure() {
                    eprintln!(
                        "error: benchmark `{}` {}",
                        outcome.name,
                        outcome.status.as_str()
                    );
                    if !outcome.message.is_empty() {
                        eprintln!("{}", outcome.message);
                    }
                }
            }
            return Err(ExitCode::from(1));
        }
        if report.outcomes.len() != scheduled.len() {
            return Err(ui::error(
                "E8102",
                "the benchmark runner returned an incomplete result",
            ));
        }
        for (index, outcome) in report.outcomes.iter().enumerate() {
            if outcome.status == Status::Passed {
                samples[index].push(outcome.duration);
            }
        }
    }
    Ok(scheduled
        .iter()
        .zip(samples)
        .map(|(entry, samples)| Measurement {
            name: entry.label.clone(),
            samples,
        })
        .collect())
}

fn report(args: &BenchArgs, measurements: &[Measurement], human: bool) -> ExitCode {
    let baseline = match &args.baseline {
        Some(path) => match read_baseline(path) {
            Ok(baseline) => Some(baseline),
            Err(code) => return code,
        },
        None => None,
    };
    if let Some(path) = &args.save
        && let Err(code) = write_baseline(path, measurements, args.iterations)
    {
        return code;
    }
    let mut regressions = Vec::new();
    if let Some(baseline) = &baseline {
        for measurement in measurements {
            let Some(previous) = baseline.get(&measurement.name).copied() else {
                continue;
            };
            let current = measurement.nanoseconds(measurement.fastest(), args.iterations);
            let allowed = previous * (1.0 + args.threshold / 100.0);
            if current > allowed {
                regressions.push((measurement.name.clone(), previous, current));
            }
        }
    }
    if args.json {
        let value = serde_json::json!({
            "iterations": args.iterations,
            "samples": args.samples,
            "benchmarks": measurements.iter().map(|measurement| {
                serde_json::json!({
                    "name": measurement.name,
                    "ns_per_iter": measurement.nanoseconds(measurement.fastest(), args.iterations),
                    "min_ns_per_iter": measurement.nanoseconds(measurement.fastest(), args.iterations),
                    "mean_ns_per_iter": measurement.nanoseconds(measurement.mean(), args.iterations),
                })
            }).collect::<Vec<_>>(),
            "regressions": regressions.iter().map(|(name, previous, current)| {
                serde_json::json!({
                    "name": name,
                    "baseline_ns_per_iter": previous,
                    "current_ns_per_iter": current,
                })
            }).collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&value) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error(
                    "E8108",
                    &format!("cannot serialize the benchmark report: {error}"),
                );
            }
        }
    } else if human {
        println!(
            "{:<44} {:>14} {:>14}",
            "benchmark", "fastest ns/iter", "mean ns/iter"
        );
        for measurement in measurements {
            println!(
                "{:<44} {:>14.1} {:>14.1}",
                measurement.name,
                measurement.nanoseconds(measurement.fastest(), args.iterations),
                measurement.nanoseconds(measurement.mean(), args.iterations)
            );
        }
    }
    for (name, previous, current) in &regressions {
        eprintln!(
            "error: benchmark `{name}` regressed from {previous:.1} to {current:.1} ns/iter (threshold {}%)",
            args.threshold
        );
    }
    if regressions.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn read_baseline(path: &Path) -> Result<BTreeMap<String, f64>, ExitCode> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        ui::error(
            "E8108",
            &format!("cannot read the baseline `{}`: {error}", path.display()),
        )
    })?;
    let value: serde_json::Value = serde_json::from_str(&text).map_err(|error| {
        ui::error(
            "E8108",
            &format!(
                "the baseline `{}` is not valid JSON: {error}",
                path.display()
            ),
        )
    })?;
    let benchmarks = value
        .get("benchmarks")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            ui::error(
                "E8108",
                &format!(
                    "the baseline `{}` has no `benchmarks` object",
                    path.display()
                ),
            )
        })?;
    let mut baseline = BTreeMap::new();
    for (name, value) in benchmarks {
        let Some(number) = value.as_f64() else {
            return Err(ui::error(
                "E8108",
                &format!("the baseline entry `{name}` is not a number"),
            ));
        };
        if !number.is_finite() || number < 0.0 {
            return Err(ui::error(
                "E8108",
                &format!("the baseline entry `{name}` is not a non-negative number"),
            ));
        }
        baseline.insert(name.clone(), number);
    }
    Ok(baseline)
}

fn write_baseline(
    path: &Path,
    measurements: &[Measurement],
    iterations: u32,
) -> Result<(), ExitCode> {
    let benchmarks: BTreeMap<&str, f64> = measurements
        .iter()
        .map(|measurement| {
            (
                measurement.name.as_str(),
                measurement.nanoseconds(measurement.fastest(), iterations),
            )
        })
        .collect();
    let value = serde_json::json!({
        "version": 1,
        "iterations": iterations,
        "benchmarks": benchmarks,
    });
    let text = serde_json::to_string_pretty(&value)
        .map_err(|error| ui::error("E8108", &format!("cannot serialize the baseline: {error}")))?;
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
    std::fs::write(path, format!("{text}\n")).map_err(|error| {
        ui::error(
            "E8108",
            &format!("cannot write the baseline `{}`: {error}", path.display()),
        )
    })
}
