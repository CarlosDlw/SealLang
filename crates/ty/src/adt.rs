//! Nominal aggregate definitions.

use seal_def::DefId;
use seal_symbol::Symbol;

use crate::id::TyId;

/// What kind of aggregate an [`AdtId`](crate::AdtId) names.
///
/// The language has structs today. Enums, tuples, and other product or sum
/// kinds grow here without adding new [`TyKind`](crate::TyKind) variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdtKind {
    /// A named product type declared with `struct`.
    Struct,
}

/// One field of an aggregate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDef {
    /// The field definition, used for spans and diagnostics.
    pub def: DefId,
    /// The declared field name.
    pub name: Symbol,
    /// The declared field type.
    pub ty: TyId,
}

/// The definition of a nominal aggregate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdtDef {
    /// The definition this aggregate was declared by.
    pub def: DefId,
    /// The declared name, for display and diagnostics.
    pub name: Symbol,
    /// What kind of aggregate this is.
    pub kind: AdtKind,
    /// Fields in declaration order (also the layout order).
    pub fields: Vec<FieldDef>,
}

impl AdtDef {
    /// Returns the field with the given definition, if it belongs to this
    /// aggregate.
    pub fn field(&self, def: DefId) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.def == def)
    }

    /// Returns the field with the given name, if it exists.
    pub fn field_by_name(&self, name: Symbol) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.name == name)
    }
}
