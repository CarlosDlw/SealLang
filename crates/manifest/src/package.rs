//! Package settings and the public API.

use serde::Deserialize;

use crate::feature::FeatureManifest;

/// The `[package]` table.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct PackageManifest {
    /// The package name.
    pub name: String,
    /// The package version, as a semantic version.
    pub version: String,
    /// A short description.
    #[serde(default)]
    pub description: Option<String>,
    /// The license expression.
    #[serde(default)]
    pub license: Option<String>,
    /// The source repository.
    #[serde(default)]
    pub repository: Option<String>,
    /// The readme path.
    #[serde(default)]
    pub readme: Option<String>,
    /// Source globs relative to the package root.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Source globs excluded from the package.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// The public API selection.
    #[serde(default)]
    pub api: Option<ApiManifest>,
    /// Feature declarations and their conflict table.
    #[serde(default)]
    pub features: FeatureManifest,
    /// Free-form tool metadata.
    #[serde(default)]
    pub metadata: Option<toml::Value>,
    /// Whether the standard library prelude is implicitly in scope.
    #[serde(default = "implicit_prelude_default")]
    pub implicit_prelude: bool,
}

impl Default for PackageManifest {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            description: None,
            license: None,
            repository: None,
            readme: None,
            sources: Vec::new(),
            exclude: Vec::new(),
            api: None,
            features: FeatureManifest::default(),
            metadata: None,
            implicit_prelude: true,
        }
    }
}

fn implicit_prelude_default() -> bool {
    true
}

/// The `[package.api]` table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ApiManifest {
    /// Module paths exposed to other packages.
    #[serde(default)]
    pub expose: Vec<String>,
    /// Whether all features are enabled for API snapshots.
    #[serde(default)]
    pub all_features: bool,
}
