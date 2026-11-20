//! The versioned, deterministic path-dependency lockfile.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::PlanOptions;
use crate::Project;
use crate::error::{ProjectError, ProjectResult};
use crate::features;

/// The target and evaluation facts of a locked instance.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockCfg {
    /// Target operating system.
    pub os: String,
    /// Target architecture.
    pub arch: String,
    /// Target environment.
    pub env: String,
    /// Target pointer width.
    pub pointer_width: u8,
    /// Whether debug assertions are enabled.
    pub debug_assertions: bool,
    /// Whether this is a test configuration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub test: bool,
    /// Whether this is a benchmark configuration.
    #[serde(default, skip_serializing_if = "is_false")]
    pub bench: bool,
}

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
    /// The union of features enabled for this package by any instance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// The target and evaluation facts shared by its instances.
    #[serde(default)]
    pub cfg: LockCfg,
    /// The resolved dependency edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<LockDependency>,
}

/// One dependency edge in the lockfile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockDependency {
    /// The dependency alias used by the dependent package.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub alias: String,
    /// The dependency package name.
    pub name: String,
    /// The dependency version.
    pub version: String,
    /// The features requested on this edge.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

/// One resolved package instance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockInstance {
    /// The package name.
    pub name: String,
    /// The package version.
    pub version: String,
    /// The deterministic instance hash.
    pub instance: u32,
    /// Enabled features.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// The target and evaluation facts.
    pub cfg: LockCfg,
    /// The resolved dependency edges.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<LockDependency>,
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
    /// The concrete instances, sorted by package name and hash.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<LockInstance>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self {
            version: 1,
            toolchain: "seal 0.1.0".to_owned(),
            targets: Vec::new(),
            packages: Vec::new(),
            instances: Vec::new(),
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

pub(crate) fn resolve(project: &Project, target: &str, options: Option<&PlanOptions>) -> Lockfile {
    let triple = seal_target::TargetTriple::parse(target).ok();
    let cfg_facts = triple
        .as_ref()
        .map(crate::cfg::CfgEnvironment::for_triple)
        .unwrap_or_else(|| crate::cfg::CfgEnvironment::for_triple(&seal_target::host_triple()));
    let selected = options.cloned().unwrap_or_else(|| PlanOptions {
        target: target.to_owned(),
        ..PlanOptions::default()
    });
    let profile = crate::profiles::resolve(project, &selected.profile).unwrap_or_default();
    let graph = project.default_package().ok().and_then(|package| {
        let target = project.package(package).targets.first()?;
        features::resolve(project, package, target, &selected, &cfg_facts).ok()
    });

    let mut instances = Vec::new();
    let mut by_package: BTreeMap<String, (Vec<String>, LockCfg, Vec<LockDependency>)> =
        BTreeMap::new();
    if let Some(graph) = &graph {
        for instance in &graph.instances {
            let package = project.package(instance.package);
            let cfg = LockCfg {
                os: cfg_facts.os().to_owned(),
                arch: cfg_facts.arch().to_owned(),
                env: cfg_facts.env().to_owned(),
                pointer_width: cfg_facts.pointer_width(),
                debug_assertions: profile.debug_assertions,
                test: false,
                bench: false,
            };
            let dependencies: Vec<LockDependency> = instance
                .dependencies
                .iter()
                .map(|edge| {
                    let declaration =
                        &project.packages[instance.package].dependencies[edge.dependency];
                    let dependency = project.package(declaration.package);
                    LockDependency {
                        alias: edge.alias.clone(),
                        name: dependency.name.clone(),
                        version: dependency.version.clone(),
                        features: edge.features.iter().cloned().collect(),
                    }
                })
                .collect();
            let feature_list: Vec<String> = instance.features.iter().cloned().collect();
            let hash = crate::plan::instance_hash(package, &feature_list, &cfg_facts, &profile);
            let entry = by_package
                .entry(package.name.clone())
                .or_insert_with(|| (Vec::new(), cfg.clone(), Vec::new()));
            entry.0.extend(instance.features.iter().cloned());
            entry.0.sort();
            entry.0.dedup();
            entry.2.extend(dependencies.clone());
            instances.push(LockInstance {
                name: package.name.clone(),
                version: package.version.clone(),
                instance: hash,
                features: feature_list,
                cfg,
                dependencies,
            });
        }
    }
    instances.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.instance.cmp(&right.instance))
    });

    let mut packages: Vec<LockPackage> = project
        .packages
        .iter()
        .map(|package| {
            let (features, cfg, mut dependencies) = by_package
                .remove(&package.name)
                .unwrap_or_else(|| (Vec::new(), LockCfg::default(), Vec::new()));
            dependencies.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then_with(|| left.alias.cmp(&right.alias))
                    .then_with(|| left.features.cmp(&right.features))
            });
            dependencies.dedup();
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
                features,
                cfg,
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
        instances,
    }
}

fn is_false(value: &bool) -> bool {
    !*value
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
                features: Vec::new(),
                cfg: LockCfg::default(),
                dependencies: Vec::new(),
            }],
            instances: Vec::new(),
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
