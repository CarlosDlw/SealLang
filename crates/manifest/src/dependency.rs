//! Dependency declarations.

use std::path::PathBuf;

use serde::Deserialize;

use crate::cfg::CfgSpec;

/// A dependency written either as a version string or as a table.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// A registry version, such as `"1.2"`.
    Version(String),
    /// A detailed declaration.
    Detailed(DependencyManifest),
}

/// A detailed `[dependencies.<name>]` table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DependencyManifest {
    /// The requested version.
    #[serde(default)]
    pub version: Option<String>,
    /// A local path dependency.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// A git dependency.
    #[serde(default)]
    pub git: Option<String>,
    /// Enabled features.
    #[serde(default)]
    pub features: Vec<String>,
    /// Whether the dependency's default features are enabled.
    #[serde(default = "default_true")]
    pub default_features: bool,
    /// Whether the dependency is optional.
    #[serde(default)]
    pub optional: bool,
    /// A typed condition that activates the dependency.
    #[serde(default)]
    pub when: Option<CfgSpec>,
    /// The registry name.
    #[serde(default)]
    pub registry: Option<String>,
}

const fn default_true() -> bool {
    true
}
