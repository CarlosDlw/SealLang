//! The checker context and per-body driver.

mod borrow;
mod call;
mod callable;
mod cast;
mod closure;
mod coerce;
mod const_context;
mod drop;
mod dynamic;
mod equality;
mod exhaustive;
mod expr;
mod ffi;
mod flow;
mod for_loop;
mod generics;
mod iteration;
mod literal;
mod method;
mod operators;
mod pattern;
mod place;
mod protocol;
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
use seal_hir::{Body, BodyKind, Hir, HirBlockId};
use seal_span::Span;
use seal_symbol::{Interner, Symbol};
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
    /// The place is borrowed exclusively, so it must be initialized and the
    /// borrow must be writable. Like [`Self::Borrow`], this does not move the
    /// place's value. Custom indexing selects `IndexMut` for this mode.
    BorrowExclusive,
}

impl PlaceMode {
    /// Returns whether the mode reads the selected storage, and therefore
    /// requires it to be initialized.
    pub(super) fn requires_initialized(self) -> bool {
        !matches!(self, Self::Write)
    }
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
    /// Trait references whose own type arguments have been checked for
    /// `Sized` storage. Impl methods share their impl's parameters, so a
    /// reference may be validated more than once without repeating errors.
    pub(super) validated_trait_arguments: FxHashSet<TraitRef>,
    /// The definition whose body is being checked, for its generic scope.
    pub(super) owner: Option<DefId>,
    /// The closure whose body is being checked, when inside one.
    pub(super) current_closure: Option<DefId>,
    /// Closures whose body has already been checked.
    pub(super) checked_closures: FxHashSet<DefId>,
    pub(super) ret: TyId,
    /// Non-`Copy` pattern bindings referenced by the guard being checked.
    ///
    /// A guard runs before the arm commits, so moving a binding into it could
    /// not be undone when the guard fails. The set starts with every non-`Copy`
    /// binding of the arm; a path removes its entry after reporting, so one
    /// diagnostic covers all uses of the same binding.
    pub(super) guard_moves: Option<FxHashSet<DefId>>,
    /// Whether the current expression is the base of a field or tuple-field
    /// projection. The base is not required to be initialized as a whole:
    /// only the selected sub-place is read or written.
    pub(super) checking_place_base: bool,
    /// Whether the body is a `const` or `static` initializer.
    pub(super) const_initializer: bool,
    /// Whether the body is a `const func`.
    pub(super) const_fn: bool,
    /// The number of enclosing `unsafe` blocks.
    pub(super) unsafe_depth: usize,
    /// Whether this body already reported a forbidden `*mut` type.
    pub(super) mut_ref_reported: bool,
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
            validated_trait_arguments: FxHashSet::default(),
            owner: None,
            current_closure: None,
            checked_closures: FxHashSet::default(),
            ret: unit,
            guard_moves: None,
            checking_place_base: false,
            const_initializer: false,
            const_fn: false,
            unsafe_depth: 0,
            mut_ref_reported: false,
        };
        checker.validate_bound_arguments();
        checker.build_signatures();
        checker.build_externs();
        checker
    }

    pub(crate) fn run(mut self) -> TypeckResults {
        let bodies: Vec<Body> = self.hir.bodies().iter().cloned().collect();
        for body in bodies {
            // A closure body is checked from its creating expression, where the
            // expected signature is known. Checking it here would run it twice.
            if body.kind == BodyKind::Closure {
                continue;
            }
            self.check_body(&body);
        }
        self.results
    }

    /// Builds every signature before any body is checked, so callers see the
    /// declared types regardless of declaration order.
    fn build_signatures(&mut self) {
        let bodies: Vec<Body> = self.hir.bodies().iter().cloned().collect();
        for body in bodies {
            // Closure signatures come from the expected type at the creating
            // expression, not from the body list.
            if body.kind == BodyKind::Closure {
                continue;
            }
            self.mut_ref_reported = false;
            let owner_span = self.hir.definitions().get(body.owner).span;
            let is_value = !matches!(body.kind, BodyKind::Function);
            let ret = match body.ret {
                Some(ty) => {
                    let ty = if is_value {
                        self.value_decl_ty(ty, owner_span, body.kind)
                    } else {
                        self.storage_ty(ty, owner_span, "return type")
                    };
                    if body.is_const || is_value {
                        self.note_declared_mut_ref(ty, owner_span, body.is_const);
                    }
                    ty
                }
                None => self.unit(),
            };
            let mut params = Vec::with_capacity(body.params.len());
            for param in &body.params {
                let span = self.hir.definitions().get(param.def).span;
                let ty = self.storage_ty(param.ty, span, "parameter");
                if body.is_const {
                    self.note_declared_mut_ref(ty, span, true);
                }
                params.push(ty);
            }
            self.results.insert_signature(
                body.owner,
                Signature {
                    params,
                    ret,
                    is_unsafe: body.is_unsafe,
                    generics: self.hir.flattened_generics(body.owner),
                },
            );
            if body.is_unsafe
                && matches!(body.kind, BodyKind::Function)
                && self.hir.definitions().get(body.owner).parent.is_none()
                && self
                    .interner
                    .resolve(self.hir.definitions().name(body.owner))
                    == "main"
            {
                let definition = self.hir.definitions().get(body.owner);
                self.error(
                    codes::E3073,
                    definition.name_span,
                    "the entry point `main` cannot be `unsafe`",
                )
                .help("make `main` safe and call an `unsafe func` inside an `unsafe { ... }` block")
                .emit();
            }
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
        self.const_fn = body.is_const;
        self.const_initializer = !matches!(body.kind, BodyKind::Function);
        self.unsafe_depth = 0;
        self.mut_ref_reported = (self.const_fn || self.const_initializer)
            && signature
                .params
                .iter()
                .chain(std::iter::once(&signature.ret))
                .any(|ty| self.contains_mut_ref(*ty));

        let mut flow = Flow::new();
        for (param, ty) in body.params.iter().zip(&signature.params) {
            self.results.record_local(param.def, *ty);
            flow.assign(param.def, &[]);
        }
        let _outcome = self.check_block(body.root, Some(self.ret), flow);
    }

    /// Reports a call that needs an enclosing `unsafe` block.
    ///
    /// The unsafe marker is part of the callee's signature, not of its body.
    /// Receiving the flag from the resolved call keeps every dispatch path
    /// (direct, inherent, trait, and dynamic) on the same rule.
    pub(super) fn require_unsafe_call(&mut self, span: Span, is_unsafe: bool, name: Symbol) {
        if !is_unsafe || self.unsafe_depth > 0 {
            return;
        }
        self.error(
            codes::E3070,
            span,
            format!(
                "calling `{}` requires an `unsafe` block",
                self.interner.resolve(name)
            ),
        )
        .help("wrap the call in `unsafe { ... }` and establish its preconditions")
        .emit();
    }

    /// Reports a call to an external function outside `unsafe`.
    ///
    /// The declaration, not only the signature, is what makes the call unsafe,
    /// so the diagnostic names the `extern` block explicitly.
    pub(super) fn require_unsafe_extern_call(&mut self, span: Span, is_unsafe: bool, name: Symbol) {
        if !is_unsafe || self.unsafe_depth > 0 {
            return;
        }
        self.error(
            codes::E3070,
            span,
            format!(
                "calling `{}` requires an `unsafe` block",
                self.interner.resolve(name)
            ),
        )
        .note("functions declared in an `extern` block are unsafe to call")
        .help("wrap the call in `unsafe { ... }` and establish its preconditions")
        .emit();
    }

    /// Reports a use of a C function pointer outside `unsafe`.
    ///
    /// Taking the address of a Seal function and converting an arbitrary raw
    /// pointer to a callable C pointer both cross the FFI boundary: the caller
    /// must establish that the resulting address is callable with the declared
    /// signature. Calling through the pointer is checked separately.
    pub(super) fn require_unsafe_function_pointer(&mut self, span: Span) {
        if self.unsafe_depth > 0 {
            return;
        }
        self.error(
            codes::E3089,
            span,
            "using a C function pointer requires an `unsafe` block",
        )
        .help("wrap the cast in `unsafe { ... }` and establish that the address is callable")
        .emit();
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
        .help(
            "use a reference, such as `*str`, `*[T]`, or `*dyn Trait`, to store the value indirectly",
        )
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
