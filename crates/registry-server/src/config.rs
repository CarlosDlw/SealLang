//! Server configuration.
//!
//! Paths are resolved relative to the configuration file. Tokens may be given
//! as plaintext in the local configuration or as precomputed `sha256:` hashes;
//! neither form is ever printed by the server.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ServerError, ServerResult};

/// The complete server configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct ServerConfig {
    /// The listening address.
    pub bind: String,
    /// The persistent data directory.
    pub storage: PathBuf,
    /// The maximum HTTP request body in bytes.
    pub max_body: usize,
    /// API requests allowed per caller per minute; zero disables the limit.
    pub requests_per_minute: u32,
    /// Published archive bytes allowed per namespace per day; zero disables it.
    pub publish_bytes_per_day: u64,
    /// Maximum versions per package; zero disables it.
    pub max_versions_per_package: usize,
    /// Whether every publication must carry valid provenance.
    pub require_provenance: bool,
    /// Namespaces approved by configuration, for local and organization setups.
    pub preapproved_namespaces: Vec<String>,
    /// A namespace applied to unscoped package names instead of the package name.
    pub default_namespace: Option<String>,
    /// The PEM certificate chain for HTTPS.
    pub tls_cert: Option<PathBuf>,
    /// The PEM private key for HTTPS.
    pub tls_key: Option<PathBuf>,
    /// TUF key seeds.
    pub tuf: TufConfig,
    /// Trusted-publishing OIDC configuration.
    pub oidc: Option<OidcConfig>,
    /// Static access tokens.
    pub tokens: Vec<TokenConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8787".to_owned(),
            storage: PathBuf::from(".seal-registry"),
            max_body: 64 * 1024 * 1024,
            requests_per_minute: 600,
            publish_bytes_per_day: 0,
            max_versions_per_package: 0,
            require_provenance: true,
            preapproved_namespaces: Vec::new(),
            default_namespace: None,
            tls_cert: None,
            tls_key: None,
            tuf: TufConfig::default(),
            oidc: None,
            tokens: Vec::new(),
        }
    }
}

/// TUF signing seeds, as 64 hexadecimal characters each.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct TufConfig {
    /// The root role seed.
    pub root_seed: Option<String>,
    /// The targets role seed.
    pub targets_seed: Option<String>,
    /// The snapshot role seed.
    pub snapshot_seed: Option<String>,
    /// The timestamp role seed.
    pub timestamp_seed: Option<String>,
    /// The transparency-log seed.
    pub log_seed: Option<String>,
}

impl TufConfig {
    /// Returns the five configured seeds, if every one is present.
    pub fn seeds(&self) -> ServerResult<Option<[[u8; 32]; 5]>> {
        let values = [
            &self.root_seed,
            &self.targets_seed,
            &self.snapshot_seed,
            &self.timestamp_seed,
            &self.log_seed,
        ];
        if values.iter().all(|value| value.is_none()) {
            return Ok(None);
        }
        let mut seeds = [[0u8; 32]; 5];
        for (index, value) in values.iter().enumerate() {
            let value = value.as_deref().ok_or_else(|| {
                ServerError::internal("the TUF configuration must set all five seeds or none")
            })?;
            seeds[index] = parse_seed(value)?;
        }
        Ok(Some(seeds))
    }
}

/// One static token.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct TokenConfig {
    /// The token name, used as an owner identity.
    pub name: String,
    /// The plaintext token. Exactly one of `token` and `hash` is required.
    #[serde(default)]
    pub token: Option<String>,
    /// The `sha256:<hex>` token digest.
    #[serde(default)]
    pub hash: Option<String>,
    /// Granted scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Namespaces the token may manage.
    #[serde(default)]
    pub namespaces: Vec<String>,
    /// Whether the token has every permission.
    #[serde(default)]
    pub admin: bool,
}

/// Trusted-publishing OIDC settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct OidcConfig {
    /// The required issuer.
    pub issuer: String,
    /// The required audience.
    pub audience: String,
    /// The 32-byte Ed25519 public key as lowercase hexadecimal.
    pub public: String,
    /// The scopes granted to a valid identity.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Namespaces the identity may manage.
    #[serde(default)]
    pub namespaces: Vec<String>,
}

impl ServerConfig {
    /// Loads a configuration file.
    pub fn load(path: &Path) -> ServerResult<Self> {
        let text = std::fs::read_to_string(path).map_err(|error| {
            ServerError::internal(format!("cannot read `{}`: {error}", path.display()))
        })?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        Self::parse(&text, base)
    }

    /// Parses configuration text, resolving paths against `base`.
    pub fn parse(text: &str, base: &Path) -> ServerResult<Self> {
        let mut config: Self = toml::from_str(text)
            .map_err(|error| ServerError::internal(format!("invalid configuration: {error}")))?;
        config.validate(base)?;
        Ok(config)
    }

    /// Validates limits and resolves filesystem paths.
    pub fn validate(&mut self, base: &Path) -> ServerResult<()> {
        if self.bind.is_empty() {
            return Err(ServerError::internal("`bind` cannot be empty"));
        }
        if self.max_body < 1024 || self.max_body > 1024 * 1024 * 1024 {
            return Err(ServerError::internal(
                "`max-body` must be between 1024 and 1073741824 bytes",
            ));
        }
        self.storage = resolve(base, &self.storage);
        self.tls_cert = self.tls_cert.as_deref().map(|path| resolve(base, path));
        self.tls_key = self.tls_key.as_deref().map(|path| resolve(base, path));
        if self.tls_cert.is_some() != self.tls_key.is_some() {
            return Err(ServerError::internal(
                "`tls-cert` and `tls-key` must be set together",
            ));
        }
        for token in &self.tokens {
            if token.name.is_empty() || token.name.chars().any(char::is_control) {
                return Err(ServerError::internal(
                    "a static token has an empty or invalid name",
                ));
            }
            if token.token.is_some() == token.hash.is_some() {
                return Err(ServerError::internal(format!(
                    "the token `{}` must set exactly one of `token` or `hash`",
                    token.name
                )));
            }
            if let Some(hash) = &token.hash {
                seal_registry::index::validate_checksum(hash).map_err(|error| {
                    ServerError::internal(format!(
                        "the token `{}` has an invalid hash: {}",
                        token.name,
                        error.message()
                    ))
                })?;
            }
        }
        for namespace in &self.preapproved_namespaces {
            validate_namespace(namespace)?;
        }
        if let Some(namespace) = &self.default_namespace {
            validate_namespace(namespace)?;
        }
        if let Some(oidc) = &self.oidc {
            if oidc.issuer.is_empty() || oidc.audience.is_empty() {
                return Err(ServerError::internal(
                    "the OIDC `issuer` and `audience` cannot be empty",
                ));
            }
            if oidc.public.len() != 64 || !oidc.public.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(ServerError::internal(
                    "the OIDC `public` key must be 64 hexadecimal characters",
                ));
            }
        }
        Ok(())
    }

    /// Returns whether the configuration enables HTTPS.
    pub fn tls(&self) -> bool {
        self.tls_cert.is_some()
    }
}

/// Validates a namespace name.
pub fn validate_namespace(namespace: &str) -> ServerResult<()> {
    let value = namespace.strip_prefix('@').unwrap_or(namespace);
    if value.is_empty()
        || value.len() > 214
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        || value.starts_with('.')
        || value.ends_with('.')
        || value.contains("..")
    {
        return Err(ServerError::bad_request(format!(
            "`{namespace}` is not a valid namespace"
        )));
    }
    Ok(())
}

/// Returns the namespace a package name belongs to.
pub fn package_namespace(name: &str, default_namespace: Option<&str>) -> ServerResult<String> {
    if let Some(rest) = name.strip_prefix('@') {
        let (scope, package) = rest.split_once('/').ok_or_else(|| {
            ServerError::bad_request(format!(
                "the scoped package `{name}` must be written `@scope/name`"
            ))
        })?;
        if scope.is_empty() || package.is_empty() {
            return Err(ServerError::bad_request(format!(
                "the scoped package `{name}` must be written `@scope/name`"
            )));
        }
        let namespace = format!("@{scope}");
        validate_namespace(&namespace)?;
        return Ok(namespace);
    }
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ServerError::bad_request(format!(
            "`{name}` is not a valid package name"
        )));
    }
    Ok(default_namespace.unwrap_or(name).to_owned())
}

fn resolve(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

/// Parses a 32-byte seed from 64 hexadecimal characters.
pub fn parse_seed(text: &str) -> ServerResult<[u8; 32]> {
    let text = text.trim();
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServerError::internal(
            "a seed must be 64 hexadecimal characters",
        ));
    }
    let bytes = hex::decode(text)
        .map_err(|error| ServerError::internal(format!("a seed is invalid: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| ServerError::internal("a seed must be 32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_resolve_against_the_configuration() {
        let config = ServerConfig::parse(
            "storage = \"data\"\n[[tokens]]\nname = \"admin\"\ntoken = \"secret\"\nscopes = [\"admin\"]\n",
            Path::new("/srv/registry"),
        )
        .unwrap();
        assert_eq!(config.storage, Path::new("/srv/registry/data"));
    }

    #[test]
    fn a_token_needs_exactly_one_secret() {
        let text = "[[tokens]]\nname = \"admin\"\n";
        assert!(ServerConfig::parse(text, Path::new(".")).is_err());
    }

    #[test]
    fn package_namespaces_are_scoped_or_explicit() {
        assert_eq!(package_namespace("@scope/pkg", None).unwrap(), "@scope");
        assert_eq!(package_namespace("demo", Some("public")).unwrap(), "public");
        assert!(package_namespace("@scope", None).is_err());
    }
}
