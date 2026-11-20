//! Collection and lowering of `extern` declarations.
//!
//! Declarations enter the value namespace like ordinary functions and statics,
//! so resolution and duplicate checks share one implementation. Their
//! signatures are lowered after structs and enums exist, because a parameter
//! may name any declared type.

use seal_ast::{GenericParams, ItemId, ItemKind};
use seal_def::{DefData, DefId, DefKind};
use seal_error_codes as codes;
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{Abi, TyId};

use super::Lowering;
use crate::body::ParamData;
use crate::extern_decl::{ExternFunction, ExternStatic, invalid_symbol};

/// One symbol's declared shape, used to detect conflicting declarations.
#[derive(Clone, PartialEq)]
enum ExternalShape {
    Function {
        abi: Abi,
        params: Vec<TyId>,
        ret: Option<TyId>,
        variadic: bool,
    },
    Static {
        abi: Abi,
        ty: TyId,
    },
}

impl Lowering<'_> {
    /// Registers every external declaration before any body is lowered.
    pub(super) fn collect_extern_items(&mut self) {
        let items = self.top_level_items();
        for item in items {
            self.enter_file(item.file);
            let item_id = item.id;
            match self.ast.item(item_id).kind.clone() {
                ItemKind::ExternFunc(declaration) => {
                    self.collect_extern_function(item_id, &declaration);
                }
                ItemKind::ExternStatic(declaration) => {
                    self.collect_extern_static(item_id, &declaration);
                }
                ItemKind::ExternBlock(block) => {
                    for link in &block.links {
                        self.hir
                            .push_link_directive(crate::extern_decl::LinkDirective {
                                name: link.name,
                                span: link.span,
                            });
                    }
                    for member in block.items {
                        match self.ast.item(member).kind.clone() {
                            ItemKind::ExternFunc(declaration) => {
                                self.collect_extern_function(member, &declaration);
                            }
                            ItemKind::ExternStatic(declaration) => {
                                self.collect_extern_static(member, &declaration);
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Lowers every external signature and detects conflicting symbols.
    pub(super) fn lower_extern_declarations(&mut self) {
        let functions = self.extern_function_items.clone();
        for (item, definition) in functions {
            self.enter_file(item.file);
            let item_id = item.id;
            let ItemKind::ExternFunc(declaration) = self.ast.item(item_id).kind.clone() else {
                continue;
            };
            let abi = self.abi_of(declaration.abi, declaration.abi_span);
            let (symbol, symbol_span) = self.extern_symbol(
                declaration.name,
                declaration.name_span,
                declaration.link_name,
            );

            let levels = self.push_generic_scope(definition);
            let params: Vec<ParamData> = declaration
                .params
                .iter()
                .map(|param| self.lower_extern_param(*param, definition))
                .collect();
            let ret = declaration.ret.map(|ty| self.lower_type(ty));
            self.pop_generic_scope(levels);

            self.hir.insert_extern_function(
                definition,
                ExternFunction {
                    abi,
                    symbol,
                    symbol_span,
                    variadic: declaration.variadic,
                    params,
                    ret,
                },
            );
        }

        let statics = self.extern_static_items.clone();
        for (item, definition) in statics {
            self.enter_file(item.file);
            let item_id = item.id;
            let ItemKind::ExternStatic(declaration) = self.ast.item(item_id).kind.clone() else {
                continue;
            };
            let abi = self.abi_of(declaration.abi, declaration.abi_span);
            let ty = self.lower_type(declaration.ty);
            let (symbol, symbol_span) =
                self.extern_symbol(declaration.name, declaration.name_span, None);
            self.hir.insert_extern_static(
                definition,
                ExternStatic {
                    abi,
                    symbol,
                    symbol_span,
                    ty,
                },
            );
        }

        self.validate_external_symbols();
    }

    /// Reports two declarations that use one symbol with different shapes.
    fn validate_external_symbols(&mut self) {
        let mut symbols: FxHashMap<Symbol, (DefId, Span, ExternalShape)> = FxHashMap::default();
        let functions = self.hir.extern_functions().to_vec();
        for definition in functions {
            let Some(function) = self.hir.extern_function(definition).cloned() else {
                continue;
            };
            let shape = ExternalShape::Function {
                abi: function.abi,
                params: function.params.iter().map(|param| param.ty).collect(),
                ret: function.ret,
                variadic: function.variadic,
            };
            self.record_symbol(
                &mut symbols,
                definition,
                function.symbol,
                function.symbol_span,
                shape,
            );
        }
        let statics = self.hir.extern_statics().to_vec();
        for definition in statics {
            let Some(data) = self.hir.extern_static(definition).cloned() else {
                continue;
            };
            let shape = ExternalShape::Static {
                abi: data.abi,
                ty: data.ty,
            };
            self.record_symbol(
                &mut symbols,
                definition,
                data.symbol,
                data.symbol_span,
                shape,
            );
        }
    }

    fn record_symbol(
        &mut self,
        symbols: &mut FxHashMap<Symbol, (DefId, Span, ExternalShape)>,
        definition: DefId,
        symbol: Symbol,
        symbol_span: Span,
        shape: ExternalShape,
    ) {
        if let Some((first, first_span, first_shape)) = symbols.get(&symbol) {
            if *first_shape != shape {
                let name = self.interner.resolve(symbol).to_owned();
                self.error(
                    codes::E3083,
                    symbol_span,
                    format!("the symbol `{name}` is declared with two different types"),
                )
                .secondary(*first_span, "first declared here")
                .help("make both declarations use the same ABI and signature")
                .emit();
            }
            let _ = (definition, first);
            return;
        }
        symbols.insert(symbol, (definition, symbol_span, shape));
    }

    /// Resolves the ABI spelling of a declaration.
    fn abi_of(&mut self, spelling: Symbol, span: Span) -> Abi {
        let name = self.interner.resolve(spelling);
        match Abi::from_name(name) {
            Some(abi) => abi,
            None => {
                self.error(
                    codes::E3080,
                    span,
                    format!("unknown or unavailable ABI `{name}`"),
                )
                .help("available conventions are `\"c\"`, `\"cdecl\"`, and `\"system\"`")
                .emit();
                Abi::C
            }
        }
    }

    /// Returns the linker symbol and its span for one declaration.
    fn extern_symbol(
        &mut self,
        name: Symbol,
        name_span: Span,
        link_name: Option<seal_ast::LinkName>,
    ) -> (Symbol, Span) {
        let (symbol, span) = link_name.map_or((name, name_span), |link| (link.symbol, link.span));
        if let Some(reason) = invalid_symbol(self.interner.resolve(symbol)) {
            self.error(codes::E3082, span, reason)
                .help(
                    "choose a non-empty symbol without whitespace; `main` belongs to the entry point",
                )
                .emit();
        }
        (symbol, span)
    }

    /// Creates the definition for one external function declaration.
    fn collect_extern_function(&mut self, item_id: ItemId, declaration: &seal_ast::ExternFnDecl) {
        let definition = self.define_item(
            item_id,
            DefData::new(
                DefKind::ExternFunction,
                declaration.name,
                declaration.name_span,
                declaration.span,
                None,
                false,
            ),
        );
        self.register_generics(
            definition,
            &GenericParams::empty(declaration.span),
            None,
            false,
        );
        self.extern_function_items
            .push((self.item_ref(item_id), definition));

        if let Some(first) = self.local_value(declaration.name) {
            self.duplicate(
                first,
                declaration.name_span,
                format!(
                    "the function `{}` is defined twice",
                    self.interner.resolve(declaration.name)
                ),
                codes::E2001,
            );
        } else {
            self.globals
                .insert((self.current_module, declaration.name), definition);
        }
    }

    /// Creates the definition for one external static declaration.
    fn collect_extern_static(&mut self, item_id: ItemId, declaration: &seal_ast::ExternStaticDecl) {
        let definition = self.define_item(
            item_id,
            DefData::new(
                DefKind::ExternStatic,
                declaration.name,
                declaration.name_span,
                declaration.span,
                None,
                declaration.mutable,
            ),
        );
        self.register_generics(
            definition,
            &GenericParams::empty(declaration.span),
            None,
            false,
        );
        self.extern_static_items
            .push((self.item_ref(item_id), definition));

        if let Some(first) = self.local_value(declaration.name) {
            self.duplicate(
                first,
                declaration.name_span,
                format!(
                    "the value `{}` is defined twice",
                    self.interner.resolve(declaration.name)
                ),
                codes::E2068,
            );
        } else {
            self.globals
                .insert((self.current_module, declaration.name), definition);
        }
    }

    /// Lowers one parameter of an external function.
    fn lower_extern_param(&mut self, param: seal_ast::ParamId, function: DefId) -> ParamData {
        let (name, name_span, ty, span) = {
            let param = self.ast.param(param);
            (param.name, param.name_span, param.ty, param.span)
        };
        let ty = self.lower_type(ty);
        let definition = self.define(DefData::new(
            DefKind::Param,
            name,
            name_span,
            span,
            Some(function),
            false,
        ));
        ParamData {
            def: definition,
            ty,
        }
    }
}
