//! The Seal high-level intermediate representation.
//!
//! Lowering turns the parser's [`Ast`](seal_ast::Ast) into a tree where every
//! name is resolved and every syntactic convenience is gone: parentheses are
//! dropped, and each function owns exactly one [`Body`].
//!
//! The result is total. A name that does not resolve, or a `break` outside a
//! loop, produces a diagnostic (`E2xxx`) and an `Error` node, so consumers can
//! keep walking without special cases.
//!
//! # Resolution
//!
//! Functions are collected before any body is lowered, so a function can call
//! one declared later. Inside a body, names resolve through a stack of ribs:
//! the innermost block wins, `let` bindings shadow parameters and outer
//! bindings, and a binding is not visible in its own initializer. Parameters and
//! bindings become [`DefId`](seal_def::DefId)s carrying their mutability, which
//! later phases use instead of re-reading the source.
//!
//! # Example
//!
//! ```ignore
//! let ast = seal_parser::parse(&lexed, session.diag(), session.interner());
//! let hir = seal_hir::lower(&ast, session.diag(), session.interner());
//! ```

#![warn(missing_docs)]

mod body;
mod expr;
mod generics;
mod id;
mod lower;
mod pattern;
mod primitives;
mod protocols;
mod res;
mod traits;

#[cfg(test)]
mod tests;

pub use body::{Block, Body, ForSource, ParamData, Stmt, StmtKind};
pub use expr::{Expr, ExprKind, Place, Projection};
pub use generics::InherentImpl;
pub use id::{BodyId, HirBlockId, HirExprId, HirPatternId, HirStmtId};
pub use pattern::{MatchArm, Pattern, PatternKind};
pub use primitives::primitive_kind;
pub use protocols::{Protocol, ProtocolDefs, ProtocolMethod};
pub use res::Res;
pub use traits::TraitMethodLookup;

use seal_ast::Ast;
use seal_def::{DefData, DefId, Definitions};
use seal_diagnostics::DiagCtxt;
use seal_hash::FxHashMap;
use seal_index::IndexVec;
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
use seal_ty::{
    AdtId, GenericParamDef, Generics, ParamEnv, ParamTyId, SubstsId, TraitDef, TraitImplDef,
    TyCtxt, TyId, TyKind, VariantIdx,
};

/// How an instance method takes its receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverKind {
    /// `self: Type` takes the value, moving or copying it.
    Value,
    /// `self: *Type` borrows the receiver for reading.
    Shared,
    /// `self: *mut Type` borrows the receiver for writing.
    Exclusive,
}

/// One source-level use of a `dyn Trait` type.
///
/// The type is recorded during lowering, but object safety can only be checked
/// once every trait definition exists, so validation runs as a final pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynTypeUse {
    /// The lowered trait-object type.
    pub ty: TyId,
    /// The source span of the complete `dyn ...` type.
    pub span: Span,
}

/// The lowered representation of one file.
#[derive(Debug)]
pub struct Hir {
    definitions: Definitions,
    bodies: IndexVec<BodyId, Body>,
    blocks: IndexVec<HirBlockId, Block>,
    stmts: IndexVec<HirStmtId, Stmt>,
    exprs: IndexVec<HirExprId, Expr>,
    patterns: IndexVec<HirPatternId, Pattern>,
    ty_ctxt: TyCtxt,
    body_of: FxHashMap<DefId, BodyId>,
    adts: FxHashMap<DefId, AdtId>,
    variants: FxHashMap<(DefId, Symbol), VariantIdx>,
    traits: FxHashMap<Symbol, DefId>,
    protocols: Option<ProtocolDefs>,
    trait_impls: traits::TraitImpls,
    /// Inherent associated functions, in declaration order per type and name.
    ///
    /// Several impl blocks may declare the same name for disjoint instances of
    /// one generic type, so a name maps to every candidate. Choosing one needs
    /// the type arguments or the receiver, which only the caller has.
    methods: FxHashMap<(DefId, Symbol), Vec<DefId>>,
    receivers: FxHashMap<DefId, ReceiverKind>,
    inherent_impls: Vec<InherentImpl>,
    dyn_types: Vec<DynTypeUse>,
}

impl Hir {
    /// Lowers a parsed file, reporting name-resolution errors through `dcx`.
    ///
    /// The returned tree is always complete; syntax or names that failed are
    /// represented by `Error` nodes after their diagnostic was emitted.
    pub fn lower(ast: &Ast, dcx: &DiagCtxt, interner: &Interner) -> Self {
        lower::lower_ast(ast, dcx, interner)
    }

    /// Returns the definition table.
    pub fn definitions(&self) -> &Definitions {
        &self.definitions
    }

    /// Returns the type context that owns every lowered type.
    pub fn ty_ctxt(&self) -> &TyCtxt {
        &self.ty_ctxt
    }

    /// Returns all bodies, indexable by [`BodyId`].
    pub fn bodies(&self) -> &IndexVec<BodyId, Body> {
        &self.bodies
    }

    /// Returns a body.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn body(&self, id: BodyId) -> &Body {
        &self.bodies[id]
    }

    /// Returns the ID of the body owned by `owner`, if it has one.
    pub fn body_id_of(&self, owner: DefId) -> Option<BodyId> {
        self.body_of.get(&owner).copied()
    }

    /// Returns the body owned by `owner`, if it has one.
    pub fn body_of(&self, owner: DefId) -> Option<&Body> {
        self.body_id_of(owner).map(|id| &self.bodies[id])
    }

    /// Returns a block.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn block(&self, id: HirBlockId) -> &Block {
        &self.blocks[id]
    }

    /// Returns a statement.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn stmt(&self, id: HirStmtId) -> &Stmt {
        &self.stmts[id]
    }

    /// Returns an expression.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn expr(&self, id: HirExprId) -> &Expr {
        &self.exprs[id]
    }

    /// Returns a pattern.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn pattern(&self, id: HirPatternId) -> &Pattern {
        &self.patterns[id]
    }

    /// Returns all patterns, indexable by [`HirPatternId`].
    pub fn pattern_arena(&self) -> &IndexVec<HirPatternId, Pattern> {
        &self.patterns
    }

    /// Returns the place denoted by `expr`, when it denotes storage.
    ///
    /// A local path is a place rooted at its definition; `*reference` is the
    /// place reached by one dereference; a field access projects from its base
    /// place. Borrows, calls, and literals are not places. Assignment and update
    /// targets are expressions, so this is how a consumer asks whether the
    /// target is assignable without matching on the tree.
    pub fn place(&self, expr: HirExprId) -> Option<Place> {
        match self.exprs[expr].kind {
            ExprKind::Path {
                res: Res::Local(def),
                ..
            } => Some(Place::local(def)),
            ExprKind::Deref { operand } => self.place(operand).map(Place::deref),
            ExprKind::Field { base, name, .. } => self.place(base).map(|place| place.field(name)),
            ExprKind::TupleField { base, index, .. } => self
                .place(base)
                .map(|place| place.tuple_field(index as usize)),
            ExprKind::Index { base, index } => self.place(base).map(|place| place.index(index)),
            _ => None,
        }
    }

    /// Returns the aggregate reserved for the nominal type definition `def`.
    ///
    /// Structs and enums both reserve one aggregate; use the type context to
    /// distinguish them when the distinction matters.
    pub fn adt(&self, def: DefId) -> Option<AdtId> {
        self.adts.get(&def).copied()
    }

    /// Returns the variant of the enum `type_def` named `name`.
    ///
    /// The registry is keyed by the enum definition, so a variant name is only
    /// found through its own enum. Structs have no entries.
    pub fn variant(&self, type_def: DefId, name: Symbol) -> Option<VariantIdx> {
        self.variants.get(&(type_def, name)).copied()
    }

    /// Returns the impl function named `name` attached to `type_def`.
    ///
    /// The lookup covers methods and associated functions. Use
    /// [`Hir::receiver`] to distinguish them.
    pub fn method(&self, type_def: DefId, name: Symbol) -> Option<DefId> {
        let candidates = self.methods(type_def, name);
        (candidates.len() == 1).then(|| candidates[0])
    }

    /// Returns every inherent function `name` declared for `type_def`.
    ///
    /// The candidates are in declaration order. Each one belongs to an impl
    /// block whose self type may restrict the instances where it applies.
    pub fn methods(&self, type_def: DefId, name: Symbol) -> &[DefId] {
        self.methods
            .get(&(type_def, name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the receiver kind of `function`, or `None` when it is associated.
    pub fn receiver(&self, function: DefId) -> Option<ReceiverKind> {
        self.receivers.get(&function).copied()
    }

    /// Returns the trait declared with `name`, if any.
    pub fn trait_def(&self, name: Symbol) -> Option<DefId> {
        self.traits.get(&name).copied()
    }

    /// Returns the semantic declaration of a trait.
    pub fn trait_definition(&self, def: DefId) -> Option<TraitDef> {
        self.ty_ctxt.trait_def(def)
    }

    /// Returns the definition IDs of the compiler-provided protocols.
    ///
    /// # Panics
    ///
    /// Panics only if the HIR was constructed without installing its built-in
    /// protocols, which is an internal invariant violation.
    pub fn protocol_defs(&self) -> ProtocolDefs {
        self.protocols
            .expect("every HIR installs the built-in protocols")
    }

    /// Returns the trait definition for `protocol`.
    pub fn protocol_trait(&self, protocol: Protocol) -> DefId {
        self.protocol_defs().trait_def(protocol)
    }

    /// Returns the protocol owned by the trait definition `def`.
    pub fn protocol_of_trait(&self, def: DefId) -> Option<Protocol> {
        self.protocols?.protocol_of_trait(def)
    }

    /// Returns the protocol and method named by a method definition.
    pub fn protocol_method_of(&self, def: DefId) -> Option<(Protocol, ProtocolMethod)> {
        self.protocols?.protocol_method_of(def)
    }

    /// Returns `true` when `def` is a compiler-provided protocol definition or
    /// one of its generic parameters.
    pub fn is_builtin_def(&self, def: DefId) -> bool {
        if self.protocol_of_trait(def).is_some() {
            return true;
        }
        self.definitions
            .get(def)
            .parent
            .is_some_and(|parent| self.is_builtin_def(parent))
    }

    /// Returns the generic parameters declared by `owner`.
    ///
    /// An impl method reports the impl as its parent; use
    /// [`Hir::flattened_generics`] for every parameter visible from its body.
    pub fn generics(&self, owner: DefId) -> Option<Generics> {
        self.ty_ctxt.generics(owner)
    }

    /// Returns every parameter visible from `owner`, outermost first.
    pub fn flattened_generics(&self, owner: DefId) -> Vec<GenericParamDef> {
        self.ty_ctxt.flattened_generics(owner)
    }

    /// Returns the bounds visible while checking `owner`.
    pub fn param_env(&self, owner: DefId) -> ParamEnv {
        self.ty_ctxt.param_env(owner)
    }

    /// Returns every trait implementation in declaration order.
    pub fn trait_impls(&self) -> Vec<TraitImplDef> {
        self.ty_ctxt.trait_impls()
    }

    /// Returns every `dyn Trait` use recorded during lowering.
    pub fn dyn_types(&self) -> &[DynTypeUse] {
        &self.dyn_types
    }

    /// Returns every inherent implementation in declaration order.
    pub fn inherent_impls(&self) -> &[InherentImpl] {
        &self.inherent_impls
    }

    /// Returns the impl blocks whose self type is the nominal type `type_def`.
    ///
    /// Both inherent and trait implementations are included, in declaration
    /// order. Generic impls match their nominal head, not a particular
    /// instantiation.
    pub fn impls_for(&self, type_def: DefId) -> Vec<DefId> {
        let matches = |ty: seal_ty::TyId| match self.ty_ctxt.kind(ty) {
            TyKind::Adt { adt, .. } => self.ty_ctxt.adt(adt).def == type_def,
            _ => false,
        };
        let mut impls: Vec<DefId> = self
            .inherent_impls
            .iter()
            .filter(|implementation| matches(implementation.self_ty))
            .map(|implementation| implementation.def)
            .collect();
        impls.extend(
            self.ty_ctxt
                .trait_impls()
                .into_iter()
                .filter(|implementation| matches(implementation.trait_ref.self_ty))
                .map(|implementation| implementation.def),
        );
        impls.sort_unstable();
        impls
    }

    /// Returns the resolved type arguments attached to `expr`, if written.
    pub fn expr_args(&self, expr: HirExprId) -> Option<SubstsId> {
        match self.expr(expr).kind {
            ExprKind::Path { args, .. }
            | ExprKind::Struct { args, .. }
            | ExprKind::Variant { args, .. } => args,
            ExprKind::MethodCall { generic_args, .. } => generic_args,
            _ => None,
        }
    }

    pub(crate) fn insert_inherent_impl(&mut self, implementation: InherentImpl) {
        self.inherent_impls.push(implementation);
    }

    /// Returns the trait method `name` implemented by `type_def`.
    ///
    /// Inherent functions are not considered here: callers try [`Hir::method`]
    /// first, so an inherent method shadows a trait method with the same name.
    pub fn trait_method(&self, type_def: DefId, name: Symbol) -> TraitMethodLookup {
        self.trait_impls.lookup(type_def, name, |trait_ref, name| {
            self.trait_definition(trait_ref.trait_def)
                .and_then(|definition| definition.method(name).cloned())
                .filter(|method| method.has_body)
                .map(|method| method.def)
        })
    }

    pub(crate) fn insert_adt(&mut self, def: DefId, adt: AdtId) {
        self.adts.insert(def, adt);
    }

    pub(crate) fn insert_variant(&mut self, type_def: DefId, name: Symbol, variant: VariantIdx) {
        self.variants.insert((type_def, name), variant);
    }

    pub(crate) fn insert_trait_name(&mut self, name: Symbol, def: DefId) {
        self.traits.insert(name, def);
    }

    pub(crate) fn insert_trait_impl(
        &mut self,
        trait_ref: seal_ty::TraitRef,
        type_def: DefId,
        functions: FxHashMap<Symbol, DefId>,
    ) {
        self.trait_impls.insert(trait_ref, type_def, functions);
    }

    pub(crate) fn insert_method(&mut self, type_def: DefId, name: Symbol, function: DefId) {
        self.methods
            .entry((type_def, name))
            .or_default()
            .push(function);
    }

    pub(crate) fn insert_receiver(&mut self, function: DefId, receiver: ReceiverKind) {
        self.receivers.insert(function, receiver);
    }

    pub(crate) fn insert_dyn_type(&mut self, usage: DynTypeUse) {
        self.dyn_types.push(usage);
    }

    /// Returns all blocks, indexable by [`HirBlockId`].
    pub fn block_arena(&self) -> &IndexVec<HirBlockId, Block> {
        &self.blocks
    }

    /// Returns all statements, indexable by [`HirStmtId`].
    pub fn stmt_arena(&self) -> &IndexVec<HirStmtId, Stmt> {
        &self.stmts
    }

    /// Returns all expressions, indexable by [`HirExprId`].
    pub fn expr_arena(&self) -> &IndexVec<HirExprId, Expr> {
        &self.exprs
    }

    pub(crate) fn empty(mut ty_ctxt: TyCtxt, interner: &Interner) -> Self {
        lower::intern_primitives(&mut ty_ctxt);
        let mut hir = Self {
            definitions: Definitions::new(),
            bodies: IndexVec::new(),
            blocks: IndexVec::new(),
            stmts: IndexVec::new(),
            exprs: IndexVec::new(),
            patterns: IndexVec::new(),
            ty_ctxt,
            body_of: FxHashMap::default(),
            adts: FxHashMap::default(),
            variants: FxHashMap::default(),
            traits: FxHashMap::default(),
            protocols: None,
            trait_impls: traits::TraitImpls::default(),
            methods: FxHashMap::default(),
            receivers: FxHashMap::default(),
            inherent_impls: Vec::new(),
            dyn_types: Vec::new(),
        };
        hir.protocols = Some(protocols::install(&mut hir, interner));
        hir
    }

    pub(crate) fn define(&mut self, data: DefData) -> DefId {
        self.definitions.push(data)
    }

    pub(crate) fn push_body(&mut self, body: Body) -> BodyId {
        self.bodies.push(body)
    }

    pub(crate) fn set_body_of(&mut self, owner: DefId, body: BodyId) {
        self.body_of.insert(owner, body);
    }

    pub(crate) fn push_block(&mut self, block: Block) -> HirBlockId {
        self.blocks.push(block)
    }

    pub(crate) fn push_stmt(&mut self, stmt: Stmt) -> HirStmtId {
        self.stmts.push(stmt)
    }

    pub(crate) fn push_expr(&mut self, expr: Expr) -> HirExprId {
        self.exprs.push(expr)
    }

    pub(crate) fn push_pattern(&mut self, pattern: Pattern) -> HirPatternId {
        self.patterns.push(pattern)
    }

    pub(crate) fn ty_ctxt_mut(&mut self) -> &mut TyCtxt {
        &mut self.ty_ctxt
    }

    #[allow(dead_code)]
    pub(crate) fn generic_param_owner(&self, param: ParamTyId) -> DefId {
        self.ty_ctxt.param_data(param).owner
    }
}

/// Lowers a parsed file, reporting name-resolution errors through `dcx`.
pub fn lower(ast: &Ast, dcx: &DiagCtxt, interner: &Interner) -> Hir {
    Hir::lower(ast, dcx, interner)
}
