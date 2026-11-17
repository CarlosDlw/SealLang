//! Type interning, construction, and the memory-model queries.

use std::cell::RefCell;

use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_index::IndexVec;
use seal_symbol::Symbol;

use crate::adt::{AdtDef, AdtKind, FieldDef, VariantDef};
use crate::id::{AdtId, InferTyId, TupleId, TyId, VariantIdx};
use crate::kind::{FloatTy, IntTy, RefKind, TyKind, UintTy};

/// Interns types and answers structural queries about them.
///
/// Interning makes type equality a single integer comparison: two types are the
/// same if and only if their [`TyId`]s are equal. Every constructor goes through
/// [`TyCtxt::intern`], so the arena never holds two equal kinds.
///
/// Interners take `&self` because a phase that only borrows the context (the
/// checker, which creates `*T` while typing `&place`) must still be able to
/// produce canonical IDs. The cells are single-threaded and no borrow is held
/// across a call, so the interior mutability is unobservable to callers.
#[derive(Clone, Debug, Default)]
pub struct TyCtxt {
    kinds: RefCell<IndexVec<TyId, TyKind>>,
    interner: RefCell<FxHashMap<TyKind, TyId>>,
    adts: RefCell<IndexVec<AdtId, AdtDef>>,
    tuples: RefCell<IndexVec<TupleId, Vec<TyId>>>,
    tuple_interner: RefCell<FxHashMap<Vec<TyId>, TupleId>>,
}

impl TyCtxt {
    /// Creates an empty context.
    pub fn new() -> Self {
        Self::default()
    }

    /// Interns `kind` and returns its canonical ID.
    pub fn intern(&self, kind: TyKind) -> TyId {
        if let Some(&id) = self.interner.borrow().get(&kind) {
            return id;
        }
        let id = self.kinds.borrow_mut().push(kind);
        self.interner.borrow_mut().insert(kind, id);
        id
    }

    /// Returns the kind of `ty`.
    ///
    /// # Panics
    ///
    /// Panics if `ty` did not come from this context. IDs from another context
    /// are a programming error, not user input.
    pub fn kind(&self, ty: TyId) -> TyKind {
        self.kinds.borrow()[ty]
    }

    /// Looks up the canonical ID of an already interned kind.
    ///
    /// Returns `None` if `kind` was never interned.
    pub fn lookup(&self, kind: TyKind) -> Option<TyId> {
        self.interner.borrow().get(&kind).copied()
    }

    /// Reserves an aggregate slot for `def` and returns its ID.
    ///
    /// Fields and variants are filled later, because an aggregate may refer to
    /// another one declared after it. Reserving first lets member types name the
    /// aggregate before its declaration is complete.
    pub fn push_adt(&self, def: DefId, name: Symbol, kind: AdtKind) -> AdtId {
        self.adts.borrow_mut().push(AdtDef {
            def,
            name,
            kind,
            fields: Vec::new(),
            variants: Vec::new(),
        })
    }

    /// Fills the fields of a reserved struct.
    pub fn fill_adt(&self, adt: AdtId, fields: Vec<FieldDef>) {
        self.adts.borrow_mut()[adt].fields = fields;
    }

    /// Fills the variants of a reserved enum, in declaration order.
    pub fn fill_variants(&self, adt: AdtId, variants: Vec<VariantDef>) {
        self.adts.borrow_mut()[adt].variants = variants;
    }

    /// Replaces one field type, used to quarantine a recursive field more
    /// precisely than replacing the whole aggregate.
    pub fn set_field_ty(&self, adt: AdtId, index: usize, ty: TyId) {
        self.adts.borrow_mut()[adt].fields[index].ty = ty;
    }

    /// Replaces one field type of one enum variant, with the same purpose as
    /// [`TyCtxt::set_field_ty`].
    pub fn set_variant_field_ty(&self, adt: AdtId, variant: VariantIdx, index: usize, ty: TyId) {
        self.adts.borrow_mut()[adt].variants[variant.index() as usize].fields[index].ty = ty;
    }

    /// Returns a copy of the aggregate definition.
    ///
    /// Definitions are immutable once lowering finishes, so a copy keeps the
    /// borrow short without exposing the cell.
    pub fn adt(&self, adt: AdtId) -> AdtDef {
        self.adts.borrow()[adt].clone()
    }

    /// Returns the number of aggregate definitions.
    pub fn adt_len(&self) -> usize {
        self.adts.borrow().len()
    }

    /// Returns the fixed-size array type `[element; length]`.
    pub fn array_ty(&self, element: TyId, length: u64) -> TyId {
        self.intern(TyKind::Array { element, length })
    }

    /// Returns the element type and length of `ty` when it is an array.
    pub fn array_parts(&self, ty: TyId) -> Option<(TyId, u64)> {
        match self.kind(ty) {
            TyKind::Array { element, length } => Some((element, length)),
            _ => None,
        }
    }

    /// Returns the slice type `[element]`.
    pub fn slice_ty(&self, element: TyId) -> TyId {
        self.intern(TyKind::Slice { element })
    }

    /// Returns the element type of `ty` when it is a slice.
    pub fn slice_parts(&self, ty: TyId) -> Option<TyId> {
        match self.kind(ty) {
            TyKind::Slice { element } => Some(element),
            _ => None,
        }
    }

    /// Returns the type naming `adt`.
    pub fn adt_ty(&self, adt: AdtId) -> TyId {
        self.intern(TyKind::Adt(adt))
    }

    /// Returns the tuple type `(elements...)`.
    ///
    /// The empty tuple is the unit type, so `tuple_ty(&[])` returns `()`.  Tuple
    /// types are structural: the element list is interned once and every
    /// occurrence of the same list shares one [`TupleId`].
    pub fn tuple_ty(&self, elements: &[TyId]) -> TyId {
        if elements.is_empty() {
            return self.unit_ty();
        }
        if let Some(&tuple) = self.tuple_interner.borrow().get(elements) {
            return self.intern(TyKind::Tuple(tuple));
        }
        let tuple = self.tuples.borrow_mut().push(elements.to_vec());
        self.tuple_interner
            .borrow_mut()
            .insert(elements.to_vec(), tuple);
        self.intern(TyKind::Tuple(tuple))
    }

    /// Returns the tuple id of `ty` when it is a non-unit tuple.
    pub fn tuple_id(&self, ty: TyId) -> Option<TupleId> {
        match self.kind(ty) {
            TyKind::Tuple(tuple) => Some(tuple),
            _ => None,
        }
    }

    /// Returns the element types of a tuple in declaration order.
    pub fn tuple_elements(&self, tuple: TupleId) -> Vec<TyId> {
        self.tuples.borrow()[tuple].clone()
    }

    /// Returns the number of elements of a tuple.
    pub fn tuple_len(&self, tuple: TupleId) -> usize {
        self.tuples.borrow()[tuple].len()
    }

    /// Returns one element type of a tuple, if the index is in range.
    pub fn tuple_element(&self, tuple: TupleId, index: usize) -> Option<TyId> {
        self.tuples.borrow()[tuple].get(index).copied()
    }

    /// Returns the number of interned tuple element lists.
    pub fn tuple_len_total(&self) -> usize {
        self.tuples.borrow().len()
    }

    /// Returns the struct field with the given definition in `adt`.
    ///
    /// Enum fields live on their variant; use [`TyCtxt::variant_field`] for
    /// those.
    pub fn field(&self, adt: AdtId, def: DefId) -> Option<FieldDef> {
        self.adts.borrow()[adt].field(def).copied()
    }

    /// Returns the struct field with the given name in `adt`.
    pub fn field_by_name(&self, adt: AdtId, name: Symbol) -> Option<FieldDef> {
        self.adts.borrow()[adt].field_by_name(name).copied()
    }

    /// Returns the enum variant at `variant` in `adt`.
    pub fn variant(&self, adt: AdtId, variant: VariantIdx) -> Option<VariantDef> {
        self.adts.borrow()[adt].variant(variant).cloned()
    }

    /// Returns the field with the given definition inside `variant`.
    pub fn variant_field(&self, adt: AdtId, variant: VariantIdx, def: DefId) -> Option<FieldDef> {
        self.adts.borrow()[adt]
            .variant(variant)
            .and_then(|variant| variant.field(def))
            .copied()
    }

    /// Returns the number of interned types.
    pub fn len(&self) -> usize {
        self.kinds.borrow().len()
    }

    /// Returns `true` if no type has been interned.
    pub fn is_empty(&self) -> bool {
        self.kinds.borrow().is_empty()
    }

    /// Returns the `bool` type.
    pub fn bool_ty(&self) -> TyId {
        self.intern(TyKind::Bool)
    }

    /// Returns the `char` type.
    pub fn char_ty(&self) -> TyId {
        self.intern(TyKind::Char)
    }

    /// Returns the unsized `str` type.
    pub fn str_ty(&self) -> TyId {
        self.intern(TyKind::Str)
    }

    /// Returns the unit type `()`.
    pub fn unit_ty(&self) -> TyId {
        self.intern(TyKind::Unit)
    }

    /// Returns the never type `!`.
    pub fn never_ty(&self) -> TyId {
        self.intern(TyKind::Never)
    }

    /// Returns the error type used during recovery.
    pub fn error_ty(&self) -> TyId {
        self.intern(TyKind::Error)
    }

    /// Returns a signed integer type.
    pub fn int_ty(&self, int: IntTy) -> TyId {
        self.intern(TyKind::Int(int))
    }

    /// Returns an unsigned integer type.
    pub fn uint_ty(&self, uint: UintTy) -> TyId {
        self.intern(TyKind::Uint(uint))
    }

    /// Returns a floating-point type.
    pub fn float_ty(&self, float: FloatTy) -> TyId {
        self.intern(TyKind::Float(float))
    }

    /// Returns the shared reference type `*T`.
    pub fn ref_ty(&self, pointee: TyId) -> TyId {
        self.ref_ty_with(RefKind::Shared, pointee)
    }

    /// Returns the exclusive reference type `*mut T`.
    pub fn mut_ref_ty(&self, pointee: TyId) -> TyId {
        self.ref_ty_with(RefKind::Exclusive, pointee)
    }

    /// Returns the reference type with `kind`.
    pub fn ref_ty_with(&self, kind: RefKind, pointee: TyId) -> TyId {
        self.intern(TyKind::Ref { kind, pointee })
    }

    /// Returns the kind and pointee of `ty` when it is a reference.
    pub fn ref_parts(&self, ty: TyId) -> Option<(RefKind, TyId)> {
        match self.kind(ty) {
            TyKind::Ref { kind, pointee } => Some((kind, pointee)),
            _ => None,
        }
    }

    /// Returns the type standing for the inference variable `var`.
    pub fn infer_ty(&self, var: InferTyId) -> TyId {
        self.intern(TyKind::Infer(var))
    }

    /// Returns `true` if values of `ty` are duplicated bitwise instead of moved.
    ///
    /// This is decision D2 of the memory model. An unresolved inference variable
    /// reports `false`: whether it is `Copy` is not known yet, so a caller that
    /// might move the value must resolve the type first.
    pub fn is_copy(&self, ty: TyId) -> bool {
        match self.kind(ty) {
            TyKind::Bool
            | TyKind::Char
            | TyKind::Int(_)
            | TyKind::Uint(_)
            | TyKind::Float(_)
            | TyKind::Ref { .. }
            | TyKind::Unit
            | TyKind::Never
            | TyKind::Error => true,
            // An array copies exactly when its element copies. Unlike a struct,
            // it gains nothing from an explicit opt-in: its elements already
            // decide whether a bitwise copy is safe.
            TyKind::Array { element, .. } => self.is_copy(element),
            // A tuple is a structural product of independent values, so it is
            // `Copy` exactly when every element is. This mirrors arrays and is
            // what makes `(i32, bool)` usable in a loop without a move.
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .all(|element| self.is_copy(*element)),
            // A user-defined aggregate owns its fields; it becomes `Copy` only
            // through an explicit opt-in, which the language does not have yet.
            // Its fields remain individually copyable.
            TyKind::Adt(_) | TyKind::Str | TyKind::Slice { .. } | TyKind::Infer(_) => false,
        }
    }

    /// Returns `true` if `ty` has a size known at compile time.
    ///
    /// `str` and slices are unsized; an unresolved inference variable reports
    /// `false` because its size is not known yet. An enum is sized when every
    /// field of every variant is sized, because any of them may be the variant
    /// the value holds.
    pub fn is_sized(&self, ty: TyId) -> bool {
        self.is_sized_inner(ty, &mut FxHashSet::default())
    }

    fn is_sized_inner(&self, ty: TyId, visiting: &mut FxHashSet<AdtId>) -> bool {
        match self.kind(ty) {
            TyKind::Str | TyKind::Slice { .. } | TyKind::Infer(_) => false,
            TyKind::Array { element, .. } => self.is_sized_inner(element, visiting),
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .all(|element| self.is_sized_inner(*element, visiting)),
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    // A by-value cycle has no finite size; type checking reports
                    // it and stops before layout.
                    return false;
                }
                let def = self.adt(adt);
                let sized = def
                    .all_fields()
                    .all(|field| self.is_sized_inner(field.ty, visiting));
                visiting.remove(&adt);
                sized
            }
            _ => true,
        }
    }

    /// Returns `true` if dropping a value of `ty` runs a destructor.
    ///
    /// No current type has one: scalars, references, `str`, `()`, and `!` are
    /// trivially disposable. An aggregate computes this from every field of
    /// every variant and, later, from its own destructor. An unresolved
    /// variable reports `false`; the caller must resolve it first, as with
    /// [`TyCtxt::is_copy`].
    pub fn needs_drop(&self, ty: TyId) -> bool {
        self.needs_drop_inner(ty, &mut FxHashSet::default())
    }

    fn needs_drop_inner(&self, ty: TyId, visiting: &mut FxHashSet<AdtId>) -> bool {
        match self.kind(ty) {
            TyKind::Array { element, .. } => self.needs_drop_inner(element, visiting),
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .any(|element| self.needs_drop_inner(*element, visiting)),
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    return false;
                }
                let def = self.adt(adt);
                let needs_drop = def
                    .all_fields()
                    .any(|field| self.needs_drop_inner(field.ty, visiting));
                visiting.remove(&adt);
                needs_drop
            }
            _ => false,
        }
    }

    /// Returns `true` if `ty` is, or contains by value, a reference.
    ///
    /// Borrow checking uses this when a value carries loans inside an
    /// aggregate: moving the aggregate must keep those loans alive.
    pub fn contains_reference(&self, ty: TyId) -> bool {
        self.contains_reference_inner(ty, &mut FxHashSet::default())
    }

    fn contains_reference_inner(&self, ty: TyId, visiting: &mut FxHashSet<AdtId>) -> bool {
        match self.kind(ty) {
            TyKind::Ref { .. } => true,
            TyKind::Slice { element } | TyKind::Array { element, .. } => {
                self.contains_reference_inner(element, visiting)
            }
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .any(|element| self.contains_reference_inner(*element, visiting)),
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    return false;
                }
                let def = self.adt(adt);
                let contains = def
                    .all_fields()
                    .any(|field| self.contains_reference_inner(field.ty, visiting));
                visiting.remove(&adt);
                contains
            }
            _ => false,
        }
    }

    /// Returns `true` if `ty` is, or contains by value, an exclusive reference.
    ///
    /// The call-site checker uses this when an aggregate argument carries a
    /// `*mut T` field: passing it grants the callee write access to the field's
    /// pointee, so the storage has to stay disjoint from every exclusive
    /// argument. The walk has the same cycle guard as [`Self::contains_reference`].
    pub fn contains_exclusive_reference(&self, ty: TyId) -> bool {
        self.contains_exclusive_reference_inner(ty, &mut FxHashSet::default())
    }

    fn contains_exclusive_reference_inner(
        &self,
        ty: TyId,
        visiting: &mut FxHashSet<AdtId>,
    ) -> bool {
        match self.kind(ty) {
            TyKind::Ref { kind, .. } => kind.is_exclusive(),
            TyKind::Slice { element } | TyKind::Array { element, .. } => {
                self.contains_exclusive_reference_inner(element, visiting)
            }
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .any(|element| self.contains_exclusive_reference_inner(*element, visiting)),
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    return false;
                }
                let def = self.adt(adt);
                let contains = def
                    .all_fields()
                    .any(|field| self.contains_exclusive_reference_inner(field.ty, visiting));
                visiting.remove(&adt);
                contains
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::adt::{AdtKind, FieldDef, VariantDef};
    use crate::id::VariantIdx;
    use crate::kind::{FloatTy, IntTy, RefKind, TyKind, UintTy};
    use seal_def::DefId;
    use seal_symbol::Interner;

    use super::TyCtxt;

    fn def(index: u32) -> DefId {
        DefId::from_u32(index).expect("test definitions are small")
    }

    #[test]
    fn interning_is_deduplicated() {
        let ctxt = TyCtxt::new();
        let first = ctxt.int_ty(IntTy::I32);
        let second = ctxt.int_ty(IntTy::I32);
        assert_eq!(first, second);
        assert_eq!(ctxt.len(), 1);
        assert!(!ctxt.is_empty());
    }

    #[test]
    fn distinct_kinds_get_distinct_ids() {
        let ctxt = TyCtxt::new();
        let ids = [
            ctxt.bool_ty(),
            ctxt.char_ty(),
            ctxt.str_ty(),
            ctxt.unit_ty(),
            ctxt.never_ty(),
            ctxt.error_ty(),
            ctxt.int_ty(IntTy::I8),
            ctxt.int_ty(IntTy::I64),
            ctxt.uint_ty(UintTy::U8),
            ctxt.uint_ty(UintTy::Usize),
            ctxt.float_ty(FloatTy::F32),
            ctxt.float_ty(FloatTy::F64),
        ];
        let unique: std::collections::BTreeSet<_> = ids.iter().map(|id| id.index()).collect();
        assert_eq!(unique.len(), ids.len());
        assert_eq!(ctxt.len(), ids.len());
    }

    #[test]
    fn kind_round_trips() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I16);
        assert_eq!(ctxt.kind(int), TyKind::Int(IntTy::I16));
        let ptr = ctxt.ref_ty(int);
        assert_eq!(
            ctxt.kind(ptr),
            TyKind::Ref {
                kind: RefKind::Shared,
                pointee: int
            }
        );
        let unit = ctxt.unit_ty();
        let error = ctxt.error_ty();
        assert_eq!(ctxt.kind(unit), TyKind::Unit);
        assert_eq!(ctxt.kind(error), TyKind::Error);
    }

    #[test]
    fn references_are_interned_structurally() {
        let ctxt = TyCtxt::new();
        let inner = ctxt.int_ty(IntTy::I32);
        let first = ctxt.ref_ty(inner);
        let second = ctxt.ref_ty(inner);
        assert_eq!(first, second);

        let nested = ctxt.ref_ty(first);
        assert_eq!(
            ctxt.kind(nested),
            TyKind::Ref {
                kind: RefKind::Shared,
                pointee: first
            }
        );
        assert_ne!(nested, first);
        assert_eq!(ctxt.len(), 3);
    }

    #[test]
    fn reference_kinds_are_distinct_types() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let shared = ctxt.ref_ty(int);
        let exclusive = ctxt.mut_ref_ty(int);

        assert_ne!(shared, exclusive);
        assert_eq!(ctxt.ref_parts(shared), Some((RefKind::Shared, int)));
        assert_eq!(ctxt.ref_parts(exclusive), Some((RefKind::Exclusive, int)));
        assert_eq!(ctxt.ref_parts(int), None);
        assert_eq!(ctxt.ref_ty_with(RefKind::Exclusive, int), exclusive);
    }

    #[test]
    fn scalars_and_references_are_copy() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let pointer = ctxt.ref_ty(int);
        let exclusive = ctxt.mut_ref_ty(int);
        for ty in [
            int,
            ctxt.uint_ty(UintTy::U128),
            ctxt.float_ty(FloatTy::F64),
            ctxt.bool_ty(),
            ctxt.char_ty(),
            ctxt.unit_ty(),
            ctxt.never_ty(),
            pointer,
            exclusive,
        ] {
            assert!(ctxt.is_copy(ty), "{ty:?} must be Copy");
        }
    }

    #[test]
    fn str_is_neither_copy_nor_sized() {
        let ctxt = TyCtxt::new();
        let str_ty = ctxt.str_ty();
        assert!(!ctxt.is_copy(str_ty));
        assert!(!ctxt.is_sized(str_ty));

        let reference = ctxt.ref_ty(str_ty);
        assert!(ctxt.is_copy(reference));
        assert!(ctxt.is_sized(reference));
    }

    #[test]
    fn inference_variables_report_unknown_classification() {
        let mut infer = crate::InferCtxt::new();
        let mut ctxt = TyCtxt::new();
        let var = infer.fresh_ty_var(&mut ctxt);
        assert!(!ctxt.is_copy(var));
        assert!(!ctxt.is_sized(var));
        assert!(!ctxt.needs_drop(var));
    }

    #[test]
    fn no_current_type_needs_drop() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        for ty in [
            int,
            ctxt.str_ty(),
            ctxt.ref_ty(int),
            ctxt.unit_ty(),
            ctxt.never_ty(),
            ctxt.error_ty(),
        ] {
            assert!(!ctxt.needs_drop(ty));
        }
    }

    #[test]
    fn lookup_returns_only_interned_kinds() {
        let ctxt = TyCtxt::new();
        assert_eq!(ctxt.lookup(TyKind::Bool), None);
        let bool_ty = ctxt.bool_ty();
        assert_eq!(ctxt.lookup(TyKind::Bool), Some(bool_ty));
        assert_eq!(ctxt.lookup(TyKind::Error), None);
    }

    #[test]
    fn constructors_are_stable() {
        let ctxt = TyCtxt::new();
        let first = ctxt.bool_ty();
        let second = ctxt.bool_ty();
        assert_eq!(first, second);
        assert_eq!(ctxt.kind(first), TyKind::Bool);
    }

    #[test]
    fn arrays_derive_their_properties_from_the_element() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let array = ctxt.array_ty(int, 4);
        assert_eq!(
            ctxt.kind(array),
            TyKind::Array {
                element: int,
                length: 4
            }
        );
        assert_eq!(ctxt.array_parts(array), Some((int, 4)));
        assert!(ctxt.is_copy(array));
        assert!(ctxt.is_sized(array));
        assert!(!ctxt.needs_drop(array));
        assert!(!ctxt.contains_reference(array));
        assert!(!ctxt.contains_exclusive_reference(array));

        let reference = ctxt.ref_ty(int);
        let references = ctxt.array_ty(reference, 2);
        assert!(ctxt.contains_reference(references));
        assert!(!ctxt.contains_exclusive_reference(references));

        let exclusive = ctxt.mut_ref_ty(int);
        let exclusives = ctxt.array_ty(exclusive, 2);
        assert!(ctxt.contains_reference(exclusives));
        assert!(ctxt.contains_exclusive_reference(exclusives));

        let str_array = ctxt.array_ty(ctxt.str_ty(), 1);
        assert!(!ctxt.is_copy(str_array));
        assert!(!ctxt.is_sized(str_array));
    }

    #[test]
    fn enum_properties_cover_every_variant() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = ctxt.push_adt(def(0), interner.intern("Shape"), AdtKind::Enum);
        let int = ctxt.int_ty(IntTy::I32);
        let shared = ctxt.ref_ty(int);
        let exclusive = ctxt.mut_ref_ty(int);
        ctxt.fill_variants(
            adt,
            vec![
                VariantDef {
                    def: def(1),
                    name: interner.intern("Empty"),
                    fields: Vec::new(),
                },
                VariantDef {
                    def: def(2),
                    name: interner.intern("Shared"),
                    fields: vec![FieldDef {
                        def: def(3),
                        name: Some(interner.intern("value")),
                        ty: shared,
                    }],
                },
                VariantDef {
                    def: def(4),
                    name: interner.intern("Exclusive"),
                    fields: vec![FieldDef {
                        def: def(5),
                        name: Some(interner.intern("value")),
                        ty: exclusive,
                    }],
                },
            ],
        );

        let ty = ctxt.adt_ty(adt);
        assert!(ctxt.is_sized(ty));
        assert!(!ctxt.is_copy(ty));
        assert!(!ctxt.needs_drop(ty));
        assert!(ctxt.contains_reference(ty));
        assert!(ctxt.contains_exclusive_reference(ty));

        let variant = VariantIdx::from_u32(1).expect("small index");
        let definition = ctxt.variant(adt, variant).expect("the variant exists");
        assert_eq!(definition.fields.len(), 1);
        assert_eq!(
            ctxt.variant_field(adt, variant, definition.fields[0].def)
                .expect("the field exists")
                .ty,
            shared
        );
    }

    #[test]
    fn an_enum_is_unsized_when_any_variant_field_is_unsized() {
        let ctxt = TyCtxt::new();
        let interner = Interner::new();
        let adt = ctxt.push_adt(def(0), interner.intern("Bad"), AdtKind::Enum);
        ctxt.fill_variants(
            adt,
            vec![VariantDef {
                def: def(1),
                name: interner.intern("Text"),
                fields: vec![FieldDef {
                    def: def(2),
                    name: Some(interner.intern("text")),
                    ty: ctxt.str_ty(),
                }],
            }],
        );
        assert!(!ctxt.is_sized(ctxt.adt_ty(adt)));
    }
}
