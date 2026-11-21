//! HTTP transport, retries, ETag caching, and token injection.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::auth::Token;
use crate::config::RegistryConfig;
use crate::error::{RegistryError, RegistryResult};

/// A response from a registry transport.
#[derive(Clone, Debug)]
pub struct HttpResponse {
    /// The HTTP status.
    pub status: u16,
    /// The response ETag, when present.
    pub etag: Option<String>,
    /// The response body.
    pub body: Vec<u8>,
}

/// A blocking HTTP transport used by the registry client.
pub trait Transport: Send + Sync {
    /// Performs one GET request. HTTP error statuses are returned as responses,
    /// not as transport errors.
    fn get(
        &self,
        url: &Url,
        token: Option<&Token>,
        etag: Option<&str>,
    ) -> RegistryResult<HttpResponse>;

    /// Performs one POST request with a JSON body.
    ///
    /// The default implementation refuses, so a read-only transport cannot
    /// accidentally claim to publish. HTTP error statuses are responses.
    fn post(
        &self,
        _url: &Url,
        _token: Option<&Token>,
        _body: &[u8],
    ) -> RegistryResult<HttpResponse> {
        Err(RegistryError::publish(
            "this registry transport does not support publication",
        ))
    }
}

/// A transport that reads `file://` URLs, used by local fixtures.
#[derive(Default)]
pub struct FileTransport;

impl Transport for FileTransport {
    fn get(
        &self,
        url: &Url,
        _token: Option<&Token>,
        _etag: Option<&str>,
    ) -> RegistryResult<HttpResponse> {
        let path = url
            .to_file_path()
            .map_err(|()| RegistryError::config(format!("`{url}` is not a local file URL")))?;
        match std::fs::read(&path) {
            Ok(body) => Ok(HttpResponse {
                status: 200,
                etag: None,
                body,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HttpResponse {
                status: 404,
                etag: None,
                body: Vec::new(),
            }),
            Err(error) => Err(RegistryError::config(format!(
                "cannot read `{}`: {error}",
                path.display()
            ))),
        }
    }

    fn post(
        &self,
        _url: &Url,
        _token: Option<&Token>,
        _body: &[u8],
    ) -> RegistryResult<HttpResponse> {
        Err(RegistryError::publish(
            "a `file://` registry is read-only; start `seal-registry-server` to publish",
        ))
    }
}

/// A `ureq` transport with certificate validation and bounded redirects.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl Default for UreqTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .max_redirects(5)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Transport for UreqTransport {
    fn get(
        &self,
        url: &Url,
        token: Option<&Token>,
        etag: Option<&str>,
    ) -> RegistryResult<HttpResponse> {
        let mut request = self.agent.get(url.as_str());
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {}", token.expose()));
            request = request.header("X-Seal-Registry", "1");
        }
        if let Some(etag) = etag {
            request = request.header("If-None-Match", etag);
        }
        let mut response = request
            .call()
            .map_err(|error| RegistryError::config(format!("cannot fetch `{url}`: {error}")))?;
        let status = response.status().as_u16();
        let etag = response
            .headers()
            .get("etag")
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned);
        let body = if status == 304 || status == 404 {
            Vec::new()
        } else {
            response
                .body_mut()
                .with_config()
                .limit(512 * 1024 * 1024)
                .read_to_vec()
                .map_err(|error| RegistryError::config(format!("cannot read `{url}`: {error}")))?
        };
        Ok(HttpResponse { status, etag, body })
    }

    fn post(&self, url: &Url, token: Option<&Token>, body: &[u8]) -> RegistryResult<HttpResponse> {
        let mut request = self
            .agent
            .post(url.as_str())
            .header("Content-Type", "application/json");
        if let Some(token) = token {
            request = request.header("Authorization", format!("Bearer {}", token.expose()));
        }
        let mut response = request
            .send(body)
            .map_err(|error| RegistryError::publish(format!("cannot post to `{url}`: {error}")))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(|error| RegistryError::publish(format!("cannot read `{url}`: {error}")))?;
        Ok(HttpResponse {
            status,
            etag: None,
            body,
        })
    }
}

/// A client that retries transient failures and caches successful bodies.
pub struct RegistryClient {
    transport: Box<dyn Transport>,
    cache: PathBuf,
    token: Option<Token>,
    offline: bool,
    attempts: usize,
}

impl RegistryClient {
    /// Creates a client for a configuration.
    pub fn new(config: &RegistryConfig) -> RegistryResult<Self> {
        let transport: Box<dyn Transport> =
            if config.bases().iter().all(|url| url.scheme() == "file") {
                Box::new(FileTransport)
            } else {
                Box::new(UreqTransport::default())
            };
        Self::with_transport(config, transport)
    }

    /// Creates a client with an injected transport, for tests.
    pub fn with_transport(
        config: &RegistryConfig,
        transport: Box<dyn Transport>,
    ) -> RegistryResult<Self> {
        let cache = config.cache_root.join("http");
        std::fs::create_dir_all(&cache).map_err(|error| {
            RegistryError::cache(format!("cannot create `{}`: {error}", cache.display()))
        })?;
        Ok(Self {
            transport,
            cache,
            token: config.token.clone(),
            offline: config.offline,
            attempts: 3,
        })
    }

    /// Fetches a path from the mirror or primary URL.
    ///
    /// A cached body is used without network in offline mode. When online, the
    /// request carries the cached ETag and a `304` reuses the cached body.
    pub fn fetch(&self, config: &RegistryConfig, path: &str) -> RegistryResult<Vec<u8>> {
        let mut last = None;
        for base in config.bases() {
            let url = crate::config::endpoint(base, path)?;
            match self.fetch_url(&url) {
                Ok(body) => return Ok(body),
                Err(error) if error.code() == "E8025" || error.code() == "E8085" => {
                    last = Some(error);
                }
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| {
            RegistryError::config(format!("the registry has no base URL for `{path}`"))
        }))
    }

    /// Fetches an absolute URL, with cache and retries.
    pub fn fetch_url(&self, url: &Url) -> RegistryResult<Vec<u8>> {
        let key = digest(url.as_str().as_bytes());
        let body_path = self.cache.join(&key);
        let meta_path = self.cache.join(format!("{key}.json"));
        let cached = std::fs::read(&body_path).ok();
        let meta = read_meta(&meta_path);
        if self.offline {
            return cached.ok_or_else(|| {
                RegistryError::offline(format!(
                    "`{url}` is not cached; run without `--offline` or vendor the dependency"
                ))
            });
        }
        let mut last_error = None;
        for attempt in 0..self.attempts {
            let etag = if attempt == 0 {
                meta.as_ref().and_then(|meta| meta.etag.as_deref())
            } else {
                None
            };
            match self.transport.get(url, self.token.as_ref(), etag) {
                Ok(response) if response.status == 304 => {
                    if let Some(body) = &cached {
                        return Ok(body.clone());
                    }
                    last_error = Some(RegistryError::cache(format!(
                        "`{url}` returned 304 but the cached body is missing"
                    )));
                }
                Ok(response) if response.status == 200 => {
                    let body_digest = digest(&response.body);
                    if let Some(meta) = &meta
                        && meta.sha256 != body_digest
                        && response.body.is_empty()
                    {
                        last_error = Some(RegistryError::cache(format!(
                            "`{url}` returned an empty body"
                        )));
                        continue;
                    }
                    let meta = CacheMeta {
                        etag: response.etag,
                        sha256: body_digest,
                    };
                    write_atomic(&body_path, &response.body)?;
                    write_meta(&meta_path, &meta)?;
                    return Ok(response.body);
                }
                Ok(response) if response.status == 404 => {
                    return Err(RegistryError::index(format!(
                        "the registry has no document at `{url}`"
                    )));
                }
                Ok(response) if response.status == 401 || response.status == 403 => {
                    return Err(RegistryError::credentials(format!(
                        "the registry rejected the request to `{url}`; check the token and its scope"
                    )));
                }
                Ok(response)
                    if matches!(response.status, 429 | 500 | 502 | 503 | 504)
                        && attempt + 1 < self.attempts =>
                {
                    last_error = Some(RegistryError::config(format!(
                        "the registry returned HTTP {} for `{url}`",
                        response.status
                    )));
                    std::thread::sleep(std::time::Duration::from_millis(100 * (1 << attempt)));
                }
                Ok(response) => {
                    return Err(RegistryError::config(format!(
                        "the registry returned HTTP {} for `{url}`",
                        response.status
                    )));
                }
                Err(error) if attempt + 1 < self.attempts => {
                    last_error = Some(error);
                    std::thread::sleep(std::time::Duration::from_millis(100 * (1 << attempt)));
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| RegistryError::config(format!("cannot fetch `{url}`"))))
    }

    /// Fetches a URL without reading or updating the HTTP cache.
    pub fn fetch_uncached(&self, url: &Url) -> RegistryResult<Vec<u8>> {
        if self.offline {
            return Err(RegistryError::offline(format!(
                "`{url}` requires network access; run without `--offline`"
            )));
        }
        match self.transport.get(url, self.token.as_ref(), None) {
            Ok(response) if response.status == 200 => Ok(response.body),
            Ok(response) if response.status == 404 => Err(RegistryError::index(format!(
                "the registry has no document at `{url}`"
            ))),
            Ok(response) if response.status == 401 || response.status == 403 => Err(
                RegistryError::credentials(format!("the registry rejected the request to `{url}`")),
            ),
            Ok(response) => Err(RegistryError::config(format!(
                "the registry returned HTTP {} for `{url}`",
                response.status
            ))),
            Err(error) => Err(error),
        }
    }

    /// Posts a JSON document to a path below the registry base.
    ///
    /// The response is returned for every HTTP status so the caller can map a
    /// structured `{"error": {...}}` body to a diagnostic code.
    pub fn post_json(
        &self,
        config: &RegistryConfig,
        path: &str,
        body: &[u8],
    ) -> RegistryResult<HttpResponse> {
        if self.offline {
            return Err(RegistryError::offline(format!(
                "posting to `{path}` needs network access; run without `--offline`"
            )));
        }
        let url = crate::config::endpoint(&config.url, path)?;
        let response = self.transport.post(&url, self.token.as_ref(), body)?;
        Ok(response)
    }

    /// Returns the cache directory.
    pub fn cache_dir(&self) -> &Path {
        &self.cache
    }
}

#[derive(Serialize, Deserialize)]
struct CacheMeta {
    etag: Option<String>,
    sha256: String,
}

fn read_meta(path: &Path) -> Option<CacheMeta> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_meta(path: &Path, meta: &CacheMeta) -> RegistryResult<()> {
    let text = serde_json::to_vec(meta)
        .map_err(|error| RegistryError::cache(format!("cannot encode cache metadata: {error}")))?;
    write_atomic(path, &text)
}

fn write_atomic(path: &Path, body: &[u8]) -> RegistryResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            RegistryError::cache(format!("cannot create `{}`: {error}", parent.display()))
        })?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, body).map_err(|error| {
        RegistryError::cache(format!("cannot write `{}`: {error}", temporary.display()))
    })?;
    std::fs::rename(&temporary, path).map_err(|error| {
        RegistryError::cache(format!("cannot replace `{}`: {error}", path.display()))
    })
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
