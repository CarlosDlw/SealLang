//! Remote execution of the real compiler binary through the reference worker.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use seal_remote::{RemoteClient, RemoteConfig, RemoteExecutionRequest, RemoteToken};
use seal_remote_server::{RunningServer, ServerConfig};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture() -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-driver-remote-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the fixture directory is created");
    Fixture { root }
}

#[test]
fn the_worker_runs_the_real_compiler_in_its_sandbox() {
    let fixture = fixture();
    let sealc = PathBuf::from(env!("CARGO_BIN_EXE_sealc"));
    let tool_path = sealc.parent().expect("the binary has a directory");
    let server = RunningServer::start(
        ServerConfig {
            root: fixture.root.join("cache"),
            token: Some(RemoteToken::new("driver-secret").expect("the token is valid")),
            execution: true,
            allow_portable_sandbox: true,
            tool_path: Some(tool_path.to_path_buf()),
            ..ServerConfig::default()
        },
        "127.0.0.1:0".parse().expect("the address parses"),
    )
    .expect("the server starts");
    let client = RemoteClient::connect(RemoteConfig {
        endpoint: server.endpoint().to_owned(),
        token: Some(RemoteToken::new("driver-secret").expect("the token is valid")),
        timeout: Duration::from_secs(30),
        execution: true,
        platform: Vec::new(),
        ..RemoteConfig::default()
    })
    .expect("the client connects");
    let result = client
        .execute(&RemoteExecutionRequest {
            arguments: vec!["sealc".to_owned(), "--version".to_owned()],
            environment: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            platform: Vec::new(),
            timeout: Duration::from_secs(30),
        })
        .expect("the compiler runs remotely");
    assert_eq!(result.exit_code, 0);
    assert!(
        String::from_utf8_lossy(&result.stdout).contains("sealc"),
        "stdout: {}",
        String::from_utf8_lossy(&result.stdout)
    );
}
