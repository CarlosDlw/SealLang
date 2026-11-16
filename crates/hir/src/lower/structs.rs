//! Struct declaration lowering and structural validation.

use seal_ast::ItemKind;
use seal_def::{DefData, DefKind};
use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_ty::{AdtId, FieldDef, TyKind};

use super::Lowering;

impl Lowering<'_> {
    /// Lowers every struct's fields into its reserved aggregate.
    pub(super) fn lower_structs(&mut self) {
        let items = self.struct_items.clone();
        for (item_id, definition) in items {
            let (declaration, span) = {
                let item = self.ast.item(item_id);
                let ItemKind::Struct(declaration) = &item.kind else {
                    continue;
                };
                (declaration.clone(), item.span)
            };

            let mut fields = Vec::with_capacity(declaration.fields.len());
            let mut names = FxHashSet::default();
            for field_id in &declaration.fields {
                let (name, name_span, ty, field_span) = {
                    let field = self.ast.field(*field_id);
                    (field.name, field.name_span, field.ty, field.span)
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
                let field_def = self.define(DefData {
                    kind: DefKind::Field,
                    name,
                    name_span,
                    span: field_span,
                    parent: Some(definition),
                    mutable: false,
                });
                fields.push(FieldDef {
                    def: field_def,
                    name,
                    ty,
                });
            }

            let adt = self
                .hir
                .adt(definition)
                .expect("every struct definition reserves an aggregate");
            self.hir.ty_ctxt().fill_adt(adt, fields);
        }
    }

    /// Rejects recursive-by-value and unsized fields, replacing each rejected
    /// field type with the error type so later phases do not cascade.
    pub(super) fn validate_structs(&mut self) {
        let adts: Vec<AdtId> = self
            .struct_items
            .iter()
            .filter_map(|(_, definition)| self.hir.adt(*definition))
            .collect();
        let mut visiting = Vec::new();
        let mut checked = FxHashSet::default();
        for adt in adts {
            self.check_struct(adt, &mut visiting, &mut checked);
        }
    }

    fn check_struct(
        &mut self,
        adt: AdtId,
        visiting: &mut Vec<AdtId>,
        checked: &mut FxHashSet<AdtId>,
    ) {
        if visiting.contains(&adt) || !checked.insert(adt) {
            return;
        }
        visiting.push(adt);

        let definition = self.hir.ty_ctxt().adt(adt);
        for (index, field) in definition.fields.iter().enumerate() {
            let field_span = self.hir.definitions().get(field.def).span;

            if let TyKind::Adt(inner) = self.hir.ty_ctxt().kind(field.ty) {
                if visiting.contains(&inner) {
                    self.error(
                        codes::E3016,
                        field_span,
                        "this field makes the type recursive by value",
                    )
                    .help("store the field behind a reference, such as `*Node`")
                    .emit();
                    let error = self.hir.ty_ctxt().error_ty();
                    self.hir.ty_ctxt().set_field_ty(adt, index, error);
                    continue;
                }
                self.check_struct(inner, visiting, checked);
            }

            if !self.hir.ty_ctxt().is_sized(field.ty) {
                let ty = self
                    .hir
                    .ty_ctxt()
                    .display_with(field.ty, self.interner)
                    .to_string();
                self.error(
                    codes::E3010,
                    field_span,
                    format!("a field cannot have the unsized type `{ty}`"),
                )
                .help("store the value behind a reference, such as `*str`")
                .emit();
                let error = self.hir.ty_ctxt().error_ty();
                self.hir.ty_ctxt().set_field_ty(adt, index, error);
            }
        }

        visiting.pop();
    }
}
