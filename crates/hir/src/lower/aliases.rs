//! Transparent type-alias lowering.
//!
//! An alias has no type identity: using one substitutes its arguments into the
//! target and returns the resulting [`TyId`]. Targets are lowered lazily because
//! an alias may name a declaration that appears later or in another file. The
//! expansion stack turns a cycle into a diagnostic instead of non-termination.

use seal_ast::{GenericArgs, ItemKind};
use seal_def::{DefId, DefKind};
use seal_error_codes as codes;
use seal_span::Span;
use seal_ty::{SubstsId, TyId, TyKind};

use super::{ItemRef, Lowering};

impl Lowering<'_> {
    /// Lowers every declared alias target once, in source order.
    ///
    /// Recursive targets may already have finished through another alias, so
    /// this is deliberately idempotent. A failed target stores the error type
    /// and later uses recover without producing a cascade.
    pub(super) fn lower_type_aliases(&mut self) {
        let items = self.type_alias_items.clone();
        for (item, definition) in items {
            self.enter_file(item.file);
            let span = self.ast.item(item.id).span;
            let _ = self.ensure_type_alias_target(definition, span);
        }
    }

    /// Returns the lowered target of `definition`, lowering it on first use.
    ///
    /// A reference back to an alias already on the expansion stack is a cycle:
    /// the alias has no finite expansion, so the call reports `E2095` and
    /// returns the error type.
    pub(super) fn ensure_type_alias_target(&mut self, definition: DefId, use_span: Span) -> TyId {
        if let Some(target) = self.hir.type_alias_target(definition) {
            return target;
        }
        if let Some(position) = self
            .type_alias_stack
            .iter()
            .position(|active| *active == definition)
        {
            let mut names: Vec<String> = self.type_alias_stack[position..]
                .iter()
                .map(|active| {
                    format!(
                        "`{}`",
                        self.interner.resolve(self.hir.definitions().name(*active))
                    )
                })
                .collect();
            names.push(format!(
                "`{}`",
                self.interner
                    .resolve(self.hir.definitions().name(definition))
            ));
            let name = self
                .interner
                .resolve(self.hir.definitions().name(definition));
            self.error(
                codes::E2095,
                use_span,
                format!("the type alias `{name}` expands recursively"),
            )
            .help(format!("the cycle is {}", names.join(" -> ")))
            .help("use a struct or enum with a pointer field for recursive data")
            .emit();
            return self.hir.ty_ctxt_mut().error_ty();
        }
        let Some(item) = self.type_alias_decl.get(&definition).copied() else {
            return self.hir.ty_ctxt_mut().error_ty();
        };
        self.ensure_type_alias_declaration(item, definition, use_span)
    }

    fn ensure_type_alias_declaration(
        &mut self,
        item: ItemRef,
        definition: DefId,
        use_span: Span,
    ) -> TyId {
        let previous_file = self.current_file;
        self.enter_file(item.file);
        let declaration = match &self.ast.item(item.id).kind {
            ItemKind::TypeAlias(declaration) => declaration.clone(),
            _ => {
                self.enter_file(previous_file);
                return self.hir.ty_ctxt_mut().error_ty();
            }
        };
        self.type_alias_stack.push(definition);
        let levels = self.push_generic_scope(definition);
        let target = self.lower_type(declaration.ty);
        self.pop_generic_scope(levels);
        self.type_alias_stack.pop();
        self.enter_file(previous_file);
        self.hir.insert_type_alias(definition, target);
        let _ = use_span;
        target
    }

    /// Lowers an alias use in a type position.
    ///
    /// The target is substituted with the supplied arguments. Missing or extra
    /// arguments are diagnosed by the shared type-argument lowering path.
    pub(super) fn lower_type_alias_use(
        &mut self,
        definition: DefId,
        args: Option<&GenericArgs>,
        name_span: Span,
        use_span: Span,
    ) -> TyId {
        let target = self.ensure_type_alias_target(definition, use_span);
        let expected = self.own_generic_count(definition);
        let Some(substs) = self.lower_type_arguments(args, expected, false, name_span) else {
            return self.hir.ty_ctxt_mut().error_ty();
        };
        self.hir.ty_ctxt().substitute(target, substs)
    }

    /// Resolves a path that must name one nominal struct or enum.
    ///
    /// Type aliases are expanded first. When an expression omits the arguments
    /// of a generic alias, the underlying nominal definition is returned with
    /// no substitution so the existing literal and associated-function paths can
    /// infer them from fields or the call context.
    pub(super) fn nominal_type_for_path(
        &mut self,
        definition: DefId,
        args: Option<&GenericArgs>,
        name_span: Span,
        use_span: Span,
    ) -> Option<(DefId, Option<SubstsId>)> {
        if self.hir.definitions().kind(definition) != DefKind::TypeAlias {
            let adt = self.hir.adt(definition)?;
            let expected = self.hir.ty_ctxt().adt_generic_count(adt) as usize;
            let args = self.lower_expr_arguments(args, expected);
            return Some((definition, args));
        }

        if let Some(args) = args {
            let target = self.lower_type_alias_use(definition, Some(args), name_span, use_span);
            return self.nominal_parts(target, definition);
        }

        let target = self.ensure_type_alias_target(definition, use_span);
        match self.hir.ty_ctxt().kind(target) {
            TyKind::Adt { adt, .. } => Some((self.hir.ty_ctxt().adt(adt).def, None)),
            TyKind::Error => None,
            _ => {
                self.report_non_nominal_alias(definition, use_span);
                None
            }
        }
    }

    fn nominal_parts(&mut self, target: TyId, alias: DefId) -> Option<(DefId, Option<SubstsId>)> {
        match self.hir.ty_ctxt().kind(target) {
            TyKind::Adt { adt, args } => Some((self.hir.ty_ctxt().adt(adt).def, Some(args))),
            TyKind::Error => None,
            _ => {
                self.report_non_nominal_alias(alias, self.hir.definitions().get(alias).span);
                None
            }
        }
    }

    fn report_non_nominal_alias(&mut self, alias: DefId, span: Span) {
        let name = self.interner.resolve(self.hir.definitions().name(alias));
        self.error(
            codes::E2097,
            span,
            format!("the type alias `{name}` does not name a struct or enum"),
        )
        .help("use the underlying nominal type, or use the alias only in a type position")
        .emit();
    }
}
