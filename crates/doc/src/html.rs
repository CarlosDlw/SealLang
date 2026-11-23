//! HTML rendering for `seal doc`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

use crate::{DeprecationDoc, DocError, DocResult, Documentation, FieldDoc, ItemDoc, ModuleDoc};

/// Renders the complete documentation set and writes it below `directory`.
pub fn write(documentation: &Documentation, directory: &Path) -> DocResult<Vec<String>> {
    std::fs::create_dir_all(directory).map_err(|error| DocError::Io {
        path: directory.display().to_string(),
        message: error.to_string(),
    })?;
    let index = directory.join("index.html");
    write_file(&index, &render(documentation))?;
    let mut written = vec![index.display().to_string()];
    let files = module_files(documentation);
    for module in &documentation.modules {
        let file = files
            .get(module.path.as_str())
            .expect("every module has a file name");
        let path = directory.join(format!("{file}.html"));
        write_file(&path, &render_module(documentation, module))?;
        written.push(path.display().to_string());
    }
    Ok(written)
}

/// Renders the index page.
pub fn render(documentation: &Documentation) -> String {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "<h1>{} {}</h1>",
        escape(&documentation.package),
        escape(&documentation.package_version)
    );
    let _ = writeln!(
        body,
        "<p class=\"meta\">Target: <code>{}</code></p>",
        escape(&documentation.target)
    );
    body.push_str("<h2>Modules</h2><ul class=\"modules\">");
    let files = module_files(documentation);
    for module in &documentation.modules {
        let file = files
            .get(module.path.as_str())
            .expect("every module has a file name");
        let _ = write!(
            body,
            "<li><a href=\"{file}.html\"><code>{}</code></a>",
            escape(&module.path)
        );
        if !module.exposed {
            body.push_str(" <span class=\"badge\">internal</span>");
        }
        if let Some(docs) = &module.docs {
            let _ = write!(body, "<p>{}</p>", markdown(docs));
        }
        body.push_str("</li>");
    }
    body.push_str("</ul>");
    for module in &documentation.modules {
        let file = files
            .get(module.path.as_str())
            .expect("every module has a file name");
        let _ = writeln!(
            body,
            "<h2 id=\"{}\"><a href=\"{file}.html\"><code>{}</code></a></h2>",
            anchor(&module.path),
            escape(&module.path)
        );
        if let Some(docs) = &module.docs {
            let _ = writeln!(body, "<div class=\"docs\">{}</div>", markdown(docs));
        }
        body.push_str("<ul class=\"items\">");
        for item in &module.items {
            let _ = write!(
                body,
                "<li><a href=\"{file}.html#{}\">{}</a> <span class=\"kind\">{}</span>",
                anchor(&item.name),
                escape(&item.name),
                escape(&item.kind)
            );
            if let Some(cfg) = &item.cfg {
                let _ = write!(body, " <span class=\"cfg\">cfg({})</span>", escape(cfg));
            }
            if let Some(deprecation) = &item.deprecated {
                let _ = write!(
                    body,
                    " <span class=\"badge deprecated\">deprecated since <code>{}</code></span>",
                    escape(&deprecation.since)
                );
            }
            if let Some(docs) = &item.docs {
                let summary = docs.lines().next().unwrap_or_default();
                let _ = write!(body, "<p>{}</p>", markdown(summary));
            }
            for field in &item.fields {
                if let Some(docs) = &field.docs {
                    let _ = write!(body, "<p>{}</p>", markdown(docs));
                }
            }
            body.push_str("</li>");
        }
        body.push_str("</ul>");
    }
    page(&documentation.package, &body)
}

/// Renders one module page.
fn render_module(documentation: &Documentation, module: &ModuleDoc) -> String {
    let mut body = String::new();
    let _ = writeln!(
        body,
        "<p><a href=\"index.html\">{} {}</a> / <code>{}</code></p>",
        escape(&documentation.package),
        escape(&documentation.package_version),
        escape(&module.path)
    );
    let _ = writeln!(body, "<h1><code>{}</code></h1>", escape(&module.path));
    if let Some(docs) = &module.docs {
        let _ = writeln!(body, "<div class=\"docs\">{}</div>", markdown(docs));
    }
    for item in &module.items {
        render_item(&mut body, item);
    }
    page(
        &format!("{} - {}", documentation.package, module.path),
        &body,
    )
}

fn render_item(body: &mut String, item: &ItemDoc) {
    let _ = writeln!(
        body,
        "<section class=\"item\" id=\"{}\">",
        anchor(&item.name)
    );
    let _ = writeln!(
        body,
        "<h2>{} <span class=\"kind\">{}</span></h2>",
        escape(&item.name),
        escape(&item.kind)
    );
    if let Some(cfg) = &item.cfg {
        let _ = writeln!(
            body,
            "<p class=\"cfg\">Available under <code>cfg({})</code></p>",
            escape(cfg)
        );
    }
    if let Some(signature) = &item.signature {
        let _ = writeln!(
            body,
            "<pre class=\"signature\"><code>{}</code></pre>",
            escape(signature)
        );
    }
    if let Some(docs) = &item.docs {
        let _ = writeln!(body, "<div class=\"docs\">{}</div>", markdown(docs));
    }
    if let Some(deprecation) = &item.deprecated {
        render_deprecation(body, deprecation);
    }
    if let Some(layout) = item.layout {
        let _ = writeln!(
            body,
            "<p class=\"layout\">Layout: {} byte(s), align {}</p>",
            layout.size, layout.align
        );
    }
    if !item.fields.is_empty() {
        body.push_str("<h3>Fields</h3><ul class=\"fields\">");
        for field in &item.fields {
            let name = field.name.as_deref().unwrap_or("#");
            let _ = write!(
                body,
                "<li><code>{}</code>: <code>{}</code>",
                escape(name),
                escape(&field.ty)
            );
            render_field_deprecation(body, field);
            if let Some(docs) = &field.docs {
                let _ = write!(body, "<div class=\"docs\">{}</div>", markdown(docs));
            }
            body.push_str("</li>");
        }
        body.push_str("</ul>");
    }
    if !item.variants.is_empty() {
        body.push_str("<h3>Variants</h3><ul class=\"variants\">");
        for variant in &item.variants {
            let _ = write!(body, "<li><code>{}</code>", escape(&variant.name));
            if let Some(deprecation) = &variant.deprecated {
                let _ = write!(
                    body,
                    " <span class=\"badge deprecated\">deprecated since <code>{}</code></span>",
                    escape(&deprecation.since)
                );
            }
            if let Some(docs) = &variant.docs {
                let _ = write!(body, "<div class=\"docs\">{}</div>", markdown(docs));
            }
            if !variant.fields.is_empty() {
                body.push_str("<ul>");
                for field in &variant.fields {
                    let name = field.name.as_deref().unwrap_or("#");
                    let _ = write!(
                        body,
                        "<li><code>{}</code>: <code>{}</code>",
                        escape(name),
                        escape(&field.ty)
                    );
                    render_field_deprecation(body, field);
                    if let Some(docs) = &field.docs {
                        let _ = write!(body, "<div class=\"docs\">{}</div>", markdown(docs));
                    }
                    body.push_str("</li>");
                }
                body.push_str("</ul>");
            }
            body.push_str("</li>");
        }
        body.push_str("</ul>");
    }
    if !item.implementations.is_empty() {
        body.push_str("<h3>Implementations</h3><ul class=\"impls\">");
        for implementation in &item.implementations {
            let target = implementation
                .trait_name
                .as_deref()
                .map(|name| format!("trait <code>{}</code>", escape(name)))
                .unwrap_or_else(|| "inherent".to_owned());
            let _ = write!(body, "<li>{target}: ");
            body.push_str(
                &implementation
                    .methods
                    .iter()
                    .map(|method| format!("<code>{}</code>", escape(method)))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            body.push_str("</li>");
        }
        body.push_str("</ul>");
    }
    body.push_str("</section>");
}

/// Renders the full deprecation notice of an item.
fn render_deprecation(body: &mut String, deprecation: &DeprecationDoc) {
    let _ = write!(
        body,
        "<p class=\"deprecated\">Deprecated since <code>{}</code>",
        escape(&deprecation.since)
    );
    if let Some(note) = &deprecation.note {
        let _ = write!(body, ". {}", escape(note));
    }
    if let Some(replacement) = &deprecation.replacement {
        let _ = write!(body, ". Use <code>{}</code> instead", escape(replacement));
    }
    body.push_str(".</p>");
}

/// Renders the compact deprecation badge attached to a field.
fn render_field_deprecation(body: &mut String, field: &FieldDoc) {
    if let Some(deprecation) = &field.deprecated {
        let _ = write!(
            body,
            " <span class=\"badge deprecated\">deprecated since <code>{}</code></span>",
            escape(&deprecation.since)
        );
    }
}

/// Returns whether a doc-comment link uses a relative path or a known safe
/// scheme. A package cannot turn documentation into a script URL.
fn safe_link(target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    let lower = target.to_ascii_lowercase();
    let Some((scheme, _)) = lower.split_once(':') else {
        return true;
    };
    matches!(scheme, "http" | "https" | "mailto")
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n{}\n</body>\n</html>\n",
        escape(title),
        body
    )
}

const STYLE: &str = "body{font-family:system-ui,sans-serif;max-width:60rem;margin:2rem auto;padding:0 1rem;line-height:1.5;color:#1f2933}code,pre{font-family:ui-monospace,monospace;background:#f3f4f6;border-radius:.2rem;padding:.1rem .25rem}pre{padding:.75rem;overflow:auto}.meta,.cfg,.layout{color:#52606d}.deprecated{color:#9a3412}.badge.deprecated{background:#ffedd5}.kind{font-size:.8rem;color:#7b8794;font-weight:normal}.item{border-top:1px solid #e4e7eb;margin-top:2rem}.badge{font-size:.7rem;background:#e4e7eb;border-radius:.2rem;padding:.1rem .3rem}a{color:#1d4ed8}";

fn write_file(path: &Path, contents: &str) -> DocResult<()> {
    std::fs::write(path, contents).map_err(|error| DocError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

/// Returns the escaped HTML text.
fn escape(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#39;"),
            other => output.push(other),
        }
    }
    output
}

/// Returns one unique file stem per module, preserving sorted module order.
fn module_files(documentation: &Documentation) -> BTreeMap<&str, String> {
    let mut used = BTreeSet::new();
    let mut files = BTreeMap::new();
    for module in &documentation.modules {
        let base = module_file(module);
        let mut candidate = base.clone();
        let mut suffix = 1;
        while !used.insert(candidate.clone()) {
            candidate = format!("{base}-{suffix}");
            suffix += 1;
        }
        files.insert(module.path.as_str(), candidate);
    }
    files
}

/// Returns a filesystem-safe module file stem.
fn module_file(module: &ModuleDoc) -> String {
    let mut output = String::new();
    for character in module.path.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            output.push(character);
        } else {
            output.push('_');
        }
    }
    if output.is_empty() {
        output.push_str("root");
    }
    output
}

/// Returns an HTML anchor for a name.
fn anchor(name: &str) -> String {
    let mut output = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            output.push(character);
        } else {
            output.push('-');
        }
    }
    output
}

/// Renders a small, safe Markdown subset used by doc comments.
///
/// Supported syntax: paragraphs, inline `code`, **bold**, and `[text](target)`
/// links. Everything else is escaped, so a doc comment can never inject HTML.
fn markdown(text: &str) -> String {
    let mut output = String::new();
    for (index, paragraph) in text.split("\n\n").enumerate() {
        if index > 0 {
            output.push_str("</p><p>");
        }
        let escaped = escape(paragraph.trim());
        output.push_str(&inline(&escaped));
    }
    format!("<p>{output}</p>")
}

fn inline(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut output = String::new();
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        if character == '`' {
            let mut end = index + 1;
            while end < characters.len() && characters[end] != '`' {
                end += 1;
            }
            if end < characters.len() {
                let code: String = characters[index + 1..end].iter().collect();
                let _ = write!(output, "<code>{code}</code>");
                index = end + 1;
                continue;
            }
        } else if character == '*' && index + 1 < characters.len() && characters[index + 1] == '*' {
            let mut end = index + 2;
            while end + 1 < characters.len()
                && !(characters[end] == '*' && characters[end + 1] == '*')
            {
                end += 1;
            }
            if end + 1 < characters.len() {
                let bold: String = characters[index + 2..end].iter().collect();
                let _ = write!(output, "<strong>{bold}</strong>");
                index = end + 2;
                continue;
            }
        } else if character == '[' {
            if let Some(end) = characters[index + 1..]
                .iter()
                .position(|item| *item == ']')
                .map(|position| index + 1 + position)
            {
                if end + 1 < characters.len() && characters[end + 1] == '(' {
                    let mut close = end + 2;
                    let mut depth = 1;
                    while close < characters.len() {
                        match characters[close] {
                            '(' => depth += 1,
                            ')' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        close += 1;
                    }
                    if close < characters.len() {
                        let label: String = characters[index + 1..end].iter().collect();
                        let target: String = characters[end + 2..close].iter().collect();
                        if safe_link(&target) {
                            let _ = write!(output, "<a href=\"{}\">{label}</a>", escape(&target));
                        } else {
                            let _ = write!(output, "[{label}]({target})");
                        }
                        index = close + 1;
                        continue;
                    }
                }
            }
        }
        output.push(character);
        index += 1;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_escapes_html_and_renders_code() {
        assert_eq!(
            markdown("A `<Point>` with **care**."),
            "<p>A <code>&lt;Point&gt;</code> with <strong>care</strong>.</p>"
        );
    }

    #[test]
    fn script_links_are_not_rendered_as_anchors() {
        assert_eq!(
            markdown("[x](javascript:alert(1))"),
            "<p>[x](javascript:alert(1))</p>"
        );
        assert_eq!(
            markdown("[x](data:text/html,<script>alert(1)</script>)"),
            "<p>[x](data:text/html,&lt;script&gt;alert(1)&lt;/script&gt;)</p>"
        );
        assert!(markdown("[home](https://example.com)").contains("href=\"https://example.com\""));
    }

    #[test]
    fn module_files_are_safe_stems() {
        let module = ModuleDoc {
            path: "app::util".to_owned(),
            exposed: true,
            docs: None,
            items: Vec::new(),
        };
        assert_eq!(module_file(&module), "app__util");
    }

    #[test]
    fn colliding_module_stems_get_distinct_files() {
        let documentation = Documentation {
            version: 1,
            package: "demo".to_owned(),
            package_version: "1.0.0".to_owned(),
            target: "x86_64-unknown-linux".to_owned(),
            modules: ["app::util", "app__util"]
                .into_iter()
                .map(|path| ModuleDoc {
                    path: path.to_owned(),
                    exposed: true,
                    docs: None,
                    items: Vec::new(),
                })
                .collect(),
        };
        let files = module_files(&documentation);
        assert_eq!(files["app::util"], "app__util");
        assert_eq!(files["app__util"], "app__util-1");
    }
}
