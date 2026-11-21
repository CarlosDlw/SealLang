//! Extracting doc comments from source spans.
//!
//! `seal-parser` keeps the source range of attached documentation comments in
//! the AST. Turning that range into text belongs to the tooling layer, which has
//! the source map; this module centralizes the small amount of comment syntax.

use seal_span::Span;
use seal_symbol::Interner;

/// Returns the documentation text attached to `docs`, when the span is valid.
///
/// Line comments become individual lines. Block comments keep their content and
/// lose the conventional leading `*` margin. The result has no leading or
/// trailing blank lines and preserves internal paragraph breaks.
pub(crate) fn render(
    source_map: &seal_span::SourceMap,
    docs: Option<Span>,
    _interner: &Interner,
) -> Option<String> {
    let span = docs?;
    let (_, file) = source_map.lookup(span.lo())?;
    let lo = file.local_offset(span.lo())? as usize;
    let hi = file.local_offset(span.hi())? as usize;
    let text = file.text().get(lo..hi)?;
    let mut lines = strip_comments(text);
    while lines.first().is_some_and(|line| line.trim().is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn strip_comments(text: &str) -> Vec<String> {
    let mut output = Vec::new();
    let mut block: Option<&str> = None;
    for raw in text.lines() {
        let line = raw.trim_start();
        if let Some(closing) = block {
            let (content, ended) = match line.find(closing) {
                Some(index) => (&line[..index], true),
                None => (line, false),
            };
            output.push(block_line(content));
            if ended {
                block = None;
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("///") {
            output.push(strip_one_leading_space(rest).to_owned());
        } else if let Some(rest) = line.strip_prefix("//!") {
            output.push(strip_one_leading_space(rest).to_owned());
        } else if let Some(rest) = line.strip_prefix("/**") {
            if let Some(index) = rest.find("*/") {
                output.push(block_line(&rest[..index]));
            } else {
                output.push(block_line(rest));
                block = Some("*/");
            }
        } else if let Some(rest) = line.strip_prefix("/*!") {
            if let Some(index) = rest.find("*/") {
                output.push(block_line(&rest[..index]));
            } else {
                output.push(block_line(rest));
                block = Some("*/");
            }
        } else {
            output.push(raw.to_owned());
        }
    }
    output
}

fn block_line(line: &str) -> String {
    let line = line.trim_start();
    let line = line.strip_prefix('*').unwrap_or(line);
    strip_one_leading_space(line).trim_end().to_owned()
}

fn strip_one_leading_space(text: &str) -> &str {
    text.strip_prefix(' ').unwrap_or(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_comments_become_plain_text() {
        assert_eq!(
            strip_comments("/// A point.\n///\n/// It has two fields."),
            vec!["A point.", "", "It has two fields."]
        );
    }

    #[test]
    fn block_comments_lose_the_margin() {
        assert_eq!(
            strip_comments("/**\n * A point.\n *\n * It has two fields.\n */"),
            vec!["", "A point.", "", "It has two fields.", ""]
        );
    }
}
