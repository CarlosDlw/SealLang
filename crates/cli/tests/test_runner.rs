//! End-to-end tests for the test runner, benchmarks, and CLI polish commands.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

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
        "seal-cli-runner-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temporary root");
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

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, contents).expect("write");
}

fn runner_fixture(name: &str) -> Fixture {
    let fixture = fixture(name);
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"runner\"\n\n[package]\nname = \"runner\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/lib.sl"),
        "public func double(value: i32) -> i32 { value * 2 }\n\
         \n\
         func hidden() -> i32 { 7 }\n\
         \n\
         test func passing_test() { assert(double(2) == 4); }\n\
         \n\
         test func private_access() { assert(hidden() == 7); }\n\
         \n\
         test func failing_test() { assert(double(2) == 5, \"on purpose\"); }\n",
    );
    write(
        &fixture.root.join("tests/integration.sl"),
        "use runner::double;\n\n\
         test func integration_test() { assert(double(21) == 42); }\n",
    );
    write(
        &fixture.root.join("benches/work.sl"),
        "bench func counting_loop() {\n\
         \x20 let mut total = 0;\n\
         \x20 let mut index = 0;\n\
         \x20 while index < 10 {\n\
         \x20   total = total + index;\n\
         \x20   index = index + 1;\n\
         \x20 }\n\
         \x20 assert(total == 45);\n\
         }\n",
    );
    fixture
}

#[test]
fn tests_run_isolated_and_fail_with_the_process_status() {
    let fixture = runner_fixture("results");
    let passing = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("passing"),
    );
    assert_success(&passing);
    let stdout = String::from_utf8_lossy(&passing.stdout);
    assert!(stdout.contains("PASS passing_test"), "{stdout}");
    assert!(stdout.contains("1 passed"), "{stdout}");

    let failing = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("failing"),
    );
    assert_eq!(failing.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&failing.stdout);
    assert!(stdout.contains("FAIL failing_test"), "{stdout}");
    assert!(stdout.contains("on purpose"), "{stdout}");
}

#[test]
fn unit_tests_see_private_items_and_integration_tests_do_not_need_them() {
    let fixture = runner_fixture("visibility");
    let unit = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("private_access"),
    );
    assert_success(&unit);
    let integration = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("integration"),
    );
    assert_success(&integration);
}

#[test]
fn stress_and_partitions_cover_every_test_once() {
    let fixture = runner_fixture("shards");
    let stressed = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("passing")
            .arg("--stress")
            .arg("2"),
    );
    assert_success(&stressed);
    let stdout = String::from_utf8_lossy(&stressed.stdout);
    assert!(stdout.contains("#1"), "{stdout}");
    assert!(stdout.contains("#2"), "{stdout}");

    let left = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("passing")
            .arg("--partition")
            .arg("0/2"),
    );
    assert_success(&left);
    let right = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("passing")
            .arg("--partition")
            .arg("1/2"),
    );
    assert_success(&right);
    let total = String::from_utf8_lossy(&left.stdout)
        .matches("PASS")
        .count()
        + String::from_utf8_lossy(&right.stdout)
            .matches("PASS")
            .count();
    assert_eq!(total, 1);
}

#[test]
fn json_and_junit_reports_are_machine_readable() {
    let fixture = runner_fixture("reports");
    let json = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("passing")
            .arg("--format")
            .arg("json"),
    );
    assert_success(&json);
    let stdout = String::from_utf8_lossy(&json.stdout);
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("NDJSON line"))
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["status"], "passed");
    assert_eq!(events[1]["event"], "summary");

    let junit_path = fixture.root.join("reports/junit.xml");
    let junit = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--filter")
            .arg("passing")
            .arg("--format")
            .arg("junit")
            .arg("--output")
            .arg(&junit_path),
    );
    assert_success(&junit);
    let xml = std::fs::read_to_string(&junit_path).expect("JUnit report");
    assert!(xml.contains("<testsuites"), "{xml}");
    assert!(xml.contains("passing_test"), "{xml}");
}

#[test]
fn assert_is_scoped_to_test_builds_and_requires_a_literal_message() {
    let fixture = fixture("assert");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"assert\"\n\n[package]\nname = \"assert\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/lib.sl"),
        "public func value() -> i32 {\n  assert(1 == 1);\n  0\n}\n",
    );
    let outside = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_eq!(outside.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&outside.stderr).contains("E8105"));

    write(
        &fixture.root.join("src/lib.sl"),
        "public func value() -> i32 { 0 }\n\ntest func bad_message() { assert(true, 1); }\n",
    );
    let message = output(
        seal()
            .arg("test")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--list"),
    );
    assert_eq!(message.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&message.stderr).contains("E8106"));
}

#[test]
fn benchmarks_run_and_report_wall_time() {
    let fixture = runner_fixture("bench");
    let bench = output(
        seal()
            .arg("bench")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--iterations")
            .arg("1")
            .arg("--samples")
            .arg("1"),
    );
    assert_success(&bench);
    let stdout = String::from_utf8_lossy(&bench.stdout);
    assert!(stdout.contains("counting_loop"), "{stdout}");
    assert!(stdout.contains("ns/iter"), "{stdout}");
}

#[test]
fn watch_once_runs_the_requested_command_without_looping() {
    let fixture = runner_fixture("watch");
    let watch = output(
        seal()
            .arg("watch")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--once")
            .arg("--action")
            .arg("test")
            .arg("--filter")
            .arg("passing"),
    );
    assert_success(&watch);
    assert!(String::from_utf8_lossy(&watch.stdout).contains("passing_test"));
}

#[test]
fn completions_and_the_manual_are_generated_for_every_shell() {
    for shell in ["bash", "zsh", "fish", "powershell"] {
        let completions = output(seal().arg("completions").arg(shell));
        assert_success(&completions);
        let text = String::from_utf8_lossy(&completions.stdout);
        assert!(text.contains("test"), "{shell}: {text}");
        assert!(text.contains("bench"), "{shell}: {text}");
    }
    let man = output(seal().arg("man"));
    assert_success(&man);
    let text = String::from_utf8_lossy(&man.stdout);
    assert!(text.contains(".SH COMMANDS"), "{text}");
    assert!(text.contains("test"), "{text}");
}

#[test]
fn config_resolves_cli_environment_and_default_origins() {
    let fixture = fixture("config");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"config\"\n\n[package]\nname = \"config\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/lib.sl"),
        "public func value() -> i32 { 1 }\n",
    );

    let environment = output(
        seal()
            .arg("config")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--key")
            .arg("jobs")
            .arg("--origin")
            .env("SEAL_JOBS", "9"),
    );
    assert_success(&environment);
    assert_eq!(
        String::from_utf8_lossy(&environment.stdout).trim(),
        "environment"
    );

    let cli = output(
        seal()
            .arg("config")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--jobs")
            .arg("3")
            .arg("--key")
            .arg("jobs")
            .arg("--origin")
            .env("SEAL_JOBS", "9"),
    );
    assert_success(&cli);
    assert_eq!(String::from_utf8_lossy(&cli.stdout).trim(), "cli");

    let default = output(
        seal()
            .arg("config")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--key")
            .arg("target")
            .arg("--origin")
            .env_remove("SEAL_TARGET"),
    );
    assert_success(&default);
    assert_eq!(String::from_utf8_lossy(&default.stdout).trim(), "default");

    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"config\"\n\n[package]\nname = \"config\"\n\
         version = \"0.1.0\"\n\n[registry]\nurl = \"https://example.invalid/index\"\n\
         offline = true\n",
    );
    let project = output(
        seal()
            .arg("config")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--key")
            .arg("offline")
            .arg("--origin")
            .env_remove("SEAL_OFFLINE"),
    );
    assert_success(&project);
    assert_eq!(String::from_utf8_lossy(&project.stdout).trim(), "project");

    let user_config = fixture.root.join("user.toml");
    write(&user_config, "jobs = 11\n");
    let user = output(
        seal()
            .arg("config")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--key")
            .arg("jobs")
            .arg("--origin")
            .env("SEAL_CONFIG", &user_config)
            .env_remove("SEAL_JOBS"),
    );
    assert_success(&user);
    assert_eq!(String::from_utf8_lossy(&user.stdout).trim(), "user");

    write(&user_config, "jobs = \"many\"\n");
    let invalid = output(
        seal()
            .arg("config")
            .arg("--project")
            .arg(&fixture.root)
            .env("SEAL_CONFIG", &user_config),
    );
    assert_eq!(invalid.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("E8110"));
}

#[test]
fn lint_denies_stable_warning_names() {
    let fixture = fixture("lint");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"lint\"\n\n[package]\nname = \"lint\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/lib.sl"),
        "use util::helper;\n\npublic func value() -> i32 { 1 }\n",
    );
    write(
        &fixture.root.join("src/util.sl"),
        "module util;\n\ninternal func helper() -> i32 { 2 }\n",
    );
    let allowed = output(seal().arg("lint").arg("--project").arg(&fixture.root));
    assert_success(&allowed);
    let denied = output(
        seal()
            .arg("lint")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--deny")
            .arg("unused_import"),
    );
    assert_eq!(denied.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&denied.stderr).contains("denied"));

    let unknown = output(
        seal()
            .arg("lint")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--deny")
            .arg("not_a_lint"),
    );
    assert_eq!(unknown.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown lint"));
}

#[test]
fn doctor_reports_the_extended_checks_and_rejects_unknown_lints() {
    let fixture = runner_fixture("doctor");
    let doctor = output(
        seal()
            .arg("doctor")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--json")
            .arg("--fix"),
    );
    assert_success(&doctor);
    let report: serde_json::Value =
        serde_json::from_slice(&doctor.stdout).expect("doctor JSON report");
    assert_eq!(report["version"], 4);
    for key in ["sandbox", "registry", "manifest", "stdlib", "disk"] {
        assert!(report.get(key).is_some(), "missing {key}: {report}");
    }
}
