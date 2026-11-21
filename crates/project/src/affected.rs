//! Mapping version-control changes to affected packages and targets.
//!
//! A change affects a package when it touches one of the package's source
//! files, its manifest, the lockfile, the toolchain, or a generated input. A
//! dependent package is affected transitively because its compilation consumes
//! the changed package. The result is deterministic and contains no absolute
//! paths, so CI can compare reports across machines.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::error::{ProjectError, ProjectResult};
use crate::{Project, TargetKind};

/// One affected build target.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct AffectedTarget {
    /// The package index in the loaded project.
    pub package: usize,
    /// The package name.
    pub package_name: String,
    /// The target name.
    pub target: String,
    /// The target kind.
    pub kind: String,
}

/// The complete affected report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AffectedReport {
    /// The revision the diff was taken from.
    pub since: String,
    /// Changed paths, relative to the project root and sorted.
    pub changed: Vec<String>,
    /// Affected packages, sorted by index.
    pub packages: Vec<String>,
    /// Affected targets, sorted by package and name.
    pub targets: Vec<AffectedTarget>,
}

impl AffectedReport {
    /// Returns whether the report contains no affected target.
    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }
}

/// Computes the affected packages and targets since `revision`.
pub fn affected(project: &Project, revision: &str) -> ProjectResult<AffectedReport> {
    let changed = changed_paths(&project.root, revision)?;
    Ok(affected_from_paths(project, revision, &changed))
}

/// Computes a report for an explicit changed-path list.
///
/// This is the testable core of [`affected`]; it does not invoke Git, so tests
/// cover the mapping rules without a repository.
pub fn affected_from_paths(
    project: &Project,
    revision: &str,
    changed: &[PathBuf],
) -> AffectedReport {
    let mut packages = BTreeSet::new();
    let mut targets = BTreeSet::new();
    let mut changed_names: Vec<String> = changed
        .iter()
        .map(|path| relative_path(&project.root, path))
        .filter(|path| !Path::new(path).is_absolute() && !path.split('/').any(|part| part == ".."))
        .collect();
    let global = changed
        .iter()
        .any(|path| is_global_change(&project.root, path));
    if global {
        for index in 0..project.packages.len() {
            packages.insert(index);
            add_all_targets(project, index, &mut targets);
        }
    } else {
        for path in changed {
            let relative = relative_path(&project.root, path);
            for (index, package) in project.packages.iter().enumerate() {
                let package_root = relative_path(&project.root, &package.root);
                if !inside(&relative, &package_root) {
                    continue;
                }
                let mut matched = false;
                for target in &package.targets {
                    if target
                        .files
                        .iter()
                        .any(|file| relative_path(&project.root, file) == relative)
                        || target
                            .generator_inputs
                            .iter()
                            .any(|file| relative_path(&project.root, file) == relative)
                        || target
                            .generator_outputs
                            .iter()
                            .any(|output| matches_output(&package_root, output, &relative))
                    {
                        packages.insert(index);
                        targets.insert((index, target.name.clone()));
                        matched = true;
                    }
                }
                if !matched
                    && (relative == join_relative(&package_root, "seal.toml")
                        || relative.starts_with(&format!("{package_root}/"))
                        || package
                            .files
                            .iter()
                            .any(|file| relative_path(&project.root, file) == relative))
                {
                    packages.insert(index);
                    add_all_targets(project, index, &mut targets);
                }
            }
        }
    }

    // A package that depends on an affected package must be rebuilt too. The
    // project load already rejects dependency cycles, so a simple worklist is
    // enough.
    let mut changed_packages = packages.clone();
    loop {
        let mut grew = false;
        for (index, package) in project.packages.iter().enumerate() {
            if changed_packages.contains(&index) {
                continue;
            }
            if package
                .dependencies
                .iter()
                .any(|dependency| changed_packages.contains(&dependency.package))
            {
                changed_packages.insert(index);
                add_all_targets(project, index, &mut targets);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    packages.extend(changed_packages.iter().copied());

    // A generator and every target that consumes its output form a small
    // target graph inside one package. Close over it so changing a generator
    // input rebuilds both the generator and its consumers.
    loop {
        let mut grew = false;
        let current: Vec<(usize, String)> = targets.iter().cloned().collect();
        for (package, name) in current {
            for target in &project.package(package).targets {
                if target.generated.iter().any(|generator| generator == &name)
                    && targets.insert((package, target.name.clone()))
                {
                    grew = true;
                }
            }
            if let Some(generator) = project
                .package(package)
                .target(&name)
                .filter(|target| target.kind == TargetKind::Generator)
            {
                let package_root = relative_path(&project.root, &project.package(package).root);
                for output in &generator.generator_outputs {
                    for target in &project.package(package).targets {
                        if target.files.iter().any(|file| {
                            matches_output(
                                &package_root,
                                output,
                                &relative_path(&project.root, file),
                            )
                        }) && targets.insert((package, target.name.clone()))
                        {
                            grew = true;
                        }
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }

    changed_names.sort();
    changed_names.dedup();
    let packages: Vec<String> = packages
        .iter()
        .map(|index| project.package(*index).name.clone())
        .collect();
    let targets: Vec<AffectedTarget> = targets
        .into_iter()
        .map(|(package, target)| {
            let entry = project
                .package(package)
                .target(&target)
                .expect("affected targets come from the package target list");
            AffectedTarget {
                package,
                package_name: project.package(package).name.clone(),
                target,
                kind: entry.kind.as_str().to_owned(),
            }
        })
        .collect();
    AffectedReport {
        since: revision.to_owned(),
        changed: changed_names,
        packages,
        targets,
    }
}

fn add_all_targets(project: &Project, index: usize, targets: &mut BTreeSet<(usize, String)>) {
    for target in &project.package(index).targets {
        targets.insert((index, target.name.clone()));
    }
}

fn is_global_change(root: &Path, path: &Path) -> bool {
    let relative = relative_path(root, path);
    relative == "seal.lock"
        || relative == "seal.toml"
        || relative == "seal-toolchain.toml"
        || relative.starts_with("seal/targets/")
        || relative == "seal/toolchain.toml"
}

fn inside(path: &str, root: &str) -> bool {
    root.is_empty() || root == "." || path == root || path.starts_with(&format!("{root}/"))
}

fn matches_output(package_root: &str, pattern: &str, relative: &str) -> bool {
    let relative = if package_root.is_empty() || package_root == "." {
        relative
    } else if let Some(relative) = relative.strip_prefix(&format!("{package_root}/")) {
        relative
    } else {
        return false;
    };
    globset::Glob::new(pattern)
        .map(|glob| glob.compile_matcher().is_match(relative))
        .unwrap_or(false)
}

fn join_relative(root: &str, path: &str) -> String {
    let path = path.replace('\\', "/");
    if root.is_empty() || root == "." || path == root || path.starts_with(&format!("{root}/")) {
        path
    } else {
        format!("{root}/{path}")
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    let path: PathBuf = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let path = normalize(&path);
    let root = normalize(root);
    let relative = path
        .strip_prefix(&root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    relative.trim_start_matches("./").to_owned()
}

fn normalize(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

fn changed_paths(root: &Path, revision: &str) -> ProjectResult<Vec<PathBuf>> {
    if revision.trim().is_empty() || revision.contains('\0') {
        return Err(ProjectError::io("`--since` needs a non-empty revision"));
    }
    if revision.starts_with('-') {
        return Err(ProjectError::io(
            "`--since` must name a revision, not a Git option",
        ));
    }
    let tracked = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("diff")
        .arg("--name-only")
        .arg("--no-renames")
        .arg("-z")
        .arg(revision)
        .arg("--")
        .output()
        .map_err(|error| ProjectError::io(format!("cannot run Git: {error}")))?;
    if !tracked.status.success() {
        let message = String::from_utf8_lossy(&tracked.stderr);
        return Err(ProjectError::io(format!(
            "`git diff {revision}` failed: {}",
            message.trim()
        )));
    }
    let untracked = Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("ls-files")
        .arg("--others")
        .arg("--exclude-standard")
        .arg("-z")
        .output()
        .map_err(|error| ProjectError::io(format!("cannot run Git: {error}")))?;
    if !untracked.status.success() {
        let message = String::from_utf8_lossy(&untracked.stderr);
        return Err(ProjectError::io(format!(
            "`git ls-files --others` failed: {}",
            message.trim()
        )));
    }
    let mut paths: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut add = |bytes: &[u8]| {
        for entry in bytes.split(|byte| *byte == 0) {
            if entry.is_empty() {
                continue;
            }
            let path = String::from_utf8_lossy(entry).into_owned();
            paths.insert(path.clone(), root.join(path));
        }
    };
    add(&tracked.stdout);
    add(&untracked.stdout);
    Ok(paths.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_are_normalized_and_stay_inside_the_root() {
        assert_eq!(
            relative_path(Path::new("/project"), Path::new("/project/src/./main.sl")),
            "src/main.sl"
        );
        assert_eq!(
            relative_path(
                Path::new("/project"),
                Path::new("/project/src/../seal.toml")
            ),
            "seal.toml"
        );
    }

    #[test]
    fn a_package_root_contains_only_its_own_paths() {
        assert!(inside("app/src/main.sl", "app"));
        assert!(!inside("application/src/main.sl", "app"));
        assert!(inside("src/main.sl", "."));
    }

    #[test]
    fn generator_output_patterns_match_only_inside_the_package() {
        assert!(matches_output(
            "crates/app",
            "generated/*.sl",
            "crates/app/generated/table.sl"
        ));
        assert!(!matches_output(
            "crates/app",
            "generated/*.sl",
            "crates/other/generated/table.sl"
        ));
        assert!(!matches_output(
            "crates/app",
            "generated/*.sl",
            "crates/app/src/lib.sl"
        ));
    }
}
