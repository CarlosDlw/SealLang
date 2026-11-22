//! Integration coverage for the reference REAPI server and client.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use seal_cas::{ActionInput, ActionRecord, Cas, Digest};
use seal_remote::{
    CacheAction, RemoteCache, RemoteClient, RemoteConfig, RemoteExecutionRequest, RemoteOutput,
    RemoteToken,
};
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
        "seal-remote-test-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    Fixture { root }
}

fn server(root: &Path, execution: bool) -> RunningServer {
    let config = ServerConfig {
        root: root.to_path_buf(),
        token: Some(RemoteToken::new("integration-secret").expect("the token is valid")),
        execution,
        allow_portable_sandbox: true,
        ..ServerConfig::default()
    };
    RunningServer::start(config, "127.0.0.1:0".parse().expect("the address parses"))
        .expect("the server starts")
}

fn client(endpoint: &str, execution: bool) -> std::sync::Arc<RemoteClient> {
    RemoteClient::connect(RemoteConfig {
        endpoint: endpoint.to_owned(),
        token: Some(RemoteToken::new("integration-secret").expect("the token is valid")),
        timeout: Duration::from_secs(30),
        execution,
        platform: Vec::new(),
        ..RemoteConfig::default()
    })
    .expect("the client connects")
}

#[test]
fn blobs_round_trip_and_missing_lookups_are_accurate() {
    let fixture = fixture("blobs");
    let server = server(&fixture.root, false);
    let client = client(server.endpoint(), false);
    client.capabilities().expect("capabilities are available");

    let bytes = vec![0x42; 32 * 1024];
    let digest = Digest::of_bytes(&bytes);
    assert!(
        client
            .find_missing(&[(digest, bytes.len() as u64)])
            .expect("the CAS is reachable")
            .contains(&digest)
    );
    client
        .upload_blob(digest, &bytes)
        .expect("the blob uploads");
    assert!(
        client
            .find_missing(&[(digest, bytes.len() as u64)])
            .expect("the CAS is reachable")
            .is_empty()
    );
    assert_eq!(
        client
            .download_blob(digest, bytes.len() as u64)
            .expect("the blob downloads"),
        bytes
    );
}

#[test]
fn large_blobs_round_trip_through_bytestream() {
    let fixture = fixture("bytestream");
    let server = server(&fixture.root, false);
    let client = client(server.endpoint(), false);
    let bytes = vec![0x5a; 5 * 1024 * 1024];
    let digest = Digest::of_bytes(&bytes);
    client
        .upload_blob(digest, &bytes)
        .expect("the large blob uploads");
    assert_eq!(
        client
            .download_blob(digest, bytes.len() as u64)
            .expect("the large blob downloads"),
        bytes
    );
}

#[test]
fn action_results_are_materialized_with_exact_outputs() {
    let fixture = fixture("actions");
    let server = server(&fixture.root, false);
    let client = client(server.endpoint(), false);
    let local = Cas::new(fixture.root.join("local")).expect("the local CAS opens");
    let bytes = b"object bytes".to_vec();
    let digest = local.put_bytes(&bytes).expect("the output is stored");
    let record = ActionRecord::new(
        "codegen-cgu",
        vec![ActionInput {
            path: "mono".to_owned(),
            digest: Digest::of_bytes(b"input"),
        }],
        vec![seal_cas::ActionOutput {
            name: "object".to_owned(),
            digest,
            mode: 0o644,
        }],
    )
    .with_command(vec!["sealc".to_owned(), "codegen".to_owned()]);
    let action = CacheAction {
        key: Digest::of_bytes(b"action key"),
        kind: "codegen-cgu".to_owned(),
        command: record.command.clone(),
        inputs: record.inputs.clone(),
        environment: BTreeMap::new(),
        outputs: vec!["object".to_owned()],
    };
    let cache = RemoteCache::new(std::sync::Arc::clone(&client), local.clone());
    cache.store(&action, &record).expect("the result is stored");
    cache
        .store(&action, &record)
        .expect("storing the same result is idempotent");
    let materialized = Cas::new(fixture.root.join("materialized")).expect("the CAS opens");
    let remote = RemoteCache::new(std::sync::Arc::clone(&client), materialized.clone());
    let hit = remote
        .lookup(&action)
        .expect("the lookup succeeds")
        .expect("the action is cached");
    assert_eq!(hit.outputs.len(), 1);
    assert_eq!(hit.outputs[0].name, "object");
    assert_eq!(
        materialized.get(digest).expect("the output is present"),
        bytes
    );
}

#[test]
fn execution_returns_verified_files_and_logs() {
    let fixture = fixture("execute");
    let server = server(&fixture.root, true);
    let client = client(server.endpoint(), true);
    let request = RemoteExecutionRequest {
        arguments: vec![
            "sh".to_owned(),
            "-c".to_owned(),
            "printf 'hello' > out; printf 'stdout'; printf 'stderr' >&2".to_owned(),
        ],
        environment: Vec::new(),
        inputs: Vec::new(),
        outputs: vec![RemoteOutput {
            path: "out".to_owned(),
            name: "object".to_owned(),
            executable: false,
        }],
        platform: Vec::new(),
        timeout: Duration::from_secs(30),
    };
    let result = client.execute(&request).expect("the action executes");
    assert_eq!(result.exit_code, 0);
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].name, "object");
    assert_eq!(result.files[0].bytes, b"hello");
    assert_eq!(result.stdout, b"stdout");
    assert_eq!(result.stderr, b"stderr");
    let cached = client
        .execute(&request)
        .expect("the cached action executes");
    assert_eq!(cached.files[0].bytes, b"hello");
    assert_eq!(cached.stdout, b"stdout");
}

#[test]
fn execution_reports_a_nonzero_exit_and_stderr() {
    let fixture = fixture("exit");
    let server = server(&fixture.root, true);
    let client = client(server.endpoint(), true);
    let result = client
        .execute(&RemoteExecutionRequest {
            arguments: vec![
                "sh".to_owned(),
                "-c".to_owned(),
                "printf 'failed' >&2; exit 7".to_owned(),
            ],
            environment: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            platform: Vec::new(),
            timeout: Duration::from_secs(30),
        })
        .expect("the worker returns the exit status");
    assert_eq!(result.exit_code, 7);
    assert_eq!(result.stderr, b"failed");
}

#[test]
fn a_missing_output_is_an_action_result_failure() {
    let fixture = fixture("missing-output");
    let server = server(&fixture.root, true);
    let client = client(server.endpoint(), true);
    let error = client
        .execute(&RemoteExecutionRequest {
            arguments: vec!["sh".to_owned(), "-c".to_owned(), "exit 0".to_owned()],
            environment: Vec::new(),
            inputs: Vec::new(),
            outputs: vec![RemoteOutput {
                path: "out".to_owned(),
                name: "object".to_owned(),
                executable: false,
            }],
            platform: Vec::new(),
            timeout: Duration::from_secs(30),
        })
        .expect_err("a missing output is rejected");
    assert_eq!(error.code(), "E8124", "{error:?}");
}

#[test]
fn an_action_deadline_is_a_timeout() {
    let fixture = fixture("timeout");
    let server = server(&fixture.root, true);
    let client = client(server.endpoint(), true);
    let error = client
        .execute(&RemoteExecutionRequest {
            arguments: vec!["sh".to_owned(), "-c".to_owned(), "sleep 2".to_owned()],
            environment: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            platform: Vec::new(),
            timeout: Duration::from_millis(100),
        })
        .expect_err("the action exceeds its deadline");
    assert_eq!(error.code(), "E8131");
}

#[test]
fn authentication_failures_are_typed() {
    let fixture = fixture("auth");
    let server = server(&fixture.root, false);
    let anonymous = RemoteClient::connect(RemoteConfig {
        endpoint: server.endpoint().to_owned(),
        token: None,
        execution: false,
        ..RemoteConfig::default()
    })
    .expect("the channel opens");
    let error = anonymous
        .capabilities()
        .expect_err("the endpoint requires a token");
    assert_eq!(error.code(), "E8123");
}

#[test]
fn tls_clients_verify_the_server_certificate() {
    let fixture = fixture("tls");
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("the test certificate is generated");
    std::fs::create_dir_all(&fixture.root).expect("the fixture directory is created");
    let certificate = fixture.root.join("cert.pem");
    let key = fixture.root.join("key.pem");
    std::fs::write(&certificate, certified.cert.pem()).expect("the certificate is written");
    std::fs::write(&key, certified.key_pair.serialize_pem()).expect("the key is written");
    let server = RunningServer::start(
        ServerConfig {
            root: fixture.root.join("cache"),
            token: Some(RemoteToken::new("integration-secret").expect("the token is valid")),
            execution: false,
            tls: Some((certificate.clone(), key)),
            ..ServerConfig::default()
        },
        "127.0.0.1:0".parse().expect("the address parses"),
    )
    .expect("the TLS server starts");
    let client = RemoteClient::connect(RemoteConfig {
        endpoint: server.endpoint().to_owned(),
        token: Some(RemoteToken::new("integration-secret").expect("the token is valid")),
        ca_certificate: Some(certificate),
        tls_domain: Some("localhost".to_owned()),
        timeout: Duration::from_secs(30),
        execution: false,
        platform: Vec::new(),
        ..RemoteConfig::default()
    })
    .expect("the TLS client connects");
    client
        .capabilities()
        .expect("TLS capabilities are available");
    let bytes = b"tls payload".to_vec();
    let digest = Digest::of_bytes(&bytes);
    client
        .upload_blob(digest, &bytes)
        .expect("the TLS blob uploads");
    assert_eq!(
        client
            .download_blob(digest, bytes.len() as u64)
            .expect("the TLS blob downloads"),
        bytes
    );
}

#[test]
fn concurrent_clients_share_identical_blobs() {
    let fixture = fixture("concurrent");
    let server = server(&fixture.root, false);
    let endpoint = server.endpoint().to_owned();
    let mut threads = Vec::new();
    for _ in 0..4 {
        let endpoint = endpoint.clone();
        threads.push(std::thread::spawn(move || {
            let client = client(endpoint.as_str(), false);
            let bytes = vec![0x5a; 64 * 1024];
            let digest = Digest::of_bytes(&bytes);
            client
                .upload_blob(digest, &bytes)
                .expect("the blob uploads");
            assert_eq!(
                client
                    .download_blob(digest, bytes.len() as u64)
                    .expect("the blob downloads"),
                bytes
            );
        }));
    }
    for thread in threads {
        thread.join().expect("the client thread finishes");
    }
}

#[test]
fn a_restarted_server_serves_the_persistent_cache() {
    let fixture = fixture("restart");
    let bytes = b"durable payload".to_vec();
    let digest = Digest::of_bytes(&bytes);
    {
        let server = server(&fixture.root, false);
        let client = client(server.endpoint(), false);
        client
            .upload_blob(digest, &bytes)
            .expect("the blob uploads");
        server.shutdown();
    }
    let server = server(&fixture.root, false);
    let client = client(server.endpoint(), false);
    assert_eq!(
        client
            .download_blob(digest, bytes.len() as u64)
            .expect("the blob survives a restart"),
        bytes
    );
}
