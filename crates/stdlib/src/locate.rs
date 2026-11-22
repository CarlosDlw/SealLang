//! Finding the installed standard library.

use std::path::{Path, PathBuf};

use crate::error::{StdlibError, StdlibResult};

/// Environment variable that selects an explicit standard-library root.
pub const ENV_VAR: &str = "SEAL_STDLIB";

/// Locates the standard-library root without loading it.
///
/// `SEAL_STDLIB` is authoritative: when it is set to an invalid root, the
/// loader fails instead of silently falling back to another installation.
pub(crate) fn locate() -> StdlibResult<PathBuf> {
    locate_with(
        std::env::var_os(ENV_VAR).map(PathBuf::from),
        std::env::current_exe().ok(),
    )
}

fn locate_with(explicit: Option<PathBuf>, executable: Option<PathBuf>) -> StdlibResult<PathBuf> {
    if let Some(root) = explicit {
        if !is_stdlib_root(&root) {
            return Err(StdlibError::locate(format!(
                "`{ENV_VAR}` names `{}`, which is not a standard-library root",
                root.display()
            )));
        }
        return root.canonicalize().map_err(|error| {
            StdlibError::locate(format!(
                "cannot resolve `{ENV_VAR}` root `{}`: {error}",
                root.display()
            ))
        });
    }

    for candidate in installed_candidates(executable) {
        if is_stdlib_root(&candidate) {
            return candidate.canonicalize().map_err(|error| {
                StdlibError::locate(format!(
                    "cannot resolve the installed standard library `{}`: {error}",
                    candidate.display()
                ))
            });
        }
    }
    for candidate in development_candidates() {
        if is_stdlib_root(&candidate) {
            return candidate.canonicalize().map_err(|error| {
                StdlibError::locate(format!(
                    "cannot resolve the development standard library `{}`: {error}",
                    candidate.display()
                ))
            });
        }
    }
    Err(StdlibError::locate(
        "cannot find the Seal standard library; set `SEAL_STDLIB` or install the standard library",
    ))
}

fn is_stdlib_root(path: &Path) -> bool {
    path.join("stdlib.toml").is_file()
}

fn installed_candidates(executable: Option<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let Some(executable) = executable else {
        return candidates;
    };
    let executable = executable
        .canonicalize()
        .unwrap_or_else(|_| executable.clone());
    let Some(bin) = executable.parent() else {
        return candidates;
    };
    let Some(prefix) = bin.parent() else {
        return candidates;
    };
    let language = seal_version::LanguageVersion::CURRENT.to_string();
    for base in [prefix.join("lib"), prefix.join("share")] {
        candidates.push(base.join("seal").join("stdlib").join(&language));
        candidates.push(base.join("seal-stdlib").join(&language));
        candidates.push(base.join("seal").join("stdlib"));
        candidates.push(base.join("seal-stdlib"));
        candidates.push(base.join("stdlib"));
    }
    candidates
}

fn development_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    // The workspace tree keeps the standard library next to `crates/`. This
    // path is compiled in only as a last-resort development fallback; installed
    // binaries find their sysroot through `../lib` or `../share` first.
    candidates.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../stdlib"));
    if let Ok(current) = std::env::current_dir() {
        for ancestor in current.ancestors() {
            candidates.push(ancestor.join("stdlib"));
        }
    }
    candidates
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{ENV_VAR, locate_with};

    struct Fixture(PathBuf);

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(name: &str) -> Fixture {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is after the epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "seal-stdlib-locate-{}-{nonce}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("the fixture root is writable");
        Fixture(root)
    }

    fn descriptor(root: &Path) {
        std::fs::write(root.join("stdlib.toml"), "format = 1\n").expect("descriptor");
    }

    #[test]
    fn an_explicit_root_wins() {
        let fixture = fixture("explicit");
        descriptor(&fixture.0);
        let found = locate_with(Some(fixture.0.clone()), None).expect("the explicit root wins");
        assert_eq!(found, fixture.0.canonicalize().expect("canonical root"));
    }

    #[test]
    fn an_invalid_explicit_root_never_falls_back() {
        let fixture = fixture("invalid");
        let missing = fixture.0.join("missing");
        let error = locate_with(Some(missing), None).expect_err("the invalid root fails");
        assert_eq!(error.code(), "E8133");
        assert!(error.message().contains(ENV_VAR));
    }

    #[test]
    fn the_installed_lib_layout_is_found() {
        let fixture = fixture("lib");
        let bin = fixture.0.join("bin");
        std::fs::create_dir_all(&bin).expect("bin");
        let executable = bin.join("seal");
        std::fs::write(&executable, "").expect("executable");
        let root = fixture.0.join("lib/seal/stdlib/0.1");
        std::fs::create_dir_all(&root).expect("stdlib");
        descriptor(&root);
        let found = locate_with(None, Some(executable)).expect("the installed layout is found");
        assert_eq!(found, root.canonicalize().expect("canonical root"));
    }

    #[test]
    fn the_development_tree_is_the_last_resort() {
        let found = locate_with(None, None).expect("the workspace stdlib is found");
        assert!(found.join("stdlib.toml").is_file());
    }

    #[test]
    fn the_installed_share_layout_is_found() {
        let fixture = fixture("share");
        let bin = fixture.0.join("bin");
        std::fs::create_dir_all(&bin).expect("bin");
        let executable = bin.join("seal");
        std::fs::write(&executable, "").expect("executable");
        let root = fixture.0.join("share/seal/stdlib/0.1");
        std::fs::create_dir_all(&root).expect("stdlib");
        descriptor(&root);
        let found = locate_with(None, Some(executable)).expect("the share layout is found");
        assert_eq!(found, root.canonicalize().expect("canonical root"));
    }
}
