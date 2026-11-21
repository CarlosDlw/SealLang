//! Compile-plan construction.

use std::path::PathBuf;

use seal_manifest::TargetKind;
use seal_plan::{BuildMode, CfgOptions, CompilePlan, PlanMetadata, PlanProfile};
use seal_target::TargetTriple;
use sha2::{Digest, Sha256};

use crate::cfg::CfgEnvironment;
use crate::error::{ProjectError, ProjectResult};
use crate::{Project, Target, features, profiles};

/// Options shared by `build`, `run`, and `check`.
#[derive(Clone, Debug)]
pub struct PlanOptions {
    /// The target triple.
    pub target: String,
    /// The profile name.
    pub profile: String,
    /// Whether `profile` came from the command line and overrides the target.
    pub profile_explicit: bool,
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
    /// Boolean `cfg` facts forced on the command line.
    pub cfgs: Vec<String>,
    /// Generated files added to the primary package before planning.
    pub extra_files: Vec<PathBuf>,
}

impl Default for PlanOptions {
    fn default() -> Self {
        Self {
            target: seal_target::host_triple().as_str().to_owned(),
            profile: "dev".to_owned(),
            profile_explicit: false,
            mode: BuildMode::Check,
            output: None,
            features: Vec::new(),
            no_default_features: false,
            all_features: false,
            cfgs: Vec::new(),
            extra_files: Vec::new(),
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
    let profile_name = if options.profile_explicit {
        options.profile.clone()
    } else {
        target
            .profile
            .clone()
            .unwrap_or_else(|| options.profile.clone())
    };
    let profile = profiles::resolve(project, &profile_name)?;
    let triple = TargetTriple::parse(&options.target)
        .map_err(|error| ProjectError::plan(error.to_string()))?;
    let spec = triple.spec();
    let cfg = CfgEnvironment::for_triple(&triple);
    if target
        .when
        .as_ref()
        .is_some_and(|condition| !cfg.matches(condition))
    {
        return Err(ProjectError::target(format!(
            "the target `{}` is inactive for `{}`",
            target.name, options.target
        )));
    }
    for value in &options.cfgs {
        if !matches!(value.as_str(), "test" | "bench" | "debug_assertions") {
            return Err(ProjectError::manifest(format!(
                "`--cfg {value}` is not a supported boolean configuration; use `test`, `bench`, or `debug_assertions`"
            )));
        }
    }
    let graph = features::resolve(project, package_index, target, options, &cfg)?;

    let primary = graph.primary;
    let mut packages = Vec::with_capacity(graph.instances.len());
    for (position, instance) in graph.instances.iter().enumerate() {
        let package = project.package(instance.package);
        let source_target = if instance.package == package_index {
            Some(target)
        } else {
            package.library().or_else(|| package.targets.first())
        };
        let mut files: Vec<seal_plan::PlanFile> = source_target
            .map(|target| {
                target
                    .files
                    .iter()
                    .map(|path| seal_plan::PlanFile {
                        path: path.clone(),
                        primary: *path == target.primary,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if instance.package == package_index {
            for path in &options.extra_files {
                if !files.iter().any(|file| file.path == *path) {
                    files.push(seal_plan::PlanFile {
                        path: path.clone(),
                        primary: false,
                    });
                }
            }
        }
        let exposed = package
            .manifest
            .package
            .as_ref()
            .and_then(|manifest| manifest.api.as_ref())
            .map_or_else(|| vec!["root".to_owned()], |api| api.expose.clone());
        let dependencies = instance
            .dependencies
            .iter()
            .map(|dependency| seal_plan::PlanDependency {
                alias: dependency.alias.clone(),
                package: dependency.instance,
            })
            .collect();
        let features: Vec<String> = instance.features.iter().cloned().collect();
        packages.push(seal_plan::PlanPackage {
            name: package.name.clone(),
            version: package.version.clone(),
            instance: instance_hash(package, &features, &cfg, &profile),
            root: package.root.clone(),
            files,
            exposed,
            dependencies,
            cfg: CfgOptions {
                features,
                target_os: spec.triple().os().as_str().to_owned(),
                target_arch: spec.triple().arch().as_str().to_owned(),
                target_env: String::new(),
                pointer_width: spec.pointer_width(),
                debug_assertions: profile.debug_assertions
                    || options.cfgs.iter().any(|value| value == "debug_assertions"),
                test: position == primary
                    && (target.kind == TargetKind::Test
                        || options.cfgs.iter().any(|value| value == "test")),
                bench: position == primary
                    && (target.kind == TargetKind::Bench
                        || options.cfgs.iter().any(|value| value == "bench")),
                env: package
                    .manifest
                    .env
                    .iter()
                    .map(|(key, value)| (key.clone(), value.value().to_owned()))
                    .collect(),
            },
        });
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
        // Tests and benches currently use the same executable shape as a
        // binary; the future test harness replaces this conventional entry.
        target.entry.clone().or_else(|| {
            matches!(target.kind, TargetKind::Test | TargetKind::Bench)
                .then(|| "root::main".to_owned())
        })
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

pub(crate) fn instance_hash(
    package: &crate::Package,
    features: &[String],
    cfg: &CfgEnvironment,
    profile: &PlanProfile,
) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(b"seal-package-instance-v1\0");
    hasher.update(package.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(package.version.as_bytes());
    hasher.update(b"\0");
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(cfg.os().as_bytes());
    hasher.update(b"\0");
    hasher.update(cfg.arch().as_bytes());
    hasher.update(b"\0");
    hasher.update(cfg.env().as_bytes());
    hasher.update(b"\0");
    hasher.update([cfg.pointer_width()]);
    hasher.update([profile.debug_assertions as u8]);
    let digest = hasher.finalize();
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
}

/// Returns the artifact name for a target, used by the CLI.
pub(crate) fn artifact_name(kind: TargetKind, name: &str) -> String {
    match kind {
        TargetKind::Lib | TargetKind::Staticlib | TargetKind::Cdylib => format!("lib{name}.o"),
        _ => name.to_owned(),
    }
}
