//! Types computed by the checker.

use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::{HirExprId, ReceiverKind};
use seal_layout::VtableSlot;
use seal_methods::MethodId;
use seal_ty::{GenericParamDef, Instance, PathElem, RefKind, TraitRef, TyId};

/// One step from a place to a sub-place, with the field resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlaceProjection {
    /// Reads through a shared reference.
    Deref,
    /// Selects a struct field by its definition.
    Field(DefId),
    /// Selects a tuple element by its zero-based position.
    TupleField(u32),
    /// Selects an array or slice element.
    Index {
        /// The index expression, evaluated by MIR lowering.
        index: HirExprId,
        /// The constant value when the index is a literal in range.
        constant: Option<u64>,
        /// The array length, or zero when the length is only known at run time.
        length: u64,
        /// Whether the length comes from a slice reference at run time.
        dynamic_length: bool,
    },
}

/// A place resolved by type checking.
///
/// HIR keeps field names symbolic because resolving them needs the base type.
/// The checker records the resolved form once, and later phases (MIR lowering,
/// move analysis) consume it instead of resolving again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedPlace {
    /// The local or parameter the place starts from.
    pub local: DefId,
    /// The declared type of that local.
    pub root_ty: TyId,
    /// The projections in application order.
    pub projections: Vec<PlaceProjection>,
    /// The type of the selected place.
    pub ty: TyId,
    /// The kind of the last dereference among the projections, or `None` when
    /// no projection reads through a reference.
    ///
    /// Writing a place through a dereference is allowed only for
    /// [`RefKind::Exclusive`]; the last dereference decides, because it is the
    /// reference that reaches the selected storage.
    pub deref: Option<RefKind>,
}

impl ResolvedPlace {
    /// Returns `true` if any projection reads through a reference.
    pub fn contains_deref(&self) -> bool {
        self.projections.contains(&PlaceProjection::Deref)
    }

    /// Returns `true` if the place names the whole local.
    pub fn is_whole(&self) -> bool {
        self.projections.is_empty()
    }

    /// Returns the move path selected by the place.
    ///
    /// Dereferences do not contribute: initialization is only tracked inside one
    /// frame's storage. A dynamic index stops the path at the array it selects,
    /// because its element is not known at compile time; the whole array then
    /// covers the access conservatively.
    pub fn path(&self) -> Vec<PathElem> {
        let mut path = Vec::new();
        for projection in &self.projections {
            match projection {
                PlaceProjection::Deref => break,
                PlaceProjection::Field(field) => path.push(PathElem::Field(*field)),
                PlaceProjection::TupleField(index) => path.push(PathElem::TupleField(*index)),
                PlaceProjection::Index {
                    constant: Some(index),
                    ..
                } => path.push(PathElem::Index(*index)),
                PlaceProjection::Index { constant: None, .. } => break,
            }
        }
        path
    }

    /// Returns `true` when the place indexes an array with a runtime value.
    pub fn has_dynamic_index(&self) -> bool {
        self.projections
            .iter()
            .any(|projection| matches!(projection, PlaceProjection::Index { constant: None, .. }))
    }
}

/// One implicit conversion selected by type checking.
///
/// A coercion is recorded on the expression that produces the converted value;
/// later phases lower it explicitly instead of re-inferring why the source and
/// target types differ.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coercion {
    /// A thin pointer to a concrete value becomes a fat trait-object pointer.
    TraitObject {
        /// The pointer type before the coercion.
        source: TyId,
        /// The trait-object pointer type after the coercion.
        target: TyId,
    },
    /// A trait-object pointer is reborrowed, copying its data and vtable pair.
    TraitObjectReborrow {
        /// The source pointer type.
        source: TyId,
        /// The resulting reference type.
        target: TyId,
    },
}

/// How a method receives its receiver at the MIR level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverForm {
    /// The receiver is storage addressed by a place, with `derefs` explicit
    /// dereferences applied before the method runs.
    Place,
    /// The receiver is a slice fat pointer, represented by its value; the
    /// pointee is unsized, so it has no place of its own.
    Value,
}

/// What defines a resolved method call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MethodKind {
    /// A method from the compiler-owned intrinsic catalog.
    Intrinsic(MethodId),
    /// A user-defined instance method from an inherent impl block.
    User {
        /// The impl function to call.
        function: DefId,
        /// How the method declares its receiver.
        receiver: ReceiverKind,
    },
    /// An instance method declared by a trait the receiver's type implements.
    ///
    /// Dispatch is still static: the call lowers to the concrete function of
    /// the implementation, exactly like an inherent method.
    Trait {
        /// The impl function to call.
        function: DefId,
        /// How the method declares its receiver.
        receiver: ReceiverKind,
        /// The trait that declares the method.
        trait_def: DefId,
    },
    /// A method reached through a bound on a type parameter.
    ///
    /// The implementation is unknown until monomorphization, so no concrete
    /// function exists yet. MIR records the symbolic target instead.
    TraitBound {
        /// The trait that declares the method.
        trait_def: DefId,
        /// The trait method declaration.
        method: DefId,
        /// How the trait method declares its receiver.
        receiver: ReceiverKind,
    },
    /// A method dispatched through a trait-object vtable.
    ///
    /// `object_trait_ref` is the receiver's principal trait, not the trait that
    /// declares the method. It validates the receiver and selects the slot from
    /// that principal vtable; the vtable pointer travels inside the receiver.
    Dynamic {
        /// The receiver's principal trait reference.
        object_trait_ref: TraitRef,
        /// The trait method declaration.
        method: DefId,
        /// The method's slot in the principal vtable.
        slot: VtableSlot,
        /// How the trait method declares its receiver.
        receiver: ReceiverKind,
    },
}

/// How MIR must adjust the receiver expression before a call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverAdjust {
    /// Pass the receiver as written.
    None,
    /// Borrow the receiver as `*T` for a shared user method.
    SharedBorrow,
    /// Borrow the receiver as `*mut T` for an exclusive user method.
    ExclusiveBorrow,
    /// Reborrow a `*mut T` receiver as `*T` for a method that only reads.
    ///
    /// The reference is dereferenced first, so the loan covers the pointee the
    /// original reference already reaches instead of the pointer variable.
    SharedReborrow,
}

/// A method call resolved by type checking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedMethod {
    /// Whether the call reaches an intrinsic or an impl function.
    pub kind: MethodKind,
    /// How the receiver expression must be adjusted before the call.
    pub receiver_adjust: ReceiverAdjust,
    /// The receiver type after any reference dereferences, capped at the owned
    /// family: scalar, array, the slice reference itself, or the user struct.
    pub receiver_ty: TyId,
    /// How the receiver reaches MIR.
    pub receiver_form: ReceiverForm,
    /// Number of dereferences between the source receiver and `receiver_ty`.
    pub receiver_derefs: usize,
    /// Whether an array receiver is viewed as a slice for this call.
    pub array_to_slice: bool,
    /// Argument types after the receiver.
    pub arg_tys: Vec<TyId>,
    /// The result type, or the unit type.
    pub ret_ty: TyId,
}

/// A function signature as seen by callers.
///
/// Parameters whose declared type is unsized are normalized to the error type,
/// because such a declaration already produced `E3010`. The return type is the
/// unit type when the declaration omits it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    /// Parameter types in source order.
    pub params: Vec<TyId>,
    /// The return type.
    pub ret: TyId,
    /// Every generic parameter visible in the signature, outermost first.
    pub generics: Vec<GenericParamDef>,
}

/// Type information for one file.
///
/// Maps are keyed by HIR IDs, so no lookup re-resolves a name or re-reads the
/// source. The callee of a valid call has no entry: Seal has no first-class
/// function type, and the callee is resolved directly to its function
/// definition.
#[derive(Clone, Debug)]
pub struct TypeckResults {
    expr_ty: FxHashMap<HirExprId, TyId>,
    local_ty: FxHashMap<DefId, TyId>,
    signatures: FxHashMap<DefId, Signature>,
    places: FxHashMap<HirExprId, ResolvedPlace>,
    methods: FxHashMap<HirExprId, ResolvedMethod>,
    instantiations: FxHashMap<HirExprId, Instance>,
    coercions: FxHashMap<HirExprId, Coercion>,
}

impl TypeckResults {
    /// Returns the type of `expr`, if it is a value expression.
    pub fn expr_ty(&self, expr: HirExprId) -> Option<TyId> {
        self.expr_ty.get(&expr).copied()
    }

    /// Returns the type of a parameter or binding.
    pub fn local_ty(&self, local: DefId) -> Option<TyId> {
        self.local_ty.get(&local).copied()
    }

    /// Returns the signature of a function.
    pub fn signature(&self, function: DefId) -> Option<&Signature> {
        self.signatures.get(&function)
    }

    /// Returns the resolved place denoted by `expr`, if it denotes storage.
    pub fn place(&self, expr: HirExprId) -> Option<&ResolvedPlace> {
        self.places.get(&expr)
    }

    /// Returns the method resolved for a method-call expression.
    pub fn method(&self, expr: HirExprId) -> Option<&ResolvedMethod> {
        self.methods.get(&expr)
    }

    /// Returns the complete instantiation of a call, literal, or method call.
    pub fn instantiation(&self, expr: HirExprId) -> Option<Instance> {
        self.instantiations.get(&expr).copied()
    }

    /// Returns the implicit conversion applied to `expr`, if any.
    pub fn coercion(&self, expr: HirExprId) -> Option<Coercion> {
        self.coercions.get(&expr).copied()
    }

    /// Returns the number of expressions that received a type.
    pub fn typed_expr_count(&self) -> usize {
        self.expr_ty.len()
    }

    pub(crate) fn new() -> Self {
        Self {
            expr_ty: FxHashMap::default(),
            local_ty: FxHashMap::default(),
            signatures: FxHashMap::default(),
            places: FxHashMap::default(),
            methods: FxHashMap::default(),
            instantiations: FxHashMap::default(),
            coercions: FxHashMap::default(),
        }
    }

    pub(crate) fn record_expr(&mut self, expr: HirExprId, ty: TyId) {
        self.expr_ty.insert(expr, ty);
    }

    pub(crate) fn record_local(&mut self, local: DefId, ty: TyId) {
        self.local_ty.insert(local, ty);
    }

    pub(crate) fn record_place(&mut self, expr: HirExprId, place: ResolvedPlace) {
        self.places.insert(expr, place);
    }

    pub(crate) fn insert_signature(&mut self, function: DefId, signature: Signature) {
        self.signatures.insert(function, signature);
    }

    pub(crate) fn record_method(&mut self, expr: HirExprId, method: ResolvedMethod) {
        self.methods.insert(expr, method);
    }

    pub(crate) fn record_instantiation(&mut self, expr: HirExprId, instance: Instance) {
        self.instantiations.insert(expr, instance);
    }

    pub(crate) fn record_coercion(&mut self, expr: HirExprId, coercion: Coercion) {
        self.coercions.insert(expr, coercion);
    }
}
