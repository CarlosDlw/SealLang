//! The definition arena.

use seal_index::{Idx, IndexVec};
use seal_span::{FileId, Span};
use seal_symbol::Symbol;

use crate::id::{DefId, ModuleId, PackageId, PackageInstanceId};
use crate::kind::DefKind;
use crate::visibility::Visibility;

/// The source-level deprecation metadata attached to a declaration.
///
/// The value is syntax-independent: the parser fills it from the contextual
/// `deprecated(...)` modifier, and later attribute syntax can produce the same
/// value without changing every consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deprecation {
    /// The release in which the item was deprecated, such as `0.2.0`.
    pub since: Symbol,
    /// Optional guidance shown with a use-site warning.
    pub note: Option<Symbol>,
    /// Optional replacement the user should migrate to.
    pub replacement: Option<Symbol>,
    /// The span of the complete `deprecated(...)` modifier.
    pub span: Span,
}

/// A named definition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DefData {
    /// What the definition introduces.
    pub kind: DefKind,
    /// The declared name.
    pub name: Symbol,
    /// The span of the name, for diagnostics that point at it.
    pub name_span: Span,
    /// The span of the whole definition.
    pub span: Span,
    /// The enclosing definition: the function for a parameter, and the owning
    /// function for a binding. `None` for functions.
    pub parent: Option<DefId>,
    /// Whether the definition may be assigned to.
    ///
    /// This is only meaningful for bindings: functions are never assignable, and
    /// parameters are immutable in the current language.
    pub mutable: bool,
    /// The package that declares the definition.
    pub package: PackageId,
    /// The package instance that declares the definition.
    pub instance: PackageInstanceId,
    /// The module that declares the definition.
    pub module: ModuleId,
    /// The declared visibility.
    pub visibility: Visibility,
    /// The documentation text attached to the declaration, when the source
    /// carries one.
    pub docs: Option<Symbol>,
    /// The source range of the attached outer doc comments.
    pub doc_span: Option<Span>,
    /// The source `cfg(...)` condition, when the declaration has one.
    pub cfg: Option<Symbol>,
    /// Whether the function was declared with the contextual `test` modifier.
    pub is_test: bool,
    /// Whether the function was declared with the contextual `bench` modifier.
    pub is_bench: bool,
    /// Deprecation metadata attached to the declaration, when present.
    pub deprecated: Option<Deprecation>,
    /// The source file that declares the definition, when it has one.
    ///
    /// Built-in definitions have no source file. The file is kept on the
    /// definition so a later phase can tell a duplicate declared in one file
    /// from the same name declared by two fragments of one module.
    pub file: Option<FileId>,
}

impl DefData {
    /// Creates a definition in the root module of the root package.
    ///
    /// Lowering stamps the real package, instance, module, and file before the
    /// definition is pushed. The constructor keeps every caller honest about
    /// the semantic fields while leaving scope to one place.
    #[inline]
    pub fn new(
        kind: DefKind,
        name: Symbol,
        name_span: Span,
        span: Span,
        parent: Option<DefId>,
        mutable: bool,
    ) -> Self {
        Self {
            kind,
            name,
            name_span,
            span,
            parent,
            mutable,
            package: PackageId::from_usize(0),
            instance: PackageInstanceId::from_usize(0),
            module: ModuleId::from_usize(0),
            visibility: Visibility::Private,
            docs: None,
            doc_span: None,
            cfg: None,
            is_test: false,
            is_bench: false,
            deprecated: None,
            file: None,
        }
    }
}

/// The definitions of one compilation, in creation order.
#[derive(Clone, Debug, Default)]
pub struct Definitions {
    defs: IndexVec<DefId, DefData>,
}

impl Definitions {
    /// Creates an empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a definition and returns its ID.
    pub fn push(&mut self, data: DefData) -> DefId {
        self.defs.push(data)
    }

    /// Returns a definition.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this table. IDs from another table are a
    /// programming error, not user input.
    pub fn get(&self, id: DefId) -> &DefData {
        &self.defs[id]
    }

    /// Returns a definition for updating.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this table.
    pub fn get_mut(&mut self, id: DefId) -> &mut DefData {
        &mut self.defs[id]
    }

    /// Sets the scope fields of a definition.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this table.
    pub fn set_scope(
        &mut self,
        id: DefId,
        package: PackageId,
        instance: PackageInstanceId,
        module: ModuleId,
        file: Option<FileId>,
    ) {
        let data = &mut self.defs[id];
        data.package = package;
        data.instance = instance;
        data.module = module;
        data.file = file;
    }

    /// Sets the declared visibility of a definition.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this table.
    pub fn set_visibility(&mut self, id: DefId, visibility: Visibility) {
        self.defs[id].visibility = visibility;
    }

    /// Sets the documentation of a definition.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this table.
    pub fn set_docs(&mut self, id: DefId, docs: Option<Symbol>) {
        self.defs[id].docs = docs;
    }

    /// Records the source range of a declaration's doc comments.
    pub fn set_doc_span(&mut self, id: DefId, span: Option<Span>) {
        self.defs[id].doc_span = span;
    }

    /// Records the source `cfg(...)` condition of a definition.
    pub fn set_cfg(&mut self, id: DefId, cfg: Option<Symbol>) {
        self.defs[id].cfg = cfg;
    }

    /// Records whether a definition is a test entry point.
    pub fn set_test(&mut self, id: DefId, is_test: bool) {
        self.defs[id].is_test = is_test;
    }

    /// Records whether a definition is a benchmark entry point.
    pub fn set_bench(&mut self, id: DefId, is_bench: bool) {
        self.defs[id].is_bench = is_bench;
    }

    /// Records the deprecation metadata of a definition.
    pub fn set_deprecated(&mut self, id: DefId, deprecated: Option<Deprecation>) {
        self.defs[id].deprecated = deprecated;
    }

    /// Returns whether `id` is a test entry point.
    pub fn is_test(&self, id: DefId) -> bool {
        self.defs[id].is_test
    }

    /// Returns whether `id` is a benchmark entry point.
    pub fn is_bench(&self, id: DefId) -> bool {
        self.defs[id].is_bench
    }

    /// Returns the kind of `id`.
    pub fn kind(&self, id: DefId) -> DefKind {
        self.defs[id].kind
    }

    /// Returns the declared name of `id`.
    pub fn name(&self, id: DefId) -> Symbol {
        self.defs[id].name
    }

    /// Returns the package that declares `id`.
    pub fn package(&self, id: DefId) -> PackageId {
        self.defs[id].package
    }

    /// Returns the package instance that declares `id`.
    pub fn instance(&self, id: DefId) -> PackageInstanceId {
        self.defs[id].instance
    }

    /// Returns the module that declares `id`.
    pub fn module(&self, id: DefId) -> ModuleId {
        self.defs[id].module
    }

    /// Returns the declared visibility of `id`.
    pub fn visibility(&self, id: DefId) -> Visibility {
        self.defs[id].visibility
    }

    /// Returns the source file of `id`, if it has one.
    pub fn file(&self, id: DefId) -> Option<FileId> {
        self.defs[id].file
    }

    /// Returns `true` if `id` may be assigned to.
    pub fn is_mutable(&self, id: DefId) -> bool {
        self.defs[id].mutable
    }

    /// Returns the number of definitions.
    pub fn len(&self) -> usize {
        self.defs.len()
    }

    /// Returns `true` if the table holds no definitions.
    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// Iterates over the definitions.
    pub fn iter(&self) -> impl Iterator<Item = &DefData> + '_ {
        self.defs.iter()
    }

    /// Iterates over `(id, definition)` pairs.
    pub fn iter_enumerated(&self) -> impl Iterator<Item = (DefId, &DefData)> + '_ {
        self.defs.iter_enumerated()
    }
}

#[cfg(test)]
mod tests {
    use seal_index::Idx;
    use seal_span::Span;
    use seal_symbol::Interner;

    use super::{DefData, Definitions};
    use crate::{DefKind, ModuleId, PackageId, PackageInstanceId, Visibility};

    fn definition(name: &Interner, text: &str, kind: DefKind, mutable: bool) -> DefData {
        DefData::new(
            kind,
            name.intern(text),
            Span::default(),
            Span::default(),
            None,
            mutable,
        )
    }

    #[test]
    fn pushing_assigns_stable_ids() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        let first = definitions.push(definition(&interner, "first", DefKind::Function, false));
        let second = definitions.push(definition(&interner, "second", DefKind::Param, false));

        assert_eq!(first.index(), 0);
        assert_eq!(second.index(), 1);
        assert_eq!(definitions.len(), 2);
        assert!(!definitions.is_empty());
        assert_eq!(definitions.kind(first), DefKind::Function);
        assert_eq!(definitions.name(second), interner.intern("second"));
    }

    #[test]
    fn mutation_only_holds_for_bindings() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        let function = definitions.push(definition(&interner, "f", DefKind::Function, false));
        let parameter = definitions.push(definition(&interner, "p", DefKind::Param, false));
        let binding = definitions.push(definition(&interner, "b", DefKind::Binding, true));

        assert!(!definitions.is_mutable(function));
        assert!(!definitions.is_mutable(parameter));
        assert!(definitions.is_mutable(binding));
    }

    #[test]
    fn iteration_is_in_creation_order() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        for name in ["a", "b", "c"] {
            definitions.push(definition(&interner, name, DefKind::Function, false));
        }
        let names: Vec<u32> = definitions
            .iter_enumerated()
            .map(|(id, _)| id.index())
            .collect();
        assert_eq!(names, [0, 1, 2]);
        assert_eq!(definitions.iter().count(), 3);
    }

    #[test]
    fn scope_fields_start_at_the_root_and_can_be_replaced() {
        let interner = Interner::new();
        let mut definitions = Definitions::new();
        let def = definitions.push(definition(&interner, "f", DefKind::Function, false));

        assert_eq!(definitions.package(def), PackageId::from_usize(0));
        assert_eq!(definitions.instance(def), PackageInstanceId::from_usize(0));
        assert_eq!(definitions.module(def), ModuleId::from_usize(0));
        assert_eq!(definitions.visibility(def), Visibility::Private);

        let package = PackageId::from_usize(1);
        let instance = PackageInstanceId::from_usize(2);
        let module = ModuleId::from_usize(3);
        definitions.set_scope(def, package, instance, module, None);
        definitions.set_visibility(def, Visibility::Internal);

        assert_eq!(definitions.package(def), package);
        assert_eq!(definitions.instance(def), instance);
        assert_eq!(definitions.module(def), module);
        assert_eq!(definitions.visibility(def), Visibility::Internal);
        assert_eq!(definitions.file(def), None);
    }
}
