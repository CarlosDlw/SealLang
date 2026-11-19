//! Function bodies in MIR.

use seal_def::DefId;
use seal_index::IndexVec;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::TyId;

use crate::id::{BasicBlockId, LocalId};
use crate::statement::Statement;
use crate::terminator::Terminator;
use crate::validate::ValidationError;

/// What an executable body belongs to.
///
/// The kind is copied from HIR so later phases can treat a constant or static
/// initializer differently from a function body without re-reading the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyKind {
    /// A function body. It may have parameters.
    Function,
    /// A constant initializer. It has no parameters.
    Const,
    /// A static initializer. It has no parameters.
    Static {
        /// Whether the static is declared `mut`.
        mutable: bool,
    },
}

/// The role a local plays in its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocalKind {
    /// The return place. It is always local zero.
    Return,
    /// A function parameter, in source order starting at local one.
    Arg,
    /// A `let` parameter or binding.
    Binding,
    /// A compiler-generated temporary.
    Temp,
    /// A drop flag: a `bool` recording whether a place still holds a value.
    ///
    /// Drop elaboration guards a conditional drop site with one flag per place,
    /// so the generated code drops a value exactly when the program's control
    /// flow left one there. A flag is not source-level storage, so debug
    /// information never declares it.
    DropFlag,
}

/// A local slot in a body.
#[derive(Clone, Debug, PartialEq)]
pub struct LocalDecl {
    /// The type of values stored in the slot.
    pub ty: TyId,
    /// The source name, for parameters and bindings.
    pub name: Option<Symbol>,
    /// What the local represents.
    pub kind: LocalKind,
    /// The source range that introduced the local.
    pub span: Span,
}

/// One node of the control-flow graph.
#[derive(Clone, Debug, PartialEq)]
pub struct BasicBlock {
    /// Statements run in order.
    pub statements: Vec<Statement>,
    /// The edge that leaves the block.
    pub terminator: Terminator,
}

/// The MIR of one function.
///
/// Local zero is always the return place. Locals `1..=arg_count` are the
/// parameters in source order. Bindings and temporaries follow. A body may
/// contain blocks that no edge reaches, for example after a `return` followed by
/// more source statements; later passes are free to remove them, and validation
/// deliberately does not require reachability.
#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    /// The function, constant, or static this body implements.
    pub owner: DefId,
    /// What kind of initializer or function the owner declares.
    pub kind: BodyKind,
    /// Whether the owner is a `const func`.
    ///
    /// Only a function body may carry this flag; a constant or static
    /// initializer is a value body and is always interpreted, never called.
    pub is_const: bool,
    /// The number of parameters, excluding the return place.
    pub arg_count: usize,
    /// All locals, indexable by [`LocalId`].
    pub locals: IndexVec<LocalId, LocalDecl>,
    /// All basic blocks, indexable by [`BasicBlockId`].
    pub blocks: IndexVec<BasicBlockId, BasicBlock>,
    /// The block where execution starts.
    pub entry: BasicBlockId,
}

impl Body {
    /// Returns the return place's local, which is always local zero.
    #[inline]
    pub fn return_local() -> LocalId {
        LocalId::from_u32(0).expect("the return local is index zero")
    }

    /// Returns a local declaration.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this body.
    #[inline]
    pub fn local(&self, id: LocalId) -> &LocalDecl {
        &self.locals[id]
    }

    /// Returns a basic block.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this body.
    #[inline]
    pub fn block(&self, id: BasicBlockId) -> &BasicBlock {
        &self.blocks[id]
    }

    /// Iterates over the parameters, excluding the return place.
    pub fn args(&self) -> impl Iterator<Item = &LocalDecl> {
        self.locals.iter().skip(1).take(self.arg_count)
    }

    /// Checks the structural invariants of this body.
    pub fn validate(&self) -> Result<(), ValidationError> {
        crate::validate::validate_body(self, None)
    }

    /// Checks the structural and type invariants of this body.
    ///
    /// The type invariants need `hir` to inspect declared types: no local or
    /// literal may keep an inference variable, a call must match its target's
    /// parameter count, an aggregate must carry the declared number of type
    /// arguments, and a by-value type parameter may only reach a symbolic trait
    /// call.
    pub fn validate_with(&self, hir: &seal_hir::Hir) -> Result<(), ValidationError> {
        crate::validate::validate_body(self, Some(hir))
    }
}
