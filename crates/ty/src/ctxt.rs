//! Type interning, construction, and the memory-model queries.

use std::cell::RefCell;

use seal_def::DefId;
use seal_hash::{FxHashMap, FxHashSet};
use seal_index::IndexVec;
use seal_symbol::Symbol;

use crate::adt::{AdtDef, AdtKind, FieldDef, VariantDef};
use crate::drop::DropImpls;
use crate::generics::{
    GenericParamDef, Generics, ParamEnv, ParamTy, TraitDef, TraitImplDef, TraitRef,
};
use crate::id::{AdtId, InferTyId, ParamTyId, SubstsId, TupleId, TyId, VariantIdx};
use crate::kind::{FloatTy, IntTy, RawPtrKind, RefKind, TyKind, UintTy};

/// The outer constructor of an impl target or trait goal.
///
/// Two types can only be equal when their heads are equal, so grouping impls
/// by trait and head rejects every non-candidate before structural matching.
/// A blanket impl has a parameter head and is kept in a separate list, because
/// it may apply to a goal with any head.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImplHead {
    /// A struct or enum.
    Adt(AdtId),
    /// A signed integer.
    Int(IntTy),
    /// An unsigned integer.
    Uint(UintTy),
    /// A floating-point number.
    Float(FloatTy),
    /// `bool`.
    Bool,
    /// `char`.
    Char,
    /// `str`.
    Str,
    /// `()`.
    Unit,
    /// The never type.
    Never,
    /// A shared or exclusive reference.
    Ref(RefKind),
    /// A read-only or writable raw pointer.
    RawPtr(RawPtrKind),
    /// An array, independent of its length.
    Array,
    /// A slice.
    Slice,
    /// A trait object.
    Dyn,
    /// A tuple with the given arity.
    Tuple(usize),
    /// A declared type parameter, which makes the impl a blanket one.
    Param,
    /// An unresolved inference variable.
    Infer,
    /// The error recovery type.
    Error,
}

impl TyCtxt {
    fn impl_head(&self, ty: TyId) -> ImplHead {
        match self.kind(ty) {
            TyKind::Adt { adt, .. } => ImplHead::Adt(adt),
            TyKind::Int(int) => ImplHead::Int(int),
            TyKind::Uint(uint) => ImplHead::Uint(uint),
            TyKind::Float(float) => ImplHead::Float(float),
            TyKind::Bool => ImplHead::Bool,
            TyKind::Char => ImplHead::Char,
            TyKind::Str => ImplHead::Str,
            TyKind::Unit => ImplHead::Unit,
            TyKind::Never => ImplHead::Never,
            TyKind::Ref { kind, .. } => ImplHead::Ref(kind),
            TyKind::RawPtr { kind, .. } => ImplHead::RawPtr(kind),
            TyKind::Array { .. } => ImplHead::Array,
            TyKind::Slice { .. } => ImplHead::Slice,
            TyKind::Dyn { .. } => ImplHead::Dyn,
            TyKind::Tuple(tuple) => ImplHead::Tuple(self.tuple_elements(tuple).len()),
            TyKind::Param(_) => ImplHead::Param,
            TyKind::Infer(_) => ImplHead::Infer,
            TyKind::Error => ImplHead::Error,
        }
    }
}

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
    params: RefCell<IndexVec<ParamTyId, ParamTy>>,
    param_interner: RefCell<FxHashMap<ParamTy, ParamTyId>>,
    substs_arena: RefCell<IndexVec<SubstsId, Vec<TyId>>>,
    substs_interner: RefCell<FxHashMap<Vec<TyId>, SubstsId>>,
    generics: RefCell<FxHashMap<DefId, Generics>>,
    param_envs: RefCell<FxHashMap<DefId, ParamEnv>>,
    trait_defs: RefCell<FxHashMap<DefId, TraitDef>>,
    trait_impls: RefCell<Vec<TraitImplDef>>,
    trait_impl_index: RefCell<FxHashMap<(DefId, ImplHead), Vec<usize>>>,
    blanket_trait_impls: RefCell<Vec<usize>>,
    /// The `impl Drop` destructor of every aggregate that declares one.
    ///
    /// Lowering fills this after validating the implementation; see
    /// [`crate::drop`]. A query on a type never observes a half-filled registry,
    /// because lowering finishes before type checking starts.
    drop_impls: RefCell<DropImpls>,
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

    /// Interns a declared type parameter and returns its canonical ID.
    ///
    /// The key includes the parameter's definition, owner, flattened index, and
    /// name. Lowering must not create two different names for the same owner and
    /// index; doing so is an internal invariant violation.
    pub fn push_param(&self, def: DefId, owner: DefId, index: u32, name: Symbol) -> ParamTyId {
        let data = ParamTy {
            def,
            owner,
            index,
            name,
        };
        if let Some(&id) = self.param_interner.borrow().get(&data) {
            return id;
        }
        let id = self.params.borrow_mut().push(data);
        self.param_interner.borrow_mut().insert(data, id);
        id
    }

    /// Returns the declaration data of a type parameter.
    pub fn param_data(&self, param: ParamTyId) -> ParamTy {
        self.params.borrow()[param]
    }

    /// Returns the type standing for a declared parameter.
    pub fn param_ty(&self, param: ParamTyId) -> TyId {
        self.intern(TyKind::Param(param))
    }

    /// Returns the number of declared parameters interned so far.
    pub fn param_len(&self) -> usize {
        self.params.borrow().len()
    }

    /// Interns an ordered list of type arguments.
    ///
    /// The empty list is canonical and is shared by every non-generic use.
    pub fn substs(&self, args: &[TyId]) -> SubstsId {
        if let Some(&id) = self.substs_interner.borrow().get(args) {
            return id;
        }
        let id = self.substs_arena.borrow_mut().push(args.to_vec());
        self.substs_interner.borrow_mut().insert(args.to_vec(), id);
        id
    }

    /// Returns the canonical empty substitution list.
    pub fn empty_substs(&self) -> SubstsId {
        self.substs(&[])
    }

    /// Returns the type arguments in `substs`, in declaration order.
    pub fn substs_args(&self, substs: SubstsId) -> Vec<TyId> {
        self.substs_arena.borrow()[substs].clone()
    }

    /// Returns the number of type arguments in `substs`.
    pub fn substs_len(&self, substs: SubstsId) -> usize {
        self.substs_arena.borrow()[substs].len()
    }

    /// Returns the number of interned substitution lists.
    pub fn substs_len_total(&self) -> usize {
        self.substs_arena.borrow().len()
    }

    /// Records the generic parameters declared by `generics.owner`.
    ///
    /// Lowering registers every owner before any type in its declaration is
    /// lowered, so a type path can resolve a parameter by name while the
    /// declaration is still being processed.
    pub fn insert_generics(&self, generics: Generics) {
        self.generics.borrow_mut().insert(generics.owner, generics);
    }

    /// Returns the generic parameters declared by `owner`.
    pub fn generics(&self, owner: DefId) -> Option<Generics> {
        self.generics.borrow().get(&owner).cloned()
    }

    /// Returns the flattened parameters visible from `owner`, outermost first.
    ///
    /// The returned list follows the parent chain, so a method inside an impl
    /// sees the impl's parameters before its own. A trait's implicit `Self`
    /// parameter is included because it is part of the trait's parameter list.
    pub fn flattened_generics(&self, owner: DefId) -> Vec<GenericParamDef> {
        let mut chain = Vec::new();
        let mut current = Some(owner);
        while let Some(def) = current {
            let Some(generics) = self.generics(def) else {
                break;
            };
            current = generics.parent;
            chain.push(generics);
        }
        chain.reverse();
        let mut params = Vec::new();
        for generics in chain {
            params.extend(generics.params);
        }
        params
    }

    /// Records the bounds visible while checking `owner`.
    pub fn set_param_env(&self, owner: DefId, env: ParamEnv) {
        self.param_envs.borrow_mut().insert(owner, env);
    }

    /// Returns the bounds visible while checking `owner`.
    pub fn param_env(&self, owner: DefId) -> ParamEnv {
        self.param_envs
            .borrow()
            .get(&owner)
            .cloned()
            .unwrap_or_default()
    }

    /// Records a semantic trait declaration.
    pub fn insert_trait_def(&self, definition: TraitDef) {
        self.trait_defs
            .borrow_mut()
            .insert(definition.def, definition);
    }

    /// Returns a semantic trait declaration.
    pub fn trait_def(&self, def: DefId) -> Option<TraitDef> {
        self.trait_defs.borrow().get(&def).cloned()
    }

    /// Records one trait implementation in declaration order.
    ///
    /// The implementation is also indexed by trait and self head, so a proof
    /// goal only considers candidates with the same outer constructor. Blanket
    /// impls have a parameter head and are kept separately, because they may
    /// apply to a goal with any head.
    pub fn push_trait_impl(&self, implementation: TraitImplDef) {
        let index = self.trait_impls.borrow().len();
        let head = self.impl_head(implementation.trait_ref.self_ty);
        if head == ImplHead::Param {
            self.blanket_trait_impls.borrow_mut().push(index);
        } else {
            self.trait_impl_index
                .borrow_mut()
                .entry((implementation.trait_ref.trait_def, head))
                .or_default()
                .push(index);
        }
        self.trait_impls.borrow_mut().push(implementation);
    }

    /// Returns every trait implementation in declaration order.
    pub fn trait_impls(&self) -> Vec<TraitImplDef> {
        self.trait_impls.borrow().clone()
    }

    /// Returns the impls of `trait_def` that may apply to `self_ty`.
    ///
    /// The result keeps declaration order and includes blanket impls. A head
    /// that cannot match is rejected without inspecting the impl header.
    pub fn trait_impls_for(&self, trait_def: DefId, self_ty: TyId) -> Vec<TraitImplDef> {
        let head = self.impl_head(self_ty);
        let impls = self.trait_impls.borrow();
        let index = self.trait_impl_index.borrow();
        let mut matches: Vec<usize> = index.get(&(trait_def, head)).cloned().unwrap_or_default();
        for candidate in self.blanket_trait_impls.borrow().iter() {
            if impls[*candidate].trait_ref.trait_def == trait_def {
                matches.push(*candidate);
            }
        }
        matches.sort_unstable();
        matches
            .into_iter()
            .map(|position| impls[position].clone())
            .collect()
    }

    /// Returns every impl that may apply to a self type with the head of `self_ty`.
    ///
    /// Method resolution uses this before knowing the trait: the head filters
    /// the candidates, and the method name filters the rest.
    pub fn trait_impls_matching(&self, self_ty: TyId) -> Vec<TraitImplDef> {
        let head = self.impl_head(self_ty);
        let impls = self.trait_impls.borrow();
        let index = self.trait_impl_index.borrow();
        let mut matches: Vec<usize> = Vec::new();
        for (key, indices) in index.iter() {
            if key.1 == head {
                matches.extend(indices.iter().copied());
            }
        }
        matches.extend(self.blanket_trait_impls.borrow().iter().copied());
        matches.sort_unstable();
        matches
            .into_iter()
            .map(|position| impls[position].clone())
            .collect()
    }

    /// Replaces every declared parameter in `ty` using `substs`.
    ///
    /// A parameter is replaced by the argument at its flattened index. The
    /// substitution list must cover every parameter that occurs in `ty`; an
    /// out-of-range index is an internal invariant violation, not user input.
    pub fn substitute(&self, ty: TyId, substs: SubstsId) -> TyId {
        crate::substs::substitute(self, ty, substs)
    }

    /// Returns the declared parameters that occur in `ty`, first occurrence
    /// first. Duplicate occurrences are removed.
    pub fn free_params(&self, ty: TyId) -> Vec<ParamTyId> {
        crate::substs::free_params(self, ty)
    }

    /// Returns `true` if `ty` mentions a declared parameter.
    pub fn contains_param(&self, ty: TyId) -> bool {
        crate::substs::contains_param(self, ty)
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
            generic_count: 0,
            fields: Vec::new(),
            variants: Vec::new(),
        })
    }

    /// Records how many type parameters an aggregate declaration takes.
    ///
    /// Lowering calls this once, when it finishes the declaration's generic
    /// list. Every later [`TyCtxt::adt_ty_with`] must match this arity.
    pub fn set_generic_count(&self, adt: AdtId, generic_count: u32) {
        self.adts.borrow_mut()[adt].generic_count = generic_count;
    }

    /// Returns the number of type parameters an aggregate declaration takes.
    pub fn adt_generic_count(&self, adt: AdtId) -> u32 {
        self.adts.borrow()[adt].generic_count
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

    /// Returns the trait object `dyn trait_def<args>`.
    ///
    /// The identity is only the trait declaration and its own arguments; the
    /// concrete implementing type is erased. The trait's implicit `Self` is not
    /// part of `args`, so a known trait must receive exactly one argument fewer
    /// than its declared parameter list.
    pub fn dyn_ty(&self, trait_def: DefId, args: SubstsId) -> TyId {
        if let Some(definition) = self.trait_def(trait_def) {
            let expected = definition.generics.params.len().saturating_sub(1);
            let found = self.substs_len(args);
            assert_eq!(
                found, expected,
                "trait {trait_def:?} expects {expected} type arguments, but received {found}"
            );
        }
        self.intern(TyKind::Dyn { trait_def, args })
    }

    /// Returns the principal trait and arguments of `ty` when it is a trait object.
    pub fn dyn_parts(&self, ty: TyId) -> Option<(DefId, SubstsId)> {
        match self.kind(ty) {
            TyKind::Dyn { trait_def, args } => Some((trait_def, args)),
            _ => None,
        }
    }

    /// Returns the principal trait followed by every reachable supertrait.
    ///
    /// The order is depth-first through supertraits in declaration order, the
    /// same order used to flatten vtable methods. A trait reference reached
    /// through several paths appears once; the same trait with different
    /// arguments is a distinct entry. A trait cycle is cut off defensively even
    /// though the declaration checker already rejects it.
    pub fn dyn_trait_refs(&self, ty: TyId) -> Vec<TraitRef> {
        let Some((trait_def, args)) = self.dyn_parts(ty) else {
            return Vec::new();
        };
        let mut refs = Vec::new();
        let mut seen = FxHashSet::default();
        let mut active = FxHashSet::default();
        self.collect_dyn_trait_refs(
            TraitRef::new(trait_def, ty, args),
            &mut refs,
            &mut seen,
            &mut active,
        );
        refs
    }

    fn collect_dyn_trait_refs(
        &self,
        current: TraitRef,
        refs: &mut Vec<TraitRef>,
        seen: &mut FxHashSet<TraitRef>,
        active: &mut FxHashSet<DefId>,
    ) {
        if !seen.insert(current) {
            return;
        }
        refs.push(current);
        if !active.insert(current.trait_def) {
            return;
        }
        if let Some(definition) = self.trait_def(current.trait_def) {
            for supertrait in &definition.supertraits {
                if let Some(instantiated) = self.instantiate_supertrait(*supertrait, current) {
                    self.collect_dyn_trait_refs(instantiated, refs, seen, active);
                }
            }
        }
        active.remove(&current.trait_def);
    }

    /// Replaces a trait's `Self` and own arguments in one of its supertraits.
    ///
    /// Returns `None` when the trait declaration is missing or its generic
    /// arity disagrees with `base`, which only malformed internal state can do.
    pub fn instantiate_supertrait(&self, supertrait: TraitRef, base: TraitRef) -> Option<TraitRef> {
        let definition = self.trait_def(base.trait_def)?;
        let mut arguments = Vec::with_capacity(definition.generics.params.len());
        arguments.push(base.self_ty);
        arguments.extend(self.substs_args(base.args));
        if arguments.len() != definition.generics.params.len() {
            return None;
        }
        let substs = self.substs(&arguments);
        let self_ty = self.substitute(supertrait.self_ty, substs);
        let args: Vec<TyId> = self
            .substs_args(supertrait.args)
            .into_iter()
            .map(|argument| self.substitute(argument, substs))
            .collect();
        Some(TraitRef::new(
            supertrait.trait_def,
            self_ty,
            self.substs(&args),
        ))
    }

    /// Returns the type naming non-generic `adt`.
    pub fn adt_ty(&self, adt: AdtId) -> TyId {
        self.adt_ty_with(adt, self.empty_substs())
    }

    /// Returns the type naming `adt` with `args`.
    ///
    /// # Panics
    ///
    /// Panics if `args` does not match the declaration's generic arity. This is
    /// an internal invariant: HIR and type checking reject an arity mismatch
    /// before any semantic type is built.
    pub fn adt_ty_with(&self, adt: AdtId, args: SubstsId) -> TyId {
        let expected = self.adt_generic_count(adt) as usize;
        let found = self.substs_len(args);
        assert_eq!(
            found, expected,
            "aggregate {adt:?} expects {expected} type arguments, but received {found}"
        );
        self.intern(TyKind::Adt { adt, args })
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

    /// Returns the struct field template with the given definition in `adt`.
    ///
    /// Enum fields live on their variant; use [`TyCtxt::variant_field`] for
    /// those. Use [`TyCtxt::field_ty`] when the instantiated type is needed.
    pub fn field(&self, adt: AdtId, def: DefId) -> Option<FieldDef> {
        self.adts.borrow()[adt].field(def).copied()
    }

    /// Returns the struct field with the given name in `adt`, as a template.
    pub fn field_by_name(&self, adt: AdtId, name: Symbol) -> Option<FieldDef> {
        self.adts.borrow()[adt].field_by_name(name).copied()
    }

    /// Returns the instantiated type of a struct field.
    pub fn field_ty(&self, adt: AdtId, args: SubstsId, def: DefId) -> Option<TyId> {
        self.field(adt, def)
            .map(|field| self.substitute(field.ty, args))
    }

    /// Returns the instantiated type of a struct field selected by name.
    pub fn field_by_name_ty(&self, adt: AdtId, args: SubstsId, name: Symbol) -> Option<TyId> {
        self.field_by_name(adt, name)
            .map(|field| self.substitute(field.ty, args))
    }

    /// Returns the enum variant at `variant` in `adt`.
    pub fn variant(&self, adt: AdtId, variant: VariantIdx) -> Option<VariantDef> {
        self.adts.borrow()[adt].variant(variant).cloned()
    }

    /// Returns the field template with the given definition inside `variant`.
    pub fn variant_field(&self, adt: AdtId, variant: VariantIdx, def: DefId) -> Option<FieldDef> {
        self.adts.borrow()[adt]
            .variant(variant)
            .and_then(|variant| variant.field(def))
            .copied()
    }

    /// Returns the instantiated type of an enum variant field.
    pub fn variant_field_ty(
        &self,
        adt: AdtId,
        args: SubstsId,
        variant: VariantIdx,
        def: DefId,
    ) -> Option<TyId> {
        self.variant_field(adt, variant, def)
            .map(|field| self.substitute(field.ty, args))
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

    /// Returns the read-only raw pointer type `*raw T`.
    pub fn raw_ptr_ty(&self, pointee: TyId) -> TyId {
        self.raw_ptr_ty_with(RawPtrKind::Const, pointee)
    }

    /// Returns the writable raw pointer type `*raw mut T`.
    pub fn raw_mut_ptr_ty(&self, pointee: TyId) -> TyId {
        self.raw_ptr_ty_with(RawPtrKind::Mut, pointee)
    }

    /// Returns the raw pointer type with `kind`.
    pub fn raw_ptr_ty_with(&self, kind: RawPtrKind, pointee: TyId) -> TyId {
        self.intern(TyKind::RawPtr { kind, pointee })
    }

    /// Returns the kind and pointee of `ty` when it is a raw pointer.
    pub fn raw_parts(&self, ty: TyId) -> Option<(RawPtrKind, TyId)> {
        match self.kind(ty) {
            TyKind::RawPtr { kind, pointee } => Some((kind, pointee)),
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
    /// might move the value must resolve the type first. A declared parameter is
    /// not `Copy` without a `Copy` bound; use [`TyCtxt::is_copy_with`] when a
    /// parameter environment is available.
    pub fn is_copy(&self, ty: TyId) -> bool {
        self.is_copy_with(ty, &ParamEnv::new())
    }

    /// Returns `true` if values of `ty` are duplicated bitwise under `env`.
    ///
    /// Only a parameter's `Copy` classification depends on the environment;
    /// concrete types use the same structural rules as [`TyCtxt::is_copy`].
    pub fn is_copy_with(&self, ty: TyId, env: &ParamEnv) -> bool {
        match self.kind(ty) {
            TyKind::Bool
            | TyKind::Char
            | TyKind::Int(_)
            | TyKind::Uint(_)
            | TyKind::Float(_)
            | TyKind::Ref { .. }
            | TyKind::RawPtr { .. }
            | TyKind::Unit
            | TyKind::Never
            | TyKind::Error => true,
            // An array copies exactly when its element copies. Unlike a struct,
            // it gains nothing from an explicit opt-in: its elements already
            // decide whether a bitwise copy is safe.
            TyKind::Array { element, .. } => self.is_copy_with(element, env),
            // A tuple is a structural product of independent values, so it is
            // `Copy` exactly when every element is. This mirrors arrays and is
            // what makes `(i32, bool)` usable in a loop without a move.
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .all(|element| self.is_copy_with(*element, env)),
            TyKind::Param(param) => env.has_copy(param),
            // A user-defined aggregate owns its fields; it becomes `Copy` only
            // through an explicit opt-in, which the language does not have yet.
            // Its fields remain individually copyable.
            TyKind::Adt { .. }
            | TyKind::Str
            | TyKind::Slice { .. }
            | TyKind::Dyn { .. }
            | TyKind::Infer(_) => false,
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

    fn is_sized_inner(&self, ty: TyId, visiting: &mut FxHashSet<(AdtId, SubstsId)>) -> bool {
        match self.kind(ty) {
            TyKind::Str | TyKind::Slice { .. } | TyKind::Dyn { .. } | TyKind::Infer(_) => false,
            TyKind::Param(_) => true,
            TyKind::Array { element, .. } => self.is_sized_inner(element, visiting),
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .all(|element| self.is_sized_inner(*element, visiting)),
            TyKind::Adt { adt, args } => {
                let key = (adt, args);
                if !visiting.insert(key) {
                    // A by-value cycle has no finite size; type checking reports
                    // it and stops before layout.
                    return false;
                }
                let def = self.adt(adt);
                let mut sized = true;
                for field in def.all_fields() {
                    let field_ty = self.substitute(field.ty, args);
                    if !self.is_sized_inner(field_ty, visiting) {
                        sized = false;
                        break;
                    }
                }
                visiting.remove(&key);
                sized
            }
            _ => true,
        }
    }

    /// Records the destructor the aggregate `adt` declares.
    ///
    /// Lowering calls this once per validated `impl Drop for T`, after checking
    /// that the implementation covers the whole declaration. The returned method
    /// is the destructor previously recorded for the same aggregate, which lets
    /// the caller reject a duplicate instead of replacing one silently.
    #[must_use]
    pub fn set_drop_method(&self, adt: AdtId, method: DefId) -> Option<DefId> {
        self.drop_impls.borrow_mut().insert(adt, method)
    }

    /// Returns the destructor `adt` declares, if any.
    ///
    /// The method is the body of the aggregate's `impl Drop`; its instantiation
    /// for one type argument list is what the drop glue calls.
    pub fn drop_method(&self, adt: AdtId) -> Option<DefId> {
        self.drop_impls.borrow().get(adt)
    }

    /// Returns `true` if dropping a value of `ty` runs a destructor.
    ///
    /// A declared destructor makes an aggregate answer `true` on its own; a
    /// reference to any such aggregate does not, because dropping a reference
    /// never drops the pointee. Scalars, `bool`, `char`, `str`, `()`, `!`,
    /// references, and slices are trivially disposable. Arrays and tuples
    /// propagate from their elements, and an aggregate without a destructor
    /// spreads the question over every field of every variant.
    ///
    /// A type parameter reports `true`, conservatively: monomorphization
    /// replaces it with a concrete type, whose drop glue is empty when that type
    /// has no destructor. Reporting `false` would skip the destructor of every
    /// type argument that has one. A trait object reports `false` because it
    /// never exists by value. An unresolved variable reports `false`; the caller
    /// must resolve it first, as with [`TyCtxt::is_copy`].
    pub fn needs_drop(&self, ty: TyId) -> bool {
        self.needs_drop_inner(ty, &mut FxHashSet::default())
    }

    fn needs_drop_inner(&self, ty: TyId, visiting: &mut FxHashSet<(AdtId, SubstsId)>) -> bool {
        match self.kind(ty) {
            TyKind::Param(_) => true,
            TyKind::Array { element, .. } => self.needs_drop_inner(element, visiting),
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .any(|element| self.needs_drop_inner(*element, visiting)),
            TyKind::Adt { adt, args } => {
                // A declared destructor answers before the fields: its glue runs
                // the destructor and then drops the fields itself.
                if self.drop_method(adt).is_some() {
                    return true;
                }
                let key = (adt, args);
                if !visiting.insert(key) {
                    return false;
                }
                let def = self.adt(adt);
                let mut needs_drop = false;
                for field in def.all_fields() {
                    let field_ty = self.substitute(field.ty, args);
                    if self.needs_drop_inner(field_ty, visiting) {
                        needs_drop = true;
                        break;
                    }
                }
                visiting.remove(&key);
                needs_drop
            }
            TyKind::Dyn { .. } => false,
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

    fn contains_reference_inner(
        &self,
        ty: TyId,
        visiting: &mut FxHashSet<(AdtId, SubstsId)>,
    ) -> bool {
        match self.kind(ty) {
            TyKind::Ref { .. } | TyKind::Param(_) => true,
            TyKind::Slice { element } | TyKind::Array { element, .. } => {
                self.contains_reference_inner(element, visiting)
            }
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .any(|element| self.contains_reference_inner(*element, visiting)),
            TyKind::Dyn { args, .. } => self
                .substs_args(args)
                .into_iter()
                .any(|argument| self.contains_reference_inner(argument, visiting)),
            TyKind::Adt { adt, args } => {
                let key = (adt, args);
                if !visiting.insert(key) {
                    return false;
                }
                let def = self.adt(adt);
                let mut contains = false;
                for field in def.all_fields() {
                    let field_ty = self.substitute(field.ty, args);
                    if self.contains_reference_inner(field_ty, visiting) {
                        contains = true;
                        break;
                    }
                }
                visiting.remove(&key);
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
        visiting: &mut FxHashSet<(AdtId, SubstsId)>,
    ) -> bool {
        match self.kind(ty) {
            TyKind::Ref { kind, .. } => kind.is_exclusive(),
            TyKind::Param(_) => true,
            TyKind::Slice { element } | TyKind::Array { element, .. } => {
                self.contains_exclusive_reference_inner(element, visiting)
            }
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .any(|element| self.contains_exclusive_reference_inner(*element, visiting)),
            TyKind::Dyn { args, .. } => self
                .substs_args(args)
                .into_iter()
                .any(|argument| self.contains_exclusive_reference_inner(argument, visiting)),
            TyKind::Adt { adt, args } => {
                let key = (adt, args);
                if !visiting.insert(key) {
                    return false;
                }
                let def = self.adt(adt);
                let mut contains = false;
                for field in def.all_fields() {
                    let field_ty = self.substitute(field.ty, args);
                    if self.contains_exclusive_reference_inner(field_ty, visiting) {
                        contains = true;
                        break;
                    }
                }
                visiting.remove(&key);
                contains
            }
            _ => false,
        }
    }

    /// Returns `true` if `ty` is, or contains by value, a raw pointer.
    ///
    /// Each pointer kind answers `true` for itself. The walk stops at any
    /// pointer, because the pointee is reached only through it; aliasing and
    /// effect summaries use this to find bodies that may reach storage through
    /// the unsafe layer.
    pub fn contains_raw_pointer(&self, ty: TyId) -> bool {
        self.contains_raw_pointer_inner(ty, &mut FxHashSet::default())
    }

    fn contains_raw_pointer_inner(
        &self,
        ty: TyId,
        visiting: &mut FxHashSet<(AdtId, SubstsId)>,
    ) -> bool {
        match self.kind(ty) {
            TyKind::RawPtr { .. } => true,
            // A parameter may be instantiated with a raw pointer, so the walk
            // cannot rule it out before monomorphization.
            TyKind::Param(_) => true,
            TyKind::Slice { element } | TyKind::Array { element, .. } => {
                self.contains_raw_pointer_inner(element, visiting)
            }
            TyKind::Tuple(tuple) => self
                .tuple_elements(tuple)
                .iter()
                .any(|element| self.contains_raw_pointer_inner(*element, visiting)),
            TyKind::Dyn { args, .. } => self
                .substs_args(args)
                .into_iter()
                .any(|argument| self.contains_raw_pointer_inner(argument, visiting)),
            TyKind::Adt { adt, args } => {
                let key = (adt, args);
                if !visiting.insert(key) {
                    return false;
                }
                let def = self.adt(adt);
                let mut contains = false;
                for field in def.all_fields() {
                    let field_ty = self.substitute(field.ty, args);
                    if self.contains_raw_pointer_inner(field_ty, visiting) {
                        contains = true;
                        break;
                    }
                }
                visiting.remove(&key);
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
    fn dyn_is_unsized_not_copy_and_needs_no_drop() {
        let ctxt = TyCtxt::new();
        let object = ctxt.dyn_ty(def(0), ctxt.empty_substs());
        assert!(!ctxt.is_copy(object));
        assert!(!ctxt.is_sized(object));
        assert!(!ctxt.needs_drop(object));
        assert!(!ctxt.contains_reference(object));
        assert!(!ctxt.contains_exclusive_reference(object));
        assert_eq!(
            ctxt.kind(object),
            TyKind::Dyn {
                trait_def: def(0),
                args: ctxt.empty_substs(),
            }
        );

        let int = ctxt.int_ty(IntTy::I32);
        let with_reference = ctxt.dyn_ty(def(1), ctxt.substs(&[ctxt.ref_ty(int)]));
        assert!(ctxt.contains_reference(with_reference));
        assert!(!ctxt.contains_exclusive_reference(with_reference));
        let with_exclusive = ctxt.dyn_ty(def(1), ctxt.substs(&[ctxt.mut_ref_ty(int)]));
        assert!(ctxt.contains_reference(with_exclusive));
        assert!(ctxt.contains_exclusive_reference(with_exclusive));
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
        let ctxt = TyCtxt::new();
        let var = infer.fresh_ty_var(&ctxt);
        assert!(!ctxt.is_copy(var));
        assert!(!ctxt.is_sized(var));
        assert!(!ctxt.needs_drop(var));
    }

    #[test]
    fn trivial_types_never_need_drop() {
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
