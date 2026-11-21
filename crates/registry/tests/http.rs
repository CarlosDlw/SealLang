//! Transport behavior: mirror fallback, authentication, caching, and offline.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_registry::auth::Token;
use seal_registry::client::{HttpResponse, RegistryClient, Transport};
use seal_registry::config::RegistryConfig;
use url::Url;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Mock {
    responses: BTreeMap<String, (u16, Vec<u8>)>,
    tokens: Arc<Mutex<Vec<Option<String>>>>,
}

impl Transport for Mock {
    fn get(
        &self,
        url: &Url,
        token: Option<&Token>,
        _etag: Option<&str>,
    ) -> seal_registry::RegistryResult<HttpResponse> {
        self.tokens
            .lock()
            .unwrap()
            .push(token.map(|token| token.expose().to_owned()));
        let key = format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap_or(""),
            url.path()
        );
        let (status, body) = self
            .responses
            .get(&key)
            .cloned()
            .unwrap_or((404, Vec::new()));
        Ok(HttpResponse {
            status,
            etag: None,
            body,
        })
    }
}

struct Fixture {
    root: PathBuf,
    config: RegistryConfig,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-http-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let config = RegistryConfig {
        name: "test".to_owned(),
        url: Url::parse("https://primary.invalid/").unwrap(),
        mirror: Some(Url::parse("https://mirror.invalid/").unwrap()),
        token: Some(Token::new("secret-token").unwrap()),
        tuf_url: Url::parse("https://primary.invalid/").unwrap(),
        tuf_root: None,
        advisories: Url::parse("https://primary.invalid/advisories.json").unwrap(),
        offline: false,
        cache_root: root.join("cache"),
        policy: Default::default(),
    };
    Fixture { root, config }
}

fn client(
    config: &RegistryConfig,
    responses: BTreeMap<String, (u16, Vec<u8>)>,
) -> (RegistryClient, Arc<Mutex<Vec<Option<String>>>>) {
    let tokens = Arc::new(Mutex::new(Vec::new()));
    let mock = Mock {
        responses,
        tokens: Arc::clone(&tokens),
    };
    let client = RegistryClient::with_transport(config, Box::new(mock)).unwrap();
    (client, tokens)
}

fn responses(entries: &[(&str, u16, &[u8])]) -> BTreeMap<String, (u16, Vec<u8>)> {
    entries
        .iter()
        .map(|(key, status, body)| ((*key).to_owned(), (*status, body.to_vec())))
        .collect()
}

#[test]
fn a_mirror_miss_falls_back_to_the_primary() {
    let fixture = fixture();
    let (client, tokens) = client(
        &fixture.config,
        responses(&[("https://primary.invalid/index/foo.json", 200, b"primary")]),
    );
    assert_eq!(
        client.fetch(&fixture.config, "index/foo.json").unwrap(),
        b"primary"
    );
    let tokens = tokens.lock().unwrap();
    assert!(!tokens.is_empty());
    assert!(
        tokens
            .iter()
            .all(|token| token.as_deref() == Some("secret-token"))
    );
}

#[test]
fn a_401_is_reported_as_missing_credentials() {
    let fixture = fixture();
    let (client, _) = client(
        &fixture.config,
        responses(&[
            ("https://mirror.invalid/index/foo.json", 401, b""),
            ("https://primary.invalid/index/foo.json", 401, b""),
        ]),
    );
    let error = client.fetch(&fixture.config, "index/foo.json").unwrap_err();
    assert_eq!(error.code(), "E8084");
}

#[test]
fn a_cached_body_is_used_offline() {
    let fixture = fixture();
    let entries = responses(&[("https://primary.invalid/index/foo.json", 200, b"cached")]);
    let (client, _) = client(&fixture.config, entries);
    assert_eq!(
        client.fetch(&fixture.config, "index/foo.json").unwrap(),
        b"cached"
    );
    let offline = RegistryConfig {
        offline: true,
        ..fixture.config.clone()
    };
    let client = RegistryClient::new(&offline).unwrap();
    assert_eq!(client.fetch(&offline, "index/foo.json").unwrap(), b"cached");
}

#[test]
fn an_offline_miss_is_e8085() {
    let fixture = fixture();
    let offline = RegistryConfig {
        offline: true,
        ..fixture.config.clone()
    };
    let client = RegistryClient::new(&offline).unwrap();
    let error = client.fetch(&offline, "index/missing.json").unwrap_err();
    assert_eq!(error.code(), "E8085");
}

#[test]
fn a_protocol_error_is_not_retried_forever() {
    let fixture = fixture();
    let (client, _) = client(
        &fixture.config,
        responses(&[
            ("https://mirror.invalid/index/foo.json", 500, b""),
            ("https://primary.invalid/index/foo.json", 500, b""),
        ]),
    );
    let error = client.fetch(&fixture.config, "index/foo.json").unwrap_err();
    assert_eq!(error.code(), "E8020");
}
