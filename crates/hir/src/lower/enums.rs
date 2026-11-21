//! Enum variant lowering.

use seal_ast::{ItemKind, VariantFieldList};
use seal_def::{DefData, DefKind};
use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_index::Idx;
use seal_ty::{FieldDef, VariantDef, VariantIdx};

use super::Lowering;
use crate::TraitMethodLookup;

impl Lowering<'_> {
    /// Lowers every enum's variants and their fields into its reserved
    /// aggregate.
    ///
    /// Variants are lowered after impl blocks are collected, so a variant whose
    /// name collides with an associated function can be reported here, with the
    /// function's declaration as the secondary location.
    pub(super) fn lower_enums(&mut self) {
        let items = self.enum_items.clone();
        for (item, definition) in items {
            self.enter_file(item.file);
            let item_id = item.id;
            let (declaration, span) = {
                let item = self.ast.item(item_id);
                let ItemKind::Enum(declaration) = &item.kind else {
                    continue;
                };
                (declaration.clone(), item.span)
            };

            let levels = self.push_generic_scope(definition);
            let mut variants = Vec::with_capacity(declaration.variants.len());
            let mut names = FxHashSet::default();
            for variant_id in &declaration.variants {
                let (name, name_span, field_list, variant_span, docs) = {
                    let variant = self.ast.variant(*variant_id);
                    (
                        variant.name,
                        variant.name_span,
                        variant.fields.clone(),
                        variant.span,
                        variant.docs,
                    )
                };
                if !names.insert(name) {
                    self.error(
                        codes::E2029,
                        name_span,
                        format!(
                            "the variant `{}` is declared twice",
                            self.interner.resolve(name)
                        ),
                    )
                    .secondary(span, "in this enum")
                    .help("rename or remove one of the variants")
                    .emit();
                    continue;
                }

                let variant_def = self.define(DefData::new(
                    DefKind::Variant,
                    name,
                    name_span,
                    variant_span,
                    Some(definition),
                    false,
                ));
                self.hir.definitions_mut().set_doc_span(variant_def, docs);

                let mut fields = Vec::new();
                match &field_list {
                    VariantFieldList::Unit => {}
                    VariantFieldList::Named(field_ids) => {
                        let mut field_names = FxHashSet::default();
                        for field_id in field_ids {
                            let (field_name, field_name_span, ty_id, field_span, docs) = {
                                let field = self.ast.field(*field_id);
                                (
                                    field.name,
                                    field.name_span,
                                    field.ty,
                                    field.span,
                                    field.docs,
                                )
                            };
                            let ty = self.lower_type(ty_id);
                            if !field_names.insert(field_name) {
                                self.error(
                                    codes::E2006,
                                    field_name_span,
                                    format!(
                                        "the field `{}` is declared twice",
                                        self.interner.resolve(field_name)
                                    ),
                                )
                                .secondary(variant_span, "in this variant")
                                .help("rename or remove one of the fields")
                                .emit();
                                continue;
                            }
                            let field_def = self.define(DefData::new(
                                DefKind::Field,
                                field_name,
                                field_name_span,
                                field_span,
                                Some(variant_def),
                                false,
                            ));
                            self.hir.definitions_mut().set_doc_span(field_def, docs);
                            fields.push(FieldDef {
                                def: field_def,
                                name: Some(field_name),
                                ty,
                            });
                        }
                    }
                    VariantFieldList::Tuple(field_ids) => {
                        for (index, field_id) in field_ids.iter().enumerate() {
                            let (field_ty, field_span) = {
                                let field = self.ast.tuple_field(*field_id);
                                (field.ty, field.span)
                            };
                            let ty = self.lower_type(field_ty);
                            let internal_name = self.interner.intern(&format!("#{index}"));
                            let field_def = self.define(DefData::new(
                                DefKind::Field,
                                internal_name,
                                field_span,
                                field_span,
                                Some(variant_def),
                                false,
                            ));
                            fields.push(FieldDef {
                                def: field_def,
                                name: None,
                                ty,
                            });
                        }
                    }
                }

                let index = VariantIdx::from_usize(variants.len());
                self.reject_associated_name_clash(definition, name, name_span);
                variants.push(VariantDef {
                    def: variant_def,
                    name,
                    fields,
                });
                self.hir.insert_variant(definition, name, index);
            }

            self.pop_generic_scope(levels);
            let adt = self
                .hir
                .adt(definition)
                .expect("every enum definition reserves an aggregate");
            self.hir.ty_ctxt().fill_variants(adt, variants);
        }
    }

    /// Rejects a variant whose name is also an associated function of its enum.
    ///
    /// `Enum::name` must denote exactly one thing: a variant or a callable
    /// associated function. The language rejects the collision instead of
    /// choosing by lookup order.
    fn reject_associated_name_clash(
        &mut self,
        enum_def: seal_def::DefId,
        name: seal_symbol::Symbol,
        name_span: seal_span::Span,
    ) {
        let function = match self.hir.method(enum_def, name) {
            Some(function) => Some(function),
            None => match self.hir.trait_method(enum_def, name) {
                TraitMethodLookup::Found { function, .. }
                | TraitMethodLookup::Default { function, .. } => Some(function),
                TraitMethodLookup::Ambiguous(_) | TraitMethodLookup::Missing => None,
            },
        };
        let Some(function) = function else {
            return;
        };
        let function_span = self.hir.definitions().get(function).name_span;
        self.error(
            codes::E2030,
            name_span,
            format!(
                "the variant `{}` has the same name as an associated function",
                self.interner.resolve(name)
            ),
        )
        .secondary(function_span, "the associated function is declared here")
        .help("rename the variant or the function; `Type::name` must mean one of them")
        .emit();
    }
}
