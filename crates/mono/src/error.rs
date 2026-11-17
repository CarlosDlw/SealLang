//! Monomorphization failures.

use std::fmt;

/// Why one instance could not be produced.
///
/// The collector reports these as `E7xxx` diagnostics with the source span of
/// the call that needed the instance. The values are also part of the public
/// API so tests and later phases can inspect a failure without re-parsing a
/// diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonoError {
    /// The entry point declares type parameters.
    GenericEntryPoint {
        /// The number of parameters `main` declares.
        parameters: usize,
    },
    /// No trait implementation matches the concrete instantiation.
    MissingImpl {
        /// The trait's name.
        trait_name: String,
        /// The concrete self type.
        self_type: String,
    },
    /// More than one implementation matches the concrete instantiation.
    ///
    /// Coherence rejects overlapping impls before this pass; the variant keeps
    /// a broken fixture from silently choosing by declaration order.
    AmbiguousImpl {
        /// The trait's name.
        trait_name: String,
        /// The concrete self type.
        self_type: String,
    },
    /// The selected implementation does not define the trait method.
    MissingMethod {
        /// The trait's name.
        trait_name: String,
        /// The method's name.
        method: String,
    },
    /// An implementation type argument is not determined by its header.
    UnconstrainedParameter {
        /// The implementation block, for the message.
        implementation: String,
        /// The parameter's name.
        parameter: String,
    },
    /// The selected implementation requires a bound the arguments do not meet.
    MissingBound {
        /// The implementation block, for the message.
        implementation: String,
        /// The concrete argument that fails the bound.
        argument: String,
    },
    /// The instance's definition has no MIR body.
    MissingBody {
        /// The definition, for the message.
        definition: String,
    },
    /// The recursion depth limit was reached.
    RecursionLimit {
        /// The configured depth.
        limit: usize,
    },
    /// The total instance limit was reached.
    InstanceLimit {
        /// The configured limit.
        limit: usize,
    },
}

impl fmt::Display for MonoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GenericEntryPoint { parameters } => write!(
                f,
                "the entry point cannot be generic, but `main` declares {parameters} type \
                 parameter(s)"
            ),
            Self::MissingImpl {
                trait_name,
                self_type,
            } => write!(
                f,
                "no implementation of `{trait_name}` for `{self_type}` was found during \
                 monomorphization"
            ),
            Self::AmbiguousImpl {
                trait_name,
                self_type,
            } => write!(
                f,
                "more than one implementation of `{trait_name}` matches `{self_type}`"
            ),
            Self::MissingMethod { trait_name, method } => write!(
                f,
                "the implementation of `{trait_name}` does not define `{method}`"
            ),
            Self::UnconstrainedParameter {
                implementation,
                parameter,
            } => write!(
                f,
                "`{parameter}` is not determined by the header of `{implementation}`"
            ),
            Self::MissingBound {
                implementation,
                argument,
            } => write!(
                f,
                "`{argument}` does not satisfy a bound of `{implementation}`"
            ),
            Self::MissingBody { definition } => {
                write!(f, "no MIR body was found for `{definition}`")
            }
            Self::RecursionLimit { limit } => write!(
                f,
                "monomorphization reached its recursion depth limit of {limit}"
            ),
            Self::InstanceLimit { limit } => {
                write!(f, "monomorphization reached its instance limit of {limit}")
            }
        }
    }
}

impl std::error::Error for MonoError {}
