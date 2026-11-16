//! HIR-to-MIR lowering.

use seal_ast::{AssignOp, BinaryOp, Literal, UnaryOp, UpdateOp};
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::{Body as HirBody, ExprKind, Hir, HirBlockId, HirExprId, HirStmtId, Res, StmtKind};
use seal_index::IndexVec;
use seal_span::Span;
use seal_ty::{TyId, TyKind};
use seal_typeck::TypeckResults;

use crate::body::{BasicBlock, Body, LocalDecl, LocalKind};
use crate::id::{BasicBlockId, LocalId};
use crate::mir::Mir;
use crate::operand::{Constant, Operand};
use crate::place::{Place, Projection};
use crate::rvalue::{AggregateKind, BinOp, Rvalue, UnOp};
use crate::statement::Statement;
use crate::terminator::{SwitchTarget, Terminator, TerminatorKind};

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
}

struct BodyLowering<'a> {
    hir: &'a Hir,
    typeck: &'a TypeckResults,
    owner: DefId,
    span: Span,
    locals: IndexVec<LocalId, LocalDecl>,
    local_of: FxHashMap<DefId, LocalId>,
    blocks: IndexVec<BasicBlockId, BlockBuilder>,
    current: Option<BasicBlockId>,
    entry: BasicBlockId,
    loops: Vec<LoopTargets>,
    arg_count: usize,
}

impl<'a> BodyLowering<'a> {
    fn new(hir: &'a Hir, typeck: &'a TypeckResults, body: &HirBody) -> Self {
        let span = hir.definitions().get(body.owner).span;
        let signature = typeck
            .signature(body.owner)
            .expect("type checking gives every body a signature");
        let ret = signature.ret;
        let param_types = signature.params.clone();

        let placeholder = BasicBlockId::from_u32(0).expect("zero is a valid block index");
        let mut lowering = Self {
            hir,
            typeck,
            owner: body.owner,
            span,
            locals: IndexVec::new(),
            local_of: FxHashMap::default(),
            blocks: IndexVec::new(),
            current: None,
            entry: placeholder,
            loops: Vec::new(),
            arg_count: body.params.len(),
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

        for (param, ty) in body.params.iter().zip(&param_types) {
            let definition = hir.definitions().get(param.def);
            let local = lowering.push_local(LocalDecl {
                ty: *ty,
                name: Some(definition.name),
                kind: LocalKind::Arg,
                span: definition.span,
            });
            lowering.local_of.insert(param.def, local);
        }

        lowering
    }

    fn run(mut self, body: &HirBody) -> Body {
        self.lower_block(body.root, Some(self.return_place()));
        if self.current.is_some() {
            let span = self.span;
            self.terminate(TerminatorKind::Return, span);
        }
        self.finish()
    }

    fn finish(self) -> Body {
        let Self {
            owner,
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
        let statement_count = self.hir.block(block).statements.len();
        for index in 0..statement_count {
            let statement = self.hir.block(block).statements[index];
            self.lower_stmt(statement);
        }

        if let Some(tail) = self.hir.block(block).tail {
            self.lower_expr_into(tail, dest);
        } else if let Some(dest) = dest
            && self.current.is_some()
        {
            let span = self.hir.block(block).span;
            self.assign_constant(dest, Constant::Unit, span);
        }
    }

    fn lower_stmt(&mut self, statement: HirStmtId) {
        let (span, kind) = {
            let statement = self.hir.stmt(statement);
            (statement.span, statement.kind.clone())
        };

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
            }
            StmtKind::Return(value) => {
                let dest = self.return_place();
                match value {
                    Some(value) => self.lower_expr_into(value, Some(dest)),
                    None => self.assign_constant(dest, Constant::Unit, span),
                }
                self.terminate(TerminatorKind::Return, span);
            }
            StmtKind::While { condition, body } => self.lower_while(condition, body, span),
            StmtKind::Break => {
                let target = self
                    .loops
                    .last()
                    .expect("type checking rejects `break` outside a loop")
                    .break_to;
                self.terminate_goto(target, span);
            }
            StmtKind::Continue => {
                let target = self
                    .loops
                    .last()
                    .expect("type checking rejects `continue` outside a loop")
                    .continue_to;
                self.terminate_goto(target, span);
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
        self.loops.push(LoopTargets {
            break_to: exit_block,
            continue_to: condition_block,
        });
        self.lower_block(body, None);
        self.loops.pop();
        if self.current.is_some() {
            self.terminate_goto(condition_block, span);
        }

        self.current = Some(exit_block);
    }

    fn lower_expr_into(&mut self, expr: HirExprId, dest: Option<Place>) {
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
            ExprKind::Path(Res::Local(local)) => {
                if let Some(dest) = dest {
                    let place = self.place_of_local(local);
                    let ty = self.expr_ty(expr);
                    let operand = self.place_operand(place, ty);
                    self.assign_operand(dest, operand, span);
                }
            }
            ExprKind::Path(Res::Function(_)) | ExprKind::Error => {
                if let Some(dest) = dest {
                    self.assign_constant(dest, Constant::Unit, span);
                }
            }
            ExprKind::Borrow { operand } => {
                if let Some(dest) = dest {
                    let place = self.place_of_expr(operand);
                    self.assign_rvalue(dest, Rvalue::Ref(place), span);
                }
            }
            ExprKind::Deref { .. } | ExprKind::Field { .. } => {
                if let Some(dest) = dest {
                    let place = self.place_of_expr(expr);
                    let ty = self.expr_ty(expr);
                    let operand = self.place_operand(place, ty);
                    self.assign_operand(dest, operand, span);
                }
            }
            ExprKind::Struct { adt, fields } => {
                if let Some(dest) = dest {
                    let fields = fields
                        .iter()
                        .map(|(field, value)| (*field, self.lower_operand(*value)))
                        .collect();
                    self.assign_rvalue(
                        dest,
                        Rvalue::Aggregate {
                            kind: AggregateKind::Adt(adt),
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
                let lhs = self.lower_operand(lhs);
                let rhs = self.lower_operand(rhs);
                if let Some(dest) = dest {
                    self.assign_rvalue(
                        dest,
                        Rvalue::Binary {
                            op: bin_op(op),
                            lhs,
                            rhs,
                        },
                        span,
                    );
                }
            }
            ExprKind::Assign { op, target, value } => {
                self.lower_assign(op, target, value, dest, span)
            }
            ExprKind::Update { op, target, .. } => self.lower_update(op, target, dest, span),
            ExprKind::Call { callee, args } => {
                let result_ty = self.expr_ty(expr);
                self.lower_call(callee, &args, dest, result_ty, span);
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if(condition, then_branch, else_branch, dest, span),
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
        let target_place = self.place_of_expr(target);
        let target_ty = self.expr_ty(target);

        match op {
            AssignOp::Assign => {
                let value = self.lower_operand(value);
                self.assign_operand(target_place, value, span);
            }
            _ => {
                let rhs = self.lower_operand(value);
                let lhs = self.place_operand(target_place.clone(), target_ty);
                self.assign_rvalue(
                    target_place,
                    Rvalue::Binary {
                        op: compound_bin_op(op),
                        lhs,
                        rhs,
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
        self.assign_rvalue(target_place, Rvalue::Binary { op, lhs, rhs }, span);

        if let Some(dest) = dest {
            self.assign_constant(dest, Constant::Unit, span);
        }
    }

    fn lower_call(
        &mut self,
        callee: HirExprId,
        args: &[HirExprId],
        dest: Option<Place>,
        result_ty: TyId,
        span: Span,
    ) {
        let function = match self.hir.expr(callee).kind.clone() {
            ExprKind::Path(Res::Function(definition)) => Some(definition),
            _ => None,
        };
        let args: Vec<Operand> = args.iter().map(|arg| self.lower_operand(*arg)).collect();

        let Some(function) = function else {
            if let Some(dest) = dest {
                self.assign_constant(dest, Constant::Unit, span);
            }
            return;
        };
        let dest = dest.unwrap_or_else(|| self.new_temp(result_ty, span));
        self.assign_rvalue(
            dest,
            Rvalue::Call {
                func: function,
                args,
            },
            span,
        );
    }

    fn lower_operand(&mut self, expr: HirExprId) -> Operand {
        let (span, kind) = {
            let expr = self.hir.expr(expr);
            (expr.span, expr.kind.clone())
        };
        let ty = self.expr_ty(expr);

        match kind {
            ExprKind::Lit(literal) => Operand::Constant(self.constant(literal, ty)),
            ExprKind::Path(Res::Local(local)) => {
                let place = self.place_of_local(local);
                self.place_operand(place, ty)
            }
            ExprKind::Path(Res::Function(_)) | ExprKind::Error => Operand::Constant(Constant::Unit),
            ExprKind::Deref { .. } | ExprKind::Field { .. } => {
                let place = self.place_of_expr(expr);
                self.place_operand(place, ty)
            }
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
            Literal::Str(symbol) => Constant::Str(symbol),
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
        // pointee outlives the read and is still owned by whoever lent it.
        if place.is_deref() || self.hir.ty_ctxt().is_copy(ty) {
            Operand::Copy(place)
        } else {
            Operand::Move(place)
        }
    }

    fn place_of_expr(&self, expr: HirExprId) -> Place {
        let place = self.typeck.place(expr).unwrap_or_else(|| {
            panic!("expression {expr:?} has no resolved place; this is a compiler bug")
        });
        let local = self.place_of_local(place.local).local;
        let projections = place
            .projections
            .iter()
            .map(|projection| match projection {
                seal_typeck::PlaceProjection::Deref => Projection::Deref,
                seal_typeck::PlaceProjection::Field(field) => Projection::Field(*field),
            })
            .collect();
        Place::with_projections(local, projections)
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
