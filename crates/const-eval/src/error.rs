//! Const-evaluation failures and their stable diagnostic codes.

use std::fmt;

use seal_error_codes as codes;

/// What went wrong during const evaluation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EvalErrorKind {
    /// Division by zero.
    DivisionByZero,
    /// Remainder by zero.
    RemainderByZero,
    /// Signed minimum divided by minus one.
    SignedOverflow,
    /// An array or slice index outside its length.
    IndexOutOfBounds {
        /// The attempted index.
        index: u64,
        /// The available length.
        length: u64,
    },
    /// A slice range that is reversed or exceeds the source length.
    SliceRange {
        /// The requested start.
        start: u64,
        /// The requested exclusive end.
        end: u64,
        /// The source length.
        length: u64,
    },
    /// A value dependency cycle.
    ValueCycle {
        /// The definitions in traversal order.
        chain: Vec<String>,
    },
    /// The statement limit was reached.
    StepLimit {
        /// The configured limit.
        limit: u64,
    },
    /// The call-depth limit was reached.
    DepthLimit {
        /// The configured limit.
        limit: usize,
    },
    /// The allocation or total value-size limit was reached.
    ResourceLimit,
    /// One value is larger than the configured value-size limit.
    ValueTooLarge,
    /// A reference escapes the frame that owns its storage.
    EscapingReference,
    /// A constant context called a non-`const` function.
    NonConstFunction {
        /// Whether the caller is a `const func` rather than an initializer.
        const_fn: bool,
    },
    /// A constant context used dynamic dispatch or a trait-bound call.
    DynamicDispatch {
        /// Whether the caller is a `const func` rather than an initializer.
        const_fn: bool,
    },
    /// An intrinsic is outside the const whitelist.
    IntrinsicNotWhitelisted,
    /// A compile-time call kept a symbolic type argument.
    NonConcreteArguments,
    /// A constant panicked, or a compile-time call had an invalid body.
    Panic(String),
    /// An internal evaluator invariant failed.
    Internal(String),
}

impl EvalErrorKind {
    /// Returns the stable code this failure reports.
    ///
    /// An internal invariant failure has no user-facing code; it is reported as
    /// an ICE through the diagnostic context instead.
    pub const fn code(&self) -> Option<seal_error_codes::ErrorCode> {
        Some(match self {
            Self::DivisionByZero => codes::E4002,
            Self::RemainderByZero => codes::E4003,
            Self::SignedOverflow => codes::E4004,
            Self::IndexOutOfBounds { .. } => codes::E4005,
            Self::SliceRange { .. } => codes::E4006,
            Self::ValueCycle { .. } => codes::E4007,
            Self::StepLimit { .. } => codes::E4008,
            Self::DepthLimit { .. } => codes::E4009,
            Self::ResourceLimit => codes::E4010,
            Self::ValueTooLarge => codes::E4025,
            Self::EscapingReference => codes::E4013,
            Self::NonConstFunction { const_fn: false } => codes::E4011,
            Self::NonConstFunction { const_fn: true } => codes::E4018,
            Self::DynamicDispatch { const_fn: false } => codes::E4016,
            Self::DynamicDispatch { const_fn: true } => codes::E4021,
            Self::IntrinsicNotWhitelisted => codes::E4022,
            Self::NonConcreteArguments | Self::Panic(_) => codes::E4027,
            Self::Internal(_) => return None,
        })
    }

    /// Returns the human-readable failure.
    pub fn message(&self) -> String {
        match self {
            Self::DivisionByZero => "division by zero in a constant expression".to_owned(),
            Self::RemainderByZero => "remainder by zero in a constant expression".to_owned(),
            Self::SignedOverflow => {
                "signed minimum divided by minus one in a constant expression".to_owned()
            }
            Self::IndexOutOfBounds { index, length } => format!(
                "index {index} is out of bounds for a length of {length} in a constant expression"
            ),
            Self::SliceRange { start, end, length } => format!(
                "slice range {start}..{end} is invalid for a length of {length} in a constant expression"
            ),
            Self::ValueCycle { chain } => format!(
                "constant evaluation cycle through value dependencies: {}",
                chain.join(" -> ")
            ),
            Self::StepLimit { limit } => {
                format!("constant evaluation reached its step limit of {limit} statements")
            }
            Self::DepthLimit { limit } => {
                format!("constant evaluation reached its call-depth limit of {limit}")
            }
            Self::ResourceLimit => {
                "constant evaluation reached its allocation or value-size limit".to_owned()
            }
            Self::ValueTooLarge => {
                "a constant value is larger than the configured size limit".to_owned()
            }
            Self::EscapingReference => {
                "a reference to const-fn local or promoted storage escapes evaluation".to_owned()
            }
            Self::NonConstFunction { .. } => {
                "a constant context cannot call a non-const function".to_owned()
            }
            Self::DynamicDispatch { .. } => {
                "a constant context cannot use dynamic dispatch or a trait-bound call".to_owned()
            }
            Self::IntrinsicNotWhitelisted => {
                "this intrinsic is outside the const-evaluation whitelist".to_owned()
            }
            Self::NonConcreteArguments => {
                "a compile-time call has non-concrete type arguments".to_owned()
            }
            Self::Panic(message) => format!("constant evaluation panicked: {message}"),
            Self::Internal(message) => {
                format!("internal const-evaluator invariant failed: {message}")
            }
        }
    }
}

impl fmt::Display for EvalErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// A failure attached to the source range that caused it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalError {
    /// What went wrong.
    pub kind: EvalErrorKind,
    /// Where it went wrong.
    pub span: seal_span::Span,
    /// The call frames active when the failure was created.
    pub call_chain: Vec<seal_def::DefId>,
}

impl EvalError {
    /// Creates a failure at `span`.
    pub const fn new(kind: EvalErrorKind, span: seal_span::Span) -> Self {
        Self {
            kind,
            span,
            call_chain: Vec::new(),
        }
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::EvalErrorKind;

    #[test]
    fn every_failure_maps_to_its_documented_code() {
        let cases = [
            (EvalErrorKind::DivisionByZero, Some("E4002")),
            (EvalErrorKind::RemainderByZero, Some("E4003")),
            (EvalErrorKind::SignedOverflow, Some("E4004")),
            (
                EvalErrorKind::IndexOutOfBounds {
                    index: 2,
                    length: 1,
                },
                Some("E4005"),
            ),
            (
                EvalErrorKind::SliceRange {
                    start: 2,
                    end: 1,
                    length: 2,
                },
                Some("E4006"),
            ),
            (
                EvalErrorKind::ValueCycle {
                    chain: vec!["A".to_owned(), "B".to_owned()],
                },
                Some("E4007"),
            ),
            (EvalErrorKind::StepLimit { limit: 1 }, Some("E4008")),
            (EvalErrorKind::DepthLimit { limit: 1 }, Some("E4009")),
            (EvalErrorKind::ResourceLimit, Some("E4010")),
            (EvalErrorKind::ValueTooLarge, Some("E4025")),
            (EvalErrorKind::EscapingReference, Some("E4013")),
            (
                EvalErrorKind::NonConstFunction { const_fn: false },
                Some("E4011"),
            ),
            (
                EvalErrorKind::NonConstFunction { const_fn: true },
                Some("E4018"),
            ),
            (
                EvalErrorKind::DynamicDispatch { const_fn: false },
                Some("E4016"),
            ),
            (
                EvalErrorKind::DynamicDispatch { const_fn: true },
                Some("E4021"),
            ),
            (EvalErrorKind::IntrinsicNotWhitelisted, Some("E4022")),
            (EvalErrorKind::NonConcreteArguments, Some("E4027")),
            (EvalErrorKind::Panic("boom".to_owned()), Some("E4027")),
            (EvalErrorKind::Internal("bug".to_owned()), None),
        ];
        for (kind, expected) in cases {
            assert_eq!(kind.code().map(|code| code.code()), expected, "{kind:?}");
        }
    }
}
