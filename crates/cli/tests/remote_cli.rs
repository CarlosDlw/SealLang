//! End-to-end remote cache coverage through the `seal` binary.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use seal_remote::RemoteToken;
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

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "seal-remote-cli-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    Fixture { root }
}

fn seal() -> Command {
    Command::new(env!("CARGO_BIN_EXE_seal"))
}

fn output(command: &mut Command) -> Output {
    command.output().expect("the seal binary runs")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "status: {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn create_project(fixture: &Fixture) {
    let created = output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    );
    assert_success(&created);
}

fn artifact(root: &std::path::Path) -> Vec<u8> {
    let entry = std::fs::read_dir(root.join("seal/target/dev"))
        .expect("the target directory exists")
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_none())
        .expect("an executable artifact exists");
    std::fs::read(entry.path()).expect("the artifact is readable")
}

fn build(fixture: &Fixture, endpoint: &str, require: bool) -> Output {
    let mut command = seal();
    command
        .arg("build")
        .arg("--project")
        .arg(&fixture.root)
        .arg("--remote")
        .arg(endpoint)
        .arg("--remote-timeout")
        .arg("30")
        .env("SEAL_REMOTE_TOKEN", "integration-secret");
    if require {
        command.arg("--require-remote");
    }
    output(&mut command)
}

#[test]
fn a_remote_hit_restores_a_clean_build_without_local_cache() {
    let fixture = fixture("hit");
    create_project(&fixture);
    let server = RunningServer::start(
        ServerConfig {
            root: fixture.root.join("remote-cache"),
            token: Some(RemoteToken::new("integration-secret").expect("the token is valid")),
            execution: false,
            ..ServerConfig::default()
        },
        "127.0.0.1:0".parse().expect("the address parses"),
    )
    .expect("the server starts");

    let first = build(&fixture, server.endpoint(), false);
    assert_success(&first);
    for directory in ["seal/target", "seal/cache"] {
        std::fs::remove_dir_all(fixture.root.join(directory))
            .expect("the local build state exists");
    }

    let second = build(&fixture, server.endpoint(), false);
    assert_success(&second);
    let stdout = String::from_utf8_lossy(&second.stdout);
    assert!(
        stdout.contains("Remote cached"),
        "the second build did not report a remote hit:\n{stdout}"
    );
    assert!(fixture.root.join("seal/target/dev").is_dir());
}

#[test]
fn local_and_remote_cache_builds_produce_identical_artifacts() {
    let fixture = fixture("bytes");
    create_project(&fixture);
    let local = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&local);
    let local_bytes = artifact(&fixture.root);
    for directory in ["seal/target", "seal/cache"] {
        std::fs::remove_dir_all(fixture.root.join(directory))
            .expect("the local build state exists");
    }
    let server = RunningServer::start(
        ServerConfig {
            root: fixture.root.join("remote-cache"),
            token: Some(RemoteToken::new("integration-secret").expect("the token is valid")),
            execution: false,
            ..ServerConfig::default()
        },
        "127.0.0.1:0".parse().expect("the address parses"),
    )
    .expect("the server starts");
    let remote = build(&fixture, server.endpoint(), false);
    assert_success(&remote);
    assert_eq!(artifact(&fixture.root), local_bytes);
}

#[test]
fn a_remote_failure_falls_back_or_fails_when_required() {
    let fixture = fixture("fallback");
    create_project(&fixture);
    let unreachable = "http://127.0.0.1:9";

    let fallback = build(&fixture, unreachable, false);
    assert_success(&fallback);
    assert!(
        String::from_utf8_lossy(&fallback.stderr).contains("remote access disabled"),
        "stderr: {}",
        String::from_utf8_lossy(&fallback.stderr)
    );

    for directory in ["seal/target", "seal/cache"] {
        std::fs::remove_dir_all(fixture.root.join(directory))
            .expect("the local build state exists");
    }
    let required = build(&fixture, unreachable, true);
    assert!(!required.status.success());
    assert!(
        String::from_utf8_lossy(&required.stderr).contains("E8121"),
        "stderr: {}",
        String::from_utf8_lossy(&required.stderr)
    );
}

#[test]
fn offline_mode_rejects_a_remote_endpoint() {
    let fixture = fixture("offline");
    create_project(&fixture);
    let result = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--offline")
            .arg("--remote")
            .arg("http://127.0.0.1:9"),
    );
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("E8128"),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}
