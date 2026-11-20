//! Opaque type lowering.

use seal_ast::ItemKind;

use super::Lowering;

impl Lowering<'_> {
    /// Interns every declared opaque type under its definition.
    ///
    /// An opaque type has no fields or layout. Interning it once here keeps the
    /// identity nominal even before a declaration is used behind a pointer.
    pub(super) fn lower_opaque_types(&mut self) {
        let items = self.opaque_items.clone();
        for (item_id, definition) in items {
            let ItemKind::OpaqueType(declaration) = &self.ast.item(item_id).kind else {
                continue;
            };
            let _ = self.hir.ty_ctxt().opaque_ty(definition, declaration.name);
        }
    }
}
