//! `seal target`: inspect and install TOML target specifications.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand};
use seal_target::TargetSpec;
use serde::Serialize;

use crate::cli::Cli;
use crate::ui;

/// `seal target`.
#[derive(Debug, Args)]
pub(crate) struct TargetArgs {
    /// The target operation.
    #[command(subcommand)]
    pub(crate) command: TargetCommand,
}

/// Target operations.
#[derive(Debug, Subcommand)]
pub(crate) enum TargetCommand {
    /// List the host, built-in, and project targets.
    List {
        /// Print a JSON array.
        #[arg(long)]
        json: bool,
    },
    /// Validate a TOML specification and install it in `seal/targets`.
    Add {
        /// The specification file.
        spec: PathBuf,
    },
    /// Print the resolved specification for a triple.
    Info {
        /// The target triple.
        triple: String,
        /// Print JSON instead of TOML.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Serialize)]
struct Info {
    source: String,
    triple: String,
    pointer_width: u8,
    endianness: String,
    data_layout: Option<String>,
    runtime: String,
    panic: String,
    linker: Option<String>,
    sysroot: Option<String>,
    linker_script: Option<String>,
    c_char_signed: bool,
    c_long_width: u16,
    c_wchar_width: u16,
    c_wchar_signed: bool,
}

fn info(spec: &TargetSpec, source: &str) -> Info {
    let abi = spec.c_abi();
    Info {
        source: source.to_owned(),
        triple: spec.triple().as_str().to_owned(),
        pointer_width: spec.pointer_width(),
        endianness: match spec.endianness() {
            seal_target::Endianness::Little => "little",
            seal_target::Endianness::Big => "big",
        }
        .to_owned(),
        data_layout: spec.data_layout().map(ToOwned::to_owned),
        runtime: spec.runtime().as_str().to_owned(),
        panic: spec.panic().as_str().to_owned(),
        linker: spec.linker().map(ToOwned::to_owned),
        sysroot: spec.sysroot().map(|path| path.display().to_string()),
        linker_script: spec.linker_script().map(|path| path.display().to_string()),
        c_char_signed: abi.char_signed(),
        c_long_width: abi.long_width(),
        c_wchar_width: abi.wchar_width(),
        c_wchar_signed: abi.wchar_signed(),
    }
}

pub(crate) fn run(_cli: &Cli, project: &Path, args: &TargetArgs) -> ExitCode {
    match &args.command {
        TargetCommand::List { json } => {
            let host = seal_target::host_triple();
            let mut entries = vec![info(&host.spec(), "host")];
            let directory = project.join("seal").join("targets");
            if directory.is_dir() {
                let mut files = match std::fs::read_dir(&directory) {
                    Ok(entries) => entries
                        .filter_map(Result::ok)
                        .map(|entry| entry.path())
                        .filter(|path| {
                            path.extension().and_then(|value| value.to_str()) == Some("toml")
                        })
                        .collect::<Vec<_>>(),
                    Err(error) => {
                        return ui::error(
                            "E8060",
                            &format!("cannot read `{}`: {error}", directory.display()),
                        );
                    }
                };
                files.sort();
                for path in files {
                    let text = match std::fs::read_to_string(&path) {
                        Ok(text) => text,
                        Err(error) => {
                            return ui::error(
                                "E8060",
                                &format!("cannot read `{}`: {error}", path.display()),
                            );
                        }
                    };
                    let spec = match TargetSpec::parse_toml(&text) {
                        Ok(spec) => spec,
                        Err(error) => {
                            return ui::error("E8060", &format!("{}: {error}", path.display()));
                        }
                    };
                    entries.push(info(
                        &spec,
                        &path
                            .strip_prefix(project)
                            .unwrap_or(&path)
                            .display()
                            .to_string(),
                    ));
                }
            }
            if *json {
                match serde_json::to_string_pretty(&entries) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        return ui::error(
                            "E8011",
                            &format!("cannot serialize the report: {error}"),
                        );
                    }
                }
            } else {
                for entry in &entries {
                    println!(
                        "{:<32} {:<8} runtime={:<7} panic={:<5} {}",
                        entry.triple,
                        entry.source,
                        entry.runtime,
                        entry.panic,
                        entry.linker.as_deref().unwrap_or("default linker")
                    );
                }
            }
            ExitCode::SUCCESS
        }
        TargetCommand::Add { spec } => {
            let text = match std::fs::read_to_string(spec) {
                Ok(text) => text,
                Err(error) => {
                    return ui::error(
                        "E8060",
                        &format!("cannot read `{}`: {error}", spec.display()),
                    );
                }
            };
            let spec = match TargetSpec::parse_toml(&text) {
                Ok(spec) => spec,
                Err(error) => return ui::error("E8060", &error.to_string()),
            };
            let directory = project.join("seal").join("targets");
            if let Err(error) = std::fs::create_dir_all(&directory) {
                return ui::error(
                    "E8011",
                    &format!("cannot create `{}`: {error}", directory.display()),
                );
            }
            let destination = directory.join(format!("{}.toml", spec.triple()));
            if destination.exists() {
                return ui::error(
                    "E8060",
                    &format!("the target `{}` is already installed", spec.triple()),
                );
            }
            let temporary = destination.with_extension(format!("toml.{}", std::process::id()));
            if let Err(error) = std::fs::write(&temporary, text) {
                return ui::error(
                    "E8011",
                    &format!("cannot write `{}`: {error}", temporary.display()),
                );
            }
            if let Err(error) = std::fs::rename(&temporary, &destination) {
                let _ = std::fs::remove_file(&temporary);
                return ui::error(
                    "E8011",
                    &format!("cannot install `{}`: {error}", destination.display()),
                );
            }
            println!("installed target {}", spec.triple());
            ExitCode::SUCCESS
        }
        TargetCommand::Info { triple, json } => {
            let parsed = match seal_target::TargetTriple::parse(triple) {
                Ok(parsed) => parsed,
                Err(error) => return ui::error("E8060", &error.to_string()),
            };
            let path = project
                .join("seal")
                .join("targets")
                .join(format!("{}.toml", parsed.as_str()));
            let (spec, source) = if path.is_file() {
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(error) => {
                        return ui::error(
                            "E8060",
                            &format!("cannot read `{}`: {error}", path.display()),
                        );
                    }
                };
                match TargetSpec::parse_toml(&text) {
                    Ok(spec) => (spec, path.display().to_string()),
                    Err(error) => {
                        return ui::error("E8060", &format!("{}: {error}", path.display()));
                    }
                }
            } else {
                (parsed.spec(), "built-in".to_owned())
            };
            if spec.triple() != &parsed {
                return ui::error(
                    "E8060",
                    &format!(
                        "the specification names `{}`, not `{triple}`",
                        spec.triple()
                    ),
                );
            }
            if *json {
                match serde_json::to_string_pretty(&info(&spec, &source)) {
                    Ok(text) => println!("{text}"),
                    Err(error) => {
                        return ui::error(
                            "E8011",
                            &format!("cannot serialize the report: {error}"),
                        );
                    }
                }
            } else {
                match spec.to_toml() {
                    Ok(text) => print!("{text}"),
                    Err(error) => return ui::error("E8060", &error.to_string()),
                }
            }
            ExitCode::SUCCESS
        }
    }
}
