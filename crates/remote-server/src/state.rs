//! Shared service state, authentication, and instance checks.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use seal_remote::RemoteToken;
use seal_remote::proto;
use seal_sandbox::Sandbox;
use tonic::metadata::MetadataMap;
use tonic::{Request, Status};

use crate::config::ServerConfig;
use crate::error::{ServerError, ServerResult};
use crate::store::Store;

/// State shared by every REAPI service.
pub(crate) struct State {
    pub(crate) config: ServerConfig,
    pub(crate) store: Store,
    pub(crate) sandbox: Sandbox,
    pub(crate) operations: Mutex<HashMap<String, proto::google::longrunning::Operation>>,
    sequence: AtomicU64,
}

impl State {
    /// Opens the persistent store and constructs the state.
    pub(crate) fn open(config: ServerConfig) -> ServerResult<Self> {
        config.validate().map_err(ServerError::invalid)?;
        Ok(Self {
            store: Store::open(&config.root)?,
            sandbox: Sandbox::detect(),
            config,
            operations: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(0),
        })
    }

    /// Authorizes one request and validates its instance.
    pub(crate) fn authorize<T>(&self, request: &Request<T>, instance: &str) -> ServerResult<()> {
        authorize(request.metadata(), self.config.token.as_ref())?;
        if !self.config.instance_name.is_empty()
            && !instance.is_empty()
            && instance != self.config.instance_name
        {
            return Err(ServerError::invalid(format!(
                "the instance `{instance}` is not served by this endpoint"
            )));
        }
        Ok(())
    }

    /// Returns a unique operation name.
    pub(crate) fn operation_name(&self) -> String {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        format!("operations/{}-{sequence}-{nonce}", std::process::id())
    }
}

fn authorize(metadata: &MetadataMap, expected: Option<&RemoteToken>) -> ServerResult<()> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let header = metadata
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ServerError::unauthorized("the request has no bearer token"))?;
    let token = header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ServerError::unauthorized("the authorization header is not a bearer token"))?
        .trim();
    if !constant_time_eq(token.as_bytes(), expected.expose().as_bytes()) {
        return Err(ServerError::unauthorized(
            "the bearer token is not recognized",
        ));
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

/// Converts a server error into a gRPC status.
pub(crate) fn status(error: ServerError) -> Status {
    error.into()
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn token_comparison_accepts_only_equal_bytes() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secreu"));
        assert!(!constant_time_eq(b"secret", b"secre"));
    }
}
