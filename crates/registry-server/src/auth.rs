//! Authentication, scopes, and namespace authorization.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::config::ServerConfig;
use crate::error::{ServerError, ServerResult};
use crate::http::Request;
use crate::oidc;
use crate::state::TokenRecord;

/// The identity behind one request.
#[derive(Clone, Debug)]
pub struct Principal {
    /// The token name or `oidc:<subject>`.
    pub name: String,
    /// Granted scopes.
    pub scopes: Vec<String>,
    /// Namespaces explicitly granted to the token.
    pub namespaces: Vec<String>,
    /// Whether the identity has every permission.
    pub admin: bool,
    /// The OIDC subject, when trusted publishing authenticated the request.
    pub subject: Option<String>,
    /// Whether the identity came from OIDC.
    pub oidc: bool,
}

impl Principal {
    /// Returns whether the identity grants `action` on `namespace`.
    pub fn can(&self, action: &str, namespace: &str) -> bool {
        if self.admin {
            return true;
        }
        self.scopes.iter().any(|scope| {
            let (scope_action, target) = scope.split_once(':').unwrap_or((scope.as_str(), "*"));
            if scope_action != action {
                return false;
            }
            target == "*" || target == namespace
        })
    }
}

/// Hashes a bearer token as `sha256:<hex>`.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Compares two byte strings without an early exit.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

/// Converts configured tokens into state records.
pub fn configured_tokens(config: &ServerConfig) -> ServerResult<Vec<TokenRecord>> {
    let mut records = Vec::with_capacity(config.tokens.len());
    for token in &config.tokens {
        let hash = match (&token.token, &token.hash) {
            (Some(value), None) => hash_token(value),
            (None, Some(value)) => value.clone(),
            _ => {
                return Err(ServerError::internal(format!(
                    "the token `{}` must set exactly one of `token` or `hash`",
                    token.name
                )));
            }
        };
        records.push(TokenRecord {
            name: token.name.clone(),
            hash,
            scopes: token.scopes.clone(),
            namespaces: token.namespaces.clone(),
            admin: token.admin,
            created_at: "configured".to_owned(),
        });
    }
    Ok(records)
}

/// Authenticates a request.
pub fn authenticate(
    request: &Request,
    config: &ServerConfig,
    tokens: &[TokenRecord],
) -> ServerResult<Principal> {
    let token = request
        .bearer()
        .ok_or_else(|| ServerError::unauthorized("the request has no bearer token"))?;
    if token.len() > 16 * 1024 {
        return Err(ServerError::unauthorized("the bearer token is too large"));
    }
    if let Some(oidc) = &config.oidc
        && token.starts_with("eyJ")
    {
        return oidc::verify(token, oidc);
    }
    let digest = hash_token(token);
    for record in tokens {
        if constant_time_eq(digest.as_bytes(), record.hash.as_bytes()) {
            return Ok(Principal {
                name: record.name.clone(),
                scopes: record.scopes.clone(),
                namespaces: record.namespaces.clone(),
                admin: record.admin,
                subject: None,
                oidc: false,
            });
        }
    }
    Err(ServerError::unauthorized(
        "the bearer token is not recognized; run `seal login` again",
    ))
}

/// Checks namespace ownership or a token grant.
pub fn authorize_namespace(
    principal: &Principal,
    action: &str,
    namespace: &str,
    owners: &BTreeSet<String>,
) -> ServerResult<()> {
    if principal.admin {
        return Ok(());
    }
    if !principal.can(action, namespace) {
        return Err(ServerError::forbidden(format!(
            "the identity `{}` lacks the `{action}` scope for `{namespace}`",
            principal.name
        )));
    }
    if owners.contains(&principal.name)
        || principal
            .subject
            .as_ref()
            .is_some_and(|subject| owners.contains(subject))
        || principal.namespaces.iter().any(|value| value == namespace)
        || principal.oidc
    {
        return Ok(());
    }
    Err(ServerError::forbidden(format!(
        "the namespace `{namespace}` is not owned by `{}`; claim it with `seal owner`",
        principal.name
    )))
}

/// Returns the scopes granted to a new token.
pub fn token_scopes(principal: &Principal, requested: &[String]) -> ServerResult<Vec<String>> {
    if !principal.admin {
        return Err(ServerError::forbidden(
            "only an administrator may create tokens",
        ));
    }
    if requested.is_empty() {
        return Err(ServerError::bad_request("a token needs at least one scope"));
    }
    for scope in requested {
        let action = scope
            .split_once(':')
            .map_or(scope.as_str(), |(action, _)| action);
        if !matches!(action, "publish" | "yank" | "owners" | "read") {
            return Err(ServerError::bad_request(format!(
                "the scope `{scope}` is not recognized"
            )));
        }
    }
    Ok(requested.to_vec())
}

/// Generates a new token secret.
pub fn generate_token(id: u64) -> ServerResult<String> {
    let mut bytes = [0u8; 24];
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut file = std::fs::File::open("/dev/urandom").map_err(|error| {
            ServerError::internal(format!("cannot open the random source: {error}"))
        })?;
        file.read_exact(&mut bytes).map_err(|error| {
            ServerError::internal(format!("cannot read the random source: {error}"))
        })?;
    }
    #[cfg(not(unix))]
    {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(id.to_be_bytes());
        hasher.update(crate::http::random_nonce().to_be_bytes());
        hasher.update(std::process::id().to_be_bytes());
        bytes.copy_from_slice(&hasher.finalize()[..24]);
    }
    Ok(format!("seal_{id}_{}", hex::encode(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_stable_and_constant_time_comparison_works() {
        let hash = hash_token("secret");
        assert_eq!(hash, hash_token("secret"));
        assert!(constant_time_eq(
            hash.as_bytes(),
            hash_token("secret").as_bytes()
        ));
        assert!(!constant_time_eq(
            hash.as_bytes(),
            hash_token("other").as_bytes()
        ));
    }

    #[test]
    fn scopes_are_action_and_namespace_specific() {
        let principal = Principal {
            name: "ci".to_owned(),
            scopes: vec!["publish:@acme".to_owned()],
            namespaces: vec!["@acme".to_owned()],
            admin: false,
            subject: None,
            oidc: false,
        };
        assert!(principal.can("publish", "@acme"));
        assert!(!principal.can("publish", "@other"));
        assert!(!principal.can("yank", "@acme"));
    }

    #[test]
    fn an_admin_scope_grants_everything() {
        let principal = Principal {
            name: "admin".to_owned(),
            scopes: vec!["admin".to_owned()],
            namespaces: Vec::new(),
            admin: true,
            subject: None,
            oidc: false,
        };
        assert!(principal.can("publish", "anything"));
        let owners = BTreeSet::new();
        authorize_namespace(&principal, "publish", "anything", &owners).unwrap();
    }
}
