//! Values computed by statements.

use seal_def::DefId;
use seal_layout::VtableSlot;
use seal_methods::MethodId;
use seal_span::Span;
use seal_ty::{CastKind, ClosureKind, Instance, SubstsId, TraitRef, TyId, VariantIdx};

use crate::operand::Operand;
use crate::place::Place;

/// A binary operator in MIR.
///
/// Short-circuiting `&&` and `||` are not binary operations: they lower to
/// `SwitchInt`, so the right operand is evaluated only when required.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp {
    /// `+`
    Add,
    /// `-`
    Sub,
    /// `*`
    Mul,
    /// `/`
    Div,
    /// `%`
    Rem,
    /// `==`
    Eq,
    /// `!=`
    Ne,
    /// `<`
    Lt,
    /// `>`
    Gt,
    /// `<=`
    Le,
    /// `>=`
    Ge,
    /// `&`
    BitAnd,
    /// `|`
    BitOr,
    /// `^`
    BitXor,
    /// `<<`
    Shl,
    /// `>>`
    Shr,
}

impl BinOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Rem => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
        }
    }
}

/// A unary operator in MIR.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnOp {
    /// Arithmetic negation.
    Neg,
    /// Logical negation.
    Not,
    /// Bitwise complement.
    BitNot,
}

impl UnOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "!",
            Self::BitNot => "~",
        }
    }
}

/// How an intrinsic method receives its subject.
#[derive(Clone, Debug, PartialEq)]
pub enum IntrinsicReceiver {
    /// A place holding a scalar, array, or aggregate. The method definition
    /// decides whether the call reads it or writes it.
    Place(Place),
    /// A slice fat pointer. The pointee is unsized, so the value itself is the
    /// receiver and the data pointer and length are extracted at codegen.
    Value(Operand),
}

/// What one call reaches.
///
/// A call is direct when type checking selected one function or one concrete
/// impl method. A call through a type parameter bound has no implementation yet:
/// it keeps the trait method declaration and its arguments as a symbolic target
/// that monomorphization resolves for each instantiation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CallTarget {
    /// A known function or method.
    Direct {
        /// The function and its complete type arguments.
        instance: Instance,
        /// The parameter count the declaration expects, receiver included for
        /// methods.
        parameters: usize,
    },
    /// A trait method reached through a bound on a type parameter.
    TraitMethod {
        /// The trait method declaration and its complete type arguments, with
        /// the implementing type as the first argument.
        instance: Instance,
        /// The parameter count the declaration expects, receiver included for
        /// methods.
        parameters: usize,
    },
    /// A call of a concrete closure value.
    ///
    /// The first argument is the environment the closure body expects, followed
    /// by the source arguments. An `Fn` call passes `*Env`, an `FnMut` call
    /// passes `*mut Env`, and an `FnOnce` call passes `Env` by value.
    Closure {
        /// The closure and its complete type arguments.
        instance: Instance,
    },
    /// A call through a parameter declared with a function-trait bound.
    ///
    /// The call stays symbolic until monomorphization substitutes the concrete
    /// self type. The first argument is the receiver in the form the bound's
    /// kind requires: a shared or exclusive borrow, or the value for `FnOnce`.
    FnTrait {
        /// The bound that supplies the callable signature.
        trait_ref: TraitRef,
        /// The strongest bound selected for the call.
        kind: ClosureKind,
        /// The bound's parameter types, in declaration order.
        parameters: SubstsId,
    },
    /// A function implemented outside Seal.
    ///
    /// The declaration has no body and no type arguments; the linker symbol is
    /// fixed by its HIR declaration.
    Extern {
        /// The external function declaration.
        def: DefId,
        /// The parameter count the declaration expects.
        parameters: usize,
        /// Whether the declaration is variadic.
        variadic: bool,
    },
    /// A call through a Seal function pointer.
    ///
    /// The pointer value is the first element of the call's `args`, followed by
    /// the real arguments in source order. That keeps the callee in evaluation
    /// order without making this target unbounded: the target stays `Copy` and
    /// names only the pointer type and how its receiver is reached. A call
    /// written through a function pointer uses `receiver: None`, where the
    /// first argument is the pointer itself; monomorphization also produces
    /// this form when a function-trait bound is instantiated with a pointer.
    FnPtr {
        /// The pointer type the call was checked against, always
        /// [`TyKind::FnPtr`](seal_ty::TyKind::FnPtr).
        ty: TyId,
        /// How the pointer is reached from its receiver, when it is not the
        /// argument itself.
        receiver: Option<seal_ty::RefKind>,
    },
}

impl CallTarget {
    /// Returns the callee instance, or `None` for an external or indirect call.
    #[inline]
    pub fn try_instance(self) -> Option<Instance> {
        match self {
            Self::Direct { instance, .. }
            | Self::TraitMethod { instance, .. }
            | Self::Closure { instance } => Some(instance),
            Self::Extern { .. } | Self::FnPtr { .. } | Self::FnTrait { .. } => None,
        }
    }

    /// Returns the callee instance.
    ///
    /// # Panics
    ///
    /// Panics for an external or function pointer call. Callers that may see
    /// one use [`CallTarget::try_instance`].
    #[inline]
    pub fn instance(self) -> Instance {
        self.try_instance().expect(
            "an external, function pointer, or symbolic function-trait call has no Seal instance",
        )
    }

    /// Returns the expected parameter count.
    ///
    /// # Panics
    ///
    /// Panics for a function pointer call: its signature lives in the target's
    /// `ty`, and readers must query the type context for the parameter list.
    #[inline]
    pub fn parameters(self) -> usize {
        match self {
            Self::Direct { parameters, .. }
            | Self::TraitMethod { parameters, .. }
            | Self::Extern { parameters, .. } => parameters,
            Self::FnPtr { .. } | Self::Closure { .. } | Self::FnTrait { .. } => {
                panic!("this call target takes its parameters from its type or signature")
            }
        }
    }

    /// Returns `true` when monomorphization must choose the implementation.
    #[inline]
    pub fn is_trait_method(self) -> bool {
        matches!(self, Self::TraitMethod { .. } | Self::FnTrait { .. })
    }

    /// Returns `true` for a call of a concrete closure value.
    #[inline]
    pub fn is_closure(self) -> bool {
        matches!(self, Self::Closure { .. })
    }

    /// Returns `true` for a call that still needs a function-trait bound
    /// resolved by monomorphization.
    #[inline]
    pub fn is_fn_trait(self) -> bool {
        matches!(self, Self::FnTrait { .. })
    }

    /// Returns `true` for a call through a Seal function pointer.
    #[inline]
    pub fn is_fn_ptr(self) -> bool {
        matches!(self, Self::FnPtr { .. })
    }

    /// Returns `true` for a call implemented outside Seal.
    #[inline]
    pub fn is_extern(self) -> bool {
        matches!(self, Self::Extern { .. })
    }
}

/// A compile-time type property read at run time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeInfoKind {
    /// The displayed source name of a type, as `*str`.
    Name,
    /// The byte size of a type, as `usize`.
    Size,
    /// The byte alignment of a type, as `usize`.
    Align,
}

/// The aggregate being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AggregateKind {
    /// A struct, identified by its definition and type arguments.
    Struct {
        /// The struct definition.
        adt: DefId,
        /// The complete type arguments of the instantiation.
        args: SubstsId,
    },
    /// One variant of an enum, identified by the enum definition, the variant's
    /// position in declaration order, and the instantiation's type arguments.
    Variant {
        /// The enum definition.
        adt: DefId,
        /// The variant's discriminant.
        variant: VariantIdx,
        /// The complete type arguments of the instantiation.
        args: SubstsId,
    },
}

/// An expression that produces a value and can be assigned to a place.
#[derive(Clone, Debug, PartialEq)]
pub enum Rvalue {
    /// Copies or moves an operand.
    Use(Operand),
    /// Converts an operand with an explicit `as` cast.
    ///
    /// The conversion is infallible: it never panics, allocates, or invokes
    /// undefined behavior. `from` and `to` are concrete types and `kind` is the
    /// classification recorded by the type checker.
    Cast {
        /// The conversion family.
        kind: CastKind,
        /// The value being converted, read exactly once.
        operand: Operand,
        /// The operand's type.
        from: TyId,
        /// The destination type.
        to: TyId,
    },
    /// Converts a thin pointer to a trait-object pointer.
    ///
    /// `source` holds a pointer whose pointee is the concrete `source_ty`
    /// pointee. Code generation replaces the thin pointer with a fat pointer
    /// carrying the same data address and the vtable selected by `target_ty`.
    MakeTraitObject {
        /// The thin pointer value.
        source: Operand,
        /// The source pointer type, which selects the concrete vtable key.
        source_ty: TyId,
        /// The resulting trait-object pointer type.
        target_ty: TyId,
    },
    /// Reborrows a trait-object place, copying its data and vtable pair.
    ///
    /// `source` is the fat pointer used by code generation. `place` is the
    /// pointee place used by borrow checking, so the loan is registered on the
    /// pointed-to storage and never on the pointer variable itself.
    TraitObjectReborrow {
        /// The source fat pointer.
        source: Operand,
        /// The pointee place the new loan covers.
        place: Place,
        /// The source pointer type.
        from: TyId,
        /// The resulting pointer type.
        to: TyId,
    },
    /// Borrows a place, written `&place` or `&mut place`.
    ///
    /// A shared reference (`*T`) keeps the borrowed place alive and permits
    /// other shared reads of it; an exclusive one (`*mut T`) permits reads and
    /// writes but conflicts with every other loan. Borrow checking compares
    /// loans by kind.
    Ref {
        /// The borrowed place.
        place: Place,
        /// Whether the borrow is exclusive.
        kind: seal_ty::RefKind,
    },
    /// Builds a slice reference from an array or slice place.
    ///
    /// `start` and `end` are `usize` values; an absent bound means the start of
    /// the source or its full length. Lowering checks `start <= end <= length`
    /// before this value is constructed.
    SliceRef {
        /// The array or slice the range selects from.
        base: Place,
        /// The inclusive start bound.
        start: Option<Operand>,
        /// The exclusive end bound.
        end: Option<Operand>,
        /// Whether the reference permits writes.
        kind: seal_ty::RefKind,
        /// The resulting `*[T]` or `*mut [T]` type.
        ty: seal_ty::TyId,
    },
    /// Reads the discriminant of an enum place.
    ///
    /// The result is a `u32`, wide enough for every supported discriminant, so
    /// a `SwitchInt` can compare it directly. Reading the discriminant never
    /// moves the enum.
    Discriminant(Place),
    /// Reads the length of a slice reference.
    Len {
        /// The fat pointer whose length is read.
        slice: Operand,
    },
    /// Produces a compile-time property of `ty`.
    ///
    /// Monomorphization substitutes `ty`, so a query written with a type
    /// parameter reports the concrete type at each call site. `arg` is present
    /// for `type_of(value)` and supplies the evaluated operand the query reads;
    /// a plain `type_name::<T>()` has no argument.
    TypeInfo {
        /// Which property is produced.
        query: TypeInfoKind,
        /// The type being inspected.
        ty: TyId,
        /// The evaluated argument for `type_of`, if any.
        arg: Option<Operand>,
        /// The type of the produced value (`*str` or `usize`).
        result_ty: TyId,
    },
    /// Applies a binary operator to two operands.
    Binary {
        /// The operator.
        op: BinOp,
        /// The left operand.
        lhs: Operand,
        /// The right operand.
        rhs: Operand,
        /// The source range of the whole binary expression.
        span: Span,
    },
    /// Applies a unary operator to an operand.
    Unary {
        /// The operator.
        op: UnOp,
        /// The operand.
        operand: Operand,
    },
    /// Calls a compiler-owned intrinsic method.
    ///
    /// The receiver is part of the call, not a preceding borrow: a shared
    /// method reads it and an exclusive method writes it for the duration of
    /// the statement. No reference escapes, so the borrow checker checks the
    /// access directly instead of creating a loan.
    Intrinsic {
        /// The resolved catalog entry.
        method: MethodId,
        /// The receiver, by place or as a slice fat pointer.
        receiver: IntrinsicReceiver,
        /// Arguments in evaluation order, excluding the receiver.
        args: Vec<Operand>,
        /// The source range of the whole method call.
        span: Span,
    },
    /// Calls a method through the vtable embedded in a trait object.
    ///
    /// The receiver is separate from `args`: code generation extracts its data
    /// pointer, while borrow checking analyzes it as the first call argument.
    /// The call consumes the vtable already inside the receiver; it never looks
    /// up a vtable by key.
    DynamicCall {
        /// The receiver's principal trait reference.
        object_trait_ref: TraitRef,
        /// The trait method declaration.
        method: DefId,
        /// The method's slot in the principal vtable.
        slot: VtableSlot,
        /// The trait-object receiver value.
        receiver: Operand,
        /// Arguments in evaluation order, excluding the receiver.
        args: Vec<Operand>,
        /// The source span of the receiver followed by each argument.
        arg_spans: Vec<Span>,
        /// The argument types after replacing the trait's `Self` and arguments.
        arg_tys: Vec<TyId>,
        /// The result type after the same replacement.
        ret_ty: TyId,
    },
    /// Calls through a raw C function pointer.
    ///
    /// The callee is evaluated before the arguments, exactly like the source
    /// expression. Its type is `*raw extern "c" func(...)`, which supplies the
    /// fixed arity, variadicity, and result type. The call is always inside
    /// `unsafe`; type checking enforces that before lowering.
    IndirectCall {
        /// The function pointer value, evaluated first.
        callee: Operand,
        /// Arguments in evaluation order.
        args: Vec<Operand>,
        /// The source range of each argument, matching `args` by position.
        arg_spans: Vec<Span>,
    },
    /// Calls a function or a trait method.
    ///
    /// Calls are rvalues because Seal has no unwinding and a call cannot fail in
    /// a way that changes control flow. A future ABI with unwind edges will turn
    /// this into a terminator. A symbolic trait call keeps its declaration as
    /// the target until monomorphization picks the implementing function.
    Call {
        /// The callee, with its complete type arguments.
        target: CallTarget,
        /// Arguments in evaluation order.
        args: Vec<Operand>,
        /// The source range of each argument, matching `args` by position.
        ///
        /// Call diagnostics point at the argument that violates the contract
        /// instead of the whole call, so lowering records one span per operand
        /// and validation keeps the two lists in step.
        arg_spans: Vec<Span>,
    },
    /// Builds an aggregate value from its fields.
    ///
    /// Field operands appear in source order, which is evaluation order. A
    /// field that is not `Copy` is moved out of its place, so building an
    /// aggregate can end the lifetime of the places it reads.
    Aggregate {
        /// What is being built.
        kind: AggregateKind,
        /// Field initializers in evaluation order.
        fields: Vec<(DefId, Operand)>,
    },
    /// Builds a tuple from its elements.
    ///
    /// Elements are evaluated in source order. A non-`Copy` element is moved
    /// into the new tuple, exactly as in a struct literal.
    Tuple {
        /// Elements in source order.
        elements: Vec<Operand>,
        /// The tuple type, which carries the arity and element types.
        ty: seal_ty::TyId,
    },
    /// Builds a fixed-size array from its elements.
    Array {
        /// Elements in source order.
        elements: Vec<Operand>,
        /// The array type, which carries the element type and length.
        ty: seal_ty::TyId,
    },
    /// Builds an array by copying one value into every element.
    Repeat {
        /// The value copied into each element.
        value: Operand,
        /// The number of elements.
        count: u64,
        /// The array type, which carries the element type and length.
        ty: seal_ty::TyId,
    },
}
