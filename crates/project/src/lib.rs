//! The project layer between `seal.toml` and the compiler driver.
//!
//! It discovers members and source files, resolves path dependencies, detects
//! cycles, validates targets, builds a deterministic lockfile, and produces one
//! [`CompilePlan`] per target. It never invokes the compiler itself.

#![warn(missing_docs)]

mod cfg;
mod error;
mod features;
mod graph;
mod lockfile;
mod plan;
mod profiles;
mod source;
mod targets;

use std::path::{Path, PathBuf};

pub use error::{ProjectError, ProjectResult};
pub use lockfile::{LockCfg, LockDependency, LockInstance, LockNative, LockPackage, Lockfile};
pub use plan::PlanOptions;

use seal_manifest::{
    CfgSpec, LoadedManifest, Manifest, ManifestWarning, ProfileManifest, TargetKind,
};
use seal_plan::CompilePlan;

/// One build target of a package.
#[derive(Clone, Debug)]
pub struct Target {
    /// The target name.
    pub name: String,
    /// The artifact kind.
    pub kind: TargetKind,
    /// The entry function path, when the target has one.
    pub entry: Option<String>,
    /// Every source file of the target, primary first.
    pub files: Vec<PathBuf>,
    /// The primary source file.
    pub primary: PathBuf,
    /// An artifact path override relative to the project root.
    pub output: Option<PathBuf>,
    /// The target's profile override.
    pub profile: Option<String>,
    /// The runtime policy (`default` or `none`).
    pub runtime: Option<String>,
    /// The panic policy (`abort` or `trap`).
    pub panic: Option<String>,
    /// The linker override.
    pub linker: Option<String>,
    /// The sysroot passed to the C toolchain.
    pub sysroot: Option<PathBuf>,
    /// The linker script.
    pub linker_script: Option<PathBuf>,
    /// Extra linker arguments declared for this target.
    pub link_args: Vec<String>,
    /// Features that must be enabled before the target is built.
    pub required_features: Vec<String>,
    /// The configuration condition under which this target exists.
    pub when: Option<seal_manifest::CfgSpec>,
    /// Generator input files, resolved from the target's input globs.
    pub generator_inputs: Vec<PathBuf>,
    /// Generator output globs, relative to the package root.
    pub generator_outputs: Vec<String>,
    /// Generator targets whose outputs this target compiles.
    pub generated: Vec<String>,
    /// Native aliases linked into this target.
    pub native: Vec<String>,
}

impl Target {
    /// Returns whether the target's `when` condition holds for a triple.
    pub fn is_active_for(&self, target: &str) -> bool {
        let Some(condition) = &self.when else {
            return true;
        };
        seal_target::TargetTriple::parse(target)
            .ok()
            .is_some_and(|triple| {
                crate::cfg::CfgEnvironment::for_triple(&triple).matches(condition)
            })
    }

    /// Returns whether the target produces an executable.
    pub fn is_executable(&self) -> bool {
        matches!(
            self.kind,
            TargetKind::Bin
                | TargetKind::Test
                | TargetKind::Bench
                | TargetKind::Example
                | TargetKind::Generator
        )
    }
}

/// One resolved dependency edge.
#[derive(Clone, Debug)]
pub struct Dependency {
    /// The name the package uses in imports.
    pub alias: String,
    /// Index into [`Project::packages`].
    pub package: usize,
    /// Features requested by this edge.
    pub features: Vec<String>,
    /// Whether the dependency's default feature is enabled.
    pub default_features: bool,
    /// Whether the dependency is activated by a feature.
    pub optional: bool,
    /// A typed activation condition.
    pub when: Option<CfgSpec>,
}

/// Where a package's bytes came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageSource {
    /// A directory inside the project.
    Path,
    /// A verified registry archive.
    Registry(RegistrySource),
}

/// Registry identity for one package version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySource {
    /// The registry base URL.
    pub registry: String,
    /// The archive SHA-256 digest.
    pub checksum: String,
    /// Whether the index marks this version yanked.
    pub yanked: bool,
    /// Provenance metadata digest, when published.
    pub provenance: Option<String>,
    /// The published API snapshot digest, when available.
    pub api: Option<String>,
}

impl PackageSource {
    /// Returns the lockfile source string.
    pub fn lock_source(
        &self,
        project_root: &Path,
        root: &Path,
        name: &str,
        version: &str,
    ) -> String {
        match self {
            Self::Path => {
                if root == project_root {
                    "path+.".to_owned()
                } else {
                    format!(
                        "path+{}",
                        root.strip_prefix(project_root).unwrap_or(root).display()
                    )
                }
            }
            Self::Registry(source) => {
                format!("registry+{}#{}@{}", source.registry, name, version)
            }
        }
    }
}

/// One resolved package, in dependency order.
#[derive(Clone, Debug)]
pub struct Package {
    /// The package name.
    pub name: String,
    /// The package version.
    pub version: String,
    /// The package root.
    pub root: PathBuf,
    /// Where the package came from.
    pub source: PackageSource,
    /// The complete manifest.
    pub manifest: Manifest,
    /// Every base source file, sorted.
    pub files: Vec<PathBuf>,
    /// Declared or inferred targets.
    pub targets: Vec<Target>,
    /// Resolved dependency edges.
    pub dependencies: Vec<Dependency>,
}

impl Package {
    /// Returns the library target, when the package declares one.
    pub fn library(&self) -> Option<&Target> {
        self.targets
            .iter()
            .find(|target| target.kind == TargetKind::Lib)
    }

    /// Returns a target by name.
    pub fn target(&self, name: &str) -> Option<&Target> {
        self.targets.iter().find(|target| target.name == name)
    }
}

/// A fully loaded project.
#[derive(Clone, Debug)]
pub struct Project {
    /// The canonical project root.
    pub root: PathBuf,
    /// The project name.
    pub name: String,
    /// Packages selected by commands without `--workspace`.
    pub default_members: Vec<String>,
    /// Packages in dependency order.
    pub packages: Vec<Package>,
    /// Non-fatal manifest warnings.
    pub warnings: Vec<ManifestWarning>,
    /// The current lockfile contents.
    pub lockfile: Lockfile,
    /// Root-level profiles. Members cannot declare hidden profiles.
    pub profiles: std::collections::BTreeMap<String, ProfileManifest>,
    /// Registry and audit configuration from the root manifest.
    pub registry: Option<seal_manifest::RegistryManifest>,
}

/// Options that affect project loading and registry resolution.
#[derive(Clone, Debug, Default)]
pub struct ProjectOptions {
    /// Forbid network access.
    pub offline: bool,
    /// Prefer the lowest compatible registry versions.
    pub minimal_versions: bool,
    /// Ignore locked version preferences, as `seal update` does.
    pub update: bool,
    /// Use only the vendor tree, even when the network is available.
    pub vendor_only: bool,
    /// Packages whose locked version may change; empty updates every package
    /// when `update` is true.
    pub update_packages: Vec<String>,
    /// Exact versions requested by `seal update --precise`.
    pub pinned: std::collections::BTreeMap<String, String>,
}

impl Project {
    /// Loads the project rooted at `root`.
    pub fn load(root: &Path) -> ProjectResult<Self> {
        graph::load(root, &ProjectOptions::default())
    }

    /// Loads the project with explicit registry options.
    pub fn load_with(root: &Path, options: &ProjectOptions) -> ProjectResult<Self> {
        graph::load(root, options)
    }

    /// Returns the package at `index`.
    pub fn package(&self, index: usize) -> &Package {
        &self.packages[index]
    }

    /// Returns the newest package with `name`.
    pub fn package_by_name(&self, name: &str) -> Option<(usize, &Package)> {
        self.packages
            .iter()
            .enumerate()
            .filter(|(_, package)| package.name == name)
            .max_by(|(_, left), (_, right)| {
                semver::Version::parse(&left.version)
                    .ok()
                    .cmp(&semver::Version::parse(&right.version).ok())
            })
    }

    /// Returns every package instance with `name`, sorted by version.
    pub fn packages_named(&self, name: &str) -> Vec<(usize, &Package)> {
        let mut packages: Vec<(usize, &Package)> = self
            .packages
            .iter()
            .enumerate()
            .filter(|(_, package)| package.name == name)
            .collect();
        packages.sort_by(|(_, left), (_, right)| left.version.cmp(&right.version));
        packages
    }

    /// Returns the package declared at the project root, if any.
    pub fn root_package(&self) -> Option<usize> {
        self.packages
            .iter()
            .position(|package| package.root == self.root)
    }

    /// Returns the default package for commands without `-p`.
    pub fn default_package(&self) -> ProjectResult<usize> {
        if let Some(index) = self.root_package() {
            return Ok(index);
        }
        for member in &self.default_members {
            let name = member.rsplit('/').next().unwrap_or(member);
            if let Some((index, _)) = self.package_by_name(name) {
                return Ok(index);
            }
            if let Some((index, _)) = self
                .packages
                .iter()
                .enumerate()
                .find(|(_, package)| package.root.ends_with(member))
            {
                return Ok(index);
            }
        }
        if let Some((index, _)) = self.package_by_name(&self.name) {
            return Ok(index);
        }
        if self.packages.len() == 1 {
            return Ok(0);
        }
        Err(ProjectError::manifest(
            "the project has several packages; select one with `-p <PACKAGE>` or `--workspace`",
        ))
    }

    /// Builds one compile plan.
    pub fn compile_plan(
        &self,
        package: usize,
        target: &str,
        options: &PlanOptions,
    ) -> ProjectResult<CompilePlan> {
        plan::compile_plan(self, package, target, options)
    }

    /// Returns the validated target specification for `triple`.
    ///
    /// A project may keep a TOML specification at
    /// `<project>/seal/targets/<triple>.toml`; without one, the built-in
    /// properties for the triple are used.
    pub fn target_spec(&self, triple: &str) -> ProjectResult<seal_target::TargetSpec> {
        let parsed = seal_target::TargetTriple::parse(triple)
            .map_err(|error| ProjectError::target_spec(error.to_string()))?;
        let path = self.target_spec_path(parsed.as_str());
        if !path.is_file() {
            return Ok(parsed.spec());
        }
        let text = std::fs::read_to_string(&path).map_err(|error| {
            ProjectError::target_spec(format!("cannot read `{}`: {error}", path.display()))
        })?;
        let spec = seal_target::TargetSpec::parse_toml(&text)
            .map_err(|error| ProjectError::target_spec(format!("{}: {error}", path.display())))?;
        let spec = resolve_spec_paths(spec, &self.root);
        if spec.triple().as_str() != triple {
            return Err(ProjectError::target_spec(format!(
                "the target specification `{}` names `{}`, not `{triple}`",
                path.display(),
                spec.triple()
            )));
        }
        Ok(spec)
    }

    /// Returns every project target specification, sorted by triple.
    pub fn target_specs(&self) -> ProjectResult<Vec<seal_target::TargetSpec>> {
        let directory = self.root.join("seal").join("targets");
        if !directory.exists() {
            return Ok(Vec::new());
        }
        let entries = std::fs::read_dir(&directory).map_err(|error| {
            ProjectError::target_spec(format!("cannot read `{}`: {error}", directory.display()))
        })?;
        let mut specs = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProjectError::target_spec(format!("cannot read `{}`: {error}", directory.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("toml") {
                continue;
            }
            let text = std::fs::read_to_string(&path).map_err(|error| {
                ProjectError::target_spec(format!("cannot read `{}`: {error}", path.display()))
            })?;
            let spec = seal_target::TargetSpec::parse_toml(&text).map_err(|error| {
                ProjectError::target_spec(format!("{}: {error}", path.display()))
            })?;
            specs.push(resolve_spec_paths(spec, &self.root));
        }
        specs.sort_by(|left, right| left.triple().as_str().cmp(right.triple().as_str()));
        Ok(specs)
    }

    /// Returns the path of the TOML specification for `triple`.
    pub fn target_spec_path(&self, triple: &str) -> PathBuf {
        self.root
            .join("seal")
            .join("targets")
            .join(format!("{triple}.toml"))
    }

    /// Returns all targets selected by `--all-targets`.
    pub fn all_targets(&self) -> Vec<(usize, &Target)> {
        self.packages
            .iter()
            .enumerate()
            .flat_map(|(index, package)| package.targets.iter().map(move |target| (index, target)))
            .collect()
    }

    /// Computes the deterministic lockfile for the project.
    pub fn resolve_lockfile(&self, target: &str) -> Lockfile {
        lockfile::resolve(self, target, None)
    }

    /// Computes the lockfile for an explicit feature selection.
    pub fn resolve_lockfile_with(&self, target: &str, options: &PlanOptions) -> Lockfile {
        lockfile::resolve(self, target, Some(options))
    }

    /// Writes the lockfile when its contents changed.
    pub fn write_lockfile(&self) -> ProjectResult<bool> {
        self.lockfile.write_if_changed(&self.root.join("seal.lock"))
    }
}

#[cfg(test)]
mod tests;

/// Resolves target paths relative to the project root.
fn resolve_spec_paths(spec: seal_target::TargetSpec, root: &Path) -> seal_target::TargetSpec {
    let resolve = |path: Option<&Path>| {
        path.map(|path| {
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
        })
    };
    let sysroot = resolve(spec.sysroot());
    let linker_script = resolve(spec.linker_script());
    spec.with_sysroot(sysroot).with_linker_script(linker_script)
}

/// Loads a manifest and returns it with the canonical root.
pub(crate) fn load_manifest(root: &Path) -> ProjectResult<LoadedManifest> {
    let path = root.join("seal.toml");
    if !path.is_file() {
        return Err(ProjectError::io(format!(
            "no `seal.toml` found at `{}`; run `seal init` to create one",
            root.display()
        )));
    }
    seal_manifest::load(&path)
        .map_err(|error| ProjectError::manifest(format!("{}: {}", path.display(), error)))
}
