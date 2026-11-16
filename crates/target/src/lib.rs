//! Target triples and the properties the compiler derives from them.
//!
//! The compiler never consults the host at compile time to decide target
//! properties: everything comes from [`TargetSpec`]. That is what makes
//! cross-compilation deterministic and prevents `cfg!(target_os = ...)`-style
//! mistakes from leaking into the pipeline.

#![warn(missing_docs)]

use std::fmt;

/// Machine architecture.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Arch {
    /// `x86_64`
    X86_64,
    /// `aarch64`
    Aarch64,
    /// `x86`
    X86,
    /// `arm`
    Arm,
    /// `riscv64`
    Riscv64,
    /// `wasm32`
    Wasm32,
}

impl Arch {
    /// Returns the triple component.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::X86 => "x86",
            Self::Arm => "arm",
            Self::Riscv64 => "riscv64",
            Self::Wasm32 => "wasm32",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "x86_64" => Self::X86_64,
            "aarch64" => Self::Aarch64,
            "x86" | "i686" => Self::X86,
            "arm" | "armv7" => Self::Arm,
            "riscv64" => Self::Riscv64,
            "wasm32" => Self::Wasm32,
            _ => return None,
        })
    }
}

/// Operating system component.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Os {
    /// `linux`
    Linux,
    /// `darwin`
    Darwin,
    /// `windows`
    Windows,
    /// `freebsd`
    Freebsd,
    /// `none` (bare metal, wasm)
    None,
}

impl Os {
    /// Returns the triple component.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Darwin => "darwin",
            Self::Windows => "windows",
            Self::Freebsd => "freebsd",
            Self::None => "none",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "linux" => Self::Linux,
            "darwin" | "macos" => Self::Darwin,
            "windows" => Self::Windows,
            "freebsd" => Self::Freebsd,
            // WebAssembly and bare-metal triples spell the OS component
            // `unknown` or `none`; both mean "no operating system".
            "none" | "unknown" => Self::None,
            _ => return None,
        })
    }
}

/// Byte order.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Endianness {
    /// Least significant byte first.
    Little,
    /// Most significant byte first.
    Big,
}

/// A parsed target triple.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TargetTriple {
    text: String,
    arch: Arch,
    os: Os,
}

impl TargetTriple {
    /// Parses `arch-vendor-os[-env[-abi]]`.
    ///
    /// The vendor and trailing environment components are accepted and preserved
    /// in the text, but only architecture and OS influence compilation today.
    pub fn parse(text: &str) -> Result<Self, TargetError> {
        let mut components = text.split('-');
        let arch_text = components.next().unwrap_or_default();
        let arch = Arch::parse(arch_text).ok_or_else(|| TargetError::UnknownArch {
            arch: arch_text.to_owned(),
            triple: text.to_owned(),
        })?;
        let _vendor = components.next();
        let os_text = components.next().ok_or_else(|| TargetError::Malformed {
            triple: text.to_owned(),
        })?;
        let os = Os::parse(os_text).ok_or_else(|| TargetError::UnknownOs {
            os: os_text.to_owned(),
            triple: text.to_owned(),
        })?;
        if text.is_empty() {
            return Err(TargetError::Malformed {
                triple: text.to_owned(),
            });
        }
        Ok(Self {
            text: text.to_owned(),
            arch,
            os,
        })
    }

    /// Returns the architecture.
    #[inline]
    pub const fn arch(&self) -> Arch {
        self.arch
    }

    /// Returns the operating system.
    #[inline]
    pub const fn os(&self) -> Os {
        self.os
    }

    /// Returns the original triple text.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Returns the specification for this triple.
    pub fn spec(&self) -> TargetSpec {
        let pointer_width = match self.arch {
            Arch::X86_64 | Arch::Aarch64 | Arch::Riscv64 => 64,
            Arch::X86 | Arch::Arm | Arch::Wasm32 => 32,
        };
        TargetSpec {
            triple: self.clone(),
            pointer_width,
            endianness: Endianness::Little,
            native_i128: pointer_width == 64,
        }
    }
}

impl fmt::Display for TargetTriple {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

/// Properties of a compilation target.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TargetSpec {
    triple: TargetTriple,
    pointer_width: u8,
    endianness: Endianness,
    native_i128: bool,
}

impl TargetSpec {
    /// Returns the target triple.
    #[inline]
    pub fn triple(&self) -> &TargetTriple {
        &self.triple
    }

    /// Returns the pointer width in bits (`usize`/`isize` width).
    #[inline]
    pub const fn pointer_width(&self) -> u8 {
        self.pointer_width
    }

    /// Returns the byte order.
    #[inline]
    pub const fn endianness(&self) -> Endianness {
        self.endianness
    }

    /// Returns `true` when 128-bit integers are supported without a runtime
    /// helper.
    #[inline]
    pub const fn has_native_i128(&self) -> bool {
        self.native_i128
    }

    /// Returns `true` for 32-bit targets.
    #[inline]
    pub const fn is_32_bit(&self) -> bool {
        self.pointer_width == 32
    }
}

impl fmt::Display for TargetSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.triple.text)
    }
}

/// Failure to parse or resolve a target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetError {
    /// The triple is missing components.
    Malformed {
        /// The rejected triple.
        triple: String,
    },
    /// The architecture is not supported.
    UnknownArch {
        /// The unknown component.
        arch: String,
        /// The rejected triple.
        triple: String,
    },
    /// The operating system is not supported.
    UnknownOs {
        /// The unknown component.
        os: String,
        /// The rejected triple.
        triple: String,
    },
}

impl fmt::Display for TargetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed { triple } => {
                write!(
                    f,
                    "invalid target triple `{triple}`: expected `arch-vendor-os`"
                )
            }
            Self::UnknownArch { arch, triple } => {
                write!(f, "unknown architecture `{arch}` in target `{triple}`")
            }
            Self::UnknownOs { os, triple } => {
                write!(f, "unknown operating system `{os}` in target `{triple}`")
            }
        }
    }
}

impl std::error::Error for TargetError {}

/// The host triple, derived from the Rust toolchain that built the compiler.
///
/// This is only a default for `sealc` without `--target`; it is never consulted
/// while compiling a file.
pub fn host_triple() -> TargetTriple {
    let arch = match std::env::consts::ARCH {
        "x86_64" => Arch::X86_64,
        "aarch64" => Arch::Aarch64,
        "x86" => Arch::X86,
        "arm" => Arch::Arm,
        "riscv64" => Arch::Riscv64,
        "wasm32" => Arch::Wasm32,
        _ => Arch::X86_64,
    };
    let os = match std::env::consts::OS {
        "linux" => Os::Linux,
        "macos" => Os::Darwin,
        "windows" => Os::Windows,
        "freebsd" => Os::Freebsd,
        _ => Os::None,
    };
    let text = format!("{}-unknown-{}", arch.as_str(), os.as_str());
    TargetTriple { text, arch, os }
}

#[cfg(test)]
mod tests {
    use super::{Arch, Endianness, Os, TargetError, TargetSpec, TargetTriple, host_triple};

    #[test]
    fn parses_common_triples() {
        let linux = TargetTriple::parse("x86_64-unknown-linux-gnu").expect("valid");
        assert_eq!(linux.arch(), Arch::X86_64);
        assert_eq!(linux.os(), Os::Linux);
        assert_eq!(linux.to_string(), "x86_64-unknown-linux-gnu");

        let apple = TargetTriple::parse("aarch64-apple-darwin").expect("valid");
        assert_eq!(apple.os(), Os::Darwin);

        let wasm = TargetTriple::parse("wasm32-unknown-unknown").expect("valid");
        assert_eq!(wasm.arch(), Arch::Wasm32);
        assert_eq!(wasm.os(), Os::None);
    }

    #[test]
    fn rejects_malformed_triples() {
        assert!(matches!(
            TargetTriple::parse("x86_64"),
            Err(TargetError::Malformed { .. })
        ));
        assert!(matches!(
            TargetTriple::parse("mips-unknown-linux"),
            Err(TargetError::UnknownArch { .. })
        ));
        assert!(matches!(
            TargetTriple::parse("x86_64-unknown-plan9"),
            Err(TargetError::UnknownOs { .. })
        ));
    }

    #[test]
    fn pointer_width_follows_architecture() {
        let spec64 = TargetTriple::parse("x86_64-unknown-linux-gnu")
            .expect("valid")
            .spec();
        assert_eq!(spec64.pointer_width(), 64);
        assert!(spec64.has_native_i128());
        assert_eq!(spec64.endianness(), Endianness::Little);

        let spec32 = TargetTriple::parse("wasm32-unknown-unknown")
            .expect("valid")
            .spec();
        assert_eq!(spec32.pointer_width(), 32);
        assert!(spec32.is_32_bit());
        assert!(!spec32.has_native_i128());
    }

    #[test]
    fn target_spec_displays_the_triple() {
        let spec: TargetSpec = TargetTriple::parse("x86_64-unknown-linux-gnu")
            .expect("valid")
            .spec();
        assert_eq!(spec.to_string(), "x86_64-unknown-linux-gnu");
        assert_eq!(spec.triple().as_str(), "x86_64-unknown-linux-gnu");
    }

    #[test]
    fn host_triple_is_usable() {
        let host = host_triple();
        assert!(host.as_str().contains('-'));
        assert_eq!(host.spec().pointer_width(), 64);
    }
}
