//! `seal config`: resolve one value per key and show its origin.
//!
//! Precedence is fixed and identical for every key:
//! command line, environment, project manifest, user config, then the built-in
//! default. Configuration never changes how a command loads the project; it
//! only explains the layers a value can come from.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use seal_project::Project;
use serde::Deserialize;

use crate::cli::{Cli, ColorChoice, ConfigArgs, MessageFormat};
use crate::ui;

/// Where a resolved value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Cli,
    Environment,
    Project,
    User,
    Default,
}

impl Origin {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Environment => "environment",
            Self::Project => "project",
            Self::User => "user",
            Self::Default => "default",
        }
    }
}

/// One resolved configuration key.
struct Entry {
    key: &'static str,
    value: Option<String>,
    origin: Origin,
}

/// The `$SEAL_CONFIG` or XDG user configuration.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub(crate) struct UserConfig {
    target: Option<String>,
    profile: Option<String>,
    jobs: Option<usize>,
    offline: Option<bool>,
    color: Option<String>,
    message_format: Option<String>,
    cache_dir: Option<PathBuf>,
    registry: UserRegistry,
    pub(crate) remote: UserRemote,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
struct UserRegistry {
    url: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case", default)]
pub(crate) struct UserRemote {
    pub(crate) url: Option<String>,
    pub(crate) execution: Option<bool>,
    pub(crate) require: Option<bool>,
    pub(crate) instance: Option<String>,
    pub(crate) timeout: Option<u64>,
    pub(crate) retries: Option<u32>,
    #[serde(rename = "ca-cert")]
    pub(crate) ca_certificate: Option<PathBuf>,
    pub(crate) tls_domain: Option<String>,
}

pub(crate) fn run(cli: &Cli, root: &Path, args: &ConfigArgs) -> ExitCode {
    let project = match crate::cli::load(cli, root) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let user = match load_user() {
        Ok(user) => user,
        Err(code) => return code,
    };
    let entries = resolve(cli, &project, &user, args);
    let selected: Vec<&Entry> = match &args.key {
        Some(key) => {
            let found: Vec<&Entry> = entries.iter().filter(|entry| entry.key == key).collect();
            if found.is_empty() {
                return ui::error("E8110", &format!("unknown configuration key `{key}`"));
            }
            found
        }
        None => entries.iter().collect(),
    };
    if args.json {
        let value: Vec<serde_json::Value> = selected
            .iter()
            .map(|entry| {
                serde_json::json!({
                    "key": entry.key,
                    "value": entry.value,
                    "origin": entry.origin.as_str(),
                })
            })
            .collect();
        match serde_json::to_string_pretty(&value) {
            Ok(text) => println!("{text}"),
            Err(error) => {
                return ui::error(
                    "E8108",
                    &format!("cannot serialize the configuration: {error}"),
                );
            }
        }
        return ExitCode::SUCCESS;
    }
    for entry in selected {
        if args.origin {
            if args.key.is_some() {
                println!("{}", entry.origin.as_str());
            } else {
                println!("{:<20} {}", entry.key, entry.origin.as_str());
            }
            continue;
        }
        let value = entry.value.as_deref().unwrap_or("(unset)");
        if args.key.is_some() {
            println!("{value}");
        } else {
            println!("{:<20} {value:<32} ({})", entry.key, entry.origin.as_str());
        }
    }
    ExitCode::SUCCESS
}

fn resolve(cli: &Cli, project: &Project, user: &UserConfig, args: &ConfigArgs) -> Vec<Entry> {
    let host = seal_target::host_triple().as_str().to_owned();
    let registry = project
        .registry
        .as_ref()
        .and_then(|value| value.url.clone());
    let project_offline = project.registry.as_ref().map(|value| value.offline);
    let default_jobs = std::thread::available_parallelism()
        .map(|value| value.get())
        .unwrap_or(1)
        .to_string();
    vec![
        pick(
            "target",
            args.target.clone(),
            env("SEAL_TARGET"),
            None,
            user.target.clone(),
            Some(host),
        ),
        pick(
            "profile",
            args.profile.clone(),
            env("SEAL_PROFILE"),
            None,
            user.profile.clone(),
            Some("dev".to_owned()),
        ),
        pick(
            "jobs",
            args.jobs.map(|jobs| jobs.to_string()),
            env("SEAL_JOBS"),
            None,
            user.jobs.map(|jobs| jobs.to_string()),
            Some(default_jobs),
        ),
        pick(
            "offline",
            cli.offline.then(|| "true".to_owned()),
            env("SEAL_OFFLINE"),
            project_offline.map(|offline| offline.to_string()),
            user.offline.map(|offline| offline.to_string()),
            Some("false".to_owned()),
        ),
        pick(
            "registry.url",
            None,
            env("SEAL_REGISTRY_URL"),
            registry,
            user.registry.url.clone(),
            None,
        ),
        pick(
            "cache.dir",
            None,
            env("SEAL_CACHE_DIR"),
            None,
            user.cache_dir
                .as_ref()
                .map(|path| path.display().to_string()),
            Some(project.root.join("seal/cache").display().to_string()),
        ),
        pick(
            "color",
            // `auto` is clap's default, so only a non-default value proves the
            // flag was written on the command line.
            (cli.color != ColorChoice::Auto).then(|| {
                match cli.color {
                    ColorChoice::Auto => "auto",
                    ColorChoice::Always => "always",
                    ColorChoice::Never => "never",
                }
                .to_owned()
            }),
            env("SEAL_COLOR"),
            None,
            user.color.clone(),
            Some("auto".to_owned()),
        ),
        pick(
            "message-format",
            (cli.message_format != MessageFormat::Human).then_some("json".to_owned()),
            env("SEAL_MESSAGE_FORMAT"),
            None,
            user.message_format.clone(),
            Some("human".to_owned()),
        ),
        pick(
            "remote.url",
            cli.remote.clone(),
            env("SEAL_REMOTE_URL"),
            None,
            user.remote.url.clone(),
            None,
        ),
        pick(
            "remote.execution",
            cli.remote_execution.then(|| "true".to_owned()),
            env("SEAL_REMOTE_EXECUTION"),
            None,
            user.remote.execution.map(|value| value.to_string()),
            Some("false".to_owned()),
        ),
        pick(
            "remote.require",
            cli.require_remote.then(|| "true".to_owned()),
            env("SEAL_REMOTE_REQUIRE"),
            None,
            user.remote.require.map(|value| value.to_string()),
            Some("false".to_owned()),
        ),
        pick(
            "remote.instance",
            cli.remote_instance.clone(),
            env("SEAL_REMOTE_INSTANCE"),
            None,
            user.remote.instance.clone(),
            Some(seal_remote::DEFAULT_INSTANCE.to_owned()),
        ),
        pick(
            "remote.timeout",
            cli.remote_timeout.map(|value| value.to_string()),
            env("SEAL_REMOTE_TIMEOUT"),
            None,
            user.remote.timeout.map(|value| value.to_string()),
            Some("300".to_owned()),
        ),
        pick(
            "remote.retries",
            cli.remote_retries.map(|value| value.to_string()),
            env("SEAL_REMOTE_RETRIES"),
            None,
            user.remote.retries.map(|value| value.to_string()),
            Some("2".to_owned()),
        ),
        pick(
            "remote.ca-cert",
            cli.remote_ca_cert
                .as_ref()
                .map(|path| path.display().to_string()),
            env("SEAL_REMOTE_CA_CERT"),
            None,
            user.remote
                .ca_certificate
                .as_ref()
                .map(|path| path.display().to_string()),
            None,
        ),
        pick(
            "remote.tls-domain",
            cli.remote_tls_domain.clone(),
            env("SEAL_REMOTE_TLS_DOMAIN"),
            None,
            user.remote.tls_domain.clone(),
            None,
        ),
    ]
}

fn pick(
    key: &'static str,
    cli: Option<String>,
    environment: Option<String>,
    project: Option<String>,
    user: Option<String>,
    default: Option<String>,
) -> Entry {
    let layers = [
        (cli, Origin::Cli),
        (environment, Origin::Environment),
        (project, Origin::Project),
        (user, Origin::User),
        (default, Origin::Default),
    ];
    for (value, origin) in layers {
        if let Some(value) = value {
            return Entry {
                key,
                value: Some(value),
                origin,
            };
        }
    }
    Entry {
        key,
        value: None,
        origin: Origin::Default,
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var_os(name)
        .map(|value| value.to_string_lossy().into_owned())
        .filter(|value| !value.is_empty())
}

pub(crate) fn load_user() -> Result<UserConfig, ExitCode> {
    let Some(path) = user_path() else {
        return Ok(UserConfig::default());
    };
    if !path.is_file() {
        return Ok(UserConfig::default());
    }
    let text = std::fs::read_to_string(&path).map_err(|error| {
        ui::error(
            "E8110",
            &format!(
                "cannot read the user configuration `{}`: {error}",
                path.display()
            ),
        )
    })?;
    toml::from_str(&text).map_err(|error| {
        ui::error(
            "E8110",
            &format!(
                "the user configuration `{}` is invalid: {error}",
                path.display()
            ),
        )
    })
}

pub(crate) fn user_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SEAL_CONFIG") {
        return Some(PathBuf::from(path));
    }
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(config).join("seal/config.toml"));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/seal/config.toml"))
}

#[cfg(test)]
mod tests {
    use super::UserConfig;

    #[test]
    fn the_user_remote_table_uses_the_documented_ca_cert_key() {
        let user: UserConfig =
            toml::from_str("[remote]\nca-cert = \"/tmp/ca.pem\"\ntls-domain = \"cache.example\"\n")
                .expect("the remote table is valid");
        assert_eq!(
            user.remote.ca_certificate.as_deref(),
            Some(std::path::Path::new("/tmp/ca.pem"))
        );
        assert_eq!(user.remote.tls_domain.as_deref(), Some("cache.example"));
    }
}
