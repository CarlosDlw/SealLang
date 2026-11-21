//! Failures to read, verify, install, or select a toolchain.

use std::fmt;
use std::path::PathBuf;

/// A toolchain operation failure.
#[derive(Debug)]
pub enum ToolchainError {
    /// An input or output could not be read or written.
    Io {
        /// What was being done.
        context: String,
        /// The underlying error.
        source: std::io::Error,
    },
    /// `seal-toolchain.toml` is malformed or incomplete.
    Manifest {
        /// The reason, ready for the diagnostic.
        detail: String,
    },
    /// The declared checksum is not a SHA-256 digest.
    InvalidChecksum {
        /// The rejected value.
        value: String,
    },
    /// The bundle or installed tree does not match its declared checksum.
    ChecksumMismatch {
        /// The checksum from the manifest.
        expected: String,
        /// The checksum of the bytes on disk.
        actual: String,
        /// What was verified.
        subject: String,
    },
    /// A component path is missing, not a regular file, or leaves the tree.
    Component {
        /// The component name.
        name: String,
        /// The declared path.
        path: PathBuf,
        /// The reason.
        detail: String,
    },
    /// A required or supplied Ed25519 signature is invalid.
    Signature {
        /// The reason, ready for the diagnostic.
        detail: String,
    },
    /// A tree entry is a symlink or another unsupported file type.
    UnsafeEntry {
        /// The rejected path.
        path: PathBuf,
    },
    /// The requested version is not installed.
    NotInstalled {
        /// The requested version.
        version: String,
    },
    /// The pinned version differs from the compiler in this binary.
    Incompatible {
        /// The version in `seal-toolchain.toml`.
        pinned: String,
        /// The embedded compiler version.
        current: String,
    },
    /// The project pins a toolchain that does not declare the requested target.
    TargetNotDeclared {
        /// The requested triple.
        triple: String,
        /// The pinned version.
        version: String,
    },
    /// The project pin and the installed toolchain describe different trees.
    PinMismatch {
        /// The reason, ready for the diagnostic.
        detail: String,
    },
    /// No local bundle exists for an install, so an offline install cannot run.
    NoOfflineBundle {
        /// The requested version.
        version: String,
    },
    /// A version string is not a semantic version.
    Version {
        /// The rejected text.
        value: String,
        /// The reason.
        detail: String,
    },
}

impl ToolchainError {
    /// Creates an I/O error with operation context.
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }

    /// Creates a manifest error.
    pub(crate) fn manifest(detail: impl Into<String>) -> Self {
        Self::Manifest {
            detail: detail.into(),
        }
    }

    /// Returns the diagnostic code for the CLI.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::TargetNotDeclared { .. } => "E8060",
            Self::Incompatible { .. }
            | Self::InvalidChecksum { .. }
            | Self::ChecksumMismatch { .. }
            | Self::Signature { .. }
            | Self::UnsafeEntry { .. }
            | Self::NoOfflineBundle { .. }
            | Self::Component { .. }
            | Self::Manifest { .. }
            | Self::Version { .. }
            | Self::PinMismatch { .. } => "E8061",
            Self::Io { .. } | Self::NotInstalled { .. } => "E8011",
        }
    }
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { context, source } => write!(f, "{context}: {source}"),
            Self::Manifest { detail } => f.write_str(detail),
            Self::InvalidChecksum { value } => write!(
                f,
                "the toolchain checksum `{value}` is not a SHA-256 digest (64 hexadecimal characters)"
            ),
            Self::ChecksumMismatch {
                expected,
                actual,
                subject,
            } => write!(
                f,
                "the toolchain checksum for {subject} does not match: expected {expected}, found {actual}; the toolchain was not installed"
            ),
            Self::Signature { detail } => f.write_str(detail),
            Self::Component { name, path, detail } => {
                write!(
                    f,
                    "the toolchain component `{name}` at `{}` is invalid: {detail}",
                    path.display()
                )
            }
            Self::UnsafeEntry { path } => write!(
                f,
                "the toolchain contains the unsupported entry `{}`; only regular files and directories are allowed",
                path.display()
            ),
            Self::NotInstalled { version } => {
                write!(f, "the toolchain `{version}` is not installed")
            }
            Self::Incompatible { pinned, current } => write!(
                f,
                "the project pins toolchain `{pinned}`, but this `seal` binary embeds compiler `{current}`; install and run the pinned toolchain"
            ),
            Self::TargetNotDeclared { triple, version } => write!(
                f,
                "the pinned toolchain `{version}` does not declare the target `{triple}`"
            ),
            Self::PinMismatch { detail } => f.write_str(detail),
            Self::NoOfflineBundle { version } => write!(
                f,
                "no offline bundle for toolchain `{version}` was provided; pass `--from <DIR>` or install it before building offline"
            ),
            Self::Version { value, detail } => {
                write!(f, "invalid toolchain version `{value}`: {detail}")
            }
        }
    }
}

impl std::error::Error for ToolchainError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
