//! Deterministic layout of Seal types.
//!
//! Layout is a query over interned types, never a field stored beside them. The
//! rules are fixed by the language, not by the host or a C ABI:
//!
//! - fields keep declaration order, with no reordering;
//! - every field starts at the next multiple of its alignment;
//! - a struct's alignment is the maximum of its fields' alignments, or 1;
//! - a struct's size is the end of its last field rounded up to its alignment;
//! - an enum starts with the smallest unsigned tag that addresses every
//!   variant, followed by the payload at the next multiple of the payload
//!   alignment; the payload is as large as the largest variant and the whole
//!   value is padded to the maximum alignment;
//! - scalar alignment equals size, references use the target pointer width,
//!   `char` is 4 bytes, `bool` is 1, and `()`/`!` are zero-sized.
//!
//! Trait-object vtables use a separate deterministic slot layout: a three-entry
//! header followed by the principal and supertrait methods. See [`VtableLayout`].
//!
//! Type checking rejects unsized and recursive-by-value aggregates before
//! codegen, so errors here are internal invariants rather than user
//! diagnostics.

#![warn(missing_docs)]

mod layout;
mod vtable;

pub use layout::{
    EnumLayout, FieldLayout, Layout, LayoutCtxt, LayoutError, VariantLayout, tag_layout,
};
pub use vtable::{
    VTABLE_HEADER_SLOTS, VtableEntry, VtableEntryKind, VtableLayout, VtableLayoutError, VtableSlot,
};
