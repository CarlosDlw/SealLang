//! Root project settings.

use serde::Deserialize;

/// The `[project]` table of the root manifest.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ProjectManifest {
    /// The project name.
    pub name: String,
    /// The language version the project targets, such as `0.1`.
    #[serde(default)]
    pub language: Option<String>,
    /// Member globs relative to the root.
    #[serde(default)]
    pub members: Vec<String>,
    /// Members built when `--workspace` is not requested.
    #[serde(default)]
    pub default_members: Vec<String>,
}
