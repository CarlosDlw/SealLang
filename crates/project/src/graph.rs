//! Member discovery and package-graph resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSetBuilder};
use seal_manifest::{DependencySpec, LoadedManifest, Manifest, ManifestWarning};
use seal_target::host_triple;
use walkdir::WalkDir;

use crate::error::{ProjectError, ProjectResult};
use crate::targets;
use crate::{Dependency, Package, Project, source};

struct Spec {
    root: PathBuf,
    loaded: LoadedManifest,
}

pub(crate) fn load(root: &Path) -> ProjectResult<Project> {
    let canonical_root = root.canonicalize().map_err(|error| {
        ProjectError::io(format!("cannot resolve `{}`: {error}", root.display()))
    })?;
    if !canonical_root.is_dir() {
        return Err(ProjectError::io(format!(
            "the project root `{}` is not a directory",
            canonical_root.display()
        )));
    }
    let root_loaded = crate::load_manifest(&canonical_root)?;
    let mut warnings = root_loaded.warnings.clone();
    let mut specs: BTreeMap<PathBuf, Spec> = BTreeMap::new();

    let member_roots = if let Some(project) = &root_loaded.manifest.project {
        discover_members(&canonical_root, project, &root_loaded.manifest)?
    } else {
        Vec::new()
    };

    if root_loaded.manifest.package.is_some() {
        specs.insert(
            canonical_root.clone(),
            Spec {
                root: canonical_root.clone(),
                loaded: root_loaded.clone(),
            },
        );
    }
    for member in member_roots {
        if specs.contains_key(&member) {
            continue;
        }
        let loaded = crate::load_manifest(&member)?;
        if loaded.manifest.package.is_none() {
            return Err(ProjectError::manifest(format!(
                "the member `{}` has no `[package]` table",
                member.display()
            )));
        }
        warnings.extend(loaded.warnings.clone());
        specs.insert(
            member.clone(),
            Spec {
                root: member,
                loaded,
            },
        );
    }
    if specs.is_empty() {
        return Err(ProjectError::manifest(
            "the project has no packages; add a `[package]` table or list members in `[project]`",
        ));
    }

    resolve_path_dependencies(&canonical_root, &mut specs, &mut warnings)?;

    let mut names = BTreeMap::new();
    for spec in specs.values() {
        let package =
            spec.loaded.manifest.package.as_ref().ok_or_else(|| {
                ProjectError::manifest("a project member has no `[package]` table")
            })?;
        if let Some(previous) = names.insert(package.name.clone(), spec.root.clone()) {
            return Err(ProjectError::manifest(format!(
                "the package name `{}` is used by both `{}` and `{}`",
                package.name,
                previous.display(),
                spec.root.display()
            )));
        }
    }

    let order = topological_order(&specs)?;
    let mut indexes = BTreeMap::new();
    for (index, root) in order.iter().enumerate() {
        indexes.insert(root.clone(), index);
    }

    let mut packages = Vec::with_capacity(order.len());
    for root in &order {
        let spec = specs
            .get(root)
            .ok_or_else(|| ProjectError::manifest("a package disappeared during resolution"))?;
        let manifest = spec.loaded.manifest.clone();
        let package = manifest.package.as_ref().ok_or_else(|| {
            ProjectError::manifest(format!("the member `{}` has no package", root.display()))
        })?;
        let files = source::package_files(root, &package.sources, &package.exclude)?;
        let targets = targets::build(root, &manifest, &files)?;
        let dependencies = dependency_edges(root, &manifest, &specs, &indexes)?;
        packages.push(Package {
            name: package.name.clone(),
            version: package.version.clone(),
            root: root.clone(),
            manifest,
            files,
            targets,
            dependencies,
        });
    }

    let name = root_loaded
        .manifest
        .project
        .as_ref()
        .map(|project| project.name.clone())
        .or_else(|| {
            root_loaded
                .manifest
                .package
                .as_ref()
                .map(|package| package.name.clone())
        })
        .or_else(|| packages.first().map(|package| package.name.clone()))
        .unwrap_or_else(|| "project".to_owned());

    let mut project = Project {
        root: canonical_root,
        name,
        packages,
        warnings,
        default_members: root_loaded
            .manifest
            .project
            .as_ref()
            .map(|project| project.default_members.clone())
            .unwrap_or_default(),
        profiles: root_loaded.manifest.profiles.clone(),
        lockfile: crate::Lockfile::default(),
    };
    project.lockfile = project.resolve_lockfile(host_triple().as_str());
    Ok(project)
}

fn discover_members(
    root: &Path,
    project: &seal_manifest::ProjectManifest,
    manifest: &Manifest,
) -> ProjectResult<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    if manifest.package.is_some() {
        candidates.push(root.to_path_buf());
    }
    if project.members.is_empty() {
        if candidates.is_empty() {
            return Err(ProjectError::manifest(
                "a `[project]` without members needs a `[package]` table at the root",
            ));
        }
        return Ok(candidates);
    }

    let mut builder = GlobSetBuilder::new();
    let mut literal = Vec::new();
    for pattern in &project.members {
        validate_member_pattern(pattern)?;
        builder.add(Glob::new(pattern).map_err(|error| {
            ProjectError::manifest(format!("invalid member glob `{pattern}`: {error}"))
        })?);
        if !pattern.contains(['*', '?', '[', ']']) {
            literal.push(pattern.clone());
        }
    }
    let set = builder
        .build()
        .map_err(|error| ProjectError::manifest(format!("invalid member globs: {error}")))?;

    let mut discovered = BTreeSet::new();
    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| !skip(entry.path(), root))
    {
        let entry = entry.map_err(|error| {
            ProjectError::io(format!("cannot walk `{}`: {error}", root.display()))
        })?;
        if !entry.file_type().is_dir() || !entry.path().join("seal.toml").is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| ProjectError::io("a member is outside the project root"))?;
        let relative = slash(relative);
        if relative.is_empty() {
            continue;
        }
        if set.is_match(&relative) {
            let canonical = entry.path().canonicalize().map_err(|error| {
                ProjectError::io(format!(
                    "cannot resolve `{}`: {error}",
                    entry.path().display()
                ))
            })?;
            if !canonical.starts_with(root) {
                return Err(ProjectError::manifest(format!(
                    "the member `{}` resolves outside the project root",
                    entry.path().display()
                )));
            }
            discovered.insert(canonical);
        }
    }
    for pattern in literal {
        let directory = root.join(&pattern);
        if directory.join("seal.toml").is_file() {
            discovered.insert(directory.canonicalize().map_err(|error| {
                ProjectError::io(format!("cannot resolve `{}`: {error}", directory.display()))
            })?);
        } else {
            return Err(ProjectError::source(format!(
                "the member `{pattern}` has no `seal.toml`"
            )));
        }
    }
    for pattern in &project.members {
        if !discovered.iter().any(|member| {
            member
                .strip_prefix(root)
                .map(slash)
                .is_ok_and(|relative| set.is_match(&relative))
        }) {
            return Err(ProjectError::source(format!(
                "the member glob `{pattern}` matched no package"
            )));
        }
    }
    let mut discovered: Vec<PathBuf> = discovered.into_iter().collect();
    for candidate in candidates {
        if !discovered.contains(&candidate) {
            discovered.push(candidate);
        }
    }
    discovered.sort();
    Ok(discovered)
}

fn validate_member_pattern(pattern: &str) -> ProjectResult<()> {
    if pattern.is_empty() {
        return Err(ProjectError::manifest("a member glob cannot be empty"));
    }
    let path = Path::new(pattern);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(ProjectError::manifest(format!(
            "the member glob `{pattern}` must stay inside the project"
        )));
    }
    Ok(())
}

fn skip(path: &Path, root: &Path) -> bool {
    if path == root {
        return false;
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    relative.components().any(|component| {
        matches!(
            component,
            Component::Normal(name)
                if matches!(name.to_str(), Some(".git" | "target" | "node_modules" | "seal"))
        )
    })
}

fn slash(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(segment) => Some(segment.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn resolve_path_dependencies(
    project_root: &Path,
    specs: &mut BTreeMap<PathBuf, Spec>,
    warnings: &mut Vec<ManifestWarning>,
) -> ProjectResult<()> {
    let mut queue: Vec<PathBuf> = specs.keys().cloned().collect();
    while let Some(root) = queue.pop() {
        let dependencies = path_dependencies(&root, specs)?;
        for (alias, path, requirement) in dependencies {
            let canonical = path.canonicalize().map_err(|error| {
                ProjectError::path_dependency(format!(
                    "the path dependency `{alias}` at `{}` cannot be resolved: {error}",
                    path.display()
                ))
            })?;
            if !canonical.starts_with(project_root) {
                return Err(ProjectError::path_escape(format!(
                    "the path dependency `{alias}` at `{}` leaves the project root",
                    path.display()
                )));
            }
            if !canonical.is_dir() || !canonical.join("seal.toml").is_file() {
                return Err(ProjectError::path_dependency(format!(
                    "the path dependency `{alias}` at `{}` has no `seal.toml`",
                    path.display()
                )));
            }
            if !specs.contains_key(&canonical) {
                let loaded = crate::load_manifest(&canonical)?;
                if loaded.manifest.package.is_none() {
                    return Err(ProjectError::dependency_declaration(format!(
                        "the path dependency `{alias}` at `{}` has no `[package]` table",
                        canonical.display()
                    )));
                }
                warnings.extend(loaded.warnings.clone());
                queue.push(canonical.clone());
                specs.insert(
                    canonical.clone(),
                    Spec {
                        root: canonical.clone(),
                        loaded,
                    },
                );
            }
            if let Some(requirement) = requirement {
                let dependency = specs
                    .get(&canonical)
                    .and_then(|spec| spec.loaded.manifest.package.as_ref())
                    .ok_or_else(|| ProjectError::dependency("a dependency has no package"))?;
                let version = semver::Version::parse(&dependency.version).map_err(|error| {
                    ProjectError::version(format!(
                        "the path dependency `{alias}` has an invalid version: {error}"
                    ))
                })?;
                if !requirement.matches(&version) {
                    return Err(ProjectError::version(format!(
                        "the path dependency `{alias}` requires `{requirement}` but the package is `{version}`"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn path_dependencies(
    root: &Path,
    specs: &BTreeMap<PathBuf, Spec>,
) -> ProjectResult<Vec<(String, PathBuf, Option<semver::VersionReq>)>> {
    let spec = specs
        .get(root)
        .ok_or_else(|| ProjectError::manifest("a package is missing from the graph"))?;
    let mut dependencies = Vec::new();
    for (alias, dependency) in &spec.loaded.manifest.dependencies {
        match dependency {
            DependencySpec::Version(_) => {
                return Err(ProjectError::dependency(format!(
                    "the dependency `{alias}` uses a registry version; this phase supports path dependencies only"
                )));
            }
            DependencySpec::Detailed(detail) => {
                if detail.git.is_some() {
                    return Err(ProjectError::dependency(format!(
                        "the dependency `{alias}` uses git; this phase supports path dependencies only"
                    )));
                }
                let path = detail.path.as_ref().ok_or_else(|| {
                    ProjectError::dependency(format!(
                        "the dependency `{alias}` has no path; this phase supports path dependencies only"
                    ))
                })?;
                let requirement = detail
                    .version
                    .as_ref()
                    .map(|version| {
                        semver::VersionReq::parse(version).map_err(|error| {
                            ProjectError::dependency(format!(
                                "the dependency `{alias}` has an invalid version requirement: {error}"
                            ))
                        })
                    })
                    .transpose()?;
                dependencies.push((alias.clone(), root.join(path), requirement));
            }
        }
    }
    Ok(dependencies)
}

fn dependency_edges(
    root: &Path,
    manifest: &Manifest,
    specs: &BTreeMap<PathBuf, Spec>,
    indexes: &BTreeMap<PathBuf, usize>,
) -> ProjectResult<Vec<Dependency>> {
    let mut dependencies = Vec::new();
    for (alias, dependency) in &manifest.dependencies {
        let detail = match dependency {
            DependencySpec::Version(_) => {
                return Err(ProjectError::dependency(format!(
                    "the dependency `{alias}` uses a registry version; this phase supports path dependencies only"
                )));
            }
            DependencySpec::Detailed(detail) => detail,
        };
        let path = detail.path.as_ref().ok_or_else(|| {
            ProjectError::dependency(format!("the dependency `{alias}` has no path"))
        })?;
        let canonical = root.join(path).canonicalize().map_err(|error| {
            ProjectError::path_dependency(format!(
                "the dependency `{alias}` cannot be resolved: {error}"
            ))
        })?;
        if !specs.contains_key(&canonical) {
            return Err(ProjectError::dependency(format!(
                "the dependency `{alias}` was not loaded"
            )));
        }
        let package = *indexes.get(&canonical).ok_or_else(|| {
            ProjectError::dependency(format!("the dependency `{alias}` has no package index"))
        })?;
        dependencies.push(Dependency {
            alias: alias.clone(),
            package,
            features: detail.features.clone(),
            default_features: detail.default_features,
            optional: detail.optional,
            when: detail.when.clone(),
        });
    }
    dependencies.sort_by(|left, right| left.alias.cmp(&right.alias));
    Ok(dependencies)
}

fn topological_order(specs: &BTreeMap<PathBuf, Spec>) -> ProjectResult<Vec<PathBuf>> {
    let mut states: BTreeMap<PathBuf, u8> = BTreeMap::new();
    let mut stack = Vec::new();
    let mut order = Vec::new();
    for root in specs.keys() {
        visit(root, specs, &mut states, &mut stack, &mut order)?;
    }
    Ok(order)
}

fn visit(
    root: &Path,
    specs: &BTreeMap<PathBuf, Spec>,
    states: &mut BTreeMap<PathBuf, u8>,
    stack: &mut Vec<PathBuf>,
    order: &mut Vec<PathBuf>,
) -> ProjectResult<()> {
    match states.get(root).copied() {
        Some(2) => return Ok(()),
        Some(1) => {
            let start = stack.iter().position(|entry| entry == root).unwrap_or(0);
            let mut cycle: Vec<String> = stack[start..]
                .iter()
                .map(|entry| entry.display().to_string())
                .collect();
            cycle.push(root.display().to_string());
            return Err(ProjectError::cycle(format!(
                "the dependency graph contains a cycle: {}",
                cycle.join(" -> ")
            )));
        }
        _ => {}
    }
    states.insert(root.to_path_buf(), 1);
    stack.push(root.to_path_buf());
    for (_, path, _) in path_dependencies(root, specs)? {
        let canonical = path.canonicalize().map_err(|error| {
            ProjectError::dependency(format!("cannot resolve `{}`: {error}", path.display()))
        })?;
        visit(&canonical, specs, states, stack, order)?;
    }
    stack.pop();
    states.insert(root.to_path_buf(), 2);
    order.push(root.to_path_buf());
    Ok(())
}
