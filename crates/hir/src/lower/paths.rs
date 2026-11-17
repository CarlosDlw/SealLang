//! Trait paths and associated paths shared by impl headers and expressions.

use seal_ast::TraitPath;
use seal_error_codes as codes;
use seal_ty::{TraitRef, TyId};

use super::Lowering;

impl Lowering<'_> {
    /// Lowers the trait path of an `impl Trait for Type` header.
    ///
    /// Existing name-resolution diagnostics are preserved: an unknown trait is
    /// `E2020`, and a type used where a trait belongs has its own help text.
    pub(super) fn lower_impl_trait_ref(
        &mut self,
        path: &TraitPath,
        self_ty: TyId,
    ) -> Option<TraitRef> {
        let Some(trait_def) = self.traits.get(&path.name).copied() else {
            let name = self.interner.resolve(path.name);
            if self.types.contains_key(&path.name) {
                self.error(
                    codes::E2020,
                    path.name_span,
                    format!("`{name}` is a type, not a trait"),
                )
                .help("write `impl Type { ... }` for an inherent implementation")
                .emit();
            } else {
                self.error(
                    codes::E2020,
                    path.name_span,
                    format!("cannot find trait `{name}` for this impl"),
                )
                .help("declare the trait with `trait`, then implement it")
                .emit();
            }
            if let Some(args) = &path.args {
                for argument in &args.args {
                    let _ = self.lower_type_inner(*argument, true);
                }
            }
            return None;
        };

        let expected = self
            .hir
            .generics(trait_def)
            .map(|generics| generics.params.len() - 1)
            .unwrap_or(0);
        let args =
            self.lower_type_arguments(path.args.as_ref(), expected, false, path.name_span)?;
        Some(TraitRef::new(trait_def, self_ty, args))
    }

    /// Returns the number of own type parameters declared by `owner`.
    pub(super) fn own_generic_count(&self, owner: seal_def::DefId) -> usize {
        self.hir
            .generics(owner)
            .map(|generics| generics.params.len())
            .unwrap_or(0)
    }
}
