//! Shared build and selection helpers for `seal test` and `seal bench`.

use std::collections::BTreeMap;
use std::process::ExitCode;
use std::sync::Arc;

use seal_build::{BuildOptions, Builder, JobRequest, ProgressEvent};
use seal_manifest::TargetKind;
use seal_plan::BuildMode;
use seal_project::{Project, Target};
use seal_sandbox::Sandbox;
use seal_test::{Executable, Kind, Partition, TestError};

use crate::cli::build::{Job, Mode, plan_options, prepare_lockfile, toolchain_version};
use crate::cli::{Cli, MessageFormat, Selection};
use crate::ui;

/// The target kind a harness is built from.
#[derive(Clone, Copy)]
pub(crate) enum Harness {
    /// `test func` targets.
    Test,
    /// `bench func` targets.
    Bench,
}

impl Harness {
    const fn target_kind(self) -> TargetKind {
        match self {
            Self::Test => TargetKind::Test,
            Self::Bench => TargetKind::Bench,
        }
    }

    const fn mode(self) -> Mode {
        match self {
            Self::Test => Mode::Test,
            Self::Bench => Mode::Bench,
        }
    }

    pub(crate) const fn kind(self) -> Kind {
        match self {
            Self::Test => Kind::Test,
            Self::Bench => Kind::Bench,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Test => "test",
            Self::Bench => "bench",
        }
    }
}

/// Builds every selected harness, reusing the action cache unless `no_cache`.
pub(crate) fn build(
    cli: &Cli,
    project: &Project,
    jobs: &[Job],
    selection: &Selection,
    harness: Harness,
    no_cache: bool,
    human: bool,
) -> Result<Vec<Executable>, ExitCode> {
    for job in jobs {
        let target = target(project, job);
        if target.kind != harness.target_kind() {
            return Err(ui::error(
                "E8102",
                &format!("`{}` is not a {} target", target.name, harness.name()),
            ));
        }
    }
    let triple = selection
        .target_triple
        .clone()
        .unwrap_or_else(|| seal_target::host_triple().as_str().to_owned());
    let toolchain = toolchain_version(project, &triple)?;
    prepare_lockfile(cli, project, &triple, selection)?;
    let parallel = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1);
    let progress = build_progress(cli, human);
    let options = BuildOptions {
        jobs: parallel,
        use_cache: !no_cache,
        sandbox: Sandbox::detect(),
        cache_root: None,
        link_args: Vec::new(),
        progress,
    };
    let requests: Vec<JobRequest> = jobs
        .iter()
        .map(|job| {
            let target = target(project, job);
            let mut options = plan_options(selection, BuildMode::Build, target);
            options.toolchain = toolchain.clone();
            JobRequest {
                package: job.package,
                target: job.target.clone(),
                options,
            }
        })
        .collect();
    let report = Builder::new(project, options)
        .build(&requests)
        .map_err(|error| ui::error(error.code(), error.message()))?;
    let mut outputs: BTreeMap<(String, String), std::path::PathBuf> = BTreeMap::new();
    for output in &report.outputs {
        outputs.insert(
            (output.package.clone(), output.target.clone()),
            output.path.clone(),
        );
    }
    let mut executables = Vec::with_capacity(jobs.len());
    for job in jobs {
        let package = &project.package(job.package).name;
        let path = outputs
            .remove(&(package.clone(), job.target.clone()))
            .ok_or_else(|| {
                ui::error(
                    "E8101",
                    &format!(
                        "the build did not produce a harness for `{package}` target `{}`",
                        job.target
                    ),
                )
            })?;
        if !path.is_file() {
            return Err(ui::error(
                "E8101",
                &format!(
                    "the harness `{}` is missing after the build",
                    path.display()
                ),
            ));
        }
        executables.push(Executable {
            package: package.clone(),
            target: job.target.clone(),
            path,
            kind: harness.kind(),
        });
    }
    Ok(executables)
}

/// Selects the explicit targets for a harness mode.
pub(crate) fn select(
    project: &Project,
    selection: &Selection,
    harness: Harness,
) -> Result<Vec<Job>, ExitCode> {
    crate::cli::build::select_jobs(project, selection, harness.mode())
}

/// Parses a `--partition` value into the runner's stable partition.
pub(crate) fn parse_partition(value: &str) -> Result<Partition, ExitCode> {
    let normalized = if value.contains(':') {
        value.to_owned()
    } else {
        format!("slice:{value}")
    };
    Partition::parse(&normalized).map_err(test_error)
}

/// Converts a runner failure into the CLI diagnostic with its stable code.
pub(crate) fn test_error(error: TestError) -> ExitCode {
    ui::error(error.code(), &error.to_string())
}

fn target<'a>(project: &'a Project, job: &Job) -> &'a Target {
    project
        .package(job.package)
        .target(&job.target)
        .expect("selection returns existing targets")
}

fn build_progress(cli: &Cli, human: bool) -> Option<Arc<seal_build::ProgressFn>> {
    if !human || cli.quiet || cli.message_format != MessageFormat::Human {
        return None;
    }
    let verbose = cli.verbose;
    Some(Arc::new(move |event: ProgressEvent| match event {
        ProgressEvent::Started { kind, label, .. } => {
            println!("Running {kind} {label}");
        }
        ProgressEvent::Cached { kind, label, .. } => {
            println!("Cached {kind} {label}");
        }
        ProgressEvent::Finished {
            kind,
            label,
            millis,
            ..
        } => {
            if verbose {
                println!("Finished {kind} {label} in {millis} ms");
            }
        }
        ProgressEvent::CacheCorrupt {
            kind,
            label,
            reason,
            key,
        } => {
            eprintln!(
                "warning: discarded the corrupt {kind} cache entry for {label} ({key}): {reason}"
            );
        }
    }))
}
