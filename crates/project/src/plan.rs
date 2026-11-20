//! Compile-plan construction.

use std::path::PathBuf;

use seal_manifest::TargetKind;
use seal_plan::{BuildMode, CfgOptions, CompilePlan, PlanMetadata, PlanProfile};
use seal_target::TargetTriple;

use crate::error::{ProjectError, ProjectResult};
use crate::{Project, Target};

/// Options shared by `build`, `run`, and `check`.
#[derive(Clone, Debug)]
pub struct PlanOptions {
    /// The target triple.
    pub target: String,
    /// The profile name.
    pub profile: String,
    /// Whether to check or write an artifact.
    pub mode: BuildMode,
    /// An explicit artifact path.
    pub output: Option<PathBuf>,
    /// Enabled features.
    pub features: Vec<String>,
    /// Whether default features are disabled.
    pub no_default_features: bool,
    /// Whether every feature is enabled.
    pub all_features: bool,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            target: seal_target::host_triple().as_str().to_owned(),
            profile: "dev".to_owned(),
            mode: BuildMode::Check,
            output: None,
            features: Vec::new(),
            no_default_features: false,
            all_features: false,
        }
    }
}

pub(crate) fn compile_plan(
    project: &Project,
    package_index: usize,
    target_name: &str,
    options: &PlanOptions,
) -> ProjectResult<CompilePlan> {
    let package = project.package(package_index);
    let target = package.target(target_name).ok_or_else(|| {
        ProjectError::target(format!(
            "the package `{}` has no target `{target_name}`",
            package.name
        ))
    })?;
    let profile_name = target
        .profile
        .clone()
        .unwrap_or_else(|| options.profile.clone());
    let profile = profile_for(project, &profile_name)?;
    let triple = TargetTriple::parse(&options.target)
        .map_err(|error| ProjectError::plan(error.to_string()))?;
    let spec = triple.spec();

    let order = project.ordered_packages(package_index);
    let mut remap = vec![usize::MAX; project.packages.len()];
    for (position, index) in order.iter().enumerate() {
        remap[*index] = position;
    }
    let primary = remap[package_index];
    let mut packages = Vec::with_capacity(order.len());
    for index in &order {
        let mut plan_package = project.plan_package(*index, &remap);
        if *index == package_index {
            plan_package.files = target
                .files
                .iter()
                .map(|path| seal_plan::PlanFile {
                    path: path.clone(),
                    primary: *path == target.primary,
                })
                .collect();
        }
        plan_package.cfg = CfgOptions {
            features: options.features.clone(),
            target_os: spec.triple().os().as_str().to_owned(),
            target_arch: spec.triple().arch().as_str().to_owned(),
            target_env: String::new(),
            pointer_width: spec.pointer_width(),
            debug_assertions: profile.debug_assertions,
            test: target.kind == TargetKind::Test,
            bench: target.kind == TargetKind::Bench,
        };
        packages.push(plan_package);
    }

    let output = match options.mode {
        BuildMode::Check => options.output.clone(),
        BuildMode::Build => Some(match &options.output {
            Some(path) => path.clone(),
            None => default_output(project, target, &profile_name),
        }),
    };
    let entry = if target.kind.needs_entry()
        || target.kind == TargetKind::Test
        || target.kind == TargetKind::Bench
    {
        target.entry.clone()
    } else {
        None
    };
    let plan = CompilePlan {
        version: seal_plan::FORMAT_VERSION,
        target: options.target.clone(),
        profile,
        mode: options.mode,
        packages,
        primary,
        entry,
        metadata: PlanMetadata {
            project: project.name.clone(),
            target: target.name.clone(),
            kind: target.kind.as_str().to_owned(),
            root: package.root.display().to_string(),
        },
        output,
    };
    plan.validate().map_err(|error| {
        ProjectError::plan(format!(
            "the project produced an invalid plan for `{}`: {}",
            target.name, error
        ))
    })?;
    Ok(plan)
}

fn default_output(project: &Project, target: &Target, profile: &str) -> PathBuf {
    if let Some(path) = &target.output {
        return project.root.join(path);
    }
    let file = artifact_name(target.kind, &target.name);
    project
        .root
        .join("seal")
        .join("target")
        .join(profile)
        .join(file)
}

fn profile_for(project: &Project, name: &str) -> ProjectResult<PlanProfile> {
    let defaults = match name {
        "dev" | "test" => PlanProfile::default(),
        "release" | "bench" => PlanProfile {
            opt_level: 2,
            debug: false,
            debug_assertions: false,
            lto: "off".to_owned(),
            codegen_units: 1,
            strip: false,
            incremental: false,
        },
        _ => {
            let Some(declared) = project
                .packages
                .iter()
                .find_map(|package| package.manifest.profiles.get(name))
                .cloned()
            else {
                return Err(ProjectError::profile(format!(
                    "the profile `{name}` is not declared in the project"
                )));
            };
            let mut inherited = declared.inherits.as_deref().map_or_else(
                || Ok(PlanProfile::default()),
                |parent| profile_for(project, parent),
            )?;
            if let Some(value) = declared.opt_level {
                inherited.opt_level = value;
            }
            if let Some(value) = declared.debug {
                inherited.debug = value;
            }
            if let Some(value) = declared.debug_assertions {
                inherited.debug_assertions = value;
            }
            if let Some(value) = declared.lto {
                inherited.lto = value;
            }
            if let Some(value) = declared.codegen_units {
                inherited.codegen_units = value;
            }
            if let Some(value) = declared.strip {
                inherited.strip = value;
            }
            if let Some(value) = declared.incremental {
                inherited.incremental = value;
            }
            return Ok(inherited);
        }
    };
    Ok(defaults)
}

/// Returns the artifact name for a target, used by the CLI.
pub(crate) fn artifact_name(kind: TargetKind, name: &str) -> String {
    match kind {
        TargetKind::Lib | TargetKind::Staticlib | TargetKind::Cdylib => format!("lib{name}.o"),
        _ => name.to_owned(),
    }
}
