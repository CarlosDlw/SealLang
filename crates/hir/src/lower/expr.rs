//! Expression lowering.

use seal_ast::{ExprId as AstExprId, ExprKind as AstExprKind};
use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_span::Span;
use seal_symbol::Symbol;

use super::Lowering;
use crate::TraitMethodLookup;
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
            AstExprKind::PathQualified {
                type_name,
                type_span,
                name,
                name_span,
            } => self.lower_qualified_path(type_name, type_span, name, name_span),
            AstExprKind::Unary { op, operand } => ExprKind::Unary {
                op,
                operand: self.lower_expr(operand),
            },
            AstExprKind::Borrow { operand, mutable } => {
                let kind = if mutable {
                    seal_ty::RefKind::Exclusive
                } else {
                    seal_ty::RefKind::Shared
                };
                match self.ast.expr(operand).kind.clone() {
                    AstExprKind::Slice { base, start, end } => ExprKind::SliceRef {
                        base: self.lower_expr(base),
                        start: start.map(|start| self.lower_expr(start)),
                        end: end.map(|end| self.lower_expr(end)),
                        kind,
                    },
                    _ => ExprKind::Borrow {
                        operand: self.lower_expr(operand),
                        kind,
                    },
                }
            }
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
            AstExprKind::Array { elements } => ExprKind::Array {
                elements: elements
                    .iter()
                    .map(|element| self.lower_expr(*element))
                    .collect(),
            },
            AstExprKind::ArrayRepeat { value, length, .. } => ExprKind::ArrayRepeat {
                value: self.lower_expr(value),
                length,
            },
            AstExprKind::Index { base, index } => {
                let base = self.lower_expr(base);
                let index = self.lower_expr(index);
                if self.hir.place(base).is_none() {
                    let span = self.ast.expr(expr).span;
                    self.error(codes::E2013, span, "indexing requires a place")
                        .help("store the array in a `let` binding and index that binding")
                        .emit();
                }
                // Keep the node even when the base is not a place: the checker
                // can still type the array and the index, and every lowered
                // expression stays reachable from its parent.
                ExprKind::Index { base, index }
            }
            AstExprKind::Slice { base, start, end } => {
                let base = self.lower_expr(base);
                let start = start.map(|start| self.lower_expr(start));
                let end = end.map(|end| self.lower_expr(end));
                if self.hir.place(base).is_none() {
                    let span = self.ast.expr(expr).span;
                    self.error(codes::E2013, span, "slicing requires a place")
                        .help("store the array in a `let` binding and slice that binding")
                        .emit();
                }
                ExprKind::Slice { base, start, end }
            }
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
            AstExprKind::Call { callee, args } => {
                if self.is_len_builtin(callee) {
                    match args.as_slice() {
                        [operand] => ExprKind::Len {
                            operand: self.lower_expr(*operand),
                        },
                        _ => {
                            self.error(
                                codes::E2014,
                                span,
                                format!(
                                    "`len` takes exactly one argument, but {} were supplied",
                                    args.len()
                                ),
                            )
                            .help("pass one slice reference, as in `len(values)`")
                            .emit();
                            for arg in &args {
                                self.lower_expr(*arg);
                            }
                            ExprKind::Error
                        }
                    }
                } else {
                    ExprKind::Call {
                        callee: self.lower_expr(callee),
                        args: args.iter().map(|arg| self.lower_expr(*arg)).collect(),
                    }
                }
            }
            AstExprKind::MethodCall {
                receiver,
                name,
                name_span,
                args,
            } => ExprKind::MethodCall {
                receiver: self.lower_expr(receiver),
                name,
                name_span,
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

    /// Resolves `Type::name` to an associated function declared by the type.
    ///
    /// Inherent functions are tried first, then the traits the type implements,
    /// which is what makes `Type::name()` work for an associated function a
    /// trait declares. The resulting node is a normal function path, so calls,
    /// arguments, and code generation need no additional case. Instance methods
    /// are rejected in both lookups: they need a receiver and are only reachable
    /// through `.`.
    fn lower_qualified_path(
        &mut self,
        type_name: Symbol,
        type_span: Span,
        name: Symbol,
        name_span: Span,
    ) -> ExprKind {
        let Some(&type_def) = self.types.get(&type_name) else {
            let missing = self.interner.resolve(type_name);
            if self.traits.contains_key(&type_name) {
                self.error(
                    codes::E2027,
                    type_span,
                    format!("`{missing}` is a trait, not a type"),
                )
                .help("call an associated function through the struct that implements it")
                .emit();
            } else if self.globals.contains_key(&type_name) {
                self.error(
                    codes::E2015,
                    type_span,
                    format!("`{missing}` is a value, not a type"),
                )
                .help("a qualified path starts with a struct name")
                .emit();
            } else {
                self.error(
                    codes::E2002,
                    type_span,
                    format!("cannot find type `{missing}` in this scope"),
                )
                .help("declare it with `struct`, or check the spelling")
                .emit();
            }
            return ExprKind::Error;
        };

        if let Some(function) = self.hir.method(type_def, name) {
            return self.qualified_function(function, name, name_span);
        }

        match self.hir.trait_method(type_def, name) {
            TraitMethodLookup::Found { function, .. } => {
                self.qualified_function(function, name, name_span)
            }
            TraitMethodLookup::Ambiguous(traits) => {
                let traits = traits
                    .iter()
                    .map(|definition| {
                        format!(
                            "`{}`",
                            self.interner
                                .resolve(self.hir.definitions().name(*definition))
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.error(
                    codes::E2028,
                    name_span,
                    format!(
                        "`{}` is declared by more than one trait implemented for `{}`",
                        self.interner.resolve(name),
                        self.interner.resolve(type_name)
                    ),
                )
                .help(format!(
                    "the traits are {traits}; rename the method in one of them"
                ))
                .emit();
                ExprKind::Error
            }
            TraitMethodLookup::Missing => {
                self.error(
                    codes::E2018,
                    name_span,
                    format!(
                        "the type `{}` has no associated function `{}`",
                        self.interner.resolve(type_name),
                        self.interner.resolve(name)
                    ),
                )
                .help(
                    "declare it in an `impl` block for the type, or in a trait the type implements",
                )
                .emit();
                ExprKind::Error
            }
        }
    }

    /// Turns a resolved associated function into a callable path.
    fn qualified_function(&mut self, function: DefId, name: Symbol, name_span: Span) -> ExprKind {
        if self.hir.receiver(function).is_some() {
            let function_name = self.interner.resolve(name);
            self.error(
                codes::E2019,
                name_span,
                format!("`{function_name}` is a method, not an associated function"),
            )
            .help(format!(
                "call it on a value, as in `value.{function_name}()`"
            ))
            .emit();
            return ExprKind::Error;
        }

        ExprKind::Path(crate::Res::Function(function))
    }

    /// Returns `true` when `callee` is the built-in `len` rather than a user item.
    ///
    /// A user function or binding named `len` shadows the intrinsic, so existing
    /// and future programs keep control over the name.
    fn is_len_builtin(&self, callee: AstExprId) -> bool {
        let AstExprKind::Path(name) = self.ast.expr(callee).kind else {
            return false;
        };
        self.interner.resolve(name) == "len"
            && !self.globals.contains_key(&name)
            && self.scopes.lookup(name).is_none()
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
            if self.traits.contains_key(&name) {
                self.error(
                    codes::E2027,
                    name_span,
                    format!("`{struct_name}` is a trait, not a struct"),
                )
                .help("build a value of a struct that implements it")
                .emit();
                return ExprKind::Error;
            }
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
