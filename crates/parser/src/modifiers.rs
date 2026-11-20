//! Contextual modifier parsing.
//!
//! `public`, `internal`, `private`, `cfg`, and `test` are ordinary identifiers
//! outside the prefix of an item, field, or method. The parser only consumes
//! them in that position, so existing code that uses those names keeps working.

use seal_ast::{Modifiers, Visibility};
use seal_error_codes as codes;

use crate::parser::Parser;

impl Parser<'_> {
    /// Parses every modifier at the current position.
    ///
    /// Shape errors are reported here. Whether a modifier is meaningful for the
    /// following construct is decided by the caller, which has the grammar
    /// context to emit one specific diagnostic.
    pub(crate) fn parse_modifiers(&mut self) -> Modifiers {
        let mut modifiers = Modifiers::none();
        let mut explicit_visibility: Option<Visibility> = None;

        loop {
            let visibility = if self.at_name("public") {
                Some(Visibility::Public)
            } else if self.at_name("internal") {
                Some(Visibility::Internal)
            } else if self.at_name("private") {
                Some(Visibility::Private)
            } else {
                None
            };
            if let Some(visibility) = visibility {
                let span = self.bump().span;
                if explicit_visibility.replace(visibility).is_some() {
                    self.error(
                        codes::E1001,
                        span,
                        "an item has only one visibility modifier",
                    )
                    .help("keep one of `public`, `internal`, or `private`")
                    .emit();
                } else {
                    modifiers.visibility = visibility;
                    modifiers.has_visibility = true;
                }
                continue;
            }

            if self.at_name("cfg") {
                let span = self.bump().span;
                if modifiers.cfg.is_some() {
                    self.error(codes::E1001, span, "an item has only one `cfg` condition")
                        .help("combine the conditions with `&&` or `||` inside one `cfg(...)`")
                        .emit();
                } else {
                    modifiers.cfg = Some(self.parse_cfg_expr(span));
                }
                continue;
            }

            if self.at_name("test") {
                let span = self.bump().span;
                if modifiers.is_test {
                    self.error(codes::E1001, span, "`test` is written only once")
                        .help("remove the duplicate modifier")
                        .emit();
                } else {
                    modifiers.is_test = true;
                }
                continue;
            }

            break;
        }

        modifiers
    }
}
