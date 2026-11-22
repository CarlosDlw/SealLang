//! `seal-remote-server`: a small self-hosted REAPI v2 endpoint.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use seal_remote::RemoteToken;
use seal_remote_server::{RunningServer, ServerConfig};

#[derive(Debug, Parser)]
#[command(
    name = "seal-remote-server",
    version,
    about = "A reference Seal REAPI v2 cache and execution server"
)]
struct Arguments {
    /// Directory holding the persistent cache.
    #[arg(long, value_name = "PATH")]
    root: PathBuf,

    /// Address to listen on.
    #[arg(long, default_value = "127.0.0.1:8980", value_name = "ADDRESS")]
    listen: SocketAddr,

    /// REAPI instance name; empty accepts any instance.
    #[arg(long, default_value = "seal", value_name = "NAME")]
    instance: String,

    /// Environment variable holding the bearer token.
    #[arg(long, default_value = "SEAL_REMOTE_TOKEN", value_name = "NAME")]
    token_env: String,

    /// File containing the bearer token.
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,

    /// Allow serving without authentication. Only use on a trusted network.
    #[arg(long)]
    allow_anonymous: bool,

    /// Disable remote execution while keeping CAS and ActionCache enabled.
    #[arg(long)]
    no_execution: bool,

    /// Allow the portable sandbox fallback when bubblewrap is unavailable.
    #[arg(long)]
    allow_portable: bool,

    /// Directory prepended when resolving the worker program.
    #[arg(long, value_name = "PATH")]
    tool_path: Option<PathBuf>,

    /// PEM certificate for TLS.
    #[arg(long, value_name = "PATH", requires = "tls_key")]
    tls_cert: Option<PathBuf>,

    /// PEM private key for TLS.
    #[arg(long, value_name = "PATH", requires = "tls_cert")]
    tls_key: Option<PathBuf>,

    /// Maximum total bytes in one batch request.
    #[arg(long, default_value_t = seal_remote_server::DEFAULT_MAX_BATCH_BYTES)]
    max_batch_bytes: usize,

    /// Maximum bytes in one CAS blob.
    #[arg(long, default_value_t = seal_remote_server::DEFAULT_MAX_BLOB_BYTES)]
    max_blob_bytes: usize,

    /// Maximum total input bytes for one action.
    #[arg(long, default_value_t = seal_remote_server::DEFAULT_MAX_INPUT_BYTES)]
    max_input_bytes: u64,

    /// Maximum action timeout in seconds.
    #[arg(long, default_value_t = 300)]
    timeout: u64,
}

fn main() -> ExitCode {
    let arguments = Arguments::parse();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("E8120: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: &Arguments) -> Result<(), String> {
    if arguments.timeout == 0 {
        return Err("`--timeout` must be at least one second".to_owned());
    }
    let token = load_token(arguments)?;
    if token.is_none() && !arguments.allow_anonymous {
        return Err(
            "no token configured; set the token environment/file or pass `--allow-anonymous`"
                .to_owned(),
        );
    }
    if token.is_none() && !arguments.listen.ip().is_loopback() {
        eprintln!("warning: serving an unauthenticated remote endpoint on a non-loopback address");
    }
    let config = ServerConfig {
        root: arguments.root.clone(),
        instance_name: arguments.instance.clone(),
        token,
        execution: !arguments.no_execution,
        max_batch_bytes: arguments.max_batch_bytes,
        max_blob_bytes: arguments.max_blob_bytes,
        max_input_bytes: arguments.max_input_bytes,
        max_action_timeout: Duration::from_secs(arguments.timeout),
        allow_portable_sandbox: arguments.allow_portable,
        tool_path: arguments.tool_path.clone(),
        tls: arguments.tls_cert.clone().zip(arguments.tls_key.clone()),
    };
    let server =
        RunningServer::start(config, arguments.listen).map_err(|error| error.to_string())?;
    println!("listening on {}", server.endpoint());
    let (sender, receiver) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = sender.send(());
    })
    .map_err(|error| format!("cannot install the shutdown handler: {error}"))?;
    let _ = receiver.recv();
    server.shutdown();
    Ok(())
}

fn load_token(arguments: &Arguments) -> Result<Option<RemoteToken>, String> {
    let value = if let Some(path) = &arguments.token_file {
        Some(
            std::fs::read_to_string(path)
                .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?,
        )
    } else {
        std::env::var_os(&arguments.token_env).map(|value| value.to_string_lossy().into_owned())
    };
    value
        .filter(|value| !value.trim().is_empty())
        .map(|value| RemoteToken::new(value).map_err(|error| error.to_string()))
        .transpose()
}
