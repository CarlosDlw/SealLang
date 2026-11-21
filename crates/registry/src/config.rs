//! Effective registry configuration.
//!
//! Precedence is command line, environment, root manifest, then defaults.
//! Secrets are held in [`crate::auth::Token`] and never formatted.

use std::fmt;
use std::path::{Path, PathBuf};

use seal_manifest::RegistryManifest;
use url::Url;

use crate::auth::{Token, load_token};
use crate::error::{RegistryError, RegistryResult};

/// The effective registry, trust, cache, and audit configuration.
#[derive(Clone)]
pub struct RegistryConfig {
    /// A stable registry name.
    pub name: String,
    /// The canonical sparse-index base URL.
    pub url: Url,
    /// An optional mirror tried before `url`.
    pub mirror: Option<Url>,
    /// The bearer token, when one was configured.
    pub token: Option<Token>,
    /// The TUF metadata base URL.
    pub tuf_url: Url,
    /// The local trusted root metadata.
    pub tuf_root: Option<PathBuf>,
    /// The advisory feed URL.
    pub advisories: Url,
    /// Whether network access is forbidden.
    pub offline: bool,
    /// The project-local registry cache.
    pub cache_root: PathBuf,
    /// Audit policy from the root manifest.
    pub policy: seal_manifest::RegistryPolicyManifest,
}

impl fmt::Debug for RegistryConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegistryConfig")
            .field("name", &self.name)
            .field("url", &self.url.as_str())
            .field("mirror", &self.mirror.as_ref().map(Url::as_str))
            .field("token", &self.token)
            .field("tuf_url", &self.tuf_url.as_str())
            .field("tuf_root", &self.tuf_root)
            .field("advisories", &self.advisories.as_str())
            .field("offline", &self.offline)
            .field("cache_root", &self.cache_root)
            .finish()
    }
}

impl RegistryConfig {
    /// Loads the configuration, returning `None` when no registry is declared.
    ///
    /// `forced_offline` is set by `--frozen` or `--offline`.
    pub fn load(
        project_root: &Path,
        manifest: Option<&RegistryManifest>,
        forced_offline: bool,
    ) -> RegistryResult<Option<Self>> {
        let env_url = std::env::var("SEAL_REGISTRY_URL").ok();
        let url = env_url
            .as_deref()
            .or_else(|| manifest.and_then(|manifest| manifest.url.as_deref()));
        let Some(url) = url else {
            if manifest.is_some()
                || env_url.is_some()
                || std::env::var_os("SEAL_REGISTRY_MIRROR").is_some()
            {
                return Err(RegistryError::config(
                    "a registry is configured without a URL; set `SEAL_REGISTRY_URL` or `[registry] url`",
                ));
            }
            return Ok(None);
        };
        let allow_insecure = manifest.is_some_and(|manifest| manifest.allow_insecure);
        let url = parse_base(url, allow_insecure, "registry URL")?;
        let mirror = std::env::var("SEAL_REGISTRY_MIRROR")
            .ok()
            .or_else(|| manifest.and_then(|manifest| manifest.mirror.clone()))
            .map(|mirror| parse_base(&mirror, allow_insecure, "registry mirror"))
            .transpose()?;
        let name = manifest
            .and_then(|manifest| manifest.name.clone())
            .unwrap_or_else(|| "default".to_owned());
        if name.is_empty() || name.chars().any(char::is_control) {
            return Err(RegistryError::config(
                "the registry name is empty or contains control characters",
            ));
        }
        let token_env = manifest
            .and_then(|manifest| manifest.token_env.clone())
            .unwrap_or_else(|| "SEAL_REGISTRY_TOKEN".to_owned());
        let token = load_token(
            project_root,
            &token_env,
            manifest.and_then(|manifest| manifest.credentials.as_deref()),
            &name,
        )?;
        let tuf_url = std::env::var("SEAL_REGISTRY_TUF_URL")
            .ok()
            .or_else(|| manifest.and_then(|manifest| manifest.tuf_url.clone()))
            .map(|url| parse_base(&url, allow_insecure, "TUF URL"))
            .transpose()?
            .unwrap_or_else(|| url.clone());
        let tuf_root = std::env::var_os("SEAL_REGISTRY_TUF_ROOT")
            .map(PathBuf::from)
            .or_else(|| manifest.and_then(|manifest| manifest.tuf_root.clone()))
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    project_root.join(path)
                }
            });
        let advisories = std::env::var("SEAL_REGISTRY_ADVISORIES")
            .ok()
            .or_else(|| manifest.and_then(|manifest| manifest.advisories.clone()))
            .map(|url| parse_base(&url, allow_insecure, "advisory URL"))
            .transpose()?
            .unwrap_or_else(|| endpoint(&url, "advisories.json").unwrap_or_else(|_| url.clone()));
        let offline = forced_offline
            || std::env::var("SEAL_OFFLINE").is_ok_and(|value| !value.is_empty())
            || manifest.is_some_and(|manifest| manifest.offline);
        let cache_root = project_root.join("seal").join("registry");
        Ok(Some(Self {
            name,
            url,
            mirror,
            token,
            tuf_url,
            tuf_root,
            advisories,
            offline,
            cache_root,
            policy: manifest
                .map(|manifest| manifest.policy.clone())
                .unwrap_or_default(),
        }))
    }

    /// Returns the base URLs in the order they should be tried.
    pub fn bases(&self) -> Vec<&Url> {
        self.mirror
            .iter()
            .chain(std::iter::once(&self.url))
            .collect()
    }

    /// Builds a URL below the index base.
    pub fn index_url(&self, path: &str) -> RegistryResult<Url> {
        endpoint(&self.url, path)
    }

    /// Builds a URL below the TUF base.
    pub fn tuf_endpoint(&self, path: &str) -> RegistryResult<Url> {
        endpoint(&self.tuf_url, path)
    }
}

fn parse_base(text: &str, allow_insecure: bool, kind: &str) -> RegistryResult<Url> {
    let mut url = Url::parse(text)
        .map_err(|error| RegistryError::config(format!("invalid {kind} `{text}`: {error}")))?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RegistryError::config(format!(
            "the {kind} must not embed credentials; use a token instead"
        )));
    }
    match url.scheme() {
        "https" | "file" => {}
        "http" if allow_insecure => {}
        "http" => {
            return Err(RegistryError::config(format!(
                "the {kind} `{text}` uses plain HTTP; set `allow-insecure = true` only for a local test registry"
            )));
        }
        scheme => {
            return Err(RegistryError::config(format!(
                "the {kind} uses the unsupported scheme `{scheme}`; use HTTPS or file"
            )));
        }
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// Joins a relative endpoint path to a base URL.
pub fn endpoint(base: &Url, path: &str) -> RegistryResult<Url> {
    if path.starts_with('/') || path.contains("..") || path.contains('\\') {
        return Err(RegistryError::config(format!(
            "the registry path `{path}` is not a safe relative URL"
        )));
    }
    let mut base = base.clone();
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    base.join(path)
        .map_err(|error| RegistryError::config(format!("cannot build the URL `{path}`: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_requires_an_explicit_opt_in() {
        assert!(parse_base("http://127.0.0.1:1", false, "registry URL").is_err());
        assert!(parse_base("http://127.0.0.1:1", true, "registry URL").is_ok());
    }

    #[test]
    fn embedded_credentials_are_rejected() {
        let error =
            parse_base("https://user:secret@example.com/", false, "registry URL").unwrap_err();
        assert!(!error.message().contains("secret"));
    }

    #[test]
    fn endpoint_rejects_parent_segments() {
        let base = Url::parse("https://example.com/").unwrap();
        assert!(endpoint(&base, "../secret").is_err());
        assert!(endpoint(&base, "/absolute").is_err());
    }
}
