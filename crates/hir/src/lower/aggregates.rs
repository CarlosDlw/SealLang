//! Structural validation shared by structs and enums.
//!
//! Every field of every variant participates: a struct can contain an enum, an
//! enum variant can contain a struct, and recursion is only rejected when it
//! closes by value. A rejected field type is replaced with the error type so
//! later phases do not report the same mistake again.

use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_index::Idx;
use seal_ty::{AdtId, FieldDef, TyKind, VariantIdx};

use super::Lowering;

impl Lowering<'_> {
    /// Rejects recursive-by-value and unsized fields in every declared
    /// aggregate.
    pub(super) fn validate_aggregates(&mut self) {
        let adts: Vec<AdtId> = self
            .struct_items
            .iter()
            .chain(self.enum_items.iter())
            .filter_map(|(_, definition)| self.hir.adt(*definition))
            .collect();
        let mut visiting = Vec::new();
        let mut checked = FxHashSet::default();
        for adt in adts {
            self.check_aggregate(adt, &mut visiting, &mut checked);
        }
    }

    fn check_aggregate(
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
        if definition.is_struct() {
            self.check_fields(adt, None, &definition.fields, visiting, checked);
        } else {
            for (index, variant) in definition.variants.iter().enumerate() {
                let index = VariantIdx::from_usize(index);
                self.check_fields(adt, Some(index), &variant.fields, visiting, checked);
            }
        }

        visiting.pop();
    }

    fn check_fields(
        &mut self,
        adt: AdtId,
        variant: Option<VariantIdx>,
        fields: &[FieldDef],
        visiting: &mut Vec<AdtId>,
        checked: &mut FxHashSet<AdtId>,
    ) {
        for (index, field) in fields.iter().enumerate() {
            let field_span = self.hir.definitions().get(field.def).span;

            if let TyKind::Adt { adt: inner, .. } = self.hir.ty_ctxt().kind(field.ty) {
                if visiting.contains(&inner) {
                    self.error(
                        codes::E3016,
                        field_span,
                        "this field makes the type recursive by value",
                    )
                    .help("store the field behind a reference, such as `*Node`")
                    .emit();
                    let error = self.hir.ty_ctxt().error_ty();
                    self.replace_field(adt, variant, index, error);
                    continue;
                }
                self.check_aggregate(inner, visiting, checked);
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
                self.replace_field(adt, variant, index, error);
            }
        }
    }

    fn replace_field(
        &self,
        adt: AdtId,
        variant: Option<VariantIdx>,
        index: usize,
        ty: seal_ty::TyId,
    ) {
        match variant {
            Some(variant) => self
                .hir
                .ty_ctxt()
                .set_variant_field_ty(adt, variant, index, ty),
            None => self.hir.ty_ctxt().set_field_ty(adt, index, ty),
        }
    }
}
