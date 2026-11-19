//! Drop glue: the destructors one concrete type runs.
//!
//! Dropping a value is not one call. A type with a declared destructor runs it
//! and then drops its fields; a struct without one drops its fields in
//! declaration order; an enum drops the fields of its active variant only; an
//! array drops every element. Code generation needs that plan per concrete type
//! rather than per drop site, so monomorphization collects one [`DropGlue`] per
//! type that needs dropping and every drop site refers to it.
//!
//! A glue is a shallow recipe: [`DropGlue::destructor`] names the instance to
//! call first, and [`DropGlue::children`] lists the types to drop afterwards,
//! each with its own glue. A child that needs no destructor is never listed, so
//! a glue function only calls glue that does something.

use seal_hir::Hir;
use seal_ty::{Instance, TyCtxt, TyId, TyKind};

/// The plan for dropping one concrete type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropGlue {
    /// The destructor instance to call before any child.
    ///
    /// Present exactly when the type's aggregate declares `impl Drop`. The
    /// instance carries the aggregate's own type arguments.
    pub destructor: Option<Instance>,
    /// The values to drop after the destructor returned, or when there is none.
    pub children: GlueChildren,
}

/// The children a glue drops, in the order it must visit them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GlueChildren {
    /// Nothing follows: the destructor drops the whole value, or no part of it
    /// needs a destructor.
    None,
    /// Tuple or struct fields, in declaration order.
    ///
    /// Only fields that need a destructor appear. A field's index in this list
    /// is not its index in the value: code generation resolves the real path
    /// from the type, not from this position.
    Fields(Vec<TyId>),
    /// Enum variants, indexed by discriminant.
    ///
    /// One entry per variant, in declaration order; an empty entry means that
    /// variant holds nothing to drop.
    Variants(Vec<Vec<TyId>>),
    /// An array's element type and element count.
    Elements {
        /// The element type, which needs a destructor.
        element: TyId,
        /// The number of elements to drop.
        count: u64,
    },
}

/// One collected glue and the type it drops.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropGlueEntry {
    /// The concrete type this glue drops.
    pub ty: TyId,
    /// The destructor call and children.
    pub glue: DropGlue,
}

/// Builds the drop plan for one concrete type.
///
/// Returns `None` when dropping `ty` runs no destructor, which is the common
/// case: scalars, `bool`, `char`, `str`, `()`, `!`, references, slices, and
/// aggregates whose parts are all trivially disposable need no glue at all.
///
/// A type parameter, an inference variable, or the error type also returns
/// `None`. `needs_drop` answers `true` for a parameter conservatively, but
/// monomorphization only sees types with every argument substituted, so one of
/// those reaching this function means an earlier phase left the body
/// incomplete: the collector records no glue and validation rejects the drop
/// site that named it.
pub(crate) fn glue_shape(hir: &Hir, ty: TyId) -> Option<DropGlue> {
    let tcx = hir.ty_ctxt();
    if !tcx.needs_drop(ty) {
        return None;
    }
    match tcx.kind(ty) {
        TyKind::Adt { adt, args } => {
            let destructor = tcx
                .drop_method(adt)
                .map(|method| Instance::new(method, args));
            let definition = tcx.adt(adt);
            let children = if definition.is_enum() {
                let variants: Vec<Vec<TyId>> = definition
                    .variants
                    .iter()
                    .map(|variant| {
                        droppable(
                            tcx,
                            variant
                                .fields
                                .iter()
                                .map(|field| tcx.substitute(field.ty, args)),
                        )
                    })
                    .collect();
                if variants.iter().all(Vec::is_empty) {
                    GlueChildren::None
                } else {
                    GlueChildren::Variants(variants)
                }
            } else {
                let fields = droppable(
                    tcx,
                    definition
                        .fields
                        .iter()
                        .map(|field| tcx.substitute(field.ty, args)),
                );
                if fields.is_empty() {
                    GlueChildren::None
                } else {
                    GlueChildren::Fields(fields)
                }
            };
            Some(DropGlue {
                destructor,
                children,
            })
        }
        TyKind::Tuple(tuple) => {
            let elements = droppable(tcx, tcx.tuple_elements(tuple).into_iter());
            Some(DropGlue {
                destructor: None,
                children: if elements.is_empty() {
                    GlueChildren::None
                } else {
                    GlueChildren::Fields(elements)
                },
            })
        }
        TyKind::Array { element, length } => Some(DropGlue {
            destructor: None,
            children: GlueChildren::Elements {
                element,
                count: length,
            },
        }),
        _ => None,
    }
}

/// Keeps the parts of `types` that need a destructor, in order.
fn droppable(tcx: &TyCtxt, types: impl Iterator<Item = TyId>) -> Vec<TyId> {
    types.filter(|ty| tcx.needs_drop(*ty)).collect()
}
