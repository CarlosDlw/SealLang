//! Lowered expressions and places.

use seal_ast::{AssignOp, BinaryOp, Literal, UnaryOp, UpdateOp};
use seal_def::DefId;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{SubstsId, TyId};

use crate::id::{HirBlockId, HirExprId, HirPatternId};
use crate::pattern::MatchArm;
use crate::res::Res;

/// A storage location: a local definition plus its projections.
///
/// References add dereference projections, so `*p` is the place rooted at the
/// local that holds the reference. Field names stay symbolic here: resolving
/// them needs the base type, which type checking computes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    local: DefId,
    projections: Vec<Projection>,
}

/// One step from a place to a sub-place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Projection {
    /// Reads through a reference, written `*place`.
    Deref,
    /// Selects a named field of a struct place.
    Field(Symbol),
    /// Selects a tuple element by its zero-based position.
    TupleField(usize),
    /// Selects an array element by an index expression.
    Index(crate::id::HirExprId),
}

impl Place {
    /// Creates the place for a whole definition-rooted place.
    ///
    /// The root may be a binding, a parameter, a constant, or a static; later
    /// phases distinguish those storage classes.
    #[inline]
    pub fn local(local: DefId) -> Self {
        Self {
            local,
            projections: Vec::new(),
        }
    }

    /// Returns the root definition this place starts from.
    #[inline]
    pub const fn local_def(&self) -> DefId {
        self.local
    }

    /// Returns the place reached by one dereference.
    #[inline]
    pub fn deref(mut self) -> Self {
        self.projections.push(Projection::Deref);
        self
    }

    /// Returns the place reached by selecting `name`.
    #[inline]
    pub fn field(mut self, name: Symbol) -> Self {
        self.projections.push(Projection::Field(name));
        self
    }

    /// Returns the place reached by selecting tuple element `index`.
    #[inline]
    pub fn tuple_field(mut self, index: usize) -> Self {
        self.projections.push(Projection::TupleField(index));
        self
    }

    /// Returns the place reached by indexing with `index`.
    #[inline]
    pub fn index(mut self, index: crate::id::HirExprId) -> Self {
        self.projections.push(Projection::Index(index));
        self
    }

    /// Returns the projections applied to the local, outermost last.
    #[inline]
    pub fn projections(&self) -> &[Projection] {
        &self.projections
    }

    /// Returns `true` if any projection reads through a reference.
    #[inline]
    pub fn contains_deref(&self) -> bool {
        self.projections.contains(&Projection::Deref)
    }

    /// Returns `true` if the place names the whole local.
    #[inline]
    pub fn is_whole(&self) -> bool {
        self.projections.is_empty()
    }
}

/// A lowered expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What the expression is.
    pub kind: ExprKind,
    /// The expression's source range.
    pub span: Span,
}

/// The kinds of lowered expression.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// A literal.
    Lit(Literal),
    /// A resolved name used as a value, with optional type arguments.
    Path {
        /// The resolved function or local.
        res: Res,
        /// Explicit type arguments for a free function, or for the aggregate a
        /// qualified associated function is reached through, or `None`.
        args: Option<SubstsId>,
        /// Explicit type arguments written on the associated function name,
        /// after the aggregate arguments, or `None`.
        function_args: Option<SubstsId>,
    },
    /// A borrow of a place, written `&place` or `&mut place`.
    Borrow {
        /// The place being borrowed.
        operand: HirExprId,
        /// Whether the borrow is exclusive (`*mut T`).
        kind: seal_ty::RefKind,
    },
    /// A read through a reference, written `*reference`.
    Deref {
        /// The reference being read.
        operand: HirExprId,
    },
    /// A field access on a place, written `base.name`.
    Field {
        /// The place the field is projected from.
        base: HirExprId,
        /// The field name, resolved against the base type by type checking.
        name: Symbol,
        /// The span of the field name.
        name_span: Span,
    },
    /// A positional tuple access on a place, written `base.0`.
    TupleField {
        /// The place the element is projected from.
        base: HirExprId,
        /// The zero-based element position.
        index: u32,
        /// The span of the numeric index.
        index_span: Span,
    },
    /// A tuple literal, including the unit value `()`.
    Tuple {
        /// Elements in source order.
        elements: Vec<HirExprId>,
    },
    /// A struct literal with resolved fields.
    Struct {
        /// The struct definition named by the literal.
        adt: DefId,
        /// Explicit type arguments, or `None` for inference.
        args: Option<SubstsId>,
        /// Initializers in source order, one per valid source field.
        fields: Vec<(DefId, HirExprId)>,
    },
    /// A constructed enum value: a variant and its resolved fields.
    ///
    /// A unit variant has no fields; the node still names the variant, so later
    /// phases build the value by writing the discriminant.
    Variant {
        /// The enum definition the variant belongs to.
        adt: DefId,
        /// The variant's position in declaration order.
        variant: seal_ty::VariantIdx,
        /// Explicit type arguments, or `None` for inference.
        args: Option<SubstsId>,
        /// Initializers in source order, one per valid variant field.
        fields: Vec<(DefId, HirExprId)>,
    },
    /// An array literal.
    Array {
        /// Elements in source order; an empty literal has no elements.
        elements: Vec<HirExprId>,
    },
    /// A repeated array literal, written `[value; length]`.
    ArrayRepeat {
        /// The value copied into each element.
        value: HirExprId,
        /// The number of elements.
        length: u64,
    },
    /// An array index place, written `base[index]`.
    Index {
        /// The array place being indexed.
        base: HirExprId,
        /// The index expression.
        index: HirExprId,
    },
    /// A slice range place, written `base[start..end]`.
    ///
    /// The range has unsized type `[T]` and is only useful as the operand of a
    /// borrow, which lowers to [`ExprKind::SliceRef`].
    Slice {
        /// The array or slice place being sliced.
        base: HirExprId,
        /// The inclusive start bound, or the beginning when absent.
        start: Option<HirExprId>,
        /// The exclusive end bound, or the source length when absent.
        end: Option<HirExprId>,
    },
    /// A borrow of a slice range, lowered directly to a fat reference.
    SliceRef {
        /// The array or slice place being sliced.
        base: HirExprId,
        /// The inclusive start bound, or the beginning when absent.
        start: Option<HirExprId>,
        /// The exclusive end bound, or the source length when absent.
        end: Option<HirExprId>,
        /// Whether the reference permits writes.
        kind: seal_ty::RefKind,
    },
    /// The length of a slice reference, written `len(slice)`.
    Len {
        /// The `*[T]` or `*mut [T]` value whose length is read.
        operand: HirExprId,
    },
    /// The built-in `assert(condition)` and `assert(condition, "message")`.
    ///
    /// The intrinsic is only available in test and benchmark configurations.
    /// The message is a compile-time string, so a failure never has to format
    /// values or allocate.
    Assert {
        /// The boolean condition that must hold.
        condition: HirExprId,
        /// The message reported on failure, when the source supplied one.
        message: Option<String>,
    },
    /// The built-in `assert_eq(left, right)` and its optional message.
    ///
    /// Like `assert`, the equality assertions are only available in test and
    /// benchmark configurations. Both operands are read, never moved, so a
    /// failure and a success leave their owners responsible for dropping them.
    AssertEq {
        /// The left operand.
        lhs: HirExprId,
        /// The right operand.
        rhs: HirExprId,
        /// `true` for `assert_eq`, `false` for `assert_ne`.
        equal: bool,
        /// The message reported on failure, when the source supplied one.
        message: Option<String>,
    },
    /// The built-in `panic(message)`, which terminates the process.
    Panic {
        /// The `*str` message reported to standard error.
        message: HirExprId,
    },
    /// The built-in `exit(code)`, which terminates the process with `code`.
    Exit {
        /// The `i32` process status.
        code: HirExprId,
    },
    /// The built-in `type_name::<T>()`.
    TypeName {
        /// The type whose displayed name is produced.
        ty: TyId,
    },
    /// The built-in `type_of(value)`, which reports the operand's static type.
    TypeOf {
        /// The operand whose static type is produced.
        operand: HirExprId,
    },
    /// The built-in `size_of::<T>()`.
    SizeOf {
        /// The type whose byte size is produced.
        ty: TyId,
    },
    /// The built-in `align_of::<T>()`.
    AlignOf {
        /// The type whose byte alignment is produced.
        ty: TyId,
    },
    /// A closure expression.
    ///
    /// The closure's body, parameters, and environment live behind its
    /// definition; this node is the value that stores the environment.
    Closure {
        /// The closure's definition.
        closure: DefId,
        /// Captures in first-mention order, copied from the definition so
        /// consumers can read the environment without a second lookup.
        captures: Vec<crate::closure::CaptureDef>,
    },
    /// A use of a value captured by the enclosing closure.
    Capture {
        /// The closure whose environment stores the value.
        closure: DefId,
        /// The environment slot.
        index: u32,
    },
    /// The built-in `drop(value)`, which releases a value before its scope
    /// ends.
    ///
    /// The operand is consumed exactly like any other move: a non-`Copy` place
    /// is moved and its destructor runs here, while a `Copy` value is read and
    /// nothing else happens.
    Drop {
        /// The value whose destructor runs now.
        operand: HirExprId,
    },
    /// A prefix operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: HirExprId,
    },
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        lhs: HirExprId,
        /// The right operand.
        rhs: HirExprId,
    },
    /// An explicit cast, written `value as Type`.
    Cast {
        /// The value being converted.
        operand: HirExprId,
        /// The resolved target type.
        ty: TyId,
        /// The span of the `as` token, for diagnostics.
        as_span: Span,
        /// The span of the target type, for diagnostics.
        target_span: Span,
    },
    /// An assignment, including compound assignment.
    ///
    /// `target` is kept as an expression because it may fail to resolve; use
    /// [`Hir::place`](crate::Hir::place) to ask for the place it denotes.
    Assign {
        /// The operator.
        op: AssignOp,
        /// The place being assigned.
        target: HirExprId,
        /// The new value.
        value: HirExprId,
    },
    /// An increment or decrement.
    Update {
        /// The operator.
        op: UpdateOp,
        /// The place being updated.
        target: HirExprId,
        /// `true` for `x++`, `false` for `++x`.
        postfix: bool,
    },
    /// A call.
    Call {
        /// The callee.
        callee: HirExprId,
        /// Arguments in source order.
        args: Vec<HirExprId>,
    },
    /// A method call on an intrinsic receiver.
    MethodCall {
        /// The receiver expression; it need not be a place, so a temporary can
        /// be borrowed for the duration of the call.
        receiver: HirExprId,
        /// The method name, resolved against the receiver type by type checking.
        name: Symbol,
        /// The span of the method name.
        name_span: Span,
        /// Explicit method type arguments, or `None`.
        generic_args: Option<SubstsId>,
        /// Arguments in source order, excluding the receiver.
        args: Vec<HirExprId>,
    },
    /// A pattern-matching expression.
    Match {
        /// The value being matched, evaluated exactly once.
        scrutinee: HirExprId,
        /// Arms in source order; the parser guarantees at least one.
        arms: Vec<MatchArm>,
    },
    /// A block expression, written `{ statements; tail }`.
    Block(HirBlockId),
    /// An `unsafe { ... }` expression.
    ///
    /// The block has its ordinary value; the wrapper only lifts the unsafe
    /// access rules for what it contains.
    Unsafe {
        /// The wrapped block.
        block: HirBlockId,
    },
    /// A conditional expression.
    If {
        /// The condition, which must have type `bool`.
        condition: HirExprId,
        /// The block evaluated when the condition is `true`.
        then_branch: HirBlockId,
        /// The block evaluated when the condition is `false`, if any.
        else_branch: Option<HirBlockId>,
    },
    /// A conditional expression that runs its body only when a pattern matches.
    IfLet {
        /// The lower pattern matched against the scrutinee.
        pattern: HirPatternId,
        /// The value tested exactly once.
        scrutinee: HirExprId,
        /// The block evaluated when the pattern matches.
        then_branch: HirBlockId,
        /// The block evaluated when the pattern does not match, if any.
        else_branch: Option<HirBlockId>,
    },
    /// Syntax that could not be lowered, after a diagnostic was reported.
    Error,
}
