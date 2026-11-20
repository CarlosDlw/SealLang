//! Closure environments and captured values.
//!
//! Lowering gives every closure a definition, a body, and a synthetic aggregate
//! whose fields are its captures. A capture is recorded once per origin, in the
//! order the body first mentions it, so the environment layout is stable and a
//! value used twice occupies one slot.
//!
//! A capture's [`CaptureSource`] tells the environment initialization where the
//! value comes from: a local of the body that creates the closure, or a slot of
//! the immediately enclosing closure when a nested closure uses a variable that
//! the outer one already captures.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_ty::AdtId;

/// Where the environment initialization of one capture reads its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureSource {
    /// The value is a local of the body that creates the closure.
    Local(DefId),
    /// The value is already a capture of the immediately enclosing closure.
    ///
    /// The index is the position in that closure's environment.
    Outer(u32),
}

/// One value stored in a closure environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureDef {
    /// The environment field that stores the value.
    pub field: DefId,
    /// The synthetic place root used by expressions inside the body.
    pub binding: DefId,
    /// The definition the value ultimately originates from.
    ///
    /// Nested closures share the origin, which is what deduplicates a value
    /// that travels through several environments.
    pub origin: DefId,
    /// Where the closure reads the value when its environment is built.
    pub source: CaptureSource,
}

/// The semantic data of one closure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosureDef {
    /// The closure's own definition.
    pub def: DefId,
    /// The synthetic aggregate that stores the captures.
    pub adt: AdtId,
    /// Whether the source wrote `move` before the parameter list.
    ///
    /// The flag changes how the body may use the captures, not which values are
    /// captured.
    pub is_move: bool,
    /// Captures in first-mention order.
    pub captures: Vec<CaptureDef>,
}

/// The lowering stack frame of one closure being lowered.
///
/// The frame keeps the capture list and the origin-to-slot map for the closure
/// whose body is currently being lowered. Nested closures push a new frame on
/// top; the outer frames stay reachable because a nested closure may need to
/// capture a value through them.
#[derive(Debug)]
pub(crate) struct ClosureEnv {
    /// The closure this frame belongs to.
    pub closure: DefId,
    /// Captures recorded so far, in first-mention order.
    pub captures: Vec<CaptureDef>,
    /// The slot of each captured origin.
    pub indices: FxHashMap<DefId, u32>,
}

impl ClosureEnv {
    /// Returns the slot already reserved for `origin`, if any.
    pub(crate) fn index_of(&self, origin: DefId) -> Option<u32> {
        self.indices.get(&origin).copied()
    }
}
