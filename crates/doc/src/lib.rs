//! Rendering public API snapshots as documentation.
//!
//! The renderer consumes only [`seal_api::ApiSnapshot`], so the HTML output and
//! the JSON document always describe the same public surface the registry
//! checks. Private items never reach this crate because they are absent from the
//! snapshot.

#![warn(missing_docs)]

mod error;
mod html;
mod json;
mod server;

pub use error::{DocError, DocResult};
pub use html::{render as render_html, write as write_html};
pub use json::{render as render_json, write as write_json};
pub use server::{Served, serve};

use seal_api::ApiSnapshot;
use serde::Serialize;

/// One documented module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModuleDoc {
    /// The module path, with `::` separators.
    pub path: String,
    /// Whether dependents can import the module.
    pub exposed: bool,
    /// The module documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// The public items declared in this module.
    pub items: Vec<ItemDoc>,
}

/// One documented public item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ItemDoc {
    /// The item name.
    pub name: String,
    /// The item kind.
    pub kind: String,
    /// The item documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// The rendered signature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// The `cfg` condition, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cfg: Option<String>,
    /// Struct fields.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDoc>,
    /// Enum variants.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<VariantDoc>,
    /// Size and alignment of an aggregate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutDoc>,
    /// Attached implementations.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub implementations: Vec<ImplementationDoc>,
}

/// One documented field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldDoc {
    /// The field name; `None` for a positional field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The field type.
    pub ty: String,
    /// The field visibility.
    pub visibility: String,
    /// The field documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

/// One documented enum variant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VariantDoc {
    /// The variant name.
    pub name: String,
    /// The variant documentation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    /// The variant fields.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDoc>,
}

/// One documented aggregate layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LayoutDoc {
    /// Size in bytes.
    pub size: u64,
    /// Alignment in bytes.
    pub align: u64,
}

/// One documented implementation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImplementationDoc {
    /// The implemented trait, if any.
    #[serde(rename = "trait", skip_serializing_if = "Option::is_none")]
    pub trait_name: Option<String>,
    /// The method names.
    pub methods: Vec<String>,
}

/// The complete documentation document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Documentation {
    /// The document schema version.
    pub version: u32,
    /// The package name.
    pub package: String,
    /// The package version.
    pub package_version: String,
    /// The target triple.
    pub target: String,
    /// Public modules, sorted by path.
    pub modules: Vec<ModuleDoc>,
}

impl Documentation {
    /// Builds a documentation document from a public API snapshot.
    pub fn from_snapshot(snapshot: &ApiSnapshot) -> Self {
        let mut modules: Vec<ModuleDoc> = snapshot
            .modules
            .iter()
            .map(|module| ModuleDoc {
                path: module.path.clone(),
                exposed: module.exposed,
                docs: module.docs.clone(),
                items: Vec::new(),
            })
            .collect();
        for item in &snapshot.items {
            let module = modules.iter_mut().find(|module| module.path == item.module);
            let Some(module) = module else {
                continue;
            };
            module.items.push(item_doc(item));
        }
        for module in &mut modules {
            module.items.sort_by(|left, right| {
                left.name
                    .cmp(&right.name)
                    .then_with(|| left.kind.cmp(&right.kind))
            });
        }
        modules.sort_by(|left, right| left.path.cmp(&right.path));
        Self {
            version: 1,
            package: snapshot.package.clone(),
            package_version: snapshot.package_version.clone(),
            target: snapshot.target.clone(),
            modules,
        }
    }
}

fn item_doc(item: &seal_api::ApiItem) -> ItemDoc {
    ItemDoc {
        name: item.name.clone(),
        kind: item.kind.clone(),
        docs: item.docs.clone(),
        signature: item.signature.clone(),
        cfg: item.cfg.clone(),
        fields: item
            .fields
            .iter()
            .map(|field| FieldDoc {
                name: field.name.clone(),
                ty: field.ty.clone(),
                visibility: field.visibility.clone(),
                docs: field.docs.clone(),
            })
            .collect(),
        variants: item
            .variants
            .iter()
            .map(|variant| VariantDoc {
                name: variant.name.clone(),
                docs: variant.docs.clone(),
                fields: variant
                    .fields
                    .iter()
                    .map(|field| FieldDoc {
                        name: field.name.clone(),
                        ty: field.ty.clone(),
                        visibility: field.visibility.clone(),
                        docs: field.docs.clone(),
                    })
                    .collect(),
            })
            .collect(),
        layout: item.layout.map(|layout| LayoutDoc {
            size: layout.size,
            align: layout.align,
        }),
        implementations: item
            .implementations
            .iter()
            .map(|implementation| ImplementationDoc {
                trait_name: implementation.trait_name.clone(),
                methods: implementation.methods.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seal_api::{ApiField, ApiItem, ApiModule, ApiSnapshot};

    fn snapshot() -> ApiSnapshot {
        ApiSnapshot {
            version: 1,
            package: "demo".to_owned(),
            package_version: "1.0.0".to_owned(),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            modules: vec![
                ApiModule {
                    path: "root".to_owned(),
                    exposed: true,
                    package: 0,
                    docs: Some("The root module.".to_owned()),
                },
                ApiModule {
                    path: "app".to_owned(),
                    exposed: true,
                    package: 0,
                    docs: None,
                },
            ],
            items: vec![
                ApiItem {
                    name: "Point".to_owned(),
                    kind: "struct".to_owned(),
                    module: "root".to_owned(),
                    canonical: None,
                    visibility: "public".to_owned(),
                    cfg: None,
                    signature: None,
                    fields: vec![ApiField {
                        name: Some("x".to_owned()),
                        ty: "i32".to_owned(),
                        visibility: "public".to_owned(),
                        docs: Some("The x coordinate.".to_owned()),
                    }],
                    variants: Vec::new(),
                    docs: Some("A point.".to_owned()),
                    layout: Some(seal_api::ApiLayout { size: 4, align: 4 }),
                    implementations: Vec::new(),
                },
                ApiItem {
                    name: "value".to_owned(),
                    kind: "function".to_owned(),
                    module: "app".to_owned(),
                    canonical: None,
                    visibility: "public".to_owned(),
                    cfg: Some("target.os == \"linux\"".to_owned()),
                    signature: Some("func value() -> i32".to_owned()),
                    fields: Vec::new(),
                    variants: Vec::new(),
                    docs: Some("Returns a value.".to_owned()),
                    layout: None,
                    implementations: Vec::new(),
                },
            ],
        }
    }

    #[test]
    fn private_items_are_absent_and_modules_are_sorted() {
        let docs = Documentation::from_snapshot(&snapshot());
        assert_eq!(docs.modules.len(), 2);
        assert_eq!(docs.modules[0].path, "app");
        assert_eq!(
            docs.modules[0].items[0].cfg.as_deref(),
            Some("target.os == \"linux\"")
        );
    }

    #[test]
    fn html_contains_docs_cfg_and_links() {
        let html = render_html(&Documentation::from_snapshot(&snapshot()));
        assert!(html.contains("A point."));
        assert!(html.contains("The x coordinate."));
        assert!(html.contains("target.os"));
        assert!(html.contains("href=\"app.html\""));
        assert!(!html.contains("private"));
    }
}
