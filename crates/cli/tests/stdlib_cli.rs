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
fn mem_forget_runs_and_rejects_use_after_consume() {
    let fixture = copy_example("stdlib");
    let source = std::fs::read_to_string(examples().join("054_mem_forget.sl"))
        .expect("the forget example is readable");
    write(&fixture.root.join("src/main.sl"), &source);

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

    write(
        &fixture.root.join("src/main.sl"),
        "struct Pair { value: i32 }\n\
         func main() {\n\
           let pair = Pair { value: 1 };\n\
           std::mem::forget(pair);\n\
           let _ = pair.value;\n\
         }\n",
    );
    let rejected = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_eq!(rejected.status.code(), Some(1));
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(
        diagnostics.contains("E5005"),
        "a use after forget must be rejected:\n{diagnostics}"
    );
}

fn assert_forget_rejected(fixture: &Fixture, source: &str, code: &str) -> String {
    write(&fixture.root.join("src/main.sl"), source);
    let rejected = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_eq!(rejected.status.code(), Some(1));
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(
        diagnostics.contains(code),
        "expected {code} for this source:\n{diagnostics}"
    );
    diagnostics
}

#[test]
fn mem_forget_rejects_invalid_ownership_cases() {
    let fixture = copy_example("stdlib");

    assert_forget_rejected(
        &fixture,
        "struct Pair { value: i32 }\n\
         func main() {\n\
           let pair = Pair { value: 1 };\n\
           let mut iteration = 0;\n\
           while iteration < 1 {\n\
             std::mem::forget(pair);\n\
             iteration += 1;\n\
           }\n\
           let _ = pair.value;\n\
         }\n",
        "E5005",
    );

    assert_forget_rejected(
        &fixture,
        "struct Inner { value: i32 }\n\
         struct Pair { inner: Inner, count: i32 }\n\
         func main() {\n\
           let pair = Pair { inner: Inner { value: 1 }, count: 2 };\n\
           std::mem::forget(pair.inner);\n\
           std::mem::forget(pair);\n\
         }\n",
        "E5005",
    );

    let duplicate = assert_forget_rejected(
        &fixture,
        "struct Pair { value: i32 }\n\
         func main() {\n\
           let pair = Pair { value: 1 };\n\
           std::mem::forget(pair);\n\
           std::mem::forget(pair);\n\
         }\n",
        "E5005",
    );
    assert_eq!(
        duplicate.matches("E5005").count(),
        1,
        "a second forget of a moved value must report one clear diagnostic:\n{duplicate}"
    );

    assert_forget_rejected(
        &fixture,
        "struct Pair { value: i32 }\n\
         func main() {\n\
           let pair = Pair { value: 1 };\n\
           let view = &pair;\n\
           std::mem::forget(pair);\n\
           let _ = (*view).value;\n\
         }\n",
        "E5002",
    );

    assert_forget_rejected(
        &fixture,
        "struct Pair { value: i32 }\n\
         func main() {\n\
           let mut pair = Pair { value: 1 };\n\
           let view = &mut pair;\n\
           std::mem::forget(pair);\n\
           (*view).value = 2;\n\
         }\n",
        "E5008",
    );

    assert_forget_rejected(
        &fixture,
        "struct Inner { value: i32 }\n\
         struct Pair { inner: Inner }\n\
         impl Drop for Pair {\n\
           func drop(self: *mut Pair) {}\n\
         }\n\
         func main() {\n\
           let pair = Pair { inner: Inner { value: 1 } };\n\
           std::mem::forget(pair.inner);\n\
         }\n",
        "E5014",
    );

    assert_forget_rejected(
        &fixture,
        "func main() {\n\
           let value = 1;\n\
           __forget(value);\n\
         }\n",
        "E2002",
    );
}

#[test]
fn mem_forget_is_not_available_in_a_constant_context() {
    let fixture = copy_example("stdlib");
    write(
        &fixture.root.join("src/main.sl"),
        "struct Pair { value: i32 }\n\
         const FORGOTTEN: () = std::mem::forget(Pair { value: 1 });\n\
         func main() -> i32 { 0 }\n",
    );
    let rejected = output(seal().arg("check").arg("--project").arg(&fixture.root));
    assert_eq!(rejected.status.code(), Some(1));
    let diagnostics = format!(
        "{}{}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
    assert!(
        diagnostics.contains("E4011"),
        "a constant context cannot call forget:\n{diagnostics}"
    );
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
