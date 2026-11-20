//! `seal metadata`: the project model as versioned JSON.

use std::path::Path;
use std::process::ExitCode;

use seal_project::{Package, Target};
use seal_target::host_triple;
use serde::Serialize;

use crate::cli::{Cli, MetadataArgs};
use crate::ui;

#[derive(Serialize)]
struct Metadata {
    version: u32,
    project: String,
    root: String,
    target: TargetInfo,
    packages: Vec<PackageInfo>,
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
    features: Vec<String>,
    enabled_features: Vec<String>,
    targets: Vec<TargetInfoEntry>,
    modules: Vec<String>,
    files: Vec<String>,
    dependencies: Vec<String>,
}

#[derive(Serialize)]
struct TargetInfoEntry {
    name: String,
    kind: String,
    entry: Option<String>,
    primary: String,
    files: Vec<String>,
    profile: Option<String>,
}

pub(crate) fn metadata(_cli: &Cli, root: &Path, args: &MetadataArgs) -> ExitCode {
    let project = match crate::cli::load(root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let triple = host_triple();
    let spec = triple.spec();
    let packages = if args.no_deps {
        match project.default_package() {
            Ok(index) => vec![project.package(index)],
            Err(error) => return ui::error(error.code(), error.message()),
        }
    } else {
        project.packages.iter().collect()
    };
    let metadata = Metadata {
        version: 1,
        project: project.name.clone(),
        root: project.root.display().to_string(),
        target: TargetInfo {
            triple: triple.as_str().to_owned(),
            os: spec.triple().os().as_str().to_owned(),
            arch: spec.triple().arch().as_str().to_owned(),
            pointer_width: spec.pointer_width(),
        },
        packages: packages.into_iter().map(package_info).collect(),
    };
    match serde_json::to_string_pretty(&metadata) {
        Ok(text) => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => ui::error("E8011", &format!("cannot serialize metadata: {error}")),
    }
}

fn package_info(package: &Package) -> PackageInfo {
    let mut features: Vec<String> = package
        .manifest
        .package
        .as_ref()
        .map(|manifest| manifest.features.keys().cloned().collect())
        .unwrap_or_default();
    features.sort();
    PackageInfo {
        name: package.name.clone(),
        version: package.version.clone(),
        root: package.root.display().to_string(),
        features,
        enabled_features: Vec::new(),
        targets: package.targets.iter().map(target_info).collect(),
        modules: modules(package),
        files: package
            .files
            .iter()
            .filter_map(|path| path.strip_prefix(&package.root).ok())
            .map(|path| path.display().to_string())
            .collect(),
        dependencies: package
            .dependencies
            .iter()
            .map(|dependency| dependency.alias.clone())
            .collect(),
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
    let relative = file.strip_prefix(root).ok()?.strip_prefix("src").ok()?;
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
