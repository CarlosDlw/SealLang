//! Building borrow-check diagnostics.

use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_hir::Hir;
use seal_mir::{Body, Place, Projection};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::RefKind;

use crate::loan::Loan;

/// Emits `E5xxx` diagnostics for one body.
pub(crate) struct Reporter<'a> {
    hir: &'a Hir,
    dcx: &'a DiagCtxt,
    body: &'a Body,
    interner: &'a Interner,
}

/// Adds a label at the borrow's last active use, unless it is the conflict point
/// itself.
fn add_last_use<'a>(builder: ErrorBuilder<'a>, loan: &Loan, at: Span) -> ErrorBuilder<'a> {
    match loan.last_use.filter(|last_use| *last_use != at) {
        Some(last_use) => builder.secondary(last_use, "borrow is used here"),
        None => builder,
    }
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
        let builder = self
            .dcx
            .error(format!("cannot assign to `{name}` because it is borrowed"))
            .code(codes::E5001.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow occurs here");
        add_last_use(builder, loan, span)
            .help("move this assignment after the borrow's last use, or end the borrow first")
            .emit();
    }

    /// A move conflicts with a live shared loan of the same place.
    pub(crate) fn move_while_borrowed(&self, place: &Place, span: Span, loan: &Loan) {
        let name = self.place_text(place);
        let builder = self
            .dcx
            .error(format!(
                "cannot move out of `{name}` because it is borrowed"
            ))
            .code(codes::E5002.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow occurs here");
        add_last_use(builder, loan, span)
            .help("move this move after the borrow's last use, or end the borrow first")
            .emit();
    }

    /// A read conflicts with a live exclusive loan of the same place.
    pub(crate) fn read_while_exclusively_borrowed(&self, place: &Place, span: Span, loan: &Loan) {
        let name = self.place_text(place);
        let builder = self
            .dcx
            .error(format!(
                "cannot read `{name}` because it is exclusively borrowed"
            ))
            .code(codes::E5007.diag_code())
            .primary(span, "")
            .secondary(loan.span, "exclusive borrow occurs here");
        add_last_use(builder, loan, span)
            .help("read the place after the borrow's last use, or read through the reference")
            .emit();
    }

    /// A write conflicts with a live exclusive loan of the same place.
    pub(crate) fn write_while_exclusively_borrowed(&self, place: &Place, span: Span, loan: &Loan) {
        let name = self.place_text(place);
        let builder = self
            .dcx
            .error(format!(
                "cannot assign to `{name}` because it is exclusively borrowed"
            ))
            .code(codes::E5008.diag_code())
            .primary(span, "")
            .secondary(loan.span, "exclusive borrow occurs here");
        add_last_use(builder, loan, span)
            .help("write after the borrow's last use, or end the borrow first")
            .emit();
    }

    /// A move conflicts with a live exclusive loan of the same place.
    pub(crate) fn move_while_exclusively_borrowed(&self, place: &Place, span: Span, loan: &Loan) {
        let name = self.place_text(place);
        let builder = self
            .dcx
            .error(format!(
                "cannot move out of `{name}` because it is exclusively borrowed"
            ))
            .code(codes::E5008.diag_code())
            .primary(span, "")
            .secondary(loan.span, "exclusive borrow occurs here");
        add_last_use(builder, loan, span)
            .help("move the value after the borrow's last use, or end the borrow first")
            .emit();
    }

    /// A new borrow conflicts with a live loan of the same place.
    ///
    /// Shared loans coexist; every other combination is `E5008`. Two shared
    /// loans never reach this method, because the caller only calls it when one
    /// side is exclusive.
    pub(crate) fn borrow_conflict(&self, place: &Place, kind: RefKind, span: Span, loan: &Loan) {
        let name = self.place_text(place);
        let message = if kind.is_exclusive() {
            format!("cannot borrow `{name}` as mutable because it is already borrowed")
        } else {
            format!("cannot borrow `{name}` as shared because it is already borrowed as mutable")
        };
        let builder = self
            .dcx
            .error(message)
            .code(codes::E5008.diag_code())
            .primary(span, "")
            .secondary(loan.span, "conflicting borrow occurs here");
        add_last_use(builder, loan, span)
            .help("end the conflicting borrow before creating this one")
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
    ///
    /// `moved` may be a prefix of `used`: reading `a.x` after `a` moved reports
    /// that `a` was moved, because that is the place the user has to reassign.
    pub(crate) fn use_of_moved_value(&self, used: &Place, moved: &Place, span: Span) {
        let used_name = self.place_text(used);
        let moved_name = self.place_text(moved);
        let message = if used_name == moved_name {
            format!("`{moved_name}` was moved and cannot be used here")
        } else {
            format!("cannot use `{used_name}` because `{moved_name}` was moved")
        };
        self.dcx
            .error(message)
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
