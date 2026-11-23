//! Compile-plan construction.

use std::path::PathBuf;

use seal_manifest::TargetKind;
use seal_plan::{BuildMode, CfgOptions, CompilePlan, PlanMetadata, PlanProfile, PlanTarget};
use seal_target::{PanicStrategy, RuntimePolicy, TargetTriple};
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
    /// Whether the standard library prelude is disabled for this plan.
    pub no_prelude: bool,
    /// Generated files added to the primary package before planning.
    pub extra_files: Vec<PathBuf>,
    /// Whether artifacts use canonical timestamps and paths.
    pub reproducible: bool,
    /// Overrides `SOURCE_DATE_EPOCH` for this plan.
    pub source_date_epoch: Option<u64>,
    /// The toolchain version that will compile the plan.
    pub toolchain: String,
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
            no_prelude: false,
            extra_files: Vec::new(),
            reproducible: true,
            source_date_epoch: None,
            toolchain: env!("CARGO_PKG_VERSION").to_owned(),
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
    let mut spec = project.target_spec(&options.target)?;
    if let Some(runtime) = &target.runtime {
        let policy = RuntimePolicy::parse(runtime).ok_or_else(|| {
            ProjectError::target(format!(
                "the target `{}` declares the unknown runtime policy `{runtime}`",
                target.name
            ))
        })?;
        spec = spec.with_runtime(policy);
    }
    if let Some(panic) = &target.panic {
        let policy = PanicStrategy::parse(panic).ok_or_else(|| {
            ProjectError::target(format!(
                "the target `{}` declares the unknown panic policy `{panic}`",
                target.name
            ))
        })?;
        spec = spec.with_panic(policy);
    }
    if target.linker.is_some() {
        spec = spec.with_linker(target.linker.clone());
    }
    if target.sysroot.is_some() {
        spec = spec.with_sysroot(target.sysroot.clone());
    }
    if target.linker_script.is_some() {
        spec = spec.with_linker_script(target.linker_script.clone());
    }
    if spec.is_freestanding() && spec.panic() != PanicStrategy::Trap {
        return Err(ProjectError::target(format!(
            "the freestanding target `{}` must use `panic = \"trap\"`",
            target.name
        )));
    }
    let host = seal_target::host_triple();
    let cross = triple != host;
    if options.mode == BuildMode::Build
        && cross
        && !matches!(target.kind, TargetKind::Lib | TargetKind::Staticlib)
        && spec.linker().is_none()
    {
        return Err(ProjectError::linker(format!(
            "the cross target `{}` needs a linker; declare `linker` in `{}` or the target manifest",
            options.target,
            project.target_spec_path(&options.target).display()
        )));
    }
    if options.mode == BuildMode::Build
        && cross
        && spec.runtime() == RuntimePolicy::Default
        && !matches!(target.kind, TargetKind::Lib | TargetKind::Generator)
    {
        return Err(ProjectError::artifact(format!(
            "the cross target `{}` would link the compiler host's runtime archive; declare `runtime = \"none\"` in the target or target manifest, or inspect the object with `seal check`",
            options.target
        )));
    }
    let source_date_epoch = source_date_epoch(options)?;
    for value in &options.cfgs {
        if !matches!(value.as_str(), "test" | "bench" | "debug_assertions") {
            return Err(ProjectError::manifest(format!(
                "`--cfg {value}` is not a supported boolean configuration; use `test`, `bench`, or `debug_assertions`"
            )));
        }
    }
    let graph = features::resolve(project, package_index, target, options, &cfg)?;
    let freestanding = spec.is_freestanding();
    let mut modes = Vec::with_capacity(graph.instances.len() + 1);
    for instance in &graph.instances {
        modes.push(crate::stdlib::mode(
            project.package(instance.package),
            freestanding,
        )?);
    }
    let loaded_stdlib = crate::stdlib::load(&modes)?;

    // An integration test is its own package: it depends on the package
    // library instead of being merged into it, so private items stay
    // unreachable. A test with `reuse = "lib"` is a unit test and keeps the
    // library's package, which is what gives it access to internals.
    let integration =
        matches!(target.kind, TargetKind::Test | TargetKind::Bench) && target.reuse.is_none();
    let mut primary = graph.primary;
    let mut packages = Vec::with_capacity(graph.instances.len() + integration as usize);
    for (position, instance) in graph.instances.iter().enumerate() {
        let package = project.package(instance.package);
        let source_target = if instance.package == package_index {
            if integration {
                package.library().or(Some(target))
            } else {
                Some(target)
            }
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
        let prelude = match &loaded_stdlib {
            Some(loaded) => {
                let manifest = package.manifest.package.as_ref().ok_or_else(|| {
                    ProjectError::manifest(format!(
                        "the package `{}` has no `[package]` table",
                        package.name
                    ))
                })?;
                crate::stdlib::prelude(
                    &loaded.library,
                    manifest,
                    modes[position],
                    options.no_prelude,
                )?
            }
            None => None,
        };
        packages.push(seal_plan::PlanPackage {
            name: package.name.clone(),
            version: package.version.clone(),
            edition: package.edition.to_string(),
            instance: instance_hash(package, &features, &cfg, &profile),
            root: package.root.clone(),
            files,
            exposed,
            dependencies,
            prelude,
            stdlib: modes[position],
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

    if integration {
        // The primary package used the library target above. Add the test
        // package after every dependency, with the library as its only edge.
        if let Some(library) = package.library() {
            let library_files: Vec<seal_plan::PlanFile> = library
                .files
                .iter()
                .map(|path| seal_plan::PlanFile {
                    path: path.clone(),
                    primary: *path == library.primary,
                })
                .collect();
            let mut primary_cfg = packages[primary].cfg.clone();
            primary_cfg.test = options.cfgs.iter().any(|value| value == "test");
            primary_cfg.bench = options.cfgs.iter().any(|value| value == "bench");
            packages[primary].files = library_files;
            packages[primary].cfg = primary_cfg;

            let mut test_cfg = packages[primary].cfg.clone();
            test_cfg.test =
                target.kind == TargetKind::Test || options.cfgs.iter().any(|value| value == "test");
            test_cfg.bench = target.kind == TargetKind::Bench
                || options.cfgs.iter().any(|value| value == "bench");
            let files = target
                .files
                .iter()
                .map(|path| seal_plan::PlanFile {
                    path: path.clone(),
                    primary: *path == target.primary,
                })
                .collect();
            let alias = package_alias(&package.name);
            let test_stdlib = modes[primary];
            let test_prelude = match &loaded_stdlib {
                Some(loaded) => {
                    let manifest = package.manifest.package.as_ref().ok_or_else(|| {
                        ProjectError::manifest(format!(
                            "the package `{}` has no `[package]` table",
                            package.name
                        ))
                    })?;
                    crate::stdlib::prelude(
                        &loaded.library,
                        manifest,
                        test_stdlib,
                        options.no_prelude,
                    )?
                }
                None => None,
            };
            let index = packages.len();
            packages.push(seal_plan::PlanPackage {
                name: package.name.clone(),
                version: package.version.clone(),
                edition: package.edition.to_string(),
                instance: test_instance_hash(
                    package,
                    &test_cfg.features,
                    &cfg,
                    &profile,
                    &target.name,
                ),
                root: package.root.clone(),
                files,
                exposed: vec!["root".to_owned()],
                dependencies: vec![seal_plan::PlanDependency {
                    alias,
                    package: primary,
                }],
                cfg: test_cfg,
                prelude: test_prelude,
                stdlib: test_stdlib,
            });
            primary = index;
        }
    }

    let output = match options.mode {
        BuildMode::Check => options.output.clone(),
        BuildMode::Build => Some(match &options.output {
            Some(path) => path.clone(),
            None => default_output(project, target, &profile_name),
        }),
    };
    // Test and benchmark targets are driven by the generated harness; they
    // have no Seal-level entry function.
    let entry = if matches!(target.kind, TargetKind::Test | TargetKind::Bench) {
        None
    } else if target.kind.needs_entry() {
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
        stdlib: loaded_stdlib.map(|loaded| loaded.info),
        entry,
        metadata: PlanMetadata {
            project: project.name.clone(),
            target: target.name.clone(),
            kind: target.kind.as_str().to_owned(),
            root: package.root.display().to_string(),
            toolchain: options.toolchain.clone(),
        },
        output,
        target_spec: Some(PlanTarget::from_spec(
            &spec,
            target.link_args.clone(),
            source_date_epoch,
            options.reproducible,
        )),
    };
    plan.validate().map_err(|error| {
        ProjectError::plan(format!(
            "the project produced an invalid plan for `{}`: {}",
            target.name, error
        ))
    })?;
    Ok(plan)
}

fn source_date_epoch(options: &PlanOptions) -> ProjectResult<u64> {
    if let Some(epoch) = options.source_date_epoch {
        return Ok(epoch);
    }
    match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(value) => value.parse::<u64>().map_err(|_| {
            ProjectError::plan(format!(
                "`SOURCE_DATE_EPOCH` must be a non-negative integer, but it is `{value}`"
            ))
        }),
        Err(std::env::VarError::NotPresent) => Ok(0),
        Err(std::env::VarError::NotUnicode(_)) => {
            Err(ProjectError::plan("`SOURCE_DATE_EPOCH` is not valid UTF-8"))
        }
    }
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

/// Returns the dependency alias a test package uses for its own library.
fn package_alias(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// Returns a stable instance number for a test package.
fn test_instance_hash(
    package: &crate::Package,
    features: &[String],
    cfg: &CfgEnvironment,
    profile: &PlanProfile,
    target: &str,
) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(b"seal-test-package-v2\0");
    hasher.update(package.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(package.version.as_bytes());
    hasher.update(b"\0");
    hasher.update(package.edition.to_string().as_bytes());
    hasher.update(b"\0");
    hasher.update(target.as_bytes());
    hasher.update(b"\0");
    for feature in features {
        hasher.update(feature.as_bytes());
        hasher.update(b"\0");
    }
    hasher.update(cfg.os().as_bytes());
    hasher.update(b"\0");
    hasher.update(cfg.arch().as_bytes());
    hasher.update([cfg.pointer_width()]);
    hasher.update([profile.debug_assertions as u8]);
    let digest = hasher.finalize();
    u32::from_be_bytes([digest[0], digest[1], digest[2], digest[3]])
}

pub(crate) fn instance_hash(
    package: &crate::Package,
    features: &[String],
    cfg: &CfgEnvironment,
    profile: &PlanProfile,
) -> u32 {
    let mut hasher = Sha256::new();
    hasher.update(b"seal-package-instance-v2\0");
    hasher.update(package.name.as_bytes());
    hasher.update(b"\0");
    hasher.update(package.version.as_bytes());
    hasher.update(b"\0");
    hasher.update(package.edition.to_string().as_bytes());
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
        TargetKind::Lib => format!("lib{name}.o"),
        TargetKind::Staticlib => seal_plan::static_library_name(name),
        TargetKind::Cdylib => seal_plan::shared_library_name(name),
        _ => name.to_owned(),
    }
}
