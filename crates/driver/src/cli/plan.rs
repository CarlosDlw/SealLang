//! Resolving a project entry path into a definition.

use seal_def::{DefId, DefKind};
use seal_hir::{Hir, ModulePath};
use seal_session::Session;

/// Renders a module path with `::` separators, or `root`.
pub(crate) fn path_string(path: &ModulePath, interner: &seal_symbol::Interner) -> String {
    if path.is_root() {
        return "root".to_owned();
    }
    path.segments()
        .iter()
        .map(|segment| interner.resolve(*segment))
        .collect::<Vec<_>>()
        .join("::")
}

/// Resolves `module::function` to a top-level function in the primary package.
///
/// Reports `E2087` when the path is missing, names a module instead of a
/// function, or is ambiguous. Returns `None` after emitting the diagnostic.
pub(crate) fn resolve_entry(session: &Session, hir: &Hir, entry: &str) -> Option<DefId> {
    let package = hir.primary_package();
    let mut module = hir.package(package).root;
    let mut segments = entry.split("::").peekable();
    while let Some(segment) = segments.next() {
        // `root` names the package root, not a child module. It is legal at
        // the start of an entry path so `root::main` and `app::main` share one
        // spelling rule.
        if segment == "root" && module == hir.package(package).root && segments.peek().is_some() {
            continue;
        }
        let symbol = session.interner().intern(segment);
        if segments.peek().is_none() {
            let candidates: Vec<DefId> = hir
                .items_in_module(module)
                .into_iter()
                .filter(|definition| {
                    hir.definitions().get(*definition).kind == DefKind::Function
                        && hir.definitions().name(*definition) == symbol
                })
                .collect();
            if candidates.len() == 1 {
                return Some(candidates[0]);
            }
            let message = if candidates.is_empty() {
                format!("the entry `{entry}` does not name a function in the primary package")
            } else {
                format!("the entry `{entry}` is ambiguous")
            };
            session
                .diag()
                .error(message)
                .code(seal_error_codes::E2087.diag_code())
                .help("add `entry = \"module::function\"` to the target, or define the function")
                .emit();
            return None;
        }
        let Some(child) = hir.child_module(module, symbol) else {
            session
                .diag()
                .error(format!(
                    "the entry `{entry}` names the missing module `{}`",
                    path_string(&hir.module(module).path, session.interner())
                ))
                .code(seal_error_codes::E2087.diag_code())
                .help("check the module declarations in the target sources")
                .emit();
            return None;
        };
        module = child;
    }
    None
}
