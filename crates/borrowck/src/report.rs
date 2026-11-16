//! Building borrow-check diagnostics.

use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hir::Hir;
use seal_mir::{Body, Place, Projection};
use seal_span::Span;
use seal_symbol::Interner;

use crate::loan::Loan;

/// Emits `E5xxx` diagnostics for one body.
pub(crate) struct Reporter<'a> {
    hir: &'a Hir,
    dcx: &'a DiagCtxt,
    body: &'a Body,
    interner: &'a Interner,
}

impl<'a> Reporter<'a> {
    pub(crate) fn new(
        hir: &'a Hir,
        dcx: &'a DiagCtxt,
        body: &'a Body,
        interner: &'a Interner,
    ) -> Self {
        Self {
            hir,
            dcx,
            body,
            interner,
        }
    }

    /// A write conflicts with a live shared loan of the same place.
    pub(crate) fn assign_while_borrowed(&self, place: &Place, span: Span, loan: &Loan) {
        let name = self.place_text(place);
        self.dcx
            .error(format!("cannot assign to `{name}` because it is borrowed"))
            .code(codes::E5001.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow occurs here")
            .help("the borrow is still used after this point; end its use before assigning")
            .emit();
    }

    /// A move conflicts with a live shared loan of the same place.
    pub(crate) fn move_while_borrowed(&self, place: &Place, span: Span, loan: &Loan) {
        let name = self.place_text(place);
        self.dcx
            .error(format!(
                "cannot move out of `{name}` because it is borrowed"
            ))
            .code(codes::E5002.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow occurs here")
            .help("the borrow is still used after this point; end its use before moving")
            .emit();
    }

    /// A read reaches a place before every path initializes it.
    pub(crate) fn use_of_uninitialized(&self, place: &Place, span: Span) {
        let name = self.place_text(place);
        self.dcx
            .error(format!("`{name}` is possibly uninitialized here"))
            .code(codes::E5003.diag_code())
            .primary(span, "")
            .help("assign a value on every path before reading it")
            .emit();
    }

    /// A reference to storage owned by the callee escapes.
    pub(crate) fn return_local_reference(&self, loan: &Loan, span: Span) {
        let name = self.place_text(&loan.place);
        self.dcx
            .error(format!(
                "cannot return a reference to `{name}`, which is a local of this function"
            ))
            .code(codes::E5004.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow created here")
            .help("return a reference to a parameter, or to data the caller owns")
            .emit();
    }

    /// A read reaches a place whose value was moved out.
    pub(crate) fn use_of_moved_value(&self, place: &Place, span: Span) {
        let name = self.place_text(place);
        self.dcx
            .error(format!("`{name}` was moved and cannot be used here"))
            .code(codes::E5005.diag_code())
            .primary(span, "")
            .help("reassign the place before using it, or borrow instead of moving")
            .emit();
    }

    /// Whole-place borrow of a place with a moved field is not allowed.
    pub(crate) fn borrow_partially_moved(&self, place: &Place, span: Span) {
        let name = self.place_text(place);
        self.dcx
            .error(format!(
                "cannot borrow `{name}` because it is partially moved"
            ))
            .code(codes::E5006.diag_code())
            .primary(span, "")
            .help("borrow only the fields that are still initialized, or reinitialize the moved field")
            .emit();
    }

    /// Renders a place with source names, including field names.
    fn place_text(&self, place: &Place) -> String {
        let mut text = self.name(place.local);
        for projection in &place.projections {
            match projection {
                Projection::Deref => {
                    text = format!("(*{text})");
                }
                Projection::Field(field) => {
                    text.push('.');
                    text.push_str(
                        self.interner
                            .resolve(self.hir.definitions().get(*field).name),
                    );
                }
            }
        }
        text
    }

    /// Returns the source name of a local, or its index for compiler temporaries.
    fn name(&self, local: seal_mir::LocalId) -> String {
        self.body.local(local).name.map_or_else(
            || format!("_{}", seal_index::Idx::index(local)),
            |symbol| self.interner.resolve(symbol).to_owned(),
        )
    }
}
