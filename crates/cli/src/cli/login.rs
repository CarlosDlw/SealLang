//! `seal login`: validate and store a registry token.

use std::path::Path;
use std::process::ExitCode;

use seal_manifest::RegistryManifest;
use seal_registry::auth::Token;
use seal_registry::client::RegistryClient;
use seal_registry::config::RegistryConfig;

use crate::cli::{Cli, LoginArgs};
use crate::ui;

pub(crate) fn run(cli: &Cli, root: &Path, args: &LoginArgs) -> ExitCode {
    let project = seal_project::Project::load(root).ok();
    let project_root = project
        .as_ref()
        .map(|project| project.root.as_path())
        .unwrap_or(root);
    let mut manifest = project
        .as_ref()
        .and_then(|project| project.registry.clone())
        .unwrap_or_default();
    if let Some(url) = &args.registry {
        manifest.url = Some(url.clone());
        manifest.allow_insecure = args.allow_insecure;
    }
    if manifest.url.is_none() {
        return ui::error(
            "E8020",
            "no registry is configured; pass `--registry <URL>` or add `[registry]`",
        );
    }
    let config = match RegistryConfig::load(project_root, Some(&manifest), false) {
        Ok(Some(mut config)) => {
            config.token = None;
            config
        }
        Ok(None) => {
            return ui::error("E8020", "no registry is configured");
        }
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let token = match if args.token_stdin {
        read_token()
    } else if let Ok(token) = std::env::var("SEAL_REGISTRY_TOKEN") {
        Ok(token)
    } else {
        read_token()
    } {
        Ok(token) => token,
        Err(code) => return code,
    };
    let token = match Token::new(token) {
        Ok(token) => token,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    let mut authorized = config.clone();
    authorized.token = Some(token.clone());
    let client = match RegistryClient::new(&authorized) {
        Ok(client) => client,
        Err(error) => return ui::error(error.code(), error.message()),
    };
    match client.post_json(&authorized, "api/v1/whoami", b"{}") {
        Ok(response) if response.status == 200 => {
            let identity: serde_json::Value =
                serde_json::from_slice(&response.body).unwrap_or_default();
            let name = args
                .name
                .clone()
                .or_else(|| manifest.name.clone())
                .unwrap_or_else(|| "default".to_owned());
            match seal_registry::auth::write_token(
                project_root,
                manifest.credentials.as_deref(),
                &name,
                &token,
            ) {
                Ok(path) => {
                    ui::status(
                        cli,
                        &format!(
                            "logged in as {} to {}",
                            identity["name"].as_str().unwrap_or("registry user"),
                            authorized.url
                        ),
                    );
                    ui::detail(cli, &format!("  credentials written to {}", path.display()));
                    ExitCode::SUCCESS
                }
                Err(error) => ui::error(error.code(), error.message()),
            }
        }
        Ok(response) => {
            let message = serde_json::from_slice::<serde_json::Value>(&response.body)
                .ok()
                .and_then(|body| body["error"]["message"].as_str().map(ToOwned::to_owned))
                .unwrap_or_else(|| format!("the registry returned HTTP {}", response.status));
            ui::error("E8084", &message)
        }
        Err(error) => ui::error(error.code(), error.message()),
    }
}

fn read_token() -> Result<String, ExitCode> {
    use std::io::Read;
    let mut text = String::new();
    std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| {
            ui::error(
                "E8084",
                &format!("cannot read the token from stdin: {error}"),
            )
        })?;
    let token = text.trim();
    if token.is_empty() {
        return Err(ui::error(
            "E8084",
            "stdin carried no token; pipe it, as in `printf '%s' \"$TOKEN\" | seal login`",
        ));
    }
    Ok(token.to_owned())
}

/// Returns the registry name used by `login`/`logout` for a manifest.
pub(crate) fn registry_name(manifest: Option<&RegistryManifest>) -> String {
    manifest
        .and_then(|manifest| manifest.name.clone())
        .unwrap_or_else(|| "default".to_owned())
}
