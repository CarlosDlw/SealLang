//! Descriptor, digest, and repository standard-library tests.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use seal_stdlib::Stdlib;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .canonicalize()
        .expect("the repository root resolves")
}

fn temporary(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after the epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("seal-stdlib-{}-{nonce}-{name}", std::process::id()))
}

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("the fixture directory is writable");
    }
    std::fs::write(path, contents).expect("the fixture file is writable");
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("the destination is writable");
    for entry in std::fs::read_dir(from).expect("the source is readable") {
        let entry = entry.expect("the directory entry is readable");
        let destination = to.join(entry.file_name());
        if entry
            .file_type()
            .expect("the file type is readable")
            .is_dir()
        {
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), &destination).expect("the source is copied");
        }
    }
}

/// Writes a minimal two-package standard library with a placeholder digest.
fn fixture(root: &Path, digest: &str) {
    write(
        root,
        "stdlib.toml",
        &format!(
            "format = 1\nlanguage = \"0.1\"\nversion = \"1.2.3\"\npackages = [\"core\", \"std\"]\ndigest = \"{digest}\"\n\n[prelude]\nstd = \"std::prelude\"\ncore = \"core::prelude\"\n"
        ),
    );
    write(
        root,
        "core/seal.toml",
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\nstdlib = \"none\"\nimplicit-prelude = false\nsources = [\"src/lib.sl\", \"src/prelude.sl\"]\n\n[package.api]\nexpose = [\"root\", \"prelude\"]\n",
    );
    write(root, "core/src/lib.sl", "// core\n");
    write(root, "core/src/prelude.sl", "module prelude;\n");
    write(
        root,
        "std/seal.toml",
        "[package]\nname = \"std\"\nversion = \"0.1.0\"\nstdlib = \"none\"\nimplicit-prelude = false\nsources = [\"src/lib.sl\", \"src/prelude.sl\"]\n\n[dependencies]\ncore = { path = \"../core\" }\n\n[package.api]\nexpose = [\"root\", \"prelude\"]\n",
    );
    write(root, "std/src/lib.sl", "// std\n");
    write(root, "std/src/prelude.sl", "module prelude;\n");
}

#[test]
fn the_repository_standard_library_is_valid() {
    let stdlib = Stdlib::load_from(&repository_root().join("stdlib"))
        .expect("the repository stdlib is valid");
    assert_eq!(stdlib.language(), "0.1");
    assert_eq!(stdlib.version(), "0.1.0");
    assert_eq!(
        stdlib
            .packages()
            .iter()
            .map(|package| package.name.as_str())
            .collect::<Vec<_>>(),
        ["core", "std"]
    );
    assert_eq!(stdlib.prelude_for("std"), Some("std::prelude"));
}

#[test]
fn copying_the_tree_preserves_the_digest() {
    let source = repository_root().join("stdlib");
    let original = Stdlib::load_from(&source).expect("the repository stdlib is valid");
    let root = temporary("copied");
    copy_tree(&source, &root);
    let copied = Stdlib::load_from(&root).expect("the copied stdlib is valid");
    assert_eq!(copied.digest(), original.digest());
    assert_eq!(
        copied.root(),
        root.canonicalize().expect("the root resolves")
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_missing_source_is_rejected() {
    let root = temporary("missing");
    fixture(&root, &"0".repeat(64));
    let error = Stdlib::load_from(&root).expect_err("the placeholder digest is rejected");
    let computed = error
        .message()
        .split_whitespace()
        .find(|word| word.len() == 64 && word.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .expect("the error names the computed digest")
        .to_owned();
    fixture(&root, &computed);
    std::fs::remove_file(root.join("core/src/prelude.sl")).expect("the source is removable");
    let error = Stdlib::load_from(&root).expect_err("a listed source is required");
    assert_eq!(error.code(), "E8139");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_matching_digest_loads_and_verifies() {
    let root = temporary("matching");
    fixture(&root, &"0".repeat(64));
    let error = Stdlib::load_from(&root).expect_err("the placeholder digest is rejected");
    let computed = error
        .message()
        .split_whitespace()
        .find(|word| word.len() == 64 && word.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .expect("the error names the computed digest")
        .to_owned();
    fixture(&root, &computed);
    let stdlib = Stdlib::load_from(&root).expect("the computed digest matches");
    stdlib.verify().expect("verify accepts the same tree");
    assert_eq!(stdlib.digest(), computed);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_changed_source_changes_the_digest_and_fails_verification() {
    let root = temporary("changed");
    fixture(&root, &"0".repeat(64));
    let error = Stdlib::load_from(&root).expect_err("the placeholder digest is rejected");
    let computed = error
        .message()
        .split_whitespace()
        .find(|word| word.len() == 64 && word.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .expect("the error names the computed digest")
        .to_owned();
    fixture(&root, &computed);
    let stdlib = Stdlib::load_from(&root).expect("the computed digest matches");
    write(&root, "core/src/lib.sl", "// edited\n");
    let error = stdlib.verify().expect_err("the edit changes the digest");
    assert_eq!(error.code(), "E8140");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn an_unknown_descriptor_field_is_rejected() {
    let root = temporary("unknown-field");
    fixture(&root, &"0".repeat(64));
    write(
        &root,
        "stdlib.toml",
        "format = 1\nlanguage = \"0.1\"\nversion = \"1.2.3\"\npackages = [\"core\"]\ndigest = \"0000000000000000000000000000000000000000000000000000000000000000\"\nunknown = true\n",
    );
    let error = Stdlib::load_from(&root).expect_err("unknown fields are rejected");
    assert_eq!(error.code(), "E8135");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_language_mismatch_is_reported_separately() {
    let root = temporary("language");
    fixture(&root, &"0".repeat(64));
    write(
        &root,
        "stdlib.toml",
        "format = 1\nlanguage = \"9.9\"\nversion = \"1.2.3\"\npackages = [\"core\"]\ndigest = \"0000000000000000000000000000000000000000000000000000000000000000\"\n",
    );
    let error = Stdlib::load_from(&root).expect_err("the language is rejected");
    assert_eq!(error.code(), "E8134");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_non_regular_source_is_rejected() {
    let root = temporary("non-regular");
    fixture(&root, &"0".repeat(64));
    std::fs::remove_file(root.join("core/src/prelude.sl")).expect("the source is removable");
    std::fs::create_dir_all(root.join("core/src/prelude.sl"))
        .expect("the fixture directory is writable");
    let error = Stdlib::load_from(&root).expect_err("a directory is not a source file");
    assert_eq!(error.code(), "E8139");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_symlinked_source_is_rejected() {
    let root = temporary("symlink");
    fixture(&root, &"0".repeat(64));
    let source = root.join("core/src/lib.sl");
    let target = root.join("outside.sl");
    std::fs::write(&target, "// outside\n").expect("the target is writable");
    std::fs::remove_file(&source).expect("the source is removable");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &source).expect("the symlink is created");
    #[cfg(not(unix))]
    std::fs::copy(&target, &source).expect("the fallback copy is written");
    #[cfg(unix)]
    {
        let error = Stdlib::load_from(&root).expect_err("symlinked sources are rejected");
        assert_eq!(error.code(), "E8139");
    }
    let _ = std::fs::remove_dir_all(root);
}
