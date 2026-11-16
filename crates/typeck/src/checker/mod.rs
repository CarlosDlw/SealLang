//! The checker context and per-body driver.

mod borrow;
mod call;
mod expr;
mod flow;
mod literal;
mod operators;
mod place;
mod slice;
mod stmt;
mod types;

use seal_def::DefId;
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_error_codes::ErrorCode;
use seal_hir::{Body, Hir, HirBlockId};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{TyId, TyKind};

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
    pub(super) ret: TyId,
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
            ret: unit,
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
            self.results
                .insert_signature(body.owner, Signature { params, ret });
        }
    }

    fn check_body(&mut self, body: &Body) {
        let signature = self
            .results
            .signature(body.owner)
            .expect("every body receives a signature")
            .clone();
        self.ret = signature.ret;

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

    /// Reports a type mismatch unless one side already absorbed the other.
    pub(super) fn mismatch(&self, span: Span, expected: TyId, found: TyId) {
        if self.is_compatible(expected, found) {
            return;
        }
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
