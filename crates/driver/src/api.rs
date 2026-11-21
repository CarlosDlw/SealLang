//! JSON API snapshots for `--emit=api`.
//!
//! The snapshot is intentionally data, not a stable ABI: it lists the package's
//! modules, public items, signatures, struct fields, and enum variants so an
//! IDE, a project tool, or the registry can inspect a library without loading
//! the compiler again. The document is the same one `seal-api` hashes and
//! compares for compatibility.

use std::path::Path;

use seal_api::{ApiField, ApiItem, ApiModule, ApiSnapshot, ApiVariant};
use seal_def::{DefId, DefKind};
use seal_hir::Hir;
use seal_session::Session;
use seal_ty::{AdtKind, TyId};

use crate::cli::plan::path_string;

/// Writes the API snapshot for `hir` to `output`.
pub(crate) fn write(
    session: &Session,
    hir: &Hir,
    package_version: &str,
    output: &Path,
) -> Result<(), String> {
    let package = hir
        .packages()
        .get(hir.primary_package())
        .map(|package| session.interner().resolve(package.name).to_owned())
        .unwrap_or_else(|| "package".to_owned());
    let modules = hir
        .modules()
        .iter()
        .map(|module| ApiModule {
            path: path_string(&module.path, session.interner()),
            exposed: module.exposed,
            package: module.package.index() as usize,
        })
        .collect();
    let mut items = Vec::new();
    for def in hir.public_items() {
        if let Some(item) = item(session, hir, *def) {
            items.push(item);
        }
    }
    let mut snapshot = ApiSnapshot {
        version: seal_api::FORMAT,
        package,
        package_version: package_version.to_owned(),
        target: session.target().triple().as_str().to_owned(),
        modules,
        items,
    };
    snapshot.normalize();
    snapshot
        .validate()
        .map_err(|error| format!("the compiler produced an invalid API snapshot: {error}"))?;
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
    let temporary = output.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, text)
        .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
    std::fs::rename(&temporary, output)
        .map_err(|error| format!("cannot replace `{}`: {error}", output.display()))?;
    Ok(())
}

fn item(session: &Session, hir: &Hir, def: DefId) -> Option<ApiItem> {
    let data = hir.definitions().get(def);
    let kind = kind_name(data.kind)?;
    let module = path_string(&hir.module(data.module).path, session.interner());
    let name = session.interner().resolve(data.name).to_owned();
    let visibility = format!("{:?}", data.visibility).to_lowercase();
    let cfg = data
        .cfg
        .map(|cfg| session.interner().resolve(cfg).to_owned());
    let mut item = ApiItem {
        name,
        kind: kind.to_owned(),
        module,
        visibility,
        cfg,
        signature: None,
        fields: Vec::new(),
        variants: Vec::new(),
    };
    match data.kind {
        DefKind::Function | DefKind::ExternFunction => {
            item.signature = Some(signature(session, hir, def));
        }
        DefKind::Const | DefKind::Static | DefKind::ExternStatic => {
            item.signature = hir
                .body_of(def)
                .and_then(|body| body.ret)
                .map(|ty| format!(": {}", display(session, hir, ty)));
        }
        DefKind::Struct | DefKind::Enum => {
            let adt = hir.adt(def)?;
            let definition = hir.ty_ctxt().adt(adt);
            match definition.kind {
                AdtKind::Struct => {
                    item.fields = fields(session, hir, &definition.fields);
                }
                AdtKind::Enum => {
                    item.variants = definition
                        .variants
                        .iter()
                        .map(|variant| ApiVariant {
                            name: session.interner().resolve(variant.name).to_owned(),
                            fields: fields(session, hir, &variant.fields),
                        })
                        .collect();
                }
                AdtKind::Closure => return None,
            }
        }
        _ => {}
    }
    Some(item)
}

fn fields(session: &Session, hir: &Hir, fields: &[seal_ty::FieldDef]) -> Vec<ApiField> {
    fields
        .iter()
        .map(|field| {
            let data = hir.definitions().get(field.def);
            ApiField {
                name: field
                    .name
                    .map(|name| session.interner().resolve(name).to_owned()),
                ty: display(session, hir, field.ty),
                visibility: format!("{:?}", data.visibility).to_lowercase(),
            }
        })
        .collect()
}

fn signature(session: &Session, hir: &Hir, def: DefId) -> String {
    let data = hir.definitions().get(def);
    let Some(body) = hir.body_of(def) else {
        return format!("func {}", session.interner().resolve(data.name));
    };
    let mut text = String::new();
    if body.is_const {
        text.push_str("const ");
    }
    if body.is_unsafe {
        text.push_str("unsafe ");
    }
    text.push_str("func ");
    text.push_str(session.interner().resolve(data.name));
    text.push('(');
    for (index, param) in body.params.iter().enumerate() {
        if index > 0 {
            text.push_str(", ");
        }
        let name = session
            .interner()
            .resolve(hir.definitions().get(param.def).name);
        text.push_str(name);
        text.push_str(": ");
        text.push_str(&display(session, hir, param.ty));
    }
    text.push(')');
    if let Some(ret) = body.ret {
        text.push_str(" -> ");
        text.push_str(&display(session, hir, ret));
    }
    text
}

fn display(session: &Session, hir: &Hir, ty: TyId) -> String {
    hir.ty_ctxt()
        .display_with(ty, session.interner())
        .to_string()
}

fn kind_name(kind: DefKind) -> Option<&'static str> {
    match kind {
        DefKind::Function => Some("function"),
        DefKind::Struct => Some("struct"),
        DefKind::Enum => Some("enum"),
        DefKind::Trait => Some("trait"),
        DefKind::Const => Some("const"),
        DefKind::Static => Some("static"),
        DefKind::ExternFunction => Some("extern-function"),
        DefKind::ExternStatic => Some("extern-static"),
        DefKind::OpaqueType => Some("opaque-type"),
        _ => None,
    }
}
