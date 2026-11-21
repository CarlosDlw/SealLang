//! Registry credentials and token loading.
//!
//! Tokens come from an environment variable or from a credentials file. The
//! file must not be readable by other users on Unix; a token is never printed,
//! debug-formatted, or included in an error message.

use std::fmt;
use std::path::{Path, PathBuf};

use crate::error::{RegistryError, RegistryResult};

/// A registry bearer token.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl Token {
    /// Creates a token after rejecting empty and control characters.
    pub fn new(value: impl Into<String>) -> RegistryResult<Self> {
        let value = value.into();
        if value.trim().is_empty() || value.chars().any(char::is_control) {
            return Err(RegistryError::credentials(
                "the registry token is empty or contains control characters",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the token value for the Authorization header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token([redacted])")
    }
}

/// Loads a token from `token_env`, then from `credentials`.
///
/// `credentials` is the explicit path from the manifest. When it is absent,
/// `$SEAL_CREDENTIALS`, `$XDG_CONFIG_HOME/seal/credentials.toml`, and
/// `$HOME/.config/seal/credentials.toml` are tried in that order.
pub fn load_token(
    project_root: &Path,
    token_env: &str,
    credentials: Option<&Path>,
    registry_name: &str,
) -> RegistryResult<Option<Token>> {
    if let Some(value) = std::env::var_os(token_env) {
        let value = value.into_string().map_err(|_| {
            RegistryError::credentials(format!(
                "the token environment variable `{token_env}` is not valid UTF-8"
            ))
        })?;
        return Token::new(value).map(Some);
    }
    let Some(path) = credentials_path(project_root, credentials) else {
        return Ok(None);
    };
    if !path.is_file() {
        return Ok(None);
    }
    check_permissions(&path)?;
    let text = std::fs::read_to_string(&path).map_err(|error| {
        RegistryError::credentials(format!("cannot read `{}`: {error}", path.display()))
    })?;
    let value: toml::Value = toml::from_str(&text).map_err(|error| {
        RegistryError::credentials(format!(
            "invalid credentials file `{}`: {error}",
            path.display()
        ))
    })?;
    let token = value
        .get("token")
        .and_then(toml::Value::as_str)
        .or_else(|| {
            value
                .get("registries")
                .and_then(|registries| registries.get(registry_name))
                .and_then(|registry| registry.get("token"))
                .and_then(toml::Value::as_str)
        });
    token.map(Token::new).transpose()
}

fn credentials_path(project_root: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(if path.is_absolute() {
            path.to_path_buf()
        } else {
            project_root.join(path)
        });
    }
    if let Some(path) = std::env::var_os("SEAL_CREDENTIALS") {
        return Some(PathBuf::from(path));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("seal").join("credentials.toml"))
}

#[cfg(unix)]
fn check_permissions(path: &Path) -> RegistryResult<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .map_err(|error| {
            RegistryError::credentials(format!("cannot inspect `{}`: {error}", path.display()))
        })?
        .permissions()
        .mode()
        & 0o777;
    if mode & 0o077 != 0 {
        return Err(RegistryError::credentials(format!(
            "the credentials file `{}` has mode {mode:o}; run `chmod 600` before using it",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_permissions(_path: &Path) -> RegistryResult<()> {
    Ok(())
}
