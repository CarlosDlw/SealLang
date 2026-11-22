//! End-to-end tests for the `seal` binary.

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
        "seal-cli-test-{}-{}-{name}",
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

fn write(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("parent");
    }
    std::fs::write(path, contents).expect("write");
}

#[test]
fn a_new_binary_project_builds_checks_and_runs() {
    let fixture = fixture("bin");
    let created = output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    );
    assert_success(&created);
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    let artifact = std::fs::read_dir(fixture.root.join("seal/target/dev"))
        .expect("target directory")
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_none())
        .expect("an executable artifact");
    assert!(artifact.path().is_file());

    let check = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_success(&check);

    write(
        &fixture.root.join("src/main.sl"),
        "use app::util::value;\n\nfunc main() -> i32 {\n  value() + 7\n}\n",
    );
    let run = output(seal().arg("run").arg("--project").arg(&fixture.root));
    assert_eq!(run.status.code(), Some(7));
}

#[test]
fn per_edge_features_build_two_instances_of_one_package() {
    let fixture = fixture("features");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"features\"\nmembers = [\"crates/*\"]\ndefault-members = [\"crates/app\"]\n",
    );
    write(
        &fixture.root.join("crates/common/seal.toml"),
        "[package]\nname = \"common\"\nversion = \"0.1.0\"\n\n[package.features]\ndefault = []\na = []\nb = []\n",
    );
    write(
        &fixture.root.join("crates/common/src/lib.sl"),
        "cfg(feature(\"a\")) public func which() -> i32 { 1 }\ncfg(feature(\"b\")) public func which() -> i32 { 2 }\npublic func base() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("crates/left/seal.toml"),
        "[package]\nname = \"left\"\nversion = \"0.1.0\"\n\n[dependencies]\ncommon = { path = \"../common\", features = [\"a\"] }\n",
    );
    write(
        &fixture.root.join("crates/left/src/lib.sl"),
        "use common::which;\npublic func left() -> i32 { which() }\n",
    );
    write(
        &fixture.root.join("crates/right/seal.toml"),
        "[package]\nname = \"right\"\nversion = \"0.1.0\"\n\n[dependencies]\ncommon = { path = \"../common\", features = [\"b\"] }\n",
    );
    write(
        &fixture.root.join("crates/right/src/lib.sl"),
        "use common::which;\npublic func right() -> i32 { which() }\n",
    );
    write(
        &fixture.root.join("crates/app/seal.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nleft = { path = \"../left\" }\nright = { path = \"../right\" }\n",
    );
    write(
        &fixture.root.join("crates/app/src/main.sl"),
        "use left::left;\nuse right::right;\nfunc main() -> i32 { left() + right() }\n",
    );
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    let lock = std::fs::read_to_string(fixture.root.join("seal.lock")).expect("lockfile");
    assert!(lock.contains("[[instances]]"));
    assert_success(&output(
        seal()
            .arg("build")
            .arg("--locked")
            .arg("--project")
            .arg(&fixture.root),
    ));
    let tree = output(seal().arg("tree").arg("--project").arg(&fixture.root));
    assert_success(&tree);
    let text = String::from_utf8_lossy(&tree.stdout);
    assert_eq!(text.matches("common 0.1.0").count(), 2);
    assert!(text.contains("[a"));
    assert!(text.contains("[b"));
    let why = output(
        seal()
            .arg("why")
            .arg("-p")
            .arg("common")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&why);
    let why = String::from_utf8_lossy(&why.stdout);
    assert_eq!(why.matches("common 0.1.0").count(), 2);
    assert!(why.contains("required by"));
}

#[test]
fn a_library_project_builds_without_a_main() {
    let fixture = fixture("lib");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--template")
            .arg("lib")
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    let objects: Vec<_> = std::fs::read_dir(fixture.root.join("seal/target/dev"))
        .expect("target directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "o")
        })
        .collect();
    assert_eq!(objects.len(), 1);
}

#[test]
fn a_workspace_builds_every_member() {
    let fixture = fixture("workspace");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--template")
            .arg("workspace")
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--workspace"),
    );
    assert_success(&build);
    assert!(fixture.root.join("seal/target/dev/app").is_file());
    assert!(fixture.root.join("seal/target/dev/libgreet.o").is_file());
}

#[test]
fn all_targets_builds_tests_benches_and_examples() {
    let fixture = fixture("all-targets");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"all-targets\"\n\n[package]\nname = \"all-targets\"\nversion = \"0.1.0\"\n\n\
         [targets.share]\nkind = \"lib\"\n\n\
         [targets.main]\nkind = \"bin\"\nentry = \"root::main\"\n\n\
         [targets.checks]\nkind = \"test\"\n\n\
         [targets.measure]\nkind = \"bench\"\n\n\
         [targets.demo]\nkind = \"example\"\nentry = \"root::main\"\n",
    );
    write(
        &fixture.root.join("src/lib.sl"),
        "public func answer() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("tests/checks.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("benches/measure.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture.root.join("examples/demo.sl"),
        "func main() -> i32 { 0 }\n",
    );

    let build = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--all-targets"),
    );
    assert_success(&build);
    for artifact in ["main", "checks", "measure", "demo", "libshare.o"] {
        assert!(
            fixture
                .root
                .join("seal/target/dev")
                .join(artifact)
                .is_file(),
            "--all-targets writes {artifact}"
        );
    }
}

#[test]
fn quiet_suppresses_status_and_color_is_explicit() {
    let fixture = fixture("output-policy");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));

    let quiet = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--quiet"),
    );
    assert_success(&quiet);
    assert!(
        quiet.stdout.is_empty(),
        "--quiet writes no status to stdout"
    );

    let colored = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--color")
            .arg("always"),
    );
    assert_success(&colored);
    assert!(
        String::from_utf8_lossy(&colored.stdout).contains("\u{1b}[1m"),
        "--color=always emits ANSI even when stdout is redirected"
    );
}

#[test]
fn metadata_is_parseable_and_lists_modules_and_targets() {
    let fixture = fixture("metadata");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let metadata = output(
        seal()
            .arg("metadata")
            .arg("--json")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&metadata);
    let value: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).expect("metadata is JSON");
    assert_eq!(value["version"], 1);
    let package = &value["packages"][0];
    assert!(
        package["name"]
            .as_str()
            .is_some_and(|name| name.starts_with("seal-cli-test"))
    );
    assert!(
        package["targets"]
            .as_array()
            .is_some_and(|targets| !targets.is_empty())
    );
    assert!(
        package["modules"]
            .as_array()
            .is_some_and(|modules| modules.iter().any(|module| module == "app::util"))
    );
}

#[test]
fn two_binaries_make_run_ambiguous_until_one_is_selected() {
    let fixture = fixture("bins");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"seal-cli-test\"\n\n[package]\nname = \"seal-cli-test\"\nversion = \"0.1.0\"\n\n\
         [targets.first]\nkind = \"bin\"\nentry = \"root::main\"\n\n\
         [targets.second]\nkind = \"bin\"\nentry = \"root::main\"\n",
    );
    let run = output(seal().arg("run").arg("--project").arg(&fixture.root));
    assert_eq!(run.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&run.stderr).contains("E8103"));
    let selected = output(
        seal()
            .arg("run")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--bin")
            .arg("first"),
    );
    assert_success(&selected);
}

#[test]
fn an_entry_that_names_no_function_is_e2087() {
    let fixture = fixture("entry");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"seal-cli-test\"\n\n[package]\nname = \"seal-cli-test\"\nversion = \"0.1.0\"\n\n\
         [targets.main]\nkind = \"bin\"\nentry = \"app::missing\"\n",
    );
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("E2087"));
}

#[test]
fn a_target_without_an_entry_is_e2087() {
    let fixture = fixture("missing-entry");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"missing-entry\"\n\n[package]\nname = \"missing-entry\"\nversion = \"0.1.0\"\n\n\
         [targets.main]\nkind = \"bin\"\n",
    );
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("E2087"));
}

#[test]
fn a_missing_manifest_points_at_seal_init() {
    let fixture = fixture("no-manifest");
    std::fs::create_dir_all(&fixture.root).expect("root");
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("seal init"));
}

#[test]
fn verify_repro_builds_two_copies_and_finds_them_identical() {
    let fixture = fixture("verify-repro");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let check = output(
        seal()
            .arg("--project")
            .arg(&fixture.root)
            .arg("verify-repro"),
    );
    assert_success(&check);
    assert!(String::from_utf8_lossy(&check.stdout).contains("reproducible"));
}

#[test]
fn a_kernel_template_builds_freestanding_and_exits_42() {
    let fixture = fixture("kernel");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--template")
            .arg("kernel")
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    assert!(
        !String::from_utf8_lossy(&build.stderr).contains("unknown key"),
        "the kernel manifest uses only known keys: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let run = output(seal().arg("run").arg("--project").arg(&fixture.root));
    assert_eq!(
        run.status.code(),
        Some(42),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(fixture.root.join("seal/target/dev/kernel").is_file());
}

#[test]
fn a_compiler_template_builds_the_multi_file_pipeline() {
    let fixture = fixture("compiler");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--template")
            .arg("compiler")
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    let run = output(seal().arg("run").arg("--project").arg(&fixture.root));
    assert_eq!(
        run.status.code(),
        Some(42),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    let metadata = output(seal().arg("metadata").arg("--project").arg(&fixture.root));
    assert_success(&metadata);
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).expect("metadata JSON");
    let modules = metadata["packages"][0]["modules"]
        .as_array()
        .expect("modules");
    assert!(
        modules.iter().any(|module| module == "app::ast")
            && modules.iter().any(|module| module == "app::eval")
            && modules.iter().any(|module| module == "app::parser"),
        "{metadata}"
    );
}

#[test]
fn target_commands_add_and_report_specifications() {
    let fixture = fixture("targets");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let list = output(
        seal()
            .arg("target")
            .arg("list")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&list);
    assert!(String::from_utf8_lossy(&list.stdout).contains("x86_64"));

    let spec = fixture.root.join("custom-target.toml");
    write(
        &spec,
        "triple = \"aarch64-unknown-none\"\nruntime = \"none\"\npanic = \"trap\"\nlinker = \"aarch64-linux-gnu-gcc\"\n",
    );
    let add = output(
        seal()
            .arg("target")
            .arg("add")
            .arg(&spec)
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&add);
    let info = output(
        seal()
            .arg("target")
            .arg("info")
            .arg("aarch64-unknown-none")
            .arg("--json")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&info);
    let value: serde_json::Value = serde_json::from_slice(&info.stdout).expect("target JSON");
    assert_eq!(value["runtime"], "none");
    assert_eq!(value["panic"], "trap");

    let invalid = fixture.root.join("invalid-target.toml");
    write(
        &invalid,
        "triple = \"x86_64-unknown-linux-gnu\"\npointer-width = 48\n",
    );
    let rejected = output(
        seal()
            .arg("target")
            .arg("add")
            .arg(&invalid)
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("E8060"));
}

#[test]
fn a_pinned_toolchain_is_verified_before_a_build() {
    let fixture = fixture("toolchain");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let offline = output(
        seal()
            .arg("toolchain")
            .arg("install")
            .arg("--offline")
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_eq!(offline.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&offline.stderr).contains("E8061"));
    let bundle = fixture.root.join("bundle");
    write(&bundle.join("bin/sealc"), "#!/bin/sh\nexit 0\n");
    let checksum = output(
        seal()
            .arg("toolchain")
            .arg("checksum")
            .arg(&bundle)
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&checksum);
    let checksum = String::from_utf8_lossy(&checksum.stdout).trim().to_owned();
    write(
        &bundle.join("seal-toolchain.toml"),
        &format!(
            "channel = \"stable\"\nversion = \"{}\"\nchecksum = \"{checksum}\"\ntargets = [\"x86_64-unknown-linux\", \"x86_64-unknown-linux-gnu\"]\n\n[components]\nsealc = \"bin/sealc\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    );
    let install = output(
        seal()
            .arg("toolchain")
            .arg("install")
            .arg("--from")
            .arg(&bundle)
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&install);
    let pin = output(
        seal()
            .arg("toolchain")
            .arg("pin")
            .arg(env!("CARGO_PKG_VERSION"))
            .arg("--project")
            .arg(&fixture.root),
    );
    assert_success(&pin);
    assert_success(&output(
        seal().arg("build").arg("--project").arg(&fixture.root),
    ));

    let installed = fixture
        .root
        .join("seal/toolchains")
        .join(env!("CARGO_PKG_VERSION"))
        .join("bin/sealc");
    std::fs::write(&installed, b"tampered").expect("tamper");
    let rejected = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_eq!(rejected.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("E8061"));
}

#[test]
fn a_cross_binary_without_a_linker_is_e8062() {
    let fixture = fixture("cross-linker");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"cross\"\n\n[package]\nname = \"cross\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/main.sl"),
        "func main() -> i32 { 0 }\n",
    );
    let build = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--target")
            .arg("aarch64-unknown-linux-gnu"),
    );
    assert_eq!(build.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&build.stderr).contains("E8062"));
}

#[test]
fn a_cross_binary_may_not_link_the_host_runtime() {
    let fixture = fixture("cross-runtime");
    write(
        &fixture.root.join("seal.toml"),
        "[project]\nname = \"cross\"\n\n[package]\nname = \"cross\"\nversion = \"0.1.0\"\n",
    );
    write(
        &fixture.root.join("src/main.sl"),
        "func main() -> i32 { 0 }\n",
    );
    write(
        &fixture
            .root
            .join("seal/targets/aarch64-unknown-linux-gnu.toml"),
        "triple = \"aarch64-unknown-linux-gnu\"\nlinker = \"cc\"\n",
    );
    let build = output(
        seal()
            .arg("build")
            .arg("--project")
            .arg(&fixture.root)
            .arg("--target")
            .arg("aarch64-unknown-linux-gnu"),
    );
    assert_eq!(build.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&build.stderr);
    assert!(stderr.contains("E6003"), "unexpected stderr: {stderr}");
    assert!(stderr.contains("runtime"), "unexpected stderr: {stderr}");
}

#[cfg(unix)]
#[test]
fn a_target_spec_linker_is_used_by_the_build() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = fixture("spec-linker");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&fixture.root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let log = fixture.root.join("linker-invoked");
    let linker = fixture.root.join("fake-linker.sh");
    write(
        &linker,
        &format!(
            "#!/bin/sh\nprintf invoked > '{}'\nout=\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = \"-o\" ]; then shift; out=\"$1\"; fi\n  shift\ndone\n: > \"$out\"\n",
            log.display()
        ),
    );
    let mut permissions = std::fs::metadata(&linker)
        .expect("linker metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&linker, permissions).expect("executable linker");

    let host = seal_target::host_triple();
    write(
        &fixture.root.join(format!("seal/targets/{host}.toml")),
        &format!("triple = \"{host}\"\nlinker = \"{}\"\n", linker.display()),
    );
    let build = output(seal().arg("build").arg("--project").arg(&fixture.root));
    assert_success(&build);
    assert!(
        log.is_file(),
        "the build must use the linker declared by the target spec\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
}

#[test]
fn paths_with_spaces_and_unicode_work() {
    let fixture = fixture("spaces");
    let root = fixture.root.join("projeto com espaço");
    assert_success(&output(
        seal()
            .arg("new")
            .arg(&root)
            .arg("--vcs")
            .arg("none")
            .arg("--yes"),
    ));
    let build = output(seal().arg("build").arg("--project").arg(&root));
    assert_success(&build);
}
