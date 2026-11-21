//! `seal build` and the shared plan runner.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_build::{BuildOptions, Builder, JobRequest, ProgressEvent};
use seal_manifest::TargetKind;
use seal_plan::{BuildMode, CompilePlan};
use seal_project::{LockNative, Lockfile, PlanOptions, Project};
use seal_sandbox::Sandbox;
use seal_target::host_triple;

use crate::cli::{BuildArgs, Cli, MessageFormat, Selection};
use crate::ui;

/// One package target selected by a command.
#[derive(Clone, Debug)]
pub(crate) struct Job {
    /// The package index.
    pub(crate) package: usize,
    /// The target name.
    pub(crate) target: String,
}

pub(crate) fn build(cli: &Cli, root: &std::path::Path, args: &BuildArgs) -> ExitCode {
    let project = match crate::cli::load(root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    report_warnings(cli, &project);
    let jobs = match select_jobs(&project, &args.selection, Mode::Build) {
        Ok(jobs) => jobs,
        Err(code) => return code,
    };
    let target = args
        .selection
        .target_triple
        .clone()
        .unwrap_or_else(|| host_triple().as_str().to_owned());
    let toolchain = match toolchain_version(&project, &target) {
        Ok(toolchain) => toolchain,
        Err(code) => return code,
    };
    if let Err(code) = prepare_lockfile(cli, &project, &target, &args.selection) {
        return code;
    }
    let jobs_parallel = match args.jobs {
        Some(0) => {
            return ui::error("E8045", "`--jobs` must be at least one");
        }
        Some(jobs) => jobs,
        None => std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1),
    };
    let progress = build_progress(cli, args.timings);
    let options = BuildOptions {
        jobs: jobs_parallel,
        use_cache: !args.no_cache,
        sandbox: Sandbox::detect(),
        cache_root: None,
        link_args: args.link_args.iter().map(OsString::from).collect(),
        progress,
    };
    let requests: Vec<JobRequest> = jobs
        .iter()
        .map(|job| {
            let target = project
                .package(job.package)
                .target(&job.target)
                .expect("selection returns existing targets");
            let mut options = plan_options(&args.selection, BuildMode::Build, target);
            options.toolchain = toolchain.clone();
            JobRequest {
                package: job.package,
                target: job.target.clone(),
                options,
            }
        })
        .collect();
    ui::verbose(
        cli,
        &format!(
            "building {} target(s) with {} job(s){}",
            requests.len(),
            jobs_parallel,
            if args.no_cache {
                ", cache disabled"
            } else {
                ""
            }
        ),
    );
    let builder = Builder::new(&project, options);
    let report = match builder.build(&requests) {
        Ok(report) => report,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    for output in &report.outputs {
        ui::status(cli, &format!("{} {}", output.package, output.target));
        ui::detail(cli, &output.path.display().to_string());
        ui::verbose(
            cli,
            &format!(
                "  action {} ({})",
                output.key,
                if output.cached {
                    "cache hit"
                } else {
                    "rebuilt"
                }
            ),
        );
    }
    if args.timings {
        for timing in &report.timings {
            println!(
                "{} {:<16} {:>6} ms{}{}",
                timing.key,
                timing.kind,
                timing.millis,
                if timing.cached { "  cached" } else { "" },
                if timing.label.is_empty() {
                    String::new()
                } else {
                    format!("  {}", timing.label)
                }
            );
        }
    }
    ui::verbose(
        cli,
        &format!(
            "{} cache hit(s), {} action(s) run in {} ms",
            report.hits(),
            report.misses(),
            report.duration.as_millis()
        ),
    );
    if let Err(code) =
        update_native_lockfile(cli, &project, &target, &args.selection, &report.native)
    {
        return code;
    }
    ExitCode::SUCCESS
}

fn build_progress(cli: &Cli, timings: bool) -> Option<Arc<seal_build::ProgressFn>> {
    if cli.quiet || cli.message_format != MessageFormat::Human {
        return None;
    }
    let verbose = cli.verbose;
    Some(Arc::new(move |event: ProgressEvent| match event {
        ProgressEvent::Started {
            kind,
            label,
            command,
            ..
        } => {
            println!("Running {kind} {label}");
            if verbose && !command.is_empty() {
                eprintln!("  command: {}", command.join(" "));
            }
        }
        ProgressEvent::Cached {
            kind,
            label,
            key,
            command,
        } => {
            println!("Cached {kind} {label} (inputs unchanged, {})", &key[..12]);
            if verbose && !command.is_empty() {
                eprintln!("  command: {}", command.join(" "));
            }
        }
        ProgressEvent::Finished {
            kind,
            label,
            millis,
            ..
        } => {
            if timings {
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

fn update_native_lockfile(
    cli: &Cli,
    project: &Project,
    target: &str,
    selection: &Selection,
    resolved: &[seal_build::NativeResult],
) -> Result<(), ExitCode> {
    if resolved.is_empty() {
        return Ok(());
    }
    let path = project.root.join("seal.lock");
    let existing = Lockfile::load(&path).unwrap_or_else(|_| {
        project.resolve_lockfile_with(target, &lock_options(project, target, selection))
    });
    let mut native = existing.native.clone();
    for result in resolved {
        native.retain(|entry| !(entry.package == result.package && entry.alias == result.alias));
        native.push(LockNative {
            package: result.package.clone(),
            alias: result.alias.clone(),
            pkg_config: result.pkg_config.clone(),
            static_link: result.static_link,
            libs: result.libs.clone(),
            search_paths: result
                .search_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            link_args: result.link_args.clone(),
        });
    }
    let updated = existing.clone().with_native(native);
    if (selection.locked || selection.frozen) && updated.native != existing.native {
        return Err(ui::error(
            "E8008",
            "the resolved native dependencies differ from the lockfile; run `seal build` without `--locked`",
        ));
    }
    updated
        .write_if_changed(&path)
        .map(|_| ())
        .map_err(|error| ui::error(error.code(), error.message()))?;
    ui::verbose(
        cli,
        &format!(
            "recorded {} native dependenc(ies) in seal.lock",
            resolved.len()
        ),
    );
    Ok(())
}

fn report_warnings(cli: &Cli, project: &Project) {
    if cli.quiet {
        return;
    }
    for warning in &project.warnings {
        let location = warning
            .location
            .map(|location| format!("{}:{}: ", location.line, location.column))
            .unwrap_or_default();
        ui::warning(&format!("{location}{}", warning.message));
        if let Some(suggestion) = &warning.suggestion {
            eprintln!("  help: did you mean `{suggestion}`?");
        }
    }
}

/// Whether selection includes every target or only build defaults.
#[derive(Clone, Copy)]
pub(crate) enum Mode {
    /// Library and binary targets.
    Build,
    /// Analyze library and binary targets.
    Check,
    /// Only executable targets.
    Run,
}

pub(crate) fn select_jobs(
    project: &Project,
    selection: &Selection,
    mode: Mode,
) -> Result<Vec<Job>, ExitCode> {
    let packages = select_packages(project, selection)?;
    let mut jobs = Vec::new();
    if !selection.targets.is_empty() {
        for name in &selection.targets {
            let mut found = false;
            for package in &packages {
                if project.package(*package).target(name).is_some() {
                    jobs.push(Job {
                        package: *package,
                        target: name.clone(),
                    });
                    found = true;
                }
            }
            if !found {
                return Err(ui::error(
                    "E8003",
                    &format!("no selected package has a target named `{name}`"),
                ));
            }
        }
        return Ok(jobs);
    }
    let triple = selection
        .target_triple
        .clone()
        .unwrap_or_else(|| host_triple().as_str().to_owned());
    for package in packages {
        for target in &project.package(package).targets {
            if !target.is_active_for(&triple) {
                continue;
            }
            let include = if selection.all_targets {
                true
            } else {
                match mode {
                    Mode::Build | Mode::Check => matches!(
                        target.kind,
                        TargetKind::Lib
                            | TargetKind::Bin
                            | TargetKind::Staticlib
                            | TargetKind::Cdylib
                    ),
                    Mode::Run => target.is_executable(),
                }
            };
            if include {
                jobs.push(Job {
                    package,
                    target: target.name.clone(),
                });
            }
        }
    }
    if jobs.is_empty() {
        return Err(ui::error(
            "E8003",
            "the selected packages have no targets for this command",
        ));
    }
    Ok(jobs)
}

fn select_packages(project: &Project, selection: &Selection) -> Result<Vec<usize>, ExitCode> {
    if selection.workspace {
        return Ok((0..project.packages.len()).collect());
    }
    if let Some(name) = &selection.package {
        let Some((index, _)) = project.package_by_name(name) else {
            return Err(ui::error(
                "E8005",
                &format!("the project has no package named `{name}`"),
            ));
        };
        return Ok(vec![index]);
    }
    project
        .default_package()
        .map(|index| vec![index])
        .map_err(|error| ui::error(error.code(), error.message()))
}

pub(crate) fn plan_options(
    selection: &Selection,
    mode: BuildMode,
    target: &seal_project::Target,
) -> PlanOptions {
    let (profile, profile_explicit) = if let Some(profile) = &selection.profile {
        (profile.clone(), true)
    } else if selection.release {
        ("release".to_owned(), true)
    } else {
        (
            target.profile.clone().unwrap_or_else(|| "dev".to_owned()),
            false,
        )
    };
    PlanOptions {
        target: selection
            .target_triple
            .clone()
            .unwrap_or_else(|| host_triple().as_str().to_owned()),
        profile,
        profile_explicit,
        mode,
        output: None,
        features: selection.features.clone(),
        no_default_features: selection.no_default_features,
        all_features: selection.all_features,
        cfgs: selection.cfgs.clone(),
        extra_files: Vec::new(),
        reproducible: true,
        source_date_epoch: None,
        toolchain: seal_toolchain::EMBEDDED_VERSION.to_owned(),
    }
}

/// Returns the pinned toolchain version after checking it can build `target`.
pub(crate) fn toolchain_version(project: &Project, target: &str) -> Result<String, ExitCode> {
    let pin = seal_toolchain::read_pin(&project.root)
        .map_err(|error| ui::error(error.code(), &error.to_string()))?;
    match pin {
        Some(_) => {
            let store = seal_toolchain::ToolchainStore::for_project(&project.root);
            let installed = store
                .which(&project.root)
                .map_err(|error| ui::error(error.code(), &error.to_string()))?
                .ok_or_else(|| {
                    ui::error(
                        "E8061",
                        "`seal-toolchain.toml` pins a toolchain that is not installed",
                    )
                })?;
            seal_toolchain::ensure_compatible(
                &installed.manifest,
                seal_toolchain::EMBEDDED_VERSION,
                target,
            )
            .map_err(|error| ui::error(error.code(), &error.to_string()))?;
            Ok(installed.manifest.version)
        }
        None => Ok(seal_toolchain::EMBEDDED_VERSION.to_owned()),
    }
}

pub(crate) fn prepare_lockfile(
    _cli: &Cli,
    project: &Project,
    target: &str,
    selection: &Selection,
) -> Result<(), ExitCode> {
    let mut expected =
        project.resolve_lockfile_with(target, &lock_options(project, target, selection));
    let path = project.root.join("seal.lock");
    if let Ok(existing) = Lockfile::load(&path) {
        let declared = declared_native(project);
        let preserved: Vec<LockNative> = existing
            .native
            .into_iter()
            .filter(|native| declared.contains(&(native.package.clone(), native.alias.clone())))
            .collect();
        expected = expected.with_native(preserved);
    }
    if selection.locked || selection.frozen {
        let existing = Lockfile::load(&path).map_err(|error| {
            ui::error(
                "E8008",
                &format!("{}; run `seal build` without `--locked`", error.message()),
            )
        })?;
        if existing != expected {
            return Err(ui::error(
                "E8008",
                "the lockfile is out of date; run `seal build` without `--locked`",
            ));
        }
        return Ok(());
    }
    expected
        .write_if_changed(&path)
        .map(|_| ())
        .map_err(|error| ui::error(error.code(), error.message()))
}

fn lock_options(project: &Project, target: &str, selection: &Selection) -> PlanOptions {
    let profile = if let Some(profile) = &selection.profile {
        profile.clone()
    } else if selection.release {
        "release".to_owned()
    } else {
        project
            .default_package()
            .ok()
            .and_then(|package| project.package(package).targets.first())
            .and_then(|target| target.profile.clone())
            .unwrap_or_else(|| "dev".to_owned())
    };
    PlanOptions {
        target: target.to_owned(),
        profile,
        profile_explicit: selection.profile.is_some() || selection.release,
        mode: BuildMode::Check,
        output: None,
        features: selection.features.clone(),
        no_default_features: selection.no_default_features,
        all_features: selection.all_features,
        cfgs: selection.cfgs.clone(),
        extra_files: Vec::new(),
        reproducible: true,
        source_date_epoch: None,
        toolchain: seal_toolchain::EMBEDDED_VERSION.to_owned(),
    }
}

fn declared_native(project: &Project) -> BTreeSet<(String, String)> {
    project
        .packages
        .iter()
        .flat_map(|package| {
            package
                .manifest
                .native
                .keys()
                .map(move |alias| (package.name.clone(), alias.clone()))
        })
        .collect()
}

static NEXT_PLAN: AtomicU64 = AtomicU64::new(0);

/// Invokes the compiler for one plan and removes the temporary plan file.
pub(crate) fn run_plan(
    cli: &Cli,
    plan: &CompilePlan,
    mode: Mode,
    emit: &str,
    link_args: &[String],
) -> Result<PathBuf, ExitCode> {
    let sequence = NEXT_PLAN.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("seal-plan-{}-{sequence}.json", std::process::id()));
    plan.write(&path)
        .map_err(|error| ui::error(error.code(), error.message()))?;
    let mut arguments: Vec<String> = vec!["sealc".to_owned()];
    arguments.push(match mode {
        Mode::Build | Mode::Run => "build".to_owned(),
        Mode::Check => "check".to_owned(),
    });
    arguments.push("--plan-file".to_owned());
    arguments.push(path.display().to_string());
    if matches!(mode, Mode::Build | Mode::Run) {
        arguments.push(format!("--emit={emit}"));
    }
    if cli.message_format == MessageFormat::Json {
        arguments.push("--error-format=json".to_owned());
    }
    arguments.push(format!("--color={}", color_name(cli.color)));
    for argument in link_args {
        arguments.push("--link-arg".to_owned());
        arguments.push(argument.clone());
    }
    ui::verbose(cli, &format!("running: {}", arguments.join(" ")));
    let status = seal_driver::run_from(arguments);
    let _ = std::fs::remove_file(&path);
    if status != ExitCode::SUCCESS {
        return Err(status);
    }
    if matches!(mode, Mode::Check) {
        return Ok(plan.output.clone().unwrap_or_default());
    }
    plan.output
        .clone()
        .ok_or_else(|| ui::error("E8010", "the compiler succeeded without an artifact path"))
}

fn color_name(color: crate::cli::ColorChoice) -> &'static str {
    match color {
        crate::cli::ColorChoice::Auto => "auto",
        crate::cli::ColorChoice::Always => "always",
        crate::cli::ColorChoice::Never => "never",
    }
}
