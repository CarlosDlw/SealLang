//! Package settings and the public API.

use serde::Deserialize;

use crate::feature::FeatureManifest;

/// The standard-library mode selected by a package.
///
/// The manifest spells the values `"std"`, `"core"`, and `"none"`. When the
/// field is absent the project layer derives the mode from the target: hosted
/// targets receive `std`, and targets with `runtime = "none"` receive `core`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StdlibMode {
    /// The hosted standard library, including `core`.
    Std,
    /// The dependency-free core library.
    Core,
    /// No standard library.
    None,
}

impl StdlibMode {
    /// Returns the manifest spelling of the mode.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Std => "std",
            Self::Core => "core",
            Self::None => "none",
        }
    }
}

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
    /// The standard-library mode, or `None` to derive it from the target.
    #[serde(default)]
    pub stdlib: Option<StdlibMode>,
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
            stdlib: None,
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
