//! Publication and yank transactions.

use std::path::{Path, PathBuf};

use seal_manifest::DependencySpec;
use seal_registry::index::{IndexDependency, LogInclusion};
use seal_registry::log::LogEntry;
use seal_registry::provenance::{self, VerifyOptions};
use seal_registry::publish::{
    PublishRequest, PublishResponse, YankRequest, YankResponse, decode_archive, document_digest,
};
use semver::Version;

use crate::auth::Principal;
use crate::error::{ServerError, ServerResult};
use crate::state::{UsageRecord, VersionRecord};
use crate::store::{Store, day, digest, now, write_atomic};

/// Validates and publishes one version.
pub fn publish(
    store: &Store,
    principal: &Principal,
    request: &PublishRequest,
) -> ServerResult<PublishResponse> {
    let namespace = store.package_namespace(&request.name)?;
    store.authorize(principal, "publish", &namespace)?;
    let version = Version::parse(&request.version).map_err(|error| {
        ServerError::bad_request(format!(
            "`{}` is not a semantic version: {error}",
            request.version
        ))
    })?;
    if version.to_string() != request.version {
        return Err(ServerError::bad_request(format!(
            "the version `{}` is not canonical; write `{version}`",
            request.version
        )));
    }
    let archive = decode_archive(&request.archive, store.config().max_body)?;
    let checksum = digest(&archive);
    let checksum_hex = checksum.trim_start_matches("sha256:");
    request
        .api
        .validate()
        .map_err(|error| ServerError::bad_request(error.message()))?;
    if request.api.package != request.name || request.api.package_version != request.version {
        return Err(ServerError::bad_request(format!(
            "the API snapshot names `{}` {}, but the request is `{}` {}",
            request.api.package, request.api.package_version, request.name, request.version
        )));
    }
    let api_digest = request
        .api
        .hash()
        .map_err(|error| ServerError::bad_request(error.message()))?;
    let sbom_digest = document_digest(&request.sbom).map_err(ServerError::from)?;
    if request.sbom["bomFormat"] != "CycloneDX" || request.sbom["specVersion"] != "1.5" {
        return Err(ServerError::bad_request(
            "the SBOM is not a CycloneDX 1.5 document",
        ));
    }
    let keyid = expected_keyid(&request.publisher.public)?;
    if keyid != request.publisher.keyid {
        return Err(ServerError::provenance(
            "the publisher key id does not match its public key",
        ));
    }
    if principal.oidc && request.publisher.identity.as_deref() != principal.subject.as_deref() {
        return Err(ServerError::provenance(
            "the publisher identity does not match the authenticated OIDC subject",
        ));
    }
    let statement = provenance::verify(
        &request.provenance,
        &VerifyOptions {
            public: Some(&request.publisher.public),
            archive_sha256: checksum_hex,
            name: &request.name,
            version: &request.version,
            identity: principal.subject.as_deref(),
        },
    )
    .map_err(ServerError::from)?;
    let parameters = &statement.predicate.build_definition.external_parameters;
    if parameters.sbom != sbom_digest {
        return Err(ServerError::provenance(format!(
            "the provenance signs the SBOM `{}`, but the request carries `{sbom_digest}`",
            parameters.sbom
        )));
    }
    if parameters.api != api_digest {
        return Err(ServerError::provenance(format!(
            "the provenance signs the API `{}`, but the request carries `{api_digest}`",
            parameters.api
        )));
    }

    let temporary = Temporary::new(store.root())?;
    let archive_path = temporary.path.join("package.tar.zst");
    write_atomic(&archive_path, &archive)?;
    let extracted = temporary.path.join("package");
    seal_registry::archive::extract(&archive_path, &extracted).map_err(ServerError::from)?;
    let manifest = seal_manifest::load(&extracted.join("seal.toml")).map_err(|error| {
        ServerError::bad_request(format!("the archive has an invalid `seal.toml`: {error}"))
    })?;
    let package = manifest
        .manifest
        .package
        .as_ref()
        .ok_or_else(|| ServerError::bad_request("the archive has no `[package]` table"))?;
    if package.name != request.name || package.version != request.version {
        return Err(ServerError::bad_request(format!(
            "the archive contains `{}` {}, but the request is `{}` {}",
            package.name, package.version, request.name, request.version
        )));
    }
    let license = package
        .license
        .as_deref()
        .filter(|license| !license.trim().is_empty())
        .ok_or_else(|| ServerError::bad_request("a published package must declare `license`"))?;
    if let Some(readme) = &package.readme {
        let path = safe_relative(&extracted, readme)?;
        if !path.is_file() {
            return Err(ServerError::bad_request(format!(
                "the package declares the readme `{readme}`, but the archive has no such file"
            )));
        }
    }
    if !contains_source(&extracted)? {
        return Err(ServerError::bad_request(
            "the package archive has no `.sl` source file",
        ));
    }
    let embedded_api = std::fs::read(extracted.join("seal.api.json")).map_err(|error| {
        ServerError::bad_request(format!(
            "the archive has no readable `seal.api.json`: {error}"
        ))
    })?;
    let embedded_api = seal_api::ApiSnapshot::parse(&embedded_api)
        .map_err(|error| ServerError::bad_request(error.message().to_owned()))?;
    if embedded_api != request.api {
        return Err(ServerError::bad_request(
            "the archive's `seal.api.json` differs from the published API snapshot",
        ));
    }
    let embedded_sbom = std::fs::read(extracted.join("seal.sbom.json")).map_err(|error| {
        ServerError::bad_request(format!(
            "the archive has no readable `seal.sbom.json`: {error}"
        ))
    })?;
    let embedded_sbom: serde_json::Value = serde_json::from_slice(&embedded_sbom)
        .map_err(|error| ServerError::bad_request(format!("invalid embedded SBOM: {error}")))?;
    if embedded_sbom != request.sbom {
        return Err(ServerError::bad_request(
            "the archive's `seal.sbom.json` differs from the signed SBOM",
        ));
    }
    let dependencies = index_dependencies(&manifest.manifest.dependencies)?;
    let features = package
        .features
        .values()
        .map(|(name, values)| (name.to_owned(), values.to_vec()))
        .collect();

    let _guard = store.mutation_guard();
    let mut state = store.state_mut();
    if state
        .packages
        .get(&request.name)
        .is_some_and(|package| package.versions.contains_key(&request.version))
    {
        return Err(ServerError::conflict(format!(
            "`{}` {} is already published and immutable",
            request.name, request.version
        )));
    }
    let versions = state
        .packages
        .get(&request.name)
        .map(|package| package.versions.len())
        .unwrap_or(0);
    if store.config().max_versions_per_package > 0
        && versions >= store.config().max_versions_per_package
    {
        return Err(ServerError::quota(format!(
            "the package `{}` reached the {} version limit",
            request.name,
            store.config().max_versions_per_package
        )));
    }
    validate_quota(store, &state, &namespace, archive.len() as u64)?;
    if let Some(previous) = previous_version(&state, &request.name, &version) {
        let previous_api = read_blob(store, &previous.api)?;
        let previous = seal_api::ApiSnapshot::parse(&previous_api).map_err(|error| {
            ServerError::internal(format!("the stored API snapshot is invalid: {error}"))
        })?;
        seal_api::enforce(&previous, &request.api, &version)
            .map_err(|error| ServerError::bad_request(error.message()))?;
    }

    let api_bytes = request
        .api
        .canonical_bytes()
        .map_err(|error| ServerError::bad_request(error.message()))?;
    let sbom_bytes = serde_json::to_vec(&request.sbom)
        .map_err(|error| ServerError::internal(format!("cannot encode the SBOM: {error}")))?;
    let provenance_value = serde_json::to_value(&request.provenance)
        .map_err(|error| ServerError::internal(format!("cannot encode the provenance: {error}")))?;
    let provenance_bytes = seal_registry::tuf::canonical_json(&provenance_value);
    let provenance_digest = store.write_blob(&provenance_bytes)?;
    let sbom_stored = store.write_blob(&sbom_bytes)?;
    let api_stored = store.write_blob(&api_bytes)?;
    if sbom_stored != sbom_digest || api_stored != api_digest {
        return Err(ServerError::internal(
            "the stored artifact digest disagrees with the verified digest",
        ));
    }
    store.write_blob(&archive)?;
    let directory = format!("packages/{}/{}", request.name, request.version);
    let archive_path = format!(
        "{directory}/{}-{}.tar.zst",
        base_name(&request.name),
        request.version
    );
    let api_path = format!("{directory}/{}.api.json", request.version);
    let sbom_path = format!("{directory}/{}.sbom.json", request.version);
    let provenance_path = format!("{directory}/{}.provenance.json", request.version);
    store.write_artifact(&archive_path, &archive)?;
    store.write_artifact(&api_path, &api_bytes)?;
    store.write_artifact(&sbom_path, &sbom_bytes)?;
    store.write_artifact(&provenance_path, &provenance_bytes)?;

    let index = state.log.len() as u64;
    let entry = LogEntry {
        format: 1,
        index,
        at: now(),
        kind: "publish".to_owned(),
        package: request.name.clone(),
        version: request.version.clone(),
        checksum: checksum.clone(),
        provenance: Some(provenance_digest.clone()),
        sbom: Some(sbom_stored.clone()),
        api: Some(api_stored.clone()),
        publisher: Some(request.publisher.keyid.clone()),
        yanked: false,
    };
    state.log.push(entry);
    let inclusion = inclusion(&state.log, index)?;
    let published_at = now();
    let record = VersionRecord {
        checksum: checksum.clone(),
        archive: archive_path,
        archive_size: archive.len() as u64,
        provenance: provenance_digest.clone(),
        provenance_path,
        sbom: sbom_stored.clone(),
        sbom_path,
        api: api_stored.clone(),
        api_path,
        yanked: false,
        license: license.to_owned(),
        description: package.description.clone(),
        repository: package.repository.clone(),
        dependencies,
        features,
        publisher: request.publisher.clone(),
        log: inclusion.clone(),
        published_at: published_at.clone(),
    };
    state
        .packages
        .entry(request.name.clone())
        .or_default()
        .versions
        .insert(request.version.clone(), record);
    record_usage(&mut state, &namespace, archive.len() as u64);
    state.audit(
        &published_at,
        &principal.name,
        "publish",
        Some(&request.name),
        Some(&request.version),
        format!("checksum {checksum}"),
    );
    state.save(&store.state_path())?;
    store.regenerate_locked(&state)?;
    let head = tree_head(store, &state)?;
    Ok(PublishResponse {
        format: 1,
        name: request.name.clone(),
        version: request.version.clone(),
        checksum,
        api: api_stored,
        provenance: provenance_digest,
        sbom: sbom_stored,
        log: inclusion,
        tree_head: head,
        already_published: false,
    })
}

/// Yanks or un-yanks one version.
pub fn yank(
    store: &Store,
    principal: &Principal,
    request: &YankRequest,
) -> ServerResult<YankResponse> {
    let namespace = store.package_namespace(&request.name)?;
    store.authorize(principal, "yank", &namespace)?;
    Version::parse(&request.version).map_err(|error| {
        ServerError::bad_request(format!(
            "`{}` is not a semantic version: {error}",
            request.version
        ))
    })?;
    let _guard = store.mutation_guard();
    let mut state = store.state_mut();
    let previous = state
        .packages
        .get(&request.name)
        .and_then(|package| package.versions.get(&request.version))
        .cloned()
        .ok_or_else(|| {
            ServerError::not_found(format!(
                "`{}` {} is not published",
                request.name, request.version
            ))
        })?;
    if previous.yanked == request.yanked {
        return Err(ServerError::bad_request(format!(
            "`{}` {} is already {}",
            request.name,
            request.version,
            if request.yanked {
                "yanked"
            } else {
                "not yanked"
            }
        )));
    }
    let index = state.log.len() as u64;
    let entry = LogEntry {
        format: 1,
        index,
        at: now(),
        kind: "yank".to_owned(),
        package: request.name.clone(),
        version: request.version.clone(),
        checksum: previous.checksum.clone(),
        provenance: Some(previous.provenance.clone()),
        sbom: Some(previous.sbom.clone()),
        api: Some(previous.api.clone()),
        publisher: Some(previous.publisher.keyid.clone()),
        yanked: request.yanked,
    };
    state.log.push(entry);
    let inclusion = inclusion(&state.log, index)?;
    if let Some(package) = state.packages.get_mut(&request.name)
        && let Some(record) = package.versions.get_mut(&request.version)
    {
        record.yanked = request.yanked;
        record.log = inclusion.clone();
    }
    state.audit(
        &now(),
        &principal.name,
        if request.yanked { "yank" } else { "unyank" },
        Some(&request.name),
        Some(&request.version),
        String::new(),
    );
    state.save(&store.state_path())?;
    store.regenerate_locked(&state)?;
    Ok(YankResponse {
        name: request.name.clone(),
        version: request.version.clone(),
        yanked: request.yanked,
        log: inclusion,
    })
}

fn tree_head(
    store: &Store,
    state: &crate::state::State,
) -> ServerResult<seal_registry::log::SignedTreeHead> {
    let tree = seal_registry::log::LogTree::from_entries(&state.log)?;
    let keys = store.keys();
    let seed = keys.seed("log")?;
    Ok(seal_registry::log::sign_tree_head(
        tree.len() as u64,
        &tree.root(),
        &now(),
        &seed,
    )?)
}

fn inclusion(entries: &[LogEntry], index: u64) -> ServerResult<LogInclusion> {
    let tree = seal_registry::log::LogTree::from_entries(entries)?;
    let entry = entries
        .get(index as usize)
        .cloned()
        .ok_or_else(|| ServerError::internal("the log index is out of range"))?;
    Ok(LogInclusion {
        entry,
        index,
        tree_size: tree.len() as u64,
        root_hash: tree.root(),
        proof: tree.proof(index as usize)?,
    })
}

fn previous_version<'a>(
    state: &'a crate::state::State,
    name: &str,
    version: &Version,
) -> Option<&'a VersionRecord> {
    state
        .packages
        .get(name)?
        .versions
        .iter()
        .filter_map(|(text, record)| Version::parse(text).ok().map(|value| (value, record)))
        .filter(|(candidate, _)| candidate < version)
        .max_by(|left, right| left.0.cmp(&right.0))
        .map(|(_, record)| record)
}

fn validate_quota(
    store: &Store,
    state: &crate::state::State,
    namespace: &str,
    bytes: u64,
) -> ServerResult<()> {
    let limit = store.config().publish_bytes_per_day;
    if limit == 0 {
        return Ok(());
    }
    let today = day();
    let used = state
        .usage
        .get(namespace)
        .filter(|usage| usage.day == today)
        .map(|usage| usage.bytes)
        .unwrap_or(0);
    if used.saturating_add(bytes) > limit {
        return Err(ServerError::quota(format!(
            "the namespace `{namespace}` would exceed its daily publication quota of {limit} bytes"
        )));
    }
    Ok(())
}

fn record_usage(state: &mut crate::state::State, namespace: &str, bytes: u64) {
    let today = day();
    let usage = state.usage.entry(namespace.to_owned()).or_default();
    if usage.day != today {
        *usage = UsageRecord {
            day: today,
            bytes: 0,
        };
    }
    usage.bytes = usage.bytes.saturating_add(bytes);
}

fn read_blob(store: &Store, digest: &str) -> ServerResult<Vec<u8>> {
    let hex = digest.strip_prefix("sha256:").ok_or_else(|| {
        ServerError::internal(format!("the stored digest `{digest}` is not SHA-256"))
    })?;
    std::fs::read(store.root().join("blobs").join(hex)).map_err(|error| {
        ServerError::internal(format!("the stored object `{digest}` is missing: {error}"))
    })
}

fn index_dependencies(
    dependencies: &std::collections::BTreeMap<String, DependencySpec>,
) -> ServerResult<Vec<IndexDependency>> {
    let mut result = Vec::new();
    for (alias, dependency) in dependencies {
        let entry = match dependency {
            DependencySpec::Version(requirement) => IndexDependency {
                name: alias.clone(),
                alias: None,
                req: requirement.clone(),
                features: Vec::new(),
                default_features: true,
                optional: false,
                when: None,
            },
            DependencySpec::Detailed(detail) => {
                if detail.path.is_some() || detail.git.is_some() {
                    return Err(ServerError::bad_request(format!(
                        "the published dependency `{alias}` uses a path or git source"
                    )));
                }
                let requirement = detail.version.as_deref().ok_or_else(|| {
                    ServerError::bad_request(format!(
                        "the published dependency `{alias}` has no version"
                    ))
                })?;
                IndexDependency {
                    name: detail.package.clone().unwrap_or_else(|| alias.clone()),
                    alias: Some(alias.clone()),
                    req: requirement.to_owned(),
                    features: detail.features.clone(),
                    default_features: detail.default_features,
                    optional: detail.optional,
                    when: detail.when.clone(),
                }
            }
        };
        if semver::VersionReq::parse(&entry.req).is_err() {
            return Err(ServerError::bad_request(format!(
                "the dependency `{alias}` has an invalid version requirement"
            )));
        }
        result.push(entry);
    }
    Ok(result)
}

fn expected_keyid(public: &str) -> ServerResult<String> {
    let bytes = hex::decode(public).map_err(|error| {
        ServerError::provenance(format!("the publisher key is invalid: {error}"))
    })?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| ServerError::provenance("the publisher key is not 32 bytes"))?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&bytes).map_err(|error| {
        ServerError::provenance(format!("the publisher key is invalid: {error}"))
    })?;
    Ok(provenance::key_id(&key))
}

fn contains_source(root: &Path) -> ServerResult<bool> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            if contains_source(&path)? {
                return Ok(true);
            }
        } else if path.extension().and_then(|value| value.to_str()) == Some("sl") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn safe_relative(root: &Path, value: &str) -> ServerResult<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ServerError::bad_request(format!(
            "the path `{value}` is not a safe relative path"
        )));
    }
    Ok(root.join(path))
}

fn base_name(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

struct Temporary {
    path: PathBuf,
}

impl Temporary {
    fn new(root: &Path) -> ServerResult<Self> {
        let path = root.join("tmp").join(format!(
            "publish-{}-{}",
            std::process::id(),
            crate::http::random_nonce()
        ));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path })
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_edges_are_built_from_manifests() {
        let mut dependencies = std::collections::BTreeMap::new();
        dependencies.insert(
            "foo".to_owned(),
            DependencySpec::Detailed(seal_manifest::DependencyManifest {
                version: Some("^1".to_owned()),
                package: Some("foo".to_owned()),
                ..Default::default()
            }),
        );
        let result = index_dependencies(&dependencies).unwrap();
        assert_eq!(result[0].name, "foo");
        assert_eq!(result[0].req, "^1");
    }

    #[test]
    fn path_dependencies_cannot_be_published() {
        let mut dependencies = std::collections::BTreeMap::new();
        dependencies.insert(
            "foo".to_owned(),
            DependencySpec::Detailed(seal_manifest::DependencyManifest {
                path: Some(PathBuf::from("../foo")),
                ..Default::default()
            }),
        );
        assert!(index_dependencies(&dependencies).is_err());
    }

    fn config(root: PathBuf) -> crate::config::ServerConfig {
        crate::config::ServerConfig {
            storage: root,
            preapproved_namespaces: vec!["demo".to_owned()],
            ..Default::default()
        }
    }

    fn admin() -> Principal {
        Principal {
            name: "admin".to_owned(),
            scopes: vec!["admin".to_owned()],
            namespaces: Vec::new(),
            admin: true,
            subject: None,
            oidc: false,
        }
    }

    fn request(name: &str, version: &str) -> PublishRequest {
        let seed = [7u8; 32];
        let manifest = format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nlicense = \"MIT\"\nreadme = \"README.md\"\n"
        );
        let api = seal_api::ApiSnapshot {
            version: 1,
            package: name.to_owned(),
            package_version: version.to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            modules: Vec::new(),
            items: Vec::new(),
        };
        let sbom = seal_registry::sbom::generate(&seal_registry::sbom::SbomInput {
            project: name.to_owned(),
            project_version: version.to_owned(),
            toolchain: "0.1.0".to_owned(),
            components: Vec::new(),
        })
        .unwrap();
        let files = [
            seal_registry::publish::PackageFile {
                path: "seal.toml".to_owned(),
                contents: manifest.into_bytes(),
                executable: false,
            },
            seal_registry::publish::PackageFile {
                path: "README.md".to_owned(),
                contents: b"# demo\n".to_vec(),
                executable: false,
            },
            seal_registry::publish::PackageFile {
                path: "lib.sl".to_owned(),
                contents: b"pub fn hello() {}\n".to_vec(),
                executable: false,
            },
            seal_registry::publish::PackageFile {
                path: "seal.api.json".to_owned(),
                contents: api.canonical_bytes().unwrap(),
                executable: false,
            },
            seal_registry::publish::PackageFile {
                path: "seal.sbom.json".to_owned(),
                contents: serde_json::to_vec_pretty(&sbom).unwrap(),
                executable: false,
            },
        ];
        let archive = seal_registry::publish::build_archive(&files).unwrap();
        let checksum = digest(&archive);
        let sbom_digest = seal_registry::publish::document_digest(&sbom).unwrap();
        let api_digest = api.hash().unwrap();
        let provenance = seal_registry::provenance::sign(
            &seal_registry::provenance::SignOptions {
                name,
                version,
                archive_sha256: checksum.trim_start_matches("sha256:"),
                sbom: &sbom_digest,
                api: &api_digest,
                repository: None,
                identity: None,
                invocation_id: "invocation",
                started_on: "2026-01-01T00:00:00Z",
                finished_on: "2026-01-01T00:00:00Z",
            },
            &seed,
        )
        .unwrap();
        PublishRequest {
            name: name.to_owned(),
            version: version.to_owned(),
            archive: seal_registry::publish::encode_archive(&archive),
            api,
            sbom,
            provenance,
            publisher: seal_registry::publish::publisher(&seed, None),
        }
    }

    fn temporary(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "seal-publish-{name}-{}-{}",
            std::process::id(),
            crate::http::random_nonce()
        ))
    }

    #[test]
    fn a_publication_is_stored_with_valid_attestations() {
        let root = temporary("store");
        let store = Store::open(config(root.clone())).unwrap();
        let response = publish(&store, &admin(), &request("demo", "1.0.0")).unwrap();
        assert_eq!(response.name, "demo");
        assert_eq!(response.version, "1.0.0");
        assert!(response.checksum.starts_with("sha256:"));
        assert_eq!(response.log.entry.package, "demo");
        assert_eq!(response.log.tree_size, 1);
        let index_path = seal_registry::index::sparse_path("demo").unwrap();
        let index: seal_registry::index::IndexPackage =
            serde_json::from_slice(&std::fs::read(store.public().join(index_path)).unwrap())
                .unwrap();
        let entry = index.version(&Version::new(1, 0, 0)).unwrap();
        assert_eq!(entry.checksum, response.checksum);
        assert!(entry.provenance.is_some());
        assert!(
            store
                .public()
                .join("packages/demo/1.0.0/demo-1.0.0.tar.zst")
                .is_file()
        );
        assert_eq!(store.state().log.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_existing_version_cannot_be_replaced() {
        let root = temporary("immutable");
        let store = Store::open(config(root.clone())).unwrap();
        publish(&store, &admin(), &request("demo", "1.0.0")).unwrap();
        let error = publish(&store, &admin(), &request("demo", "1.0.0")).unwrap_err();
        assert_eq!(error.code(), "E8082");
        assert_eq!(store.state().log.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn yanking_records_a_log_entry_and_can_be_undone() {
        let root = temporary("yank");
        let store = Store::open(config(root.clone())).unwrap();
        publish(&store, &admin(), &request("demo", "1.0.0")).unwrap();
        let request = YankRequest {
            name: "demo".to_owned(),
            version: "1.0.0".to_owned(),
            yanked: true,
        };
        let response = yank(&store, &admin(), &request).unwrap();
        assert!(response.yanked);
        assert_eq!(response.log.entry.kind, "yank");
        let index_path = seal_registry::index::sparse_path("demo").unwrap();
        let index: seal_registry::index::IndexPackage =
            serde_json::from_slice(&std::fs::read(store.public().join(index_path)).unwrap())
                .unwrap();
        assert!(index.version(&Version::new(1, 0, 0)).unwrap().yanked);
        let restored = yank(
            &store,
            &admin(),
            &YankRequest {
                yanked: false,
                ..request
            },
        )
        .unwrap();
        assert!(!restored.yanked);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_namespace_must_be_owned_before_publishing() {
        let root = temporary("namespace");
        let store = Store::open(config(root.clone())).unwrap();
        let developer = Principal {
            name: "developer".to_owned(),
            scopes: vec!["publish:*".to_owned()],
            namespaces: Vec::new(),
            admin: false,
            subject: None,
            oidc: false,
        };
        let error = publish(&store, &developer, &request("@other/pkg", "1.0.0")).unwrap_err();
        assert_eq!(error.code(), "E8083");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn rotating_tuf_keys_preserves_artifacts_and_bumps_the_root() {
        let root = temporary("rotate");
        let store = Store::open(config(root.clone())).unwrap();
        publish(&store, &admin(), &request("demo", "1.0.0")).unwrap();
        let root_before = store.trusted_root().unwrap();
        let rotated = store.rotate(&admin()).unwrap();
        assert_eq!(rotated["signed"]["version"], 2);
        assert_ne!(root_before, store.trusted_root().unwrap());
        assert!(
            store
                .public()
                .join("packages/demo/1.0.0/demo-1.0.0.tar.zst")
                .is_file()
        );
        assert_eq!(store.state().log.len(), 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reopening_the_store_keeps_index_cas_and_log() {
        let root = temporary("restart");
        let store = Store::open(config(root.clone())).unwrap();
        let response = publish(&store, &admin(), &request("demo", "1.0.0")).unwrap();
        drop(store);
        let store = Store::open(config(root.clone())).unwrap();
        assert_eq!(store.state().log.len(), 1);
        assert_eq!(store.state().log[0].checksum, response.checksum);
        assert!(
            store
                .public()
                .join("packages/demo/1.0.0/demo-1.0.0.tar.zst")
                .is_file()
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
