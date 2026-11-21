//! `seal-registry-server`, the reference registry.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use clap::Parser;
use seal_registry_server::{Server, ServerConfig};

#[derive(Debug, Parser)]
#[command(
    name = "seal-registry-server",
    version,
    about = "A Seal package registry with TUF, provenance, and a transparency log",
    long_about = "Runs the reference Seal registry. Without `--config`, the server uses \
                  local defaults and generates signing keys under `.seal-registry/`."
)]
struct Cli {
    /// The TOML configuration file.
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Override the listening address.
    #[arg(long)]
    bind: Option<String>,
    /// Override the storage directory.
    #[arg(long)]
    storage: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let mut config = match &cli.config {
        Some(path) => match ServerConfig::load(path) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("error[{}]: {}", error.code(), error.message());
                return ExitCode::from(1);
            }
        },
        None => ServerConfig::default(),
    };
    if let Some(bind) = cli.bind {
        config.bind = bind;
    }
    if let Some(storage) = cli.storage {
        config.storage = storage;
    }
    if let Err(error) = config.validate(std::path::Path::new(".")) {
        eprintln!("error[{}]: {}", error.code(), error.message());
        return ExitCode::from(1);
    }
    let server = match Server::bind(config) {
        Ok(server) => server,
        Err(error) => {
            eprintln!("error[{}]: {}", error.code(), error.message());
            return ExitCode::from(1);
        }
    };
    let address = match server.local_addr() {
        Ok(address) => address,
        Err(error) => {
            eprintln!("error[{}]: {}", error.code(), error.message());
            return ExitCode::from(1);
        }
    };
    eprintln!("seal registry listening on http://{address}/");
    match server.run(Arc::new(AtomicBool::new(false))) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error[{}]: {}", error.code(), error.message());
            ExitCode::from(1)
        }
    }
}
