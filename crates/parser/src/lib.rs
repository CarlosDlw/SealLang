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
//! program    = module? use* item*
//! module     = "module" IDENT ("::" IDENT)* ";"
//! use        = modifiers? "use" use_path ";"
//! use_path   = IDENT ("::" IDENT)* ("::" group)? ("as" IDENT)?
//! group      = "{" IDENT ("as" IDENT)? ("," IDENT ("as" IDENT)?)* ","? "}"
//! modifiers  = ("public" | "internal" | "private")?
//!              ("cfg" "(" cfg_expr ")")? ("test")?
//! cfg_expr   = cfg_unary (("&&" | "||") cfg_unary)*
//! cfg_unary  = "!" cfg_unary | "(" cfg_expr ")" | cfg_atom
//! cfg_atom   = "feature" "(" STRING ")" | "env" "(" STRING ")"
//!            | "target" "." IDENT (("==" | "!=") literal)?
//!            | "debug_assertions" | "test" | "bench"
//! item       = modifiers? ("func" IDENT "(" params? ")" ("->" type)? block
//!            | "const" "func" IDENT "(" params? ")" ("->" type)? block
//!            | "const" IDENT ":" type "=" expr ";"
//!            | "static" "mut"? IDENT ":" type "=" expr ";")
//!            | "struct" IDENT generics? "{" fields "}"
//!            | "enum" IDENT generics? "{" variants "}"
//!            | "impl" generics? type ("for" type)? "{" item* "}"
//!            | "trait" IDENT generics? (":" bound ("," bound)*)? "{" item* "}"
//! params     = param ("," param)* ","?
//! param      = IDENT ":" type
//! type       = "*" type | PRIMITIVE | IDENT ("::" IDENT)* generic_args?
//! block      = "{" statement* expr? "}"
//! statement  = "let" "mut"? IDENT (":" type)? ("=" expr)? ";"
//!            | "const" IDENT ":" type "=" expr ";"
//!            | "static" "mut"? IDENT ":" type "=" expr ";"
//!            | "return" expr? ";"
//!            | "while" expr block
//!            | "break" ";"
//!            | "continue" ";"
//!            | expr ";"?
//! expr       = assignment and binary expressions over literals, names,
//!              calls, `if`, `unsafe` blocks, and parenthesized expressions
//! if_expr    = "if" expr block ("else" (block | if_expr))?
//! ```
//!
//! `module`, `use`, the visibility words, `cfg`, and `test` are contextual: they
//! are reserved only in the prefix position shown above and remain ordinary
//! identifiers in bindings, function names, and type names. A module declaration
//! is at most one and must precede every import and item; imports are absolute.
//!
//! `if` is an expression whose value is the block that runs; without `else` its
//! type is unit. `unsafe { ... }` is a prefix expression whose value is the
//! block's value; it needs no semicolon in statement position. A block-like
//! expression used as a statement needs no semicolon. `break` and `continue` act
//! on the innermost enclosing loop.
//!
//! `const` and `static` may be declared at file scope or as a declaration
//! statement in a block. The token after `const` selects the form: `func` starts
//! a constant function, and anything else starts a constant declaration.
//!
//! Bindings are immutable unless `mut` is written; the AST records the flag, so
//! the checker never has to infer mutability from how a name is used.
//!
//! # Recovery
//!
//! Parsing never stops at the first error. A malformed item, statement, or
//! expression becomes an `Error` node, and the parser synchronizes at `;`, `}`,
//! `func`, `const`, `static`, and end of file. Diagnostics are emitted through [`DiagCtxt`] with
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

mod cfg;
mod expr;
mod extern_item;
mod generics;
mod modifiers;
mod module;
mod parser;
mod pattern;
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
