//! The persistent registry: metadata, CAS objects, public index, and TUF state.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use seal_registry::index::{Catalog, CatalogEntry, IndexPackage, IndexVersion};
use semver::Version;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::auth::{Principal, configured_tokens, hash_token};
use crate::config::{ServerConfig, package_namespace};
use crate::error::{ServerError, ServerResult};
use crate::state::{Keys, NamespaceRecord, State, keys_path, state_path};
use crate::tuf::{self, TufFiles};

/// The open registry.
pub struct Store {
    config: ServerConfig,
    root: PathBuf,
    public: PathBuf,
    state: RwLock<State>,
    keys: RwLock<Keys>,
    mutation: Mutex<()>,
}

impl Store {
    /// Opens or creates the registry below the configured storage directory.
    pub fn open(config: ServerConfig) -> ServerResult<Self> {
        let root = config.storage.clone();
        let public = root.join("public");
        std::fs::create_dir_all(root.join("blobs"))?;
        std::fs::create_dir_all(root.join("tmp"))?;
        std::fs::create_dir_all(public.join("index"))?;
        std::fs::create_dir_all(public.join("packages"))?;
        std::fs::create_dir_all(public.join("tuf"))?;
        let configured = config.tuf.seeds()?;
        let keys = Keys::load_or_create(&keys_path(&root), configured)?;
        let mut state = State::load(&state_path(&root))?;
        merge_config(&mut state, &config);
        state.save(&state_path(&root))?;
        let store = Self {
            config,
            root,
            public,
            state: RwLock::new(state),
            keys: RwLock::new(keys),
            mutation: Mutex::new(()),
        };
        store.regenerate()?;
        Ok(store)
    }

    /// Returns the effective configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Returns a read lock on metadata.
    pub fn state(&self) -> RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|error| error.into_inner())
    }

    /// Returns a read lock on the signing keys.
    pub fn keys(&self) -> RwLockReadGuard<'_, Keys> {
        self.keys.read().unwrap_or_else(|error| error.into_inner())
    }

    /// Returns the storage root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the state path.
    pub(crate) fn state_path(&self) -> PathBuf {
        state_path(&self.root)
    }

    /// Serializes mutations across publication, ownership, and rotation.
    pub(crate) fn mutation_guard(&self) -> MutexGuard<'_, ()> {
        self.mutation
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Returns a write lock on metadata.
    pub(crate) fn state_mut(&self) -> RwLockWriteGuard<'_, State> {
        self.state
            .write()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// Writes a file below the public root.
    pub(crate) fn write_artifact(&self, relative: &str, bytes: &[u8]) -> ServerResult<()> {
        self.write_public(relative, bytes)
    }

    /// Regenerates the public metadata from a state the caller already holds.
    pub(crate) fn regenerate_locked(&self, state: &State) -> ServerResult<()> {
        let keys = self.keys.read().unwrap_or_else(|error| error.into_inner());
        self.regenerate_with(state, &keys, None)
    }

    /// Returns the public root.
    pub fn public(&self) -> &Path {
        &self.public
    }

    /// Returns the trusted root bytes.
    pub fn trusted_root(&self) -> ServerResult<Vec<u8>> {
        std::fs::read(self.public.join("tuf/root.json"))
            .map_err(|error| ServerError::internal(format!("cannot read `root.json`: {error}")))
    }

    /// Resolves namespace ownership and scope for one action.
    pub fn authorize(
        &self,
        principal: &Principal,
        action: &str,
        namespace: &str,
    ) -> ServerResult<()> {
        if !principal.can(action, namespace) {
            return Err(ServerError::forbidden(format!(
                "the identity `{}` lacks the `{action}` scope for `{namespace}`",
                principal.name
            )));
        }
        if principal.admin {
            return Ok(());
        }
        let state = self.state();
        if self
            .config
            .preapproved_namespaces
            .iter()
            .any(|value| value == namespace)
        {
            return Ok(());
        }
        let owners = state
            .namespaces
            .get(namespace)
            .map(|record| record.owners.clone())
            .unwrap_or_default();
        if owners.is_empty() {
            return Err(ServerError::forbidden(format!(
                "the namespace `{namespace}` is not owned by anyone; claim it first"
            )));
        }
        crate::auth::authorize_namespace(principal, action, namespace, &owners)
    }

    /// Returns the namespace of a package and checks ownership.
    pub fn package_namespace(&self, name: &str) -> ServerResult<String> {
        package_namespace(name, self.config.default_namespace.as_deref())
    }

    /// Rebuilds the index, catalog, advisories, artifacts, and TUF metadata.
    pub fn regenerate(&self) -> ServerResult<()> {
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        let keys = self.keys.read().unwrap_or_else(|error| error.into_inner());
        self.regenerate_with(&state, &keys, None)
    }

    fn regenerate_with(
        &self,
        state: &State,
        keys: &Keys,
        old_root: Option<&[u8; 32]>,
    ) -> ServerResult<()> {
        for (name, package) in &state.packages {
            let path = seal_registry::index::sparse_path(name).map_err(ServerError::from)?;
            let index = IndexPackage {
                format: 1,
                name: name.clone(),
                versions: package
                    .versions
                    .iter()
                    .map(|(version, record)| {
                        (
                            version.clone(),
                            IndexVersion {
                                checksum: record.checksum.clone(),
                                archive: record.archive.clone(),
                                yanked: record.yanked,
                                license: Some(record.license.clone()),
                                description: record.description.clone(),
                                repository: record.repository.clone(),
                                provenance: Some(record.provenance.clone()),
                                provenance_file: Some(record.provenance_path.clone()),
                                sbom: Some(record.sbom.clone()),
                                sbom_file: Some(record.sbom_path.clone()),
                                api: Some(seal_registry::index::ApiEntry {
                                    hash: record.api.clone(),
                                    snapshot: record.api_path.clone(),
                                }),
                                publisher: Some(record.publisher.clone()),
                                log: Some(record.log.clone()),
                                published_at: Some(record.published_at.clone()),
                                dependencies: record.dependencies.clone(),
                                features: record.features.clone(),
                            },
                        )
                    })
                    .collect(),
            };
            let bytes = serde_json::to_vec_pretty(&index).map_err(|error| {
                ServerError::internal(format!("cannot encode `{path}`: {error}"))
            })?;
            self.write_public(&path, &bytes)?;
            self.ensure_artifacts(state, name, &package.versions)?;
        }

        let mut entries = Vec::new();
        for (name, package) in &state.packages {
            let latest = package
                .versions
                .iter()
                .filter_map(|(version, record)| {
                    Version::parse(version)
                        .ok()
                        .filter(|version| version.pre.is_empty() && !record.yanked)
                        .map(|version| (version, record))
                })
                .max_by(|left, right| left.0.cmp(&right.0));
            let (latest, description, license) =
                latest.map_or((None, None, None), |(version, record)| {
                    (
                        Some(version.to_string()),
                        record.description.clone(),
                        Some(record.license.clone()),
                    )
                });
            entries.push(CatalogEntry {
                name: name.clone(),
                latest,
                description,
                license,
            });
        }
        let catalog = Catalog {
            format: 1,
            packages: entries,
        };
        let bytes = serde_json::to_vec_pretty(&catalog).map_err(|error| {
            ServerError::internal(format!("cannot encode the catalog: {error}"))
        })?;
        self.write_public("index/catalog.json", &bytes)?;

        let advisories_path = self.public.join("advisories.json");
        if !advisories_path.is_file() {
            let bytes = serde_json::to_vec_pretty(&json!({
                "format": 1,
                "advisories": []
            }))
            .map_err(|error| ServerError::internal(format!("cannot encode advisories: {error}")))?;
            self.write_public("advisories.json", &bytes)?;
        }

        let targets = self.public_targets()?;
        let metadata_version = state.log.len().max(1) as u64;
        let root_version = state.rotations + 1;
        let files = tuf::generate(keys, &targets, root_version, metadata_version, old_root)?;
        self.write_tuf(&files)?;
        Ok(())
    }

    fn ensure_artifacts(
        &self,
        state: &State,
        name: &str,
        versions: &BTreeMap<String, crate::state::VersionRecord>,
    ) -> ServerResult<()> {
        let _ = state;
        for (version, record) in versions {
            let directory = format!("packages/{name}/{version}");
            for (path, digest) in [
                (&record.archive, &record.checksum),
                (&record.provenance_path, &record.provenance),
                (&record.sbom_path, &record.sbom),
                (&record.api_path, &record.api),
            ] {
                if self.public.join(path).is_file() {
                    continue;
                }
                let Some(hex) = digest.strip_prefix("sha256:") else {
                    return Err(ServerError::internal(format!(
                        "the record for `{name}` {version} has an invalid digest"
                    )));
                };
                let source = self.root.join("blobs").join(hex);
                let bytes = std::fs::read(&source).map_err(|error| {
                    ServerError::internal(format!(
                        "the content-addressed object for `{name}` {version} is missing: {error}"
                    ))
                })?;
                let expected = path.strip_prefix(&format!("{directory}/")).unwrap_or(path);
                debug_assert!(!expected.is_empty());
                self.write_public(path, &bytes)?;
            }
        }
        Ok(())
    }

    fn public_targets(&self) -> ServerResult<BTreeMap<String, Vec<u8>>> {
        let mut targets = BTreeMap::new();
        collect(&self.public, &self.public, &mut targets)?;
        targets.remove("tuf/root.json");
        targets.remove("tuf/targets.json");
        targets.remove("tuf/snapshot.json");
        targets.remove("tuf/timestamp.json");
        Ok(targets)
    }

    fn write_tuf(&self, files: &TufFiles) -> ServerResult<()> {
        for (name, bytes) in files.entries() {
            self.write_public(&format!("tuf/{name}"), bytes)?;
        }
        Ok(())
    }

    fn write_public(&self, relative: &str, bytes: &[u8]) -> ServerResult<()> {
        let path = safe_join(&self.public, relative)?;
        write_atomic(&path, bytes)
    }

    /// Stores bytes in the content-addressed store and returns their digest.
    pub(crate) fn write_blob(&self, bytes: &[u8]) -> ServerResult<String> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let hex = hex::encode(hasher.finalize());
        let path = self.root.join("blobs").join(&hex);
        if path.is_file() {
            let existing = std::fs::read(&path)?;
            if existing.len() != bytes.len() {
                return Err(ServerError::internal(format!(
                    "the content-addressed object `{hex}` has a different length"
                )));
            }
            return Ok(format!("sha256:{hex}"));
        }
        write_atomic(&path, bytes)?;
        Ok(format!("sha256:{hex}"))
    }

    /// Returns a static public file by its URL path.
    pub fn static_file(&self, path: &str) -> Option<Vec<u8>> {
        let relative = path.trim_start_matches('/');
        if relative.is_empty()
            || relative
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            || relative.contains('\\')
            || relative.contains('\0')
        {
            return None;
        }
        if !(relative.starts_with("index/")
            || relative.starts_with("packages/")
            || relative.starts_with("tuf/")
            || relative == "advisories.json")
        {
            return None;
        }
        let path = safe_join(&self.public, relative).ok()?;
        if !path.is_file() {
            return None;
        }
        std::fs::read(path).ok()
    }

    /// Builds the signed tree head for the current log.
    pub fn tree_head(&self) -> ServerResult<seal_registry::log::SignedTreeHead> {
        let state = self.state();
        let keys = self.keys();
        tree_head(&state, &keys)
    }

    /// Returns an inclusion proof for one log entry.
    pub fn log_proof(&self, index: u64) -> ServerResult<seal_registry::log::LogProof> {
        let state = self.state();
        let keys = self.keys();
        let entry = state
            .log
            .get(index as usize)
            .cloned()
            .ok_or_else(|| ServerError::not_found(format!("the log has no entry {index}")))?;
        let tree = seal_registry::log::LogTree::from_entries(&state.log)?;
        let proof = tree.proof(index as usize)?;
        let head = tree_head(&state, &keys)?;
        Ok(seal_registry::log::LogProof { head, entry, proof })
    }

    /// Records a claim on a namespace.
    pub fn claim_namespace(
        &self,
        principal: &Principal,
        namespace: &str,
        kind: &str,
    ) -> ServerResult<NamespaceRecord> {
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(existing) = state.namespaces.get(namespace) {
            if existing.owners.contains(&principal.name) || principal.admin {
                return Ok(existing.clone());
            }
            if self
                .config
                .preapproved_namespaces
                .iter()
                .any(|value| value == namespace)
                || principal.oidc
            {
                let record = state
                    .namespaces
                    .get_mut(namespace)
                    .expect("the namespace was just found");
                record.owners.insert(principal.name.clone());
                let record = record.clone();
                state.save(&self.state_path())?;
                return Ok(record);
            }
            return Err(ServerError::forbidden(format!(
                "the namespace `{namespace}` already has owners"
            )));
        }
        let record = NamespaceRecord {
            owners: BTreeSet::from([principal.name.clone()]),
            verified: kind.to_owned(),
        };
        state
            .namespaces
            .insert(namespace.to_owned(), record.clone());
        let now = now();
        state.audit(
            &now,
            &principal.name,
            "namespace-claim",
            None,
            None,
            format!("claimed `{namespace}` with `{kind}`"),
        );
        state.save(&state_path(&self.root))?;
        Ok(record)
    }

    /// Adds or removes a namespace owner.
    pub fn owners(
        &self,
        principal: &Principal,
        namespace: &str,
        action: &str,
        owner: Option<&str>,
    ) -> ServerResult<NamespaceRecord> {
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let owners = state
            .namespaces
            .get(namespace)
            .map(|record| record.owners.clone())
            .unwrap_or_default();
        crate::auth::authorize_namespace(principal, "owners", namespace, &owners)?;
        let record = state.namespaces.get_mut(namespace).ok_or_else(|| {
            ServerError::not_found(format!("the namespace `{namespace}` is unknown"))
        })?;
        let owner = owner
            .ok_or_else(|| ServerError::bad_request("`owner` is required for add and remove"))?;
        if owner.is_empty() || owner.chars().any(char::is_control) {
            return Err(ServerError::bad_request("the owner identity is invalid"));
        }
        match action {
            "add" => {
                record.owners.insert(owner.to_owned());
            }
            "remove" => {
                if record.owners.len() == 1 {
                    return Err(ServerError::bad_request(
                        "a namespace must keep at least one owner",
                    ));
                }
                if !record.owners.remove(owner) {
                    return Err(ServerError::not_found(format!(
                        "`{owner}` is not an owner of `{namespace}`"
                    )));
                }
            }
            other => {
                return Err(ServerError::bad_request(format!(
                    "the owner action `{other}` is not `add` or `remove`"
                )));
            }
        }
        let record = record.clone();
        let now = now();
        state.audit(
            &now,
            &principal.name,
            &format!("owner-{action}"),
            None,
            None,
            format!("`{owner}` on `{namespace}`"),
        );
        state.save(&state_path(&self.root))?;
        Ok(record)
    }

    /// Creates a token and returns its one-time secret.
    pub fn create_token(
        &self,
        principal: &Principal,
        name: &str,
        scopes: Vec<String>,
        namespaces: Vec<String>,
    ) -> ServerResult<(String, crate::state::TokenRecord)> {
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(ServerError::bad_request("the token name is invalid"));
        }
        let scopes = crate::auth::token_scopes(principal, &scopes)?;
        for namespace in &namespaces {
            crate::config::validate_namespace(namespace)?;
            self.authorize(principal, "owners", namespace)?;
        }
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let id = state.next_token;
        state.next_token += 1;
        let secret = crate::auth::generate_token(id)?;
        let record = crate::state::TokenRecord {
            name: name.to_owned(),
            hash: hash_token(&secret),
            scopes,
            namespaces,
            admin: false,
            created_at: now(),
        };
        state.tokens.insert(record.hash.clone(), record.clone());
        state.audit(
            &now(),
            &principal.name,
            "token-create",
            None,
            None,
            format!("created `{name}`"),
        );
        state.save(&state_path(&self.root))?;
        Ok((secret, record))
    }

    /// Rotates every signing key and returns the new root document.
    pub fn rotate(&self, principal: &Principal) -> ServerResult<Value> {
        if !principal.admin {
            return Err(ServerError::forbidden(
                "only an administrator may rotate TUF keys",
            ));
        }
        let _guard = self
            .mutation
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        let mut keys = self.keys.write().unwrap_or_else(|error| error.into_inner());
        let old = keys.clone();
        let old_seed = old.seed("root")?;
        keys.rotate()?;
        state.rotations += 1;
        if let Err(error) = self.regenerate_with(&state, &keys, Some(&old_seed)) {
            *keys = old;
            state.rotations -= 1;
            return Err(error);
        }
        keys.save(&keys_path(&self.root))?;
        let version = state.rotations + 1;
        state.audit(
            &now(),
            &principal.name,
            "tuf-rotate",
            None,
            None,
            format!("root version {version}"),
        );
        state.save(&state_path(&self.root))?;
        let root: Value = serde_json::from_slice(&self.trusted_root()?).map_err(|error| {
            ServerError::internal(format!("cannot decode the new root: {error}"))
        })?;
        Ok(root)
    }
}

fn merge_config(state: &mut State, config: &ServerConfig) {
    let records = configured_tokens(config).unwrap_or_default();
    for record in records {
        state
            .tokens
            .retain(|_, existing| existing.name != record.name);
        state.tokens.insert(record.hash.clone(), record);
    }
    for namespace in &config.preapproved_namespaces {
        state
            .namespaces
            .entry(namespace.clone())
            .and_modify(|record| {
                record.owners.insert("config".to_owned());
            })
            .or_insert_with(|| NamespaceRecord {
                owners: BTreeSet::from(["config".to_owned()]),
                verified: "config".to_owned(),
            });
    }
}

fn tree_head(state: &State, keys: &Keys) -> ServerResult<seal_registry::log::SignedTreeHead> {
    let tree = seal_registry::log::LogTree::from_entries(&state.log)?;
    let root = tree.root();
    let seed = keys.seed("log")?;
    Ok(seal_registry::log::sign_tree_head(
        tree.len() as u64,
        &root,
        &now(),
        &seed,
    )?)
}

/// Returns the current time as RFC 3339.
pub fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned())
}

/// Returns the current UTC day.
pub fn day() -> String {
    OffsetDateTime::now_utc().date().to_string()
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> ServerResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all().ok();
    }
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn safe_join(root: &Path, relative: &str) -> ServerResult<PathBuf> {
    if relative.is_empty()
        || relative.starts_with('/')
        || relative.contains('\\')
        || relative
            .split('/')
            .any(|segment| segment == ".." || segment == ".")
    {
        return Err(ServerError::internal(format!(
            "the public path `{relative}` is not safe"
        )));
    }
    Ok(root.join(relative))
}

fn collect(
    root: &Path,
    directory: &Path,
    targets: &mut BTreeMap<String, Vec<u8>>,
) -> ServerResult<()> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(ServerError::internal(format!(
                "the public tree contains the symlink `{}`",
                path.display()
            )));
        }
        if file_type.is_dir() {
            collect(root, &path, targets)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(ServerError::internal(format!(
                "the public tree contains a special file `{}`",
                path.display()
            )));
        }
        let relative = path
            .strip_prefix(root)
            .map_err(|_| ServerError::internal("a public file is outside the public root"))?;
        let relative = relative
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(value) => Some(value.to_string_lossy()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(&path)?;
        targets.insert(relative, bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(root: PathBuf) -> ServerConfig {
        ServerConfig {
            storage: root,
            preapproved_namespaces: vec!["demo".to_owned()],
            tokens: vec![crate::config::TokenConfig {
                name: "admin".to_owned(),
                token: Some("secret".to_owned()),
                hash: None,
                scopes: vec!["admin".to_owned()],
                namespaces: Vec::new(),
                admin: true,
            }],
            ..ServerConfig::default()
        }
    }

    #[test]
    fn opening_a_store_creates_index_catalog_and_tuf() {
        let root = std::env::temp_dir().join(format!(
            "seal-store-{}-{}",
            std::process::id(),
            crate::http::random_nonce()
        ));
        let store = Store::open(config(root.clone())).unwrap();
        assert!(store.public().join("index/catalog.json").is_file());
        assert!(store.public().join("tuf/root.json").is_file());
        assert!(store.public().join("advisories.json").is_file());
        assert_eq!(store.state().namespaces["demo"].verified, "config");
        let _ = std::fs::remove_dir_all(root);
    }
}
