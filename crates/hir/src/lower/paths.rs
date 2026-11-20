//! Path resolution shared by impl headers, types, and expressions.

use seal_ast::{PathSegment, TraitPath};
use seal_def::ModuleId;
use seal_error_codes as codes;
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{TraitRef, TyId};

use super::Lowering;

/// The last segment of a resolved module path.
#[derive(Clone, PartialEq)]
pub(super) struct ResolvedSegment {
    pub(super) name: Symbol,
    pub(super) name_span: Span,
    pub(super) args: Option<seal_ast::GenericArgs>,
}

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
        let Some(trait_def) = self.trait_def(path.name) else {
            let name = self.interner.resolve(path.name);
            if self.type_exists(path.name) {
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
                    let _ = self.lower_type_inner(*argument, true, false);
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

    /// Resolves the module part of a multi-segment path.
    ///
    /// The first segment is the package root (`root`), a top-level module of
    /// the current package, a dependency alias, or a module imported by the
    /// current file. Each following segment must name a child module; the last
    /// segment is returned for the caller to resolve in its namespace.
    ///
    /// Returns `None` after reporting why the path does not resolve.
    pub(super) fn resolve_module_path(
        &mut self,
        root: Symbol,
        root_span: Span,
        segments: &[PathSegment],
    ) -> Option<(ModuleId, Option<ResolvedSegment>)> {
        let mut current = self.resolve_root_module(root, root_span)?;
        let Some((last, prefix)) = segments.split_last() else {
            return Some((current, None));
        };
        for segment in prefix {
            let Some(child) = self.hir.child_module(current, segment.name) else {
                self.error(
                    codes::E2070,
                    segment.name_span,
                    format!(
                        "cannot find the module `{}`",
                        self.interner.resolve(segment.name)
                    ),
                )
                .help("check the path, or declare the module with `module ...;`")
                .emit();
                return None;
            };
            if !self.module_reachable(child, segment.name_span) {
                return None;
            }
            current = child;
        }
        Some((
            current,
            Some(ResolvedSegment {
                name: last.name,
                name_span: last.name_span,
                args: last.args.clone(),
            }),
        ))
    }

    /// Returns the module named by `name` in the current file, without
    /// reporting an error or marking an import used.
    pub(super) fn module_named(&self, name: Symbol) -> Option<ModuleId> {
        let package = self.hir.package(self.current_package);
        if let Some(module) = self.hir.child_module(package.root, name) {
            return Some(module);
        }
        if let Some(table) = self.imports.get(self.current_file)
            && let Some(crate::module::ImportTarget::Module { module }) =
                table.get(name).map(|binding| binding.target)
        {
            return Some(module);
        }
        package
            .dependencies
            .iter()
            .find(|dependency| dependency.alias == name)
            .map(|dependency| self.hir.package(dependency.package).root)
    }

    /// Resolves the first segment of a path to a module.
    pub(super) fn resolve_root_module(&mut self, root: Symbol, span: Span) -> Option<ModuleId> {
        let package = self.current_package;
        let root_module = self.hir.package(package).root;
        if self.interner.resolve(root) == "root" {
            return Some(root_module);
        }
        if let Some(module) = self.hir.child_module(root_module, root) {
            return self.module_if_reachable(module, span);
        }
        if let Some(module) = self.imported_module(root) {
            return Some(module);
        }
        let dependency = self
            .hir
            .package(package)
            .dependencies
            .iter()
            .find(|dependency| dependency.alias == root)
            .map(|dependency| dependency.package);
        if let Some(dependency) = dependency {
            let module = self.hir.package(dependency).root;
            return self.module_if_reachable(module, span);
        }
        self.error(
            codes::E2070,
            span,
            format!(
                "cannot find the module `{}` in this package",
                self.interner.resolve(root)
            ),
        )
        .help("name a top-level module, a dependency alias, or `root`")
        .emit();
        None
    }

    fn module_if_reachable(&mut self, module: ModuleId, span: Span) -> Option<ModuleId> {
        self.module_reachable(module, span).then_some(module)
    }

    fn module_reachable(&mut self, module: ModuleId, span: Span) -> bool {
        if self.hir.module(module).package == self.current_package
            || self.hir.module(module).exposed
        {
            return true;
        }
        self.error(
            codes::E2071,
            span,
            format!(
                "the module `{}` is private to its package",
                self.hir.module(module).path.display(self.interner)
            ),
        )
        .help("expose the module in `[package.api]`, or keep the path inside its package")
        .emit();
        false
    }

    /// Reports an item that is not visible from the current module.
    pub(super) fn require_visible(&mut self, def: seal_def::DefId, span: Span) -> bool {
        if self.hir.is_visible_from(def, self.current_module) {
            return true;
        }
        let visibility = self.hir.definitions().visibility(def);
        let help = if visibility == seal_def::Visibility::Public {
            "make the module part of the package API, or use the item inside the package"
        } else {
            "make the item `internal` or `public`, or keep the use in a module that can see it"
        };
        self.error(
            codes::E2072,
            span,
            format!(
                "the item is `{}` and is not visible here",
                visibility.as_str()
            ),
        )
        .help(help)
        .emit();
        false
    }
}
