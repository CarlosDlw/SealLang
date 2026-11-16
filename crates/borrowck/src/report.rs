//! Building borrow-check diagnostics.

use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_mir::{Body, LocalId};
use seal_span::Span;
use seal_symbol::Interner;

use crate::loan::Loan;

/// Emits `E5xxx` diagnostics for one body.
pub(crate) struct Reporter<'a> {
    dcx: &'a DiagCtxt,
    body: &'a Body,
    interner: &'a Interner,
}

impl<'a> Reporter<'a> {
    pub(crate) fn new(dcx: &'a DiagCtxt, body: &'a Body, interner: &'a Interner) -> Self {
        Self {
            dcx,
            body,
            interner,
        }
    }

    /// A write conflicts with a live shared loan of the same place.
    pub(crate) fn assign_while_borrowed(&self, local: LocalId, span: Span, loan: &Loan) {
        self.dcx
            .error(format!(
                "cannot assign to `{}` because it is borrowed",
                self.name(local)
            ))
            .code(codes::E5001.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow occurs here")
            .help("the borrow is still used after this point; end its use before assigning")
            .emit();
    }

    /// A move conflicts with a live shared loan of the same place.
    pub(crate) fn move_while_borrowed(&self, local: LocalId, span: Span, loan: &Loan) {
        self.dcx
            .error(format!(
                "cannot move out of `{}` because it is borrowed",
                self.name(local)
            ))
            .code(codes::E5002.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow occurs here")
            .help("the borrow is still used after this point; end its use before moving")
            .emit();
    }

    /// A read reaches a local before every path initializes it.
    pub(crate) fn use_of_uninitialized(&self, local: LocalId, span: Span) {
        self.dcx
            .error(format!(
                "`{}` is possibly uninitialized here",
                self.name(local)
            ))
            .code(codes::E5003.diag_code())
            .primary(span, "")
            .help("assign a value on every path before reading it")
            .emit();
    }

    /// A reference to storage owned by the callee escapes.
    pub(crate) fn return_local_reference(&self, loan: &Loan, span: Span) {
        self.dcx
            .error(format!(
                "cannot return a reference to `{}`, which is a local of this function",
                self.name(loan.place.local)
            ))
            .code(codes::E5004.diag_code())
            .primary(span, "")
            .secondary(loan.span, "borrow created here")
            .help("return a reference to a parameter, or to data the caller owns")
            .emit();
    }

    /// A read reaches a local whose value was moved out.
    pub(crate) fn use_of_moved_value(&self, local: LocalId, span: Span) {
        self.dcx
            .error(format!(
                "`{}` was moved and cannot be used here",
                self.name(local)
            ))
            .code(codes::E5005.diag_code())
            .primary(span, "")
            .help("reassign the place before using it, or borrow instead of moving")
            .emit();
    }

    /// Returns the source name of a local, or its index for compiler temporaries.
    fn name(&self, local: LocalId) -> String {
        self.body.local(local).name.map_or_else(
            || format!("_{}", local.index()),
            |symbol| self.interner.resolve(symbol).to_owned(),
        )
    }
}
