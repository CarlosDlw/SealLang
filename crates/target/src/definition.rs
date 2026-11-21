//! TOML target specifications.
//!
//! A project can declare a target whose properties differ from the built-in
//! defaults for its triple. The TOML form is intentionally explicit: every
//! property the compiler uses is named, so a target file can be reviewed and
//! checked without consulting the host.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::c_abi::CAbiProperties;
use crate::runtime::{PanicStrategy, RuntimePolicy};
use crate::{Endianness, TargetError, TargetSpec, TargetTriple};

/// One `[target]` specification file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct TargetDefinition {
    /// The target triple.
    pub triple: String,
    /// The LLVM data layout, when it differs from the triple default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_layout: Option<String>,
    /// Pointer width in bits (`32` or `64`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer_width: Option<u8>,
    /// Byte order (`little` or `big`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endianness: Option<String>,
    /// Explicit C ABI scalar properties.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub c_abi: Option<CAbiDefinition>,
    /// Runtime policy (`default` or `none`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// Panic policy (`abort` or `trap`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panic: Option<String>,
    /// The linker program used for executable targets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linker: Option<String>,
    /// The sysroot passed to the C toolchain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sysroot: Option<PathBuf>,
    /// The linker script passed to the C toolchain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linker_script: Option<PathBuf>,
}

/// Explicit C ABI properties in a target specification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct CAbiDefinition {
    /// Whether plain `char` is signed.
    pub char_signed: bool,
    /// Width of `long` in bits (`32` or `64`).
    pub long_width: u16,
    /// Width of `wchar_t` in bits (`16` or `32`).
    pub wchar_width: u16,
    /// Whether `wchar_t` is signed.
    pub wchar_signed: bool,
}

impl TargetDefinition {
    /// Parses a specification from TOML text.
    pub fn parse(text: &str) -> Result<Self, TargetError> {
        toml::from_str(text).map_err(|error| TargetError::InvalidSpec {
            detail: format!("cannot parse the target specification: {error}"),
        })
    }

    /// Resolves the definition into a validated [`TargetSpec`].
    pub fn resolve(self) -> Result<TargetSpec, TargetError> {
        let triple = TargetTriple::parse(&self.triple)?;
        let builtin = triple.spec();
        let pointer_width = self.pointer_width.unwrap_or(builtin.pointer_width());
        if !matches!(pointer_width, 32 | 64) {
            return Err(TargetError::InvalidSpec {
                detail: format!(
                    "the target `{}` declares pointer width {pointer_width}; only 32 and 64 are supported",
                    self.triple
                ),
            });
        }
        let endianness = match self.endianness.as_deref() {
            None => builtin.endianness(),
            Some("little") => Endianness::Little,
            Some("big") => Endianness::Big,
            Some(other) => {
                return Err(TargetError::InvalidSpec {
                    detail: format!(
                        "the target `{}` declares the unknown byte order `{other}`",
                        self.triple
                    ),
                });
            }
        };
        let base_abi = CAbiProperties::for_target_parts(&triple, pointer_width);
        let c_abi = match self.c_abi {
            None => base_abi,
            Some(abi) => {
                if !matches!(abi.long_width, 32 | 64) {
                    return Err(TargetError::InvalidSpec {
                        detail: format!(
                            "the target `{}` declares a {}-bit `long`; only 32 and 64 are supported",
                            self.triple, abi.long_width
                        ),
                    });
                }
                if !matches!(abi.wchar_width, 16 | 32) {
                    return Err(TargetError::InvalidSpec {
                        detail: format!(
                            "the target `{}` declares a {}-bit `wchar_t`; only 16 and 32 are supported",
                            self.triple, abi.wchar_width
                        ),
                    });
                }
                CAbiProperties::new(
                    abi.char_signed,
                    abi.long_width,
                    abi.wchar_width,
                    abi.wchar_signed,
                )
            }
        };
        let runtime = match self.runtime.as_deref() {
            None => builtin.runtime(),
            Some(value) => RuntimePolicy::parse(value).ok_or_else(|| TargetError::InvalidSpec {
                detail: format!(
                    "the target `{}` declares the unknown runtime policy `{value}`; expected `default` or `none`",
                    self.triple
                ),
            })?,
        };
        let panic = match self.panic.as_deref() {
            None if runtime == RuntimePolicy::None => PanicStrategy::Trap,
            None => builtin.panic(),
            Some(value) => {
                PanicStrategy::parse(value).ok_or_else(|| TargetError::InvalidSpec {
                    detail: format!(
                        "the target `{}` declares the unknown panic policy `{value}`; expected `abort` or `trap`",
                        self.triple
                    ),
                })?
            }
        };
        if runtime == RuntimePolicy::None && panic != PanicStrategy::Trap {
            return Err(TargetError::InvalidSpec {
                detail: format!(
                    "the freestanding target `{}` must use `panic = \"trap\"`; there is no runtime to abort through",
                    self.triple
                ),
            });
        }
        if self
            .data_layout
            .as_ref()
            .is_some_and(|layout| layout.trim().is_empty())
        {
            return Err(TargetError::InvalidSpec {
                detail: format!("the target `{}` declares an empty data layout", self.triple),
            });
        }
        if self
            .linker
            .as_ref()
            .is_some_and(|linker| linker.trim().is_empty())
        {
            return Err(TargetError::InvalidSpec {
                detail: format!("the target `{}` declares an empty linker", self.triple),
            });
        }
        Ok(TargetSpec::from_parts(
            triple,
            pointer_width,
            endianness,
            pointer_width == 64,
            self.data_layout,
            c_abi,
            runtime,
            panic,
            self.linker,
            self.sysroot,
            self.linker_script,
        ))
    }
}

impl TargetSpec {
    /// Parses and resolves a TOML target specification.
    pub fn parse_toml(text: &str) -> Result<Self, TargetError> {
        TargetDefinition::parse(text)?.resolve()
    }

    /// Renders this specification as TOML.
    pub fn to_toml(&self) -> Result<String, TargetError> {
        let definition = TargetDefinition {
            triple: self.triple().as_str().to_owned(),
            data_layout: self.data_layout().map(ToOwned::to_owned),
            pointer_width: Some(self.pointer_width()),
            endianness: Some(
                match self.endianness() {
                    Endianness::Little => "little",
                    Endianness::Big => "big",
                }
                .to_owned(),
            ),
            c_abi: Some(CAbiDefinition {
                char_signed: self.c_abi().char_signed(),
                long_width: self.c_abi().long_width(),
                wchar_width: self.c_abi().wchar_width(),
                wchar_signed: self.c_abi().wchar_signed(),
            }),
            runtime: Some(self.runtime().as_str().to_owned()),
            panic: Some(self.panic().as_str().to_owned()),
            linker: self.linker().map(ToOwned::to_owned),
            sysroot: self.sysroot().map(PathBuf::from),
            linker_script: self.linker_script().map(PathBuf::from),
        };
        toml::to_string_pretty(&definition).map_err(|error| TargetError::InvalidSpec {
            detail: format!("cannot serialize the target specification: {error}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Endianness, PanicStrategy, RuntimePolicy};

    use super::TargetDefinition;

    #[test]
    fn a_minimal_definition_uses_the_triple_defaults() {
        let spec = TargetDefinition::parse("triple = \"x86_64-unknown-linux-gnu\"\n")
            .expect("valid TOML")
            .resolve()
            .expect("valid target");
        assert_eq!(spec.pointer_width(), 64);
        assert_eq!(spec.endianness(), Endianness::Little);
        assert!(spec.c_abi().char_signed());
        assert_eq!(spec.runtime(), RuntimePolicy::Default);
        assert_eq!(spec.panic(), PanicStrategy::Abort);
        assert_eq!(spec.linker(), None);
    }

    #[test]
    fn an_explicit_definition_overrides_every_property() {
        let spec = TargetDefinition::parse(
            r#"
triple = "aarch64-unknown-none"
pointer-width = 64
endianness = "big"
data-layout = "e-m:e-p:64:64"
runtime = "none"
panic = "trap"
linker = "lld"
sysroot = "/opt/sysroot"
linker-script = "kernel.ld"

[c-abi]
char-signed = false
long-width = 64
wchar-width = 32
wchar-signed = true
"#,
        )
        .expect("valid TOML")
        .resolve()
        .expect("valid target");
        assert_eq!(spec.endianness(), Endianness::Big);
        assert_eq!(spec.data_layout(), Some("e-m:e-p:64:64"));
        assert_eq!(spec.runtime(), RuntimePolicy::None);
        assert_eq!(spec.panic(), PanicStrategy::Trap);
        assert_eq!(spec.linker(), Some("lld"));
        assert_eq!(spec.sysroot(), Some(std::path::Path::new("/opt/sysroot")));
        assert_eq!(
            spec.linker_script(),
            Some(std::path::Path::new("kernel.ld"))
        );
        assert!(!spec.c_abi().char_signed());
    }

    #[test]
    fn freestanding_defaults_to_trap_and_rejects_abort() {
        let spec =
            TargetDefinition::parse("triple = \"x86_64-unknown-none\"\nruntime = \"none\"\n")
                .expect("valid TOML")
                .resolve()
                .expect("valid target");
        assert_eq!(spec.panic(), PanicStrategy::Trap);

        assert!(
            TargetDefinition::parse(
                "triple = \"x86_64-unknown-none\"\nruntime = \"none\"\npanic = \"abort\"\n",
            )
            .expect("valid TOML")
            .resolve()
            .is_err()
        );
    }

    #[test]
    fn an_explicit_pointer_width_drives_the_default_c_abi() {
        let spec =
            TargetDefinition::parse("triple = \"x86_64-unknown-linux-gnu\"\npointer-width = 32\n")
                .expect("valid TOML")
                .resolve()
                .expect("valid target");
        assert_eq!(spec.pointer_width(), 32);
        assert_eq!(spec.c_abi().long_width(), 32);
    }

    #[test]
    fn malformed_definitions_are_rejected() {
        for text in [
            "triple = \"x86_64-unknown-linux-gnu\"\nunknown = true\n",
            "triple = \"x86_64-unknown-linux-gnu\"\npointer-width = 48\n",
            "triple = \"x86_64-unknown-linux-gnu\"\nendianness = \"middle\"\n",
            "triple = \"x86_64-unknown-linux-gnu\"\nruntime = \"alloc\"\n",
            "triple = \"x86_64-unknown-linux-gnu\"\nlinker = \" \"\n",
            "triple = \"x86_64-unknown-linux-gnu\"\n[c-abi]\nchar-signed = true\nlong-width = 128\nwchar-width = 32\nwchar-signed = true\n",
        ] {
            let rejected = TargetDefinition::parse(text)
                .and_then(TargetDefinition::resolve)
                .is_err();
            assert!(rejected, "`{text}` must be rejected");
        }
    }

    #[test]
    fn a_specification_round_trips_through_toml() {
        let spec = TargetDefinition::parse(
            "triple = \"x86_64-unknown-linux-gnu\"\ndata-layout = \"e-m:e-p:64:64\"\nlinker = \"clang\"\nlinker-script = \"kernel.ld\"\nsysroot = \"/opt/sysroot\"\n",
        )
        .expect("valid TOML")
        .resolve()
        .expect("valid target");
        let rendered = spec.to_toml().expect("serializes");
        let parsed = crate::TargetSpec::parse_toml(&rendered).expect("round trips");
        assert_eq!(parsed, spec);
    }

    #[test]
    fn a_big_endian_64_bit_target_keeps_native_i128() {
        let spec =
            TargetDefinition::parse("triple = \"aarch64-unknown-none\"\nendianness = \"big\"\n")
                .expect("valid TOML")
                .resolve()
                .expect("valid target");
        assert_eq!(spec.pointer_width(), 64);
        assert_eq!(spec.endianness(), Endianness::Big);
        assert!(spec.has_native_i128());
    }
}
