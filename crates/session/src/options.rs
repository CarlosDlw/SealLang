//! Compiler options.
//!
//! Every option is either **tracked** (it can change the compiler's output and
//! therefore participates in cache and reproducibility fingerprints) or
//! **untracked** (presentation or bookkeeping only). The split is documented per
//! field and enforced by [`Options::tracked_fingerprint`].

use seal_diagnostics::{ColorChoice, DEFAULT_ERROR_LIMIT};
use seal_hash::{Fingerprint, StableHash, StableHasher};
use seal_target::{TargetSpec, TargetTriple};
use seal_version::LanguageVersion;

/// Optimization level.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum OptLevel {
    /// No optimization.
    #[default]
    O0,
    /// Basic optimization.
    O1,
    /// Standard optimization.
    O2,
    /// Aggressive optimization.
    O3,
}

impl OptLevel {
    /// Returns `0..=3`.
    #[inline]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::O0 => 0,
            Self::O1 => 1,
            Self::O2 => 2,
            Self::O3 => 3,
        }
    }
}

/// How diagnostics are written.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ErrorFormat {
    /// Human-readable snippets.
    #[default]
    Human,
    /// Newline-delimited JSON.
    Json,
}

/// Options for one compilation session.
#[derive(Clone, Debug)]
pub struct Options {
    /// Crate name. Tracked: embedded in metadata.
    pub crate_name: Option<String>,
    /// Target triple. Tracked: changes layout and code generation.
    pub target: TargetTriple,
    /// Full target specification for custom, cross, and freestanding targets.
    ///
    /// `None` derives it from `target`. Tracked: every property changes layout,
    /// ABI, or linking.
    pub target_spec: Option<TargetSpec>,
    /// Optimization level. Tracked.
    pub opt_level: OptLevel,
    /// Emit debug information. Tracked.
    pub debug_info: bool,
    /// Language version. Tracked: changes accepted syntax.
    pub language_version: LanguageVersion,
    /// Allow unstable features. Tracked.
    pub allow_unstable: bool,
    /// `--remap-path-prefix` specifications. Tracked: affects emitted paths.
    pub remap_prefixes: Vec<String>,
    /// Maximum number of errors before emission stops. Tracked only in the sense
    /// that it truncates diagnostics; it does not affect compiled output, but is
    /// included for reproducible diagnostic sets.
    pub max_errors: usize,
    /// Diagnostic format. Untracked: presentation only.
    pub error_format: ErrorFormat,
    /// Color policy. Untracked: presentation only.
    pub color: ColorChoice,
    /// Collect phase timings. Untracked: measurement only.
    pub time_passes: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            crate_name: None,
            target: seal_target::host_triple(),
            target_spec: None,
            opt_level: OptLevel::default(),
            debug_info: false,
            language_version: LanguageVersion::CURRENT,
            allow_unstable: false,
            remap_prefixes: Vec::new(),
            max_errors: DEFAULT_ERROR_LIMIT,
            error_format: ErrorFormat::default(),
            color: ColorChoice::default(),
            time_passes: false,
        }
    }
}

impl Options {
    /// Returns a fingerprint of every tracked option.
    ///
    /// Untracked options (format, color, timing) are deliberately excluded: two
    /// runs that differ only in those must produce identical artifacts and cache
    /// keys.
    pub fn tracked_fingerprint(&self) -> Fingerprint {
        let mut hasher = StableHasher::new();
        self.crate_name.stable_hash(&mut hasher);
        self.target.as_str().stable_hash(&mut hasher);
        if let Some(spec) = &self.target_spec {
            spec.triple().as_str().stable_hash(&mut hasher);
            hasher.write_u8(spec.pointer_width());
            spec.data_layout()
                .unwrap_or_default()
                .stable_hash(&mut hasher);
            hasher.write_u8(match spec.endianness() {
                seal_target::Endianness::Little => 0,
                seal_target::Endianness::Big => 1,
            });
            let abi = spec.c_abi();
            abi.char_signed().stable_hash(&mut hasher);
            abi.long_width().stable_hash(&mut hasher);
            abi.wchar_width().stable_hash(&mut hasher);
            abi.wchar_signed().stable_hash(&mut hasher);
            spec.runtime().as_str().stable_hash(&mut hasher);
            spec.panic().as_str().stable_hash(&mut hasher);
            spec.linker().unwrap_or_default().stable_hash(&mut hasher);
            spec.sysroot()
                .map(|path| path.to_string_lossy())
                .unwrap_or_default()
                .stable_hash(&mut hasher);
            spec.linker_script()
                .map(|path| path.to_string_lossy())
                .unwrap_or_default()
                .stable_hash(&mut hasher);
        }
        hasher.write_u8(self.opt_level.as_u8());
        self.debug_info.stable_hash(&mut hasher);
        self.language_version.major().stable_hash(&mut hasher);
        self.language_version.minor().stable_hash(&mut hasher);
        self.allow_unstable.stable_hash(&mut hasher);
        hasher.write_usize(self.remap_prefixes.len());
        for spec in &self.remap_prefixes {
            spec.stable_hash(&mut hasher);
        }
        self.max_errors.stable_hash(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use seal_diagnostics::ColorChoice;
    use seal_hash::StableHash;
    use seal_target::TargetTriple;
    use seal_version::LanguageVersion;

    use super::{ErrorFormat, OptLevel, Options};

    #[test]
    fn defaults_are_sane() {
        let options = Options::default();
        assert_eq!(options.opt_level, OptLevel::O0);
        assert_eq!(options.error_format, ErrorFormat::Human);
        assert_eq!(options.color, ColorChoice::Auto);
        assert_eq!(options.language_version, LanguageVersion::CURRENT);
        assert!(!options.allow_unstable);
    }

    #[test]
    fn presentation_options_do_not_change_the_tracked_fingerprint() {
        let base = Options::default();
        let mut presentation = base.clone();
        presentation.color = ColorChoice::Never;
        presentation.error_format = ErrorFormat::Json;
        presentation.time_passes = true;
        assert_eq!(
            base.tracked_fingerprint(),
            presentation.tracked_fingerprint()
        );
    }

    #[test]
    fn tracked_options_change_the_fingerprint() {
        let base = Options::default();

        let mut target = base.clone();
        target.target = TargetTriple::parse("aarch64-unknown-linux-gnu").expect("valid target");
        assert_ne!(base.tracked_fingerprint(), target.tracked_fingerprint());

        let mut opt = base.clone();
        opt.opt_level = OptLevel::O2;
        assert_ne!(base.tracked_fingerprint(), opt.tracked_fingerprint());

        let mut version = base.clone();
        version.language_version = LanguageVersion::new(0, 2);
        assert_ne!(base.tracked_fingerprint(), version.tracked_fingerprint());

        let mut remap = base.clone();
        remap.remap_prefixes = vec!["/a=/b".to_owned()];
        assert_ne!(base.tracked_fingerprint(), remap.tracked_fingerprint());
    }

    #[test]
    fn remap_order_matters() {
        let first = Options {
            remap_prefixes: vec!["/a=/b".to_owned(), "/c=/d".to_owned()],
            ..Options::default()
        };
        let mut second = first.clone();
        second.remap_prefixes.reverse();
        assert_ne!(first.tracked_fingerprint(), second.tracked_fingerprint());
    }

    #[test]
    fn stable_hash_is_reachable_for_options_fields() {
        let mut hasher = seal_hash::StableHasher::new();
        "x".stable_hash(&mut hasher);
        assert_ne!(hasher.finish(), seal_hash::Fingerprint::ZERO);
    }
}
