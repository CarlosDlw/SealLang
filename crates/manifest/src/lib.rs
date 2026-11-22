//! Reading and validating `seal.toml`.
//!
//! The manifest is the project contract: it names packages, targets, source
//! globs, path dependencies, profiles, environment values, and the public API.
//! This crate parses TOML into typed structures, reports semantic errors with a
//! line and column, and warns about unknown keys instead of silently ignoring
//! them.

#![warn(missing_docs)]

mod cfg;
mod dependency;
mod env;
mod error;
mod feature;
mod native;
mod package;
mod profile;
mod project;
mod registry;
mod target;
mod task;
mod validate;

pub use cfg::CfgSpec;
pub use dependency::{DependencyManifest, DependencySpec};
pub use env::EnvValue;
pub use error::{Location, ManifestError, ManifestResult, ManifestWarning};
pub use feature::FeatureManifest;
pub use native::NativeManifest;
pub use package::{ApiManifest, PackageManifest, StdlibMode};
pub use profile::ProfileManifest;
pub use project::ProjectManifest;
pub use registry::{RegistryManifest, RegistryPolicyManifest};
pub use target::{TargetKind, TargetManifest};
pub use task::{TaskManifest, TaskStepManifest};
pub use validate::{LoadedManifest, load, load_str};

/// The complete contents of one `seal.toml`.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Manifest {
    /// Root project settings; present only in the root manifest.
    #[serde(default)]
    pub project: Option<ProjectManifest>,
    /// Package settings; a member manifest always has one.
    #[serde(default)]
    pub package: Option<PackageManifest>,
    /// Declared targets, keyed by target name.
    #[serde(default)]
    pub targets: std::collections::BTreeMap<String, TargetManifest>,
    /// Runtime dependencies.
    #[serde(default)]
    pub dependencies: std::collections::BTreeMap<String, DependencySpec>,
    /// Dependencies used by tests and examples.
    #[serde(default, rename = "dev-dependencies")]
    pub dev_dependencies: std::collections::BTreeMap<String, DependencySpec>,
    /// Dependencies used by generators.
    #[serde(default, rename = "build-dependencies")]
    pub build_dependencies: std::collections::BTreeMap<String, DependencySpec>,
    /// Declared environment values.
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, EnvValue>,
    /// Build profiles.
    #[serde(default)]
    pub profiles: std::collections::BTreeMap<String, ProfileManifest>,
    /// Declarative task recipes.
    #[serde(default)]
    pub tasks: std::collections::BTreeMap<String, TaskManifest>,
    /// Declarative native library dependencies.
    #[serde(default)]
    pub native: std::collections::BTreeMap<String, NativeManifest>,
    /// Registry and audit configuration; valid only in the root manifest.
    #[serde(default)]
    pub registry: Option<RegistryManifest>,
}
