//! Dropping the value an assignment is about to replace.
//!
//! `x = v` writes over whatever `x` held. When that value owns something, the
//! old value has to be dropped before the store, exactly like the end of a scope
//! drops it. The old value is not always there: a local assigned only on some
//! paths has no value on the others, so the caller guards the drop with the same
//! flag the drop sites use.
//!
//! Globals never get a replacement drop. A `static` lives for the whole program
//! and is never dropped, and a `const` has no storage of its own: lowering
//! materializes its value into a temporary before an assignment can target it.

use seal_hir::Hir;

use crate::body::Body;
use crate::place::{Place, PlaceRoot};
use crate::rvalue::Rvalue;
use crate::validate::place_ty;

use super::moved_places;

/// Returns whether assigning to `place` must drop the value it replaces.
///
/// Only the type decides: a value that needs no destructor can be overwritten
/// in place. Whether the value is actually there is the caller's question,
/// because it depends on the control-flow path that reaches the assignment.
pub(crate) fn must_drop_old_value(hir: &Hir, body: &Body, place: &Place) -> bool {
    match place.root {
        PlaceRoot::Local(local) => {
            // The return place is written by the function, never read before
            // the write, so there is no old value to drop. A temporary that
            // holds the result is not a local zero either: only the return
            // place itself is excluded.
            if local == Body::return_local() {
                return false;
            }
        }
        PlaceRoot::Static(_) | PlaceRoot::Const(_) => return false,
    }
    hir.ty_ctxt().needs_drop(place_ty(hir, body, place))
}

/// Returns whether the assignment takes the value it replaces out of itself.
///
/// `x = x` moves `x` into the rvalue and writes it straight back. The old value
/// is not replaced, it is the new value, so dropping it first would drop a value
/// that was just moved out. Any moved place that overlaps the destination is
/// read the same way: the assignment is built from the storage it overwrites.
pub(crate) fn takes_value_from_destination(rvalue: &Rvalue, destination: &Place) -> bool {
    moved_places(rvalue).iter().any(|moved| {
        moved.root == destination.root
            && (moved.is_prefix_of(destination) || destination.is_prefix_of(moved))
    })
}
