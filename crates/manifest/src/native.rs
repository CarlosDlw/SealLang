//! Declarative native library dependencies.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One `[native.<alias>]` table.
///
/// A native dependency is resolved either from explicit `libs` and
/// `search-paths`, or by asking `pkg-config` for the package named by
/// `pkg-config`. The two forms can be combined: pkg-config supplies its result
/// and the explicit entries are appended.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct NativeManifest {
    /// The pkg-config package name.
    #[serde(default)]
    pub pkg_config: Option<String>,
    /// Library names passed to the linker.
    #[serde(default)]
    pub libs: Vec<String>,
    /// Directories searched for the libraries.
    #[serde(default)]
    pub search_paths: Vec<PathBuf>,
    /// Whether pkg-config should request static libraries.
    #[serde(default, rename = "static")]
    pub static_link: bool,
}

impl NativeManifest {
    /// Returns whether the declaration resolves anything.
    pub fn is_empty(&self) -> bool {
        self.pkg_config.is_none() && self.libs.is_empty() && self.search_paths.is_empty()
    }
}
