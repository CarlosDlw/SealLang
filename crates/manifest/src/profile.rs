//! Build profiles.

use serde::Deserialize;

/// One `[profiles.<name>]` table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ProfileManifest {
    /// The profile this one inherits from.
    #[serde(default)]
    pub inherits: Option<String>,
    /// Optimization level, 0 through 3.
    #[serde(default)]
    pub opt_level: Option<u8>,
    /// Whether debug information is emitted.
    #[serde(default)]
    pub debug: Option<bool>,
    /// Whether debug assertions are active.
    #[serde(default)]
    pub debug_assertions: Option<bool>,
    /// Link-time optimization mode.
    #[serde(default)]
    pub lto: Option<String>,
    /// Number of code generation units.
    #[serde(default)]
    pub codegen_units: Option<u32>,
    /// Whether symbols are stripped.
    #[serde(default)]
    pub strip: Option<bool>,
    /// Whether incremental compilation is enabled.
    #[serde(default)]
    pub incremental: Option<bool>,
}
