//! Expression lowering.

use seal_ast::{ExprId as AstExprId, ExprKind as AstExprKind};
use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
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
            AstExprKind::Path {
                name,
                name_span: _,
                args,
            } => match self.resolve(name, span) {
                Some(res) => {
                    let expected = match res {
                        crate::Res::Function(function)
                        | crate::Res::TraitFunction { function, .. } => {
                            self.own_generic_count(function)
                        }
                        crate::Res::Const(_) | crate::Res::Static(_) | crate::Res::Local(_) => 0,
                    };
                    let args = self.lower_expr_arguments(args.as_ref(), expected);
                    ExprKind::Path { res, args }
                }
                None => ExprKind::Error,
            },
            AstExprKind::PathQualified {
                type_name,
                type_span,
                type_args,
                name,
                name_span,
            } => {
                self.lower_qualified_path(type_name, type_span, type_args.as_ref(), name, name_span)
            }
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
            AstExprKind::TupleField {
                base,
                index,
                index_span,
            } => {
                let base = self.lower_expr(base);
                if self.hir.place(base).is_none() {
                    self.error(codes::E2012, index_span, "tuple access requires a place")
                        .help(
                            "store the tuple in a `let` binding and access the element through it",
                        )
                        .emit();
                    ExprKind::Error
                } else {
                    ExprKind::TupleField {
                        base,
                        index,
                        index_span,
                    }
                }
            }
            AstExprKind::Tuple { elements } => ExprKind::Tuple {
                elements: elements
                    .iter()
                    .map(|element| self.lower_expr(*element))
                    .collect(),
            },
            AstExprKind::Struct {
                name,
                name_span,
                type_args,
                fields,
            } => self.lower_struct_literal(name, name_span, type_args.as_ref(), &fields),
            AstExprKind::Variant {
                type_name,
                type_span,
                type_args,
                name,
                name_span,
                fields,
            } => self.lower_variant_literal(
                type_name,
                type_span,
                type_args.as_ref(),
                name,
                name_span,
                &fields,
            ),
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
            AstExprKind::Cast {
                operand,
                ty,
                as_span,
            } => {
                let operand = self.lower_expr(operand);
                let target_span = self.ast.ty(ty).span;
                ExprKind::Cast {
                    operand,
                    ty: self.lower_type(ty),
                    as_span,
                    target_span,
                }
            }
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
                generic_args,
                args,
            } => {
                let count = generic_args.as_ref().map_or(0, |args| args.args.len());
                let generic_args = self.lower_expr_arguments(generic_args.as_ref(), count);
                ExprKind::MethodCall {
                    receiver: self.lower_expr(receiver),
                    name,
                    name_span,
                    generic_args,
                    args: args.iter().map(|arg| self.lower_expr(*arg)).collect(),
                }
            }
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
            AstExprKind::IfLet {
                pattern,
                scrutinee,
                then_branch,
                else_branch,
            } => {
                let scrutinee = self.lower_expr(scrutinee);
                self.scopes.push();
                let pattern = self.lower_arm_pattern(pattern);
                let then_branch = self.lower_block(then_branch);
                self.scopes.pop();
                ExprKind::IfLet {
                    pattern,
                    scrutinee,
                    then_branch,
                    else_branch: else_branch.map(|branch| self.lower_block(branch)),
                }
            }
            AstExprKind::Block(block) => ExprKind::Block(self.lower_block(block)),
            // An unsafe block has no runtime representation: its value is the
            // value of its block.
            AstExprKind::Unsafe(block) => ExprKind::Block(self.lower_block(block)),
            AstExprKind::Paren(inner) => return self.lower_expr(inner),
            AstExprKind::Error => ExprKind::Error,
        };

        self.hir.push_expr(Expr { kind, span })
    }

    /// Resolves `Type::name` to an associated function declared by the type.
    ///
    /// Inherent functions are tried first, then the traits the type implements,
    /// which is what makes `Type::name()` work for an associated function a
    /// trait declares. A concrete implementation becomes a normal function
    /// path; an omitted default becomes [`crate::Res::TraitFunction`], so calls,
    /// arguments, and code generation share one symbolic path. Instance methods
    /// are rejected in both lookups: they need a receiver and are only reachable
    /// through `.`.
    fn lower_qualified_path(
        &mut self,
        type_name: Symbol,
        type_span: Span,
        type_args: Option<&seal_ast::GenericArgs>,
        name: Symbol,
        name_span: Span,
    ) -> ExprKind {
        if type_name == self.self_symbol {
            if let Some(resolved) = self.lower_trait_self_function(type_args, name, name_span) {
                return resolved;
            }
            self.error(
                codes::E2018,
                type_span,
                "`Self` names an associated function only inside a trait body",
            )
            .help("call the function through a concrete type instead")
            .emit();
            return ExprKind::Error;
        }
        // A type parameter is not a type definition, but it can still name an
        // associated function through its bounds.
        if let Some(param) = self.generic_scope_param(type_name) {
            return self
                .lower_bound_associated(param, type_name, type_span, type_args, name, name_span);
        }
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
        let type_args = self.lower_nominal_arguments(type_def, type_args);

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
                args: type_args,
                fields: Vec::new(),
            };
        }

        match self.select_inherent_function(type_def, name, type_args, name_span) {
            InherentLookup::Found(function) => {
                return self.qualified_function(function, name, name_span, type_args);
            }
            InherentLookup::Rejected => return ExprKind::Error,
            InherentLookup::Missing => {}
        }

        match self.hir.trait_method(type_def, name) {
            TraitMethodLookup::Found { function, .. } => {
                self.qualified_function(function, name, name_span, type_args)
            }
            TraitMethodLookup::Default {
                trait_ref,
                function,
            } => {
                let Some(trait_ref) =
                    self.instantiate_impl_trait_ref(trait_ref, type_def, type_args)
                else {
                    let trait_name = self
                        .interner
                        .resolve(self.hir.definitions().name(trait_ref.trait_def));
                    self.error(
                        codes::E2018,
                        name_span,
                        format!(
                            "no implementation of `{trait_name}` matches the supplied type arguments for `{}`",
                            self.interner.resolve(name)
                        ),
                    )
                    .help("use type arguments that match the impl self type")
                    .emit();
                    return ExprKind::Error;
                };
                let Some(definition) = self.hir.trait_definition(trait_ref.trait_def) else {
                    return ExprKind::Error;
                };
                let Some(method) = definition.method(name) else {
                    return ExprKind::Error;
                };
                if method.receiver.is_some() {
                    self.error(
                        codes::E2019,
                        name_span,
                        format!(
                            "`{}` is a method, not an associated function",
                            self.interner.resolve(name)
                        ),
                    )
                    .help("call it on a value, as in `value.name()`")
                    .emit();
                    return ExprKind::Error;
                }
                ExprKind::Path {
                    res: crate::Res::TraitFunction {
                        function,
                        trait_ref,
                    },
                    args: None,
                }
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

    /// Lowers `Self::name(...)` inside a trait default body.
    ///
    /// The call stays symbolic: `Self` is the trait's implicit parameter, so
    /// monomorphization selects an override or the default body itself.
    fn lower_trait_self_function(
        &mut self,
        type_args: Option<&seal_ast::GenericArgs>,
        name: Symbol,
        name_span: Span,
    ) -> Option<ExprKind> {
        let function = self.function?;
        let trait_def = self.hir.definitions().get(function).parent?;
        if self.hir.definitions().kind(trait_def) != seal_def::DefKind::Trait
            || self.self_ty.is_some()
        {
            return None;
        }
        let generics = self.hir.generics(trait_def)?;
        let self_param = generics.params.iter().find(|parameter| parameter.is_self)?;
        let tcx = self.hir.ty_ctxt();
        let self_ty = tcx.param_ty(self_param.param);
        let trait_arguments: Vec<seal_ty::TyId> = generics
            .params
            .iter()
            .filter(|parameter| !parameter.is_self)
            .map(|parameter| tcx.param_ty(parameter.param))
            .collect();
        let trait_ref = seal_ty::TraitRef::new(trait_def, self_ty, tcx.substs(&trait_arguments));
        let Some(method) = self
            .hir
            .trait_definition(trait_def)
            .and_then(|definition| definition.method(name).cloned())
        else {
            let trait_name = self
                .interner
                .resolve(self.hir.definitions().name(trait_def));
            self.error(
                codes::E2018,
                name_span,
                format!(
                    "the trait `{trait_name}` has no associated function `{}`",
                    self.interner.resolve(name)
                ),
            )
            .help("declare it in the trait, or check the spelling")
            .emit();
            return Some(ExprKind::Error);
        };
        if method.receiver.is_some() {
            self.error(
                codes::E2019,
                name_span,
                format!(
                    "`{}` is a method, not an associated function",
                    self.interner.resolve(name)
                ),
            )
            .help(format!(
                "call it on a value, as in `value.{}()`",
                self.interner.resolve(name)
            ))
            .emit();
            return Some(ExprKind::Error);
        }
        let expected = method.generics.params.len();
        let args = self.lower_expr_arguments(type_args, expected);
        Some(ExprKind::Path {
            res: crate::Res::TraitFunction {
                function: method.def,
                trait_ref,
            },
            args,
        })
    }

    /// Instantiates an impl's trait reference with the type arguments written
    /// before an associated function.
    fn instantiate_impl_trait_ref(
        &self,
        trait_ref: seal_ty::TraitRef,
        type_def: DefId,
        type_args: Option<seal_ty::SubstsId>,
    ) -> Option<seal_ty::TraitRef> {
        let Some(type_args) = type_args else {
            return Some(trait_ref);
        };
        let tcx = self.hir.ty_ctxt();
        let Some(adt) = self.hir.adt(type_def) else {
            return Some(trait_ref);
        };
        let actual = tcx.adt_ty_with(adt, type_args);
        let mut unifier = seal_ty::TemplateUnifier::new(tcx);
        if !unifier.unify(trait_ref.self_ty, actual) {
            return None;
        }
        let matched = unifier.into_match();
        let implementation = self
            .hir
            .trait_impls()
            .into_iter()
            .find(|implementation| implementation.trait_ref == trait_ref)?;
        let impl_arguments: Vec<seal_ty::TyId> = implementation
            .generics
            .params
            .iter()
            .map(|parameter| {
                matched
                    .left()
                    .get(&parameter.param)
                    .copied()
                    .unwrap_or_else(|| tcx.param_ty(parameter.param))
            })
            .collect();
        let substs = tcx.substs(&impl_arguments);
        let trait_arguments: Vec<seal_ty::TyId> = tcx
            .substs_args(trait_ref.args)
            .iter()
            .map(|argument| tcx.substitute(*argument, substs))
            .collect();
        Some(seal_ty::TraitRef::new(
            trait_ref.trait_def,
            tcx.substitute(trait_ref.self_ty, substs),
            tcx.substs(&trait_arguments),
        ))
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
            type_args,
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
        let type_args = self.lower_nominal_arguments(definition, type_args.as_ref());
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
            args: type_args,
            fields,
        })
    }

    /// Turns a resolved associated function into a callable path.
    fn qualified_function(
        &mut self,
        function: DefId,
        name: Symbol,
        name_span: Span,
        args: Option<seal_ty::SubstsId>,
    ) -> ExprKind {
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

        ExprKind::Path {
            res: crate::Res::Function(function),
            args,
        }
    }

    /// Lowers the explicit type arguments written before an associated name.
    fn lower_nominal_arguments(
        &mut self,
        definition: DefId,
        args: Option<&seal_ast::GenericArgs>,
    ) -> Option<seal_ty::SubstsId> {
        let adt = self
            .hir
            .adt(definition)
            .expect("every type definition reserves an aggregate");
        let expected = self.hir.ty_ctxt().adt_generic_count(adt) as usize;
        self.lower_expr_arguments(args, expected)
    }

    /// Returns `true` when `callee` is the built-in `len` rather than a user item.
    ///
    /// A user function or binding named `len` shadows the intrinsic, so existing
    /// and future programs keep control over the name.
    fn is_len_builtin(&self, callee: AstExprId) -> bool {
        let AstExprKind::Path { name, .. } = self.ast.expr(callee).kind else {
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
        type_args: Option<&seal_ast::GenericArgs>,
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

        let type_args = self.lower_nominal_arguments(definition, type_args);
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
            args: type_args,
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
        type_args: Option<&seal_ast::GenericArgs>,
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

        let type_args = self.lower_nominal_arguments(definition, type_args);
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
                    args: type_args,
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
            args: type_args,
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

    /// Chooses the inherent associated function named by `Type::name`.
    ///
    /// Several impl blocks for one generic type may declare the same name for
    /// disjoint instances. Written type arguments select the block whose self
    /// type they match; without them, more than one candidate is ambiguous.
    fn select_inherent_function(
        &mut self,
        type_def: DefId,
        name: Symbol,
        type_args: Option<seal_ty::SubstsId>,
        name_span: Span,
    ) -> InherentLookup {
        let candidates: Vec<DefId> = self.hir.methods(type_def, name).to_vec();
        let Some(&first) = candidates.first() else {
            return InherentLookup::Missing;
        };
        let Some(type_args) = type_args else {
            if candidates.len() == 1 {
                return InherentLookup::Found(first);
            }
            self.report_ambiguous_associated(type_def, name, name_span);
            return InherentLookup::Rejected;
        };
        let adt = self
            .hir
            .adt(type_def)
            .expect("every type definition reserves an aggregate");
        let actual = self.hir.ty_ctxt().adt_ty_with(adt, type_args);
        let matches: Vec<DefId> = candidates
            .into_iter()
            .filter(|function| {
                self.impl_self_ty_of_function
                    .get(function)
                    .is_some_and(|self_ty| {
                        seal_ty::match_template(
                            self.hir.ty_ctxt(),
                            *self_ty,
                            actual,
                            &mut FxHashMap::default(),
                        )
                    })
            })
            .collect();
        match matches.len() {
            1 => InherentLookup::Found(matches[0]),
            0 => {
                let type_name = self.interner.resolve(self.type_name(type_def));
                let function = self.interner.resolve(name);
                self.error(
                    codes::E2018,
                    name_span,
                    format!(
                        "no impl of `{type_name}` matching these type arguments declares `{function}`"
                    ),
                )
                .help("use type arguments that match an impl self type")
                .emit();
                InherentLookup::Rejected
            }
            _ => {
                self.report_ambiguous_associated(type_def, name, name_span);
                InherentLookup::Rejected
            }
        }
    }

    fn report_ambiguous_associated(&self, type_def: DefId, name: Symbol, span: Span) {
        let type_name = self.interner.resolve(self.type_name(type_def));
        let function = self.interner.resolve(name);
        self.error(
            codes::E2063,
            span,
            format!(
                "the associated function `{function}` is declared by more than one impl of `{type_name}`"
            ),
        )
        .help(format!(
            "write the type arguments on the aggregate, as in `{type_name}::<...>::{function}()`"
        ))
        .emit();
    }

    fn type_name(&self, type_def: DefId) -> Symbol {
        self.hir.definitions().name(type_def)
    }

    /// Lowers `T::name()` where `T` is a type parameter.
    ///
    /// The method itself is symbolic until monomorphization: the call records
    /// the trait method declaration, and type checking resolves the trait from
    /// the parameter's bounds. Only associated functions are reachable this way.
    fn lower_bound_associated(
        &mut self,
        param: seal_ty::ParamTyId,
        type_name: Symbol,
        type_span: Span,
        type_args: Option<&seal_ast::GenericArgs>,
        name: Symbol,
        name_span: Span,
    ) -> ExprKind {
        let Some(function) = self.function else {
            return ExprKind::Error;
        };
        let environment = self.hir.param_env(function);
        let mut candidates: Vec<seal_ty::TraitMethodDef> = Vec::new();
        let mut seen_traits = FxHashSet::default();
        for bound in environment.bounds(param) {
            let seal_ty::Bound::Trait(trait_ref) = bound else {
                continue;
            };
            self.collect_bound_methods(*trait_ref, name, &mut seen_traits, &mut candidates);
        }
        let mut seen_methods = FxHashSet::default();
        candidates.retain(|method| seen_methods.insert(method.def));
        let parameter = self.interner.resolve(type_name);
        let function_name = self.interner.resolve(name);
        match candidates.len() {
            1 => {
                let method = &candidates[0];
                if method.receiver.is_some() {
                    self.error(
                        codes::E2019,
                        name_span,
                        format!("`{function_name}` is a method, not an associated function"),
                    )
                    .help(format!(
                        "call it on a value of type `{parameter}`, as in `value.{function_name}()`"
                    ))
                    .emit();
                    return ExprKind::Error;
                }
                let expected = method.generics.params.len();
                let args = self.lower_expr_arguments(type_args, expected);
                ExprKind::Path {
                    res: crate::Res::Function(method.def),
                    args,
                }
            }
            0 => {
                self.error(
                    codes::E2018,
                    type_span,
                    format!(
                        "the bounds of `{parameter}` do not declare an associated function `{function_name}`"
                    ),
                )
                .help("add a bound whose trait declares it, or call it on a concrete type")
                .emit();
                ExprKind::Error
            }
            _ => {
                let traits: Vec<String> = candidates
                    .iter()
                    .filter_map(|method| self.hir.definitions().get(method.def).parent)
                    .map(|trait_def| {
                        format!(
                            "`{}`",
                            self.interner
                                .resolve(self.hir.definitions().name(trait_def))
                        )
                    })
                    .collect();
                self.error(
                    codes::E2028,
                    name_span,
                    format!(
                        "`{function_name}` is declared by more than one trait visible on `{parameter}`"
                    ),
                )
                .help(format!(
                    "the traits are {}; rename the method in one of them",
                    traits.join(", ")
                ))
                .emit();
                ExprKind::Error
            }
        }
    }

    /// Adds the trait methods named `name` visible through `trait_ref`.
    fn collect_bound_methods(
        &self,
        trait_ref: seal_ty::TraitRef,
        name: Symbol,
        seen: &mut FxHashSet<DefId>,
        out: &mut Vec<seal_ty::TraitMethodDef>,
    ) {
        if !seen.insert(trait_ref.trait_def) {
            return;
        }
        let Some(declaration) = self.hir.trait_definition(trait_ref.trait_def) else {
            return;
        };
        if let Some(method) = declaration.method(name) {
            out.push(method.clone());
        }
        for supertrait in &declaration.supertraits {
            self.collect_bound_methods(*supertrait, name, seen, out);
        }
    }
}

/// The outcome of resolving one `Type::name` qualified path.
enum InherentLookup {
    /// Exactly one impl declares the function for the written type.
    Found(DefId),
    /// The type declares no such function, so trait lookup continues.
    Missing,
    /// An error was reported and the path cannot be resolved.
    Rejected,
}
