//! Source loading and atomic artifact writing.
//!
//! All filesystem access from the compiler goes through this crate. Two rules
//! motivate it:
//!
//! - Source text must be valid UTF-8. Guessing an encoding would make byte
//!   offsets dependent on the host locale.
//! - Artifacts are written atomically (temporary file plus rename), so an
//!   interrupted or failed build never leaves a truncated output that looks
//!   valid.

#![warn(missing_docs)]

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Default maximum source file size (64 MiB).
pub const DEFAULT_MAX_FILE_SIZE: u64 = 64 * 1024 * 1024;

/// A source file loaded from disk.
#[derive(Debug)]
pub struct SourceText {
    path: PathBuf,
    text: String,
    had_bom: bool,
}

impl SourceText {
    /// Returns the path that was read.
    #[inline]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the file contents, including a BOM if present.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns `true` if the file started with a UTF-8 BOM.
    #[inline]
    pub fn had_bom(&self) -> bool {
        self.had_bom
    }

    /// Consumes the value and returns the text.
    #[inline]
    pub fn into_text(self) -> String {
        self.text
    }
}

/// Failure to load a source file.
#[derive(Debug)]
pub enum LoadError {
    /// The path does not exist.
    NotFound {
        /// The requested path.
        path: PathBuf,
    },
    /// The path is a directory.
    NotAFile {
        /// The requested path.
        path: PathBuf,
    },
    /// The file is larger than the configured limit.
    TooLarge {
        /// The requested path.
        path: PathBuf,
        /// Observed length.
        len: u64,
        /// Accepted maximum.
        max: u64,
    },
    /// The file is not valid UTF-8.
    NotUtf8 {
        /// The requested path.
        path: PathBuf,
        /// Byte offset of the first invalid sequence.
        valid_up_to: usize,
    },
    /// Any other I/O failure.
    Io {
        /// The requested path.
        path: PathBuf,
        /// The underlying error.
        source: io::Error,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "cannot read {}: no such file", path.display()),
            Self::NotAFile { path } => write!(f, "cannot read {}: is a directory", path.display()),
            Self::TooLarge { path, len, max } => write!(
                f,
                "cannot read {}: {len} bytes exceeds the {max}-byte limit",
                path.display()
            ),
            Self::NotUtf8 { path, valid_up_to } => write!(
                f,
                "cannot read {}: invalid UTF-8 at byte {valid_up_to}",
                path.display()
            ),
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Reads a UTF-8 source file.
pub fn load(path: &Path, max_bytes: u64) -> Result<SourceText, LoadError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(LoadError::NotFound {
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(LoadError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.is_dir() {
        return Err(LoadError::NotAFile {
            path: path.to_path_buf(),
        });
    }
    if metadata.len() > max_bytes {
        return Err(LoadError::TooLarge {
            path: path.to_path_buf(),
            len: metadata.len(),
            max: max_bytes,
        });
    }
    let bytes = fs::read(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let text = String::from_utf8(bytes).map_err(|error| LoadError::NotUtf8 {
        path: path.to_path_buf(),
        valid_up_to: error.utf8_error().valid_up_to(),
    })?;
    let had_bom = text.starts_with('\u{feff}');
    Ok(SourceText {
        path: path.to_path_buf(),
        text,
        had_bom,
    })
}

/// Moves `source` over `destination`, replacing it.
///
/// `rename` replaces the destination atomically on Unix. Windows refuses to
/// rename onto an existing file, so the destination is removed first: a reader
/// still observes either the previous file or the new one, never a partial
/// write.
pub fn replace(source: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            fs::remove_file(destination)?;
            fs::rename(source, destination)
        }
        Err(error) => Err(error),
    }
}

/// Writes `bytes` to `path` atomically.
///
/// The data is written to a uniquely named temporary file in the same directory
/// and then renamed over the destination. On any failure the temporary file is
/// removed; the destination is never left partially written.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let directory = parent.unwrap_or_else(|| Path::new("."));
    let file_name = path.file_name().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "output path has no file name")
    })?;

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = directory.join(format!(
        ".{}.{}.{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        unique
    ));

    let result = (|| -> io::Result<()> {
        let mut file = File::create(&temp_path)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        replace(&temp_path, path)
    })();

    if result.is_err() {
        // Best effort: the primary error is the one worth reporting.
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{DEFAULT_MAX_FILE_SIZE, LoadError, load, replace, write_atomic};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let unique = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("seal-fs-test-{}-{unique}", std::process::id()));
            fs::create_dir_all(&path).expect("temp dir is created");
            Self { path }
        }

        fn file(&self, name: &str, contents: &[u8]) -> PathBuf {
            let path = self.path.join(name);
            fs::write(&path, contents).expect("test file is written");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn loads_utf8_text() {
        let dir = TempDir::new();
        let path = dir.file("main.sl", b"func main() -> i32 {\n  0\n}\n");
        let source = load(&path, DEFAULT_MAX_FILE_SIZE).expect("file loads");
        assert_eq!(source.path(), path);
        assert!(source.text().contains("func main"));
        assert!(!source.had_bom());
    }

    #[test]
    fn detects_bom() {
        let dir = TempDir::new();
        let path = dir.file("bom.sl", b"\xef\xbb\xbffunc main() {}");
        let source = load(&path, DEFAULT_MAX_FILE_SIZE).expect("file loads");
        assert!(source.had_bom());
        assert!(source.text().starts_with('\u{feff}'));
    }

    #[test]
    fn rejects_invalid_utf8() {
        let dir = TempDir::new();
        let path = dir.file("bad.sl", b"abc\xff");
        let error = load(&path, DEFAULT_MAX_FILE_SIZE).expect_err("invalid UTF-8 is rejected");
        match error {
            LoadError::NotUtf8 { valid_up_to, .. } => assert_eq!(valid_up_to, 3),
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn rejects_directories_and_missing_files() {
        let dir = TempDir::new();
        assert!(matches!(
            load(&dir.path, DEFAULT_MAX_FILE_SIZE),
            Err(LoadError::NotAFile { .. })
        ));
        assert!(matches!(
            load(&dir.path.join("missing.sl"), DEFAULT_MAX_FILE_SIZE),
            Err(LoadError::NotFound { .. })
        ));
    }

    #[test]
    fn rejects_oversized_files() {
        let dir = TempDir::new();
        let path = dir.file("big.sl", b"0123456789");
        let error = load(&path, 4).expect_err("oversized file is rejected");
        match error {
            LoadError::TooLarge { len, max, .. } => {
                assert_eq!(len, 10);
                assert_eq!(max, 4);
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[test]
    fn atomic_write_replaces_content_and_cleans_up() {
        let dir = TempDir::new();
        let path = dir.file("out.txt", b"old");
        write_atomic(&path, b"new contents").expect("write succeeds");
        assert_eq!(fs::read(&path).expect("file exists"), b"new contents");
        let entries: Vec<_> = fs::read_dir(&dir.path)
            .expect("directory readable")
            .map(|entry| entry.expect("entry readable").file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("out.txt")]);
    }

    #[test]
    fn atomic_write_to_missing_directory_fails_without_artifacts() {
        let dir = TempDir::new();
        let path = dir.path.join("missing").join("out.txt");
        assert!(write_atomic(&path, b"data").is_err());
        assert!(!path.exists());
    }

    #[test]
    fn replace_moves_the_source_over_the_destination() {
        let dir = TempDir::new();
        let source = dir.file("new", b"new bytes");
        let destination = dir.file("out", b"old bytes");
        replace(&source, &destination).expect("the replacement succeeds");
        assert!(!source.exists(), "the source is gone after the replace");
        assert_eq!(
            fs::read(&destination).expect("destination readable"),
            b"new bytes"
        );
    }

    #[test]
    fn replace_into_a_missing_directory_keeps_the_source() {
        let dir = TempDir::new();
        let source = dir.file("artifact", b"bytes");
        let destination = dir.path.join("missing").join("artifact");
        assert!(replace(&source, &destination).is_err());
        assert_eq!(fs::read(&source).expect("source readable"), b"bytes");
        assert!(!destination.exists());
    }

    #[test]
    fn error_messages_include_the_path() {
        let dir = TempDir::new();
        let missing = dir.path.join("nope.sl");
        let error = load(&missing, DEFAULT_MAX_FILE_SIZE).expect_err("missing file");
        assert!(error.to_string().contains("nope.sl"));
    }
}
