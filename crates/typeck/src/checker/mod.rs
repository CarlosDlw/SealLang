//! The checker context and per-body driver.

mod borrow;
mod call;
mod exhaustive;
mod expr;
mod flow;
mod for_loop;
mod generics;
mod literal;
mod method;
mod operators;
mod pattern;
mod place;
mod slice;
mod slice_patterns;
mod stmt;
mod trait_solver;
mod types;

use seal_def::DefId;
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::{Body, Hir, HirBlockId};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{InferCtxt, ParamEnv, TraitRef, TyId, TyKind};

use crate::results::{Signature, TypeckResults};
use flow::Flow;

/// How a place expression is being used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PlaceMode {
    /// The place's value is read, so it must be initialized. Reading a
    /// non-`Copy` pointee through a reference moves it and is rejected.
    Read,
    /// The place is an assignment target, so it need not be initialized yet.
    Write,
    /// The place is borrowed, so it must be initialized, but its value is not
    /// read: borrowing a non-`Copy` pointee does not move it.
    Borrow,
}

/// One type-checking pass over a lowered file.
pub(crate) struct Checker<'a> {
    pub(super) hir: &'a Hir,
    pub(super) dcx: &'a DiagCtxt,
    pub(super) interner: &'a Interner,
    pub(super) pointer_width: u8,
    pub(super) results: TypeckResults,
    pub(super) infer: InferCtxt,
    pub(super) env: ParamEnv,
    pub(super) proof_cache: FxHashMap<TraitRef, trait_solver::Proof>,
    /// The definition whose body is being checked, for its generic scope.
    pub(super) owner: Option<DefId>,
    pub(super) ret: TyId,
    /// Non-`Copy` pattern bindings referenced by the guard being checked.
    ///
    /// A guard runs before the arm commits, so moving a binding into it could
    /// not be undone when the guard fails. The set starts with every non-`Copy`
    /// binding of the arm; a path removes its entry after reporting, so one
    /// diagnostic covers all uses of the same binding.
    pub(super) guard_moves: Option<FxHashSet<DefId>>,
}

impl<'a> Checker<'a> {
    pub(crate) fn new(
        hir: &'a Hir,
        dcx: &'a DiagCtxt,
        interner: &'a Interner,
        pointer_width: u8,
    ) -> Self {
        let unit = hir
            .ty_ctxt()
            .lookup(TyKind::Unit)
            .expect("HIR lowering interns the unit type");
        let mut checker = Self {
            hir,
            dcx,
            interner,
            pointer_width,
            results: TypeckResults::new(),
            infer: InferCtxt::new(),
            env: ParamEnv::new(),
            proof_cache: FxHashMap::default(),
            owner: None,
            ret: unit,
            guard_moves: None,
        };
        checker.build_signatures();
        checker
    }

    pub(crate) fn run(mut self) -> TypeckResults {
        let bodies: Vec<Body> = self.hir.bodies().iter().cloned().collect();
        for body in bodies {
            self.check_body(&body);
        }
        self.results
    }

    /// Builds every signature before any body is checked, so callers see the
    /// declared types regardless of declaration order.
    fn build_signatures(&mut self) {
        let bodies: Vec<Body> = self.hir.bodies().iter().cloned().collect();
        for body in bodies {
            let owner_span = self.hir.definitions().get(body.owner).span;
            let ret = match body.ret {
                Some(ty) => self.storage_ty(ty, owner_span, "return type"),
                None => self.unit(),
            };
            let mut params = Vec::with_capacity(body.params.len());
            for param in &body.params {
                let span = self.hir.definitions().get(param.def).span;
                params.push(self.storage_ty(param.ty, span, "parameter"));
            }
            self.results.insert_signature(
                body.owner,
                Signature {
                    params,
                    ret,
                    generics: self.hir.flattened_generics(body.owner),
                },
            );
        }
    }

    fn check_body(&mut self, body: &Body) {
        let signature = self
            .results
            .signature(body.owner)
            .expect("every body receives a signature")
            .clone();
        self.ret = signature.ret;
        self.infer = InferCtxt::new();
        self.env = self.hir.param_env(body.owner);
        self.proof_cache.clear();
        self.owner = Some(body.owner);

        let mut flow = Flow::new();
        for (param, ty) in body.params.iter().zip(&signature.params) {
            self.results.record_local(param.def, *ty);
            flow.assign(param.def, &[]);
        }
        let _outcome = self.check_block(body.root, Some(self.ret), flow);
    }

    pub(super) fn error(
        &self,
        code: ErrorCode,
        span: Span,
        message: impl Into<String>,
    ) -> ErrorBuilder<'_> {
        self.dcx
            .error(message)
            .code(code.diag_code())
            .primary(span, "")
    }

    /// Reports a type mismatch unless unification can make the types equal.
    ///
    /// A failed unification leaves the inference table unchanged, so the error
    /// reports the original types rather than a partially bound variable.
    pub(super) fn mismatch(&mut self, span: Span, expected: TyId, found: TyId) {
        if self.is_compatible(expected, found) {
            return;
        }
        if self
            .infer
            .unify(self.hir.ty_ctxt(), expected, found)
            .is_ok()
        {
            return;
        }
        let expected = self.resolve(expected);
        let found = self.resolve(found);
        self.dcx
            .error("mismatched types")
            .code(codes::E3001.diag_code())
            .primary(
                span,
                format!(
                    "expected `{}`, found `{}`",
                    self.ty_text(expected),
                    self.ty_text(found)
                ),
            )
            .emit();
    }

    pub(super) fn report_unsized(&self, span: Span, ty: TyId, what: &str) {
        self.error(
            codes::E3010,
            span,
            format!("{what} cannot have the unsized type `{}`", self.ty_text(ty)),
        )
        .help("use a reference, such as `*str`, to store the value indirectly")
        .emit();
    }

    pub(super) fn report_function_as_value(&self, span: Span) {
        self.error(
            codes::E3007,
            span,
            "a function item cannot be used as a value",
        )
        .help("call the function instead, for example `name(...)`")
        .emit();
    }

    /// Returns the declared type of a local definition.
    pub(super) fn local_type(&self, local: DefId) -> TyId {
        self.results.local_ty(local).unwrap_or_else(|| {
            panic!("local definition {local:?} reached type checking without a type")
        })
    }

    /// Proves `goal` under the current body's parameter environment.
    pub(super) fn prove_trait(&mut self, goal: TraitRef) -> trait_solver::Proof {
        let mut solver = trait_solver::Solver::new(self.hir, &self.env, &mut self.proof_cache);
        solver.prove(goal, 0)
    }

    /// Returns the display name of a definition, for diagnostics.
    pub(super) fn def_name(&self, definition: DefId) -> &str {
        self.interner
            .resolve(self.hir.definitions().name(definition))
    }

    /// Checks a block as a value of `expected`, when given.
    pub(super) fn check_block(
        &mut self,
        block: HirBlockId,
        expected: Option<TyId>,
        flow: Flow,
    ) -> stmt::BlockOutcome {
        stmt::check_block(self, block, expected, flow)
    }
}
