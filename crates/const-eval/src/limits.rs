//! Resource limits for const evaluation.

/// Deterministic bounds on one evaluation run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalLimits {
    /// Maximum number of statements and control-flow edges.
    pub steps: u64,
    /// Maximum call-stack depth.
    pub call_depth: usize,
    /// Maximum number of live allocations.
    pub allocations: usize,
    /// Maximum total size of allocated values, in bytes.
    pub value_bytes: u64,
}

impl Default for EvalLimits {
    fn default() -> Self {
        Self {
            steps: 10_000_000,
            call_depth: 128,
            allocations: 1_000_000,
            value_bytes: 64 * 1024 * 1024,
        }
    }
}
