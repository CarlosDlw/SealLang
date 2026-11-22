//! Remote endpoint resolution for build-like commands.
//!
//! Precedence is command line, environment, user configuration, then default.
//! The bearer token is read only from `SEAL_REMOTE_TOKEN`; it is never accepted
//! as a command-line value or written to the user configuration.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use seal_remote::{RemoteConfig, RemoteToken};

use crate::cli::Cli;
use crate::cli::config::load_user;
use crate::ui;

pub(crate) fn options(cli: &Cli) -> Result<Option<RemoteConfig>, ExitCode> {
    let user = load_user()?;
    let url = cli
        .remote
        .clone()
        .or_else(|| env("SEAL_REMOTE_URL"))
        .or_else(|| user.remote.url.clone());
    let execution = cli.remote_execution
        || optional_bool("SEAL_REMOTE_EXECUTION")?.unwrap_or(false)
        || user.remote.execution.unwrap_or(false);
    let require_remote = cli.require_remote
        || optional_bool("SEAL_REMOTE_REQUIRE")?.unwrap_or(false)
        || user.remote.require.unwrap_or(false);
    if cli.offline && url.is_some() {
        return Err(ui::error(
            "E8128",
            "remote access is disabled by `--offline`; unset the remote endpoint or remove `--offline`",
        ));
    }
    if (execution || require_remote) && url.is_none() {
        return Err(ui::error(
            "E8129",
            "`--remote-execution` and `--require-remote` need a remote endpoint",
        ));
    }
    let Some(url) = url.filter(|url| !url.trim().is_empty()) else {
        return Ok(None);
    };
    let instance = cli
        .remote_instance
        .clone()
        .or_else(|| env("SEAL_REMOTE_INSTANCE"))
        .or_else(|| user.remote.instance.clone())
        .unwrap_or_else(|| seal_remote::DEFAULT_INSTANCE.to_owned());
    let timeout = match cli.remote_timeout {
        Some(timeout) => timeout,
        None => match optional_u64("SEAL_REMOTE_TIMEOUT")? {
            Some(timeout) => timeout,
            None => user.remote.timeout.unwrap_or(300),
        },
    };
    let retries = match cli.remote_retries {
        Some(retries) => retries,
        None => match optional_u64("SEAL_REMOTE_RETRIES")? {
            Some(retries) => u32::try_from(retries)
                .map_err(|_| ui::error("E8120", "the remote retry limit is too large"))?,
            None => user.remote.retries.unwrap_or(2),
        },
    };
    let ca_certificate = cli
        .remote_ca_cert
        .clone()
        .or_else(|| env_path("SEAL_REMOTE_CA_CERT"))
        .or_else(|| user.remote.ca_certificate.clone());
    let tls_domain = cli
        .remote_tls_domain
        .clone()
        .or_else(|| env("SEAL_REMOTE_TLS_DOMAIN"))
        .or_else(|| user.remote.tls_domain.clone());
    let token = match env("SEAL_REMOTE_TOKEN") {
        Some(value) => Some(
            RemoteToken::new(value).map_err(|error| ui::error(error.code(), error.message()))?,
        ),
        None => None,
    };
    if timeout == 0 {
        return Err(ui::error(
            "E8120",
            "the remote timeout must be at least one second",
        ));
    }
    let config = RemoteConfig {
        endpoint: url,
        instance_name: instance,
        token,
        ca_certificate,
        tls_domain,
        connect_timeout: Duration::from_secs(10),
        timeout: Duration::from_secs(timeout),
        retries,
        max_batch_bytes: seal_remote::DEFAULT_MAX_BATCH_BYTES,
        execution,
        require_remote,
        platform: RemoteConfig::host_platform(),
    };
    config
        .validate()
        .map_err(|message| ui::error("E8120", &message))?;
    Ok(Some(config))
}

fn env(name: &str) -> Option<String> {
    std::env::var_os(name)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

fn env_path(name: &str) -> Option<PathBuf> {
    env(name).map(PathBuf::from)
}

fn optional_bool(name: &str) -> Result<Option<bool>, ExitCode> {
    let Some(value) = env(name) else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(Some(true)),
        "0" | "false" | "no" | "off" => Ok(Some(false)),
        _ => Err(ui::error(
            "E8120",
            &format!("`{name}` must be a boolean, not `{value}`"),
        )),
    }
}

fn optional_u64(name: &str) -> Result<Option<u64>, ExitCode> {
    let Some(value) = env(name) else {
        return Ok(None);
    };
    value.parse().map(Some).map_err(|_| {
        ui::error(
            "E8120",
            &format!("`{name}` must be a positive integer, not `{value}`"),
        )
    })
}
