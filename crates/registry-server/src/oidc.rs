//! Trusted-publishing OIDC tokens.
//!
//! The server accepts a compact EdDSA JWT signed by the configured issuer key.
//! It checks the signature, issuer, audience, expiry, subject, and namespace
//! claims before granting a short-lived publishing identity.

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;

use crate::auth::Principal;
use crate::config::OidcConfig;
use crate::error::{ServerError, ServerResult};

/// Verifies a JWT and returns the publishing identity.
pub fn verify(token: &str, config: &OidcConfig) -> ServerResult<Principal> {
    let mut parts = token.split('.');
    let (Some(header_part), Some(payload_part), Some(signature)) =
        (parts.next(), parts.next(), parts.next())
    else {
        return Err(ServerError::unauthorized(
            "the OIDC token is not a compact JWT",
        ));
    };
    if parts.next().is_some() {
        return Err(ServerError::unauthorized(
            "the OIDC token has too many segments",
        ));
    }
    if token.len() > 16 * 1024 {
        return Err(ServerError::unauthorized("the OIDC token is too large"));
    }
    let header: Header = decode_json(header_part, "header")?;
    if header.alg != "EdDSA" {
        return Err(ServerError::unauthorized(format!(
            "the OIDC algorithm `{}` is not EdDSA",
            header.alg
        )));
    }
    let public = decode_hex(&config.public)?;
    let key = VerifyingKey::from_bytes(&public)
        .map_err(|error| ServerError::unauthorized(format!("the OIDC key is invalid: {error}")))?;
    let signature = decode_base64(signature, "signature")?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| ServerError::unauthorized("the OIDC signature is not 64 bytes"))?;
    let message = format!("{header_part}.{payload_part}");
    key.verify_strict(message.as_bytes(), &Signature::from_bytes(&signature))
        .map_err(|_| ServerError::unauthorized("the OIDC signature is invalid"))?;
    let payload: Value = decode_json(payload_part, "payload")?;
    let claims = Claims::from_value(&payload)?;
    if claims.iss != config.issuer {
        return Err(ServerError::unauthorized(format!(
            "the OIDC issuer `{}` is not `{}`",
            claims.iss, config.issuer
        )));
    }
    if !claims.audience.iter().any(|aud| aud == &config.audience) {
        return Err(ServerError::unauthorized(format!(
            "the OIDC audience does not include `{}`",
            config.audience
        )));
    }
    let now = OffsetDateTime::now_utc().unix_timestamp();
    if claims.exp <= now {
        return Err(ServerError::unauthorized("the OIDC token expired"));
    }
    if claims.exp > now + 3600 {
        return Err(ServerError::unauthorized(
            "the OIDC token has an unreasonable lifetime",
        ));
    }
    if let Some(not_before) = claims.nbf
        && not_before > now + 30
    {
        return Err(ServerError::unauthorized("the OIDC token is not valid yet"));
    }
    if claims.sub.is_empty() || claims.sub.len() > 512 {
        return Err(ServerError::unauthorized(
            "the OIDC token has an invalid subject",
        ));
    }
    let mut scopes = claims.scopes;
    if scopes.is_empty() {
        scopes = config.scopes.clone();
    }
    if scopes.is_empty() {
        scopes = vec!["publish".to_owned(), "yank".to_owned(), "owners".to_owned()];
    }
    let mut namespaces = claims.namespaces;
    namespaces.extend(config.namespaces.iter().cloned());
    namespaces.sort();
    namespaces.dedup();
    Ok(Principal {
        name: format!("oidc:{}", claims.sub),
        subject: Some(claims.sub),
        scopes,
        namespaces,
        admin: false,
        oidc: true,
    })
}

#[derive(Deserialize)]
struct Header {
    alg: String,
    #[allow(dead_code)]
    #[serde(default)]
    typ: Option<String>,
}

struct Claims {
    iss: String,
    sub: String,
    audience: Vec<String>,
    exp: i64,
    nbf: Option<i64>,
    scopes: Vec<String>,
    namespaces: Vec<String>,
}

impl Claims {
    fn from_value(value: &Value) -> ServerResult<Self> {
        let object = value
            .as_object()
            .ok_or_else(|| ServerError::unauthorized("the OIDC payload is not an object"))?;
        let iss = string(object.get("iss"), "iss")?;
        let sub = string(object.get("sub"), "sub")?;
        let exp = object
            .get("exp")
            .and_then(Value::as_i64)
            .ok_or_else(|| ServerError::unauthorized("the OIDC payload has no `exp`"))?;
        let audience = match object.get("aud") {
            Some(Value::String(value)) => vec![value.clone()],
            Some(Value::Array(values)) => values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
            _ => {
                return Err(ServerError::unauthorized(
                    "the OIDC payload has no valid `aud`",
                ));
            }
        };
        let nbf = object.get("nbf").and_then(Value::as_i64);
        let mut scopes = Vec::new();
        if let Some(scope) = object.get("scope").and_then(Value::as_str) {
            scopes.extend(
                scope
                    .split_whitespace()
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned),
            );
        }
        if let Some(values) = object.get("scopes").and_then(Value::as_array) {
            scopes.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned),
            );
        }
        let mut namespaces = Vec::new();
        match object.get("namespace") {
            Some(Value::String(value)) => namespaces.push(value.clone()),
            Some(Value::Array(values)) => namespaces.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned),
            ),
            _ => {}
        }
        if let Some(values) = object.get("namespaces").and_then(Value::as_array) {
            namespaces.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned),
            );
        }
        Ok(Self {
            iss,
            sub,
            audience,
            exp,
            nbf,
            scopes,
            namespaces,
        })
    }
}

fn string(value: Option<&Value>, name: &str) -> ServerResult<String> {
    value
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| ServerError::unauthorized(format!("the OIDC payload has no `{name}`")))
}

fn decode_json<T: serde::de::DeserializeOwned>(part: &str, name: &str) -> ServerResult<T> {
    let bytes = decode_base64(part, name)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| ServerError::unauthorized(format!("the OIDC {name} is invalid: {error}")))
}

fn decode_base64(part: &str, name: &str) -> ServerResult<Vec<u8>> {
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    engine
        .decode(part)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(part))
        .map_err(|error| {
            ServerError::unauthorized(format!("the OIDC {name} is not base64: {error}"))
        })
}

fn decode_hex(text: &str) -> ServerResult<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServerError::internal(
            "the OIDC public key must be 64 hexadecimal characters",
        ));
    }
    let bytes = hex::decode(text)
        .map_err(|error| ServerError::internal(format!("invalid OIDC key: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| ServerError::internal("the OIDC key must be 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn token(seed: &[u8; 32], claims: Value) -> String {
        let header = serde_json::json!({"alg": "EdDSA", "typ": "JWT"});
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&header).unwrap());
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        let message = format!("{header}.{payload}");
        let signature = SigningKey::from_bytes(seed).sign(message.as_bytes());
        let signature =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(signature.to_bytes());
        format!("{message}.{signature}")
    }

    fn config(seed: &[u8; 32]) -> OidcConfig {
        OidcConfig {
            issuer: "https://token.example".to_owned(),
            audience: "seal-registry".to_owned(),
            public: hex::encode(SigningKey::from_bytes(seed).verifying_key().to_bytes()),
            scopes: vec!["publish".to_owned()],
            namespaces: Vec::new(),
        }
    }

    fn claims() -> Value {
        serde_json::json!({
            "iss": "https://token.example",
            "aud": "seal-registry",
            "sub": "repo:acme/demo:ref:refs/heads/main",
            "exp": OffsetDateTime::now_utc().unix_timestamp() + 600,
            "namespace": "acme"
        })
    }

    #[test]
    fn a_valid_token_yields_an_identity() {
        let seed = [9u8; 32];
        let principal = verify(&token(&seed, claims()), &config(&seed)).unwrap();
        assert_eq!(principal.name, "oidc:repo:acme/demo:ref:refs/heads/main");
        assert_eq!(principal.namespaces, vec!["acme"]);
        assert!(principal.can("publish", "acme"));
    }

    #[test]
    fn the_wrong_issuer_or_audience_is_rejected() {
        let seed = [9u8; 32];
        let mut wrong = claims();
        wrong["iss"] = Value::String("https://evil.example".to_owned());
        assert!(verify(&token(&seed, wrong), &config(&seed)).is_err());
        let mut wrong = claims();
        wrong["aud"] = Value::String("other".to_owned());
        assert!(verify(&token(&seed, wrong), &config(&seed)).is_err());
    }

    #[test]
    fn an_expired_token_is_rejected() {
        let seed = [9u8; 32];
        let mut expired = claims();
        expired["exp"] = Value::from(OffsetDateTime::now_utc().unix_timestamp() - 1);
        assert!(verify(&token(&seed, expired), &config(&seed)).is_err());
    }
}
