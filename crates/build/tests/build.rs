//! End-to-end build tests over real Seal projects.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use seal_build::{BuildOptions, Builder, JobRequest, TaskRequest, TaskRunner};
use seal_project::{PlanOptions, Project};
use seal_sandbox::Sandbox;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(prefix: &str, files: &[(&str, &str)]) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("fixture root");
        for (path, contents) in files {
            let path = root.join(path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("fixture parent");
            }
            std::fs::write(path, contents).expect("fixture file");
        }
        Self { root }
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    fn project(&self) -> Project {
        Project::load(&self.root).expect("the fixture project loads")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn options(jobs: usize, cache: bool) -> BuildOptions {
    BuildOptions {
        jobs,
        use_cache: cache,
        sandbox: Sandbox::detect(),
        cache_root: None,
        link_args: Vec::new(),
        progress: None,
    }
}

fn job(target: &str) -> JobRequest {
    JobRequest {
        package: 0,
        target: target.to_owned(),
        options: PlanOptions {
            target: seal_target::host_triple().as_str().to_owned(),
            ..PlanOptions::default()
        },
    }
}

fn basic_project() -> Fixture {
    Fixture::new(
        "seal-build-basic",
        &[
            (
                "seal.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
            ),
            ("src/main.sl", "func main() -> i32 {\n  0\n}\n"),
        ],
    )
}

fn library_project(kind: &str) -> Fixture {
    Fixture::new(
        "seal-build-library",
        &[
            (
                "seal.toml",
                &format!(
                    "[package]\nname = \"libdemo\"\nversion = \"0.1.0\"\n\n\
                     [targets.math]\nkind = \"{kind}\"\nsources = [\"src/lib.sl\"]\n"
                ),
            ),
            (
                "src/lib.sl",
                "export \"c\" func add(a: i32, b: i32) -> i32 {\n  a + b\n}\n",
            ),
        ],
    )
}

#[test]
fn static_library_targets_produce_a_deterministic_archive() {
    let fixture = library_project("staticlib");
    let project = fixture.project();
    let builder = Builder::new(&project, options(1, false));
    let first = builder
        .build(&[job("math")])
        .expect("the static library builds");
    assert_eq!(first.outputs.len(), 1);
    let archive = fixture
        .path("seal/target/dev")
        .join(seal_plan::static_library_name("math"));
    assert_eq!(first.outputs[0].path, archive);
    let bytes = std::fs::read(&archive).expect("the archive exists");

    std::fs::remove_dir_all(fixture.path("seal")).expect("clean target");
    builder
        .build(&[job("math")])
        .expect("the static library rebuilds");
    assert_eq!(
        std::fs::read(&archive).expect("the rebuilt archive exists"),
        bytes,
        "the archive must not depend on temporary paths or timestamps"
    );
}

#[test]
fn shared_library_targets_produce_a_platform_library() {
    let fixture = library_project("cdylib");
    let project = fixture.project();
    let build = Builder::new(&project, options(1, false))
        .build(&[job("math")])
        .expect("the shared library builds");
    assert_eq!(build.outputs.len(), 1);
    let expected = fixture
        .path("seal/target/dev")
        .join(seal_plan::shared_library_name("math"));
    assert_eq!(build.outputs[0].path, expected);
}

fn generator_project() -> Fixture {
    Fixture::new(
        "seal-build-generator",
        &[
            (
                "seal.toml",
                "[package]\nname = \"genproj\"\nversion = \"0.1.0\"\n\n\
                 [targets.gen]\nkind = \"generator\"\nentry = \"root::main\"\n\
                 sources = [\"src/gen.sl\"]\ninputs = [\"data/*.txt\"]\n\
                 outputs = [\"generated/table.sl\"]\n\n\
                 [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\n\
                 sources = [\"src/main.sl\"]\ngenerated = [\"gen\"]\n",
            ),
            ("src/gen.sl", GENERATOR_SOURCE),
            ("src/main.sl", "func main() -> i32 {\n  value()\n}\n"),
            ("data/table.txt", "func value() -> i32 {\n  0\n}\n"),
        ],
    )
}

const GENERATOR_SOURCE: &str = r#"extern "c" {
  func getenv(name: *raw u8) -> *raw u8;
  func fopen(path: *raw u8, mode: *raw u8) -> *raw u8;
  func fgets(buffer: *raw mut u8, size: c_int, stream: *raw u8) -> *raw u8;
  func fputs(text: *raw u8, stream: *raw u8) -> c_int;
  func fclose(stream: *raw u8) -> c_int;
}

func main() -> i32 {
  unsafe {
    let input = getenv(c"SEAL_GENERATOR_INPUTS");
    if input.is_null() { return 1; }
    let source = fopen(input, c"r");
    if source.is_null() { return 2; }
    let output = fopen(c"generated/table.sl", c"w");
    if output.is_null() { return 3; }
    let mut buffer: [u8; 256] = [0u8; 256];
    while !fgets(&mut buffer[0] as *raw mut u8, 256, source).is_null() {
      if fputs(&buffer[0] as *raw u8, output) < 0 { return 4; }
    }
    fclose(source);
    fclose(output);
  }
  0
}
"#;

#[test]
fn a_repeated_build_is_a_full_cache_hit() {
    let fixture = basic_project();
    let project = fixture.project();
    let builder = Builder::new(&project, options(1, true));
    let first = builder
        .build(&[job("demo")])
        .expect("the first build succeeds");
    assert_eq!(first.outputs.len(), 1);
    assert!(fixture.path("seal/target/dev/demo").is_file());
    assert!(first.misses() > 0);

    let second = builder
        .build(&[job("demo")])
        .expect("the second build succeeds");
    assert_eq!(second.hits(), second.timings.len());
    assert_eq!(second.misses(), 0);
    assert!(second.outputs[0].cached);

    std::fs::write(fixture.path("unrelated.txt"), "not a source").expect("unrelated file");
    let third = builder
        .build(&[job("demo")])
        .expect("the third build succeeds");
    assert_eq!(third.misses(), 0);

    std::fs::write(
        fixture.path("src/main.sl"),
        "func main() -> i32 {\n  // changed\n  0\n}\n",
    )
    .expect("source change");
    let fourth = builder
        .build(&[job("demo")])
        .expect("the fourth build succeeds");
    assert!(
        fourth.misses() >= 1,
        "a source change must invalidate the frontend"
    );
}

#[test]
fn job_count_does_not_change_the_artifact() {
    let fixture = basic_project();
    let project = fixture.project();
    let single = Builder::new(&project, options(1, false))
        .build(&[job("demo")])
        .expect("jobs 1");
    let first = std::fs::read(single.outputs[0].path.clone()).expect("artifact");
    std::fs::remove_dir_all(fixture.path("seal")).expect("clean");
    let parallel = Builder::new(&project, options(4, false))
        .build(&[job("demo")])
        .expect("jobs 4");
    let second = std::fs::read(parallel.outputs[0].path.clone()).expect("artifact");
    assert_eq!(first, second);
}

#[test]
fn cgu_splits_are_parallel_and_only_the_changed_unit_reruns() {
    let fixture = Fixture::new(
        "seal-build-cgu",
        &[
            (
                "seal.toml",
                "[package]\nname = \"cgudemo\"\nversion = \"0.1.0\"\n\n\
                 [profiles.dev]\ncodegen-units = 4\nopt-level = 1\n",
            ),
            (
                "src/main.sl",
                "use app::util::value;\n\nfunc main() -> i32 {\n  value()\n}\n",
            ),
            (
                "src/util.sl",
                "module app::util;\n\ninternal func value() -> i32 {\n  0\n}\n",
            ),
        ],
    );
    let project = fixture.project();
    let builder = Builder::new(&project, options(4, true));
    let first = builder
        .build(&[job("cgudemo")])
        .expect("the first build succeeds");
    let codegen: Vec<_> = first
        .timings
        .iter()
        .filter(|timing| timing.kind == "codegen-cgu")
        .collect();
    assert_eq!(codegen.len(), 4, "one action per code generation unit");
    assert!(codegen.iter().all(|timing| !timing.cached));
    let artifact = fixture.path("seal/target/dev/cgudemo");
    assert_eq!(run(&artifact), 0);

    std::fs::write(
        fixture.path("src/util.sl"),
        "module app::util;\n\ninternal func value() -> i32 {\n  7\n}\n",
    )
    .expect("source change");
    let second = builder
        .build(&[job("cgudemo")])
        .expect("the second build succeeds");
    let misses: Vec<_> = second
        .timings
        .iter()
        .filter(|timing| timing.kind == "codegen-cgu" && !timing.cached)
        .collect();
    assert_eq!(misses.len(), 1, "only the affected unit reruns");
    assert!(
        second
            .timings
            .iter()
            .any(|timing| timing.kind == "link" && !timing.cached),
        "the link consumes the changed object"
    );
    assert_eq!(run(&artifact), 7);

    std::fs::write(
        fixture.path("seal.toml"),
        "[package]\nname = \"cgudemo\"\nversion = \"0.1.0\"\n\n\
         [profiles.dev]\ncodegen-units = 4\nopt-level = 2\n",
    )
    .expect("profile change");
    let project = fixture.project();
    let builder = Builder::new(&project, options(4, true));
    let third = builder
        .build(&[job("cgudemo")])
        .expect("the third build succeeds");
    let misses: Vec<_> = third
        .timings
        .iter()
        .filter(|timing| timing.kind == "codegen-cgu" && !timing.cached)
        .collect();
    assert_eq!(
        misses.len(),
        4,
        "an optimization change invalidates every unit even when the bitcode is unchanged"
    );
}

#[test]
fn generators_run_only_when_their_inputs_or_tool_change() {
    let fixture = generator_project();
    let project = fixture.project();
    let builder = Builder::new(&project, options(1, true));
    let jobs = [job("gen"), job("app")];
    let first = builder.build(&jobs).expect("the generator build succeeds");
    assert_eq!(first.outputs.len(), 2);
    let app = fixture.path("seal/target/dev/app");
    assert_eq!(run(&app), 0);
    assert_eq!(
        std::fs::read_to_string(fixture.path("generated/table.sl")).expect("generated file"),
        "func value() -> i32 {\n  0\n}\n"
    );

    let second = builder.build(&jobs).expect("the no-op build succeeds");
    assert_eq!(second.misses(), 0);

    std::fs::write(
        fixture.path("data/table.txt"),
        "func value() -> i32 {\n  7\n}\n",
    )
    .expect("input change");
    let third = builder
        .build(&jobs)
        .expect("the incremental build succeeds");
    assert!(
        third.misses() >= 2,
        "the generator and its consumer must rerun"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.path("generated/table.sl")).expect("generated file"),
        "func value() -> i32 {\n  7\n}\n"
    );
    assert_eq!(run(&app), 7);

    std::fs::write(
        fixture.path("src/gen.sl"),
        format!("{GENERATOR_SOURCE}\n// tool change\n"),
    )
    .expect("tool change");
    let fourth = builder.build(&jobs).expect("the tool rebuild succeeds");
    assert!(fourth.misses() >= 1);
}

fn run(program: &Path) -> i32 {
    std::process::Command::new(program)
        .status()
        .expect("the program runs")
        .code()
        .expect("the program exits normally")
}

#[test]
fn an_undeclared_generator_output_is_e8042() {
    let fixture = Fixture::new(
        "seal-build-generator-extra",
        &[
            (
                "seal.toml",
                "[package]\nname = \"genproj\"\nversion = \"0.1.0\"\n\n\
                 [targets.gen]\nkind = \"generator\"\nentry = \"root::main\"\n\
                 sources = [\"src/gen.sl\"]\noutputs = [\"generated/table.sl\"]\n\n\
                 [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\n\
                 sources = [\"src/main.sl\"]\ngenerated = [\"gen\"]\n",
            ),
            (
                "src/gen.sl",
                r#"extern "c" {
  func fopen(path: *raw u8, mode: *raw u8) -> *raw u8;
  func fputs(text: *raw u8, stream: *raw u8) -> c_int;
  func fclose(stream: *raw u8) -> c_int;
}
func main() -> i32 {
  unsafe {
    let a = fopen(c"generated/table.sl", c"w");
    fputs(c"func value() -> i32 { 0 }\n", a);
    fclose(a);
    let b = fopen(c"generated/extra.sl", c"w");
    fputs(c"func extra() -> i32 { 0 }\n", b);
    fclose(b);
  }
  0
}
"#,
            ),
            ("src/main.sl", "func main() -> i32 { value() }\n"),
        ],
    );
    let project = fixture.project();
    let error = Builder::new(&project, options(1, true))
        .build(&[job("gen"), job("app")])
        .expect_err("the undeclared output fails the build");
    assert_eq!(error.code(), "E8042");
}

#[test]
fn a_generator_output_may_not_overwrite_a_source() {
    let fixture = Fixture::new(
        "seal-build-generator-collision",
        &[
            (
                "seal.toml",
                "[package]\nname = \"genproj\"\nversion = \"0.1.0\"\n\n\
                 [targets.gen]\nkind = \"generator\"\nentry = \"root::main\"\n\
                 sources = [\"src/gen.sl\"]\noutputs = [\"src/main.sl\"]\n\n\
                 [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\n\
                 sources = [\"src/main.sl\"]\ngenerated = [\"gen\"]\n",
            ),
            ("src/gen.sl", "func main() -> i32 {\n  0\n}\n"),
            ("src/main.sl", "func main() -> i32 { 0 }\n"),
        ],
    );
    let project = fixture.project();
    let error = Builder::new(&project, options(1, true))
        .build(&[job("gen"), job("app")])
        .expect_err("the collision fails the build");
    assert_eq!(error.code(), "E8042");
    assert!(fixture.path("src/main.sl").is_file());
}

#[test]
fn a_generator_cannot_read_an_undeclared_package_file() {
    let fixture = Fixture::new(
        "seal-build-generator-input",
        &[
            (
                "seal.toml",
                r#"[package]
name = "genproj"
version = "0.1.0"

[targets.gen]
kind = "generator"
entry = "root::main"
sources = ["src/gen.sl"]
inputs = ["data/*.txt"]
outputs = ["generated/out.sl"]

[targets.app]
kind = "bin"
entry = "root::main"
sources = ["src/main.sl"]
generated = ["gen"]
"#,
            ),
            (
                "src/gen.sl",
                r#"extern "c" {
  func fopen(path: *raw u8, mode: *raw u8) -> *raw u8;
  func fclose(stream: *raw u8) -> c_int;
}
func main() -> i32 {
  unsafe {
    let secret = fopen(c"secret.txt", c"r");
    if secret.is_null() { return 2; }
    fclose(secret);
  }
  0
}
"#,
            ),
            ("src/main.sl", "func main() -> i32 { 0 }\n"),
            ("data/input.txt", "payload\n"),
            ("secret.txt", "must not be visible\n"),
        ],
    );
    let project = fixture.project();
    let error = Builder::new(&project, options(1, true))
        .build(&[job("gen"), job("app")])
        .expect_err("the undeclared read fails");
    assert_eq!(error.code(), "E8041");
}

#[test]
fn a_generator_that_writes_nothing_is_e8042() {
    let fixture = Fixture::new(
        "seal-build-generator-missing",
        &[
            (
                "seal.toml",
                r#"[package]
name = "genproj"
version = "0.1.0"

[targets.gen]
kind = "generator"
entry = "root::main"
sources = ["src/gen.sl"]
outputs = ["generated/out.sl"]

[targets.app]
kind = "bin"
entry = "root::main"
sources = ["src/main.sl"]
generated = ["gen"]
"#,
            ),
            ("src/gen.sl", "func main() -> i32 { 0 }\n"),
            ("src/main.sl", "func main() -> i32 { 0 }\n"),
        ],
    );
    let project = fixture.project();
    let error = Builder::new(&project, options(1, true))
        .build(&[job("gen"), job("app")])
        .expect_err("the missing output fails");
    assert_eq!(error.code(), "E8042");
}

#[test]
fn a_missing_pkg_config_package_is_e8043() {
    let fixture = Fixture::new(
        "seal-build-native-missing",
        &[
            (
                "seal.toml",
                r#"[package]
name = "nativeproj"
version = "0.1.0"

[native.missing]
pkg-config = "definitely-not-a-real-package"

[targets.app]
kind = "bin"
entry = "root::main"
sources = ["src/main.sl"]
native = ["missing"]
"#,
            ),
            ("src/main.sl", "func main() -> i32 { 0 }\n"),
        ],
    );
    let project = fixture.project();
    let error = Builder::new(&project, options(1, true))
        .build(&[job("app")])
        .expect_err("the missing native package fails");
    assert_eq!(error.code(), "E8043");
}

#[test]
fn declared_native_libraries_are_recorded_and_linked() {
    let fixture = Fixture::new(
        "seal-build-native",
        &[
            (
                "seal.toml",
                "[package]\nname = \"nativeproj\"\nversion = \"0.1.0\"\n\n\
                 [native.math]\nlibs = [\"m\"]\n\n\
                 [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\n\
                 sources = [\"src/main.sl\"]\nnative = [\"math\"]\n",
            ),
            ("src/main.sl", "func main() -> i32 {\n  0\n}\n"),
        ],
    );
    let project = fixture.project();
    let report = Builder::new(&project, options(1, true))
        .build(&[job("app")])
        .expect("the native build succeeds");
    assert_eq!(report.native.len(), 1);
    assert_eq!(report.native[0].alias, "math");
    assert!(report.native[0].libs.iter().any(|lib| lib == "m"));
    assert!(fixture.path("seal/target/dev/app").is_file());
}

#[test]
fn pkg_config_native_dependencies_are_resolved_when_available() {
    let available = std::process::Command::new("pkg-config")
        .args(["--libs", "zlib"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        return;
    }
    let fixture = Fixture::new(
        "seal-build-pkg-config",
        &[
            (
                "seal.toml",
                "[package]\nname = \"nativeproj\"\nversion = \"0.1.0\"\n\n\
                 [native.zlib]\npkg-config = \"zlib\"\n\n\
                 [targets.app]\nkind = \"bin\"\nentry = \"root::main\"\n\
                 sources = [\"src/main.sl\"]\nnative = [\"zlib\"]\n",
            ),
            ("src/main.sl", "func main() -> i32 {\n  0\n}\n"),
        ],
    );
    let project = fixture.project();
    let report = Builder::new(&project, options(1, true))
        .build(&[job("app")])
        .expect("the pkg-config build succeeds");
    assert_eq!(report.native.len(), 1);
    assert!(report.native[0].libs.iter().any(|lib| lib == "z"));
}

#[cfg(unix)]
#[test]
fn tasks_run_in_order_and_honor_fail_fast() {
    let fixture = Fixture::new(
        "seal-build-tasks",
        &[
            (
                "seal.toml",
                "[package]\nname = \"taskproj\"\nversion = \"0.1.0\"\n\n\
                 [tasks.ci]\nsteps = [\n\
                   { program = \"/bin/sh\", args = [\"-c\", \"echo one\"] },\n\
                   { program = \"/bin/sh\", args = [\"-c\", \"exit 7\"] },\n\
                   { program = \"/bin/sh\", args = [\"-c\", \"echo three\"] },\n\
                 ]\n",
            ),
            ("src/main.sl", "func main() -> i32 { 0 }\n"),
        ],
    );
    let loaded = seal_manifest::load_str(include_str!("fixtures/tasks.toml"))
        .expect("the task manifest parses");
    let task = loaded.manifest.tasks.get("ci").expect("the task exists");
    let runner = TaskRunner::new(&fixture.root, Sandbox::detect());
    let report = runner
        .run(
            &TaskRequest {
                name: "ci".to_owned(),
            },
            task,
            true,
        )
        .expect("the task runs");
    assert_eq!(report.outcomes.len(), 2);
    assert_eq!(report.failed, 1);
    let report = runner
        .run(
            &TaskRequest {
                name: "ci".to_owned(),
            },
            task,
            false,
        )
        .expect("the task runs");
    assert_eq!(report.outcomes.len(), 3);
    assert_eq!(report.failed, 1);
}
