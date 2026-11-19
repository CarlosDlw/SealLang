//! HIR-to-MIR lowering.

mod for_loop;
mod if_let;
mod match_expr;
mod while_let;

use seal_ast::{AssignOp, BinaryOp, Literal, UnaryOp, UpdateOp};
use seal_def::{DefId, DefKind};
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::{Body as HirBody, ExprKind, Hir, HirBlockId, HirExprId, HirStmtId, Res, StmtKind};
use seal_index::{Idx, IndexVec};
use seal_span::Span;
use seal_ty::{Instance, ParamEnv, RefKind, SubstsId, TyId, TyKind};
use seal_typeck::{
    Coercion, IndexProtocol, MethodKind, PlaceProjection, ProtocolCall, ReceiverAdjust,
    ResolvedMethod, ResolvedPlace, TypeckResults,
};

use crate::body::{BasicBlock, Body, BodyKind, LocalDecl, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::mir::Mir;
use crate::operand::{Constant, Operand};
use crate::place::{Place, PlaceRoot, Projection};
use crate::rvalue::{AggregateKind, BinOp, CallTarget, IntrinsicReceiver, Rvalue, UnOp};
use crate::statement::{DropKind, Statement};
use crate::terminator::{SwitchTarget, Terminator, TerminatorKind};

/// Converts a HIR body kind into its MIR form.
fn body_kind(kind: seal_hir::BodyKind) -> BodyKind {
    match kind {
        seal_hir::BodyKind::Function => BodyKind::Function,
        seal_hir::BodyKind::Const => BodyKind::Const,
        seal_hir::BodyKind::Static { mutable } => BodyKind::Static { mutable },
    }
}

/// Lowers every body in `hir` to MIR.
///
/// The caller must have run the type checker on `hir` without errors; lowering
/// relies on every value expression having a type and every assignment target
/// being a local place.
pub fn lower(hir: &Hir, typeck: &TypeckResults) -> Mir {
    let mut mir = Mir::new();
    for body in hir.bodies().iter() {
        mir.push_body(BodyLowering::new(hir, typeck, body).run(body));
    }
    mir
}

#[derive(Default)]
struct BlockBuilder {
    statements: Vec<Statement>,
    terminator: Option<Terminator>,
}

#[derive(Clone, Copy)]
struct LoopTargets {
    break_to: BasicBlockId,
    continue_to: BasicBlockId,
    /// The scope index of the loop body: leaving the loop drops that scope and
    /// everything inside it, and nothing declared before the loop.
    depth: usize,
}

/// The optional bounds of a slice range.
#[derive(Clone, Copy)]
struct SliceBounds {
    start: Option<HirExprId>,
    end: Option<HirExprId>,
}

/// The resolved pieces of one intrinsic method call.
struct IntrinsicCall<'a> {
    receiver: HirExprId,
    name_span: Span,
    args: &'a [HirExprId],
    dest: Option<Place>,
    span: Span,
    method: seal_methods::MethodId,
    resolved: &'a seal_typeck::ResolvedMethod,
}

struct BodyLowering<'a> {
    hir: &'a Hir,
    typeck: &'a TypeckResults,
    owner: DefId,
    kind: BodyKind,
    is_const: bool,
    locals: IndexVec<LocalId, LocalDecl>,
    local_of: FxHashMap<DefId, LocalId>,
    blocks: IndexVec<BasicBlockId, BlockBuilder>,
    current: Option<BasicBlockId>,
    entry: BasicBlockId,
    loops: Vec<LoopTargets>,
    arg_count: usize,
    param_env: ParamEnv,
    /// The scopes enclosing the code being lowered, outermost first.
    ///
    /// Each entry owns the locals it declares, in declaration order. A scope's
    /// drops are emitted once per path that reaches the end of the scope: the
    /// fallthrough path emits them where the scope ends, and a path that leaves
    /// early — `return`, `break`, `continue` — emits them where it leaves.
    /// Which is why a scope is never marked: two paths through one scope are
    /// two different drop sites, not a duplicate.
    scopes: Vec<Vec<LocalId>>,
    /// The number of locals that existed when the current statement started.
    temp_window: usize,
    /// Temporaries the current statement must not drop itself.
    ///
    /// A temporary whose address outlives the statement joins the enclosing
    /// block instead, and a by-value loop owns its array, dropping only the
    /// elements the body left behind.
    detached_temps: FxHashSet<LocalId>,
}

impl<'a> BodyLowering<'a> {
    fn new(hir: &'a Hir, typeck: &'a TypeckResults, body: &HirBody) -> Self {
        let span = hir.definitions().get(body.owner).span;
        let signature = typeck
            .signature(body.owner)
            .expect("type checking gives every body a signature");
        let ret = signature.ret;
        let param_types = signature.params.clone();
        let param_env = hir.param_env(body.owner);

        let placeholder = BasicBlockId::from_u32(0).expect("zero is a valid block index");
        let mut lowering = Self {
            hir,
            typeck,
            owner: body.owner,
            kind: body_kind(body.kind),
            is_const: body.is_const,
            locals: IndexVec::new(),
            local_of: FxHashMap::default(),
            blocks: IndexVec::new(),
            current: None,
            entry: placeholder,
            loops: Vec::new(),
            arg_count: body.params.len(),
            param_env,
            scopes: Vec::new(),
            temp_window: 0,
            detached_temps: FxHashSet::default(),
        };

        let return_local = lowering.push_local(LocalDecl {
            ty: ret,
            name: None,
            kind: LocalKind::Return,
            span,
        });
        debug_assert_eq!(return_local, Body::return_local());

        let entry = lowering.new_block();
        lowering.entry = entry;
        lowering.current = Some(entry);

        let mut parameters = Vec::with_capacity(body.params.len());
        for (param, ty) in body.params.iter().zip(&param_types) {
            let definition = hir.definitions().get(param.def);
            let local = lowering.push_local(LocalDecl {
                ty: *ty,
                name: Some(definition.name),
                kind: LocalKind::Arg,
                span: definition.span,
            });
            lowering.local_of.insert(param.def, local);
            parameters.push(local);
        }
        // Parameters are owned by the function itself: their drops run after the
        // body's own scopes, when the value is about to leave the frame.
        lowering.push_scope(parameters);

        lowering
    }

    fn run(mut self, body: &HirBody) -> Body {
        // A tail expression is the returned value, so the implicit return
        // belongs to that expression. The block span covers the braces and
        // would point at the whole function body in diagnostics.
        let return_span = self
            .hir
            .block(body.root)
            .tail
            .map(|tail| self.hir.expr(tail).span);
        self.lower_block(body.root, Some(self.return_place()));
        if self.current.is_some() {
            let span = return_span.unwrap_or_else(|| self.hir.block(body.root).span);
            // The body's own scope ended with its block; the parameters are the
            // last values this frame owns.
            self.drop_scopes_down_to(0, span);
            self.terminate(TerminatorKind::Return, span);
        }
        self.finish()
    }

    fn finish(self) -> Body {
        let Self {
            owner,
            kind,
            is_const,
            arg_count,
            locals,
            blocks,
            entry,
            ..
        } = self;
        let blocks = blocks
            .into_iter()
            .map(|block| BasicBlock {
                statements: block.statements,
                terminator: block
                    .terminator
                    .expect("every MIR block receives a terminator"),
            })
            .collect();
        Body {
            owner,
            kind,
            is_const,
            arg_count,
            locals,
            blocks,
            entry,
        }
    }

    fn new_block(&mut self) -> BasicBlockId {
        self.blocks.push(BlockBuilder::default())
    }

    fn ensure_block(&mut self) -> BasicBlockId {
        match self.current {
            Some(block) => block,
            None => {
                let block = self.new_block();
                self.current = Some(block);
                block
            }
        }
    }

    fn terminate(&mut self, kind: TerminatorKind, span: Span) {
        let block = self.ensure_block();
        debug_assert!(
            self.blocks[block].terminator.is_none(),
            "a MIR block was terminated twice"
        );
        self.blocks[block].terminator = Some(Terminator { kind, span });
        self.current = None;
    }

    fn terminate_goto(&mut self, target: BasicBlockId, span: Span) {
        self.terminate(TerminatorKind::Goto(target), span);
    }

    fn push_local(&mut self, declaration: LocalDecl) -> LocalId {
        self.locals.push(declaration)
    }

    fn new_temp(&mut self, ty: TyId, span: Span) -> Place {
        let local = self.push_local(LocalDecl {
            ty,
            name: None,
            kind: LocalKind::Temp,
            span,
        });
        Place::local(local)
    }

    fn return_place(&self) -> Place {
        Place::local(Body::return_local())
    }

    fn push_statement(&mut self, statement: Statement) {
        let block = self.ensure_block();
        self.blocks[block].statements.push(statement);
    }

    /// Starts a scope that owns `locals`.
    fn push_scope(&mut self, locals: Vec<LocalId>) {
        self.scopes.push(locals);
    }

    /// Adds a local to the innermost scope, so it is dropped when that ends.
    fn register_local(&mut self, local: LocalId) {
        let scope = self
            .scopes
            .last_mut()
            .expect("a local is declared inside the body's first scope");
        scope.push(local);
    }

    /// Ends the innermost scope, dropping the values it still owns.
    ///
    /// A block whose path already ended in a terminator has no `current` block
    /// and emits nothing here: that path dropped these locals where it left.
    fn pop_scope(&mut self, span: Span) {
        let locals = self
            .scopes
            .pop()
            .expect("every scope is popped by the block that pushed it");
        self.drop_locals(&locals, span);
    }

    /// Drops every scope from `depth` inwards, innermost first.
    ///
    /// A path that leaves early (`return`, `break`, `continue`) unwinds the
    /// scopes it skips this way. The scopes stay on the stack, because the
    /// blocks that pushed them still end on their own fallthrough paths, which
    /// need the same drops at their own end.
    fn drop_scopes_down_to(&mut self, depth: usize, span: Span) {
        for index in (depth..self.scopes.len()).rev() {
            let locals = self.scopes[index].clone();
            self.drop_locals(&locals, span);
        }
    }

    /// Emits a scope drop for every local that needs a destructor.
    ///
    /// A `Copy` value has no destructor of its own, and neither does an
    /// aggregate whose fields are all trivial, so `needs_drop` decides whether a
    /// site exists at all.
    fn drop_locals(&mut self, locals: &[LocalId], span: Span) {
        if self.current.is_none() {
            // There is no block left to write to. A path only reaches here after
            // it left through a terminator, which already dropped the scopes it
            // unwound.
            return;
        }
        for local in locals.iter().rev() {
            if self.hir.ty_ctxt().needs_drop(self.locals[*local].ty) {
                self.push_statement(Statement::drop(Place::local(*local), DropKind::Scope, span));
            }
        }
    }

    /// Drops the temporaries the current statement created.
    ///
    /// A temporary lives only as long as the statement that built it, unless it
    /// was detached: an escaping address hands it to the enclosing block, and a
    /// by-value loop owns its array until its exit.
    fn drop_statement_temps(&mut self, span: Span) {
        if self.current.is_none() {
            return;
        }
        for index in (self.temp_window..self.locals.len()).rev() {
            let local = LocalId::from_usize(index);
            if self.detached_temps.contains(&local)
                || self.locals[local].kind != LocalKind::Temp
                || !self.hir.ty_ctxt().needs_drop(self.locals[local].ty)
            {
                continue;
            }
            self.push_statement(Statement::drop(Place::local(local), DropKind::Scope, span));
        }
    }

    /// Returns whether `local` was created by the current statement.
    fn is_statement_temp(&self, local: LocalId) -> bool {
        local.index() as usize >= self.temp_window && self.locals[local].kind == LocalKind::Temp
    }

    /// Returns whether `place` is a whole temporary of the current statement.
    fn is_statement_temp_place(&self, place: &Place) -> bool {
        place.projections.is_empty()
            && place
                .local_id()
                .is_some_and(|local| self.is_statement_temp(local))
    }

    /// Hands a temporary to the enclosing block.
    ///
    /// The temporary is registered as a block local, so it is dropped with the
    /// block, after the locals declared inside it.
    fn promote_temp(&mut self, local: LocalId) {
        if self.detached_temps.insert(local) {
            self.register_local(local);
        }
    }

    /// Leaves a temporary to be dropped by something other than its statement.
    ///
    /// A by-value loop owns its array: it drops the elements the body did not
    /// take when it exits, so the statement that built the array must not drop
    /// it as well.
    fn hand_over_temp(&mut self, local: LocalId) {
        self.detached_temps.insert(local);
    }

    /// Keeps the storage a borrow reads alive for as long as the borrow is.
    ///
    /// A statement drops its own temporaries when it ends. When the borrow is
    /// stored somewhere that outlives the statement, the storage would die with
    /// it, so the temporary joins the enclosing block instead — exactly like a
    /// binding, and dropped after the block's own locals.
    fn promote_borrowed_temp(&mut self, place: &Place, dest: &Place) {
        let Some(local) = place.local_id() else {
            return;
        };
        if self.is_statement_temp(local) && !self.is_statement_temp_place(dest) {
            self.promote_temp(local);
        }
    }

    fn assign_constant(&mut self, place: Place, constant: Constant, span: Span) {
        self.assign_rvalue(place, Rvalue::Use(Operand::Constant(constant)), span);
    }

    fn assign_operand(&mut self, place: Place, operand: Operand, span: Span) {
        self.assign_rvalue(place, Rvalue::Use(operand), span);
    }

    fn assign_rvalue(&mut self, place: Place, rvalue: Rvalue, span: Span) {
        self.push_statement(Statement::assign(place, rvalue, span));
    }

    fn lower_block(&mut self, block: HirBlockId, dest: Option<Place>) {
        self.lower_block_with(block, dest, Vec::new());
    }

    /// Lowers a block whose scope already owns `bindings`.
    ///
    /// A pattern binding is assigned before its body runs, so it is handed to
    /// the body's scope: it must be dropped when that block ends, after the
    /// locals the block declares itself. `if let`, `while let`, `match`, and
    /// `for` pass their bindings here.
    fn lower_block_with(&mut self, block: HirBlockId, dest: Option<Place>, bindings: Vec<LocalId>) {
        self.push_scope(bindings);
        let statement_count = self.hir.block(block).statements.len();
        for index in 0..statement_count {
            let statement = self.hir.block(block).statements[index];
            self.lower_stmt(statement);
        }

        if let Some(tail) = self.hir.block(block).tail {
            // The tail is the block's last expression, so the temporaries it
            // builds live until the block ends. They are dropped here, after the
            // value the expression produces has been stored.
            let outer_window = self.temp_window;
            let outer_detached = std::mem::take(&mut self.detached_temps);
            self.temp_window = self.locals.len();
            self.lower_expr_into(tail, dest);
            self.drop_statement_temps(self.hir.expr(tail).span);
            self.temp_window = outer_window;
            self.detached_temps = outer_detached;
        } else if let Some(dest) = dest
            && self.current.is_some()
        {
            let span = self.hir.block(block).span;
            self.assign_constant(dest, Constant::Unit, span);
        }
        self.pop_scope(self.hir.block(block).span);
    }

    /// Lowers one statement and drops the temporaries it created.
    fn lower_stmt(&mut self, statement: HirStmtId) {
        let (span, kind) = {
            let statement = self.hir.stmt(statement);
            (statement.span, statement.kind.clone())
        };

        // A statement owns the temporaries it builds. Nested statements open
        // their own window, so the enclosing one is restored afterwards.
        let outer_window = self.temp_window;
        let outer_detached = std::mem::take(&mut self.detached_temps);
        self.temp_window = self.locals.len();
        self.lower_stmt_kind(span, kind);
        self.drop_statement_temps(span);
        self.temp_window = outer_window;
        self.detached_temps = outer_detached;
    }

    fn lower_stmt_kind(&mut self, span: Span, kind: StmtKind) {
        match kind {
            StmtKind::Let { local, init, .. } => {
                let ty = self.expr_or_local_ty(local);
                let name = Some(self.hir.definitions().name(local));
                let mir_local = self.push_local(LocalDecl {
                    ty,
                    name,
                    kind: LocalKind::Binding,
                    span,
                });
                self.local_of.insert(local, mir_local);
                if let Some(init) = init {
                    self.lower_expr_into(init, Some(Place::local(mir_local)));
                }
                // Registered after the initializer, so its temporaries are
                // dropped before the binding they fed.
                self.register_local(mir_local);
            }
            StmtKind::Return(value) => {
                let dest = self.return_place();
                let return_span = value.map_or(span, |value| self.hir.expr(value).span);
                match value {
                    Some(value) => self.lower_expr_into(value, Some(dest)),
                    None => self.assign_constant(dest, Constant::Unit, span),
                }
                // The return value is already in place. Operands' temporaries
                // die first, then every scope from the innermost outwards.
                self.drop_statement_temps(span);
                self.drop_scopes_down_to(0, span);
                self.terminate(TerminatorKind::Return, return_span);
            }
            StmtKind::While { condition, body } => self.lower_while(condition, body, span),
            StmtKind::WhileLet {
                pattern,
                scrutinee,
                body,
            } => self.lower_while_let(pattern, scrutinee, body, span),
            StmtKind::For {
                pattern,
                source,
                body,
            } => self.lower_for(pattern, source, body, span),
            StmtKind::Break => {
                let targets = *self
                    .loops
                    .last()
                    .expect("type checking rejects `break` outside a loop");
                // Leaving the loop drops its body's scopes and nothing before
                // them: the loop may still be in use by the code around it.
                self.drop_statement_temps(span);
                self.drop_scopes_down_to(targets.depth, span);
                self.terminate_goto(targets.break_to, span);
            }
            StmtKind::Continue => {
                let targets = *self
                    .loops
                    .last()
                    .expect("type checking rejects `continue` outside a loop");
                self.drop_statement_temps(span);
                self.drop_scopes_down_to(targets.depth, span);
                self.terminate_goto(targets.continue_to, span);
            }
            StmtKind::Const { .. } | StmtKind::Static { .. } => {
                // The initializer is its own body; the statement only scopes the
                // name for name resolution.
            }
            StmtKind::Expr(expr) => self.lower_expr_into(expr, None),
            StmtKind::Error => {}
        }
    }

    fn lower_while(&mut self, condition: HirExprId, body: HirBlockId, span: Span) {
        let condition_block = self.new_block();
        let body_block = self.new_block();
        let exit_block = self.new_block();

        self.terminate_goto(condition_block, span);
        self.current = Some(condition_block);

        let discriminant = self.lower_operand(condition);
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant,
                targets: vec![SwitchTarget {
                    value: 1,
                    target: body_block,
                }],
                otherwise: exit_block,
            },
            span,
        );

        self.current = Some(body_block);
        let depth = self.scopes.len();
        self.loops.push(LoopTargets {
            break_to: exit_block,
            continue_to: condition_block,
            depth,
        });
        self.lower_block(body, None);
        self.loops.pop();
        if self.current.is_some() {
            self.terminate_goto(condition_block, span);
        }

        self.current = Some(exit_block);
    }

    fn lower_expr_into(&mut self, expr: HirExprId, dest: Option<Place>) {
        if let Some(coercion) = self.typeck.coercion(expr) {
            self.lower_coerced_expr_into(expr, coercion, dest);
            return;
        }
        self.lower_expr_into_uncoerced(expr, dest);
    }

    /// Lowers an expression whose type checker result selected a coercion.
    ///
    /// The source expression is lowered with its natural type and the recorded
    /// conversion is emitted explicitly. No later phase re-infers why the
    /// source and destination types differ.
    fn lower_coerced_expr_into(
        &mut self,
        expr: HirExprId,
        coercion: Coercion,
        dest: Option<Place>,
    ) {
        let span = self.hir.expr(expr).span;
        match coercion {
            Coercion::TraitObject { source, target } => {
                let source_place = self.new_temp(source, span);
                self.lower_expr_into_uncoerced(expr, Some(source_place.clone()));
                // The object takes the source value, so an owned source is moved
                // out of the temporary rather than duplicated: otherwise the
                // temporary and the object would each drop the same value.
                let source_operand = self.place_operand(source_place, source);
                let target_place = dest.unwrap_or_else(|| self.new_temp(target, span));
                self.assign_rvalue(
                    target_place,
                    Rvalue::MakeTraitObject {
                        source: source_operand,
                        source_ty: source,
                        target_ty: target,
                    },
                    span,
                );
            }
            Coercion::TraitObjectReborrow { source, target } => {
                let (place, pointer) = self.lower_reborrow_parts(expr, source, span);
                let target_place = dest.unwrap_or_else(|| self.new_temp(target, span));
                self.assign_rvalue(
                    target_place,
                    Rvalue::TraitObjectReborrow {
                        source: pointer,
                        place,
                        from: source,
                        to: target,
                    },
                    span,
                );
            }
        }
    }

    /// Returns the pointee place and source pointer of a recorded dyn reborrow.
    fn lower_reborrow_parts(
        &mut self,
        expr: HirExprId,
        source_ty: TyId,
        span: Span,
    ) -> (Place, Operand) {
        let ExprKind::Borrow { operand, .. } = self.hir.expr(expr).kind.clone() else {
            panic!("a trait-object reborrow must be a borrow expression; this is a compiler bug")
        };
        let ExprKind::Deref { operand: pointer } = self.hir.expr(operand).kind.clone() else {
            panic!("a trait-object reborrow must dereference a pointer; this is a compiler bug")
        };
        let (pointer_place, source) =
            self.lower_pointer_place_and_operand(pointer, source_ty, span);
        (pointer_place.deref(), source)
    }

    /// Lowers a pointer value as both storage and the operand read from it.
    ///
    /// A pointer with a resolved place is read directly; any other value is
    /// materialized once in a temporary, so a reborrow never evaluates its
    /// source pointer twice.
    fn lower_pointer_place_and_operand(
        &mut self,
        expr: HirExprId,
        ty: TyId,
        span: Span,
    ) -> (Place, Operand) {
        if let Some(place) = self.typeck.place(expr).cloned() {
            let place = self.resolved_place(&place);
            let operand = self.place_operand(place.clone(), ty);
            return (place, operand);
        }
        let temporary = self.new_temp(ty, span);
        self.lower_expr_into(expr, Some(temporary.clone()));
        (temporary.clone(), Operand::Copy(temporary))
    }

    fn lower_expr_into_uncoerced(&mut self, expr: HirExprId, dest: Option<Place>) {
        let (span, kind) = {
            let expr = self.hir.expr(expr);
            (expr.span, expr.kind.clone())
        };

        match kind {
            ExprKind::Lit(literal) => {
                if let Some(dest) = dest {
                    let ty = self.expr_ty(expr);
                    let constant = self.constant(literal, ty);
                    self.assign_constant(dest, constant, span);
                }
            }
            ExprKind::Path {
                res: Res::Local(local),
                ..
            } => {
                if let Some(dest) = dest {
                    let place = self.place_of_local(local);
                    let ty = self.expr_ty(expr);
                    let operand = self.place_operand(place, ty);
                    self.assign_operand(dest, operand, span);
                }
            }
            ExprKind::Path {
                res: Res::Const(def),
                ..
            } => {
                if let Some(dest) = dest {
                    self.assign_operand(dest, Operand::Const(def), span);
                }
            }
            ExprKind::Path {
                res: Res::Static(def),
                ..
            } => {
                if let Some(dest) = dest {
                    let place = Place::static_root(def);
                    let ty = self.expr_ty(expr);
                    let operand = if self.hir.ty_ctxt().is_copy_with(ty, &self.param_env) {
                        Operand::Copy(place)
                    } else {
                        Operand::Move(place)
                    };
                    self.assign_operand(dest, operand, span);
                }
            }
            ExprKind::Path {
                res: Res::Function(_) | Res::TraitFunction { .. },
                ..
            }
            | ExprKind::Error => {
                if let Some(dest) = dest {
                    self.assign_constant(dest, Constant::Unit, span);
                }
            }
            ExprKind::Borrow { operand, kind } => {
                if let Some(dest) = dest {
                    let place = self.place_of_expr(operand);
                    let ty = self.expr_ty(expr);
                    // A borrow stored beyond this statement keeps its storage
                    // alive; one consumed by the statement does not.
                    self.promote_borrowed_temp(&place, &dest);
                    let is_slice = matches!(
                        self.hir.ty_ctxt().kind(ty),
                        TyKind::Ref { pointee, .. }
                            if matches!(self.hir.ty_ctxt().kind(pointee), TyKind::Slice { .. })
                    );
                    if is_slice {
                        self.assign_rvalue(
                            dest,
                            Rvalue::SliceRef {
                                base: place,
                                start: None,
                                end: None,
                                kind,
                                ty,
                            },
                            span,
                        );
                    } else {
                        self.assign_rvalue(dest, Rvalue::Ref { place, kind }, span);
                    }
                }
            }
            ExprKind::SliceRef {
                base,
                start,
                end,
                kind,
            } => {
                self.lower_slice_ref(base, SliceBounds { start, end }, kind, dest, span);
            }
            ExprKind::Len { operand } => {
                let slice = self.lower_operand(operand);
                if let Some(dest) = dest {
                    self.assign_rvalue(dest, Rvalue::Len { slice }, span);
                }
            }
            ExprKind::Drop { operand } => {
                // The operand is lowered as a place, so a non-`Copy` value is
                // moved out of it by the statement itself. A value that does
                // not need a destructor is still evaluated, but there is
                // nothing to emit.
                let place = self.place_of_expr(operand);
                let ty = self.expr_ty(operand);
                if self.hir.ty_ctxt().needs_drop(ty) {
                    self.push_statement(Statement::drop(place, DropKind::Explicit, span));
                }
                if let Some(dest) = dest {
                    self.assign_constant(dest, Constant::Unit, span);
                }
            }
            ExprKind::Slice { .. }
            | ExprKind::Deref { .. }
            | ExprKind::Field { .. }
            | ExprKind::TupleField { .. }
            | ExprKind::Index { .. } => {
                if let Some(dest) = dest {
                    let place = self.place_of_expr(expr);
                    let ty = self.expr_ty(expr);
                    let operand = self.place_operand(place, ty);
                    self.assign_operand(dest, operand, span);
                }
            }
            ExprKind::Tuple { elements } => {
                let ty = self.expr_ty(expr);
                let elements: Vec<Operand> = elements
                    .iter()
                    .map(|element| self.lower_operand(*element))
                    .collect();
                if let Some(dest) = dest {
                    self.assign_rvalue(dest, Rvalue::Tuple { elements, ty }, span);
                }
            }
            ExprKind::Array { elements } => {
                let ty = self.expr_ty(expr);
                let elements: Vec<Operand> = elements
                    .iter()
                    .map(|element| self.lower_operand(*element))
                    .collect();
                if let Some(dest) = dest {
                    self.assign_rvalue(dest, Rvalue::Array { elements, ty }, span);
                }
            }
            ExprKind::ArrayRepeat { value, length } => {
                let ty = self.expr_ty(expr);
                let value = self.lower_operand(value);
                if let Some(dest) = dest {
                    self.assign_rvalue(
                        dest,
                        Rvalue::Repeat {
                            value,
                            count: length,
                            ty,
                        },
                        span,
                    );
                }
            }
            ExprKind::Struct { adt, fields, .. } => {
                if let Some(dest) = dest {
                    let args = self.instantiation_args(expr);
                    let fields = fields
                        .iter()
                        .map(|(field, value)| (*field, self.lower_operand(*value)))
                        .collect();
                    self.assign_rvalue(
                        dest,
                        Rvalue::Aggregate {
                            kind: AggregateKind::Struct { adt, args },
                            fields,
                        },
                        span,
                    );
                }
            }
            ExprKind::Variant {
                adt,
                variant,
                fields,
                ..
            } => {
                if let Some(dest) = dest {
                    let args = self.instantiation_args(expr);
                    let fields = fields
                        .iter()
                        .map(|(field, value)| (*field, self.lower_operand(*value)))
                        .collect();
                    self.assign_rvalue(
                        dest,
                        Rvalue::Aggregate {
                            kind: AggregateKind::Variant { adt, variant, args },
                            fields,
                        },
                        span,
                    );
                }
            }
            ExprKind::Unary { op, operand } => {
                let operand = self.lower_operand(operand);
                if let Some(dest) = dest {
                    self.assign_rvalue(
                        dest,
                        Rvalue::Unary {
                            op: un_op(op),
                            operand,
                        },
                        span,
                    );
                }
            }
            ExprKind::Binary {
                op: op @ (BinaryOp::And | BinaryOp::Or),
                lhs,
                rhs,
            } => {
                let op = logical_op(op);
                self.lower_logical(op, lhs, rhs, dest, span)
            }
            ExprKind::Binary { op, lhs, rhs } => {
                // Equality only reads its operands, so it never takes ownership
                // of them: after `a == b` both values are still usable. Lowering
                // the operands as `Copy` expresses that read even for an
                // aggregate that is not `Copy`; code generation loads the value
                // and the owner keeps dropping it exactly once.
                let reads = matches!(op, BinaryOp::Eq | BinaryOp::Ne);
                let lhs = self.lower_operand_for(lhs, reads);
                let rhs = self.lower_operand_for(rhs, reads);
                if let Some(dest) = dest {
                    self.assign_rvalue(
                        dest,
                        Rvalue::Binary {
                            op: bin_op(op),
                            lhs,
                            rhs,
                            span,
                        },
                        span,
                    );
                }
            }
            ExprKind::Cast { operand, .. } => {
                let from = self.expr_ty(operand);
                let to = self.expr_ty(expr);
                let kind = self.typeck.cast_kind(expr).unwrap_or_else(|| {
                    panic!(
                        "cast expression {expr:?} has no recorded kind; \
                         this is an internal compiler error"
                    )
                });
                let operand = self.lower_operand(operand);
                if let Some(dest) = dest {
                    let tcx = self.hir.ty_ctxt();
                    let unsizes_to_object = match (tcx.kind(from), tcx.kind(to)) {
                        (
                            TyKind::Ref {
                                pointee: source_pointee,
                                ..
                            },
                            TyKind::RawPtr {
                                pointee: target_pointee,
                                ..
                            },
                        ) => {
                            !matches!(tcx.kind(source_pointee), TyKind::Dyn { .. })
                                && matches!(tcx.kind(target_pointee), TyKind::Dyn { .. })
                        }
                        _ => false,
                    };
                    let rvalue = if unsizes_to_object {
                        Rvalue::MakeTraitObject {
                            source: operand,
                            source_ty: from,
                            target_ty: to,
                        }
                    } else {
                        Rvalue::Cast {
                            kind,
                            operand,
                            from,
                            to,
                        }
                    };
                    self.assign_rvalue(dest, rvalue, span);
                }
            }
            ExprKind::Assign { op, target, value } => {
                self.lower_assign(op, target, value, dest, span)
            }
            ExprKind::Update { op, target, .. } => self.lower_update(op, target, dest, span),
            ExprKind::Call { callee, args } => {
                let result_ty = self.expr_ty(expr);
                self.lower_call(expr, callee, &args, dest, result_ty, span);
            }
            ExprKind::MethodCall {
                receiver,
                name_span,
                args,
                ..
            } => self.lower_method(expr, receiver, name_span, &args, dest, span),
            ExprKind::Match { scrutinee, arms } => self.lower_match(scrutinee, &arms, dest, span),
            ExprKind::Block(block) | ExprKind::Unsafe { block } => self.lower_block(block, dest),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if(condition, then_branch, else_branch, dest, span),
            ExprKind::IfLet {
                pattern,
                scrutinee,
                then_branch,
                else_branch,
            } => self.lower_if_let(pattern, scrutinee, then_branch, else_branch, dest, span),
        }
    }

    fn lower_logical(
        &mut self,
        op: LogicalOp,
        lhs: HirExprId,
        rhs: HirExprId,
        dest: Option<Place>,
        span: Span,
    ) {
        let lhs = self.lower_operand(lhs);
        let rhs_block = self.new_block();
        let short_block = self.new_block();
        let join_block = self.new_block();

        let (true_target, false_target) = match op {
            LogicalOp::And => (rhs_block, short_block),
            LogicalOp::Or => (short_block, rhs_block),
        };
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant: lhs,
                targets: vec![SwitchTarget {
                    value: 1,
                    target: true_target,
                }],
                otherwise: false_target,
            },
            span,
        );

        self.current = Some(short_block);
        if let Some(dest) = &dest {
            self.assign_constant(
                dest.clone(),
                Constant::Bool(matches!(op, LogicalOp::Or)),
                span,
            );
        }
        self.terminate_goto(join_block, span);

        self.current = Some(rhs_block);
        self.lower_expr_into(rhs, dest);
        if self.current.is_some() {
            self.terminate_goto(join_block, span);
        }

        self.current = Some(join_block);
    }

    fn lower_if(
        &mut self,
        condition: HirExprId,
        then_branch: HirBlockId,
        else_branch: Option<HirBlockId>,
        dest: Option<Place>,
        span: Span,
    ) {
        let discriminant = self.lower_operand(condition);

        let then_block = self.new_block();
        let else_block = self.new_block();
        let join_block = self.new_block();
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant,
                targets: vec![SwitchTarget {
                    value: 1,
                    target: then_block,
                }],
                otherwise: else_block,
            },
            span,
        );

        self.current = Some(then_block);
        self.lower_block(then_branch, dest.clone());
        if self.current.is_some() {
            self.terminate_goto(join_block, span);
        }

        self.current = Some(else_block);
        if let Some(else_branch) = else_branch {
            self.lower_block(else_branch, dest);
        } else if let Some(dest) = dest {
            self.assign_constant(dest, Constant::Unit, span);
        }
        if self.current.is_some() {
            self.terminate_goto(join_block, span);
        }

        self.current = Some(join_block);
    }

    fn lower_assign(
        &mut self,
        op: AssignOp,
        target: HirExprId,
        value: HirExprId,
        dest: Option<Place>,
        span: Span,
    ) {
        match op {
            AssignOp::Assign => {
                // A simple assignment evaluates its value before the place.
                // This lets the value read from the same container that the
                // target place later borrows, matching the language's
                // assignment evaluation order.
                let value = self.lower_operand(value);
                let target_place = self.place_of_expr(target);
                self.assign_operand(target_place, value, span);
            }
            _ => {
                let target_place = self.place_of_expr(target);
                let target_ty = self.expr_ty(target);
                let rhs = self.lower_operand(value);
                let lhs = self.place_operand(target_place.clone(), target_ty);
                self.assign_rvalue(
                    target_place,
                    Rvalue::Binary {
                        op: compound_bin_op(op),
                        lhs,
                        rhs,
                        span,
                    },
                    span,
                );
            }
        }

        if let Some(dest) = dest {
            self.assign_constant(dest, Constant::Unit, span);
        }
    }

    fn lower_update(&mut self, op: UpdateOp, target: HirExprId, dest: Option<Place>, span: Span) {
        let target_place = self.place_of_expr(target);
        let target_ty = self.expr_ty(target);
        let lhs = self.place_operand(target_place.clone(), target_ty);
        let rhs = Operand::Constant(self.constant_one(target_ty));
        let op = match op {
            UpdateOp::Increment => BinOp::Add,
            UpdateOp::Decrement => BinOp::Sub,
        };
        self.assign_rvalue(target_place, Rvalue::Binary { op, lhs, rhs, span }, span);

        if let Some(dest) = dest {
            self.assign_constant(dest, Constant::Unit, span);
        }
    }

    fn lower_call(
        &mut self,
        expr: HirExprId,
        callee: HirExprId,
        args: &[HirExprId],
        dest: Option<Place>,
        result_ty: TyId,
        span: Span,
    ) {
        let function = match self.hir.expr(callee).kind.clone() {
            ExprKind::Path {
                res: Res::Function(definition),
                ..
            } => Some(definition),
            ExprKind::Path {
                res: Res::TraitFunction { function, .. },
                ..
            } => Some(function),
            _ => None,
        };
        let arg_spans: Vec<Span> = args.iter().map(|arg| self.hir.expr(*arg).span).collect();
        let args: Vec<Operand> = args.iter().map(|arg| self.lower_operand(*arg)).collect();

        let Some(function) = function else {
            if let Some(dest) = dest {
                self.assign_constant(dest, Constant::Unit, span);
            }
            return;
        };
        let instance = self.call_instance(expr, function);
        // A trait method reached through a type parameter has no concrete
        // function yet, so monomorphization receives the symbolic target. The
        // same syntax on a concrete type names a trait implementation and is a
        // direct call like any other function.
        let symbolic = self.hir.definitions().kind(function) == DefKind::TraitMethod;
        let parameters = if symbolic {
            self.trait_method_parameters(function)
        } else {
            self.typeck
                .signature(function)
                .map_or(args.len(), |signature| signature.params.len())
        };
        let target = if symbolic {
            CallTarget::TraitMethod {
                instance,
                parameters,
            }
        } else {
            CallTarget::Direct {
                instance,
                parameters,
            }
        };
        let dest = dest.unwrap_or_else(|| self.new_temp(result_ty, span));
        self.assign_rvalue(
            dest,
            Rvalue::Call {
                target,
                args,
                arg_spans,
            },
            span,
        );
    }

    fn lower_method(
        &mut self,
        expr: HirExprId,
        receiver: HirExprId,
        name_span: Span,
        args: &[HirExprId],
        dest: Option<Place>,
        span: Span,
    ) {
        let resolved = self.typeck.method(expr).cloned().unwrap_or_else(|| {
            panic!("method call at {name_span:?} was not resolved; this is a compiler bug")
        });

        // A user method and a statically resolved trait method are ordinary
        // functions whose first parameter is the receiver, so both lower to a
        // direct call. A bound method is symbolic until monomorphization selects
        // the implementation; a method on a trait object lowers to
        // `Rvalue::DynamicCall` below. The receiver adjustment is explicit in
        // the argument expression, and the borrow checker sees the same
        // `Rvalue::Ref` it would see for a written borrow.
        let (function, symbolic) = match resolved.kind {
            MethodKind::Intrinsic(method) => {
                return self.lower_intrinsic_call(IntrinsicCall {
                    receiver,
                    name_span,
                    args,
                    dest,
                    span,
                    method,
                    resolved: &resolved,
                });
            }
            MethodKind::User { function, .. } | MethodKind::Trait { function, .. } => {
                (function, false)
            }
            MethodKind::TraitBound { method, .. } => (method, true),
            MethodKind::Dynamic { .. } => {
                return self.lower_dynamic_call(receiver, args, dest, span, &resolved);
            }
        };
        let receiver_argument = match resolved.receiver_adjust {
            ReceiverAdjust::None => self.lower_operand(receiver),
            ReceiverAdjust::SharedBorrow | ReceiverAdjust::ExclusiveBorrow => {
                self.lower_borrowed_receiver(receiver, &resolved)
            }
            ReceiverAdjust::SharedReborrow => self.lower_shared_reborrow(receiver, &resolved),
        };
        let mut call_args = Vec::with_capacity(args.len() + 1);
        call_args.push(receiver_argument);
        call_args.extend(args.iter().map(|arg| self.lower_operand(*arg)));
        let mut arg_spans = Vec::with_capacity(args.len() + 1);
        arg_spans.push(self.hir.expr(receiver).span);
        arg_spans.extend(args.iter().map(|arg| self.hir.expr(*arg).span));

        // A method call always runs, even when its value is discarded. A
        // modifying method has its whole effect in the call, so dropping it
        // because the statement has no value would remove the mutation.
        let instance = self.call_instance(expr, function);
        self.lower_resolved_call(
            &resolved, instance, symbolic, call_args, arg_spans, dest, span,
        );
    }

    /// Emits a resolved function or method call.
    ///
    /// `symbolic` distinguishes a trait method reached through a bound from a
    /// concrete impl. Both are ordinary calls, but monomorphization resolves the
    /// former later.
    #[allow(clippy::too_many_arguments)]
    fn lower_resolved_call(
        &mut self,
        resolved: &ResolvedMethod,
        instance: Instance,
        symbolic: bool,
        args: Vec<Operand>,
        arg_spans: Vec<Span>,
        dest: Option<Place>,
        span: Span,
    ) {
        let parameters = resolved.arg_tys.len() + 1;
        let target = if symbolic {
            CallTarget::TraitMethod {
                instance,
                parameters,
            }
        } else {
            CallTarget::Direct {
                instance,
                parameters,
            }
        };
        let dest = dest.unwrap_or_else(|| self.new_temp(resolved.ret_ty, span));
        self.assign_rvalue(
            dest,
            Rvalue::Call {
                target,
                args,
                arg_spans,
            },
            span,
        );
    }

    /// Emits a call recorded by a built-in protocol.
    ///
    /// Protocol calls are ordinary calls or dynamic calls. The receiver is
    /// already lowered because protocol callers frequently start from a
    /// storage place rather than an HIR method-call expression.
    #[allow(clippy::too_many_arguments)]
    fn lower_protocol_call(
        &mut self,
        call: &ProtocolCall,
        receiver: Operand,
        args: Vec<Operand>,
        arg_spans: Vec<Span>,
        dest: Option<Place>,
        span: Span,
    ) {
        match call.method.kind {
            MethodKind::Dynamic {
                object_trait_ref,
                method,
                slot,
                ..
            } => self.lower_dynamic_resolved_call(
                object_trait_ref,
                method,
                slot,
                &call.method,
                receiver,
                args,
                arg_spans,
                dest,
                span,
            ),
            MethodKind::Intrinsic(_) => {
                panic!("a protocol call cannot resolve to an intrinsic; this is a compiler bug")
            }
            MethodKind::User { .. } | MethodKind::Trait { .. } | MethodKind::TraitBound { .. } => {
                let instance = call.lowering_instance.unwrap_or_else(|| {
                    panic!(
                        "protocol call at {span:?} has no lowering instance; this is a compiler bug"
                    )
                });
                let symbolic = matches!(call.method.kind, MethodKind::TraitBound { .. });
                let mut call_args = Vec::with_capacity(args.len() + 1);
                call_args.push(receiver);
                call_args.extend(args);
                self.lower_resolved_call(
                    &call.method,
                    instance,
                    symbolic,
                    call_args,
                    arg_spans,
                    dest,
                    span,
                );
            }
        }
    }

    /// Lowers a method call dispatched through a trait-object vtable.
    fn lower_dynamic_call(
        &mut self,
        receiver: HirExprId,
        args: &[HirExprId],
        dest: Option<Place>,
        span: Span,
        resolved: &ResolvedMethod,
    ) {
        let MethodKind::Dynamic {
            object_trait_ref,
            method,
            slot,
            ..
        } = resolved.kind
        else {
            panic!("a dynamic call must resolve to a vtable slot; this is a compiler bug")
        };
        let receiver_value = self.lower_dynamic_receiver(receiver, resolved);
        let mut arg_spans = Vec::with_capacity(args.len() + 1);
        arg_spans.push(self.hir.expr(receiver).span);
        arg_spans.extend(args.iter().map(|arg| self.hir.expr(*arg).span));
        let args: Vec<Operand> = args.iter().map(|arg| self.lower_operand(*arg)).collect();
        self.lower_dynamic_resolved_call(
            object_trait_ref,
            method,
            slot,
            resolved,
            receiver_value,
            args,
            arg_spans,
            dest,
            span,
        );
    }

    /// Emits a dynamic call whose receiver has already been lowered.
    #[allow(clippy::too_many_arguments)]
    fn lower_dynamic_resolved_call(
        &mut self,
        object_trait_ref: seal_ty::TraitRef,
        method: DefId,
        slot: seal_layout::VtableSlot,
        resolved: &ResolvedMethod,
        receiver: Operand,
        args: Vec<Operand>,
        arg_spans: Vec<Span>,
        dest: Option<Place>,
        span: Span,
    ) {
        let dest = dest.unwrap_or_else(|| self.new_temp(resolved.ret_ty, span));
        self.assign_rvalue(
            dest,
            Rvalue::DynamicCall {
                object_trait_ref,
                method,
                slot,
                receiver,
                args,
                arg_spans,
                arg_tys: resolved.arg_tys.clone(),
                ret_ty: resolved.ret_ty,
            },
            span,
        );
    }

    /// Produces the receiver value for a dynamic call.
    fn lower_dynamic_receiver(
        &mut self,
        receiver: HirExprId,
        resolved: &ResolvedMethod,
    ) -> Operand {
        let span = self.hir.expr(receiver).span;
        match resolved.receiver_adjust {
            ReceiverAdjust::None => self.lower_operand(receiver),
            ReceiverAdjust::SharedReborrow => {
                let source_ty = self.expr_ty(receiver);
                let (pointer_place, source) =
                    self.lower_pointer_place_and_operand(receiver, source_ty, span);
                let place = pointer_place.deref();
                let target = self
                    .hir
                    .ty_ctxt()
                    .ref_ty_with(RefKind::Shared, resolved.receiver_ty);
                self.materialize_reborrow(source, place, source_ty, target, span)
            }
            ReceiverAdjust::SharedBorrow | ReceiverAdjust::ExclusiveBorrow => {
                let ExprKind::Deref { operand: pointer } = self.hir.expr(receiver).kind.clone()
                else {
                    panic!(
                        "a dyn receiver adjustment requires an explicit dereference; this is a \
                         compiler bug"
                    )
                };
                let source_ty = self.expr_ty(pointer);
                let (pointer_place, source) =
                    self.lower_pointer_place_and_operand(pointer, source_ty, span);
                let place = pointer_place.deref();
                let kind = if resolved.receiver_adjust == ReceiverAdjust::ExclusiveBorrow {
                    RefKind::Exclusive
                } else {
                    RefKind::Shared
                };
                let target = self.hir.ty_ctxt().ref_ty_with(kind, resolved.receiver_ty);
                self.materialize_reborrow(source, place, source_ty, target, span)
            }
        }
    }

    /// Materializes one reborrow into a temporary and returns a copy of it.
    fn materialize_reborrow(
        &mut self,
        source: Operand,
        place: Place,
        from: TyId,
        to: TyId,
        span: Span,
    ) -> Operand {
        let temporary = self.new_temp(to, span);
        self.assign_rvalue(
            temporary.clone(),
            Rvalue::TraitObjectReborrow {
                source,
                place,
                from,
                to,
            },
            span,
        );
        Operand::Copy(temporary)
    }

    /// Lowers one intrinsic method call.
    fn lower_intrinsic_call(&mut self, call: IntrinsicCall<'_>) {
        let IntrinsicCall {
            receiver,
            name_span,
            args,
            dest,
            span,
            method,
            resolved,
        } = call;
        let receiver = match resolved.receiver_form {
            seal_typeck::ReceiverForm::Value => {
                IntrinsicReceiver::Value(self.lower_operand(receiver))
            }
            seal_typeck::ReceiverForm::Place => {
                let mut place = if let Some(place) = self.typeck.place(receiver) {
                    let place = place.clone();
                    self.resolved_place(&place)
                } else {
                    let ty = self.expr_ty(receiver);
                    let temp = self.new_temp(ty, name_span);
                    self.lower_expr_into(receiver, Some(temp.clone()));
                    temp
                };
                for _ in 0..resolved.receiver_derefs {
                    place = place.deref();
                }
                IntrinsicReceiver::Place(place)
            }
        };
        let args: Vec<Operand> = args.iter().map(|arg| self.lower_operand(*arg)).collect();
        let dest = dest.unwrap_or_else(|| self.new_temp(resolved.ret_ty, span));
        self.assign_rvalue(
            dest,
            Rvalue::Intrinsic {
                method,
                receiver,
                args,
                span,
            },
            span,
        );
    }

    /// Returns the complete instantiation type checking recorded for `expr`.
    ///
    /// Every call and method call records one; a missing instance would make
    /// monomorphization use the wrong body, so the failure is an internal bug.
    fn call_instance(&self, expr: HirExprId, function: DefId) -> Instance {
        self.typeck.instantiation(expr).unwrap_or_else(|| {
            panic!("call to {function:?} has no recorded instantiation; this is a compiler bug")
        })
    }

    /// Returns the number of parameters a trait method declaration takes.
    ///
    /// The trait method has no type-checked signature of its own, so the count
    /// comes from the declaration in the trait.
    fn trait_method_parameters(&self, function: DefId) -> usize {
        let Some(trait_def) = self.hir.definitions().get(function).parent else {
            return 0;
        };
        let Some(definition) = self.hir.trait_definition(trait_def) else {
            return 0;
        };
        definition
            .methods
            .iter()
            .find(|method| method.def == function)
            .map_or(0, |method| method.signature.params.len())
    }

    /// Returns the type arguments of the aggregate literal `expr`.
    fn instantiation_args(&self, expr: HirExprId) -> SubstsId {
        match self.hir.ty_ctxt().kind(self.expr_ty(expr)) {
            TyKind::Adt { args, .. } => args,
            _ => self.hir.ty_ctxt().substs(&[]),
        }
    }

    /// Materializes the automatic borrow of a user method receiver.
    fn lower_borrowed_receiver(
        &mut self,
        receiver: HirExprId,
        resolved: &seal_typeck::ResolvedMethod,
    ) -> Operand {
        let place = if let Some(place) = self.typeck.place(receiver) {
            let place = place.clone();
            self.resolved_place(&place)
        } else {
            let ty = self.expr_ty(receiver);
            let temp = self.new_temp(ty, self.hir.expr(receiver).span);
            self.lower_expr_into(receiver, Some(temp.clone()));
            temp
        };
        let kind = match resolved.receiver_adjust {
            ReceiverAdjust::ExclusiveBorrow => seal_ty::RefKind::Exclusive,
            ReceiverAdjust::SharedBorrow => seal_ty::RefKind::Shared,
            ReceiverAdjust::None | ReceiverAdjust::SharedReborrow => {
                unreachable!("only borrowed receivers need materialization")
            }
        };
        let reference_ty = self.hir.ty_ctxt().ref_ty_with(kind, resolved.receiver_ty);
        let reference = self.new_temp(reference_ty, self.hir.expr(receiver).span);
        self.assign_rvalue(
            reference.clone(),
            Rvalue::Ref { place, kind },
            self.hir.expr(receiver).span,
        );
        Operand::Copy(reference)
    }

    /// Reborrows a `*mut T` receiver as a `*T` for a read-only method.
    ///
    /// The receiver expression holds the exclusive reference, so the new shared
    /// loan covers the pointee, not the pointer variable: the place is
    /// dereferenced before the borrow, exactly as `&*reference` would be. The
    /// borrow checker then orders this loan against every other loan of the
    /// pointee and allows the method to read it.
    fn lower_shared_reborrow(
        &mut self,
        receiver: HirExprId,
        resolved: &seal_typeck::ResolvedMethod,
    ) -> Operand {
        let reference = if let Some(place) = self.typeck.place(receiver) {
            let place = place.clone();
            self.resolved_place(&place)
        } else {
            let ty = self.expr_ty(receiver);
            let temp = self.new_temp(ty, self.hir.expr(receiver).span);
            self.lower_expr_into(receiver, Some(temp.clone()));
            temp
        };
        let pointee = reference.deref();
        let shared_ty = self
            .hir
            .ty_ctxt()
            .ref_ty_with(seal_ty::RefKind::Shared, resolved.receiver_ty);
        let span = self.hir.expr(receiver).span;
        let shared = self.new_temp(shared_ty, span);
        self.assign_rvalue(
            shared.clone(),
            Rvalue::Ref {
                place: pointee,
                kind: seal_ty::RefKind::Shared,
            },
            span,
        );
        Operand::Copy(shared)
    }

    /// Lowers a custom index expression whose protocol was resolved by type
    /// checking.
    fn lower_custom_index_expr(&mut self, expr: HirExprId) -> Place {
        let span = self.hir.expr(expr).span;
        let ExprKind::Index { base, .. } = self.hir.expr(expr).kind.clone() else {
            panic!("a recorded custom index is an index expression; this is a compiler bug")
        };
        let base_place = self.place_of_expr(base);
        let base_ty = self.expr_ty(base);
        self.lower_custom_index(expr, base_place, base_ty, span).0
    }

    /// Lowers the protocol call that selects a custom indexed element.
    ///
    /// The index operand is evaluated first and exactly once, before the
    /// receiver is borrowed, so a call or mutation in the index does not get
    /// reordered by the receiver adjustment.
    fn lower_custom_index(
        &mut self,
        expr: HirExprId,
        base_place: Place,
        base_ty: TyId,
        span: Span,
    ) -> (Place, TyId) {
        let protocol = self
            .typeck
            .index_protocol(expr)
            .cloned()
            .unwrap_or_else(|| {
                panic!("custom index at {span:?} has no recorded protocol; this is a compiler bug")
            });
        let IndexProtocol::Custom {
            element_ty, call, ..
        } = protocol
        else {
            panic!("a custom index must use the custom protocol; this is a compiler bug")
        };
        let ExprKind::Index { base, index, .. } = self.hir.expr(expr).kind.clone() else {
            panic!("a recorded custom index is an index expression; this is a compiler bug")
        };
        let index_value = self.lower_operand(index);
        let receiver = self.lower_place_receiver(base_place, base_ty, &call.method, span);
        let arg_spans = vec![self.hir.expr(base).span, self.hir.expr(index).span];
        let destination = self.new_temp(call.method.ret_ty, span);
        self.lower_protocol_call(
            &call,
            receiver,
            vec![index_value],
            arg_spans,
            Some(destination.clone()),
            span,
        );

        (destination.deref(), element_ty)
    }

    /// Lowers the receiver of a call whose base storage is already known.
    fn lower_place_receiver(
        &mut self,
        place: Place,
        place_ty: TyId,
        resolved: &ResolvedMethod,
        span: Span,
    ) -> Operand {
        match resolved.receiver_adjust {
            ReceiverAdjust::None => self.place_operand(place, place_ty),
            ReceiverAdjust::SharedBorrow | ReceiverAdjust::ExclusiveBorrow => {
                let kind = if resolved.receiver_adjust == ReceiverAdjust::ExclusiveBorrow {
                    RefKind::Exclusive
                } else {
                    RefKind::Shared
                };
                let reference_ty = self.hir.ty_ctxt().ref_ty_with(kind, resolved.receiver_ty);
                let reference = self.new_temp(reference_ty, span);
                self.assign_rvalue(reference.clone(), Rvalue::Ref { place, kind }, span);
                Operand::Copy(reference)
            }
            ReceiverAdjust::SharedReborrow => {
                let pointee = place.deref();
                let shared_ty = self
                    .hir
                    .ty_ctxt()
                    .ref_ty_with(RefKind::Shared, resolved.receiver_ty);
                let shared = self.new_temp(shared_ty, span);
                self.assign_rvalue(
                    shared.clone(),
                    Rvalue::Ref {
                        place: pointee,
                        kind: RefKind::Shared,
                    },
                    span,
                );
                Operand::Copy(shared)
            }
        }
    }

    /// Lowers an operand, reading it without moving when `read` is set.
    ///
    /// A move transfers ownership, which equality must not do. Converting the
    /// move into a `Copy` of the same place keeps the operand's owner
    /// responsible for its drop while still yielding the value.
    fn lower_operand_for(&mut self, expr: HirExprId, read: bool) -> Operand {
        let operand = self.lower_operand(expr);
        match operand {
            Operand::Move(place) if read => Operand::Copy(place),
            operand => operand,
        }
    }

    fn lower_operand(&mut self, expr: HirExprId) -> Operand {
        let (span, kind) = {
            let expr = self.hir.expr(expr);
            (expr.span, expr.kind.clone())
        };
        let ty = self.expr_ty(expr);

        if self.typeck.coercion(expr).is_some() {
            let temporary = self.new_temp(ty, span);
            self.lower_expr_into(expr, Some(temporary.clone()));
            return self.place_operand(temporary, ty);
        }

        match kind {
            ExprKind::Lit(literal) => Operand::Constant(self.constant(literal, ty)),
            ExprKind::Path {
                res: Res::Local(local),
                ..
            } => {
                let place = self.place_of_local(local);
                self.place_operand(place, ty)
            }
            ExprKind::Path {
                res: Res::Function(_) | Res::TraitFunction { .. },
                ..
            }
            | ExprKind::Error => Operand::Constant(Constant::Unit),
            ExprKind::Deref { .. } | ExprKind::Field { .. } | ExprKind::Index { .. } => {
                let place = self.place_of_expr(expr);
                self.place_operand(place, ty)
            }
            ExprKind::Slice { .. } => Operand::Constant(Constant::Unit),
            _ => {
                let temp = self.new_temp(ty, span);
                self.lower_expr_into(expr, Some(temp.clone()));
                self.place_operand(temp, ty)
            }
        }
    }

    fn constant(&self, literal: Literal, ty: TyId) -> Constant {
        match literal {
            Literal::Int { value, .. } => Constant::Int { value, ty },
            Literal::Float { value, .. } => Constant::Float { value, ty },
            Literal::Str(symbol) => Constant::Str { value: symbol, ty },
            Literal::Char(value) => Constant::Char(value),
            Literal::Bool(value) => Constant::Bool(value),
        }
    }

    fn constant_one(&self, ty: TyId) -> Constant {
        match self.hir.ty_ctxt().kind(ty) {
            TyKind::Int(_) | TyKind::Uint(_) => Constant::Int { value: 1, ty },
            TyKind::Float(_) => Constant::Float { value: 1.0, ty },
            other => panic!("`{other:?}` is not a numeric type; type checking should reject it"),
        }
    }

    fn place_operand(&self, place: Place, ty: TyId) -> Operand {
        // Reading through a shared reference never transfers ownership: the
        // pointee outlives the read and is still owned by whoever lent it. A
        // raw dereference is the unsafe layer's move: the programmer owns the
        // state of that storage.
        if (place.is_deref() && !place.is_raw_deref())
            || self.hir.ty_ctxt().is_copy_with(ty, &self.param_env)
        {
            Operand::Copy(place)
        } else {
            Operand::Move(place)
        }
    }

    /// Lowers the storage denoted by `expr`.
    ///
    /// Type checking records a place for every projection rooted at a local. A
    /// dereference of a reference *value* — `*identity(&v)` or `*&x` — has no
    /// local to start from, because the reference itself is a temporary.
    /// Lowering stores that reference in a temporary and reads through it, so
    /// the expression still denotes storage and still copies on read.
    ///
    /// Any other value expression that type checking accepted as a slice base,
    /// such as the array literal in `&[1, 2][0..1]`, is materialized the same
    /// way. The temporary owns the value for the rest of the frame.
    fn place_of_expr(&mut self, expr: HirExprId) -> Place {
        if matches!(
            self.typeck.index_protocol(expr),
            Some(IndexProtocol::Custom { .. })
        ) {
            return self.lower_custom_index_expr(expr);
        }
        if let Some(place) = self.typeck.place(expr).cloned() {
            return self.resolved_place(&place);
        }

        let kind = self.hir.expr(expr).kind.clone();
        let span = self.hir.expr(expr).span;
        if let ExprKind::Deref { operand } = kind {
            let reference_ty = self.expr_ty(operand);
            let raw = matches!(self.hir.ty_ctxt().kind(reference_ty), TyKind::RawPtr { .. });
            let reference = self.new_temp(reference_ty, span);
            self.lower_expr_into(operand, Some(reference.clone()));
            return if raw {
                reference.raw_deref()
            } else {
                reference.deref()
            };
        }

        let ty = self.expr_ty(expr);
        let temporary = self.new_temp(ty, span);
        self.lower_expr_into(expr, Some(temporary.clone()));
        temporary
    }

    /// Converts a place resolved by type checking into a MIR place.
    ///
    /// Index expressions are evaluated here, in source order, and a runtime
    /// index receives a bounds check before any projection uses it. The check
    /// becomes an explicit two-block graph, so code generation never has to
    /// invent control flow while computing an address.
    fn resolved_place(&mut self, place: &ResolvedPlace) -> Place {
        let root = match place.root {
            seal_typeck::PlaceRoot::Local(def) => {
                PlaceRoot::Local(self.place_of_local(def).expect_local())
            }
            seal_typeck::PlaceRoot::Static(def) => PlaceRoot::Static(def),
            seal_typeck::PlaceRoot::Const(def) => {
                // A constant has no storage, so a projected or borrowed const
                // path starts from a temporary that holds its value.
                let span = self.hir.definitions().get(def).span;
                let temp = self.new_temp(place.root_ty, span);
                self.assign_operand(temp.clone(), Operand::Const(def), span);
                temp.root
            }
        };
        let mut root = root;
        let mut projections = Vec::with_capacity(place.projections.len());
        let mut ty = place.root_ty;
        let mut slice_reference: Option<Place> = None;
        for projection in &place.projections {
            match projection {
                PlaceProjection::CustomIndex { expr, .. } => {
                    let base = Place::with_root(root, projections.clone());
                    let span = self.hir.expr(*expr).span;
                    let (custom, element_ty) = self.lower_custom_index(*expr, base, ty, span);
                    root = custom.root;
                    projections = custom.projections;
                    ty = element_ty;
                    slice_reference = None;
                }
                PlaceProjection::Deref | PlaceProjection::RawDeref => {
                    let pointee = match self.hir.ty_ctxt().kind(ty) {
                        TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. } => pointee,
                        _ => ty,
                    };
                    if matches!(self.hir.ty_ctxt().kind(pointee), TyKind::Slice { .. }) {
                        slice_reference = Some(Place::with_root(root, projections.clone()));
                    } else {
                        slice_reference = None;
                    }
                    projections.push(match projection {
                        PlaceProjection::RawDeref => Projection::RawDeref,
                        _ => Projection::Deref,
                    });
                    ty = pointee;
                }
                PlaceProjection::Field(field) => {
                    if let TyKind::Adt { adt, args } = self.hir.ty_ctxt().kind(ty)
                        && let Some(field_ty) = self.hir.ty_ctxt().field_ty(adt, args, *field)
                    {
                        ty = field_ty;
                    }
                    slice_reference = None;
                    projections.push(Projection::Field(*field));
                }
                PlaceProjection::TupleField(index) => {
                    if let TyKind::Tuple(tuple) = self.hir.ty_ctxt().kind(ty)
                        && let Some(element) =
                            self.hir.ty_ctxt().tuple_element(tuple, *index as usize)
                    {
                        ty = element;
                    }
                    slice_reference = None;
                    projections.push(Projection::TupleField(*index));
                }
                PlaceProjection::Index {
                    index,
                    constant,
                    length,
                    dynamic_length,
                } => {
                    let span = self.hir.expr(*index).span;
                    let index_ty = self.expr_ty(*index);
                    let temp = self.new_temp(index_ty, span);
                    self.lower_expr_into(*index, Some(temp.clone()));
                    if constant.is_none() {
                        let length_operand = if *dynamic_length {
                            let reference = slice_reference.clone().unwrap_or_else(|| {
                                panic!(
                                    "a slice index has no slice reference; this is a compiler bug"
                                )
                            });
                            let len_ty = self.usize_ty();
                            let len = self.new_temp(len_ty, span);
                            self.assign_rvalue(
                                len.clone(),
                                Rvalue::Len {
                                    slice: Operand::Copy(reference),
                                },
                                span,
                            );
                            Operand::Copy(len)
                        } else {
                            Operand::Constant(Constant::Int {
                                value: u128::from(*length),
                                ty: self.usize_ty(),
                            })
                        };
                        let message = if *dynamic_length {
                            "index out of bounds".to_owned()
                        } else {
                            format!("index out of bounds: the length is {length}")
                        };
                        self.emit_bounds_check_operand(
                            Operand::Copy(temp.clone()),
                            length_operand,
                            span,
                            &message,
                        );
                    }
                    ty = match self.hir.ty_ctxt().kind(ty) {
                        TyKind::Array { element, .. } | TyKind::Slice { element } => element,
                        _ => ty,
                    };
                    slice_reference = None;
                    projections.push(Projection::Index {
                        local: temp.expect_local(),
                        constant: *constant,
                    });
                }
            }
        }
        Place::with_root(root, projections)
    }

    /// Lowers `&base[start..end]` into a fat-pointer construction.
    fn lower_slice_ref(
        &mut self,
        base: HirExprId,
        bounds: SliceBounds,
        kind: seal_ty::RefKind,
        dest: Option<Place>,
        span: Span,
    ) {
        let SliceBounds { start, end } = bounds;
        let ty = self.expr_ty(base);
        let ty = match self.hir.ty_ctxt().kind(ty) {
            TyKind::Array { element, .. } => {
                let slice = self.hir.ty_ctxt().slice_ty(element);
                self.hir.ty_ctxt().ref_ty_with(kind, slice)
            }
            TyKind::Ref { pointee, .. } => self.hir.ty_ctxt().ref_ty_with(kind, pointee),
            _ => ty,
        };
        let base_place = self.place_of_expr(base);
        if let Some(dest) = &dest {
            self.promote_borrowed_temp(&base_place, dest);
        }
        let base_ty = self.expr_ty(base);
        let length = match self.hir.ty_ctxt().kind(base_ty) {
            TyKind::Array { length, .. } => self.constant_length(length),
            TyKind::Ref { pointee, .. } => match self.hir.ty_ctxt().kind(pointee) {
                TyKind::Array { length, .. } => self.constant_length(length),
                TyKind::Slice { .. } => {
                    let length_ty = self.usize_ty();
                    let length = self.new_temp(length_ty, span);
                    self.assign_rvalue(
                        length.clone(),
                        Rvalue::Len {
                            slice: Operand::Copy(base_place.clone()),
                        },
                        span,
                    );
                    Operand::Copy(length)
                }
                _ => self.constant_length(0),
            },
            _ => self.constant_length(0),
        };
        let start_operand = start.map(|start| self.lower_operand(start));
        let end_operand = end.map(|end| self.lower_operand(end));

        if let Some(start) = &start_operand {
            self.emit_le_check(
                start.clone(),
                end_operand.clone().unwrap_or_else(|| length.clone()),
                span,
                "slice range starts after it ends",
            );
        }
        if let Some(end) = &end_operand {
            self.emit_le_check(
                end.clone(),
                length,
                span,
                "slice range end is out of bounds",
            );
        }

        if let Some(dest) = dest {
            self.assign_rvalue(
                dest,
                Rvalue::SliceRef {
                    base: base_place,
                    start: start_operand,
                    end: end_operand,
                    kind,
                    ty,
                },
                span,
            );
        }
    }

    fn constant_length(&self, length: u64) -> Operand {
        Operand::Constant(Constant::Int {
            value: u128::from(length),
            ty: self.usize_ty(),
        })
    }

    fn usize_ty(&self) -> TyId {
        self.hir
            .ty_ctxt()
            .lookup(TyKind::Uint(seal_ty::UintTy::Usize))
            .expect("HIR lowering interns every primitive type")
    }

    fn emit_bounds_check_operand(
        &mut self,
        index: Operand,
        length: Operand,
        span: Span,
        message: &str,
    ) {
        self.emit_compare_check(BinOp::Lt, index, length, span, message);
    }

    fn emit_le_check(&mut self, lhs: Operand, rhs: Operand, span: Span, message: &str) {
        self.emit_compare_check(BinOp::Le, lhs, rhs, span, message);
    }

    fn emit_compare_check(
        &mut self,
        op: BinOp,
        lhs: Operand,
        rhs: Operand,
        span: Span,
        message: &str,
    ) {
        let bool_ty = self
            .hir
            .ty_ctxt()
            .lookup(TyKind::Bool)
            .expect("HIR lowering interns every primitive type");
        let condition = self.new_temp(bool_ty, span);
        self.assign_rvalue(
            condition.clone(),
            Rvalue::Binary { op, lhs, rhs, span },
            span,
        );

        let success = self.new_block();
        let panic = self.new_block();
        self.terminate(
            TerminatorKind::SwitchInt {
                discriminant: Operand::Copy(condition),
                targets: vec![SwitchTarget {
                    value: 1,
                    target: success,
                }],
                otherwise: panic,
            },
            span,
        );
        self.current = Some(panic);
        self.terminate(
            TerminatorKind::Panic {
                message: message.to_owned(),
            },
            span,
        );
        self.current = Some(success);
    }

    fn place_of_local(&self, local: DefId) -> Place {
        let local = self.local_of.get(&local).unwrap_or_else(|| {
            panic!("local definition {local:?} has no MIR local; this is a compiler bug")
        });
        Place::local(*local)
    }

    fn expr_ty(&self, expr: HirExprId) -> TyId {
        self.typeck
            .expr_ty(expr)
            .unwrap_or_else(|| panic!("expression {expr:?} has no type; this is a compiler bug"))
    }

    fn expr_or_local_ty(&self, local: DefId) -> TyId {
        self.typeck
            .local_ty(local)
            .unwrap_or_else(|| panic!("local {local:?} has no type; this is a compiler bug"))
    }
}

#[derive(Clone, Copy)]
enum LogicalOp {
    And,
    Or,
}

fn logical_op(op: BinaryOp) -> LogicalOp {
    match op {
        BinaryOp::And => LogicalOp::And,
        BinaryOp::Or => LogicalOp::Or,
        other => panic!("`{other:?}` is not a short-circuiting operator"),
    }
}

fn un_op(op: UnaryOp) -> UnOp {
    match op {
        UnaryOp::Neg => UnOp::Neg,
        UnaryOp::Not => UnOp::Not,
        UnaryOp::BitNot => UnOp::BitNot,
    }
}

fn bin_op(op: BinaryOp) -> BinOp {
    match op {
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::Div => BinOp::Div,
        BinaryOp::Rem => BinOp::Rem,
        BinaryOp::Eq => BinOp::Eq,
        BinaryOp::Ne => BinOp::Ne,
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::Le => BinOp::Le,
        BinaryOp::Ge => BinOp::Ge,
        BinaryOp::BitAnd => BinOp::BitAnd,
        BinaryOp::BitOr => BinOp::BitOr,
        BinaryOp::BitXor => BinOp::BitXor,
        BinaryOp::Shl => BinOp::Shl,
        BinaryOp::Shr => BinOp::Shr,
        BinaryOp::And | BinaryOp::Or => {
            panic!("logical operators are lowered to control flow, not to a binary rvalue")
        }
    }
}

fn compound_bin_op(op: AssignOp) -> BinOp {
    match op {
        AssignOp::AddAssign => BinOp::Add,
        AssignOp::SubAssign => BinOp::Sub,
        AssignOp::MulAssign => BinOp::Mul,
        AssignOp::DivAssign => BinOp::Div,
        AssignOp::RemAssign => BinOp::Rem,
        AssignOp::ShlAssign => BinOp::Shl,
        AssignOp::ShrAssign => BinOp::Shr,
        AssignOp::BitAndAssign => BinOp::BitAnd,
        AssignOp::BitOrAssign => BinOp::BitOr,
        AssignOp::BitXorAssign => BinOp::BitXor,
        AssignOp::Assign => {
            panic!("plain assignment is not a binary operator")
        }
    }
}
