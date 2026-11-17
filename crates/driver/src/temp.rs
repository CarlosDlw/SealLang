//! Temporary directories owned by the `run` command.
//!
//! The directory is created before the build and removed when the guard is
//! dropped, so failed builds and failed executions leave no artifacts behind.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};

/// A unique temporary directory that removes itself on drop.
#[derive(Debug)]
pub(crate) struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    /// Creates a unique directory below the system temporary directory.
    pub(crate) fn new() -> Result<Self, String> {
        let parent = std::env::temp_dir();
        for _ in 0..64 {
            let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = parent.join(format!("seal-run-{}-{sequence}", process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "cannot create temporary directory `{}`: {error}",
                        path.display()
                    ));
                }
            }
        }
        Err(format!(
            "cannot create a unique temporary directory below `{}`",
            parent.display()
        ))
    }

    /// Returns the path of the executable built inside this directory.
    pub(crate) fn executable(&self) -> PathBuf {
        self.path.join(if cfg!(windows) {
            "program.exe"
        } else {
            "program"
        })
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!(
                "warning: cannot remove temporary directory `{}`: {error}",
                self.path.display()
            );
        }
    }
}

/// Process-local sequence that makes names unique across simultaneous runs.
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_directories_are_unique() {
        let first = TemporaryDirectory::new().expect("first temporary directory");
        let second = TemporaryDirectory::new().expect("second temporary directory");

        assert_ne!(first.path, second.path);
        assert!(first.path.is_dir());
        assert!(second.path.is_dir());
    }

    #[test]
    fn dropping_a_temporary_directory_removes_it() {
        let directory = TemporaryDirectory::new().expect("temporary directory");
        let path = directory.path.clone();
        fs::write(path.join("artifact"), b"contents").expect("artifact");

        drop(directory);

        assert!(!path.exists());
    }

    #[test]
    fn executable_is_inside_the_temporary_directory() {
        let directory = TemporaryDirectory::new().expect("temporary directory");
        let executable = directory.executable();

        assert_eq!(executable.parent(), Some(directory.path.as_path()));
        #[cfg(windows)]
        assert_eq!(
            executable.extension().and_then(|value| value.to_str()),
            Some("exe")
        );
        #[cfg(not(windows))]
        assert_eq!(executable.extension(), None);
    }
}
