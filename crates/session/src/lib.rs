//! The compiler session.
//!
//! A [`Session`] owns everything a compilation needs and everything that must be
//! shared between phases: options, target specification, source map, symbol
//! interner, diagnostic context, feature gates, cancellation, and profiling.
//!
//! There is no global mutable state. Two sessions in one process are fully
//! independent, which is what makes parallel compilation of independent crates
//! and in-process tests possible.

#![warn(missing_docs)]

mod options;

use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use seal_diagnostics::{
    ColorChoice, DiagCtxt, Emitter, ErrorGuaranteed, JsonEmitter, LintLevel, RichEmitter,
    known_lint, should_use_color,
};
use seal_error_codes as codes;
use seal_fs::{self, LoadError};
use seal_profiling::Profiler;
use seal_query::QueryDb;
use seal_span::{FileId, FileName, PathRemapper, SourceMap};
use seal_symbol::Interner;
use seal_target::{TargetError, TargetSpec, TargetTriple};
use seal_version::{Edition, FeatureGates, LanguageVersion};

pub use options::{ErrorFormat, OptLevel, Options};

/// Failure to create a session.
#[derive(Debug)]
pub enum SessionError {
    /// The target triple is invalid.
    Target(TargetError),
    /// A path remapping specification is invalid.
    Remap(seal_span::RemapError),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Target(error) => error.fmt(f),
            Self::Remap(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SessionError {}

/// Compilation was cancelled cooperatively.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("compilation cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// One compilation session.
pub struct Session {
    options: Options,
    target: TargetSpec,
    source_map: Arc<SourceMap>,
    interner: Arc<Interner>,
    diag: DiagCtxt,
    gates: FeatureGates,
    profiler: Profiler,
    queries: QueryDb,
    /// Last file loaded under a persistent build identity.
    ///
    /// A persistent session keeps every source generation so spans from earlier
    /// values stay valid. Loading changed text appends a new `FileId`; unchanged
    /// text returns the existing one.
    persistent_files: Mutex<HashMap<String, (FileId, seal_hash::Fingerprint)>>,
    /// Number of changed-file generations appended by persistent loads.
    persistent_versions: AtomicUsize,
    cancelled: AtomicBool,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("options", &self.options)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Creates a session with the default emitter for the configured format.
    pub fn new(options: Options) -> Result<Self, SessionError> {
        Self::with_emitter(options, None)
    }

    /// Creates a session with an explicit emitter, used by tests and embedders.
    pub fn with_emitter(
        options: Options,
        emitter: Option<Box<dyn Emitter>>,
    ) -> Result<Self, SessionError> {
        let parsed = TargetTriple::parse(options.target.as_str()).map_err(SessionError::Target)?;
        let target = match &options.target_spec {
            Some(spec) if spec.triple() == &parsed => spec.clone(),
            Some(spec) => {
                return Err(SessionError::Target(TargetError::InvalidSpec {
                    detail: format!(
                        "the target specification names `{}`, but the session compiles for `{parsed}`",
                        spec.triple()
                    ),
                }));
            }
            None => parsed.spec(),
        };
        let remapper = PathRemapper::from_specs(options.remap_prefixes.iter().map(String::as_str))
            .map_err(SessionError::Remap)?;
        let source_map = Arc::new(SourceMap::new());
        source_map.set_remapper(remapper);

        let emitter = emitter.unwrap_or_else(|| default_emitter(&options));
        let diag = DiagCtxt::new(emitter, Arc::clone(&source_map));
        diag.set_error_limit(options.max_errors);
        apply_lint_policy(&diag, &options);

        let gates = FeatureGates::new(options.allow_unstable);
        let profiler = Profiler::new(options.time_passes);

        Ok(Self {
            options,
            target,
            source_map,
            interner: Arc::new(Interner::new()),
            diag,
            gates,
            profiler,
            queries: QueryDb::new(),
            persistent_files: Mutex::new(HashMap::new()),
            persistent_versions: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
        })
    }

    /// Returns the options.
    #[inline]
    pub fn options(&self) -> &Options {
        &self.options
    }

    /// Returns the target specification.
    #[inline]
    pub fn target(&self) -> &TargetSpec {
        &self.target
    }

    /// Returns the source map.
    #[inline]
    pub fn source_map(&self) -> &Arc<SourceMap> {
        &self.source_map
    }

    /// Returns the symbol interner.
    #[inline]
    pub fn interner(&self) -> &Arc<Interner> {
        &self.interner
    }

    /// Returns the diagnostic context.
    #[inline]
    pub fn diag(&self) -> &DiagCtxt {
        &self.diag
    }

    /// Returns the feature gates.
    #[inline]
    pub fn gates(&self) -> &FeatureGates {
        &self.gates
    }

    /// Returns the profiler.
    #[inline]
    pub fn profiler(&self) -> &Profiler {
        &self.profiler
    }

    /// Returns the incremental query database for this session.
    ///
    /// The database is scoped to the session: two sessions never share memoized
    /// compiler arenas. Persistent byte results belong in a `DiskCache` owned by
    /// the build layer, not in this in-memory table.
    #[inline]
    pub fn queries(&self) -> &QueryDb {
        &self.queries
    }

    /// Returns the selected compatibility edition.
    #[inline]
    pub fn edition(&self) -> Edition {
        self.options.edition
    }

    /// Returns the selected language version.
    #[inline]
    pub fn language_version(&self) -> LanguageVersion {
        self.options.language_version
    }

    /// Resets per-request diagnostic and cancellation state.
    ///
    /// A persistent compiler reuses one session across edits. Memoized query
    /// values and registered source generations stay; error counts and the
    /// deduplication set start clean for the next request.
    pub fn reset_for_request(&self) {
        self.diag.reset();
        self.cancelled.store(false, Ordering::Relaxed);
    }

    /// Returns how many changed-file generations this session has appended.
    ///
    /// A persistent build keeps one source generation per file version so old
    /// spans stay valid. The count excludes the first load of each identity, so
    /// a large project does not look like a long-running incremental session.
    pub fn source_generations(&self) -> usize {
        self.persistent_versions.load(Ordering::Relaxed)
    }

    /// Requests cooperative cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Returns `true` if cancellation was requested.
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Returns [`Cancelled`] if cancellation was requested.
    pub fn check_cancelled(&self) -> Result<(), Cancelled> {
        if self.is_cancelled() {
            Err(Cancelled)
        } else {
            Ok(())
        }
    }

    /// Returns `Ok(())` if no error was emitted.
    pub fn abort_if_errors(&self) -> Result<(), ErrorGuaranteed> {
        self.diag.abort_if_errors()
    }

    /// Registers an in-memory file.
    ///
    /// A source-map failure is reported as a diagnostic and returned as an error
    /// guarantee; it is never silently dropped.
    pub fn add_file(&self, name: FileName, text: String) -> Result<FileId, ErrorGuaranteed> {
        self.add_file_impl(name, text, false)
    }

    /// Registers an in-memory file as a distinct compilation unit.
    ///
    /// This is used when one real source belongs to several package instances.
    pub fn add_file_distinct(
        &self,
        name: FileName,
        text: String,
    ) -> Result<FileId, ErrorGuaranteed> {
        self.add_file_impl(name, text, true)
    }

    fn add_file_impl(
        &self,
        name: FileName,
        text: String,
        distinct: bool,
    ) -> Result<FileId, ErrorGuaranteed> {
        let result = if distinct {
            self.source_map.add_file_distinct(name, text)
        } else {
            self.source_map.add_file(name, text)
        };
        match result {
            Ok(id) => Ok(id),
            Err(seal_span::SourceMapError::FileTooLarge { len, max }) => Err(self
                .diag
                .error(format!(
                    "source file is {len} bytes, exceeding the {max}-byte limit"
                ))
                .code(codes::E0002.diag_code())
                .emit()),
            Err(error) => Err(self
                .diag
                .error(format!("cannot register source file: {error}"))
                .emit()),
        }
    }

    /// Loads a source file from disk and registers it.
    ///
    /// Load failures become diagnostics with the appropriate code; the returned
    /// guarantee forces callers to stop.
    pub fn load_file(&self, path: &Path) -> Result<FileId, ErrorGuaranteed> {
        self.profiler.time("load", || {
            match seal_fs::load(path, seal_fs::DEFAULT_MAX_FILE_SIZE) {
                Ok(source) => self.add_file(FileName::real(source.path()), source.into_text()),
                Err(error) => Err(self.report_load_error(error)),
            }
        })
    }

    /// Loads a source file under a persistent build identity.
    ///
    /// `identity` names the logical compilation unit, such as a package index
    /// plus path. Reloading identical bytes returns the previous `FileId`.
    /// Reloading changed bytes appends a new source generation with a fresh
    /// `FileId`, so cached values for unchanged files keep valid spans while the
    /// changed file gets a new query key. The old generation stays in the source
    /// map for values that still refer to it.
    pub fn load_file_persistent(
        &self,
        path: &Path,
        identity: &str,
    ) -> Result<FileId, ErrorGuaranteed> {
        self.profiler.time("load", || {
            let source = match seal_fs::load(path, seal_fs::DEFAULT_MAX_FILE_SIZE) {
                Ok(source) => source,
                Err(error) => return Err(self.report_load_error(error)),
            };
            let digest = seal_hash::stable_hash_of(source.text());
            let mut files = self
                .persistent_files
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((file, previous)) = files.get(identity)
                && *previous == digest
            {
                return Ok(*file);
            }
            if files.contains_key(identity) {
                self.persistent_versions.fetch_add(1, Ordering::Relaxed);
            }
            let name = FileName::real(source.path());
            let text = source.into_text();
            let result = self.source_map.add_file_distinct(name, text);
            let file = match result {
                Ok(file) => file,
                Err(seal_span::SourceMapError::FileTooLarge { len, max }) => {
                    return Err(self
                        .diag
                        .error(format!(
                            "source file is {len} bytes, exceeding the {max}-byte limit"
                        ))
                        .code(codes::E0002.diag_code())
                        .emit());
                }
                Err(error) => {
                    return Err(self
                        .diag
                        .error(format!("cannot register source file: {error}"))
                        .emit());
                }
            };
            files.insert(identity.to_owned(), (file, digest));
            Ok(file)
        })
    }

    /// Loads a source file as a distinct compilation unit.
    pub fn load_file_distinct(&self, path: &Path) -> Result<FileId, ErrorGuaranteed> {
        self.profiler.time("load", || {
            match seal_fs::load(path, seal_fs::DEFAULT_MAX_FILE_SIZE) {
                Ok(source) => {
                    self.add_file_distinct(FileName::real(source.path()), source.into_text())
                }
                Err(error) => Err(self.report_load_error(error)),
            }
        })
    }

    fn report_load_error(&self, error: LoadError) -> ErrorGuaranteed {
        let shown = |path: &Path| self.source_map.remap_path(path).display().to_string();
        let (message, code) = match &error {
            LoadError::NotUtf8 { path, valid_up_to } => (
                format!(
                    "source file `{}` is not valid UTF-8: invalid byte at offset {valid_up_to}",
                    shown(path)
                ),
                codes::E0001,
            ),
            LoadError::TooLarge { path, len, max } => (
                format!(
                    "source file `{}` is {len} bytes, exceeding the {max}-byte limit",
                    shown(path)
                ),
                codes::E0002,
            ),
            LoadError::NotFound { path } => (
                format!("cannot read `{}`: no such file", shown(path)),
                codes::E0003,
            ),
            LoadError::NotAFile { path } => (
                format!("cannot read `{}`: is a directory", shown(path)),
                codes::E0003,
            ),
            LoadError::Io { path, source } => (
                format!("cannot read `{}`: {source}", shown(path)),
                codes::E0003,
            ),
        };
        self.diag.error(message).code(code.diag_code()).emit()
    }
}

/// Applies the option-level lint policy to a fresh diagnostic context.
///
/// Unknown names are ignored here: the driver rejects them before creating a
/// session, and an embedder that bypasses the driver still gets the default
/// warning level instead of a panic. A name in both lists is denied, because
/// denying is the safer interpretation of contradictory input.
fn apply_lint_policy(diag: &DiagCtxt, options: &Options) {
    for name in &options.allow_lints {
        if let Some(lint) = known_lint(name) {
            diag.set_lint_level(lint, LintLevel::Allow);
        }
    }
    for name in &options.deny_lints {
        if let Some(lint) = known_lint(name) {
            diag.set_lint_level(lint, LintLevel::Deny);
        }
    }
}

fn default_emitter(options: &Options) -> Box<dyn Emitter> {
    match options.error_format {
        ErrorFormat::Human => {
            let color = should_use_color(options.color, std::io::stderr().is_terminal(), |key| {
                std::env::var(key).ok()
            });
            Box::new(RichEmitter::with_color(std::io::stderr(), color))
        }
        ErrorFormat::Json => Box::new(JsonEmitter::new(std::io::stdout())),
    }
}

/// Resolves a [`ColorChoice`] for a session writing to stderr. Exposed for the
/// driver, which may write to a different stream.
pub fn resolve_color(choice: ColorChoice) -> bool {
    should_use_color(choice, std::io::stderr().is_terminal(), |key| {
        std::env::var(key).ok()
    })
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Arc, Mutex};

    use seal_diagnostics::{Diagnostic, Emitter};
    use seal_span::{FileName, SourceMap};

    use super::{Options, Session, SessionError};

    /// Recording emitter that can be inspected after being boxed into a session.
    #[derive(Clone, Default)]
    struct Recording(Arc<Mutex<Vec<Diagnostic>>>);

    impl Emitter for Recording {
        fn emit(&mut self, diagnostic: &Diagnostic, _source_map: &SourceMap) -> io::Result<()> {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(diagnostic.clone());
            Ok(())
        }
    }

    impl Recording {
        fn diagnostics(&self) -> Vec<Diagnostic> {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        }
    }

    fn session() -> (Session, Recording) {
        let emitter = Recording::default();
        let session = Session::with_emitter(Options::default(), Some(Box::new(emitter.clone())))
            .expect("session builds");
        (session, emitter)
    }

    #[test]
    fn session_owns_sources_and_symbols() {
        let (session, _) = session();
        let id = session
            .add_file(FileName::virtual_name("main"), "func main() {}".to_owned())
            .expect("file registers");
        assert_eq!(
            session.source_map().file(id).expect("file exists").text(),
            "func main() {}"
        );
        assert_eq!(session.interner().resolve(seal_symbol::Symbol::EMPTY), "");
        assert!(!session.is_cancelled());
    }

    #[test]
    fn cancellation_is_cooperative() {
        let (session, _) = session();
        assert!(session.check_cancelled().is_ok());
        session.cancel();
        assert!(session.is_cancelled());
        assert!(session.check_cancelled().is_err());
    }

    #[test]
    fn invalid_target_is_rejected() {
        let error = seal_target::TargetTriple::parse("bad").expect_err("invalid target");
        let wrapped = SessionError::Target(error);
        assert!(wrapped.to_string().contains("bad"));
        assert_eq!(
            Session::with_emitter(Options::default(), Some(Box::new(Recording::default())),)
                .expect("default target is valid")
                .target()
                .pointer_width(),
            64
        );
    }

    #[test]
    fn invalid_remap_is_rejected() {
        let options = Options {
            remap_prefixes: vec!["missing-separator".to_owned()],
            ..Options::default()
        };
        let error = Session::with_emitter(options, Some(Box::new(Recording::default())))
            .expect_err("invalid remap");
        assert!(matches!(error, SessionError::Remap(_)));
    }

    #[test]
    fn session_options_apply_the_lint_policy() {
        let options = Options {
            allow_lints: vec!["unused_import".to_owned()],
            deny_lints: vec!["deprecated".to_owned()],
            ..Options::default()
        };
        let emitter = Recording::default();
        let session = Session::with_emitter(options, Some(Box::new(emitter.clone())))
            .expect("session builds");
        assert_eq!(
            session.diag().lint_level("unused_import"),
            seal_diagnostics::LintLevel::Allow
        );
        assert_eq!(
            session.diag().lint_level("deprecated"),
            seal_diagnostics::LintLevel::Deny
        );
        let id = session
            .add_file(FileName::virtual_name("t"), "abc".to_owned())
            .expect("file registers");
        let span = session.source_map().span(id, 0..1).expect("valid span");
        session
            .diag()
            .warning("old")
            .lint("deprecated")
            .primary(span, "")
            .emit();
        assert!(session.abort_if_errors().is_err());
        let diagnostics = emitter.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].level(), seal_diagnostics::Level::Error);
    }

    #[test]
    fn errors_set_the_abort_flag() {
        let (session, emitter) = session();
        let id = session
            .add_file(FileName::virtual_name("t"), "abc".to_owned())
            .expect("file registers");
        let span = session.source_map().span(id, 0..1).expect("valid span");
        session.diag().error("boom").primary(span, "here").emit();
        assert!(session.abort_if_errors().is_err());
        assert_eq!(emitter.diagnostics().len(), 1);
    }

    #[test]
    fn file_too_large_is_reported_with_its_code() {
        let (session, emitter) = session();
        let error = session
            .add_file(
                FileName::virtual_name("big"),
                "x".repeat(seal_span::MAX_FILE_SIZE as usize + 1),
            )
            .expect_err("oversized file is rejected");
        let _ = error;
        let diagnostics = emitter.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code().map(|code| code.as_str()),
            Some("E0002")
        );
    }

    #[test]
    fn load_errors_apply_path_remapping() {
        let options = Options {
            remap_prefixes: vec!["/definitely=/src".to_owned()],
            ..Options::default()
        };
        let emitter = Recording::default();
        let session = Session::with_emitter(options, Some(Box::new(emitter.clone())))
            .expect("session builds");
        let _ = session
            .load_file(std::path::Path::new("/definitely/missing.sl"))
            .expect_err("missing file is rejected");
        let diagnostics = emitter.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert!(
            diagnostics[0].message().contains("/src/missing.sl"),
            "message is remapped: {}",
            diagnostics[0].message()
        );
        assert!(!diagnostics[0].message().contains("/definitely"));
    }

    #[test]
    fn missing_file_produces_a_diagnostic() {
        let (session, emitter) = session();
        let error = session
            .load_file(std::path::Path::new("/definitely/not/a/seal/file.sl"))
            .expect_err("missing file is rejected");
        let _ = error;
        let diagnostics = emitter.diagnostics();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].code().map(|code| code.as_str()),
            Some("E0003")
        );
    }
}
