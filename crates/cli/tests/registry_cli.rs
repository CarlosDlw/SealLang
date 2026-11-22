//! End-to-end registry, vendor, audit, SBOM, and query tests.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    registry: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn seal() -> Command {
    Command::new(env!("CARGO_BIN_EXE_seal"))
}

fn output(command: &mut Command) -> Output {
    command.output().expect("the seal binary runs")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write(path: &Path, contents: impl AsRef<[u8]>) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, contents).expect("write");
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256(bytes))
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-registry-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("project")).expect("project");
    let registry = root.join("registry");
    std::fs::create_dir_all(&registry).expect("registry");
    let fixture = Fixture { root, registry };
    build_registry(&fixture);
    fixture
}

fn build_registry(fixture: &Fixture) {
    let mut targets = BTreeMap::new();
    for (name, version, dependency) in [
        ("foo", "1.0.0", None),
        ("foo", "1.2.0", None),
        ("foo", "1.3.0", None),
        ("foo", "2.0.0", None),
        ("bar", "1.0.0", Some(("foo", "^1"))),
    ] {
        let manifest = package_manifest(name, version, dependency);
        let archive = archive(&manifest);
        let path = format!("packages/{name}/{version}/{name}-{version}.tar.zst");
        write(&fixture.registry.join(&path), &archive);
        let checksum = digest(&archive);
        add_index_version(
            &fixture.registry,
            &mut targets,
            Release {
                name,
                version,
                dependency,
                yanked: false,
            },
            &checksum,
            &path,
        );
    }
    // A yanked release is visible to `info` but never selected.
    let manifest = package_manifest("foo", "0.9.0", None);
    let archive = archive(&manifest);
    let path = "packages/foo/0.9.0/foo-0.9.0.tar.zst";
    write(&fixture.registry.join(path), &archive);
    add_index_version(
        &fixture.registry,
        &mut targets,
        Release {
            name: "foo",
            version: "0.9.0",
            dependency: None,
            yanked: true,
        },
        &digest(&archive),
        path,
    );
    let catalog = json!({
        "format": 1,
        "packages": [
            {"name": "bar", "latest": "1.0.0", "description": "A helper", "license": "MIT"},
            {"name": "foo", "latest": "1.3.0", "description": "The example package", "license": "MIT"}
        ]
    });
    let catalog = serde_json::to_vec_pretty(&catalog).unwrap();
    write(&fixture.registry.join("index/catalog.json"), &catalog);
    targets.insert("index/catalog.json".to_owned(), meta(&catalog));
    let advisories = serde_json::to_vec_pretty(&json!({
        "format": 1,
        "advisories": [{
            "id": "SEAL-2026-0001",
            "package": "foo",
            "affected": "<1.2.0",
            "severity": "high",
            "description": "An old foo release is vulnerable"
        }]
    }))
    .unwrap();
    write(&fixture.registry.join("advisories.json"), &advisories);
    targets.insert("advisories.json".to_owned(), meta(&advisories));

    let seed = [42u8; 32];
    let keyid = seal_registry::tuf::key_id(&seed);
    let public = seal_registry::tuf::public_key(&seed);
    let expires = "2099-01-01T00:00:00Z";
    let role = json!({"keyids": [keyid.clone()], "threshold": 1});
    let mut keys = Map::new();
    keys.insert(
        keyid.clone(),
        json!({"keytype": "ed25519", "scheme": "ed25519", "keyval": {"public": public}}),
    );
    let mut roles = Map::new();
    for name in ["root", "timestamp", "snapshot", "targets"] {
        roles.insert(name.to_owned(), role.clone());
    }
    let root_signed = json!({
        "type": "root",
        "version": 1,
        "expires": expires,
        "keys": keys,
        "roles": roles
    });
    let root = signed(&root_signed, &seed);
    write(
        &fixture.registry.join("tuf/root.json"),
        serde_json::to_vec_pretty(&root).unwrap(),
    );

    let mut target_meta = Map::new();
    for (path, file) in &targets {
        target_meta.insert(path.clone(), file.clone());
    }
    let targets_signed = json!({
        "type": "targets",
        "version": 1,
        "expires": expires,
        "targets": target_meta
    });
    let targets_file = signed(&targets_signed, &seed);
    let targets_bytes = serde_json::to_vec_pretty(&targets_file).unwrap();
    write(&fixture.registry.join("tuf/targets.json"), &targets_bytes);

    let mut snapshot_meta = Map::new();
    let mut targets_meta = meta(&targets_bytes);
    targets_meta["version"] = json!(1);
    snapshot_meta.insert("targets.json".to_owned(), targets_meta);
    let snapshot_signed = json!({
        "type": "snapshot",
        "version": 1,
        "expires": expires,
        "meta": snapshot_meta
    });
    let snapshot_file = signed(&snapshot_signed, &seed);
    let snapshot_bytes = serde_json::to_vec_pretty(&snapshot_file).unwrap();
    write(&fixture.registry.join("tuf/snapshot.json"), &snapshot_bytes);

    let mut timestamp_meta = Map::new();
    let mut snapshot_meta = meta(&snapshot_bytes);
    snapshot_meta["version"] = json!(1);
    timestamp_meta.insert("snapshot.json".to_owned(), snapshot_meta);
    let timestamp_signed = json!({
        "type": "timestamp",
        "version": 1,
        "expires": expires,
        "meta": timestamp_meta
    });
    let timestamp_file = signed(&timestamp_signed, &seed);
    write(
        &fixture.registry.join("tuf/timestamp.json"),
        serde_json::to_vec_pretty(&timestamp_file).unwrap(),
    );

    let trusted = fixture.root.join("project/trusted-root.json");
    write(&trusted, serde_json::to_vec_pretty(&root).unwrap());
    let registry_url = url::Url::from_directory_path(&fixture.registry)
        .unwrap()
        .to_string();
    let tuf_url = url::Url::from_directory_path(fixture.registry.join("tuf"))
        .unwrap()
        .to_string();
    write(
        &fixture.root.join("project/seal.toml"),
        format!(
            "[project]\nname = \"demo\"\n\n[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[registry]\nurl = \"{registry_url}\"\ntuf-url = \"{tuf_url}\"\ntuf-root = \"trusted-root.json\"\n\n[registry.policy]\nallow-licenses = [\"MIT\"]\n"
        ),
    );
    write(
        &fixture.root.join("project/src/main.sl"),
        "func main() -> i32 { 0 }\n",
    );
}

fn package_manifest(name: &str, version: &str, dependency: Option<(&str, &str)>) -> String {
    let dependencies = dependency.map_or(String::new(), |(name, requirement)| {
        format!("\n[dependencies]\n{name} = \"{requirement}\"\n")
    });
    format!(
        "[package]\nname = \"{name}\"\nversion = \"{version}\"\nlicense = \"MIT\"\ndescription = \"Fixture package\"\n\n[package.features]\ndefault = []\nstd = []\n{dependencies}"
    )
}

fn archive(manifest: &str) -> Vec<u8> {
    let encoder = zstd::Encoder::new(Vec::new(), 3).expect("zstd");
    let mut builder = tar::Builder::new(encoder);
    for (path, contents) in [
        ("seal.toml", manifest.as_bytes()),
        (
            "src/lib.sl",
            b"public func answer() -> i32 { 42 }\n".as_slice(),
        ),
    ] {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).expect("archive path");
        header.set_size(contents.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, contents).expect("archive entry");
    }
    let encoder = builder.into_inner().expect("archive finish");
    encoder.finish().expect("zstd finish")
}

struct Release<'a> {
    name: &'a str,
    version: &'a str,
    dependency: Option<(&'a str, &'a str)>,
    yanked: bool,
}

fn add_index_version(
    registry: &Path,
    targets: &mut BTreeMap<String, Value>,
    release: Release<'_>,
    checksum: &str,
    archive: &str,
) {
    let Release {
        name,
        version,
        dependency,
        yanked,
    } = release;
    let path = seal_registry::index::sparse_path(name).expect("sparse path");
    let mut package: Value = std::fs::read(registry.join(&path))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_else(|| json!({"format": 1, "name": name, "versions": {}}));
    let dependencies: Vec<Value> = dependency
        .map(|(name, requirement)| {
            vec![json!({
                "name": name,
                "req": requirement,
                "features": [],
                "default-features": true,
                "optional": false
            })]
        })
        .unwrap_or_default();
    package["versions"][version] = json!({
        "checksum": checksum,
        "archive": archive,
        "yanked": yanked,
        "license": "MIT",
        "description": "Fixture package",
        "provenance": null,
        "dependencies": dependencies,
        "features": {"default": [], "std": []}
    });
    let bytes = serde_json::to_vec_pretty(&package).unwrap();
    write(&registry.join(&path), &bytes);
    targets.insert(path, meta(&bytes));
    targets.insert(
        archive.to_owned(),
        meta(&std::fs::read(registry.join(archive)).unwrap()),
    );
}

fn meta(bytes: &[u8]) -> Value {
    json!({
        "length": bytes.len(),
        "hashes": {"sha256": sha256(bytes)}
    })
}

fn signed(signed: &Value, seed: &[u8; 32]) -> Value {
    json!({
        "signed": signed,
        "signatures": [seal_registry::tuf::sign_document(signed, seed).unwrap()]
    })
}

#[test]
fn add_vendor_offline_build_audit_sbom_and_queries() {
    let fixture = fixture();
    let project = fixture.root.join("project");

    let dry_run = output(
        seal()
            .arg("add")
            .arg("foo@~1.2")
            .arg("--dry-run")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&dry_run);
    let manifest = std::fs::read_to_string(project.join("seal.toml")).unwrap();
    assert!(!manifest.contains("foo"));
    assert!(!project.join("seal.lock").exists());

    let add = output(
        seal()
            .arg("add")
            .arg("foo@~1.2")
            .arg("--yes")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&add);
    let manifest = std::fs::read_to_string(project.join("seal.toml")).unwrap();
    assert!(
        manifest.contains("foo = { version = \"~1.2\" }"),
        "{manifest}"
    );
    let lock = std::fs::read_to_string(project.join("seal.lock")).unwrap();
    assert!(lock.contains("version = 2"));
    assert!(lock.contains("registry+file://"));
    assert!(lock.contains("sha256:"));

    let tree = output(seal().arg("tree").arg("--project").arg(&project));
    assert_success(&tree);
    assert!(String::from_utf8_lossy(&tree.stdout).contains("foo 1.2.0"));

    let info = output(
        seal()
            .arg("info")
            .arg("foo")
            .arg("--json")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&info);
    let info: Value = serde_json::from_slice(&info.stdout).expect("info JSON");
    assert_eq!(info["name"], "foo");
    assert!(info["versions"]["1.2.0"]["yanked"] == false);

    let search = output(
        seal()
            .arg("search")
            .arg("fo")
            .arg("--json")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&search);
    let search: Value = serde_json::from_slice(&search.stdout).expect("search JSON");
    assert_eq!(search[0]["name"], "foo");

    let audit = output(
        seal()
            .arg("audit")
            .arg("--json")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&audit);
    let audit: Value = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    assert_eq!(audit["violations"].as_array().unwrap().len(), 0);

    let sbom = output(seal().arg("sbom").arg("--project").arg(&project));
    assert_success(&sbom);
    let sbom: Value = serde_json::from_slice(&sbom.stdout).expect("SBOM JSON");
    assert_eq!(sbom["bomFormat"], "CycloneDX");
    assert!(
        sbom["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["name"] == "foo")
    );

    let vendor = output(seal().arg("vendor").arg("--project").arg(&project));
    assert_success(&vendor);
    assert!(project.join("vendor/seal-vendor.toml").is_file());
    assert!(project.join("vendor/foo-1.2.0/seal.toml").is_file());

    // Delete the registry: the offline build must use only the vendor tree.
    std::fs::remove_dir_all(&fixture.registry).expect("remove registry");
    let build = output(
        seal()
            .arg("build")
            .arg("--offline")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&build);
    let locked = output(
        seal()
            .arg("build")
            .arg("--frozen")
            .arg("--locked")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&locked);
}

#[test]
fn two_major_lines_coexist_and_tree_reports_them() {
    let fixture = fixture();
    let project = fixture.root.join("project");
    let manifest = std::fs::read_to_string(project.join("seal.toml")).unwrap();
    write(
        &project.join("seal.toml"),
        format!(
            "{manifest}\n[dependencies]\nfoo1 = {{ version = \"^1\", package = \"foo\" }}\nfoo2 = {{ version = \"^2\", package = \"foo\" }}\n"
        ),
    );
    let tree = output(
        seal()
            .arg("tree")
            .arg("--duplicates")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&tree);
    let stdout = String::from_utf8_lossy(&tree.stdout);
    assert!(stdout.contains("foo: "), "{stdout}");
    assert!(stdout.contains("1.3.0"), "{stdout}");
    assert!(stdout.contains("2.0.0"), "{stdout}");
}

#[test]
fn audit_reports_a_matching_advisory() {
    let fixture = fixture();
    let project = fixture.root.join("project");
    let add = output(
        seal()
            .arg("add")
            .arg("foo@=1.0.0")
            .arg("--yes")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&add);
    let audit = output(
        seal()
            .arg("audit")
            .arg("--json")
            .arg("--project")
            .arg(&project),
    );
    assert_eq!(audit.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&audit.stdout).expect("audit JSON");
    assert!(
        report["violations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|violation| violation["code"] == "E8036")
    );
}

#[test]
fn precise_update_moves_a_locked_version() {
    let fixture = fixture();
    let project = fixture.root.join("project");
    assert_success(&output(
        seal()
            .arg("add")
            .arg("foo@^1.2")
            .arg("--yes")
            .arg("--project")
            .arg(&project),
    ));
    let update = output(
        seal()
            .arg("update")
            .arg("--precise")
            .arg("foo@1.2.0")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&update);
    let lock = std::fs::read_to_string(project.join("seal.lock")).unwrap();
    assert!(
        lock.contains("name = \"foo\"\nversion = \"1.2.0\""),
        "{lock}"
    );
}

#[test]
fn a_corrupt_archive_is_reported_as_e8021() {
    let fixture = fixture();
    let project = fixture.root.join("project");
    let path = fixture
        .registry
        .join("packages/foo/1.2.0/foo-1.2.0.tar.zst");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] ^= 0xFF;
    std::fs::write(&path, &bytes).unwrap();
    let add = output(
        seal()
            .arg("add")
            .arg("foo@=1.2.0")
            .arg("--yes")
            .arg("--project")
            .arg(&project),
    );
    assert_eq!(add.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&add.stderr);
    assert!(
        stderr.contains("E8021") || stderr.contains("E8080"),
        "{stderr}"
    );
}

#[test]
fn a_missing_tuf_root_is_e8080() {
    let fixture = fixture();
    let project = fixture.root.join("project");
    let manifest = std::fs::read_to_string(project.join("seal.toml")).unwrap();
    let manifest = manifest.replace("tuf-root = \"trusted-root.json\"\n", "");
    write(&project.join("seal.toml"), manifest);
    let add = output(
        seal()
            .arg("add")
            .arg("foo@1.2")
            .arg("--yes")
            .arg("--project")
            .arg(&project),
    );
    assert_eq!(add.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&add.stderr).contains("E8080"));
}

#[test]
fn the_registry_example_vendors_and_runs_offline() {
    let fixture = fixture();
    let project = fixture.root.join("project");
    let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/registry");
    copy_tree(&example, &project);

    let registry_url = url::Url::from_directory_path(&fixture.registry)
        .unwrap()
        .to_string();
    let tuf_url = url::Url::from_directory_path(fixture.registry.join("tuf"))
        .unwrap()
        .to_string();
    let manifest = std::fs::read_to_string(project.join("seal.toml")).expect("example manifest");
    let manifest = manifest
        .replace("file:///tmp/seal-registry/tuf/", &tuf_url)
        .replace("file:///tmp/seal-registry/", &registry_url);
    write(&project.join("seal.toml"), manifest);
    assert!(
        project.join("trusted-root.json").is_file(),
        "the fixture provides the trusted root the manifest pins"
    );

    let vendor = output(seal().arg("vendor").arg("--project").arg(&project));
    assert_success(&vendor);
    assert!(project.join("vendor/foo-1.2.0/seal.toml").is_file());

    // The offline build and the executed program use only the vendor tree.
    std::fs::remove_dir_all(&fixture.registry).expect("remove registry");
    let build = output(
        seal()
            .arg("build")
            .arg("--offline")
            .arg("--vendor")
            .arg("--project")
            .arg(&project),
    );
    assert_success(&build);
    let run = output(
        seal()
            .arg("run")
            .arg("--offline")
            .arg("--vendor")
            .arg("--project")
            .arg(&project),
    );
    assert_eq!(
        run.status.code(),
        Some(42),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create the destination directory");
    for entry in std::fs::read_dir(from).expect("read the example directory") {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        let destination = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &destination);
        } else {
            std::fs::copy(&path, &destination).expect("copy an example file");
        }
    }
}

// Keep the unused import available for future protocol fixtures.
#[allow(dead_code)]
fn _write_stderr(text: &str) {
    let _ = std::io::stderr().write_all(text.as_bytes());
}
