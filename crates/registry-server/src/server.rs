//! The HTTP server, routing, rate limiting, and TLS listener.

use std::collections::{BTreeMap, VecDeque};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use seal_registry::publish::{OwnerRequest, OwnerResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::Principal;
use crate::config::ServerConfig;
use crate::dns;
use crate::error::{ServerError, ServerResult};
use crate::http::{self, Request, Response};
use crate::publish;
use crate::state::NamespaceRecord;
use crate::store::Store;

/// A bound registry server.
pub struct Server {
    listener: TcpListener,
    store: Arc<Store>,
    tls: Option<Arc<rustls::ServerConfig>>,
    limiter: RateLimiter,
}

impl Server {
    /// Binds the configured address and opens the store.
    pub fn bind(config: ServerConfig) -> ServerResult<Self> {
        let listener = TcpListener::bind(&config.bind).map_err(|error| {
            ServerError::internal(format!("cannot bind `{}`: {error}", config.bind))
        })?;
        listener.set_nonblocking(true)?;
        let tls = match (&config.tls_cert, &config.tls_key) {
            (Some(cert), Some(key)) => Some(http::load_tls(cert, key)?),
            _ => None,
        };
        let limiter = RateLimiter::new(config.requests_per_minute);
        let store = Store::open(config)?;
        Ok(Self {
            listener,
            store: Arc::new(store),
            tls,
            limiter,
        })
    }

    /// Returns the bound address.
    pub fn local_addr(&self) -> ServerResult<SocketAddr> {
        self.listener.local_addr().map_err(|error| {
            ServerError::internal(format!("cannot read the local address: {error}"))
        })
    }

    /// Returns the store.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Binds and serves on a background thread.
    pub fn start(config: ServerConfig) -> ServerResult<RunningServer> {
        let server = Self::bind(config)?;
        let address = server.local_addr()?;
        let store = Arc::clone(&server.store);
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&shutdown);
        let handle = std::thread::spawn(move || server.run(flag));
        Ok(RunningServer {
            address,
            store,
            shutdown,
            handle: Some(handle),
        })
    }

    /// Serves until the shutdown flag is set.
    pub fn run(self, shutdown: Arc<AtomicBool>) -> ServerResult<()> {
        let server = Arc::new(self);
        while !shutdown.load(Ordering::Relaxed) {
            match server.listener.accept() {
                Ok((stream, peer)) => {
                    let server = Arc::clone(&server);
                    std::thread::spawn(move || {
                        if let Err(error) = server.connection(stream, peer) {
                            eprintln!("registry: {error}");
                        }
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(error) => {
                    return Err(ServerError::internal(format!(
                        "cannot accept a connection: {error}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn connection(&self, mut stream: TcpStream, peer: SocketAddr) -> ServerResult<()> {
        stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
        stream.set_write_timeout(Some(Duration::from_secs(30))).ok();
        let key = peer.ip().to_string();
        if !self.limiter.check(&key) {
            let error = ServerError::quota("the request rate limit was exceeded");
            http::write_response(&mut stream, &Response::from(error))?;
            return Ok(());
        }
        let max_body = self.store.config().max_body;
        if let Some(tls) = &self.tls {
            return http::serve_tls(stream, tls, max_body, |request| self.route(request, &key));
        }
        let Some(request) = http::read_request(&mut stream, max_body)? else {
            return Ok(());
        };
        let response = self.route(&request, &key);
        http::write_response(&mut stream, &response)
    }

    fn route(&self, request: &Request, peer: &str) -> Response {
        route(request, peer, &self.store).unwrap_or_else(Response::from)
    }
}

/// A server running on a background thread.
pub struct RunningServer {
    address: SocketAddr,
    store: Arc<Store>,
    shutdown: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<ServerResult<()>>>,
}

impl RunningServer {
    /// Returns the bound address.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Returns the base URL.
    pub fn url(&self) -> String {
        format!("http://{}/", self.address)
    }

    /// Returns the running store.
    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Stops the server and joins its thread.
    pub fn stop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        self.stop();
    }
}

struct RateLimiter {
    limit: u32,
    hits: Mutex<BTreeMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    fn new(limit: u32) -> Self {
        Self {
            limit,
            hits: Mutex::new(BTreeMap::new()),
        }
    }

    fn check(&self, key: &str) -> bool {
        if self.limit == 0 {
            return true;
        }
        let now = Instant::now();
        let mut hits = self.hits.lock().unwrap_or_else(|error| error.into_inner());
        let window = hits.entry(key.to_owned()).or_default();
        while window
            .front()
            .is_some_and(|time| now.duration_since(*time) > Duration::from_secs(60))
        {
            window.pop_front();
        }
        if window.len() >= self.limit as usize {
            return false;
        }
        window.push_back(now);
        true
    }
}

fn route(request: &Request, _peer: &str, store: &Store) -> ServerResult<Response> {
    if request.path == "/health" && request.method == "GET" {
        return Ok(Response::ok());
    }
    if request.method == "GET" || request.method == "HEAD" {
        if request.path == "/api/v1/log/sth" {
            return Response::json(200, &store.tree_head()?);
        }
        if let Some(index) = request.path.strip_prefix("/api/v1/log/") {
            let index = index
                .parse::<u64>()
                .map_err(|_| ServerError::bad_request(format!("`{index}` is not a log index")))?;
            return Response::json(200, &store.log_proof(index)?);
        }
        if let Some(bytes) = store.static_file(&request.path) {
            let tag = http::etag(&bytes);
            if request.header("if-none-match") == Some(tag.as_str()) {
                return Ok(Response {
                    status: 304,
                    content_type: http::content_type(&request.path).to_owned(),
                    headers: vec![("ETag".to_owned(), tag)],
                    body: Vec::new(),
                });
            }
            let mut response = Response::bytes(200, http::content_type(&request.path), bytes);
            response.headers.push(("ETag".to_owned(), tag));
            return Ok(response);
        }
        return Ok(Response::bytes(
            404,
            "application/json",
            b"{\"error\":{\"code\":\"E8025\",\"message\":\"no such registry file\"}}".to_vec(),
        ));
    }
    if request.method != "POST" {
        return Ok(Response::bytes(
            405,
            "application/json",
            b"{\"error\":{\"code\":\"E8093\",\"message\":\"method not allowed\"}}".to_vec(),
        ));
    }
    if !request.path.starts_with("/api/v1/") {
        return Err(ServerError::not_found("no such API endpoint"));
    }
    let principal = authenticate(store, request)?;
    match request.path.as_str() {
        "/api/v1/whoami" => {
            let state = store.state();
            let mut namespaces: Vec<String> = state
                .namespaces
                .iter()
                .filter(|(_, record)| {
                    record.owners.contains(&principal.name)
                        || principal
                            .subject
                            .as_ref()
                            .is_some_and(|subject| record.owners.contains(subject))
                })
                .map(|(name, _)| name.clone())
                .collect();
            namespaces.extend(principal.namespaces.iter().cloned());
            namespaces.sort();
            namespaces.dedup();
            Response::json(
                200,
                &WhoAmIResponse {
                    name: principal.name.clone(),
                    scopes: principal.scopes.clone(),
                    namespaces,
                    subject: principal.subject.clone(),
                },
            )
        }
        "/api/v1/publish" => {
            let body: seal_registry::publish::PublishRequest = decode(&request.body)?;
            Response::json(200, &publish::publish(store, &principal, &body)?)
        }
        "/api/v1/yank" => {
            let body: seal_registry::publish::YankRequest = decode(&request.body)?;
            Response::json(200, &publish::yank(store, &principal, &body)?)
        }
        "/api/v1/owners" => {
            let body: OwnerRequest = decode(&request.body)?;
            let response = owners(store, &principal, &body)?;
            Response::json(200, &response)
        }
        "/api/v1/namespaces" => {
            let body: OwnerRequest = decode(&request.body)?;
            let record = claim(store, &principal, &body)?;
            Response::json(200, &owner_response(&body.namespace, &record))
        }
        "/api/v1/tokens" => {
            let body: CreateTokenRequest = decode(&request.body)?;
            let (secret, record) =
                store.create_token(&principal, &body.name, body.scopes, body.namespaces)?;
            Response::json(
                200,
                &CreateTokenResponse {
                    token: secret,
                    name: record.name,
                    scopes: record.scopes,
                    namespaces: record.namespaces,
                },
            )
        }
        "/api/v1/tuf/rotate" => {
            let root = store.rotate(&principal)?;
            Response::json(200, &RotateResponse { root })
        }
        _ => Err(ServerError::not_found("no such API endpoint")),
    }
}

fn authenticate(store: &Store, request: &Request) -> ServerResult<Principal> {
    let state = store.state();
    let tokens: Vec<crate::state::TokenRecord> = state.tokens.values().cloned().collect();
    drop(state);
    crate::auth::authenticate(request, store.config(), &tokens)
}

fn owners(
    store: &Store,
    principal: &Principal,
    request: &OwnerRequest,
) -> ServerResult<OwnerResponse> {
    match request.action.as_str() {
        "list" => {
            store.authorize(principal, "owners", &request.namespace)?;
            let state = store.state();
            let record = state.namespaces.get(&request.namespace).ok_or_else(|| {
                ServerError::not_found(format!("the namespace `{}` is unknown", request.namespace))
            })?;
            Ok(owner_response(&request.namespace, record))
        }
        "add" | "remove" => {
            let record = store.owners(
                principal,
                &request.namespace,
                &request.action,
                request.owner.as_deref(),
            )?;
            Ok(owner_response(&request.namespace, &record))
        }
        "claim" => {
            let record = claim(store, principal, request)?;
            Ok(owner_response(&request.namespace, &record))
        }
        other => Err(ServerError::bad_request(format!(
            "the owner action `{other}` is not `list`, `add`, `remove`, or `claim`"
        ))),
    }
}

fn claim(
    store: &Store,
    principal: &Principal,
    request: &OwnerRequest,
) -> ServerResult<NamespaceRecord> {
    let proof = request
        .proof
        .as_ref()
        .ok_or_else(|| ServerError::bad_request("a namespace claim needs a `proof`"))?;
    crate::config::validate_namespace(&request.namespace)?;
    let (kind, value) = (proof.kind.as_str(), proof.value.as_deref());
    let preapproved = store
        .config()
        .preapproved_namespaces
        .iter()
        .any(|namespace| namespace == &request.namespace);
    let kind = match kind {
        "config" | "org" if preapproved || principal.admin => "config",
        "oidc" => {
            let subject = principal.subject.as_deref().ok_or_else(|| {
                ServerError::forbidden("an `oidc` claim needs an authenticated OIDC token")
            })?;
            if let Some(value) = value
                && value != subject
            {
                return Err(ServerError::forbidden(
                    "the OIDC proof does not match the authenticated subject",
                ));
            }
            "oidc"
        }
        "dns" => {
            let token = value.ok_or_else(|| {
                ServerError::bad_request("a `dns` claim needs the TXT verification token")
            })?;
            dns::verify(&request.namespace, token)?;
            "dns"
        }
        other => {
            return Err(ServerError::bad_request(format!(
                "the proof kind `{other}` is not `config`, `org`, `oidc`, or `dns`"
            )));
        }
    };
    store.claim_namespace(principal, &request.namespace, kind)
}

fn owner_response(namespace: &str, record: &NamespaceRecord) -> OwnerResponse {
    OwnerResponse {
        namespace: namespace.to_owned(),
        owners: record.owners.iter().cloned().collect(),
        verified: record.verified.clone(),
    }
}

fn decode<T: serde::de::DeserializeOwned>(body: &[u8]) -> ServerResult<T> {
    if body.len() > 128 * 1024 * 1024 {
        return Err(ServerError::bad_request("the JSON body is too large"));
    }
    serde_json::from_slice(body)
        .map_err(|error| ServerError::bad_request(format!("the JSON request is invalid: {error}")))
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    name: String,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    namespaces: Vec<String>,
}

#[derive(Serialize)]
struct CreateTokenResponse {
    token: String,
    name: String,
    scopes: Vec<String>,
    namespaces: Vec<String>,
}

#[derive(Serialize)]
struct WhoAmIResponse {
    name: String,
    scopes: Vec<String>,
    namespaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
}

#[derive(Serialize)]
struct RotateResponse {
    root: Value,
}
