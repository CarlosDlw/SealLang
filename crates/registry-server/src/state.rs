//! Persistent registry state and signing keys.
//!
//! Metadata lives in a redb table as one versioned JSON document. Archives and
//! other large objects live in the content-addressed store; the database only
//! records their digests and relative paths. Signing seeds live in a separate
//! file with mode `0600`, so the metadata can be inspected without exposing keys.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use redb::{Database, Durability, TableDefinition, TableError};
use seal_registry::index::{IndexDependency, LogInclusion, Publisher};
use seal_registry::log::LogEntry;
use serde::{Deserialize, Serialize};

use crate::config::parse_seed;
use crate::error::{ServerError, ServerResult};

/// The metadata schema version.
pub const FORMAT: u32 = 1;

/// The redb table and key holding the metadata document.
const STATE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata");
const STATE_KEY: &str = "state";

/// The registry's signing seeds.
#[derive(Clone, Serialize, Deserialize)]
pub struct Keys {
    /// The root role seed.
    pub root: String,
    /// The targets role seed.
    pub targets: String,
    /// The snapshot role seed.
    pub snapshot: String,
    /// The timestamp role seed.
    pub timestamp: String,
    /// The transparency-log seed.
    pub log: String,
}

impl std::fmt::Debug for Keys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Keys([redacted])")
    }
}

impl Keys {
    /// Creates keys from the five seeds.
    pub fn from_seeds(seeds: &[[u8; 32]; 5]) -> Self {
        Self {
            root: hex::encode(seeds[0]),
            targets: hex::encode(seeds[1]),
            snapshot: hex::encode(seeds[2]),
            timestamp: hex::encode(seeds[3]),
            log: hex::encode(seeds[4]),
        }
    }

    /// Generates fresh keys from the operating system.
    pub fn generate() -> ServerResult<Self> {
        let mut seeds = [[0u8; 32]; 5];
        for seed in &mut seeds {
            *seed = random_seed()?;
        }
        Ok(Self::from_seeds(&seeds))
    }

    /// Returns one seed.
    pub fn seed(&self, role: &str) -> ServerResult<[u8; 32]> {
        let text = match role {
            "root" => &self.root,
            "targets" => &self.targets,
            "snapshot" => &self.snapshot,
            "timestamp" => &self.timestamp,
            "log" => &self.log,
            _ => {
                return Err(ServerError::internal(format!(
                    "unknown signing role `{role}`"
                )));
            }
        };
        parse_seed(text)
    }

    /// Loads keys from a file or creates them with mode `0600`.
    pub fn load_or_create(path: &Path, configured: Option<[[u8; 32]; 5]>) -> ServerResult<Self> {
        if path.is_file() {
            let text = std::fs::read_to_string(path).map_err(|error| {
                ServerError::internal(format!("cannot read `{}`: {error}", path.display()))
            })?;
            let keys: Self = serde_json::from_str(&text).map_err(|error| {
                ServerError::internal(format!(
                    "invalid signing keys `{}`: {error}",
                    path.display()
                ))
            })?;
            for role in ["root", "targets", "snapshot", "timestamp", "log"] {
                keys.seed(role)?;
            }
            return Ok(keys);
        }
        let keys = match configured {
            Some(seeds) => Self::from_seeds(&seeds),
            None => Self::generate()?,
        };
        let text = serde_json::to_vec_pretty(&keys).map_err(|error| {
            ServerError::internal(format!("cannot encode signing keys: {error}"))
        })?;
        write_private(path, &text)?;
        Ok(keys)
    }

    /// Replaces every seed after a root rotation.
    pub fn rotate(&mut self) -> ServerResult<()> {
        let mut seeds = [[0u8; 32]; 5];
        for seed in &mut seeds {
            *seed = random_seed()?;
        }
        *self = Self::from_seeds(&seeds);
        Ok(())
    }

    /// Persists the keys.
    pub fn save(&self, path: &Path) -> ServerResult<()> {
        let text = serde_json::to_vec_pretty(self).map_err(|error| {
            ServerError::internal(format!("cannot encode signing keys: {error}"))
        })?;
        write_private(path, &text)
    }
}

/// One owned namespace.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NamespaceRecord {
    /// Owner identities.
    pub owners: BTreeSet<String>,
    /// `config`, `dns`, or `oidc`.
    pub verified: String,
}

/// One published version.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VersionRecord {
    /// The archive digest.
    pub checksum: String,
    /// The archive path relative to the public root.
    pub archive: String,
    /// The archive size.
    pub archive_size: u64,
    /// The provenance digest.
    pub provenance: String,
    /// The provenance path relative to the public root.
    pub provenance_path: String,
    /// The SBOM digest.
    pub sbom: String,
    /// The SBOM path relative to the public root.
    pub sbom_path: String,
    /// The API snapshot digest.
    pub api: String,
    /// The API snapshot path relative to the public root.
    pub api_path: String,
    /// Whether the version is yanked.
    pub yanked: bool,
    /// The license expression.
    pub license: String,
    /// The description.
    pub description: Option<String>,
    /// The repository URL.
    pub repository: Option<String>,
    /// The declared dependencies.
    pub dependencies: Vec<IndexDependency>,
    /// The declared features.
    pub features: BTreeMap<String, Vec<String>>,
    /// The signing identity.
    pub publisher: Publisher,
    /// The transparency entry covering the publication.
    pub log: LogInclusion,
    /// The RFC 3339 publication time.
    pub published_at: String,
}

/// One package and its versions.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PackageRecord {
    /// Versions keyed by their canonical semantic version.
    pub versions: BTreeMap<String, VersionRecord>,
}

/// One dynamically created token.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenRecord {
    /// The token name.
    pub name: String,
    /// The `sha256:<hex>` digest.
    pub hash: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// Namespaces the token may manage.
    pub namespaces: Vec<String>,
    /// Whether the token has every permission.
    pub admin: bool,
    /// The RFC 3339 creation time.
    pub created_at: String,
}

/// One audit event.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuditEvent {
    /// The RFC 3339 time.
    pub at: String,
    /// The acting identity.
    pub actor: String,
    /// The action name.
    pub action: String,
    /// The package, when any.
    #[serde(default)]
    pub package: Option<String>,
    /// The version, when any.
    #[serde(default)]
    pub version: Option<String>,
    /// A short detail.
    pub detail: String,
}

/// Daily quota usage for one namespace.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsageRecord {
    /// The UTC day, `YYYY-MM-DD`.
    pub day: String,
    /// Published archive bytes.
    pub bytes: u64,
}

/// Everything the server persists.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct State {
    /// The metadata schema version.
    pub format: u32,
    /// Namespaces and their owners.
    pub namespaces: BTreeMap<String, NamespaceRecord>,
    /// Packages and versions.
    pub packages: BTreeMap<String, PackageRecord>,
    /// Dynamic tokens keyed by digest.
    pub tokens: BTreeMap<String, TokenRecord>,
    /// The transparency log, in append order.
    pub log: Vec<LogEntry>,
    /// Audit events, in append order.
    pub audit: Vec<AuditEvent>,
    /// Daily usage by namespace.
    pub usage: BTreeMap<String, UsageRecord>,
    /// The number of root rotations.
    pub rotations: u64,
    /// The next token sequence number.
    pub next_token: u64,
}

impl Default for State {
    fn default() -> Self {
        Self {
            format: FORMAT,
            namespaces: BTreeMap::new(),
            packages: BTreeMap::new(),
            tokens: BTreeMap::new(),
            log: Vec::new(),
            audit: Vec::new(),
            usage: BTreeMap::new(),
            rotations: 0,
            next_token: 1,
        }
    }
}
impl State {
    /// Loads metadata, or returns an empty state when the database is absent.
    pub fn load(path: &Path) -> ServerResult<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let database = Database::create(path).map_err(|error| {
            ServerError::internal(format!("cannot open `{}`: {error}", path.display()))
        })?;
        let transaction = database.begin_read().map_err(|error| {
            ServerError::internal(format!("cannot read `{}`: {error}", path.display()))
        })?;
        let table = match transaction.open_table(STATE_TABLE) {
            Ok(table) => table,
            Err(TableError::TableDoesNotExist(_)) => return Ok(Self::default()),
            Err(error) => {
                return Err(ServerError::internal(format!(
                    "cannot read the state table in `{}`: {error}",
                    path.display()
                )));
            }
        };
        let value = table
            .get(STATE_KEY)
            .map_err(|error| {
                ServerError::internal(format!(
                    "cannot read the state row in `{}`: {error}",
                    path.display()
                ))
            })?
            .ok_or_else(|| {
                ServerError::internal(format!(
                    "the registry state in `{}` has no metadata row",
                    path.display()
                ))
            })?;
        let state: Self = serde_json::from_slice(value.value()).map_err(|error| {
            ServerError::internal(format!(
                "invalid registry state `{}`: {error}",
                path.display()
            ))
        })?;
        if state.format != FORMAT {
            return Err(ServerError::internal(format!(
                "the registry state uses format {}, but this server understands {FORMAT}",
                state.format
            )));
        }
        Ok(state)
    }

    /// Persists metadata in a durable redb transaction.
    pub fn save(&self, path: &Path) -> ServerResult<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec(self)
            .map_err(|error| ServerError::internal(format!("cannot encode state: {error}")))?;
        let database = Database::create(path).map_err(|error| {
            ServerError::internal(format!("cannot open `{}`: {error}", path.display()))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(
                |error| {
                    ServerError::internal(format!("cannot secure `{}`: {error}", path.display()))
                },
            )?;
        }
        let mut transaction = database.begin_write().map_err(|error| {
            ServerError::internal(format!("cannot write `{}`: {error}", path.display()))
        })?;
        transaction.set_durability(Durability::Immediate);
        {
            let mut table = transaction.open_table(STATE_TABLE).map_err(|error| {
                ServerError::internal(format!("cannot open the state table: {error}"))
            })?;
            table
                .insert(STATE_KEY, bytes.as_slice())
                .map_err(|error| ServerError::internal(format!("cannot store state: {error}")))?;
        }
        transaction
            .commit()
            .map_err(|error| ServerError::internal(format!("cannot commit state: {error}")))
    }

    /// Records an audit event.
    pub fn audit(
        &mut self,
        at: &str,
        actor: &str,
        action: &str,
        package: Option<&str>,
        version: Option<&str>,
        detail: impl Into<String>,
    ) {
        self.audit.push(AuditEvent {
            at: at.to_owned(),
            actor: actor.to_owned(),
            action: action.to_owned(),
            package: package.map(ToOwned::to_owned),
            version: version.map(ToOwned::to_owned),
            detail: detail.into(),
        });
    }
}

fn random_seed() -> ServerResult<[u8; 32]> {
    let mut seed = [0u8; 32];
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut file = std::fs::File::open("/dev/urandom").map_err(|error| {
            ServerError::internal(format!("cannot open the system random source: {error}"))
        })?;
        file.read_exact(&mut seed).map_err(|error| {
            ServerError::internal(format!("cannot read the system random source: {error}"))
        })?;
        Ok(seed)
    }
    #[cfg(not(unix))]
    {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
                .to_be_bytes(),
        );
        hasher.update(std::process::id().to_be_bytes());
        hasher.update(crate::http::random_nonce().to_be_bytes());
        seed.copy_from_slice(&hasher.finalize());
        Ok(seed)
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> ServerResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all().ok();
    }
    #[cfg(not(unix))]
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// Returns the state database path below a storage root.
pub fn state_path(root: &Path) -> PathBuf {
    root.join("state.redb")
}

/// Returns the keys path below a storage root.
pub fn keys_path(root: &Path) -> PathBuf {
    root.join("keys.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_through_a_file() {
        let root = std::env::temp_dir().join(format!(
            "seal-server-keys-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = keys_path(&root);
        let keys = Keys::generate().unwrap();
        keys.save(&path).unwrap();
        let loaded = Keys::load_or_create(&path, None).unwrap();
        assert_eq!(loaded.root, keys.root);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_unknown_state_format_is_rejected() {
        let root = std::env::temp_dir().join(format!(
            "seal-server-state-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let path = state_path(&root);
        let state = State {
            format: 9,
            ..State::default()
        };
        state.save(&path).unwrap();
        assert!(State::load(&path).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
