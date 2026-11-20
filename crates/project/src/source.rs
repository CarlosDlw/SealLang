//! Deterministic source discovery.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use walkdir::WalkDir;

use crate::error::{ProjectError, ProjectResult};

/// Discovers the base source files of a package.
pub(crate) fn package_files(
    root: &Path,
    sources: &[String],
    exclude: &[String],
) -> ProjectResult<Vec<PathBuf>> {
    let patterns = if sources.is_empty() {
        vec!["src/**/*.sl".to_owned()]
    } else {
        sources.to_vec()
    };
    discover(root, &patterns, exclude, "source")
}

/// Discovers files matching target globs.
pub(crate) fn target_files(
    root: &Path,
    sources: &[String],
    exclude: &[String],
) -> ProjectResult<Vec<PathBuf>> {
    discover(root, sources, exclude, "target source")
}

/// Resolves one relative path inside a package, rejecting escapes and symlinks.
pub(crate) fn inside(root: &Path, path: &Path) -> ProjectResult<PathBuf> {
    let canonical_root = root.canonicalize().map_err(|error| {
        ProjectError::io(format!("cannot resolve `{}`: {error}", root.display()))
    })?;
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let canonical = joined.canonicalize().map_err(|error| {
        ProjectError::source(format!("cannot resolve `{}`: {error}", joined.display()))
    })?;
    if !canonical.starts_with(&canonical_root) {
        return Err(ProjectError::source(format!(
            "`{}` resolves outside the package root `{}`",
            path.display(),
            canonical_root.display()
        )));
    }
    if !canonical.is_file() {
        return Err(ProjectError::source(format!(
            "`{}` is not a file",
            canonical.display()
        )));
    }
    Ok(canonical)
}

fn discover(
    root: &Path,
    patterns: &[String],
    exclude: &[String],
    kind: &str,
) -> ProjectResult<Vec<PathBuf>> {
    let canonical_root = root.canonicalize().map_err(|error| {
        ProjectError::io(format!("cannot resolve `{}`: {error}", root.display()))
    })?;
    let excludes = build_set(exclude, "exclude")?;
    let mut files = BTreeSet::new();
    for pattern in patterns {
        validate_pattern(pattern)?;
        let glob = Glob::new(pattern).map_err(|error| {
            ProjectError::manifest(format!("invalid {kind} glob `{pattern}`: {error}"))
        })?;
        let matcher = glob.compile_matcher();
        let mut matched = false;
        for entry in WalkDir::new(&canonical_root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !skip(entry.path(), &canonical_root))
        {
            let entry = entry.map_err(|error| {
                ProjectError::source(format!(
                    "cannot walk `{}`: {error}",
                    canonical_root.display()
                ))
            })?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("sl") {
                continue;
            }
            let relative = relative_slash(path, &canonical_root)?;
            if matcher.is_match(&relative) && !excludes.is_match(&relative) {
                let canonical = path.canonicalize().map_err(|error| {
                    ProjectError::source(format!("cannot resolve `{}`: {error}", path.display()))
                })?;
                if !canonical.starts_with(&canonical_root) {
                    return Err(ProjectError::source(format!(
                        "`{}` resolves outside the package root",
                        path.display()
                    )));
                }
                files.insert(canonical);
                matched = true;
            }
        }
        if !matched {
            return Err(ProjectError::source(format!(
                "the {kind} glob `{pattern}` matched no `.sl` files under `{}`",
                canonical_root.display()
            )));
        }
    }
    if files.is_empty() {
        return Err(ProjectError::source(format!(
            "the package `{}` has no source files",
            canonical_root.display()
        )));
    }
    Ok(files.into_iter().collect())
}

fn build_set(patterns: &[String], kind: &str) -> ProjectResult<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        validate_pattern(pattern)?;
        builder.add(Glob::new(pattern).map_err(|error| {
            ProjectError::manifest(format!("invalid {kind} glob `{pattern}`: {error}"))
        })?);
    }
    builder
        .build()
        .map_err(|error| ProjectError::manifest(format!("invalid {kind} globs: {error}")))
}

fn validate_pattern(pattern: &str) -> ProjectResult<()> {
    if pattern.is_empty() {
        return Err(ProjectError::manifest("a source glob cannot be empty"));
    }
    let path = Path::new(pattern);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(ProjectError::manifest(format!(
            "the source glob `{pattern}` must stay inside the package"
        )));
    }
    Ok(())
}

fn relative_slash(path: &Path, root: &Path) -> ProjectResult<String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        ProjectError::source(format!(
            "`{}` is not inside `{}`",
            path.display(),
            root.display()
        ))
    })?;
    let mut text = String::new();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err(ProjectError::source(format!(
                "`{}` has an unsupported path component",
                relative.display()
            )));
        };
        if !text.is_empty() {
            text.push('/');
        }
        text.push_str(&segment.to_string_lossy());
    }
    Ok(text)
}

fn skip(path: &Path, root: &Path) -> bool {
    if path == root {
        return false;
    }
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    relative.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        matches!(
            name.to_str(),
            Some(".git" | "target" | "node_modules" | "seal" | ".seal")
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glob_cannot_escape_the_package() {
        let error = validate_pattern("../src/*.sl").unwrap_err();
        assert_eq!(error.code(), "E8005");
    }

    #[test]
    fn an_empty_glob_is_rejected() {
        assert!(validate_pattern("").is_err());
    }

    #[test]
    fn relative_paths_use_forward_slashes() {
        let root = Path::new("/tmp/project");
        let path = root.join("src").join("a").join("b.sl");
        assert_eq!(relative_slash(&path, root).unwrap(), "src/a/b.sl");
    }
}
