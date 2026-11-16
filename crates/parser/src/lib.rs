//! The Seal parser.
//!
//! Recursive descent parses items, parameters, blocks, and statements; a Pratt
//! (precedence-climbing) parser handles expressions with the binding powers of a
//! C-family language. The result is a [`seal_ast::Ast`] whose nodes carry exact
//! spans, built through typed arenas.
//!
//! # Grammar
//!
//! ```text
//! program    = item*
//! item       = "func" IDENT "(" params? ")" ("->" type)? block
//! params     = param ("," param)* ","?
//! param      = IDENT ":" type
//! type       = "*" type | PRIMITIVE
//! block      = "{" statement* expr? "}"
//! statement  = "let" "mut"? IDENT (":" type)? ("=" expr)? ";"
//!            | "return" expr? ";"
//!            | "while" expr block
//!            | "break" ";"
//!            | "continue" ";"
//!            | expr ";"?
//! expr       = assignment and binary expressions over literals, names,
//!              calls, `if`, and parenthesized expressions
//! if_expr    = "if" expr block ("else" (block | if_expr))?
//! ```
//!
//! `if` is an expression whose value is the block that runs; without `else` its
//! type is unit. A block-like expression used as a statement needs no semicolon.
//! `break` and `continue` act on the innermost enclosing loop.
//!
//! Bindings are immutable unless `mut` is written; the AST records the flag, so
//! the checker never has to infer mutability from how a name is used.
//!
//! # Recovery
//!
//! Parsing never stops at the first error. A malformed item, statement, or
//! expression becomes an `Error` node, and the parser synchronizes at `;`, `}`,
//! `func`, and end of file. Diagnostics are emitted through [`DiagCtxt`] with
//! codes in the `E1xxx` range. Every loop asserts progress, so hostile input
//! cannot hang the compiler, and recursion is bounded by a nesting limit.
//!
//! # Example
//!
//! ```ignore
//! let lexed = seal_lexer::lex(&file, session.diag(), session.interner());
//! let ast = seal_parser::parse(&lexed, session.diag(), session.interner());
//! ```

#![warn(missing_docs)]

mod expr;
mod parser;
mod stmt;
mod types;

#[cfg(test)]
mod tests;

use seal_ast::Ast;
use seal_diagnostics::DiagCtxt;
use seal_lexer::Lexed;
use seal_symbol::Interner;

/// Parses a lexed file.
///
/// `lexed` must come from `seal_lexer::lex`; in particular its token stream
/// ends with `Eof`. Errors are reported through `dcx`; the returned tree is
/// total and uses `Error` nodes for syntax that could not be parsed.
pub fn parse(lexed: &Lexed, dcx: &DiagCtxt, interner: &Interner) -> Ast {
    parser::Parser::new(lexed, dcx, interner).parse_program()
}
