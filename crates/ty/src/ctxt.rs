//! Type interning, construction, and the memory-model queries.

use std::cell::RefCell;

use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_index::IndexVec;
use seal_symbol::Symbol;

use crate::adt::{AdtDef, FieldDef};
use crate::id::{AdtId, InferTyId, TyId};
use crate::kind::{FloatTy, IntTy, TyKind, UintTy};

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
    /// Fields are filled later, because a struct may refer to another struct
    /// declared after it. Reserving first lets field types name the aggregate
    /// before its declaration is complete.
    pub fn push_adt(&self, def: DefId, name: Symbol) -> AdtId {
        self.adts.borrow_mut().push(AdtDef {
            def,
            name,
            kind: crate::adt::AdtKind::Struct,
            fields: Vec::new(),
        })
    }

    /// Fills the fields of a reserved aggregate.
    pub fn fill_adt(&self, adt: AdtId, fields: Vec<FieldDef>) {
        self.adts.borrow_mut()[adt].fields = fields;
    }

    /// Replaces one field type, used to quarantine a recursive field more
    /// precisely than replacing the whole aggregate.
    pub fn set_field_ty(&self, adt: AdtId, index: usize, ty: TyId) {
        self.adts.borrow_mut()[adt].fields[index].ty = ty;
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

    /// Returns the type naming `adt`.
    pub fn adt_ty(&self, adt: AdtId) -> TyId {
        self.intern(TyKind::Adt(adt))
    }

    /// Returns the field with the given definition in `adt`.
    pub fn field(&self, adt: AdtId, def: DefId) -> Option<FieldDef> {
        self.adts.borrow()[adt].field(def).copied()
    }

    /// Returns the field with the given name in `adt`.
    pub fn field_by_name(&self, adt: AdtId, name: Symbol) -> Option<FieldDef> {
        self.adts.borrow()[adt].field_by_name(name).copied()
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
        self.intern(TyKind::Ref(pointee))
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
            | TyKind::Ref(_)
            | TyKind::Unit
            | TyKind::Never
            | TyKind::Error => true,
            // A user-defined aggregate owns its fields; it becomes `Copy` only
            // through an explicit opt-in, which the language does not have yet.
            // Its fields remain individually copyable.
            TyKind::Adt(_) | TyKind::Str | TyKind::Infer(_) => false,
        }
    }

    /// Returns `true` if `ty` has a size known at compile time.
    ///
    /// `str` is the only unsized kind in the current language; an unresolved
    /// inference variable reports `false` because its size is not known yet.
    pub fn is_sized(&self, ty: TyId) -> bool {
        self.is_sized_inner(ty, &mut FxHashSet::default())
    }

    fn is_sized_inner(&self, ty: TyId, visiting: &mut FxHashSet<AdtId>) -> bool {
        match self.kind(ty) {
            TyKind::Str | TyKind::Infer(_) => false,
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    // A by-value cycle has no finite size; type checking reports
                    // it and stops before layout.
                    return false;
                }
                let def = self.adt(adt);
                let sized = def
                    .fields
                    .iter()
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
    /// trivially disposable. Aggregates will compute this from their fields and
    /// their own destructor. An unresolved variable reports `false`; the caller
    /// must resolve it first, as with [`TyCtxt::is_copy`].
    pub fn needs_drop(&self, ty: TyId) -> bool {
        self.needs_drop_inner(ty, &mut FxHashSet::default())
    }

    fn needs_drop_inner(&self, ty: TyId, visiting: &mut FxHashSet<AdtId>) -> bool {
        match self.kind(ty) {
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    return false;
                }
                let def = self.adt(adt);
                let needs_drop = def
                    .fields
                    .iter()
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
            TyKind::Ref(_) => true,
            TyKind::Adt(adt) => {
                if !visiting.insert(adt) {
                    return false;
                }
                let def = self.adt(adt);
                let contains = def
                    .fields
                    .iter()
                    .any(|field| self.contains_reference_inner(field.ty, visiting));
                visiting.remove(&adt);
                contains
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::kind::{FloatTy, IntTy, TyKind, UintTy};

    use super::TyCtxt;

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
        assert_eq!(ctxt.kind(ptr), TyKind::Ref(int));
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
        assert_eq!(ctxt.kind(nested), TyKind::Ref(first));
        assert_ne!(nested, first);
        assert_eq!(ctxt.len(), 3);
    }

    #[test]
    fn scalars_and_references_are_copy() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let pointer = ctxt.ref_ty(int);
        for ty in [
            int,
            ctxt.uint_ty(UintTy::U128),
            ctxt.float_ty(FloatTy::F64),
            ctxt.bool_ty(),
            ctxt.char_ty(),
            ctxt.unit_ty(),
            ctxt.never_ty(),
            pointer,
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
}
