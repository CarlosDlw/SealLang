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
use crate::pattern::MatchArm;

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
            AstExprKind::Variant {
                type_name,
                type_span,
                name,
                name_span,
                fields,
            } => self.lower_variant_literal(type_name, type_span, name, name_span, &fields),
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
                if let Some(kind) = self.lower_variant_call(callee, &args) {
                    kind
                } else if self.is_len_builtin(callee) {
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
            AstExprKind::Match { scrutinee, arms } => {
                let scrutinee = self.lower_expr(scrutinee);
                let arms = arms
                    .iter()
                    .map(|arm| {
                        self.scopes.push();
                        let pattern = self.lower_arm_pattern(arm.pattern);
                        let guard = arm.guard.map(|guard| self.lower_expr(guard));
                        let body = self.lower_expr(arm.body);
                        self.scopes.pop();
                        MatchArm {
                            pattern,
                            guard,
                            body,
                            span: arm.span,
                        }
                    })
                    .collect();
                ExprKind::Match { scrutinee, arms }
            }
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

        if let Some(variant) = self.hir.variant(type_def, name) {
            let adt = self
                .hir
                .adt(type_def)
                .expect("every enum definition reserves an aggregate");
            let definition = self
                .hir
                .ty_ctxt()
                .variant(adt, variant)
                .expect("a registered variant belongs to its enum");
            if !definition.fields.is_empty() {
                let type_name = self.interner.resolve(type_name).to_owned();
                let variant_name = self.interner.resolve(name).to_owned();
                self.error(
                    codes::E2034,
                    name_span,
                    format!("the variant `{type_name}::{variant_name}` has fields"),
                )
                .help(if definition.is_tuple() {
                    format!("construct it with `{type_name}::{variant_name}(...)`")
                } else {
                    format!("construct it with `{type_name}::{variant_name} {{ ... }}`")
                })
                .emit();
                return ExprKind::Error;
            }
            return ExprKind::Variant {
                adt: type_def,
                variant,
                fields: Vec::new(),
            };
        }

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
                        "the type `{}` has neither a variant nor an associated function `{}`",
                        self.interner.resolve(type_name),
                        self.interner.resolve(name)
                    ),
                )
                .help(
                    "declare it in an `impl` block for the type, in a trait the type implements, or as an enum variant",
                )
                .emit();
                ExprKind::Error
            }
        }
    }

    /// Lowers `Enum::Variant(value, ...)` when the qualified name is a variant.
    ///
    /// Parsing keeps every qualified call in the same `Call` node, because the
    /// parser cannot know whether `Type::name` is a variant or an associated
    /// function. Resolution decides here: when `name` is a variant, the call
    /// becomes a positional variant value; otherwise it falls through to the
    /// ordinary function-call path.
    fn lower_variant_call(&mut self, callee: AstExprId, args: &[AstExprId]) -> Option<ExprKind> {
        let AstExprKind::PathQualified {
            type_name,
            name,
            name_span,
            ..
        } = self.ast.expr(callee).kind.clone()
        else {
            return None;
        };
        let definition = *self.types.get(&type_name)?;
        let adt = self.hir.adt(definition)?;
        let adt_def = self.hir.ty_ctxt().adt(adt);
        if !adt_def.is_enum() {
            return None;
        }
        let variant = self.hir.variant(definition, name)?;
        let variant_def = adt_def.variant(variant)?.clone();
        let enum_name = self.interner.resolve(type_name).to_owned();
        let variant_name = self.interner.resolve(name).to_owned();
        let path = format!("{enum_name}::{variant_name}");

        if variant_def.is_unit() {
            let code = if args.is_empty() {
                codes::E2039
            } else {
                codes::E2033
            };
            let message = if args.is_empty() {
                format!("the unit variant `{path}` is written without parentheses")
            } else {
                format!("the unit variant `{path}` has no fields")
            };
            self.error(code, name_span, message)
                .help(format!("write `{path}`"))
                .emit();
            for arg in args {
                self.lower_expr(*arg);
            }
            return Some(ExprKind::Error);
        }

        if !variant_def.is_tuple() {
            self.error(
                codes::E2037,
                name_span,
                format!("the variant `{path}` has named fields"),
            )
            .help(format!("construct it with `{path} {{ ... }}`"))
            .emit();
            for arg in args {
                self.lower_expr(*arg);
            }
            return Some(ExprKind::Error);
        }

        if args.len() != variant_def.fields.len() {
            self.error(
                codes::E2038,
                name_span,
                format!(
                    "the variant `{path}` takes {} field(s), but {} were supplied",
                    variant_def.fields.len(),
                    args.len()
                ),
            )
            .help(if args.len() < variant_def.fields.len() {
                "supply the missing values"
            } else {
                "remove the extra values"
            })
            .emit();
            for arg in args {
                self.lower_expr(*arg);
            }
            return Some(ExprKind::Error);
        }

        let fields = variant_def
            .fields
            .iter()
            .zip(args)
            .map(|(field, arg)| (field.def, self.lower_expr(*arg)))
            .collect();
        Some(ExprKind::Variant {
            adt: definition,
            variant,
            fields,
        })
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
        let initializers = self.lower_initializers(fields);

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
                    format!("cannot find type `{struct_name}` in this scope"),
                )
                .help("declare it with `struct` or `enum`")
                .emit();
            }
            return ExprKind::Error;
        };

        let adt = self
            .hir
            .adt(definition)
            .expect("every type definition reserves an aggregate");
        let adt_def = self.hir.ty_ctxt().adt(adt);
        if adt_def.is_enum() {
            let type_name = self.interner.resolve(name).to_owned();
            self.error(
                codes::E2035,
                name_span,
                format!("`{type_name}` is an enum, not a struct"),
            )
            .help(format!(
                "construct a variant with `{type_name}::Variant {{ ... }}`"
            ))
            .emit();
            return ExprKind::Error;
        }

        let what = format!("the struct `{}`", self.interner.resolve(name));
        let resolved = self.resolve_named_fields(
            initializers,
            &adt_def.fields,
            &what,
            name_span,
            "this struct literal is missing fields",
        );

        ExprKind::Struct {
            adt: definition,
            fields: resolved,
        }
    }

    /// Lowers the initializers of a struct or variant literal.
    ///
    /// Initializers are evaluated in the order they are written, even when a
    /// name is invalid or repeated, so later resolution never reorders side
    /// effects.
    fn lower_initializers(
        &mut self,
        fields: &[seal_ast::FieldInit],
    ) -> Vec<(Symbol, Span, HirExprId)> {
        let mut initializers = Vec::with_capacity(fields.len());
        for field in fields {
            initializers.push((field.name, field.name_span, self.lower_expr(field.value)));
        }
        initializers
    }

    fn lower_variant_literal(
        &mut self,
        type_name: Symbol,
        type_span: Span,
        name: Symbol,
        name_span: Span,
        fields: &[seal_ast::FieldInit],
    ) -> ExprKind {
        let initializers = self.lower_initializers(fields);

        let Some(definition) = self.types.get(&type_name).copied() else {
            let missing = self.interner.resolve(type_name);
            if self.traits.contains_key(&type_name) {
                self.error(
                    codes::E2027,
                    type_span,
                    format!("`{missing}` is a trait, not a type"),
                )
                .help("construct a variant of an enum")
                .emit();
            } else if self.globals.contains_key(&type_name) {
                self.error(
                    codes::E2015,
                    type_span,
                    format!("`{missing}` is a value, not a type"),
                )
                .help("a variant literal starts with an enum name")
                .emit();
            } else {
                self.error(
                    codes::E2002,
                    type_span,
                    format!("cannot find type `{missing}` in this scope"),
                )
                .help("declare it with `enum`, or check the spelling")
                .emit();
            }
            return ExprKind::Error;
        };

        let adt = self
            .hir
            .adt(definition)
            .expect("every type definition reserves an aggregate");
        let adt_def = self.hir.ty_ctxt().adt(adt);
        if !adt_def.is_enum() {
            let type_name = self.interner.resolve(type_name).to_owned();
            self.error(
                codes::E2031,
                type_span,
                format!("`{type_name}` is a struct, not an enum"),
            )
            .help(format!("construct the struct with `{type_name} {{ ... }}`"))
            .emit();
            return ExprKind::Error;
        }

        let Some(variant) = self.hir.variant(definition, name) else {
            let type_name = self.interner.resolve(type_name).to_owned();
            let variant_name = self.interner.resolve(name).to_owned();
            let declared: Vec<String> = adt_def
                .variants
                .iter()
                .map(|variant| format!("`{}`", self.interner.resolve(variant.name)))
                .collect();
            let help = if declared.is_empty() {
                "this enum declares no variants".to_owned()
            } else {
                format!("the enum declares {}", declared.join(", "))
            };
            self.error(
                codes::E2032,
                name_span,
                format!("the enum `{type_name}` has no variant `{variant_name}`"),
            )
            .help(help)
            .emit();
            return ExprKind::Error;
        };
        let variant_def = adt_def
            .variant(variant)
            .expect("a registered variant belongs to its enum")
            .clone();

        if variant_def.is_tuple() {
            let enum_type = self.interner.resolve(type_name).to_owned();
            let variant_name = self.interner.resolve(name).to_owned();
            self.error(
                codes::E2036,
                name_span,
                format!("the variant `{enum_type}::{variant_name}` has positional fields"),
            )
            .help(format!(
                "construct it with `{enum_type}::{variant_name}(...)`"
            ))
            .emit();
            return ExprKind::Error;
        }

        if variant_def.is_unit() {
            if initializers.is_empty() {
                return ExprKind::Variant {
                    adt: definition,
                    variant,
                    fields: Vec::new(),
                };
            }
            let enum_type = self.interner.resolve(type_name).to_owned();
            let variant_name = self.interner.resolve(name).to_owned();
            self.error(
                codes::E2033,
                name_span,
                format!("the unit variant `{enum_type}::{variant_name}` has no fields"),
            )
            .help(format!(
                "write `{enum_type}::{variant_name}` without braces"
            ))
            .emit();
            return ExprKind::Error;
        }

        let what = format!(
            "the variant `{}::{}`",
            self.interner.resolve(type_name),
            self.interner.resolve(name)
        );
        let resolved = self.resolve_named_fields(
            initializers,
            &variant_def.fields,
            &what,
            name_span,
            "this variant literal is missing fields",
        );
        ExprKind::Variant {
            adt: definition,
            variant,
            fields: resolved,
        }
    }

    /// Resolves named initializers against a field list.
    fn resolve_named_fields(
        &mut self,
        initializers: Vec<(Symbol, Span, HirExprId)>,
        fields: &[seal_ty::FieldDef],
        what: &str,
        span: Span,
        missing_message: &str,
    ) -> Vec<(DefId, HirExprId)> {
        let mut seen = FxHashSet::default();
        let mut resolved = Vec::new();

        for (field_name, field_span, value) in initializers {
            match fields.iter().find(|field| field.name == Some(field_name)) {
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
                            "{what} has no field `{}`",
                            self.interner.resolve(field_name)
                        ),
                    )
                    .help("check the field name against the declaration")
                    .emit();
                }
            }
        }

        let missing: Vec<String> = fields
            .iter()
            .filter(|field| field.name.is_some() && !seen.contains(&field.def))
            .filter_map(|field| {
                field
                    .name
                    .map(|name| format!("`{}`", self.interner.resolve(name)))
            })
            .collect();
        if !missing.is_empty() {
            self.error(codes::E2009, span, missing_message)
                .help(format!("add {}", missing.join(", ")))
                .emit();
        }
        resolved
    }
}
