//! Seal registry client: sparse index, TUF, resolution, verified downloads,
//! vendoring, audit, and SBOM generation.

#![warn(missing_docs)]

pub mod archive;
pub mod audit;
pub mod auth;
pub mod client;
pub mod config;
pub mod download;
pub mod error;
pub mod index;
pub mod log;
pub mod provenance;
pub mod publish;
pub mod resolve;
pub mod sbom;
pub mod semver;
pub mod tuf;
pub mod vendor;

use std::path::{Path, PathBuf};

use ::semver::Version;
use seal_manifest::{LoadedManifest, RegistryManifest};

pub use error::{RegistryError, RegistryResult};
pub use resolve::{Requirement, ResolveOptions, ResolvedPackage};

use crate::client::RegistryClient;
use crate::config::RegistryConfig;
use crate::download::Downloader;
use crate::tuf::TufSession;

/// An open registry session.
pub struct RegistrySession {
    config: RegistryConfig,
    client: RegistryClient,
    tuf: TufSession,
}

impl RegistrySession {
    /// Opens a session, loading and verifying TUF metadata.
    ///
    /// Returns `None` when the project does not configure a registry.
    pub fn open(
        project_root: &Path,
        manifest: Option<&RegistryManifest>,
        forced_offline: bool,
    ) -> RegistryResult<Option<Self>> {
        let Some(config) = RegistryConfig::load(project_root, manifest, forced_offline)? else {
            return Ok(None);
        };
        let client = RegistryClient::new(&config)?;
        let tuf = TufSession::load(&client, &config)?;
        Ok(Some(Self {
            config,
            client,
            tuf,
        }))
    }

    /// Returns the effective configuration.
    pub fn config(&self) -> &RegistryConfig {
        &self.config
    }

    /// Returns the HTTP client.
    pub fn client(&self) -> &RegistryClient {
        &self.client
    }

    /// Returns the verified TUF session.
    pub fn tuf(&self) -> &TufSession {
        &self.tuf
    }

    /// Resolves registry requirements with PubGrub.
    pub fn resolve(
        &self,
        requirements: &[Requirement],
        options: &ResolveOptions,
    ) -> RegistryResult<Vec<ResolvedPackage>> {
        resolve::resolve(
            &self.client,
            &self.config,
            Some(&self.tuf),
            requirements,
            options,
        )
    }

    /// Downloads and extracts one selected package, returning its root.
    pub fn materialize(&self, package: &ResolvedPackage) -> RegistryResult<PathBuf> {
        Downloader::new(&self.client, &self.config, Some(&self.tuf)).ensure(
            &package.name,
            &package.version,
            &package.entry,
        )
    }

    /// Loads a materialized package manifest and checks it against the index.
    pub fn manifest(
        &self,
        package: &ResolvedPackage,
        root: &Path,
    ) -> RegistryResult<LoadedManifest> {
        let marker = Downloader::new(&self.client, &self.config, Some(&self.tuf)).ensure(
            &package.name,
            &package.version,
            &package.entry,
        )?;
        if marker != root && !root.starts_with(&marker) {
            return Err(RegistryError::package(format!(
                "the package `{}` {} is not under its verified cache root",
                package.name, package.version
            )));
        }
        download::verify_cached(root, &package.entry.checksum)?;
        let loaded = seal_manifest::load(&root.join("seal.toml")).map_err(|error| {
            RegistryError::package(format!(
                "the package `{}` {} has an invalid manifest: {error}",
                package.name, package.version
            ))
        })?;
        let manifest = loaded.manifest.package.as_ref().ok_or_else(|| {
            RegistryError::package(format!(
                "the package `{}` {} has no `[package]` table",
                package.name, package.version
            ))
        })?;
        if manifest.name != package.name {
            return Err(RegistryError::package(format!(
                "the archive for `{}` {} contains the package `{}`",
                package.name, package.version, manifest.name
            )));
        }
        if manifest.version != package.version.to_string() {
            return Err(RegistryError::package(format!(
                "the archive for `{}` {} contains version `{}`",
                package.name, package.version, manifest.version
            )));
        }
        verify_dependencies(package, &loaded.manifest)?;
        Ok(loaded)
    }

    /// Fetches the advisory feed.
    pub fn advisories(&self) -> RegistryResult<Vec<audit::Advisory>> {
        audit::load_advisories(&self.client, &self.config, Some(&self.tuf))
    }

    /// Searches the registry catalog.
    pub fn search(&self, query: &str) -> RegistryResult<Vec<index::CatalogEntry>> {
        let catalog = index::catalog(&self.client, &self.config, Some(&self.tuf))?;
        let query = query.to_ascii_lowercase();
        Ok(catalog
            .packages
            .into_iter()
            .filter(|entry| entry.name.to_ascii_lowercase().contains(&query))
            .collect())
    }

    /// Loads one package index for `seal info`.
    pub fn info(&self, name: &str) -> RegistryResult<index::IndexPackage> {
        index::load(&self.client, &self.config, Some(&self.tuf), name)
    }

    /// Posts a JSON document to a registry API path.
    pub fn post(&self, path: &str, body: &[u8]) -> RegistryResult<client::HttpResponse> {
        self.client.post_json(&self.config, path, body)
    }

    /// Fetches a transparency-log proof path below the registry base.
    pub fn fetch(&self, path: &str) -> RegistryResult<Vec<u8>> {
        self.client.fetch(&self.config, path)
    }

    /// Fetches the signed tree head.
    pub fn tree_head(&self) -> RegistryResult<log::SignedTreeHead> {
        let bytes = self.fetch("api/v1/log/sth")?;
        serde_json::from_slice(&bytes).map_err(|error| {
            RegistryError::transparency(format!("invalid signed tree head: {error}"))
        })
    }

    /// Opens a session from an already validated configuration.
    ///
    /// This is used by tests and embedding tools that already know the trust
    /// root and the registry URL.
    pub fn from_config(config: RegistryConfig) -> RegistryResult<Self> {
        let client = RegistryClient::new(&config)?;
        let tuf = TufSession::load(&client, &config)?;
        Ok(Self {
            config,
            client,
            tuf,
        })
    }
}

fn verify_dependencies(
    package: &ResolvedPackage,
    manifest: &seal_manifest::Manifest,
) -> RegistryResult<()> {
    let mut declared = std::collections::BTreeMap::new();
    for (alias, dependency) in &manifest.dependencies {
        match dependency {
            seal_manifest::DependencySpec::Version(version) => {
                declared.insert(
                    alias.clone(),
                    (
                        alias.clone(),
                        ::semver::VersionReq::parse(version).map_err(|error| {
                            RegistryError::package(format!(
                                "the package `{}` {} has an invalid dependency `{alias}`: {error}",
                                package.name, package.version
                            ))
                        })?,
                    ),
                );
            }
            seal_manifest::DependencySpec::Detailed(detail) => {
                if detail.path.is_some() || detail.git.is_some() {
                    return Err(RegistryError::package(format!(
                        "the registry package `{}` {} declares a path or git dependency `{alias}`",
                        package.name, package.version
                    )));
                }
                let version = detail.version.as_deref().ok_or_else(|| {
                    RegistryError::package(format!(
                        "the registry package `{}` {} has a dependency `{alias}` without a version",
                        package.name, package.version
                    ))
                })?;
                declared.insert(
                    alias.clone(),
                    (
                        detail.package.clone().unwrap_or_else(|| alias.clone()),
                        ::semver::VersionReq::parse(version).map_err(|error| {
                            RegistryError::package(format!(
                                "the package `{}` {} has an invalid dependency `{alias}`: {error}",
                                package.name, package.version
                            ))
                        })?,
                    ),
                );
            }
        }
    }
    for dependency in &package.entry.dependencies {
        let alias = dependency.alias();
        let Some((name, requirement)) = declared.get(alias) else {
            return Err(RegistryError::package(format!(
                "the index for `{}` {} lists `{alias}`, but its manifest does not",
                package.name, package.version
            )));
        };
        if name != &dependency.name || requirement != &dependency.requirement()? {
            return Err(RegistryError::package(format!(
                "the dependency `{alias}` of `{}` {} disagrees with its index entry",
                package.name, package.version
            )));
        }
        declared.remove(alias);
    }
    if let Some(alias) = declared.keys().next() {
        return Err(RegistryError::package(format!(
            "the manifest of `{}` {} declares `{alias}`, but its index entry does not",
            package.name, package.version
        )));
    }
    Ok(())
}

/// Returns a package's source string for a lockfile.
pub fn source_string(config: &RegistryConfig, name: &str, version: &Version) -> String {
    format!("registry+{}#{}@{}", config.url, name, version)
}
