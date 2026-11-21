//! Struct declaration lowering.

use seal_ast::ItemKind;
use seal_def::{DefData, DefKind};
use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_ty::FieldDef;

use super::Lowering;

impl Lowering<'_> {
    /// Lowers every struct's fields into its reserved aggregate.
    pub(super) fn lower_structs(&mut self) {
        let items = self.struct_items.clone();
        for (item, definition) in items {
            self.enter_file(item.file);
            let item_id = item.id;
            let (declaration, span) = {
                let item = self.ast.item(item_id);
                let ItemKind::Struct(declaration) = &item.kind else {
                    continue;
                };
                (declaration.clone(), item.span)
            };

            let levels = self.push_generic_scope(definition);
            let mut fields = Vec::with_capacity(declaration.fields.len());
            let mut names = FxHashSet::default();
            for field_id in &declaration.fields {
                let (name, name_span, ty, field_span, visibility, docs) = {
                    let field = self.ast.field(*field_id);
                    (
                        field.name,
                        field.name_span,
                        field.ty,
                        field.span,
                        field.visibility,
                        field.docs,
                    )
                };
                let ty = self.lower_type(ty);
                if !names.insert(name) {
                    self.error(
                        codes::E2006,
                        name_span,
                        format!(
                            "the field `{}` is declared twice",
                            self.interner.resolve(name)
                        ),
                    )
                    .secondary(span, "in this struct")
                    .help("rename or remove one of the fields")
                    .emit();
                    continue;
                }
                let field_def = self.define_field(
                    visibility,
                    DefData::new(
                        DefKind::Field,
                        name,
                        name_span,
                        field_span,
                        Some(definition),
                        false,
                    ),
                );
                self.hir.definitions_mut().set_doc_span(field_def, docs);
                fields.push(FieldDef {
                    def: field_def,
                    name: Some(name),
                    ty,
                });
            }

            self.pop_generic_scope(levels);
            let adt = self
                .hir
                .adt(definition)
                .expect("every struct definition reserves an aggregate");
            self.hir.ty_ctxt().fill_adt(adt, fields);
        }
    }
}
