//! Nominal aggregate definitions.

use seal_def::DefId;
use seal_index::Idx;
use seal_symbol::Symbol;

use crate::id::{TyId, VariantIdx};

/// The layout representation of an aggregate.
///
/// A `Rust` aggregate uses the language's own layout. A `C` aggregate is
/// declared with `extern struct` and uses the C representation of the target:
/// fields keep declaration order and C alignment, which is the layout the
/// language already assigns, but only a `C` aggregate may cross an FFI boundary
/// by value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AdtRepr {
    /// The default Seal representation.
    #[default]
    Rust,
    /// The C representation declared by `extern struct`.
    C,
}

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
    /// The synthetic environment of a closure.
    ///
    /// A closure value is a product of its captures, so its environment is
    /// laid out like a struct. The fields are filled by type checking, which is
    /// the first phase that knows the type of each capture.
    Closure,
}

/// One field of an aggregate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldDef {
    /// The field definition, used for spans and diagnostics.
    pub def: DefId,
    /// The declared field name, or `None` for a positional enum variant field.
    pub name: Option<Symbol>,
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
        self.fields.iter().find(|field| field.name == Some(name))
    }

    /// Returns `true` when the variant declares positional fields.
    ///
    /// A unit variant and a named variant with no fields both report `false`;
    /// the parser rejects a positional field list with no fields.
    pub fn is_tuple(&self) -> bool {
        self.fields
            .first()
            .is_some_and(|field| field.name.is_none())
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
    /// The aggregate's layout representation.
    pub repr: AdtRepr,
    /// The number of type parameters the declaration takes.
    ///
    /// Every instantiation must supply exactly this many arguments. Lowering
    /// sets it once, before any field type is instantiated.
    pub generic_count: u32,
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
        self.fields.iter().find(|field| field.name == Some(name))
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

    /// Returns `true` when the aggregate uses the C representation.
    #[inline]
    pub fn is_c_repr(&self) -> bool {
        self.repr == AdtRepr::C
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

    /// Returns `true` for a closure environment.
    #[inline]
    pub fn is_closure(&self) -> bool {
        matches!(self.kind, AdtKind::Closure)
    }
}
