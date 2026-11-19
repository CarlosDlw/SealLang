//! Type lowering.

use seal_ast::{TypeId, TypeKind};
use seal_error_codes as codes;
use seal_ty::{RawPtrKind, TyCtxt, TyId};

use super::Lowering;
use crate::primitives::primitive_kind;

impl Lowering<'_> {
    /// Lowers a type annotation into the shared type context.
    ///
    /// Every occurrence of the same type returns the same [`TyId`], so the
    /// checker can compare types by identity. Explicit type arguments are
    /// resolved here; an omitted argument list on a generic type is an arity
    /// error because an annotation has nothing to infer from.
    pub(super) fn lower_type(&mut self, ty: TypeId) -> TyId {
        self.lower_type_inner(ty, false)
    }

    /// Lowers a type, choosing the diagnostic for an unknown parameter name.
    ///
    /// Bound arguments use `E2054` because an unknown name there is most often a
    /// missing type parameter; ordinary annotations keep `E2002`.
    pub(super) fn lower_type_inner(&mut self, ty: TypeId, unknown_parameter: bool) -> TyId {
        match self.ast.ty(ty).kind.clone() {
            TypeKind::Primitive(primitive) => {
                let c_abi = self.hir.ty_ctxt().c_abi();
                self.hir
                    .ty_ctxt_mut()
                    .intern(primitive_kind(primitive, c_abi))
            }
            TypeKind::Pointer { pointee, mutable } => {
                let pointee = self.lower_type_inner(pointee, unknown_parameter);
                if mutable {
                    self.hir.ty_ctxt_mut().mut_ref_ty(pointee)
                } else {
                    self.hir.ty_ctxt_mut().ref_ty(pointee)
                }
            }
            TypeKind::RawPointer { pointee, mutable } => {
                let pointee = self.lower_type_inner(pointee, unknown_parameter);
                let kind = if mutable {
                    RawPtrKind::Mut
                } else {
                    RawPtrKind::Const
                };
                self.hir.ty_ctxt_mut().raw_ptr_ty_with(kind, pointee)
            }
            TypeKind::Array {
                element, length, ..
            } => {
                let element = self.lower_type_inner(element, unknown_parameter);
                self.hir.ty_ctxt_mut().array_ty(element, length)
            }
            TypeKind::Tuple(elements) => {
                let elements: Vec<TyId> = elements
                    .into_iter()
                    .map(|element| self.lower_type_inner(element, unknown_parameter))
                    .collect();
                self.hir.ty_ctxt_mut().tuple_ty(&elements)
            }
            TypeKind::Slice { element } => {
                let element = self.lower_type_inner(element, unknown_parameter);
                self.hir.ty_ctxt_mut().slice_ty(element)
            }
            TypeKind::Dyn(path) => {
                let span = self.ast.ty(ty).span;
                let Some((trait_def, args)) = self.lower_bound_trait_path(&path) else {
                    return self.hir.ty_ctxt_mut().error_ty();
                };
                let object = self.hir.ty_ctxt().dyn_ty(trait_def, args);
                self.hir
                    .insert_dyn_type(crate::DynTypeUse { ty: object, span });
                object
            }
            TypeKind::Path(path) if path.name == self.self_symbol => {
                if self.capture.is_some() {
                    let span = self.ast.ty(ty).span;
                    self.error(codes::E3060, span, "a local static cannot mention `Self`")
                        .help("write the concrete type, or move the declaration to file scope")
                        .emit();
                    return self.hir.ty_ctxt_mut().error_ty();
                }
                if let Some(args) = &path.args {
                    self.error(
                        codes::E2053,
                        args.span,
                        "`Self` does not take type arguments",
                    )
                    .help("remove the type argument list")
                    .emit();
                    for argument in &args.args {
                        let _ = self.lower_type(*argument);
                    }
                }
                if let Some(param) = self.generic_scope_param(path.name) {
                    return self.hir.ty_ctxt().param_ty(param);
                }
                let span = self.ast.ty(ty).span;
                match self.self_ty {
                    Some(self_ty) => self_ty,
                    None => {
                        self.error(
                            codes::E2059,
                            span,
                            "`Self` is only allowed in a trait or impl context",
                        )
                        .help("write the concrete type instead")
                        .emit();
                        self.hir.ty_ctxt_mut().error_ty()
                    }
                }
            }
            TypeKind::Path(path) => {
                let symbol = path.name;
                if let Some(param) = self.generic_scope_param(symbol) {
                    if self.capture.is_some() {
                        let span = self.ast.ty(ty).span;
                        self.error(
                            codes::E3060,
                            span,
                            format!(
                                "a local static cannot mention the generic parameter `{}`",
                                self.interner.resolve(symbol)
                            ),
                        )
                        .help("write a concrete type, or move the declaration to file scope")
                        .emit();
                        return self.hir.ty_ctxt_mut().error_ty();
                    }
                    if let Some(args) = &path.args {
                        self.error(
                            codes::E2053,
                            args.span,
                            format!(
                                "the type parameter `{}` does not take type arguments",
                                self.interner.resolve(symbol)
                            ),
                        )
                        .help("remove the type argument list")
                        .emit();
                        for argument in &args.args {
                            let _ = self.lower_type(*argument);
                        }
                    }
                    return self.hir.ty_ctxt().param_ty(param);
                }

                if let Some(definition) = self.types.get(&symbol).copied() {
                    let adt = self
                        .hir
                        .adt(definition)
                        .expect("every struct definition reserves an aggregate");
                    let expected = self.hir.ty_ctxt().adt_generic_count(adt) as usize;
                    let Some(args) = self.lower_type_arguments(
                        path.args.as_ref(),
                        expected,
                        false,
                        path.name_span,
                    ) else {
                        return self.hir.ty_ctxt_mut().error_ty();
                    };
                    return self.hir.ty_ctxt().adt_ty_with(adt, args);
                }
                if self.traits.contains_key(&symbol) {
                    if let Some(args) = &path.args {
                        for argument in &args.args {
                            let _ = self.lower_type(*argument);
                        }
                    }
                    let span = self.ast.ty(ty).span;
                    let name = self.interner.resolve(symbol);
                    self.error(
                        codes::E2027,
                        span,
                        format!("`{name}` is a trait, not a type"),
                    )
                    .help("name the struct that implements it, or call a method on a value")
                    .emit();
                    return self.hir.ty_ctxt_mut().error_ty();
                }

                let span = self.ast.ty(ty).span;
                let name = self.interner.resolve(symbol);
                if self.globals.contains_key(&symbol) {
                    self.error(
                        codes::E2011,
                        span,
                        format!("`{name}` is a value, not a type"),
                    )
                    .help("annotations accept primitive types, references, and struct names")
                    .emit();
                } else if self.capture.is_some()
                    && self.generic_parameter_declared_somewhere(symbol).is_some()
                {
                    self.error(
                        codes::E3060,
                        span,
                        format!("a local static cannot mention the generic parameter `{name}`"),
                    )
                    .help("write a concrete type, or move the declaration to file scope")
                    .emit();
                } else if let Some(first) = self.generic_parameter_declared_somewhere(symbol) {
                    self.error(
                        codes::E2055,
                        span,
                        format!("the type parameter `{name}` is not in scope here"),
                    )
                    .secondary(first, "declared here")
                    .help("use a parameter of the current declaration, or pass the type explicitly")
                    .emit();
                } else if unknown_parameter {
                    self.error(
                        codes::E2054,
                        span,
                        format!("cannot find type parameter `{name}` in this scope"),
                    )
                    .help("declare the parameter, or use a concrete type")
                    .emit();
                } else {
                    self.error(
                        codes::E2002,
                        span,
                        format!("cannot find type `{name}` in this scope"),
                    )
                    .help("declare it with `struct`, or use a built-in type")
                    .emit();
                }
                self.hir.ty_ctxt_mut().error_ty()
            }
            TypeKind::Error => self.hir.ty_ctxt_mut().error_ty(),
        }
    }
}

/// Interns every type the checker can create without mutating the context.
///
/// The checker does not own the type context, so lowering guarantees these
/// canonical types exist: all primitive scalars, `()`, `!`, the error type, and
/// the string-literal type `*str`. A later phase that introduces aggregate types
/// will extend this set or take the context mutably.
pub(crate) fn intern_primitives(ctxt: &mut TyCtxt) {
    let c_abi = ctxt.c_abi();
    for primitive in seal_ast::PrimitiveType::ALL {
        ctxt.intern(primitive_kind(primitive, c_abi));
    }
    ctxt.unit_ty();
    ctxt.never_ty();
    ctxt.error_ty();
    let str_ty = ctxt.str_ty();
    ctxt.ref_ty(str_ty);
}
