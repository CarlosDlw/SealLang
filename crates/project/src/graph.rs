//! Member discovery and package-graph resolution.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSetBuilder};
use seal_manifest::{DependencySpec, LoadedManifest, Manifest, ManifestWarning};
use seal_registry::vendor::{self, VendorManifest};
use seal_registry::{
    RegistrySession, Requirement, ResolveOptions, ResolvedPackage, download, semver::Line,
};
use seal_target::host_triple;
use semver::{Version, VersionReq};
use walkdir::WalkDir;

use crate::error::{ProjectError, ProjectResult};
use crate::targets;
use crate::{Dependency, Package, PackageSource, Project, ProjectOptions, RegistrySource, source};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum SpecKey {
    Path(PathBuf),
    Registry { name: String, version: String },
}

impl std::fmt::Display for SpecKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Path(root) => write!(f, "{}", root.display()),
            Self::Registry { name, version } => write!(f, "{name} {version}"),
        }
    }
}

struct Spec {
    root: PathBuf,
    loaded: LoadedManifest,
    source: PackageSource,
}

struct LockCandidate {
    name: String,
    version: Version,
    checksum: String,
    registry: String,
    root: PathBuf,
    loaded: LoadedManifest,
    yanked: bool,
    provenance: Option<String>,
    api: Option<String>,
}

pub(crate) fn load(root: &Path, options: &ProjectOptions) -> ProjectResult<Project> {
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
    let mut specs: BTreeMap<SpecKey, Spec> = BTreeMap::new();

    let member_roots = if let Some(project) = &root_loaded.manifest.project {
        discover_members(&canonical_root, project, &root_loaded.manifest)?
    } else {
        Vec::new()
    };

    if root_loaded.manifest.package.is_some() {
        specs.insert(
            SpecKey::Path(canonical_root.clone()),
            Spec {
                root: canonical_root.clone(),
                loaded: root_loaded.clone(),
                source: PackageSource::Path,
            },
        );
    }
    for member in member_roots {
        let key = SpecKey::Path(member.clone());
        if specs.contains_key(&key) {
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
            key,
            Spec {
                root: member,
                loaded,
                source: PackageSource::Path,
            },
        );
    }
    if specs.is_empty() {
        return Err(ProjectError::manifest(
            "the project has no packages; add a `[package]` table or list members in `[project]`",
        ));
    }

    resolve_path_dependencies(&canonical_root, &mut specs, &mut warnings)?;
    reject_duplicate_path_names(&specs)?;

    let requirements = registry_requirements(&specs)?;
    if !requirements.is_empty() {
        let existing = load_lockfile(&canonical_root)?;
        let vendor = vendor::load(&canonical_root).map_err(project_registry_error)?;
        let mut locked = existing
            .as_ref()
            .map(locked_preferences)
            .unwrap_or_default();
        if options.update {
            locked.clear();
        } else if !options.update_packages.is_empty() {
            locked.retain(|(name, _), _| {
                !options
                    .update_packages
                    .iter()
                    .any(|updated| updated == name)
            });
        }
        let mut pinned = BTreeMap::new();
        for (name, text) in &options.pinned {
            let version = Version::parse(text).map_err(|error| {
                ProjectError::resolution(format!(
                    "`--precise {name}@{text}` is not a valid version: {error}"
                ))
            })?;
            pinned.insert(name.clone(), version);
        }
        let use_vendor = vendor.is_some() && (options.offline || options.vendor_only);
        if options.vendor_only && vendor.is_none() {
            return Err(ProjectError::offline(format!(
                "`--vendor` requires `{}`",
                vendor::MANIFEST
            )));
        }
        if use_vendor {
            add_lock_specs(
                &canonical_root,
                &mut specs,
                existing.as_ref(),
                vendor.as_ref(),
                &mut warnings,
            )?;
        } else {
            let session = RegistrySession::open(
                &canonical_root,
                root_loaded.manifest.registry.as_ref(),
                options.offline,
            )
            .map_err(project_registry_error)?
            .ok_or_else(|| {
                ProjectError::registry(
                    "registry dependencies need `[registry]` or `SEAL_REGISTRY_URL`",
                )
            })?;
            let resolve_options = ResolveOptions {
                minimal_versions: options.minimal_versions,
                locked,
                pinned,
                ..ResolveOptions::default()
            };
            let resolved = session
                .resolve(&requirements, &resolve_options)
                .map_err(project_registry_error)?;
            let resolved = ensure_yanked_requests(&requirements, &resolved)?;
            for package in &resolved {
                let root = session
                    .materialize(package)
                    .map_err(project_registry_error)?;
                let loaded = session
                    .manifest(package, &root)
                    .map_err(project_registry_error)?;
                warnings.extend(loaded.warnings.clone());
                let key = SpecKey::Registry {
                    name: package.name.clone(),
                    version: package.version.to_string(),
                };
                specs.insert(
                    key,
                    Spec {
                        root,
                        loaded,
                        source: PackageSource::Registry(RegistrySource {
                            registry: session.config().url.to_string(),
                            checksum: package.entry.checksum.clone(),
                            yanked: package.entry.yanked,
                            provenance: package.entry.provenance.clone(),
                            api: package.entry.api.as_ref().map(|api| api.hash.clone()),
                        }),
                    },
                );
            }
        }
    }

    let order = topological_order(&specs)?;
    let mut indexes = BTreeMap::new();
    for (index, key) in order.iter().enumerate() {
        indexes.insert(key.clone(), index);
    }

    let mut packages = Vec::with_capacity(order.len());
    for key in &order {
        let spec = specs
            .get(key)
            .ok_or_else(|| ProjectError::manifest("a package disappeared during resolution"))?;
        let manifest = spec.loaded.manifest.clone();
        let package = manifest.package.as_ref().ok_or_else(|| {
            ProjectError::manifest(format!(
                "the member `{}` has no package",
                spec.root.display()
            ))
        })?;
        let files = source::package_files(&spec.root, &package.sources, &package.exclude)?;
        let targets = targets::build(&spec.root, &manifest, &files)?;
        let dependencies = dependency_edges(key, &spec.root, &manifest, &specs, &indexes)?;
        packages.push(Package {
            name: package.name.clone(),
            version: package.version.clone(),
            root: spec.root.clone(),
            source: spec.source.clone(),
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
        registry: root_loaded.manifest.registry.clone(),
        lockfile: crate::Lockfile::default(),
    };
    project.lockfile = project.resolve_lockfile(host_triple().as_str());
    Ok(project)
}

fn load_lockfile(root: &Path) -> ProjectResult<Option<crate::Lockfile>> {
    let path = root.join("seal.lock");
    if !path.is_file() {
        return Ok(None);
    }
    crate::Lockfile::load(&path).map(Some)
}

fn locked_preferences(lockfile: &crate::Lockfile) -> BTreeMap<(String, Line), Version> {
    let mut locked = BTreeMap::new();
    for package in &lockfile.packages {
        let Some((_, name, version)) = parse_registry_source(&package.source) else {
            continue;
        };
        if let Ok(version) = Version::parse(version) {
            locked
                .entry((name.to_owned(), Line::of(&version)))
                .or_insert(version);
        }
    }
    locked
}

fn parse_registry_source(source: &str) -> Option<(&str, &str, &str)> {
    let rest = source.strip_prefix("registry+")?;
    let (registry, package) = rest.split_once('#')?;
    let (name, version) = package.rsplit_once('@')?;
    Some((registry, name, version))
}

fn add_lock_specs(
    project_root: &Path,
    specs: &mut BTreeMap<SpecKey, Spec>,
    lockfile: Option<&crate::Lockfile>,
    vendor: Option<&VendorManifest>,
    warnings: &mut Vec<ManifestWarning>,
) -> ProjectResult<()> {
    let lockfile = lockfile.ok_or_else(|| {
        ProjectError::offline(
            "an offline vendor build needs `seal.lock`; run `seal build` online first",
        )
    })?;
    let vendor = vendor.ok_or_else(|| {
        ProjectError::offline("the vendor manifest is missing; run `seal vendor`")
    })?;
    let mut candidates: BTreeMap<String, Vec<LockCandidate>> = BTreeMap::new();
    for package in &lockfile.packages {
        let Some((registry, name, version)) = parse_registry_source(&package.source) else {
            continue;
        };
        let version = Version::parse(version).map_err(|error| {
            ProjectError::lockfile(format!(
                "the registry version `{version}` in `seal.lock` is invalid: {error}"
            ))
        })?;
        let root =
            match vendor::materialize(project_root, vendor, name, &version, &package.checksum)
                .map_err(project_registry_error)?
            {
                Some(root) => root,
                None => download::cached_path(
                    &project_root.join("seal").join("registry"),
                    name,
                    &version,
                    &package.checksum,
                ),
            };
        if !root.join("seal.toml").is_file() {
            return Err(ProjectError::offline(format!(
                "`{name}` {version} is not vendored or cached; run `seal vendor` online"
            )));
        }
        download::verify_cached(&root, &package.checksum).map_err(project_registry_error)?;
        let loaded = crate::load_manifest(&root)?;
        let manifest = loaded.manifest.package.as_ref().ok_or_else(|| {
            ProjectError::manifest(format!(
                "the cached package `{name}` {version} has no `[package]` table"
            ))
        })?;
        if manifest.name != name || manifest.version != version.to_string() {
            return Err(ProjectError::metadata(format!(
                "the cached package `{name}` {version} contains `{}` {}",
                manifest.name, manifest.version
            )));
        }
        warnings.extend(loaded.warnings.clone());
        candidates
            .entry(name.to_owned())
            .or_default()
            .push(LockCandidate {
                name: name.to_owned(),
                version,
                checksum: package.checksum.clone(),
                registry: registry.to_owned(),
                root,
                loaded,
                yanked: package.yanked,
                provenance: package.provenance.clone(),
                api: package.api.clone(),
            });
    }

    let mut queue: Vec<SpecKey> = specs.keys().cloned().collect();
    while let Some(key) = queue.pop() {
        let requirements = spec_registry_requirements(
            specs
                .get(&key)
                .ok_or_else(|| ProjectError::manifest("a package is missing from the graph"))?,
        )?;
        for requirement in requirements {
            let selected = candidates
                .get(&requirement.package)
                .and_then(|versions| {
                    versions
                        .iter()
                        .filter(|candidate| requirement.requirement.matches(&candidate.version))
                        .max_by(|left, right| left.version.cmp(&right.version))
                })
                .ok_or_else(|| {
                    ProjectError::offline(format!(
                        "`{}` requires `{}` {}, but no locked version is vendored for it; run `seal update {}` online",
                        requirement.source,
                        requirement.package,
                        requirement.requirement,
                        requirement.package
                    ))
                })?;
            let target = SpecKey::Registry {
                name: selected.name.clone(),
                version: selected.version.to_string(),
            };
            if specs.contains_key(&target) {
                continue;
            }
            let selected_loaded = selected.loaded.clone();
            let selected_root = selected.root.clone();
            let source = PackageSource::Registry(RegistrySource {
                registry: selected.registry.clone(),
                checksum: selected.checksum.clone(),
                yanked: selected.yanked,
                provenance: selected.provenance.clone(),
                api: selected.api.clone(),
            });
            specs.insert(
                target.clone(),
                Spec {
                    root: selected_root,
                    loaded: selected_loaded,
                    source,
                },
            );
            queue.push(target);
        }
    }
    Ok(())
}

fn ensure_yanked_requests(
    requirements: &[Requirement],
    resolved: &[ResolvedPackage],
) -> ProjectResult<Vec<ResolvedPackage>> {
    for requirement in requirements {
        let exact = requirement
            .requirement
            .comparators
            .iter()
            .all(|comparator| comparator.op == semver::Op::Exact)
            && !requirement.requirement.comparators.is_empty();
        if !exact {
            continue;
        }
        if let Some(package) = resolved.iter().find(|package| {
            package.name == requirement.package && requirement.requirement.matches(&package.version)
        }) && package.entry.yanked
        {
            return Err(ProjectError::yanked(format!(
                "`{}` requested `{}` {}, which is yanked",
                requirement.source, requirement.package, requirement.requirement
            )));
        }
    }
    Ok(resolved.to_vec())
}

fn project_registry_error(error: seal_registry::RegistryError) -> ProjectError {
    ProjectError::registry_code(error.code(), error.message())
}

fn registry_requirements(specs: &BTreeMap<SpecKey, Spec>) -> ProjectResult<Vec<Requirement>> {
    let mut requirements = Vec::new();
    for spec in specs.values() {
        requirements.extend(spec_registry_requirements(spec)?);
    }
    Ok(requirements)
}

fn spec_registry_requirements(spec: &Spec) -> ProjectResult<Vec<Requirement>> {
    let source = spec
        .loaded
        .manifest
        .package
        .as_ref()
        .map(|package| format!("{} {}", package.name, package.version))
        .unwrap_or_else(|| spec.root.display().to_string());
    let mut requirements = Vec::new();
    for (alias, dependency) in &spec.loaded.manifest.dependencies {
        let (package, requirement) = match dependency {
            DependencySpec::Version(requirement) => (alias.clone(), requirement.clone()),
            DependencySpec::Detailed(detail) => {
                if detail.git.is_some() {
                    return Err(ProjectError::dependency(format!(
                        "the dependency `{alias}` uses git; Seal supports path and registry dependencies"
                    )));
                }
                if detail.path.is_some() {
                    continue;
                }
                let Some(requirement) = &detail.version else {
                    return Err(ProjectError::dependency_declaration(format!(
                        "the dependency `{alias}` has neither a path nor a version"
                    )));
                };
                (
                    detail.package.clone().unwrap_or_else(|| alias.clone()),
                    requirement.clone(),
                )
            }
        };
        requirements.push(
            Requirement::parse(package, alias.clone(), &requirement, source.clone())
                .map_err(project_registry_error)?,
        );
    }
    Ok(requirements)
}

fn reject_duplicate_path_names(specs: &BTreeMap<SpecKey, Spec>) -> ProjectResult<()> {
    let mut names = BTreeMap::new();
    for (key, spec) in specs {
        let SpecKey::Path(_) = key else {
            continue;
        };
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
    Ok(())
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
                if matches!(
                    name.to_str(),
                    Some(".git" | "target" | "node_modules" | "seal" | "vendor")
                )
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
    specs: &mut BTreeMap<SpecKey, Spec>,
    warnings: &mut Vec<ManifestWarning>,
) -> ProjectResult<()> {
    let mut queue: Vec<PathBuf> = specs
        .keys()
        .filter_map(|key| match key {
            SpecKey::Path(root) => Some(root.clone()),
            SpecKey::Registry { .. } => None,
        })
        .collect();
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
            let key = SpecKey::Path(canonical.clone());
            if !specs.contains_key(&key) {
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
                    key.clone(),
                    Spec {
                        root: canonical,
                        loaded,
                        source: PackageSource::Path,
                    },
                );
            }
            if let Some(requirement) = requirement {
                let dependency = specs
                    .get(&key)
                    .and_then(|spec| spec.loaded.manifest.package.as_ref())
                    .ok_or_else(|| ProjectError::dependency("a dependency has no package"))?;
                let version = Version::parse(&dependency.version).map_err(|error| {
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
    specs: &BTreeMap<SpecKey, Spec>,
) -> ProjectResult<Vec<(String, PathBuf, Option<VersionReq>)>> {
    let key = SpecKey::Path(root.to_path_buf());
    let spec = specs
        .get(&key)
        .ok_or_else(|| ProjectError::manifest("a package is missing from the graph"))?;
    let mut dependencies = Vec::new();
    for (alias, dependency) in &spec.loaded.manifest.dependencies {
        let DependencySpec::Detailed(detail) = dependency else {
            continue;
        };
        if detail.git.is_some() {
            return Err(ProjectError::dependency(format!(
                "the dependency `{alias}` uses git; Seal supports path and registry dependencies"
            )));
        }
        let Some(path) = &detail.path else {
            continue;
        };
        let requirement = detail
            .version
            .as_ref()
            .map(|version| {
                VersionReq::parse(version).map_err(|error| {
                    ProjectError::dependency(format!(
                        "the dependency `{alias}` has an invalid version requirement: {error}"
                    ))
                })
            })
            .transpose()?;
        dependencies.push((alias.clone(), root.join(path), requirement));
    }
    Ok(dependencies)
}

fn dependency_edges(
    current: &SpecKey,
    root: &Path,
    manifest: &Manifest,
    specs: &BTreeMap<SpecKey, Spec>,
    indexes: &BTreeMap<SpecKey, usize>,
) -> ProjectResult<Vec<Dependency>> {
    let mut dependencies = Vec::new();
    for (alias, dependency) in &manifest.dependencies {
        let detail = match dependency {
            DependencySpec::Version(requirement) => {
                let key = select_registry_key(specs, alias, requirement)?;
                let package = *indexes.get(&key).ok_or_else(|| {
                    ProjectError::dependency(format!(
                        "the dependency `{alias}` has no package index"
                    ))
                })?;
                dependencies.push(Dependency {
                    alias: alias.clone(),
                    package,
                    features: Vec::new(),
                    default_features: true,
                    optional: false,
                    when: None,
                });
                continue;
            }
            DependencySpec::Detailed(detail) => detail,
        };
        if detail.git.is_some() {
            return Err(ProjectError::dependency(format!(
                "the dependency `{alias}` uses git; Seal supports path and registry dependencies"
            )));
        }
        let key = if let Some(path) = &detail.path {
            let canonical = root.join(path).canonicalize().map_err(|error| {
                ProjectError::path_dependency(format!(
                    "the dependency `{alias}` cannot be resolved: {error}"
                ))
            })?;
            SpecKey::Path(canonical)
        } else {
            let package = detail.package.as_deref().unwrap_or(alias);
            let requirement = detail.version.as_deref().ok_or_else(|| {
                ProjectError::dependency_declaration(format!(
                    "the dependency `{alias}` has neither a path nor a version"
                ))
            })?;
            select_registry_key(specs, package, requirement)?
        };
        if !specs.contains_key(&key) {
            return Err(ProjectError::dependency(format!(
                "the dependency `{alias}` ({current}) was not loaded"
            )));
        }
        let package = *indexes.get(&key).ok_or_else(|| {
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

fn select_registry_key(
    specs: &BTreeMap<SpecKey, Spec>,
    name: &str,
    requirement: &str,
) -> ProjectResult<SpecKey> {
    let requirement = VersionReq::parse(requirement).map_err(|error| {
        ProjectError::dependency(format!(
            "the dependency `{name}` has an invalid version requirement: {error}"
        ))
    })?;
    specs
        .keys()
        .filter_map(|key| match key {
            SpecKey::Registry {
                name: candidate,
                version,
            } if candidate == name => {
                let version = Version::parse(version).ok()?;
                requirement
                    .matches(&version)
                    .then_some((key.clone(), version))
            }
            _ => None,
        })
        .max_by(|left, right| left.1.cmp(&right.1))
        .map(|(key, _)| key)
        .ok_or_else(|| {
            ProjectError::resolution(format!(
                "no resolved or locked version of `{name}` satisfies `{requirement}`"
            ))
        })
}

fn topological_order(specs: &BTreeMap<SpecKey, Spec>) -> ProjectResult<Vec<SpecKey>> {
    let mut states: BTreeMap<SpecKey, u8> = BTreeMap::new();
    let mut stack = Vec::new();
    let mut order = Vec::new();
    for key in specs.keys() {
        visit(key, specs, &mut states, &mut stack, &mut order)?;
    }
    Ok(order)
}

fn visit(
    key: &SpecKey,
    specs: &BTreeMap<SpecKey, Spec>,
    states: &mut BTreeMap<SpecKey, u8>,
    stack: &mut Vec<SpecKey>,
    order: &mut Vec<SpecKey>,
) -> ProjectResult<()> {
    match states.get(key).copied() {
        Some(2) => return Ok(()),
        Some(1) => {
            let start = stack.iter().position(|entry| entry == key).unwrap_or(0);
            let mut cycle: Vec<String> = stack[start..].iter().map(ToString::to_string).collect();
            cycle.push(key.to_string());
            return Err(ProjectError::cycle(format!(
                "the dependency graph contains a cycle: {}",
                cycle.join(" -> ")
            )));
        }
        _ => {}
    }
    states.insert(key.clone(), 1);
    stack.push(key.clone());
    let spec = specs
        .get(key)
        .ok_or_else(|| ProjectError::manifest("a package is missing from the graph"))?;
    let root = spec.root.clone();
    let manifest = spec.loaded.manifest.clone();
    for (alias, dependency) in &manifest.dependencies {
        let dependency_key = match dependency {
            DependencySpec::Version(requirement) => select_registry_key(specs, alias, requirement)?,
            DependencySpec::Detailed(detail) => {
                if detail.git.is_some() {
                    return Err(ProjectError::dependency(
                        "the graph contains a git dependency",
                    ));
                }
                if let Some(path) = &detail.path {
                    let canonical = root.join(path).canonicalize().map_err(|error| {
                        ProjectError::path_dependency(format!(
                            "cannot resolve `{}`: {error}",
                            path.display()
                        ))
                    })?;
                    SpecKey::Path(canonical)
                } else {
                    let package = detail.package.as_deref().unwrap_or(alias);
                    let requirement = detail.version.as_deref().ok_or_else(|| {
                        ProjectError::dependency_declaration("a registry dependency has no version")
                    })?;
                    select_registry_key(specs, package, requirement)?
                }
            }
        };
        visit(&dependency_key, specs, states, stack, order)?;
    }
    stack.pop();
    states.insert(key.clone(), 2);
    order.push(key.clone());
    Ok(())
}
