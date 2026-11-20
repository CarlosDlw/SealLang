//! The built-in `Fn`, `FnMut`, and `FnOnce` traits.
//!
//! These traits have no user implementations. A goal is proved by inspecting
//! the closure or safe function pointer directly: the closure kind must be at
//! least the required one and its signature must match the bound's tuple and
//! result. A type parameter delegates to the ordinary solver, which already
//! follows the installed supertrait chain.
//!
//! Calls through a parameter use the strongest declared bound, so an `Fn` bound
//! borrows the value shared even when `FnMut` and `FnOnce` are also available
//! through supertraits. The chosen bound, kind, and parameter list are recorded
//! for later phases.

use seal_error_codes as codes;
use seal_hash::FxHashMap;
use seal_hir::HirExprId;
use seal_span::Span;
use seal_ty::{Bound, ClosureKind, GenericParamDef, TraitRef, TyId, TyKind};

use super::Checker;
use super::flow::Flow;
use super::trait_solver::Proof;
use crate::results::{ClosureHint, ResolvedCall};

impl Checker<'_> {
    /// Records the bounds of generic parameters that were instantiated with
    /// fresh inference variables.
    ///
    /// A closure argument is checked against the variable standing for `F`, so
    /// the bounds of `F` must remain reachable after instantiation. Substituting
    /// them here lets closure signature inference and the call-site coercion
    /// read exactly the requirements the declaration wrote.
    pub(super) fn register_argument_bounds(&mut self, generics: &[GenericParamDef], args: &[TyId]) {
        if generics.len() != args.len() {
            return;
        }
        let substs = self.hir.ty_ctxt().substs(args);
        for (parameter, argument) in generics.iter().zip(args) {
            if !self.contains_infer(*argument) {
                continue;
            }
            let mut bounds = Vec::with_capacity(parameter.bounds.len());
            for bound in &parameter.bounds {
                match bound {
                    Bound::Copy => bounds.push(Bound::Copy),
                    Bound::Trait(trait_ref) => {
                        bounds.push(Bound::Trait(self.substitute_trait_ref(*trait_ref, substs)));
                    }
                }
            }
            if !bounds.is_empty() {
                self.inferred_bounds
                    .entry(*argument)
                    .or_default()
                    .extend(bounds);
            }
        }
    }

    /// Returns every function-trait bound visible for `ty`.
    pub(super) fn fn_trait_bounds_of_ty(&self, ty: TyId) -> Vec<(ClosureKind, TraitRef)> {
        let bounds: Vec<Bound> = match self.hir.ty_ctxt().kind(ty) {
            TyKind::Param(param) => self.env.bounds(param).to_vec(),
            TyKind::Infer(_) => self.inferred_bounds.get(&ty).cloned().unwrap_or_default(),
            _ => return Vec::new(),
        };
        let mut found = Vec::new();
        for bound in bounds {
            let Bound::Trait(trait_ref) = bound else {
                continue;
            };
            let Some(kind) = self.hir.fn_trait_kind(trait_ref.trait_def) else {
                continue;
            };
            if !found.contains(&(kind, trait_ref)) {
                found.push((kind, trait_ref));
            }
        }
        found
    }

    /// Returns the callable signature of a function-trait reference.
    pub(super) fn fn_trait_signature(&mut self, trait_ref: TraitRef) -> Option<(Vec<TyId>, TyId)> {
        let args = self.hir.ty_ctxt().substs_args(trait_ref.args);
        let [parameters, output] = args.as_slice() else {
            return None;
        };
        let parameters = self.resolve_deep(*parameters);
        let output = self.resolve_deep(*output);
        let parameters = match self.hir.ty_ctxt().kind(parameters) {
            TyKind::Unit => Vec::new(),
            TyKind::Tuple(tuple) => self.hir.ty_ctxt().tuple_elements(tuple),
            _ => return None,
        };
        Some((parameters, output))
    }

    /// Returns the closure hint supplied by the strongest function-trait bound.
    pub(super) fn fn_trait_hint(&mut self, ty: TyId) -> Option<ClosureHint> {
        let bounds = self.fn_trait_bounds_of_ty(ty);
        let mut best: Option<(ClosureKind, Vec<TyId>, TyId)> = None;
        for (kind, trait_ref) in bounds {
            let Some((params, ret)) = self.fn_trait_signature(trait_ref) else {
                continue;
            };
            if best
                .as_ref()
                .is_none_or(|(current, _, _)| rank(kind) > rank(*current))
            {
                best = Some((kind, params, ret));
            }
        }
        best.map(|(_, params, ret)| ClosureHint { params, ret })
    }

    /// Proves a function-trait goal for a closure or a safe pointer.
    ///
    /// `None` means the self type belongs to the ordinary solver: a rigid
    /// parameter uses the bounds in the environment, and an error type absorbs
    /// the goal.
    pub(super) fn prove_builtin_trait(&mut self, goal: TraitRef) -> Option<Proof> {
        let required = self.hir.fn_trait_kind(goal.trait_def)?;
        let Some((params, ret)) = self.fn_trait_signature(goal) else {
            return Some(Proof::Missing);
        };
        match self.hir.ty_ctxt().kind(goal.self_ty) {
            TyKind::Param(_) | TyKind::Error => None,
            TyKind::Dyn { .. } => Some(Proof::Missing),
            TyKind::Adt { adt, .. } => {
                let Some(closure) = self.hir.closure_for_adt(adt) else {
                    return Some(Proof::Missing);
                };
                let Some(info) = self.results.closure_info(closure) else {
                    return Some(Proof::Missing);
                };
                let signature = (info.signature.params.clone(), info.signature.ret);
                if closure_satisfies(info.kind, required) && signature == (params, ret) {
                    Some(Proof::Proven)
                } else {
                    Some(Proof::Missing)
                }
            }
            TyKind::FnPtr(_) => {
                let Some(definition) = self.hir.ty_ctxt().fn_ptr_parts(goal.self_ty) else {
                    return Some(Proof::Missing);
                };
                if !definition.is_unsafe && definition.params == params && definition.ret == ret {
                    Some(Proof::Proven)
                } else {
                    Some(Proof::Missing)
                }
            }
            _ => Some(Proof::Missing),
        }
    }

    /// Accepts a closure or safe pointer as an argument for a parameter whose
    /// bounds require a function trait.
    pub(super) fn coerce_to_fn_trait(
        &mut self,
        expr: HirExprId,
        found: TyId,
        expected: TyId,
    ) -> Result<bool, ()> {
        if self.contains_infer(found) || self.is_error(found) {
            return Ok(false);
        }
        // A rigid parameter is constrained by the ordinary bound check at the
        // call site, which reports the unsatisfied bound with its own context.
        if matches!(self.hir.ty_ctxt().kind(found), TyKind::Param(_)) {
            return Ok(false);
        }
        let bounds = self.fn_trait_bounds_of_ty(expected);
        if bounds.is_empty() {
            return Ok(false);
        }
        let span = self.hir.expr(expr).span;
        for (required, trait_ref) in &bounds {
            let goal = TraitRef::new(trait_ref.trait_def, found, trait_ref.args);
            if self.prove_trait(goal) != Proof::Proven {
                self.report_fn_trait_mismatch(span, found, *required, *trait_ref);
                return Err(());
            }
        }
        if matches!(self.hir.ty_ctxt().kind(expected), TyKind::Infer(_)) {
            let _ = self.infer.unify(self.hir.ty_ctxt(), expected, found);
        }
        Ok(true)
    }

    /// Checks `f(...)` where `f` has a declared function trait.
    pub(super) fn check_fn_trait_param_call(
        &mut self,
        expr: HirExprId,
        callee: HirExprId,
        callee_ty: TyId,
        args: &[HirExprId],
        flow: &mut Flow,
        span: Span,
    ) -> Option<TyId> {
        let bounds = self.fn_trait_bounds_of_ty(callee_ty);
        if bounds.is_empty() {
            return None;
        }

        let mut chosen: Option<(ClosureKind, TraitRef, Vec<TyId>, TyId)> = None;
        let mut conflict = false;
        for (kind, trait_ref) in bounds {
            let Some((params, ret)) = self.fn_trait_signature(trait_ref) else {
                continue;
            };
            if let Some((current, _, current_params, current_ret)) = &chosen {
                if *current_params != params || *current_ret != ret {
                    conflict = true;
                }
                if rank(kind) > rank(*current) {
                    chosen = Some((kind, trait_ref, params, ret));
                }
            } else {
                chosen = Some((kind, trait_ref, params, ret));
            }
        }
        if conflict {
            self.error(
                codes::E3096,
                span,
                "the function-trait bounds on this parameter disagree on the callable signature",
            )
            .help("make every `Fn` bound use the same parameter and result types")
            .emit();
            for arg in args {
                self.infer_expr(*arg, None, flow);
            }
            return Some(self.error_ty());
        }
        let (kind, trait_ref, params, ret) = chosen?;

        if kind == ClosureKind::FnMut
            && self.results.place(callee).is_some()
            && !self.borrow_is_writable(callee)
        {
            self.report_mutable_borrow(callee);
        }
        self.note_fn_once_call(callee, kind, flow);
        if params.len() != args.len() {
            self.error(
                codes::E3008,
                span,
                format!(
                    "this callable takes {} argument(s) but {} were supplied",
                    params.len(),
                    args.len()
                ),
            )
            .help("pass exactly one argument for each parameter in the bound")
            .emit();
        }
        for (index, arg) in args.iter().enumerate() {
            match params.get(index) {
                Some(expected) => {
                    self.check_expr(*arg, *expected, flow);
                }
                None => {
                    self.infer_expr(*arg, None, flow);
                }
            }
        }

        self.results.record_call(
            expr,
            ResolvedCall::FnTrait {
                trait_ref,
                kind,
                parameters: self.hir.ty_ctxt().substs(&params),
            },
        );
        Some(ret)
    }

    fn report_fn_trait_mismatch(
        &mut self,
        span: Span,
        found: TyId,
        required: ClosureKind,
        trait_ref: TraitRef,
    ) {
        let required_text = self
            .fn_trait_signature(trait_ref)
            .map(|(params, ret)| self.signature_text(&params, ret))
            .unwrap_or_else(|| required.as_str().to_string());
        if let Some(closure) = self.closure_of_ty(found) {
            if let Some(info) = self.results.closure_info(closure).cloned() {
                if !closure_satisfies(info.kind, required) {
                    self.error(
                        codes::E3096,
                        span,
                        format!(
                            "this closure is `{}`, but the bound requires `{}`",
                            info.kind.as_str(),
                            required.as_str()
                        ),
                    )
                    .help("use a closure with the required callable kind, or relax the bound")
                    .emit();
                    return;
                }
                let found_text = self.signature_text(&info.signature.params, info.signature.ret);
                self.error(
                    codes::E3096,
                    span,
                    format!(
                        "this closure has the signature `{found_text}`, but the bound requires `{required_text}`"
                    ),
                )
                .help("make the closure parameters and result match the bound")
                .emit();
                return;
            }
        }
        if let Some(definition) = self.hir.ty_ctxt().fn_ptr_parts(found) {
            if definition.is_unsafe {
                self.error(
                    codes::E3096,
                    span,
                    "an `unsafe func(...)` pointer cannot satisfy a safe function-trait bound",
                )
                .help("call the pointer inside an `unsafe` block directly, or use a safe function pointer")
                .emit();
                return;
            }
            let params = definition.params;
            let ret = definition.ret;
            let found_text = self.signature_text(&params, ret);
            self.error(
                codes::E3096,
                span,
                format!(
                    "this function pointer has the signature `{found_text}`, but the bound requires `{required_text}`"
                ),
            )
            .help("use a pointer with the parameters and result the bound declares")
            .emit();
            return;
        }
        let found_text = self.ty_text(found);
        self.error(
            codes::E3096,
            span,
            format!("`{found_text}` does not satisfy the function-trait bound `{required_text}`"),
        )
        .help("pass a closure or a safe function pointer with the required signature")
        .emit();
    }

    /// Marks the callee of an `FnOnce` call as moved.
    ///
    /// A `Copy` callable is duplicated by the call instead, so only a
    /// non-`Copy` whole local is consumed. Projections are left alone: a call
    /// through a place that dereferences or selects a field does not move the
    /// entire local.
    pub(super) fn note_fn_once_call(
        &mut self,
        callee: HirExprId,
        kind: ClosureKind,
        flow: &mut Flow,
    ) {
        if kind != ClosureKind::FnOnce {
            return;
        }
        let Some(place) = self.results.place(callee).cloned() else {
            return;
        };
        if !place.root.is_local()
            || place.contains_deref()
            || !place.is_whole()
            || self.is_copy_with(place.ty, &self.env)
        {
            return;
        }
        flow.move_out(place.root.def(), &place.path());
    }

    /// Returns `true` when `name` spells one of the call methods of the
    /// function traits. Those methods are never called with method syntax.
    pub(super) fn is_function_trait_method(&self, name: seal_symbol::Symbol) -> bool {
        matches!(
            self.interner.resolve(name),
            "call" | "call_mut" | "call_once"
        )
    }
}

/// Returns the total order of closure kinds, from weakest to strongest.
fn rank(kind: ClosureKind) -> u8 {
    match kind {
        ClosureKind::Fn => 2,
        ClosureKind::FnMut => 1,
        ClosureKind::FnOnce => 0,
    }
}

/// Returns `true` when a closure of `actual` kind can be used where `required`
/// is declared.
fn closure_satisfies(actual: ClosureKind, required: ClosureKind) -> bool {
    rank(actual) >= rank(required)
}

/// The function-trait bounds recorded for inference variables.
pub(super) type InferredBounds = FxHashMap<TyId, Vec<Bound>>;
