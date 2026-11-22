//! End-to-end coverage for the real core/std library and its examples.

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
        "seal-cli-stdlib-{}-{}-{name}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("fixture root");
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
    std::fs::write(path, contents).expect("fixture source");
}

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("destination");
    for entry in std::fs::read_dir(from).expect("source directory") {
        let entry = entry.expect("directory entry");
        let path = entry.path();
        let destination = to.join(entry.file_name());
        if path.is_dir() {
            copy_tree(&path, &destination);
        } else {
            std::fs::copy(&path, &destination).expect("source file");
        }
    }
}

fn copy_example(name: &str) -> Fixture {
    let fixture = fixture(name);
    copy_tree(&examples().join(name), &fixture.root);
    fixture
}

#[test]
fn the_repository_standard_library_verifies() {
    let verify = output(seal().arg("stdlib").arg("verify"));
    assert_success(&verify);
    assert!(String::from_utf8_lossy(&verify.stdout).contains("digest"));
}

#[test]
fn both_standard_library_examples_run_and_test() {
    for name in ["stdlib", "core-only"] {
        let fixture = copy_example(name);
        let run = output(
            seal()
                .arg("run")
                .arg("--bin")
                .arg(name)
                .arg("--project")
                .arg(&fixture.root),
        );
        assert_eq!(
            run.status.code(),
            Some(0),
            "{name}: stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );

        let tests = output(seal().arg("test").arg("--project").arg(&fixture.root));
        assert_success(&tests);
        let stdout = String::from_utf8_lossy(&tests.stdout);
        for test in ["option_variants", "result_variants"] {
            assert!(
                stdout.contains(test),
                "{name} did not run `{test}`:\n{stdout}"
            );
        }
    }
}

#[test]
fn std_alloc_is_visible_only_under_the_hosted_standard_library() {
    let fixture = copy_example("stdlib");
    write(
        &fixture.root.join("src/main.sl"),
        "use std::alloc::Layout;\n\nfunc main() -> usize {\n  Layout::new::<u8>().size()\n}\n",
    );

    let hosted = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_success(&hosted);

    for mode in ["core", "none"] {
        write(
            &fixture.root.join("seal.toml"),
            &format!(
                "[project]\nname = \"stdlib\"\nlanguage = \"0.1\"\n\n[package]\nname = \"stdlib\"\nversion = \"0.1.0\"\ndescription = \"The hosted standard library and its prelude\"\nstdlib = \"{mode}\"\n"
            ),
        );
        let rejected = output(seal().arg("check").arg("--project").arg(&fixture.root));
        assert_eq!(
            rejected.status.code(),
            Some(1),
            "mode={mode}: stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&rejected.stdout),
            String::from_utf8_lossy(&rejected.stderr)
        );
        let diagnostics = format!(
            "{}{}",
            String::from_utf8_lossy(&rejected.stdout),
            String::from_utf8_lossy(&rejected.stderr)
        );
        assert!(
            diagnostics.contains("E2073"),
            "mode={mode}: `std::alloc` must be unresolved:\n{diagnostics}"
        );
        assert!(
            !diagnostics.contains("E2070"),
            "mode={mode}: the unresolved `std` path cascaded:\n{diagnostics}"
        );
    }
}

#[test]
fn the_string_surface_runs_in_debug_and_release_profiles() {
    let fixture = copy_example("stdlib");
    write(
        &fixture.root.join("src/main.sl"),
        concat!(
            "func main() -> i32 {\n",
            "  let text: *str = \"a🦀é\";\n",
            "  let mut scalars = 0;\n",
            "  for value in text { let _ = value; scalars += 1; }\n",
            "  let mut bytes = 0;\n",
            "  for value in text.bytes() { let _ = value; bytes += 1; }\n",
            "  let mut pieces = 0;\n",
            "  for piece in \"a,b,c\".split(\",\") { let _ = piece; pieces += 1; }\n",
            "  let mut lines = 0;\n",
            "  for line in \"a\\r\\nb\\n\".lines() { let _ = line; lines += 1; }\n",
            "  let mut words = 0;\n",
            "  for word in \" a\\tb\\u{a0}c \".split_whitespace() { let _ = word; words += 1; }\n",
            "  if scalars != 3 || bytes != 7 || pieces != 3 || lines != 2 || words != 3 { return 1; }\n",
            "  match text.find(\"🦀\") {\n",
            "    Option::Some(index) => if index != 1 { return 2; },\n",
            "    Option::None => { return 3; }\n",
            "  }\n",
            "  if !text.starts_with(\"a\") || !text.ends_with(\"é\") || !text.contains(\"🦀\") { return 4; }\n",
            "  if text.trim() != text || !\"HeLLo\".eq_ignore_ascii_case(\"hello\") { return 5; }\n",
            "  0\n",
            "}\n",
        ),
    );

    for release in [false, true] {
        let run = output(
            seal()
                .arg("run")
                .arg("--bin")
                .arg("stdlib")
                .arg("--project")
                .arg(&fixture.root)
                .args(release.then_some("--release")),
        );
        assert_eq!(
            run.status.code(),
            Some(0),
            "release={release}: stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn the_strings_example_runs_in_debug_and_release_profiles() {
    let source = std::fs::read_to_string(examples().join("052_strings.sl"))
        .expect("the strings example is readable");
    let fixture = copy_example("stdlib");
    write(&fixture.root.join("src/main.sl"), &source);

    let check = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_success(&check);

    for release in [false, true] {
        let run = output(
            seal()
                .arg("run")
                .arg("--bin")
                .arg("stdlib")
                .arg("--project")
                .arg(&fixture.root)
                .args(release.then_some("--release")),
        );
        assert_eq!(
            run.status.code(),
            Some(0),
            "release={release}: stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn string_views_from_bytes_cannot_outlive_their_owner() {
    let fixture = copy_example("stdlib");
    write(
        &fixture.root.join("src/main.sl"),
        "func escape() -> *str {\n\
         \x20 let data: [u8; 2] = [104u8, 105u8];\n\
         \x20 let bytes: *[u8] = &data[..];\n\
         \x20 match std::str::from_utf8(bytes) {\n\
         \x20   Option::Some(view) => view,\n\
         \x20   Option::None => \"\",\n\
         \x20 }\n\
         }\n\
         func main() -> i32 { 0 }\n",
    );
    let check = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_eq!(
        check.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        diagnostics.contains("E5004"),
        "the escaped string view must be rejected:\n{diagnostics}"
    );
}

#[test]
fn string_from_utf8_unchecked_requires_an_unsafe_block() {
    let fixture = copy_example("stdlib");
    write(
        &fixture.root.join("src/main.sl"),
        "func main() -> i32 {\n\
         \x20 let data: [u8; 1] = [97u8];\n\
         \x20 let bytes: *[u8] = &data[..];\n\
         \x20 let text: *str = std::str::from_utf8_unchecked(bytes);\n\
         \x20 0\n\
         }\n",
    );
    let check = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_eq!(
        check.status.code(),
        Some(1),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        diagnostics.contains("E3070"),
        "the unsafe constructor must require an unsafe block:\n{diagnostics}"
    );
}

#[test]
fn string_split_at_panics_with_its_documented_messages() {
    for (index, message) in [
        (1, "split_at index is not a char boundary"),
        (3, "split_at index is out of bounds"),
    ] {
        let fixture = copy_example("stdlib");
        write(
            &fixture.root.join("src/main.sl"),
            &format!(
                "func main() -> i32 {{\n  let text: *str = \"é\";\n  let parts = text.split_at({index});\n  0\n}}\n"
            ),
        );
        let run = output(
            seal()
                .arg("run")
                .arg("--bin")
                .arg("stdlib")
                .arg("--project")
                .arg(&fixture.root),
        );
        assert_eq!(run.status.code(), Some(101));
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(
            stderr.contains(message),
            "expected `{message}` at index {index}:\n{stderr}"
        );
    }
}
