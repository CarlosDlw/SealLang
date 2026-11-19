//! Building borrow-check diagnostics.

use seal_def::DefId;
use seal_diagnostics::{DiagCtxt, ErrorBuilder};
use seal_error_codes as codes;
use seal_hir::Hir;
use seal_mir::{Body, Place, Projection};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::RefKind;

use crate::effects::{GlobalPath, GlobalStep};
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
                "cannot read `{name}` because {}",
                self.exclusive_borrow_phrase(&name, loan)
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
                "cannot assign to `{name}` because {}",
                self.exclusive_borrow_phrase(&name, loan)
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
                "cannot move out of `{name}` because {}",
                self.exclusive_borrow_phrase(&name, loan)
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

    /// An exclusive argument conflicts with a live loan of its pointee.
    pub(crate) fn exclusive_argument_borrowed(&self, argument: &Place, span: Span, loan: &Loan) {
        let name = self.place_text(argument);
        let builder = self
            .dcx
            .error(format!(
                "cannot pass `{name}` as an exclusive argument because it is borrowed"
            ))
            .code(codes::E5009.diag_code())
            .primary(span, "exclusive argument")
            .secondary(loan.span, "borrow occurs here");
        add_last_use(builder, loan, span)
            .note("an exclusive argument gives the callee sole access for the whole call")
            .help("move the call after the borrow's last use, or end the borrow first")
            .emit();
    }

    /// An exclusive argument may name the same storage as another argument.
    ///
    /// Positions are counted from one and include every argument, so they match
    /// what the user sees at the call. Two copies of one pointer render the
    /// same place twice, which reads better as "both name `p`" than as
    /// "`p` and `p` may name the same storage".
    pub(crate) fn arguments_alias(
        &self,
        span: Span,
        first: (&Place, Span, usize),
        second: (&Place, Span, usize),
    ) {
        let first_name = self.place_text(first.0);
        let second_name = self.place_text(second.0);
        let message = if first_name == second_name {
            format!(
                "arguments {} and {} both name `{first_name}`",
                first.2 + 1,
                second.2 + 1
            )
        } else {
            format!(
                "arguments {} (`{first_name}`) and {} (`{second_name}`) may name the same storage",
                first.2 + 1,
                second.2 + 1
            )
        };
        self.dcx
            .error(message)
            .code(codes::E5010.diag_code())
            .primary(span, "")
            .secondary(first.1, format!("argument {}", first.2 + 1))
            .secondary(second.1, format!("argument {}", second.2 + 1))
            .note("the callee may access the pointee through both arguments, and one is exclusive")
            .help("pass references to distinct places")
            .emit();
    }

    /// A move takes a field out of a value whose type implements `Drop`.
    pub(crate) fn move_out_of_drop_type(&self, ty: seal_ty::TyId, span: Span) {
        let ty = self
            .hir
            .ty_ctxt()
            .display_with(ty, self.interner)
            .to_string();
        self.dcx
            .error(format!(
                "cannot move out of `{ty}`, which implements `Drop`"
            ))
            .code(codes::E5014.diag_code())
            .primary(span, "")
            .help("borrow the field (`&value`), move the whole value, or drop it")
            .emit();
    }

    /// A move takes one element out of an array.
    pub(crate) fn move_out_of_array(&self, place: &Place, span: Span) {
        let name = self.place_text(place);
        self.dcx
            .error("cannot move an element out of an array")
            .code(codes::E5015.diag_code())
            .primary(span, format!("`{name}` would be moved out"))
            .help("borrow the element (`&values[i]`) or iterate by reference")
            .emit();
    }

    /// An assignment initializes one field of a value that needs `Drop`.
    pub(crate) fn initialize_field_of_drop_type(&self, place: &Place, span: Span) {
        let name = self.place_text(place);
        self.dcx
            .error("cannot initialize one field of a value that needs `Drop`")
            .code(codes::E5016.diag_code())
            .primary(span, format!("`{name}` is assigned here"))
            .help("build the whole value in one expression")
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
            .primary(span, "returned here")
            .secondary(loan.span, "borrow created here")
            .help("return a reference to a parameter, or to data the caller owns")
            .emit();
    }

    /// A call may read or write a mutable static the caller borrowed.
    pub(crate) fn call_global_conflict(
        &self,
        callee: seal_def::DefId,
        path: &GlobalPath,
        write: bool,
        span: Span,
        loan: &Loan,
    ) {
        let callee = self.interner.resolve(self.hir.definitions().name(callee));
        let path = self.global_path_text(path);
        let (code, message) = if write {
            (
                codes::E5013,
                format!(
                    "call to `{callee}` may write the mutable static `{path}` while it is borrowed"
                ),
            )
        } else {
            (
                codes::E5011,
                format!(
                    "call to `{callee}` may read the mutable static `{path}` while it is exclusively borrowed"
                ),
            )
        };
        let builder = self
            .dcx
            .error(message)
            .code(code.diag_code())
            .primary(span, "this call")
            .secondary(loan.span, "borrow occurs here")
            .note("the callee accesses the static directly or through another call");
        add_last_use(builder, loan, span).emit();
    }

    fn global_path_text(&self, path: &GlobalPath) -> String {
        let mut text = self
            .interner
            .resolve(self.hir.definitions().name(path.def))
            .to_owned();
        for step in &path.steps {
            match step {
                GlobalStep::Field(field) => {
                    text.push('.');
                    text.push_str(
                        self.interner
                            .resolve(self.hir.definitions().get(*field).name),
                    );
                }
                GlobalStep::TupleField(index) => text.push_str(&format!(".{index}")),
                GlobalStep::Index(index) => text.push_str(&format!("[{index}]")),
            }
        }
        text
    }

    /// A reference into a mutable static escapes its function.
    pub(crate) fn return_mutable_static_reference(&self, def: DefId, span: Span) {
        let name = self.interner.resolve(self.hir.definitions().name(def));
        self.dcx
            .error(format!(
                "cannot return a reference to the mutable static `{name}`"
            ))
            .code(codes::E5012.diag_code())
            .primary(span, "returned here")
            .help("use the reference only inside this function, and reborrow the static where it is needed")
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

    /// Explains which place an exclusive loan blocks.
    ///
    /// An access through a reference (`(*alias)`) names the storage it reaches,
    /// while the loan names the place the user borrowed; pointing at the loan's
    /// place is what makes the cause of the conflict obvious.
    fn exclusive_borrow_phrase(&self, access: &str, loan: &Loan) -> String {
        let borrowed = self.place_text(&loan.place);
        if borrowed == access {
            "it is exclusively borrowed".to_owned()
        } else {
            format!("`{borrowed}` is exclusively borrowed")
        }
    }

    /// Renders a place with source names, including field names.
    fn place_text(&self, place: &Place) -> String {
        let mut text = match place.root {
            seal_mir::PlaceRoot::Local(local) => self.name(local),
            seal_mir::PlaceRoot::Static(def) | seal_mir::PlaceRoot::Const(def) => self
                .interner
                .resolve(self.hir.definitions().name(def))
                .to_owned(),
        };
        for projection in &place.projections {
            match projection {
                Projection::Deref => {
                    text = format!("(*{text})");
                }
                Projection::Field(field) | Projection::VariantField { field, .. } => {
                    text.push('.');
                    text.push_str(
                        self.interner
                            .resolve(self.hir.definitions().get(*field).name),
                    );
                }
                Projection::TupleField(index) => text.push_str(&format!(".{index}")),
                Projection::Index {
                    constant: Some(index),
                    ..
                } => text.push_str(&format!("[{index}]")),
                Projection::Index { constant: None, .. } => text.push_str("[..]"),
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
