//! The Unicode rules Seal fixes for identifiers and terminal rendering.
//!
//! Every consumer of these rules (lexer, parser, diagnostics, future language
//! server) goes through this crate, so identifiers are classified identically
//! everywhere and no component depends on the host locale.
//!
//! Seal does not normalize source text (NFC/NFD): two identifiers that differ in
//! normalization are distinct, which keeps symbol identity a pure byte
//! comparison. A future lint may warn about visually confusable identifiers.

#![warn(missing_docs)]

/// Returns `true` if `character` may start an identifier.
///
/// The rule is `_` plus Unicode `XID_Start`.
#[inline]
pub fn is_ident_start(character: char) -> bool {
    character == '_' || unicode_ident::is_xid_start(character)
}

/// Returns `true` if `character` may continue an identifier.
///
/// The rule is Unicode `XID_Continue`.
#[inline]
pub fn is_ident_continue(character: char) -> bool {
    unicode_ident::is_xid_continue(character)
}

/// Returns `true` if `text` is a valid identifier.
///
/// An empty string is not an identifier.
pub fn is_ident(text: &str) -> bool {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) if is_ident_start(first) => characters.all(is_ident_continue),
        _ => false,
    }
}

/// Width of a tab stop used when rendering source snippets.
pub const TAB_WIDTH: usize = 4;

/// Returns the number of terminal columns `character` occupies.
///
/// Control characters and combining marks report `0`; wide characters (CJK,
/// emoji) report `2`. Tabs have no intrinsic width: callers that render source
/// text must expand them with [`TAB_WIDTH`].
#[inline]
pub fn char_width(character: char) -> usize {
    if character == '\t' {
        return 0;
    }
    unicode_width::UnicodeWidthChar::width(character).unwrap_or(0)
}

/// Returns the number of terminal columns `text` occupies.
///
/// Tabs are not expanded; use [`display_width_with_tabs`] when rendering source
/// text that may contain them.
#[inline]
pub fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// Returns the number of terminal columns `text` occupies, expanding tabs to the
/// next multiple of [`TAB_WIDTH`].
pub fn display_width_with_tabs(text: &str) -> usize {
    let mut width = 0;
    for character in text.chars() {
        if character == '\t' {
            width += TAB_WIDTH - (width % TAB_WIDTH);
        } else {
            width += char_width(character);
        }
    }
    width
}

/// Expands each tab in `text` to spaces up to the next [`TAB_WIDTH`] stop.
///
/// This is used by diagnostic renderers before computing marker positions. The
/// returned string contains no tabs, so byte positions and display positions
/// still diverge whenever the text contains non-ASCII characters; renderers must
/// use [`display_width`] for alignment, never `len()`.
pub fn expand_tabs(text: &str) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut width = 0;
    for character in text.chars() {
        if character == '\t' {
            let spaces = TAB_WIDTH - (width % TAB_WIDTH);
            for _ in 0..spaces {
                expanded.push(' ');
            }
            width += spaces;
        } else {
            expanded.push(character);
            width += char_width(character);
        }
    }
    expanded
}

#[cfg(test)]
mod tests {
    use super::{
        TAB_WIDTH, char_width, display_width, display_width_with_tabs, expand_tabs, is_ident,
        is_ident_continue, is_ident_start,
    };

    #[test]
    fn ascii_identifiers() {
        assert!(is_ident("name"));
        assert!(is_ident("_name"));
        assert!(is_ident("name_2"));
        assert!(!is_ident("2name"));
        assert!(!is_ident(""));
        assert!(!is_ident("na me"));
    }

    #[test]
    fn unicode_identifiers() {
        assert!(is_ident("ação"));
        assert!(is_ident("_日本"));
        assert!(is_ident_start('é'));
        assert!(is_ident_continue('é'));
        assert!(is_ident_continue('2'));
        assert!(!is_ident_start('2'));
        assert!(!is_ident_start(' '));
        assert!(!is_ident_continue('−'));
    }

    #[test]
    fn combining_marks_continue_but_do_not_start() {
        let combining_acute = '\u{0301}';
        assert!(!is_ident_start(combining_acute));
        assert!(is_ident_continue(combining_acute));
        assert!(is_ident("e\u{0301}"));
    }

    #[test]
    fn widths_of_common_characters() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('é'), 1);
        assert_eq!(char_width('日'), 2);
        assert_eq!(char_width('\u{0301}'), 0);
        assert_eq!(char_width('\t'), 0);
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("日本"), 4);
    }

    #[test]
    fn tabs_expand_to_stops() {
        assert_eq!(TAB_WIDTH, 4);
        assert_eq!(expand_tabs("\tx"), "    x");
        assert_eq!(expand_tabs("a\tb"), "a   b");
        assert_eq!(expand_tabs("abcd\te"), "abcd    e");
        assert_eq!(display_width_with_tabs("a\tb"), 5);
    }

    #[test]
    fn tab_expansion_preserves_wide_characters() {
        let expanded = expand_tabs("日\tx");
        assert_eq!(expanded, "日  x");
        assert_eq!(display_width(&expanded), 5);
    }
}
