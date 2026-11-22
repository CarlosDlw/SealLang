//! Type lowering.

use seal_ast::{TypeId, TypeKind};
use seal_error_codes as codes;
use seal_ty::{Abi, FnSigDef, RawPtrKind, TyCtxt, TyId};

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
        self.lower_type_inner(ty, false, false)
    }

    /// Lowers a type, choosing the diagnostic for an unknown parameter name.
    ///
    /// Bound arguments use `E2054` because an unknown name there is most often a
    /// missing type parameter; ordinary annotations keep `E2002`.
    pub(super) fn lower_type_inner(
        &mut self,
        ty: TypeId,
        unknown_parameter: bool,
        behind_raw_pointer: bool,
    ) -> TyId {
        match self.ast.ty(ty).kind.clone() {
            TypeKind::Primitive(primitive) => {
                let c_abi = self.hir.ty_ctxt().c_abi();
                self.hir
                    .ty_ctxt_mut()
                    .intern(primitive_kind(primitive, c_abi))
            }
            TypeKind::Pointer { pointee, mutable } => {
                let pointee = self.lower_type_inner(pointee, unknown_parameter, false);
                if mutable {
                    self.hir.ty_ctxt_mut().mut_ref_ty(pointee)
                } else {
                    self.hir.ty_ctxt_mut().ref_ty(pointee)
                }
            }
            TypeKind::RawPointer { pointee, mutable } => {
                let pointee = self.lower_type_inner(pointee, unknown_parameter, true);
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
                let element = self.lower_type_inner(element, unknown_parameter, false);
                self.hir.ty_ctxt_mut().array_ty(element, length)
            }
            TypeKind::PathMulti {
                root,
                root_span,
                args,
                segments,
            } => {
                let span = self.ast.ty(ty).span;
                let Some((module, Some(last))) =
                    self.resolve_module_path(root, root_span, &segments)
                else {
                    return self.hir.ty_ctxt_mut().error_ty();
                };
                if let Some((definition, export)) =
                    self.module_item(module, last.name, crate::module::Namespace::Type)
                {
                    if !self.export_reachable(export, last.name_span) {
                        return self.hir.ty_ctxt_mut().error_ty();
                    }
                    if !self.require_visible(definition, last.name_span) {
                        return self.hir.ty_ctxt_mut().error_ty();
                    }
                    let generic_args = last.args.as_ref().or(args.as_ref());
                    return self.lower_type_definition(
                        definition,
                        generic_args,
                        last.name_span,
                        span,
                        behind_raw_pointer,
                    );
                }
                let name = self.interner.resolve(last.name);
                if self
                    .module_item(module, last.name, crate::module::Namespace::Trait)
                    .is_some()
                {
                    self.error(
                        codes::E2027,
                        span,
                        format!("`{name}` is a trait, not a type"),
                    )
                    .help("name the struct that implements it, or call a method on a value")
                    .emit();
                } else if self
                    .module_item(module, last.name, crate::module::Namespace::Value)
                    .is_some()
                {
                    self.error(
                        codes::E2011,
                        span,
                        format!("`{name}` is a value, not a type"),
                    )
                    .help("annotations accept primitive types, references, and struct names")
                    .emit();
                } else {
                    self.error(
                        codes::E2002,
                        span,
                        format!("cannot find type `{name}` in this module"),
                    )
                    .help("declare it with `struct`, or check the path")
                    .emit();
                }
                self.hir.ty_ctxt_mut().error_ty()
            }
            TypeKind::Tuple(elements) => {
                let elements: Vec<TyId> = elements
                    .into_iter()
                    .map(|element| self.lower_type_inner(element, unknown_parameter, false))
                    .collect();
                self.hir.ty_ctxt_mut().tuple_ty(&elements)
            }
            TypeKind::Slice { element } => {
                let element = self.lower_type_inner(element, unknown_parameter, false);
                self.hir.ty_ctxt_mut().slice_ty(element)
            }
            TypeKind::FnSig {
                abi,
                abi_span,
                params,
                ret,
                variadic,
                ..
            } => {
                let span = self.ast.ty(ty).span;
                if !behind_raw_pointer {
                    self.error(
                        codes::E3089,
                        span,
                        "a C function signature must appear behind `*raw`",
                    )
                    .help("write `*raw extern \"c\" func(...)` for a callable C function pointer")
                    .emit();
                }
                let abi_name = self.interner.resolve(abi);
                let abi = match Abi::from_name(abi_name) {
                    Some(abi) => abi,
                    None => {
                        self.error(
                            codes::E3080,
                            abi_span,
                            format!("unknown or unavailable ABI `{abi_name}`"),
                        )
                        .help("available conventions are `\"c\"`, `\"cdecl\"`, and `\"system\"`")
                        .emit();
                        Abi::C
                    }
                };
                let params: Vec<TyId> = params
                    .into_iter()
                    .map(|parameter| self.lower_type_inner(parameter, unknown_parameter, false))
                    .collect();
                let ret = match ret {
                    Some(ret) => self.lower_type_inner(ret, unknown_parameter, false),
                    None => self.hir.ty_ctxt().unit_ty(),
                };
                let signature = self.hir.ty_ctxt().fn_sig_ty(FnSigDef {
                    abi,
                    params,
                    ret,
                    variadic,
                });
                self.hir.insert_fn_sig(crate::FnSigUse {
                    ty: signature,
                    span,
                });
                signature
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

                if let Some(definition) = self.type_def(symbol) {
                    let span = self.ast.ty(ty).span;
                    return self.lower_type_definition(
                        definition,
                        path.args.as_ref(),
                        path.name_span,
                        span,
                        behind_raw_pointer,
                    );
                }
                if self.trait_exists(symbol) {
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

                if self.import_failed(symbol) {
                    return self.hir.ty_ctxt_mut().error_ty();
                }
                let span = self.ast.ty(ty).span;
                let name = self.interner.resolve(symbol);
                if self.value_exists(symbol) {
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
            TypeKind::FnPtr {
                params,
                ret,
                is_unsafe,
            } => {
                let mut lowered = Vec::with_capacity(params.len());
                for parameter in params {
                    let span = self.ast.ty(parameter).span;
                    let parameter = self.lower_type_inner(parameter, unknown_parameter, false);
                    self.require_sized(parameter, span, "a function pointer parameter");
                    lowered.push(parameter);
                }
                let ret = match ret {
                    Some(ret) => {
                        let span = self.ast.ty(ret).span;
                        let ret = self.lower_type_inner(ret, unknown_parameter, false);
                        self.require_sized(ret, span, "a function pointer result");
                        ret
                    }
                    None => self.hir.ty_ctxt().unit_ty(),
                };
                self.hir.ty_ctxt().fn_ptr_ty(seal_ty::FnPtrDef {
                    params: lowered,
                    ret,
                    is_unsafe,
                })
            }
            TypeKind::Error => self.hir.ty_ctxt_mut().error_ty(),
        }
    }

    /// Lowers a type name that resolved to `definition`.
    fn lower_type_definition(
        &mut self,
        definition: seal_def::DefId,
        args: Option<&seal_ast::GenericArgs>,
        name_span: seal_span::Span,
        span: seal_span::Span,
        behind_raw_pointer: bool,
    ) -> TyId {
        if self.hir.definitions().kind(definition) == seal_def::DefKind::OpaqueType {
            let name = self.hir.definitions().name(definition);
            let opaque = self.hir.ty_ctxt().opaque_ty(definition, name);
            if behind_raw_pointer {
                return opaque;
            }
            let display = self.interner.resolve(name).to_owned();
            self.error(
                codes::E3087,
                span,
                format!("the opaque type `{display}` can only appear behind a raw pointer"),
            )
            .help(format!(
                "write `*raw {display}` or `*raw mut {display}` and let C own the storage"
            ))
            .emit();
            return self.hir.ty_ctxt_mut().error_ty();
        }
        let adt = self
            .hir
            .adt(definition)
            .expect("every struct definition reserves an aggregate");
        let expected = self.hir.ty_ctxt().adt_generic_count(adt) as usize;
        let Some(args) = self.lower_type_arguments(args, expected, false, name_span) else {
            return self.hir.ty_ctxt_mut().error_ty();
        };
        self.hir.ty_ctxt().adt_ty_with(adt, args)
    }

    /// Reports `ty` when a function pointer parameter or result is unsized.
    ///
    /// The signature describes calls, so every parameter and the result are
    /// passed by value and need a size fixed at compile time. The function
    /// pointer itself is always sized.
    fn require_sized(&mut self, ty: TyId, span: seal_span::Span, what: &str) {
        if self.hir.ty_ctxt().is_sized(ty) {
            return;
        }
        let text = self
            .hir
            .ty_ctxt()
            .display_with(ty, self.interner)
            .to_string();
        self.error(
            codes::E3010,
            span,
            format!("{what} cannot have the unsized type `{text}`"),
        )
        .help("pass a pointer, such as `*str` or `*[T]`, instead of the value")
        .emit();
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
