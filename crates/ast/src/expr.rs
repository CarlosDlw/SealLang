//! Expressions, operators, and literals.

use seal_span::Span;
use seal_symbol::Symbol;

use crate::generics::GenericArgs;
use crate::id::{BlockId, ExprId, PatternId, TypeId};
use crate::pattern::MatchArm;

/// A binary operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinaryOp {
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
    /// `&&`
    And,
    /// `||`
    Or,
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

impl BinaryOp {
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
            Self::And => "&&",
            Self::Or => "||",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
            Self::Shl => "<<",
            Self::Shr => ">>",
        }
    }
}

/// A prefix operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnaryOp {
    /// `-`
    Neg,
    /// `!`
    Not,
    /// `~`
    BitNot,
}

impl UnaryOp {
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

/// An assignment operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AssignOp {
    /// `=`
    Assign,
    /// `+=`
    AddAssign,
    /// `-=`
    SubAssign,
    /// `*=`
    MulAssign,
    /// `/=`
    DivAssign,
    /// `%=`
    RemAssign,
    /// `<<=`
    ShlAssign,
    /// `>>=`
    ShrAssign,
    /// `&=`
    BitAndAssign,
    /// `|=`
    BitOrAssign,
    /// `^=`
    BitXorAssign,
}

impl AssignOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Assign => "=",
            Self::AddAssign => "+=",
            Self::SubAssign => "-=",
            Self::MulAssign => "*=",
            Self::DivAssign => "/=",
            Self::RemAssign => "%=",
            Self::ShlAssign => "<<=",
            Self::ShrAssign => ">>=",
            Self::BitAndAssign => "&=",
            Self::BitOrAssign => "|=",
            Self::BitXorAssign => "^=",
        }
    }
}

/// An increment or decrement operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UpdateOp {
    /// `++`
    Increment,
    /// `--`
    Decrement,
}

impl UpdateOp {
    /// Returns the source spelling of the operator.
    #[inline]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Increment => "++",
            Self::Decrement => "--",
        }
    }
}

/// A literal value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Literal {
    /// An integer literal, with its radix and optional type suffix.
    Int {
        /// Value of the digits, separators ignored.
        value: u128,
        /// Radix written in the source: 2, 8, 10, or 16.
        base: u32,
        /// Type suffix, if any.
        suffix: Option<Symbol>,
    },
    /// A floating-point literal and its optional type suffix.
    Float {
        /// The correctly rounded value.
        value: f64,
        /// Type suffix, if any.
        suffix: Option<Symbol>,
    },
    /// A string literal; the symbol holds the unescaped contents.
    Str(Symbol),
    /// A character literal.
    Char(char),
    /// A boolean literal.
    Bool(bool),
}

/// One field initializer in a struct literal.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldInit {
    /// The field name.
    pub name: Symbol,
    /// The span of the field name.
    pub name_span: Span,
    /// The value assigned to the field.
    pub value: ExprId,
    /// The span from the field name through the value.
    pub span: Span,
}

/// An expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    /// What the expression is.
    pub kind: ExprKind,
    /// The expression's source range.
    pub span: Span,
}

/// The kinds of expression.
#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// A literal.
    Lit(Literal),
    /// A variable reference, optionally with turbofish type arguments.
    Path {
        /// The referenced name.
        name: Symbol,
        /// The span of the name.
        name_span: Span,
        /// Explicit arguments written as `name::<...>`.
        args: Option<GenericArgs>,
    },
    /// A qualified path, written `Type::name`, used for associated functions.
    PathQualified {
        /// The type or namespace name.
        type_name: Symbol,
        /// The span of the type name.
        type_span: Span,
        /// Explicit type arguments written as `Type::<...>::name`.
        type_args: Option<GenericArgs>,
        /// The associated name.
        name: Symbol,
        /// The span of the associated name.
        name_span: Span,
    },
    /// A borrow of a place, written `&place` or `&mut place`.
    Borrow {
        /// The place being borrowed.
        operand: ExprId,
        /// `true` for `&mut`, which borrows the place exclusively.
        mutable: bool,
    },
    /// A read through a reference, written `*reference`.
    Deref {
        /// The reference being read.
        operand: ExprId,
    },
    /// A field access, written `base.field`.
    Field {
        /// The expression the field is projected from.
        base: ExprId,
        /// The field name.
        name: Symbol,
        /// The span of the field name.
        name_span: Span,
    },
    /// A positional tuple access, written `base.0` or `base.1`.
    ///
    /// Tuple fields are numbered from zero in declaration order. The index is
    /// checked against the tuple's arity by type checking, not by the parser.
    TupleField {
        /// The expression the tuple element is projected from.
        base: ExprId,
        /// The zero-based element position.
        index: u32,
        /// The span of the numeric index, for diagnostics.
        index_span: Span,
    },
    /// A tuple literal, written `(a, b)` or `(a,)`.
    ///
    /// The empty tuple is the unit value `()`.
    Tuple {
        /// Elements in source order.
        elements: Vec<ExprId>,
    },
    /// A struct literal, written `Name { field: value, ... }`.
    Struct {
        /// The struct name.
        name: Symbol,
        /// The span of the struct name.
        name_span: Span,
        /// Explicit type arguments written as `Name::<...>`.
        type_args: Option<GenericArgs>,
        /// Initializers in source order.
        fields: Vec<FieldInit>,
    },
    /// A qualified enum variant literal, written
    /// `Enum::Variant { field: value, ... }`.
    ///
    /// A unit variant has no braces and is written as a qualified path; this
    /// node always has the literal form.
    Variant {
        /// The enum name.
        type_name: Symbol,
        /// The span of the enum name.
        type_span: Span,
        /// Explicit type arguments written as `Enum::<...>::Variant`.
        type_args: Option<GenericArgs>,
        /// The variant name.
        name: Symbol,
        /// The span of the variant name.
        name_span: Span,
        /// Initializers in source order.
        fields: Vec<FieldInit>,
    },
    /// An array literal, written `[a, b, c]`.
    Array {
        /// Elements in source order; an empty literal has no elements.
        elements: Vec<ExprId>,
    },
    /// A repeated array literal, written `[value; length]`.
    ArrayRepeat {
        /// The element expression, evaluated once.
        value: ExprId,
        /// The number of elements.
        length: u64,
        /// The span of the length literal, for diagnostics.
        length_span: Span,
    },
    /// An array index, written `base[index]`.
    Index {
        /// The array place being indexed.
        base: ExprId,
        /// The index expression.
        index: ExprId,
    },
    /// A slice range, written `base[start..end]`. Either bound may be absent.
    ///
    /// A range is a place of unsized slice type; it is useful only as the
    /// operand of a borrow, which becomes a slice reference.
    Slice {
        /// The array or slice place being sliced.
        base: ExprId,
        /// The inclusive start bound, or the beginning when absent.
        start: Option<ExprId>,
        /// The exclusive end bound, or the source length when absent.
        end: Option<ExprId>,
    },
    /// A prefix operation.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: ExprId,
    },
    /// A binary operation.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        lhs: ExprId,
        /// The right operand.
        rhs: ExprId,
    },
    /// An explicit cast, written `value as Type`.
    Cast {
        /// The value being converted.
        operand: ExprId,
        /// The target type.
        ty: TypeId,
        /// The span of the `as` token, for diagnostics.
        as_span: Span,
    },
    /// An assignment, including compound assignment.
    Assign {
        /// The operator.
        op: AssignOp,
        /// The place being assigned.
        target: ExprId,
        /// The new value.
        value: ExprId,
    },
    /// An increment or decrement.
    Update {
        /// The operator.
        op: UpdateOp,
        /// The place being updated.
        operand: ExprId,
        /// `true` for `x++`, `false` for `++x`.
        postfix: bool,
    },
    /// A call.
    Call {
        /// The callee.
        callee: ExprId,
        /// Arguments in source order.
        args: Vec<ExprId>,
    },
    /// A method call, written `receiver.name(args)`.
    MethodCall {
        /// The receiver expression.
        receiver: ExprId,
        /// The method name.
        name: Symbol,
        /// The span of the method name.
        name_span: Span,
        /// Explicit method arguments written as `.name::<...>`.
        generic_args: Option<GenericArgs>,
        /// Arguments in source order, excluding the receiver.
        args: Vec<ExprId>,
    },
    /// A parenthesized expression. Kept so formatting and spans stay faithful.
    Paren(ExprId),
    /// A pattern-matching expression, written `match scrutinee { ... }`.
    Match {
        /// The value being matched.
        scrutinee: ExprId,
        /// The arms in source order; the parser rejects an empty list.
        arms: Vec<MatchArm>,
    },
    /// A block expression, written `{ statements; tail }`.
    ///
    /// The value of the block is its tail expression, or `()` when the block
    /// has statements but no tail. The block owns its own scope.
    Block(BlockId),
    /// An unsafe block, written `unsafe { statements; tail }`.
    ///
    /// The expression has the value of its block. In this feature the marker
    /// only unlocks places rooted in a `static mut`; it does not relax type
    /// checking, borrow checking, or initialization checking, and it has no
    /// runtime representation.
    Unsafe(BlockId),
    /// A conditional expression.
    ///
    /// The value of the expression is the value of the branch that runs. An
    /// `else if` chain is represented as a block whose tail is another `If`, so
    /// every branch is a block and later phases see one shape.
    If {
        /// The condition, which must have type `bool`.
        condition: ExprId,
        /// The block evaluated when the condition is `true`.
        then_branch: BlockId,
        /// The block evaluated when the condition is `false`, if any.
        else_branch: Option<BlockId>,
    },
    /// A conditional expression with a refutable pattern, written
    /// `if let pattern = scrutinee { ... } else { ... }`.
    ///
    /// The scrutinee is evaluated once and matched against the pattern. The
    /// body runs only when the pattern matches, and the bindings exist only in
    /// that body.
    IfLet {
        /// The pattern matched against the scrutinee; it may be refutable.
        pattern: PatternId,
        /// The value tested exactly once.
        scrutinee: ExprId,
        /// The block evaluated when the pattern matches.
        then_branch: BlockId,
        /// The block evaluated when the pattern does not match, if any.
        else_branch: Option<BlockId>,
    },
    /// Syntax the parser could not turn into an expression.
    Error,
}
