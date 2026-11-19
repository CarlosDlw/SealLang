//! Validation of opt-in `Copy` implementations.
//!
//! `Copy` is not an ordinary trait contract. An implementation opts the whole
//! nominal declaration into bitwise duplication, so it must cover every
//! declaration parameter, the type may not also declare a destructor, and every
//! field of every variant must itself be `Copy`.
//!
//! The last rule is recursive: `Outer` is `Copy` only when `Inner` is, and
//! `Inner`'s implementation may be declared later in the file. Validation
//! therefore registers candidates first and removes the ones whose fields are
//! not `Copy` until the registry reaches a fixed point. A candidate removed in
//! one round can make another candidate fail in the next, so the loop continues
//! while it removes anything.

use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_ty::{AdtId, ParamEnv, TraitImplDef, TyId, TyKind};

use super::Lowering;

/// One implementation that passed the structural checks and is being checked
/// against the fields of its declaration.
struct Candidate {
    /// The `impl Copy` block.
    implementation: DefId,
    /// The aggregate the implementation applies to.
    adt: AdtId,
    /// The nominal self type, including its declaration parameters.
    self_ty: TyId,
    /// The implementation's source span, used for `E3077`.
    span: Span,
    /// Declaration order, so diagnostics do not depend on removal order.
    order: usize,
}

impl Lowering<'_> {
    /// Validates every `impl Copy` and records the implementations that survive.
    ///
    /// The pass runs after `validate_trait_impls` and before
    /// `validate_drop_impls`. `Drop` implementations are collected first so a
    /// `Copy`/`Drop` conflict is reported on the `Copy` side without depending
    /// on declaration order.
    pub(super) fn validate_copy_impls(&mut self) {
        let copy_trait = self.hir.copy_trait();
        let drop_trait = self.hir.drop_trait();
        let implementations = self.hir.trait_impls();

        let mut drop_spans: FxHashMap<AdtId, Span> = FxHashMap::default();
        for implementation in &implementations {
            if implementation.trait_ref.trait_def != drop_trait {
                continue;
            }
            if !self.impl_covers_declaration(implementation) {
                // A specialized `Drop` is already `E3067`; it cannot make a
                // valid `Copy` implementation conflict.
                continue;
            }
            let self_ty = implementation.trait_ref.self_ty;
            if let TyKind::Adt { adt, .. } = self.hir.ty_ctxt().kind(self_ty) {
                let span = self.hir.definitions().get(implementation.def).span;
                drop_spans.entry(adt).or_insert(span);
            }
        }

        let mut seen: FxHashMap<AdtId, Span> = FxHashMap::default();
        let mut candidates = Vec::new();
        for implementation in implementations {
            if implementation.trait_ref.trait_def != copy_trait {
                continue;
            }
            self.validate_copy_impl(&implementation, &drop_spans, &mut seen, &mut candidates);
        }

        let mut failures = Vec::new();
        loop {
            let mut removed_any = false;
            let mut index = 0;
            while index < candidates.len() {
                let implementation = candidates[index].implementation;
                let self_ty = candidates[index].self_ty;
                let environment = self.hir.param_env(implementation);
                if self.hir.ty_ctxt().is_copy_with(self_ty, &environment) {
                    index += 1;
                    continue;
                }
                let candidate = candidates.remove(index);
                let _ = self.hir.ty_ctxt().remove_copy_impl(candidate.adt);
                failures.push(candidate);
                removed_any = true;
            }
            if !removed_any {
                break;
            }
        }

        failures.sort_by_key(|candidate| candidate.order);
        for candidate in failures {
            self.report_non_copy_field(candidate);
        }
    }

    /// Applies the structural rules to one `impl Copy`.
    fn validate_copy_impl(
        &mut self,
        implementation: &TraitImplDef,
        drop_spans: &FxHashMap<AdtId, Span>,
        seen: &mut FxHashMap<AdtId, Span>,
        candidates: &mut Vec<Candidate>,
    ) {
        let self_ty = implementation.trait_ref.self_ty;
        let span = self.hir.definitions().get(implementation.def).span;
        let type_name = self.display_ty(self_ty);
        let TyKind::Adt { adt, .. } = self.hir.ty_ctxt().kind(self_ty) else {
            self.error(
                codes::E2015,
                span,
                format!("`{type_name}` is not a nominal type, so it cannot implement `Copy`"),
            )
            .help("implement `Copy` for a struct or enum declared in this file")
            .emit();
            return;
        };

        if !self.impl_covers_declaration(implementation) {
            let mut error = self.error(
                codes::E3076,
                span,
                format!("`impl Copy for {type_name}` does not cover the whole type declaration"),
            );
            if let Some(first) = seen.get(&adt) {
                error = error.secondary(*first, "the first implementation is here");
            }
            error
                .help(format!(
                    "write one `impl Copy for {type_name}` that repeats the declaration's parameters in order; `Copy` cannot be specialized"
                ))
                .emit();
            return;
        }

        // An exact duplicate was already reported by the coherence pass
        // (`E2021` for concrete headers, `E2058` for generic ones). Validation
        // keeps the first candidate and does not add a second `E3076`.
        if seen.contains_key(&adt) {
            return;
        }
        seen.insert(adt, span);

        if let Some(drop_span) = drop_spans.get(&adt) {
            self.error(
                codes::E3068,
                span,
                format!("`{type_name}` cannot implement `Copy` because it implements `Drop`"),
            )
            .secondary(*drop_span, "the destructor is declared here")
            .help(
                "assignment duplicates a `Copy` value, so it cannot own a destructor; remove one of the implementations",
            )
            .emit();
            return;
        }

        let order = candidates.len();
        if self
            .hir
            .ty_ctxt()
            .set_copy_impl(adt, implementation.def)
            .is_some()
        {
            self.dcx.bug("a validated `impl Copy` was recorded twice");
        }
        candidates.push(Candidate {
            implementation: implementation.def,
            adt,
            self_ty,
            span,
            order,
        });
    }

    /// Returns `true` when the implementation repeats the declaration's
    /// parameters in order, without specializing or adding any.
    fn impl_covers_declaration(&self, implementation: &TraitImplDef) -> bool {
        let tcx = self.hir.ty_ctxt();
        let TyKind::Adt { adt, args } = tcx.kind(implementation.trait_ref.self_ty) else {
            return false;
        };
        let declaration = tcx.adt(adt).def;
        let declared_count = self
            .hir
            .generics(declaration)
            .map(|generics| generics.params.len())
            .unwrap_or(0);
        let arguments = tcx.substs_args(args);
        arguments.len() == declared_count
            && implementation.generics.params.len() == declared_count
            && arguments
                .iter()
                .zip(&implementation.generics.params)
                .all(|(argument, parameter)| {
                    matches!(
                        tcx.kind(*argument),
                        TyKind::Param(param) if param == parameter.param
                    )
                })
    }

    /// Reports `E3077` for the first field that blocks the candidate.
    fn report_non_copy_field(&mut self, candidate: Candidate) {
        let environment = self.hir.param_env(candidate.implementation);
        let Some((place, field_ty)) = self.first_non_copy_field(candidate.self_ty, &environment)
        else {
            let type_name = self.display_ty(candidate.self_ty);
            self.error(
                codes::E3077,
                candidate.span,
                format!("`{type_name}` has a field that is not `Copy`"),
            )
            .help("every field of every variant must be `Copy`")
            .emit();
            return;
        };
        let type_name = self.display_ty(candidate.self_ty);
        let field_name = self.display_ty(field_ty);
        let diagnostic = self.error(
            codes::E3077,
            candidate.span,
            format!("{place} of `{type_name}` has type `{field_name}`, which is not `Copy`"),
        );
        let help = match self.hir.ty_ctxt().kind(field_ty) {
            TyKind::Param(param) => {
                let name = self
                    .interner
                    .resolve(self.hir.ty_ctxt().param_data(param).name);
                format!("add `{name}: Copy` to the implementation's generic parameters")
            }
            _ => "every field of every variant must be `Copy`; remove the field, make its type `Copy`, or remove this implementation".to_owned(),
        };
        diagnostic.help(help).emit();
    }

    /// Returns the first field of `self_ty` that is not `Copy`, with a
    /// human-readable description of its position.
    fn first_non_copy_field(
        &self,
        self_ty: TyId,
        environment: &ParamEnv,
    ) -> Option<(String, TyId)> {
        let tcx = self.hir.ty_ctxt();
        let TyKind::Adt { adt, args } = tcx.kind(self_ty) else {
            return None;
        };
        let definition = tcx.adt(adt);

        for (index, field) in definition.fields.iter().enumerate() {
            let field_ty = tcx.substitute(field.ty, args);
            if tcx.is_copy_with(field_ty, environment) {
                continue;
            }
            let place = match field.name {
                Some(name) => format!("field `{}`", self.interner.resolve(name)),
                None => format!("field {index}"),
            };
            return Some((place, field_ty));
        }

        for variant in &definition.variants {
            for (index, field) in variant.fields.iter().enumerate() {
                let field_ty = tcx.substitute(field.ty, args);
                if tcx.is_copy_with(field_ty, environment) {
                    continue;
                }
                let variant_name = self.interner.resolve(variant.name);
                let place = match field.name {
                    Some(name) => {
                        format!(
                            "field `{}` of variant `{variant_name}`",
                            self.interner.resolve(name)
                        )
                    }
                    None => format!("field {index} of variant `{variant_name}`"),
                };
                return Some((place, field_ty));
            }
        }

        None
    }
}
