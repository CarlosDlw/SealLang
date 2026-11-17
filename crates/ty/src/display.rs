//! Rendering types in source syntax.

use std::fmt;

use seal_symbol::Interner;

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
        TyDisplay {
            ctxt: self,
            ty,
            interner: None,
        }
    }

    /// Returns a display adapter that resolves aggregate names.
    ///
    /// Types are interned without their names, so rendering a struct as the
    /// user wrote it requires the interner that owns the symbol.
    pub fn display_with<'a>(&'a self, ty: TyId, interner: &'a Interner) -> TyDisplay<'a> {
        TyDisplay {
            ctxt: self,
            ty,
            interner: Some(interner),
        }
    }
}

/// A type rendered in Seal syntax.
#[derive(Clone, Copy, Debug)]
pub struct TyDisplay<'a> {
    ctxt: &'a TyCtxt,
    ty: TyId,
    interner: Option<&'a Interner>,
}

impl fmt::Display for TyDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ctxt.kind(self.ty) {
            TyKind::Bool => f.write_str("bool"),
            TyKind::Char => f.write_str("char"),
            TyKind::Str => f.write_str("str"),
            TyKind::Slice { element } => write!(f, "[{}]", self.display(element)),
            TyKind::Int(int) => fmt::Display::fmt(&int, f),
            TyKind::Uint(uint) => fmt::Display::fmt(&uint, f),
            TyKind::Float(float) => fmt::Display::fmt(&float, f),
            TyKind::Ref { kind, pointee } => {
                write!(f, "{}{}", kind.prefix(), self.display(pointee))
            }
            TyKind::Array { element, length } => {
                write!(f, "[{}; {length}]", self.display(element))
            }
            TyKind::Adt { adt, args } => {
                match self.interner {
                    Some(interner) => f.write_str(interner.resolve(self.ctxt.adt(adt).name))?,
                    None => write!(f, "<struct#{}>", adt.index())?,
                }
                let arguments = self.ctxt.substs_args(args);
                if !arguments.is_empty() {
                    f.write_str("<")?;
                    for (index, argument) in arguments.iter().enumerate() {
                        if index > 0 {
                            f.write_str(", ")?;
                        }
                        write!(f, "{}", self.display(*argument))?;
                    }
                    f.write_str(">")?;
                }
                Ok(())
            }
            TyKind::Param(param) => {
                let data = self.ctxt.param_data(param);
                match self.interner {
                    Some(interner) => f.write_str(interner.resolve(data.name)),
                    None => write!(f, "?P{}", data.index),
                }
            }
            TyKind::Tuple(tuple) => {
                let elements = self.ctxt.tuple_elements(tuple);
                f.write_str("(")?;
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", self.display(*element))?;
                }
                if elements.len() == 1 {
                    f.write_str(",")?;
                }
                f.write_str(")")
            }
            TyKind::Unit => f.write_str("()"),
            TyKind::Never => f.write_str("!"),
            TyKind::Error => f.write_str("<error>"),
            TyKind::Infer(var) => write!(f, "?T{}", var.index()),
        }
    }
}

impl TyDisplay<'_> {
    fn display(&self, ty: TyId) -> TyDisplay<'_> {
        TyDisplay {
            ctxt: self.ctxt,
            ty,
            interner: self.interner,
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
    fn exclusive_references_render_with_mut() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let exclusive = ctxt.mut_ref_ty(int);
        assert_eq!(ctxt.display(exclusive).to_string(), "*mut i32");
        let shared = ctxt.ref_ty(exclusive);
        assert_eq!(ctxt.display(shared).to_string(), "**mut i32");
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

    #[test]
    fn slices_render_with_their_element() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        assert_eq!(ctxt.display(ctxt.slice_ty(int)).to_string(), "[i32]");
        assert_eq!(
            ctxt.display(ctxt.ref_ty(ctxt.slice_ty(int))).to_string(),
            "*[i32]"
        );
    }

    #[test]
    fn tuples_render_with_source_syntax_and_a_trailing_comma() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        let bool_ty = ctxt.bool_ty();
        assert_eq!(
            ctxt.display(ctxt.tuple_ty(&[int, bool_ty])).to_string(),
            "(i32, bool)"
        );
        assert_eq!(ctxt.display(ctxt.tuple_ty(&[int])).to_string(), "(i32,)");
        assert_eq!(ctxt.display(ctxt.tuple_ty(&[])).to_string(), "()");
    }

    #[test]
    fn arrays_render_with_their_element_and_length() {
        let ctxt = TyCtxt::new();
        let int = ctxt.int_ty(IntTy::I32);
        assert_eq!(ctxt.display(ctxt.array_ty(int, 4)).to_string(), "[i32; 4]");
        assert_eq!(
            ctxt.display(ctxt.array_ty(ctxt.array_ty(int, 2), 3))
                .to_string(),
            "[[i32; 2]; 3]"
        );
    }
}
