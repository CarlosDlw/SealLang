//! JSON API snapshots for `--emit=api`.
//!
//! The snapshot is intentionally data, not a stable ABI: it lists the package's
//! modules and public items so an IDE or a project tool can inspect a library
//! without loading the compiler again.

use std::path::Path;

use seal_def::DefKind;
use seal_hir::Hir;
use seal_session::Session;
use serde::Serialize;

use crate::plan::path_string;

/// One module in the snapshot.
#[derive(Serialize)]
struct ApiModule {
    path: String,
    exposed: bool,
    package: usize,
}

/// One public item in the snapshot.
#[derive(Serialize)]
struct ApiItem {
    name: String,
    kind: &'static str,
    module: String,
    visibility: String,
}

/// The complete snapshot.
#[derive(Serialize)]
struct Snapshot {
    version: u32,
    target: String,
    package: String,
    modules: Vec<ApiModule>,
    items: Vec<ApiItem>,
}

/// Writes the API snapshot for `hir` to `output`.
pub(crate) fn write(session: &Session, hir: &Hir, output: &Path) -> Result<(), String> {
    let modules = hir
        .modules()
        .iter()
        .map(|module| ApiModule {
            path: path_string(&module.path, session.interner()),
            exposed: module.exposed,
            package: module.package.index() as usize,
        })
        .collect();
    let mut items: Vec<ApiItem> = hir
        .definitions()
        .iter_enumerated()
        .filter(|(_, definition)| definition.parent.is_none())
        .filter(|(definition, _)| hir.is_public_api(*definition))
        .map(|(_definition, data)| ApiItem {
            name: session.interner().resolve(data.name).to_owned(),
            kind: kind_name(data.kind),
            module: path_string(&hir.module(data.module).path, session.interner()),
            visibility: format!("{:?}", data.visibility).to_lowercase(),
        })
        .collect();
    items.sort_by(|left, right| {
        left.module
            .cmp(&right.module)
            .then_with(|| left.name.cmp(&right.name))
    });
    let package = hir
        .packages()
        .get(hir.primary_package())
        .map(|package| session.interner().resolve(package.name).to_owned())
        .unwrap_or_else(|| "package".to_owned());
    let snapshot = Snapshot {
        version: 1,
        target: session.target().triple().as_str().to_owned(),
        package,
        modules,
        items,
    };
    let text = serde_json::to_string_pretty(&snapshot)
        .map_err(|error| format!("cannot serialize the API snapshot: {error}"))?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create the API snapshot directory `{}`: {error}",
                parent.display()
            )
        })?;
    }
    let temporary = output.with_extension("json.tmp");
    std::fs::write(&temporary, text)
        .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
    std::fs::rename(&temporary, output)
        .map_err(|error| format!("cannot replace `{}`: {error}", output.display()))?;
    Ok(())
}

fn kind_name(kind: DefKind) -> &'static str {
    match kind {
        DefKind::Function => "function",
        DefKind::Struct => "struct",
        DefKind::Enum => "enum",
        DefKind::Trait => "trait",
        DefKind::Const => "const",
        DefKind::Static => "static",
        DefKind::ExternFunction => "extern-function",
        DefKind::ExternStatic => "extern-static",
        DefKind::OpaqueType => "opaque-type",
        DefKind::TraitMethod => "trait-method",
        DefKind::Variant => "variant",
        _ => "item",
    }
}
