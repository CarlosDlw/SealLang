//! Loading or creating the local Ed25519 publication key.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::ui;

/// Returns the signing seed from an explicit file, the environment, or the
/// per-user key file, creating the latter with mode `0600` when absent.
pub(crate) fn load(explicit: Option<&Path>) -> Result<[u8; 32], ExitCode> {
    if let Some(path) = explicit {
        return read_seed(path);
    }
    if let Some(value) = std::env::var_os("SEAL_SIGNING_KEY") {
        let value = value
            .into_string()
            .map_err(|_| ui::error("E8096", "`SEAL_SIGNING_KEY` is not valid UTF-8"))?;
        return parse(value.trim());
    }
    let path = default_path().ok_or_else(|| {
        ui::error(
            "E8096",
            "no signing key is available; pass `--signing-key` or set `SEAL_SIGNING_KEY`",
        )
    })?;
    if path.is_file() {
        return read_seed(&path);
    }
    let seed = random_seed()?;
    write_private(&path, &hex::encode(seed))?;
    Ok(seed)
}

fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(base.join("seal").join("signing.key"))
}

fn read_seed(path: &Path) -> Result<[u8; 32], ExitCode> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        ui::error(
            "E8096",
            &format!("cannot read `{}`: {error}", path.display()),
        )
    })?;
    parse(text.trim())
}

fn parse(text: &str) -> Result<[u8; 32], ExitCode> {
    if text.len() != 64 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ui::error(
            "E8096",
            "a signing key must be 64 hexadecimal characters",
        ));
    }
    let bytes = hex::decode(text)
        .map_err(|error| ui::error("E8096", &format!("invalid signing key: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| ui::error("E8096", "the signing key must be 32 bytes"))
}

fn random_seed() -> Result<[u8; 32], ExitCode> {
    let mut seed = [0u8; 32];
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut file = std::fs::File::open("/dev/urandom").map_err(|error| {
            ui::error("E8096", &format!("cannot open the random source: {error}"))
        })?;
        file.read_exact(&mut seed).map_err(|error| {
            ui::error("E8096", &format!("cannot read the random source: {error}"))
        })?;
        Ok(seed)
    }
    #[cfg(not(unix))]
    {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
                .to_be_bytes(),
        );
        hasher.update(std::process::id().to_be_bytes());
        seed.copy_from_slice(&hasher.finalize());
        Ok(seed)
    }
}

fn write_private(path: &Path, value: &str) -> Result<(), ExitCode> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            ui::error(
                "E8096",
                &format!("cannot create `{}`: {error}", parent.display()),
            )
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    ui::error(
                        "E8096",
                        &format!("cannot secure `{}`: {error}", parent.display()),
                    )
                },
            )?;
        }
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| {
                ui::error(
                    "E8096",
                    &format!("cannot create `{}`: {error}", path.display()),
                )
            })?;
        writeln!(file, "{value}").map_err(|error| {
            ui::error(
                "E8096",
                &format!("cannot write `{}`: {error}", path.display()),
            )
        })?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, format!("{value}\n")).map_err(|error| {
            ui::error(
                "E8096",
                &format!("cannot write `{}`: {error}", path.display()),
            )
        })?;
    }
    Ok(())
}
