//! `seal build` and the shared plan runner.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_manifest::TargetKind;
use seal_plan::{BuildMode, CompilePlan};
use seal_project::{PlanOptions, Project};
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
    if let Err(code) = prepare_lockfile(cli, &project, &target, &args.selection) {
        return code;
    }
    for job in jobs {
        let package = project.package(job.package);
        let target = package
            .target(&job.target)
            .expect("selection returns existing targets");
        let options = plan_options(&args.selection, BuildMode::Build, target);
        let plan = match project.compile_plan(job.package, &job.target, &options) {
            Ok(plan) => plan,
            Err(error) => return ui::error(error.code(), error.message()),
        };
        let emit = if target.is_executable() { "exe" } else { "obj" };
        ui::status(
            cli,
            &format!(
                "Compiling {} {} ({})",
                package.name,
                target.name,
                target.kind.as_str()
            ),
        );
        if let Err(code) = run_plan(cli, &plan, Mode::Build, emit, &args.link_args) {
            return code;
        }
        if let Some(output) = &plan.output {
            ui::detail(cli, &output.display().to_string());
        }
    }
    ExitCode::SUCCESS
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
                    Mode::Build | Mode::Check => {
                        matches!(target.kind, TargetKind::Lib | TargetKind::Bin)
                    }
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
    }
}

pub(crate) fn prepare_lockfile(
    _cli: &Cli,
    project: &Project,
    target: &str,
    selection: &Selection,
) -> Result<(), ExitCode> {
    let lock_profile = if let Some(profile) = &selection.profile {
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
    let options = PlanOptions {
        target: target.to_owned(),
        profile: lock_profile,
        profile_explicit: selection.profile.is_some() || selection.release,
        mode: BuildMode::Check,
        output: None,
        features: selection.features.clone(),
        no_default_features: selection.no_default_features,
        all_features: selection.all_features,
        cfgs: selection.cfgs.clone(),
    };
    let expected = project.resolve_lockfile_with(target, &options);
    let path = project.root.join("seal.lock");
    if selection.locked || selection.frozen {
        let existing = seal_project::Lockfile::load(&path).map_err(|error| {
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
