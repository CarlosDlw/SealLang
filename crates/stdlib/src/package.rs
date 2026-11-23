//! Standard-library package metadata and source discovery.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use semver::Version;
use walkdir::WalkDir;

use crate::descriptor::Descriptor;
use crate::error::{StdlibError, StdlibResult};

/// One internal dependency edge of a standard-library package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dependency {
    /// The name imports use.
    pub alias: String,
    /// The target package name.
    pub package: String,
}

/// One validated standard-library package.
#[derive(Clone, Debug)]
pub struct Package {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Compatibility edition selected by the package manifest.
    pub edition: seal_version::Edition,
    /// Canonical package directory.
    pub root: PathBuf,
    /// Source files, sorted by relative path.
    pub files: Vec<PathBuf>,
    /// Modules exposed by the package API.
    pub exposed: Vec<String>,
    /// The prelude module selected by the descriptor, if any.
    pub prelude: Option<String>,
    /// Internal dependency edges.
    pub dependencies: Vec<Dependency>,
    /// The validated package manifest.
    pub manifest: seal_manifest::Manifest,
}

/// Loads every package named by `descriptor`.
pub(crate) fn load_all(root: &Path, descriptor: &Descriptor) -> StdlibResult<Vec<Package>> {
    let mut roots = BTreeMap::new();
    let mut manifests = BTreeMap::new();
    for name in &descriptor.packages {
        let package_root = safe_join(root, Path::new(name))?;
        let metadata = std::fs::symlink_metadata(&package_root).map_err(|error| {
            StdlibError::package(format!(
                "the standard-library package directory `{}` is unreadable: {error}",
                package_root.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(StdlibError::package(format!(
                "the standard-library package `{name}` is a symlink, which is not allowed"
            )));
        }
        if !metadata.is_dir() {
            return Err(StdlibError::package(format!(
                "the standard-library package `{name}` is not a directory"
            )));
        }
        let canonical = package_root.canonicalize().map_err(|error| {
            StdlibError::package(format!(
                "cannot resolve the standard-library package `{name}`: {error}"
            ))
        })?;
        if !canonical.starts_with(root) {
            return Err(StdlibError::package(format!(
                "the standard-library package `{name}` escapes the standard-library root"
            )));
        }
        let loaded = seal_manifest::load(&canonical.join("seal.toml")).map_err(|error| {
            StdlibError::package(format!(
                "the standard-library package `{name}` has an invalid manifest: {error}"
            ))
        })?;
        let package = loaded.manifest.package.as_ref().ok_or_else(|| {
            StdlibError::package(format!(
                "the standard-library package `{name}` has no `[package]` table"
            ))
        })?;
        if package.name != *name {
            return Err(StdlibError::package(format!(
                "the standard-library directory `{name}` declares the package `{}`",
                package.name
            )));
        }
        Version::parse(&package.version).map_err(|error| {
            StdlibError::package(format!(
                "the standard-library package `{name}` has a non-semantic version: {error}"
            ))
        })?;
        if let Some(edition) = &package.edition {
            seal_version::Edition::parse(edition).map_err(|error| {
                StdlibError::package(format!(
                    "the standard-library package `{name}` declares the edition `{edition}`, which is invalid: {error}"
                ))
            })?;
        }
        roots.insert(name.clone(), canonical);
        manifests.insert(name.clone(), loaded.manifest);
    }

    let mut packages = Vec::with_capacity(descriptor.packages.len());
    for name in &descriptor.packages {
        let package_root = roots
            .get(name)
            .cloned()
            .ok_or_else(|| StdlibError::package(format!("the package `{name}` disappeared")))?;
        let manifest = manifests
            .get(name)
            .cloned()
            .ok_or_else(|| StdlibError::package(format!("the package `{name}` disappeared")))?;
        let package = manifest.package.as_ref().expect("package was checked");
        let files = source_files(&package_root, &manifest, name)?;
        let exposed = package
            .api
            .as_ref()
            .map(|api| api.expose.clone())
            .filter(|expose| !expose.is_empty())
            .unwrap_or_else(|| vec!["root".to_owned()]);
        let dependencies = internal_dependencies(name, &package_root, &manifest, &roots, root)?;
        let edition = match package.edition.as_deref() {
            Some(text) => seal_version::Edition::parse(text).map_err(|error| {
                StdlibError::package(format!(
                    "the standard-library package `{name}` declares the edition `{text}`, which is invalid: {error}"
                ))
            })?,
            None => seal_version::Edition::CURRENT,
        };
        packages.push(Package {
            name: name.clone(),
            version: package.version.clone(),
            edition,
            root: package_root,
            files,
            exposed,
            prelude: descriptor.prelude_for(name).map(str::to_owned),
            dependencies,
            manifest,
        });
    }
    reject_cycles(&packages)?;
    Ok(packages)
}

fn internal_dependencies(
    package_name: &str,
    root: &Path,
    manifest: &seal_manifest::Manifest,
    roots: &BTreeMap<String, PathBuf>,
    stdlib_root: &Path,
) -> StdlibResult<Vec<Dependency>> {
    let mut dependencies = Vec::new();
    for (alias, spec) in &manifest.dependencies {
        let seal_manifest::DependencySpec::Detailed(detailed) = spec else {
            return Err(StdlibError::dependencies(format!(
                "the standard-library package `{package_name}` uses the non-path dependency `{alias}`"
            )));
        };
        if detailed.git.is_some()
            || detailed.registry.is_some()
            || detailed.version.is_some()
            || detailed.optional
            || detailed.when.is_some()
            || !detailed.features.is_empty()
        {
            return Err(StdlibError::dependencies(format!(
                "the standard-library package `{package_name}` uses unsupported dependency options for `{alias}`"
            )));
        }
        let Some(path) = &detailed.path else {
            return Err(StdlibError::dependencies(format!(
                "the standard-library dependency `{alias}` in `{package_name}` has no path"
            )));
        };
        let target = detailed.package.as_deref().unwrap_or(alias);
        let target_root = roots.get(target).ok_or_else(|| {
            StdlibError::dependencies(format!(
                "the standard-library package `{package_name}` depends on `{target}`, which the descriptor does not list"
            ))
        })?;
        // A dependency path is relative to the package directory and may leave
        // it (`../core`), but it must never leave the standard-library root.
        let resolved = join_inside(root, path)?;
        let canonical = resolved.canonicalize().map_err(|error| {
            StdlibError::dependencies(format!(
                "the standard-library dependency `{alias}` in `{package_name}` cannot be resolved: {error}"
            ))
        })?;
        if !canonical.starts_with(stdlib_root) {
            return Err(StdlibError::dependencies(format!(
                "the standard-library dependency `{alias}` in `{package_name}` escapes the standard-library root"
            )));
        }
        if canonical != *target_root {
            return Err(StdlibError::dependencies(format!(
                "the standard-library dependency `{alias}` in `{package_name}` does not point at `{target}`"
            )));
        }
        dependencies.push(Dependency {
            alias: alias.clone(),
            package: target.to_owned(),
        });
    }
    Ok(dependencies)
}

fn source_files(
    root: &Path,
    manifest: &seal_manifest::Manifest,
    package: &str,
) -> StdlibResult<Vec<PathBuf>> {
    let package_manifest = manifest.package.as_ref().expect("package was checked");
    let patterns = if package_manifest.sources.is_empty() {
        vec!["src/**/*.sl".to_owned()]
    } else {
        package_manifest.sources.clone()
    };
    let sources = glob_set(&patterns, package, "source")?;
    let excludes = glob_set(&package_manifest.exclude, package, "exclude")?;
    let matchers = patterns
        .iter()
        .map(|pattern| {
            Glob::new(pattern)
                .map(|glob| glob.compile_matcher())
                .map_err(|error| {
                    StdlibError::package(format!(
                        "the standard-library source glob `{pattern}` in `{package}` is invalid: {error}"
                    ))
                })
        })
        .collect::<StdlibResult<Vec<_>>>()?;
    let mut files = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| {
            StdlibError::package(format!(
                "cannot walk the standard-library package `{package}`: {error}"
            ))
        })?;
        let relative = entry.path().strip_prefix(root).map_err(|_| {
            StdlibError::package(format!(
                "a path under the standard-library package `{package}` escaped its root"
            ))
        })?;
        let relative_text = slash_path(relative);
        if !sources.is_match(&relative_text) || excludes.is_match(&relative_text) {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err(StdlibError::package(format!(
                "the standard-library source `{}` in `{package}` is a symlink, which is not allowed",
                entry.path().display()
            )));
        }
        if !entry.file_type().is_file() {
            return Err(StdlibError::package(format!(
                "the standard-library source `{}` in `{package}` is not a regular file",
                entry.path().display()
            )));
        }
        let canonical = entry.path().canonicalize().map_err(|error| {
            StdlibError::package(format!(
                "cannot resolve the standard-library source `{}`: {error}",
                entry.path().display()
            ))
        })?;
        if !canonical.starts_with(root) {
            return Err(StdlibError::package(format!(
                "the standard-library source `{}` in `{package}` escapes its package root",
                entry.path().display()
            )));
        }
        seal_fs::load(&canonical, seal_fs::DEFAULT_MAX_FILE_SIZE).map_err(|error| {
            StdlibError::package(format!(
                "the standard-library source `{}` is invalid: {error}",
                entry.path().display()
            ))
        })?;
        files.push(canonical);
    }
    files.sort();
    for (pattern, matcher) in patterns.iter().zip(&matchers) {
        let matched = files.iter().any(|file| {
            let relative = file.strip_prefix(root).unwrap_or(file);
            matcher.is_match(slash_path(relative))
        });
        if !matched {
            return Err(StdlibError::package(format!(
                "the standard-library source glob `{pattern}` in `{package}` matches no file"
            )));
        }
    }
    if files.is_empty() {
        return Err(StdlibError::package(format!(
            "the standard-library package `{package}` has no source files"
        )));
    }
    Ok(files)
}

fn glob_set(patterns: &[String], package: &str, kind: &str) -> StdlibResult<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        validate_glob(pattern, package, kind)?;
        builder.add(Glob::new(pattern).map_err(|error| {
            StdlibError::package(format!(
                "the standard-library {kind} glob `{pattern}` in `{package}` is invalid: {error}"
            ))
        })?);
    }
    builder.build().map_err(|error| {
        StdlibError::package(format!(
            "the standard-library {kind} globs in `{package}` are invalid: {error}"
        ))
    })
}

fn validate_glob(pattern: &str, package: &str, kind: &str) -> StdlibResult<()> {
    let path = Path::new(pattern);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(StdlibError::package(format!(
            "the standard-library {kind} glob `{pattern}` in `{package}` must stay inside the package"
        )));
    }
    Ok(())
}

fn safe_join(root: &Path, path: &Path) -> StdlibResult<PathBuf> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(StdlibError::package(format!(
            "the standard-library path `{}` must stay inside its root",
            path.display()
        )));
    }
    Ok(root.join(path))
}

/// Joins a dependency path, allowing `..` while rejecting absolute paths.
///
/// The caller canonicalizes the result and checks that it stays inside the
/// standard-library root; lexical normalization alone is not a security
/// boundary because symlinks resolve during canonicalization.
fn join_inside(root: &Path, path: &Path) -> StdlibResult<PathBuf> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::RootDir | Component::Prefix(_)))
    {
        return Err(StdlibError::dependencies(format!(
            "the standard-library dependency path `{}` must be relative",
            path.display()
        )));
    }
    Ok(root.join(path))
}

fn reject_cycles(packages: &[Package]) -> StdlibResult<()> {
    let indexes: BTreeMap<&str, usize> = packages
        .iter()
        .enumerate()
        .map(|(index, package)| (package.name.as_str(), index))
        .collect();
    let mut state = vec![0u8; packages.len()];
    for index in 0..packages.len() {
        visit(index, packages, &indexes, &mut state, &mut Vec::new())?;
    }
    Ok(())
}

fn visit(
    index: usize,
    packages: &[Package],
    indexes: &BTreeMap<&str, usize>,
    state: &mut [u8],
    stack: &mut Vec<String>,
) -> StdlibResult<()> {
    match state[index] {
        1 => {
            return Err(StdlibError::dependencies(format!(
                "the standard-library dependency graph has a cycle through `{}`",
                stack.join(" -> ")
            )));
        }
        2 => return Ok(()),
        _ => {}
    }
    state[index] = 1;
    stack.push(packages[index].name.clone());
    for dependency in &packages[index].dependencies {
        let target = indexes.get(dependency.package.as_str()).ok_or_else(|| {
            StdlibError::dependencies(format!(
                "the standard-library package `{}` depends on the unknown package `{}`",
                packages[index].name, dependency.package
            ))
        })?;
        visit(*target, packages, indexes, state, stack)?;
    }
    stack.pop();
    state[index] = 2;
    Ok(())
}

fn slash_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
