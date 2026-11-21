//! End-to-end HTTP flow: publish, verify attestations, yank, rotate, restart.

use std::path::{Path, PathBuf};

use seal_registry::RegistrySession;
use seal_registry::auth::Token;
use seal_registry::config::RegistryConfig;
use seal_registry::publish::{PublishRequest, YankRequest};
use seal_registry_server::auth::Principal;
use seal_registry_server::config::{ServerConfig, TokenConfig};
use seal_registry_server::server::Server;
use semver::Version;

fn temporary(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "seal-flow-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn config(storage: PathBuf) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".to_owned(),
        storage,
        preapproved_namespaces: vec!["demo".to_owned()],
        tokens: vec![
            TokenConfig {
                name: "admin".to_owned(),
                token: Some("secret".to_owned()),
                hash: None,
                scopes: vec!["admin".to_owned()],
                namespaces: Vec::new(),
                admin: true,
            },
            TokenConfig {
                name: "reader".to_owned(),
                token: Some("read-only".to_owned()),
                hash: None,
                scopes: vec!["read:*".to_owned()],
                namespaces: Vec::new(),
                admin: false,
            },
        ],
        ..ServerConfig::default()
    }
}

fn session(url: &str, root: &Path, cache: &Path) -> RegistrySession {
    session_with(url, root, cache, "secret")
}

fn session_with(url: &str, root: &Path, cache: &Path, token: &str) -> RegistrySession {
    let base = url::Url::parse(url).unwrap();
    let config = RegistryConfig {
        name: "flow".to_owned(),
        url: base.clone(),
        mirror: None,
        token: Some(Token::new(token.to_owned()).unwrap()),
        tuf_url: base.join("tuf/").unwrap(),
        tuf_root: Some(root.to_path_buf()),
        advisories: base.join("advisories.json").unwrap(),
        offline: false,
        cache_root: cache.to_path_buf(),
        policy: Default::default(),
    };
    RegistrySession::from_config(config).unwrap()
}

fn request(name: &str, version: &str) -> PublishRequest {
    let seed = [9u8; 32];
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
            contents: format!(
                "[package]\nname = \"{name}\"\nversion = \"{version}\"\nlicense = \"MIT\"\nreadme = \"README.md\"\n"
            )
            .into_bytes(),
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
    let checksum = {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&archive);
        format!("sha256:{}", hex::encode(hasher.finalize()))
    };
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

#[test]
fn publish_audit_yank_rotate_and_restart_work_over_http() {
    let storage = temporary("storage");
    let cache = temporary("cache");
    let root = temporary("root.json");
    let mut server = Server::start(config(storage.clone())).unwrap();
    std::fs::write(
        &root,
        std::fs::read(server.store().public().join("tuf/root.json")).unwrap(),
    )
    .unwrap();

    let first = session(&server.url(), &root, &cache);
    let response = seal_registry::publish::publish(&first, &request("demo", "1.0.0")).unwrap();
    assert_eq!(response.version, "1.0.0");

    let second = session(&server.url(), &root, &cache);
    let index = second.info("demo").unwrap();
    let entry = index.version(&Version::new(1, 0, 0)).unwrap();
    assert!(entry.provenance.is_some());
    seal_registry::audit::verify_attestations(&second, "demo", &Version::new(1, 0, 0), entry)
        .unwrap();

    let reader = session_with(&server.url(), &root, &cache, "read-only");
    let denied = seal_registry::publish::publish(&reader, &request("demo", "2.0.0")).unwrap_err();
    assert_eq!(denied.code(), "E8083");

    let duplicate =
        seal_registry::publish::publish(&second, &request("demo", "1.0.0")).unwrap_err();
    assert_eq!(duplicate.code(), "E8082");

    let yanked = seal_registry::publish::yank(
        &second,
        &YankRequest {
            name: "demo".to_owned(),
            version: "1.0.0".to_owned(),
            yanked: true,
        },
    )
    .unwrap();
    assert!(yanked.yanked);
    let third = session(&server.url(), &root, &cache);
    assert!(
        third
            .info("demo")
            .unwrap()
            .version(&Version::new(1, 0, 0))
            .unwrap()
            .yanked
    );

    let principal = Principal {
        name: "admin".to_owned(),
        scopes: vec!["admin".to_owned()],
        namespaces: Vec::new(),
        admin: true,
        subject: None,
        oidc: false,
    };
    let rotated = server.store().rotate(&principal).unwrap();
    assert_eq!(rotated["signed"]["version"], 2);
    let fourth = session(&server.url(), &root, &cache);
    assert!(
        fourth
            .info("demo")
            .unwrap()
            .version(&Version::new(1, 0, 0))
            .is_some()
    );

    server.stop();
    let mut restarted = Server::start(config(storage.clone())).unwrap();
    let fifth = session(&restarted.url(), &root, &cache);
    assert!(
        fifth
            .info("demo")
            .unwrap()
            .version(&Version::new(1, 0, 0))
            .unwrap()
            .yanked
    );
    restarted.stop();

    let _ = std::fs::remove_dir_all(storage);
    let _ = std::fs::remove_dir_all(cache);
    let _ = std::fs::remove_file(root);
}
