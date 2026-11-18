//! Typed constant values and compiler-owned allocations.
//!
//! Seal has no unions, raw pointers, or pointer arithmetic, so a constant is a
//! typed tree rather than a byte blob. The tree keeps every value form explicit,
//! which validation and LLVM lowering can match exhaustively.

use seal_def::DefId;
use seal_index::{IndexVec, newtype_index};
use seal_symbol::Symbol;
use seal_ty::{Instance, RefKind, SubstsId, TraitRef, TyId, VariantIdx};

newtype_index! {
    /// Index of a compiler-owned allocation in a [`ConstProgram`](crate::ConstProgram).
    pub struct AllocId;
}

/// A fully evaluated constant value.
#[derive(Clone, Debug, PartialEq)]
pub enum ConstValue {
    /// An integer, stored truncated to its declared width.
    Int {
        /// The value's bits, truncated to the integer width.
        bits: u128,
        /// The integer type.
        ty: TyId,
    },
    /// A floating-point value with its raw IEEE bits.
    Float {
        /// The raw IEEE bit pattern, in the low bits of the word.
        bits: u64,
        /// `f32` or `f64`.
        ty: TyId,
    },
    /// A boolean.
    Bool(bool),
    /// A Unicode scalar value.
    Char(char),
    /// The unit value.
    Unit,
    /// A tuple, in element order.
    Tuple(Vec<ConstValue>),
    /// A fixed-size array, in element order.
    Array(Vec<ConstValue>),
    /// A struct, with every field in declaration order.
    Struct {
        /// The struct definition.
        adt: DefId,
        /// The complete type arguments of the instantiation.
        args: SubstsId,
        /// Every field, in declaration order.
        fields: Vec<(DefId, ConstValue)>,
    },
    /// One enum variant, with every field in declaration order.
    Variant {
        /// The enum definition.
        adt: DefId,
        /// The active variant.
        variant: VariantIdx,
        /// The complete type arguments of the instantiation.
        args: SubstsId,
        /// Every field of the variant, in declaration order.
        fields: Vec<(DefId, ConstValue)>,
    },
    /// A `*str` fat pointer naming an interned string.
    Str {
        /// The unescaped contents.
        value: Symbol,
        /// The `*str` type.
        ty: TyId,
    },
    /// A shared or exclusive reference to a compiler-owned allocation.
    Ref {
        /// The referenced allocation.
        target: AllocId,
        /// Byte offset inside the allocation.
        offset: u64,
        /// The referenced type.
        pointee: TyId,
        /// Whether the reference permits writes.
        kind: RefKind,
    },
    /// A slice fat pointer into a compiler-owned allocation.
    Slice {
        /// The referenced allocation.
        target: AllocId,
        /// Byte offset of the first element inside the allocation.
        offset: u64,
        /// The number of elements.
        length: u64,
        /// The element type.
        element: TyId,
    },
    /// A `*dyn Trait` fat pointer.
    Dyn {
        /// The allocation holding the concrete value.
        data: AllocId,
        /// Byte offset of the concrete value inside its allocation.
        data_offset: u64,
        /// The concrete type the trait object erases.
        concrete: TyId,
        /// The principal trait reference, which records the vtable.
        trait_ref: TraitRef,
    },
}

/// What created an allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AllocSource {
    /// A `static` item.
    Static(DefId),
    /// A `const` item whose value is addressed.
    Const(DefId),
    /// A promoted temporary inside a const or static initializer.
    Promoted {
        /// The definition whose initializer owns the promoted value.
        owner: DefId,
        /// The promotion's position inside that initializer.
        index: u32,
    },
    /// A local of a `const func` frame.
    ConstFnLocal {
        /// The instance being interpreted.
        instance: Instance,
        /// The frame-local index.
        index: u32,
    },
}

/// One compiler-owned allocation.
#[derive(Clone, Debug, PartialEq)]
pub struct ConstAllocation {
    /// The type stored at the allocation's start.
    pub ty: TyId,
    /// The stored value.
    pub value: ConstValue,
    /// Whether writes through a reference are permitted.
    pub mutable: bool,
    /// What created the allocation.
    pub source: AllocSource,
}

/// A resolved pointer into the evaluator's memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConstPointer {
    /// The allocation.
    pub alloc: AllocId,
    /// Byte offset inside the allocation.
    pub offset: u64,
    /// The type stored at the pointer.
    pub ty: TyId,
}

/// One evaluated static item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StaticData {
    /// The static definition.
    pub def: DefId,
    /// The static's declared type.
    pub ty: TyId,
    /// Whether the static is declared `mut`.
    pub mutable: bool,
    /// The allocation holding the static's value.
    pub alloc: AllocId,
}

/// How one item depends on another during evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DependencyKind {
    /// The dependency's value was read.
    Value,
    /// Only the dependency's address was taken.
    Address,
}

/// One dependency edge discovered while evaluating an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dependency {
    /// The item depended on.
    pub target: DefId,
    /// How the item was used.
    pub kind: DependencyKind,
}

/// The key of a trait-object constant, used to collect its vtable.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TraitObjectKey {
    /// The concrete type the trait object erases.
    pub concrete: TyId,
    /// The principal trait reference.
    pub trait_ref: TraitRef,
}

/// The evaluated allocations of one file.
pub type Allocations = IndexVec<AllocId, ConstAllocation>;
