//! Lowered external declarations.
//!
//! An external function has a signature but no body, and an external static has
//! a storage type but no initializer. Both carry the linker symbol resolved from
//! the declaration, so later phases never re-read the source.

use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::{Abi, TyId};

use crate::body::ParamData;

/// One library named by a `link "...";` directive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LinkDirective {
    /// The library name passed to the linker.
    pub name: Symbol,
    /// The span of the directive, for diagnostics.
    pub span: Span,
}

/// One external function declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternFunction {
    /// The declared calling convention.
    pub abi: Abi,
    /// The linker symbol.
    pub symbol: Symbol,
    /// The span of the symbol, for diagnostics.
    pub symbol_span: Span,
    /// Whether the declaration ends with `...`.
    pub variadic: bool,
    /// Parameters in source order.
    pub params: Vec<ParamData>,
    /// The return type; `None` means `void`.
    pub ret: Option<TyId>,
}

/// One external static declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternStatic {
    /// The declared calling convention.
    pub abi: Abi,
    /// The linker symbol.
    pub symbol: Symbol,
    /// The span of the symbol, for diagnostics.
    pub symbol_span: Span,
    /// The storage type.
    pub ty: TyId,
}

/// Returns why `name` cannot be a linker symbol.
///
/// The check covers the cases that are invalid on every platform: an empty
/// name, a name with whitespace or a NUL byte, and `main`, which belongs to the
/// Seal entry point. Other spellings are accepted because C libraries export
/// names the compiler does not need to interpret.
pub(crate) fn invalid_symbol(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("a symbol name cannot be empty");
    }
    if name == "main" {
        return Some("the symbol `main` is reserved for the entry point");
    }
    if name.chars().any(|character| character == '\0') {
        return Some("a symbol name cannot contain a NUL byte");
    }
    if name.chars().any(char::is_whitespace) {
        return Some("a symbol name cannot contain whitespace");
    }
    if name.chars().any(char::is_control) {
        return Some("a symbol name cannot contain control characters");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::invalid_symbol;

    #[test]
    fn ordinary_c_symbols_are_accepted() {
        for name in ["abs", "getpid", "_exit", "sqlite3_open", "foo.bar"] {
            assert!(invalid_symbol(name).is_none(), "`{name}` must be valid");
        }
    }

    #[test]
    fn empty_reserved_and_whitespace_symbols_are_rejected() {
        assert!(invalid_symbol("").is_some());
        assert!(invalid_symbol("main").is_some());
        assert!(invalid_symbol("a b").is_some());
        assert!(invalid_symbol("a\0b").is_some());
        assert!(invalid_symbol("a\tb").is_some());
    }
}
