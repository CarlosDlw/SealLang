//! C ABI properties derived from a target specification.
//!
//! These properties decide how the C scalar names (`c_char`, `c_long`,
//! `c_wchar`) resolve. They are facts about the target, never about the host,
//! so the compiler does not consult `cfg!` or the machine that built it.

use crate::{Arch, Os, TargetSpec, TargetTriple};

/// The C ABI properties of one target.
///
/// The struct is cheap to copy and describes the facts the type system needs to
/// resolve the target-dependent C scalar names. Fixed-width C types
/// (`int32_t`, `size_t`, ...) are expressed by the Seal integer types directly
/// and need no property here.
///
/// [`CAbiProperties::default`] describes the LP64 Unix ABI: signed `char`, a
/// 64-bit `long`, and a signed 32-bit `wchar_t`. Target-aware code must use
/// [`TargetSpec::c_abi`](crate::TargetSpec::c_abi) instead of relying on it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct CAbiProperties {
    char_signed: bool,
    long_width: u16,
    wchar_width: u16,
    wchar_signed: bool,
}

impl CAbiProperties {
    /// Creates explicit C ABI properties.
    ///
    /// Callers validate the widths; this constructor only records them.
    pub(crate) const fn new(
        char_signed: bool,
        long_width: u16,
        wchar_width: u16,
        wchar_signed: bool,
    ) -> Self {
        Self {
            char_signed,
            long_width,
            wchar_width,
            wchar_signed,
        }
    }

    /// Derives the properties from `target`.
    ///
    /// `char` is signed on Windows, Darwin, and the x86 family, and unsigned on
    /// the ARM, AArch64, RISC-V, and WebAssembly ABIs. `long` is 64 bits on
    /// LP64 targets and 32 bits on 32-bit targets and on Windows (LLP64).
    /// `wchar_t` is a 16-bit unsigned type on Windows and a 32-bit signed type
    /// elsewhere.
    pub fn for_target(target: &TargetSpec) -> Self {
        Self::for_target_parts(target.triple(), target.pointer_width())
    }

    /// Derives the properties from a triple and an explicit pointer width.
    ///
    /// A custom target specification can change the pointer width of the
    /// triple it names, so the C ABI must use the resolved width rather than
    /// the built-in default.
    pub(crate) fn for_target_parts(triple: &TargetTriple, pointer_width: u8) -> Self {
        let os = triple.os();
        let arch = triple.arch();
        let char_signed =
            matches!(os, Os::Windows | Os::Darwin) || matches!(arch, Arch::X86 | Arch::X86_64);
        let long_width = if pointer_width == 32 || os == Os::Windows {
            32
        } else {
            64
        };
        let wchar_width = if os == Os::Windows { 16 } else { 32 };
        Self {
            char_signed,
            long_width,
            wchar_width,
            wchar_signed: os != Os::Windows,
        }
    }

    /// Returns whether plain `char` is signed on this target.
    #[inline]
    pub const fn char_signed(self) -> bool {
        self.char_signed
    }

    /// Returns the width of `long` and `unsigned long` in bits.
    ///
    /// The value is always 32 or 64.
    #[inline]
    pub const fn long_width(self) -> u16 {
        self.long_width
    }

    /// Returns the width of `wchar_t` in bits.
    ///
    /// The value is 16 on Windows and 32 elsewhere.
    #[inline]
    pub const fn wchar_width(self) -> u16 {
        self.wchar_width
    }

    /// Returns whether `wchar_t` is signed on this target.
    #[inline]
    pub const fn wchar_signed(self) -> bool {
        self.wchar_signed
    }

    /// Returns the width of C `_Bool` in bytes.
    ///
    /// `_Bool` is one byte on every supported target. The width is a property
    /// of the C ABI rather than a per-target field, so it is a constant.
    #[inline]
    pub const fn bool_width(self) -> u16 {
        1
    }
}

impl Default for CAbiProperties {
    /// Returns the LP64 Unix C ABI.
    fn default() -> Self {
        Self {
            char_signed: true,
            long_width: 64,
            wchar_width: 32,
            wchar_signed: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CAbiProperties;
    use crate::TargetTriple;

    fn properties(triple: &str) -> CAbiProperties {
        CAbiProperties::for_target(&TargetTriple::parse(triple).expect("valid triple").spec())
    }

    #[test]
    fn unix_x86_64_uses_the_lp64_abi() {
        let abi = properties("x86_64-unknown-linux-gnu");
        assert!(abi.char_signed());
        assert_eq!(abi.long_width(), 64);
        assert_eq!(abi.wchar_width(), 32);
        assert!(abi.wchar_signed());
        assert_eq!(abi.bool_width(), 1);
    }

    #[test]
    fn arm_families_use_an_unsigned_char() {
        for triple in [
            "aarch64-unknown-linux-gnu",
            "arm-unknown-linux-gnueabihf",
            "riscv64-unknown-linux-gnu",
        ] {
            let abi = properties(triple);
            assert!(!abi.char_signed(), "`{triple}` must use an unsigned char");
            let expected_long = if triple == "arm-unknown-linux-gnueabihf" {
                32
            } else {
                64
            };
            assert_eq!(abi.long_width(), expected_long);
        }
    }

    #[test]
    fn wasm32_uses_an_unsigned_char_and_a_32_bit_long() {
        let abi = properties("wasm32-unknown-unknown");
        assert!(!abi.char_signed());
        assert_eq!(abi.long_width(), 32);
        assert_eq!(abi.wchar_width(), 32);
        assert!(abi.wchar_signed());
    }

    #[test]
    fn darwin_keeps_char_signed() {
        let abi = properties("aarch64-apple-darwin");
        assert!(abi.char_signed());
        assert_eq!(abi.long_width(), 64);
        assert!(abi.wchar_signed());
    }

    #[test]
    fn windows_is_llp64_with_a_16_bit_unsigned_wchar() {
        for triple in ["x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"] {
            let abi = properties(triple);
            assert!(abi.char_signed(), "`{triple}` uses a signed char");
            assert_eq!(abi.long_width(), 32);
            assert_eq!(abi.wchar_width(), 16);
            assert!(!abi.wchar_signed());
        }
    }

    #[test]
    fn thirty_two_bit_targets_use_a_32_bit_long() {
        for triple in ["i686-unknown-linux-gnu", "arm-unknown-linux-gnueabihf"] {
            assert_eq!(properties(triple).long_width(), 32);
        }
    }

    #[test]
    fn the_default_is_the_lp64_unix_abi() {
        let abi = CAbiProperties::default();
        assert!(abi.char_signed());
        assert_eq!(abi.long_width(), 64);
        assert_eq!(abi.wchar_width(), 32);
        assert!(abi.wchar_signed());
    }
}
