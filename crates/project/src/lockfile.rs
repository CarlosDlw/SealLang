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
    /// Provenance metadata digest, when published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    /// Whether the registry marks this version yanked.
    #[serde(default, skip_serializing_if = "is_false")]
    pub yanked: bool,
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
    /// The dependency source, which distinguishes same-name versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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
    /// The package source, which distinguishes same-name versions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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

/// One resolved native dependency.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockNative {
    /// The package that declares the alias.
    pub package: String,
    /// The alias from `[native.<alias>]`.
    pub alias: String,
    /// The pkg-config package, when one was declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pkg_config: Option<String>,
    /// Whether static libraries were requested.
    #[serde(default, skip_serializing_if = "is_false")]
    pub static_link: bool,
    /// Library names in link order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub libs: Vec<String>,
    /// Linker search paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_paths: Vec<String>,
    /// Extra linker flags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_args: Vec<String>,
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
    /// Resolved native dependencies, sorted by package and alias.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native: Vec<LockNative>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self {
            version: 2,
            toolchain: "seal 0.1.0".to_owned(),
            targets: Vec::new(),
            packages: Vec::new(),
            instances: Vec::new(),
            native: Vec::new(),
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
        if lockfile.version != 1 && lockfile.version != 2 {
            return Err(ProjectError::lockfile(format!(
                "the lockfile version {} is not supported; this compiler understands versions 1 and 2",
                lockfile.version
            )));
        }
        let mut lockfile = lockfile;
        if lockfile.version == 1 {
            lockfile.version = 2;
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

    /// Returns the first package record for `name`.
    pub fn package(&self, name: &str) -> Option<&LockPackage> {
        self.packages.iter().find(|package| package.name == name)
    }

    /// Returns the record for one name, version, and source.
    pub fn package_with_source(
        &self,
        name: &str,
        version: &str,
        source: &str,
    ) -> Option<&LockPackage> {
        self.packages.iter().find(|package| {
            package.name == name && package.version == version && package.source == source
        })
    }

    /// Replaces the native records, sorting them deterministically.
    pub fn with_native(mut self, mut native: Vec<LockNative>) -> Self {
        native.sort_by(|left, right| {
            left.package
                .cmp(&right.package)
                .then_with(|| left.alias.cmp(&right.alias))
        });
        native.dedup();
        self.native = native;
        self
    }
}

type PackageRecord = (Vec<String>, LockCfg, Vec<LockDependency>);

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
    let mut by_package: BTreeMap<(String, String, String), PackageRecord> = BTreeMap::new();
    if let Some(graph) = &graph {
        for instance in &graph.instances {
            let package = project.package(instance.package);
            let source = package.source.lock_source(
                &project.root,
                &package.root,
                &package.name,
                &package.version,
            );
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
                        source: Some(dependency.source.lock_source(
                            &project.root,
                            &dependency.root,
                            &dependency.name,
                            &dependency.version,
                        )),
                        features: edge.features.iter().cloned().collect(),
                    }
                })
                .collect();
            let feature_list: Vec<String> = instance.features.iter().cloned().collect();
            let hash = crate::plan::instance_hash(package, &feature_list, &cfg_facts, &profile);
            let entry = by_package
                .entry((
                    package.name.clone(),
                    package.version.clone(),
                    source.clone(),
                ))
                .or_insert_with(|| (Vec::new(), cfg.clone(), Vec::new()));
            entry.0.extend(instance.features.iter().cloned());
            entry.0.sort();
            entry.0.dedup();
            entry.2.extend(dependencies.clone());
            instances.push(LockInstance {
                name: package.name.clone(),
                version: package.version.clone(),
                source: Some(source),
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
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.source.cmp(&right.source))
            .then_with(|| left.instance.cmp(&right.instance))
    });

    let mut packages: Vec<LockPackage> = project
        .packages
        .iter()
        .map(|package| {
            let source = package.source.lock_source(
                &project.root,
                &package.root,
                &package.name,
                &package.version,
            );
            let (features, cfg, mut dependencies) = by_package
                .remove(&(
                    package.name.clone(),
                    package.version.clone(),
                    source.clone(),
                ))
                .unwrap_or_else(|| (Vec::new(), LockCfg::default(), Vec::new()));
            dependencies.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then_with(|| left.version.cmp(&right.version))
                    .then_with(|| left.alias.cmp(&right.alias))
                    .then_with(|| left.features.cmp(&right.features))
            });
            dependencies.dedup();
            let (checksum, provenance, api, yanked) = match &package.source {
                crate::PackageSource::Path => {
                    (checksum(&package.files, &project.root), None, None, false)
                }
                crate::PackageSource::Registry(registry) => (
                    registry.checksum.clone(),
                    registry.provenance.clone(),
                    registry.api.clone(),
                    registry.yanked,
                ),
            };
            LockPackage {
                name: package.name.clone(),
                version: package.version.clone(),
                source,
                checksum,
                api,
                features,
                cfg,
                provenance,
                yanked,
                dependencies,
            }
        })
        .collect();
    packages.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
            .then_with(|| left.source.cmp(&right.source))
    });
    Lockfile {
        version: 2,
        toolchain: "seal 0.1.0".to_owned(),
        targets: vec![target.to_owned()],
        packages,
        instances,
        native: Vec::new(),
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
            version: 2,
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
                provenance: None,
                yanked: false,
                dependencies: Vec::new(),
            }],
            instances: Vec::new(),
            native: Vec::new(),
        };
        let text = lockfile.render().unwrap();
        assert_eq!(Lockfile::parse(&text).unwrap(), lockfile);
    }

    #[test]
    fn an_unknown_lockfile_version_is_rejected() {
        let error = Lockfile::parse("version = 9\n").unwrap_err();
        assert_eq!(error.code(), "E8008");
    }

    #[test]
    fn a_version_one_lockfile_is_migrated_in_memory() {
        let text = r#"version = 1
toolchain = "seal 0.1.0"
targets = ["x86_64-unknown-linux"]

[[package]]
name = "demo"
version = "0.1.0"
source = "path+."
checksum = "sha256:00"

[[package]]
name = "foo"
version = "1.0.0"
source = "path+vendor/foo"
checksum = "sha256:01"

[[package.dependencies]]
name = "foo"
version = "1.0.0"
"#;
        let lockfile = Lockfile::parse(text).unwrap();
        assert_eq!(lockfile.version, 2);
        assert_eq!(lockfile.packages.len(), 2);
        assert_eq!(lockfile.packages[1].dependencies[0].source, None);
        assert!(lockfile.render().unwrap().starts_with("version = 2"));
    }
}
