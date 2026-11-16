//! Rendering types in source syntax.

use std::fmt;

use crate::ctxt::TyCtxt;
use crate::id::TyId;
use crate::kind::TyKind;

impl TyCtxt {
    /// Returns a display adapter that renders `ty` in Seal syntax.
    ///
    /// Inference variables render as `?T0`, `?T1`, and so on; callers that have
    /// an [`InferCtxt`](crate::InferCtxt) should resolve the type first so the
    /// diagnostic shows what the variable was bound to.
    pub fn display(&self, ty: TyId) -> TyDisplay<'_> {
        TyDisplay { ctxt: self, ty }
    }
}

/// A type rendered in Seal syntax.
#[derive(Clone, Copy, Debug)]
pub struct TyDisplay<'a> {
    ctxt: &'a TyCtxt,
    ty: TyId,
}

impl fmt::Display for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ctxt.kind(self.ty) {
            TyKind::Bool => f.write_str("bool"),
            TyKind::Char => f.write_str("char"),
            TyKind::Str => f.write_str("str"),
            TyKind::Int(int) => fmt::Display::fmt(&int, f),
            TyKind::Uint(uint) => fmt::Display::fmt(&uint, f),
            TyKind::Float(float) => fmt::Display::fmt(&float, f),
            TyKind::Ref(pointee) => write!(f, "*{}", self.ctxt.display(pointee)),
            TyKind::Unit => f.write_str("()"),
            TyKind::Never => f.write_str("!"),
            TyKind::Error => f.write_str("<error>"),
            TyKind::Infer(var) => write!(f, "?T{}", var.index()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::kind::{FloatTy, IntTy, UintTy};
    use crate::{InferCtxt, TyCtxt};

    #[test]
    fn scalars_render_as_written_in_the_source() {
        let ctxt = TyCtxt::new();
        let cases = [
            (ctxt.int_ty(IntTy::I32), "i32"),
            (ctxt.int_ty(IntTy::Isize), "isize"),
            (ctxt.uint_ty(UintTy::U8), "u8"),
            (ctxt.uint_ty(UintTy::Usize), "usize"),
            (ctxt.float_ty(FloatTy::F64), "f64"),
            (ctxt.bool_ty(), "bool"),
            (ctxt.char_ty(), "char"),
            (ctxt.str_ty(), "str"),
            (ctxt.unit_ty(), "()"),
            (ctxt.never_ty(), "!"),
            (ctxt.error_ty(), "<error>"),
        ];
        for (ty, expected) in cases {
            assert_eq!(ctxt.display(ty).to_string(), expected);
        }
    }

    #[test]
    fn references_nest_with_the_source_syntax() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let pointer = ctxt.ref_ty(int);
        assert_eq!(ctxt.display(pointer).to_string(), "*i32");
        let pointer = ctxt.ref_ty(pointer);
        assert_eq!(ctxt.display(pointer).to_string(), "**i32");
    }

    #[test]
    fn an_unresolved_variable_shows_its_index() {
        let mut ctxt = TyCtxt::new();
        let mut infer = InferCtxt::new();
        let first = infer.fresh_ty_var(&mut ctxt);
        let second = infer.fresh_ty_var(&mut ctxt);
        assert_eq!(ctxt.display(first).to_string(), "?T0");
        assert_eq!(ctxt.display(second).to_string(), "?T1");
    }
}
