//! Structured diagnostics for the Seal compiler.
//!
//! A diagnostic is data, not a formatted string. Phases build a [`Diagnostic`],
//! and one of the [`Emitter`] backends renders it:
//!
//! - [`RichEmitter`] for humans, with colored headers, source snippets, labels,
//!   notes, helps, and suggestion diffs.
//! - [`JsonEmitter`] for tools, with a versioned schema and one object per line.
//! - [`SilentEmitter`] for tests, which collects diagnostics for assertions.
//!
//! Error handling is encoded in the type system: emitting an error returns an
//! [`ErrorGuaranteed`], and `DiagCtxt::abort_if_errors` refuses to continue
//! without one.
//!
//! Output is deterministic for a given diagnostic and source map; emitters never
//! read the clock, environment, or terminal size implicitly.

#![warn(missing_docs)]

mod code;
mod context;
mod diagnostic;
mod emitter;
mod level;

pub use code::DiagCode;
pub use context::{
    DEFAULT_ERROR_LIMIT, DiagCtxt, DiagnosticBuilder, ErrorBuilder, ErrorGuaranteed, FatalError,
    IcePanic, WarningBuilder,
};
pub use diagnostic::{Applicability, Diagnostic, Label, LabelStyle, Suggestion};
pub use emitter::{
    ColorChoice, Emitter, JsonEmitter, RichEmitter, SCHEMA_VERSION, SilentEmitter, should_use_color,
};
pub use level::Level;
