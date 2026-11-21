//! Registry, trust-root, and audit policy configuration.

use std::path::PathBuf;

use serde::Deserialize;

/// The root `[registry]` table.
///
/// A project may omit the table entirely; it then uses only path
/// dependencies. When a registry dependency is declared, the URL comes from
/// this table or from `SEAL_REGISTRY_URL`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RegistryManifest {
    /// A stable name for this registry, used in lockfile sources.
    #[serde(default)]
    pub name: Option<String>,
    /// The sparse-index base URL.
    #[serde(default)]
    pub url: Option<String>,
    /// An optional mirror that is tried before `url`.
    #[serde(default)]
    pub mirror: Option<String>,
    /// The environment variable that holds the bearer token.
    #[serde(default)]
    pub token_env: Option<String>,
    /// A credential file, relative to the project root or absolute.
    #[serde(default)]
    pub credentials: Option<PathBuf>,
    /// The local TUF root metadata file.
    #[serde(default)]
    pub tuf_root: Option<PathBuf>,
    /// A TUF metadata base URL; defaults to `url`.
    #[serde(default)]
    pub tuf_url: Option<String>,
    /// An advisory feed URL; defaults to `<url>/advisories.json`.
    #[serde(default)]
    pub advisories: Option<String>,
    /// Whether normal builds avoid the network.
    #[serde(default)]
    pub offline: bool,
    /// Whether a plain `http://` URL is accepted for a local test registry.
    #[serde(default)]
    pub allow_insecure: bool,
    /// Audit policy.
    #[serde(default)]
    pub policy: RegistryPolicyManifest,
}

/// `[registry.policy]` controls `seal audit`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct RegistryPolicyManifest {
    /// License expressions that are allowed. Empty means no allow-list.
    #[serde(default)]
    pub allow_licenses: Vec<String>,
    /// License expressions that are always denied.
    #[serde(default)]
    pub deny_licenses: Vec<String>,
    /// Allowed source prefixes, such as `registry+https://...`.
    #[serde(default)]
    pub allow_sources: Vec<String>,
    /// Whether every registry package must carry provenance metadata.
    #[serde(default)]
    pub require_provenance: bool,
}
