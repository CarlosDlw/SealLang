//! Interned Seal function pointers and closure kinds.
//!
//! A Seal function pointer describes the address of a function with the
//! internal Seal ABI. It is a sized, `Copy` value: the type names two fixed
//! lists (parameters and result) and whether calling it requires `unsafe`.
//! Unlike a C function signature, it never carries a calling convention or a
//! variadic tail.
//!
//! [`ClosureKind`] names how a closure value may be called. Phase 5 of the
//! first-class function plan uses it for the `Fn`/`FnMut`/`FnOnce` bounds.

use std::fmt;

use crate::TyId;

/// One Seal function pointer type.
///
/// The definition is stored in the [`TyCtxt`](crate::TyCtxt) arena and shared
/// by every occurrence of the same signature.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FnPtrDef {
    /// Fixed parameter types in declaration order.
    pub params: Vec<TyId>,
    /// The return type, or `()` when the source omits it.
    pub ret: TyId,
    /// Whether calling through the pointer requires an `unsafe` block.
    pub is_unsafe: bool,
}

impl FnPtrDef {
    /// Returns `true` when `other` has the same parameters, result, and
    /// safety qualifier.
    pub fn matches(&self, other: &Self) -> bool {
        self.is_unsafe == other.is_unsafe && self.params == other.params && self.ret == other.ret
    }
}

/// How a closure value may be called.
///
/// The kind decides the receiver a call needs: `Fn` borrows shared, `FnMut`
/// borrows exclusively, and `FnOnce` consumes the closure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClosureKind {
    /// Callable through a shared borrow, any number of times.
    Fn,
    /// Callable through an exclusive borrow, while mutation is allowed.
    FnMut,
    /// Callable once, consuming the captured environment.
    FnOnce,
}

impl ClosureKind {
    /// Every closure kind, from the weakest receiver to the strongest.
    pub const ALL: [Self; 3] = [Self::Fn, Self::FnMut, Self::FnOnce];

    /// Returns the trait name that describes the kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fn => "Fn",
            Self::FnMut => "FnMut",
            Self::FnOnce => "FnOnce",
        }
    }
}

impl fmt::Display for ClosureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
