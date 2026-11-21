//! Resolution behavior against an in-memory registry.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_registry::client::{HttpResponse, RegistryClient, Transport};
use seal_registry::config::RegistryConfig;
use seal_registry::resolve::{Requirement, ResolveOptions, resolve};
use serde_json::{Value, json};
use url::Url;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Mock {
    files: BTreeMap<String, Value>,
}

impl Transport for Mock {
    fn get(
        &self,
        url: &Url,
        _token: Option<&seal_registry::auth::Token>,
        _etag: Option<&str>,
    ) -> seal_registry::RegistryResult<HttpResponse> {
        let path = url.path().trim_start_matches('/').to_owned();
        match self.files.get(&path) {
            Some(value) => Ok(HttpResponse {
                status: 200,
                etag: None,
                body: serde_json::to_vec(value).unwrap(),
            }),
            None => Ok(HttpResponse {
                status: 404,
                etag: None,
                body: Vec::new(),
            }),
        }
    }
}

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-registry-resolution-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    Fixture { root }
}

fn config(root: &std::path::Path) -> RegistryConfig {
    RegistryConfig {
        name: "test".to_owned(),
        url: Url::parse("https://registry.invalid/").unwrap(),
        mirror: None,
        token: None,
        tuf_url: Url::parse("https://registry.invalid/").unwrap(),
        tuf_root: None,
        advisories: Url::parse("https://registry.invalid/advisories.json").unwrap(),
        offline: false,
        cache_root: root.join("cache"),
        policy: Default::default(),
    }
}

type Dependency<'a> = (&'a str, &'a str);
type Release<'a> = (&'a str, bool, &'a [Dependency<'a>]);

fn package(name: &str, versions: &[Release<'_>]) -> Value {
    let mut map = serde_json::Map::new();
    for (version, yanked, dependencies) in versions {
        let dependencies: Vec<Value> = dependencies
            .iter()
            .map(|(name, req)| {
                json!({
                    "name": name,
                    "req": req,
                    "features": [],
                    "default-features": true,
                    "optional": false
                })
            })
            .collect();
        map.insert(
            (*version).to_owned(),
            json!({
                "checksum": format!("sha256:{}", "a".repeat(64)),
                "archive": format!("packages/{name}/{version}/{name}-{version}.tar.zst"),
                "yanked": yanked,
                "license": "MIT",
                "dependencies": dependencies,
                "features": {}
            }),
        );
    }
    json!({"format": 1, "name": name, "versions": map})
}

fn resolve_with(
    registry: &std::path::Path,
    files: BTreeMap<String, Value>,
    requirements: &[Requirement],
    options: ResolveOptions,
) -> seal_registry::RegistryResult<Vec<seal_registry::ResolvedPackage>> {
    let config = config(registry);
    let client = RegistryClient::with_transport(&config, Box::new(Mock { files })).expect("client");
    resolve(&client, &config, None, requirements, &options)
}

fn files(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
    entries
        .iter()
        .map(|(name, value)| {
            (
                seal_registry::index::sparse_path(name).unwrap(),
                value.clone(),
            )
        })
        .collect()
}

#[test]
fn two_majors_are_resolved_side_by_side() {
    let fixture = fixture();
    let files = files(&[(
        "foo",
        package(
            "foo",
            &[
                ("1.0.0", false, &[]),
                ("1.5.0", false, &[]),
                ("2.0.0", false, &[]),
            ],
        ),
    )]);
    let requirements = vec![
        Requirement::parse("foo", "foo1", "^1", "app 0.1.0").unwrap(),
        Requirement::parse("foo", "foo2", "^2", "app 0.1.0").unwrap(),
    ];
    let packages = resolve_with(
        &fixture.root,
        files,
        &requirements,
        ResolveOptions::default(),
    )
    .expect("resolution");
    assert_eq!(packages.len(), 2);
    assert_eq!(packages[0].version.to_string(), "1.5.0");
    assert_eq!(packages[1].version.to_string(), "2.0.0");
}

#[test]
fn incompatible_ranges_report_the_conflict_path() {
    let fixture = fixture();
    let files = files(&[
        (
            "foo",
            package("foo", &[("1.0.0", false, &[("bar", "=1.0.0")])]),
        ),
        (
            "bar",
            package("bar", &[("1.0.0", false, &[]), ("1.5.0", false, &[])]),
        ),
    ]);
    let requirements = vec![
        Requirement::parse("foo", "foo", "^1", "app 0.1.0").unwrap(),
        Requirement::parse("bar", "bar", "=1.5.0", "app 0.1.0").unwrap(),
    ];
    let error = resolve_with(
        &fixture.root,
        files,
        &requirements,
        ResolveOptions::default(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "E8026");
    assert!(error.message().contains("bar"), "{}", error.message());
    assert!(
        error.message().contains("foo") || error.message().contains("project"),
        "{}",
        error.message()
    );
}

#[test]
fn prereleases_need_an_explicit_opt_in() {
    let fixture = fixture();
    let files = files(&[(
        "foo",
        package(
            "foo",
            &[("1.0.0", false, &[]), ("1.1.0-beta.1", false, &[])],
        ),
    )]);
    let stable = resolve_with(
        &fixture.root,
        files.clone(),
        &[Requirement::parse("foo", "foo", "^1", "app 0.1.0").unwrap()],
        ResolveOptions::default(),
    )
    .unwrap();
    assert_eq!(stable[0].version.to_string(), "1.0.0");
    let beta = resolve_with(
        &fixture.root,
        files,
        &[Requirement::parse("foo", "foo", "^1.1.0-beta.1", "app 0.1.0").unwrap()],
        ResolveOptions::default(),
    )
    .unwrap();
    assert_eq!(beta[0].version.to_string(), "1.1.0-beta.1");
}

#[test]
fn yanked_versions_are_skipped_and_minimal_versions_select_the_lowest() {
    let fixture = fixture();
    let files = files(&[(
        "foo",
        package(
            "foo",
            &[
                ("1.0.0", false, &[]),
                ("1.1.0", false, &[]),
                ("1.2.0", true, &[]),
            ],
        ),
    )]);
    let requirements = vec![Requirement::parse("foo", "foo", "^1", "app 0.1.0").unwrap()];
    let latest = resolve_with(
        &fixture.root,
        files.clone(),
        &requirements,
        ResolveOptions::default(),
    )
    .unwrap();
    assert_eq!(latest[0].version.to_string(), "1.1.0");
    let minimal = resolve_with(
        &fixture.root,
        files,
        &requirements,
        ResolveOptions {
            minimal_versions: true,
            ..ResolveOptions::default()
        },
    )
    .unwrap();
    assert_eq!(minimal[0].version.to_string(), "1.0.0");
}

#[test]
fn a_locked_version_is_preferred() {
    let fixture = fixture();
    let files = files(&[(
        "foo",
        package("foo", &[("1.0.0", false, &[]), ("1.1.0", false, &[])]),
    )]);
    let mut locked = BTreeMap::new();
    locked.insert(
        (
            "foo".to_owned(),
            seal_registry::semver::Line {
                major: 1,
                minor: None,
            },
        ),
        semver::Version::new(1, 0, 0),
    );
    let packages = resolve_with(
        &fixture.root,
        files,
        &[Requirement::parse("foo", "foo", "^1", "app 0.1.0").unwrap()],
        ResolveOptions {
            locked,
            ..ResolveOptions::default()
        },
    )
    .unwrap();
    assert_eq!(packages[0].version.to_string(), "1.0.0");
}

#[test]
fn the_step_limit_stops_a_hostile_resolution() {
    let fixture = fixture();
    let files = files(&[(
        "foo",
        package("foo", &[("1.0.0", false, &[]), ("1.1.0", false, &[])]),
    )]);
    let error = resolve_with(
        &fixture.root,
        files,
        &[Requirement::parse("foo", "foo", "^1", "app 0.1.0").unwrap()],
        ResolveOptions {
            step_limit: 1,
            ..ResolveOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.code(), "E8026");
    assert!(error.message().contains("step limit"));
}
