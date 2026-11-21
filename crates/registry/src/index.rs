//! The sparse package index.

use std::collections::BTreeMap;

use seal_manifest::CfgSpec;
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::client::RegistryClient;
use crate::config::RegistryConfig;
use crate::error::{RegistryError, RegistryResult};
use crate::tuf::TufSession;

/// One package's sparse index document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexPackage {
    /// The index schema version.
    pub format: u32,
    /// The package name.
    pub name: String,
    /// Versions keyed by their semantic version.
    pub versions: BTreeMap<String, IndexVersion>,
}

/// One published version in an index.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexVersion {
    /// The archive digest, `sha256:<hex>`.
    pub checksum: String,
    /// The archive path relative to the registry root.
    pub archive: String,
    /// Whether this version is yanked.
    #[serde(default)]
    pub yanked: bool,
    /// The license expression.
    #[serde(default)]
    pub license: Option<String>,
    /// A short description.
    #[serde(default)]
    pub description: Option<String>,
    /// The repository URL.
    #[serde(default)]
    pub repository: Option<String>,
    /// Provenance metadata digest, when published.
    #[serde(default)]
    pub provenance: Option<String>,
    /// The provenance envelope path relative to the registry root.
    #[serde(default)]
    pub provenance_file: Option<String>,
    /// SBOM metadata digest, when published.
    #[serde(default)]
    pub sbom: Option<String>,
    /// The SBOM path relative to the registry root.
    #[serde(default)]
    pub sbom_file: Option<String>,
    /// The public-API snapshot metadata, when published.
    #[serde(default)]
    pub api: Option<ApiEntry>,
    /// The signing identity that published this version.
    #[serde(default)]
    pub publisher: Option<Publisher>,
    /// Transparency-log inclusion metadata.
    #[serde(default)]
    pub log: Option<LogInclusion>,
    /// The RFC 3339 publication time.
    #[serde(default)]
    pub published_at: Option<String>,
    /// Dependencies used for resolution.
    #[serde(default)]
    pub dependencies: Vec<IndexDependency>,
    /// Feature declarations, used for validation before download.
    #[serde(default)]
    pub features: BTreeMap<String, Vec<String>>,
}

/// The API snapshot attached to a published version.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ApiEntry {
    /// The snapshot digest, `sha256:<hex>`.
    pub hash: String,
    /// The snapshot path relative to the registry root.
    pub snapshot: String,
}

/// The Ed25519 identity that signed a publication.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Publisher {
    /// The key id, the SHA-256 digest of the public key.
    pub keyid: String,
    /// The 32-byte public key as lowercase hexadecimal.
    pub public: String,
    /// The OIDC identity, when trusted publishing was used.
    #[serde(default)]
    pub identity: Option<String>,
}

/// One verified inclusion in the transparency log.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LogInclusion {
    /// The exact leaf that was appended.
    pub entry: crate::log::LogEntry,
    /// The leaf index.
    pub index: u64,
    /// The tree size at publication.
    pub tree_size: u64,
    /// The signed tree root, `sha256:<hex>`.
    pub root_hash: String,
    /// The audit path from the leaf to the root.
    #[serde(default)]
    pub proof: Vec<String>,
}

/// A dependency edge in an index entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct IndexDependency {
    /// The package name.
    pub name: String,
    /// The local alias; defaults to the package name.
    #[serde(default)]
    pub alias: Option<String>,
    /// The version requirement.
    pub req: String,
    /// Features requested on the edge.
    #[serde(default)]
    pub features: Vec<String>,
    /// Whether default features are enabled.
    #[serde(default = "default_true")]
    pub default_features: bool,
    /// Whether this is an optional dependency.
    #[serde(default)]
    pub optional: bool,
    /// A configuration condition.
    #[serde(default)]
    pub when: Option<CfgSpec>,
}

impl IndexDependency {
    /// Returns the alias used by the package.
    pub fn alias(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }

    /// Parses and returns the version requirement.
    pub fn requirement(&self) -> RegistryResult<VersionReq> {
        crate::semver::parse_requirement(&self.req)
    }
}

/// A search-catalog entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogEntry {
    /// The package name.
    pub name: String,
    /// The newest stable version, when one exists.
    pub latest: Option<String>,
    /// A short description.
    #[serde(default)]
    pub description: Option<String>,
    /// The license expression.
    #[serde(default)]
    pub license: Option<String>,
}

/// The search catalog served as `index/catalog.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    /// The catalog schema version.
    pub format: u32,
    /// Packages sorted by name.
    #[serde(default)]
    pub packages: Vec<CatalogEntry>,
}

impl IndexPackage {
    /// Returns the newest non-yanked stable version.
    pub fn latest(&self) -> Option<Version> {
        self.parsed_versions()
            .into_iter()
            .filter(|(version, entry)| !entry.yanked && version.pre.is_empty())
            .map(|(version, _)| version)
            .max()
    }

    /// Returns a parsed version entry.
    pub fn version(&self, version: &Version) -> Option<&IndexVersion> {
        self.versions.get(&version.to_string())
    }

    /// Returns every parsed version and entry, sorted ascending.
    pub fn parsed_versions(&self) -> Vec<(Version, &IndexVersion)> {
        let mut versions: Vec<(Version, &IndexVersion)> = self
            .versions
            .iter()
            .filter_map(|(version, entry)| {
                Version::parse(version).ok().map(|version| (version, entry))
            })
            .collect();
        versions.sort_by(|left, right| left.0.cmp(&right.0));
        versions
    }
}

/// Loads one package index document.
pub fn load(
    client: &RegistryClient,
    config: &RegistryConfig,
    tuf: Option<&TufSession>,
    name: &str,
) -> RegistryResult<IndexPackage> {
    let path = sparse_path(name)?;
    let bytes = match tuf {
        Some(tuf) => tuf.target_from(client, config, &path)?,
        None => client.fetch(config, &path)?,
    };
    let package: IndexPackage = serde_json::from_slice(&bytes)
        .map_err(|error| RegistryError::index(format!("invalid index for `{name}`: {error}")))?;
    validate(name, &package)?;
    Ok(package)
}

/// Loads the search catalog.
pub fn catalog(
    client: &RegistryClient,
    config: &RegistryConfig,
    tuf: Option<&TufSession>,
) -> RegistryResult<Catalog> {
    let path = "index/catalog.json";
    let bytes = match tuf {
        Some(tuf) => tuf.target_from(client, config, path)?,
        None => client.fetch(config, path)?,
    };
    let catalog: Catalog = serde_json::from_slice(&bytes)
        .map_err(|error| RegistryError::query(format!("invalid search catalog: {error}")))?;
    if catalog.format != 1 {
        return Err(RegistryError::query(format!(
            "the search catalog uses format {}, but this client understands 1",
            catalog.format
        )));
    }
    Ok(catalog)
}

/// Returns the sparse index path for a package name.
pub fn sparse_path(name: &str) -> RegistryResult<String> {
    if name.is_empty() || name.contains(['/', '\\']) && !name.starts_with('@') {
        return Err(RegistryError::index(format!(
            "`{name}` is not a valid registry package name"
        )));
    }
    let (scope, package) = if let Some(rest) = name.strip_prefix('@') {
        let (scope, package) = rest.split_once('/').ok_or_else(|| {
            RegistryError::index(format!("the scoped package `{name}` needs `@scope/name`"))
        })?;
        (Some(scope), package)
    } else {
        (None, name)
    };
    if !safe_component(package) || scope.is_some_and(|scope| !safe_component(scope)) {
        return Err(RegistryError::index(format!(
            "`{name}` is not a valid registry package name"
        )));
    }
    let path = if let Some(scope) = scope {
        format!("index/@{scope}/{package}.json")
    } else {
        match package.len() {
            1 => format!("index/1/{package}.json"),
            2 => format!("index/2/{package}.json"),
            3 => format!("index/3/{}/{package}.json", &package[..1]),
            _ => format!("index/{}/{}/{package}.json", &package[..2], &package[2..4]),
        }
    };
    Ok(path)
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains("..")
        && !path.contains('\\')
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'-' | b'_' | b'@')
        })
}

fn valid_public_key(public: &str) -> bool {
    public.len() == 64
        && public
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn safe_component(component: &str) -> bool {
    !component.is_empty()
        && component != "."
        && component != ".."
        && component
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Validates a package index against the requested name and schema.
pub fn validate(name: &str, package: &IndexPackage) -> RegistryResult<()> {
    if package.format != 1 {
        return Err(RegistryError::index(format!(
            "the index for `{name}` uses format {}, but this client understands 1",
            package.format
        )));
    }
    if package.name != name {
        return Err(RegistryError::index(format!(
            "the index for `{name}` names `{}`",
            package.name
        )));
    }
    if package.versions.is_empty() {
        return Err(RegistryError::index(format!(
            "the index for `{name}` has no versions"
        )));
    }
    for (text, entry) in &package.versions {
        let version = Version::parse(text).map_err(|error| {
            RegistryError::index(format!(
                "the index for `{name}` contains the invalid version `{text}`: {error}"
            ))
        })?;
        if version.to_string() != *text {
            return Err(RegistryError::index(format!(
                "the index for `{name}` spells `{text}` instead of `{version}`"
            )));
        }
        validate_checksum(&entry.checksum)?;
        if entry.archive.is_empty()
            || entry.archive.starts_with('/')
            || entry.archive.contains("..")
            || entry.archive.contains('\\')
            || !entry.archive.ends_with(".tar.zst")
        {
            return Err(RegistryError::index(format!(
                "the archive path `{}` for `{name}` {version} is not a safe relative `.tar.zst` path",
                entry.archive
            )));
        }
        if let Some(provenance) = &entry.provenance {
            validate_checksum(provenance)?;
        }
        if let Some(path) = &entry.provenance_file
            && !safe_relative(path)
        {
            return Err(RegistryError::index(format!(
                "the provenance path `{path}` for `{name}` {version} is not safe"
            )));
        }
        if let Some(sbom) = &entry.sbom {
            validate_checksum(sbom)?;
        }
        if let Some(path) = &entry.sbom_file
            && !safe_relative(path)
        {
            return Err(RegistryError::index(format!(
                "the SBOM path `{path}` for `{name}` {version} is not safe"
            )));
        }
        if let Some(api) = &entry.api {
            validate_checksum(&api.hash)?;
            if !safe_relative(&api.snapshot) {
                return Err(RegistryError::index(format!(
                    "the API snapshot path `{}` for `{name}` {version} is not a safe relative path",
                    api.snapshot
                )));
            }
        }
        if let Some(publisher) = &entry.publisher
            && !valid_public_key(&publisher.public)
        {
            return Err(RegistryError::index(format!(
                "the publisher key for `{name}` {version} is not a 32-byte lowercase hexadecimal key"
            )));
        }
        if let Some(log) = &entry.log {
            validate_checksum(&log.root_hash)?;
            if log.tree_size == 0 || log.index >= log.tree_size {
                return Err(RegistryError::index(format!(
                    "the transparency entry for `{name}` {version} is outside its tree"
                )));
            }
            for hash in &log.proof {
                validate_checksum(hash)?;
            }
        }
        let mut aliases = std::collections::BTreeSet::new();
        for dependency in &entry.dependencies {
            if dependency.name.is_empty() {
                return Err(RegistryError::index(format!(
                    "a dependency of `{name}` {version} has no name"
                )));
            }
            dependency.requirement()?;
            if !aliases.insert(dependency.alias().to_owned()) {
                return Err(RegistryError::index(format!(
                    "`{name}` {version} declares the dependency alias `{}` twice",
                    dependency.alias()
                )));
            }
            for feature in &dependency.features {
                if feature.is_empty() || feature.contains('/') {
                    return Err(RegistryError::index(format!(
                        "the dependency feature `{feature}` of `{name}` {version} is invalid"
                    )));
                }
            }
        }
        for feature in entry.features.keys() {
            if feature.is_empty() || feature.contains('/') {
                return Err(RegistryError::index(format!(
                    "the feature `{feature}` of `{name}` {version} is invalid"
                )));
            }
        }
    }
    Ok(())
}

/// Validates a `sha256:<64 lowercase hex>` digest.
pub fn validate_checksum(checksum: &str) -> RegistryResult<()> {
    let Some(hex) = checksum.strip_prefix("sha256:") else {
        return Err(RegistryError::integrity(format!(
            "the checksum `{checksum}` does not use `sha256:`"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RegistryError::integrity(format!(
            "the checksum `{checksum}` is not a SHA-256 digest"
        )));
    }
    if hex.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(RegistryError::integrity(format!(
            "the checksum `{checksum}` must use lowercase hexadecimal"
        )));
    }
    Ok(())
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sparse_paths_follow_the_standard_layout() {
        assert_eq!(sparse_path("a").unwrap(), "index/1/a.json");
        assert_eq!(sparse_path("ab").unwrap(), "index/2/ab.json");
        assert_eq!(sparse_path("abc").unwrap(), "index/3/a/abc.json");
        assert_eq!(sparse_path("serde").unwrap(), "index/se/rd/serde.json");
        assert_eq!(sparse_path("@scope/pkg").unwrap(), "index/@scope/pkg.json");
    }

    #[test]
    fn an_invalid_package_name_is_rejected() {
        assert!(sparse_path("../secret").is_err());
        assert!(sparse_path("@scope").is_err());
    }

    #[test]
    fn checksums_must_be_lowercase_sha256() {
        assert!(validate_checksum(&format!("sha256:{}", "a".repeat(64))).is_ok());
        assert!(validate_checksum(&format!("sha256:{}", "A".repeat(64))).is_err());
        assert!(validate_checksum("sha256:00").is_err());
    }
}
