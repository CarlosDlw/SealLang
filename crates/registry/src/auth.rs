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

/// Returns the credentials file used for `registry_name`.
///
/// This is the explicit manifest path, `$SEAL_CREDENTIALS`, or the user's
/// configuration directory, in that order.
pub fn credentials_path(project_root: &Path, explicit: Option<&Path>) -> Option<PathBuf> {
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

/// Writes a token for one registry, creating the file with mode `0600`.
///
/// The file is written atomically. Existing entries for other registries are
/// preserved; the token itself is never printed or formatted.
pub fn write_token(
    project_root: &Path,
    explicit: Option<&Path>,
    registry_name: &str,
    token: &Token,
) -> RegistryResult<PathBuf> {
    if registry_name.is_empty() || registry_name.chars().any(char::is_control) {
        return Err(RegistryError::credentials(
            "the registry name is empty or contains control characters",
        ));
    }
    let path = credentials_path(project_root, explicit).ok_or_else(|| {
        RegistryError::credentials(
            "no credentials file location is available; set `SEAL_CREDENTIALS` or `HOME`",
        )
    })?;
    let mut document: toml::Value = if path.is_file() {
        check_permissions(&path)?;
        let text = std::fs::read_to_string(&path).map_err(|error| {
            RegistryError::credentials(format!("cannot read `{}`: {error}", path.display()))
        })?;
        toml::from_str(&text).map_err(|error| {
            RegistryError::credentials(format!(
                "invalid credentials file `{}`: {error}",
                path.display()
            ))
        })?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };
    let table = document
        .as_table_mut()
        .ok_or_else(|| RegistryError::credentials("the credentials file is not a TOML table"))?;
    let registries = table
        .entry("registries")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| RegistryError::credentials("`registries` is not a TOML table"))?;
    let registry = registries
        .entry(registry_name.to_owned())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
        .as_table_mut()
        .ok_or_else(|| {
            RegistryError::credentials(format!("`registries.{registry_name}` is not a TOML table"))
        })?;
    registry.insert(
        "token".to_owned(),
        toml::Value::String(token.expose().to_owned()),
    );
    let text = toml::to_string_pretty(&document).map_err(|error| {
        RegistryError::credentials(format!("cannot encode the credentials: {error}"))
    })?;
    write_private(&path, text.as_bytes())?;
    Ok(path)
}

/// Removes the token for one registry, returning whether an entry was removed.
pub fn remove_token(
    project_root: &Path,
    explicit: Option<&Path>,
    registry_name: &str,
) -> RegistryResult<bool> {
    let Some(path) = credentials_path(project_root, explicit) else {
        return Ok(false);
    };
    if !path.is_file() {
        return Ok(false);
    }
    check_permissions(&path)?;
    let text = std::fs::read_to_string(&path).map_err(|error| {
        RegistryError::credentials(format!("cannot read `{}`: {error}", path.display()))
    })?;
    let mut document: toml::Value = toml::from_str(&text).map_err(|error| {
        RegistryError::credentials(format!(
            "invalid credentials file `{}`: {error}",
            path.display()
        ))
    })?;
    let Some(table) = document.as_table_mut() else {
        return Err(RegistryError::credentials(
            "the credentials file is not a TOML table",
        ));
    };
    let removed = table
        .get_mut("registries")
        .and_then(toml::Value::as_table_mut)
        .and_then(|registries| registries.remove(registry_name))
        .is_some();
    if removed {
        let text = toml::to_string_pretty(&document).map_err(|error| {
            RegistryError::credentials(format!("cannot encode the credentials: {error}"))
        })?;
        write_private(&path, text.as_bytes())?;
    }
    Ok(removed)
}

fn write_private(path: &Path, bytes: &[u8]) -> RegistryResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            RegistryError::credentials(format!("cannot create `{}`: {error}", parent.display()))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    RegistryError::credentials(format!(
                        "cannot secure `{}`: {error}",
                        parent.display()
                    ))
                },
            )?;
        }
    }
    let temporary = path.with_extension(format!("tmp{}", std::process::id()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|error| {
                RegistryError::credentials(format!(
                    "cannot create `{}`: {error}",
                    temporary.display()
                ))
            })?;
        use std::io::Write;
        file.write_all(bytes).map_err(|error| {
            RegistryError::credentials(format!("cannot write `{}`: {error}", temporary.display()))
        })?;
        file.sync_all().ok();
    }
    #[cfg(not(unix))]
    std::fs::write(&temporary, bytes).map_err(|error| {
        RegistryError::credentials(format!("cannot write `{}`: {error}", temporary.display()))
    })?;
    std::fs::rename(&temporary, path).map_err(|error| {
        RegistryError::credentials(format!("cannot replace `{}`: {error}", path.display()))
    })?;
    Ok(())
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
