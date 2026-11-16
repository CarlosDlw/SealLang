//! Expression lowering.

use seal_ast::{ExprId as AstExprId, ExprKind as AstExprKind};
use seal_error_codes as codes;
use seal_hash::FxHashSet;

use super::Lowering;
use crate::expr::{Expr, ExprKind};
use crate::id::HirExprId;

impl Lowering<'_> {
    /// Lowers one expression. `Paren` is removed: the tree already makes
    /// grouping explicit.
    pub(super) fn lower_expr(&mut self, expr: AstExprId) -> HirExprId {
        let (span, kind) = {
            let expr = self.ast.expr(expr);
            (expr.span, expr.kind.clone())
        };

        let kind = match kind {
            AstExprKind::Lit(literal) => ExprKind::Lit(literal),
            AstExprKind::Path(name) => match self.resolve(name, span) {
                Some(res) => ExprKind::Path(res),
                None => ExprKind::Error,
            },
            AstExprKind::Unary { op, operand } => ExprKind::Unary {
                op,
                operand: self.lower_expr(operand),
            },
            AstExprKind::Borrow { operand, mutable } => ExprKind::Borrow {
                operand: self.lower_expr(operand),
                kind: if mutable {
                    seal_ty::RefKind::Exclusive
                } else {
                    seal_ty::RefKind::Shared
                },
            },
            AstExprKind::Deref { operand } => ExprKind::Deref {
                operand: self.lower_expr(operand),
            },
            AstExprKind::Field {
                base,
                name,
                name_span,
            } => {
                let base = self.lower_expr(base);
                if self.hir.place(base).is_none() {
                    self.error(codes::E2012, name_span, "field access requires a place")
                        .help("store the value in a `let` binding and access the field through it")
                        .emit();
                    ExprKind::Error
                } else {
                    ExprKind::Field {
                        base,
                        name,
                        name_span,
                    }
                }
            }
            AstExprKind::Struct {
                name,
                name_span,
                fields,
            } => self.lower_struct_literal(name, name_span, &fields),
            AstExprKind::Binary { op, lhs, rhs } => ExprKind::Binary {
                op,
                lhs: self.lower_expr(lhs),
                rhs: self.lower_expr(rhs),
            },
            AstExprKind::Assign { op, target, value } => ExprKind::Assign {
                op,
                target: self.lower_expr(target),
                value: self.lower_expr(value),
            },
            AstExprKind::Update {
                op,
                operand,
                postfix,
            } => ExprKind::Update {
                op,
                target: self.lower_expr(operand),
                postfix,
            },
            AstExprKind::Call { callee, args } => ExprKind::Call {
                callee: self.lower_expr(callee),
                args: args.iter().map(|arg| self.lower_expr(*arg)).collect(),
            },
            AstExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => ExprKind::If {
                condition: self.lower_expr(condition),
                then_branch: self.lower_block(then_branch),
                else_branch: else_branch.map(|branch| self.lower_block(branch)),
            },
            AstExprKind::Paren(inner) => return self.lower_expr(inner),
            AstExprKind::Error => ExprKind::Error,
        };

        self.hir.push_expr(Expr { kind, span })
    }

    fn lower_struct_literal(
        &mut self,
        name: seal_symbol::Symbol,
        name_span: seal_span::Span,
        fields: &[seal_ast::FieldInit],
    ) -> ExprKind {
        // Initializers are evaluated in the order they are written, even when a
        // name is invalid or repeated, so later resolution never reorders side
        // effects.
        let mut initializers = Vec::with_capacity(fields.len());
        for field in fields {
            initializers.push((field.name, field.name_span, self.lower_expr(field.value)));
        }

        let Some(definition) = self.types.get(&name).copied() else {
            let struct_name = self.interner.resolve(name);
            if self.globals.contains_key(&name) {
                self.error(
                    codes::E2011,
                    name_span,
                    format!("`{struct_name}` is a value, not a struct"),
                )
                .help("a struct literal starts with a struct name")
                .emit();
            } else {
                self.error(
                    codes::E2002,
                    name_span,
                    format!("cannot find struct `{struct_name}` in this scope"),
                )
                .help("declare it with `struct`")
                .emit();
            }
            return ExprKind::Error;
        };

        let adt = self
            .hir
            .adt(definition)
            .expect("every struct definition reserves an aggregate");
        let adt_def = self.hir.ty_ctxt().adt(adt);
        let mut seen = FxHashSet::default();
        let mut resolved = Vec::new();

        for (field_name, field_span, value) in initializers {
            match adt_def.field_by_name(field_name) {
                Some(field) => {
                    if !seen.insert(field.def) {
                        self.error(
                            codes::E2008,
                            field_span,
                            format!(
                                "the field `{}` is specified twice",
                                self.interner.resolve(field_name)
                            ),
                        )
                        .help("write each field exactly once")
                        .emit();
                    } else {
                        resolved.push((field.def, value));
                    }
                }
                None => {
                    self.error(
                        codes::E2007,
                        field_span,
                        format!(
                            "the struct `{}` has no field `{}`",
                            self.interner.resolve(name),
                            self.interner.resolve(field_name)
                        ),
                    )
                    .help("check the field name against the struct declaration")
                    .emit();
                }
            }
        }

        let missing: Vec<String> = adt_def
            .fields
            .iter()
            .filter(|field| !seen.contains(&field.def))
            .map(|field| format!("`{}`", self.interner.resolve(field.name)))
            .collect();
        if !missing.is_empty() {
            self.error(
                codes::E2009,
                name_span,
                "this struct literal is missing fields",
            )
            .help(format!("add {}", missing.join(", ")))
            .emit();
        }

        ExprKind::Struct {
            adt: definition,
            fields: resolved,
        }
    }
}
