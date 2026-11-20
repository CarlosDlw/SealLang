//! The versioned, deterministic path-dependency lockfile.

use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::Project;
use crate::error::{ProjectError, ProjectResult};

/// One package recorded in `seal.lock`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockPackage {
    /// The package name.
    pub name: String,
    /// The package version.
    pub version: String,
    /// The source, always a relative `path+...` for this phase.
    pub source: String,
    /// A checksum over the package's source files.
    pub checksum: String,
    /// The API hash, once API snapshots are recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// The resolved dependency edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<LockDependency>,
}

/// One dependency edge in the lockfile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockDependency {
    /// The dependency package name.
    pub name: String,
    /// The dependency version.
    pub version: String,
}

/// The complete lockfile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lockfile {
    /// The lockfile schema version.
    pub version: u32,
    /// The compiler that produced the lockfile.
    pub toolchain: String,
    /// The targets this lockfile has been resolved for.
    pub targets: Vec<String>,
    /// Packages sorted by name.
    #[serde(default, rename = "package")]
    pub packages: Vec<LockPackage>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self {
            version: 1,
            toolchain: "seal 0.1.0".to_owned(),
            targets: Vec::new(),
            packages: Vec::new(),
        }
    }
}

impl Lockfile {
    /// Reads a lockfile, rejecting unknown schema versions.
    pub fn load(path: &Path) -> ProjectResult<Self> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            ProjectError::lockfile(format!("cannot read `{}`: {error}", path.display()))
        })?;
        Self::parse(&text)
    }

    /// Parses lockfile text.
    pub fn parse(text: &str) -> ProjectResult<Self> {
        let lockfile: Self = toml::from_str(text)
            .map_err(|error| ProjectError::lockfile(format!("invalid `seal.lock`: {error}")))?;
        if lockfile.version != 1 {
            return Err(ProjectError::lockfile(format!(
                "the lockfile version {} is not supported; this compiler understands version 1",
                lockfile.version
            )));
        }
        Ok(lockfile)
    }

    /// Serializes the lockfile deterministically.
    pub fn render(&self) -> ProjectResult<String> {
        toml::to_string_pretty(self)
            .map_err(|error| ProjectError::lockfile(format!("cannot write the lockfile: {error}")))
    }

    /// Writes the lockfile if its contents changed.
    pub fn write_if_changed(&self, path: &Path) -> ProjectResult<bool> {
        let text = self.render()?;
        if let Ok(existing) = std::fs::read_to_string(path) {
            if existing == text {
                return Ok(false);
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ProjectError::lockfile(format!(
                    "cannot create the lockfile directory `{}`: {error}",
                    parent.display()
                ))
            })?;
        }
        let temporary = path.with_extension("lock.tmp");
        std::fs::write(&temporary, &text).map_err(|error| {
            ProjectError::lockfile(format!("cannot write `{}`: {error}", temporary.display()))
        })?;
        std::fs::rename(&temporary, path).map_err(|error| {
            ProjectError::lockfile(format!("cannot replace `{}`: {error}", path.display()))
        })?;
        Ok(true)
    }

    /// Returns the package record for `name`.
    pub fn package(&self, name: &str) -> Option<&LockPackage> {
        self.packages.iter().find(|package| package.name == name)
    }
}

pub(crate) fn resolve(project: &Project, target: &str) -> Lockfile {
    let mut packages: Vec<LockPackage> = project
        .packages
        .iter()
        .map(|package| {
            let mut dependencies: Vec<LockDependency> = package
                .dependencies
                .iter()
                .map(|dependency| {
                    let target = &project.packages[dependency.package];
                    LockDependency {
                        name: target.name.clone(),
                        version: target.version.clone(),
                    }
                })
                .collect();
            dependencies.sort_by(|left, right| left.name.cmp(&right.name));
            let source = if package.root == project.root {
                "path+.".to_owned()
            } else {
                format!(
                    "path+{}",
                    package
                        .root
                        .strip_prefix(&project.root)
                        .unwrap_or(&package.root)
                        .display()
                )
            };
            LockPackage {
                name: package.name.clone(),
                version: package.version.clone(),
                source,
                checksum: checksum(&package.files, &project.root),
                api: None,
                dependencies,
            }
        })
        .collect();
    packages.sort_by(|left, right| left.name.cmp(&right.name));
    Lockfile {
        version: 1,
        toolchain: "seal 0.1.0".to_owned(),
        targets: vec![target.to_owned()],
        packages,
    }
}

fn checksum(files: &[std::path::PathBuf], root: &std::path::Path) -> String {
    let mut files = files.to_vec();
    files.sort();
    let mut hasher = Sha256::new();
    for file in files {
        let relative = file.strip_prefix(root).unwrap_or(&file);
        hasher.update(relative.to_string_lossy().as_bytes());
        if let Ok(contents) = std::fs::read(&file) {
            hasher.update(&contents);
        }
    }
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lockfile_round_trips() {
        let lockfile = Lockfile {
            version: 1,
            toolchain: "seal 0.1.0".to_owned(),
            targets: vec!["x86_64-unknown-linux-gnu".to_owned()],
            packages: vec![LockPackage {
                name: "demo".to_owned(),
                version: "0.1.0".to_owned(),
                source: "path+.".to_owned(),
                checksum: "sha256:00".to_owned(),
                api: None,
                dependencies: Vec::new(),
            }],
        };
        let text = lockfile.render().unwrap();
        assert_eq!(Lockfile::parse(&text).unwrap(), lockfile);
    }

    #[test]
    fn an_unknown_lockfile_version_is_rejected() {
        let error = Lockfile::parse("version = 9\n").unwrap_err();
        assert_eq!(error.code(), "E8008");
    }
}
