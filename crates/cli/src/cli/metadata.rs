//! `seal metadata`: the project model as versioned JSON.

use std::path::Path;
use std::process::ExitCode;

use seal_project::{LockInstance, Package, Project, Target};
use seal_target::host_triple;
use serde::Serialize;

use crate::cli::{Cli, MetadataArgs};
use crate::ui;

#[derive(Serialize)]
struct Metadata {
    version: u32,
    project: String,
    root: String,
    workspace: WorkspaceInfo,
    target: TargetInfo,
    packages: Vec<PackageInfo>,
    instances: Vec<LockInstance>,
}

#[derive(Serialize)]
struct WorkspaceInfo {
    name: String,
    root: String,
    default_members: Vec<String>,
    members: Vec<String>,
    lockfile: LockfileInfo,
}

#[derive(Serialize)]
struct LockfileInfo {
    version: u32,
    toolchain: String,
    targets: Vec<String>,
}

#[derive(Serialize)]
struct TargetInfo {
    triple: String,
    os: String,
    arch: String,
    pointer_width: u8,
}

#[derive(Serialize)]
struct PackageInfo {
    name: String,
    version: String,
    root: String,
    source: String,
    description: Option<String>,
    license: Option<String>,
    repository: Option<String>,
    features: Vec<String>,
    enabled_features: Vec<String>,
    targets: Vec<TargetInfoEntry>,
    modules: Vec<String>,
    files: Vec<String>,
    dependencies: Vec<String>,
    dependency_edges: Vec<DependencyInfo>,
}

#[derive(Serialize)]
struct DependencyInfo {
    alias: String,
    package: usize,
    name: String,
    version: String,
    features: Vec<String>,
    optional: bool,
    default_features: bool,
}

#[derive(Serialize)]
struct TargetInfoEntry {
    name: String,
    kind: String,
    entry: Option<String>,
    primary: String,
    files: Vec<String>,
    profile: Option<String>,
    required_features: Vec<String>,
    generator_inputs: Vec<String>,
    generator_outputs: Vec<String>,
    generated: Vec<String>,
    native: Vec<String>,
}

pub(crate) fn metadata(cli: &Cli, root: &Path, args: &MetadataArgs) -> ExitCode {
    let _ = (args.json, args.format);
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    match report_json(&project, args.no_deps) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(message) => ui::error("E8011", &message),
    }
}

/// Serializes the metadata document for a loaded project.
///
/// Keeping this separate from stdout makes the contract testable without
/// capturing process output.
pub(crate) fn report_json(project: &Project, no_deps: bool) -> Result<String, String> {
    let triple = host_triple();
    let spec = triple.spec();
    let packages = if no_deps {
        vec![
            project
                .default_package()
                .map(|index| project.package(index))
                .map_err(|error| error.message().to_owned())?,
        ]
    } else {
        project.packages.iter().collect()
    };
    let metadata = Metadata {
        version: 1,
        project: project.name.clone(),
        root: project.root.display().to_string(),
        workspace: WorkspaceInfo {
            name: project.name.clone(),
            root: project.root.display().to_string(),
            default_members: project.default_members.clone(),
            members: project
                .packages
                .iter()
                .filter(|package| {
                    package.root != project.root
                        && matches!(package.source, seal_project::PackageSource::Path)
                })
                .map(|package| package.name.clone())
                .collect(),
            lockfile: LockfileInfo {
                version: project.lockfile.version,
                toolchain: project.lockfile.toolchain.clone(),
                targets: project.lockfile.targets.clone(),
            },
        },
        target: TargetInfo {
            triple: triple.as_str().to_owned(),
            os: spec.triple().os().as_str().to_owned(),
            arch: spec.triple().arch().as_str().to_owned(),
            pointer_width: spec.pointer_width(),
        },
        packages: packages
            .into_iter()
            .map(|package| package_info(package, project, !no_deps))
            .collect(),
        instances: project.lockfile.instances.clone(),
    };
    serde_json::to_string_pretty(&metadata)
        .map_err(|error| format!("cannot serialize metadata: {error}"))
}

fn package_info(package: &Package, project: &Project, include_dependencies: bool) -> PackageInfo {
    let mut features: Vec<String> = package
        .manifest
        .package
        .as_ref()
        .map(|manifest| manifest.features.names().map(ToOwned::to_owned).collect())
        .unwrap_or_default();
    features.sort();
    PackageInfo {
        name: package.name.clone(),
        version: package.version.clone(),
        root: package.root.display().to_string(),
        source: package_source(package),
        description: package
            .manifest
            .package
            .as_ref()
            .and_then(|manifest| manifest.description.clone()),
        license: package
            .manifest
            .package
            .as_ref()
            .and_then(|manifest| manifest.license.clone()),
        repository: package
            .manifest
            .package
            .as_ref()
            .and_then(|manifest| manifest.repository.clone()),
        features,
        enabled_features: project
            .lockfile
            .instances
            .iter()
            .filter(|instance| instance.name == package.name && instance.version == package.version)
            .flat_map(|instance| instance.features.iter().cloned())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
        targets: package.targets.iter().map(target_info).collect(),
        modules: modules(package),
        files: package
            .files
            .iter()
            .filter_map(|path| path.strip_prefix(&package.root).ok())
            .map(|path| path.display().to_string())
            .collect(),
        dependencies: if include_dependencies {
            package
                .dependencies
                .iter()
                .map(|dependency| dependency.alias.clone())
                .collect()
        } else {
            Vec::new()
        },
        dependency_edges: if include_dependencies {
            package
                .dependencies
                .iter()
                .map(|dependency| {
                    let target = project.package(dependency.package);
                    DependencyInfo {
                        alias: dependency.alias.clone(),
                        package: dependency.package,
                        name: target.name.clone(),
                        version: target.version.clone(),
                        features: dependency.features.clone(),
                        optional: dependency.optional,
                        default_features: dependency.default_features,
                    }
                })
                .collect()
        } else {
            Vec::new()
        },
    }
}

fn target_info(target: &Target) -> TargetInfoEntry {
    TargetInfoEntry {
        name: target.name.clone(),
        kind: target.kind.as_str().to_owned(),
        entry: target.entry.clone(),
        primary: target.primary.display().to_string(),
        files: target
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        profile: target.profile.clone(),
        required_features: target.required_features.clone(),
        generator_inputs: target
            .generator_inputs
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        generator_outputs: target.generator_outputs.clone(),
        generated: target.generated.clone(),
        native: target.native.clone(),
    }
}

fn package_source(package: &Package) -> String {
    match &package.source {
        seal_project::PackageSource::Path => "path".to_owned(),
        seal_project::PackageSource::Registry(source) => format!("registry+{}", source.registry),
    }
}

fn modules(package: &Package) -> Vec<String> {
    let mut modules: Vec<String> = package
        .files
        .iter()
        .filter_map(|file| module_of(file, &package.root))
        .collect();
    modules.sort();
    modules.dedup();
    modules
}

/// Returns the module a file contributes to.
///
/// The declaration is the first non-comment item. Parsing it here keeps
/// metadata independent from the compiler pipeline; a file without a
/// declaration contributes to the root. A malformed declaration is omitted
/// from the index and reported by `seal build` instead.
fn module_of(file: &std::path::Path, root: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(file).ok()?;
    for line in text.lines() {
        let line = line.split("//").next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix("module ") else {
            break;
        };
        let path = rest.strip_suffix(';')?.trim();
        if path.is_empty() {
            return None;
        }
        let segments: Vec<&str> = path.split("::").collect();
        if segments
            .iter()
            .any(|segment| segment.is_empty() || *segment == "root")
        {
            return None;
        }
        return Some(segments.join("::"));
    }
    let relative = file.strip_prefix(root).ok()?;
    let relative = relative.strip_prefix("src").unwrap_or(relative);
    let relative = relative.with_extension("");
    let mut segments: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if matches!(segments.last().map(String::as_str), Some("main" | "lib")) {
        segments.pop();
    }
    if segments.is_empty() {
        Some("root".to_owned())
    } else {
        Some(segments.join("::"))
    }
}
