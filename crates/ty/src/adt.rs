//! Nominal aggregate definitions.

use seal_def::DefId;
use seal_index::Idx;
use seal_symbol::Symbol;

use crate::id::{TyId, VariantIdx};

/// What kind of aggregate an [`AdtId`](crate::AdtId) names.
///
/// Tuples and other product or sum kinds grow here without adding new
/// [`TyKind`](crate::TyKind) variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AdtKind {
    /// A named product type declared with `struct`.
    Struct,
    /// A named sum type declared with `enum`.
    Enum,
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

/// One variant of an enum.
///
/// A variant owns its field list: fields of different variants may share a name
/// and are distinct definitions, so `Shape::Circle.radius` and
/// `Shape::Ring.radius` never collide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VariantDef {
    /// The definition this variant was declared by.
    pub def: DefId,
    /// The declared variant name.
    pub name: Symbol,
    /// Fields in declaration order (also the layout order inside the variant).
    pub fields: Vec<FieldDef>,
}

impl VariantDef {
    /// Returns the field with the given definition, if it belongs to this
    /// variant.
    pub fn field(&self, def: DefId) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.def == def)
    }

    /// Returns the field with the given name, if it exists.
    pub fn field_by_name(&self, name: Symbol) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// Returns `true` if the variant carries no fields.
    #[inline]
    pub fn is_unit(&self) -> bool {
        self.fields.is_empty()
    }
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
    /// Struct fields in declaration order (also the layout order).
    ///
    /// Always empty for an enum: its fields live on each [`VariantDef`].
    pub fields: Vec<FieldDef>,
    /// Enum variants in declaration order (also the discriminant order).
    ///
    /// Always empty for a struct.
    pub variants: Vec<VariantDef>,
}

impl AdtDef {
    /// Returns the field with the given definition, if it belongs to this
    /// struct.
    pub fn field(&self, def: DefId) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.def == def)
    }

    /// Returns the field with the given name, if it exists in this struct.
    pub fn field_by_name(&self, name: Symbol) -> Option<&FieldDef> {
        self.fields.iter().find(|field| field.name == name)
    }

    /// Returns the variant at `index`, if it exists.
    pub fn variant(&self, index: VariantIdx) -> Option<&VariantDef> {
        self.variants.get(index.index() as usize)
    }

    /// Returns the variant with the given name, if it exists.
    pub fn variant_by_name(&self, name: Symbol) -> Option<&VariantDef> {
        self.variants.iter().find(|variant| variant.name == name)
    }

    /// Returns the index of the variant with the given definition.
    pub fn variant_index(&self, def: DefId) -> Option<VariantIdx> {
        self.variants
            .iter()
            .position(|variant| variant.def == def)
            .map(VariantIdx::from_usize)
    }

    /// Iterates over every field of the aggregate.
    ///
    /// A struct yields its fields; an enum yields the fields of every variant,
    /// each attributed to its own variant. Queries that care about whether a
    /// value of the type can hold something (sizedness, references, drop) walk
    /// this iterator instead of the struct-only field list.
    pub fn all_fields(&self) -> impl Iterator<Item = &FieldDef> {
        self.fields.iter().chain(
            self.variants
                .iter()
                .flat_map(|variant| variant.fields.iter()),
        )
    }

    /// Returns `true` for a struct declaration.
    #[inline]
    pub fn is_struct(&self) -> bool {
        matches!(self.kind, AdtKind::Struct)
    }

    /// Returns `true` for an enum declaration.
    #[inline]
    pub fn is_enum(&self) -> bool {
        matches!(self.kind, AdtKind::Enum)
    }
}
