//! `seal toolchain`: verify, install, pin, and inspect toolchains.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::cli::Cli;
use crate::ui;

/// `seal toolchain`.
#[derive(Debug, Args)]
pub(crate) struct ToolchainArgs {
    /// The toolchain operation.
    #[command(subcommand)]
    pub(crate) command: ToolchainCommand,
}

/// Toolchain operations.
#[derive(Debug, Subcommand)]
pub(crate) enum ToolchainCommand {
    /// List installed toolchains.
    List {
        /// Print a JSON array.
        #[arg(long)]
        json: bool,
    },
    /// Install a verified local bundle. No network access is attempted.
    Install {
        /// The bundle directory containing `seal-toolchain.toml`.
        #[arg(long, value_name = "DIR")]
        from: Option<PathBuf>,
        /// Require a purely local install (the default behavior).
        #[arg(long)]
        offline: bool,
        /// Trusted Ed25519 public key for a signed bundle, as 64 hex characters.
        #[arg(long, value_name = "HEX")]
        public_key: Option<String>,
    },
    /// Pin an installed toolchain in `seal-toolchain.toml`.
    Pin {
        /// The installed version.
        version: String,
    },
    /// Pin the latest installed toolchain.
    Update {
        /// Require a purely local update (the default behavior).
        #[arg(long)]
        offline: bool,
    },
    /// Print the pinned or embedded toolchain.
    Which {
        /// Print a JSON object.
        #[arg(long)]
        json: bool,
    },
    /// Verify one installed toolchain.
    Verify {
        /// The installed version.
        version: String,
    },
    /// Print the tree checksum used in a bundle manifest.
    Checksum {
        /// The bundle directory.
        path: PathBuf,
    },
}

#[derive(Serialize)]
struct WhichReport {
    version: String,
    path: Option<String>,
    pinned: bool,
}

pub(crate) fn run(cli: &Cli, project: &Path, args: &ToolchainArgs) -> ExitCode {
    let store = seal_toolchain::ToolchainStore::for_project(project);
    match &args.command {
        ToolchainCommand::List { json } => {
            let installed = match store.list() {
                Ok(installed) => installed,
                Err(error) => return ui::error(error.code(), &error.to_string()),
            };
            if *json {
                match serde_json::to_string_pretty(&installed) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        return ui::error(
                            "E8011",
                            &format!("cannot serialize the report: {error}"),
                        );
                    }
                }
            } else if installed.is_empty() {
                ui::status(cli, "no toolchains installed");
            } else {
                for toolchain in &installed {
                    println!(
                        "{} ({})  {}  targets: {}",
                        toolchain.manifest.version,
                        toolchain.manifest.channel,
                        toolchain.manifest.checksum,
                        toolchain.manifest.targets.join(", ")
                    );
                }
            }
            ExitCode::SUCCESS
        }
        ToolchainCommand::Install {
            from,
            offline,
            public_key,
        } => {
            let Some(from) = from else {
                let error = seal_toolchain::ToolchainError::NoOfflineBundle {
                    version: "any".to_owned(),
                };
                return ui::error(error.code(), &error.to_string());
            };
            let _ = offline;
            match store.install_from_with_key(from, public_key.as_deref()) {
                Ok(toolchain) => {
                    ui::status(
                        cli,
                        &format!(
                            "{} toolchain {} from {}",
                            if toolchain.reused {
                                "verified"
                            } else {
                                "installed"
                            },
                            toolchain.manifest.version,
                            from.display()
                        ),
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => ui::error(error.code(), &error.to_string()),
            }
        }
        ToolchainCommand::Pin { version } => match store.pin(project, version) {
            Ok(manifest) => {
                ui::status(
                    cli,
                    &format!(
                        "pinned toolchain {} in {}",
                        manifest.version,
                        project.join(seal_toolchain::PIN_NAME).display()
                    ),
                );
                ExitCode::SUCCESS
            }
            Err(error) => ui::error(error.code(), &error.to_string()),
        },
        ToolchainCommand::Update { offline } => {
            let _ = offline;
            match store.update(project) {
                Ok(manifest) => {
                    ui::status(
                        cli,
                        &format!(
                            "pinned latest installed toolchain {} in {}",
                            manifest.version,
                            project.join(seal_toolchain::PIN_NAME).display()
                        ),
                    );
                    ExitCode::SUCCESS
                }
                Err(error) => ui::error(error.code(), &error.to_string()),
            }
        }
        ToolchainCommand::Which { json } => {
            let pinned = match seal_toolchain::read_pin(project) {
                Ok(pinned) => pinned,
                Err(error) => return ui::error(error.code(), &error.to_string()),
            };
            let installed = match store.which(project) {
                Ok(installed) => installed,
                Err(error) => return ui::error(error.code(), &error.to_string()),
            };
            let report = WhichReport {
                version: pinned.as_ref().map_or_else(
                    || seal_toolchain::EMBEDDED_VERSION.to_owned(),
                    |pin| pin.version.clone(),
                ),
                path: installed.map(|toolchain| toolchain.root.display().to_string()),
                pinned: pinned.is_some(),
            };
            if *json {
                match serde_json::to_string_pretty(&report) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        return ui::error(
                            "E8011",
                            &format!("cannot serialize the report: {error}"),
                        );
                    }
                }
            } else if report.pinned {
                println!(
                    "{} at {}",
                    report.version,
                    report.path.as_deref().unwrap_or("<not installed>")
                );
            } else {
                println!(
                    "{} (embedded compiler; no project pin)",
                    seal_toolchain::EMBEDDED_VERSION
                );
            }
            ExitCode::SUCCESS
        }
        ToolchainCommand::Verify { version } => match store.verify(version) {
            Ok(manifest) => {
                ui::status(
                    cli,
                    &format!(
                        "verified toolchain {} ({})",
                        manifest.version, manifest.checksum
                    ),
                );
                ExitCode::SUCCESS
            }
            Err(error) => ui::error(error.code(), &error.to_string()),
        },
        ToolchainCommand::Checksum { path } => match seal_toolchain::tree_checksum(path) {
            Ok(checksum) => {
                println!("{checksum}");
                ExitCode::SUCCESS
            }
            Err(error) => ui::error(error.code(), &error.to_string()),
        },
    }
}
