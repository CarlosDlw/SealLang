//! The stable diagnostic-code registry.
//!
//! Codes are a public interface: tooling filters on them, and users search for
//! them. Once assigned, a code never changes meaning and is never reused, even
//! if the diagnostic is retired.
//!
//! Ranges are allocated per compiler phase:
//!
//! | Range | Phase |
//! |-------|-------|
//! | `E0xxx` | source loading / lexer |
//! | `E1xxx` | parser |
//! | `E2xxx` | name resolution |
//! | `E3xxx` | type checking |
//! | `E4xxx` | const evaluation / patterns |
//! | `E5xxx` | MIR / ownership |
//! | `E6xxx` | code generation / linking |
//! | `E9xxx` | compiler internals |
//!
//! Warnings are identified by lint name, not by code.

#![warn(missing_docs)]

use seal_diagnostics::DiagCode;

/// A documented diagnostic code.
#[derive(Clone, Copy, Debug)]
pub struct ErrorCode {
    code: &'static str,
    title: &'static str,
    explanation: &'static str,
}

impl ErrorCode {
    /// Returns the stable code text.
    #[inline]
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// Returns the one-line title.
    #[inline]
    pub const fn title(self) -> &'static str {
        self.title
    }

    /// Returns the long-form explanation.
    #[inline]
    pub const fn explanation(self) -> &'static str {
        self.explanation
    }

    /// Returns the code as a diagnostic code.
    ///
    /// The registry is the source of truth for code format; a malformed entry is
    /// caught by the registry's own tests before it can reach this method.
    #[inline]
    pub fn diag_code(self) -> DiagCode {
        DiagCode::new(self.code)
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code)
    }
}

/// The source file is not valid UTF-8.
pub const E0001: ErrorCode = ErrorCode {
    code: "E0001",
    title: "source file is not valid UTF-8",
    explanation: "\
Seal source files must be encoded as UTF-8. Unlike some languages, Seal does not
guess an encoding from the locale: a file with invalid UTF-8 is rejected, because
byte offsets in diagnostics and debug information would otherwise be ambiguous.

This error reports the first byte offset that is not part of a valid UTF-8
sequence. Convert the file to UTF-8 (most editors do this by default) and rebuild.",
};

/// The source file exceeds the compiler's size limit.
pub const E0002: ErrorCode = ErrorCode {
    code: "E0002",
    title: "source file is too large",
    explanation: "\
Source positions are 32-bit, so a single file cannot exceed the position space.
The limit is far above any source file a human writes; exceeding it usually
indicates that the wrong file was passed to the compiler (for example, a data or
binary file).

Split the input or verify the path passed to `sealc`.",
};

/// The source file could not be read.
pub const E0003: ErrorCode = ErrorCode {
    code: "E0003",
    title: "could not read source file",
    explanation: "\
The compiler could not read the path given on the command line. The attached note
contains the operating-system error: the file may not exist, may be a directory,
or may lack read permission.

Check the path and the file permissions, then rebuild.",
};

/// A character that cannot start any token.
pub const E0004: ErrorCode = ErrorCode {
    code: "E0004",
    title: "invalid character",
    explanation: "\
The lexer found a code point that cannot start a token. Common causes are a
stray punctuation character from another language, non-ASCII whitespace (such as
a no-break space pasted from a document), or an invisible format character.

Seal accepts only ASCII space, tab, line feed, and carriage-return/line-feed as
whitespace. Replace the character with its ASCII equivalent, or remove it. A
bidirectional control character is reported separately as E0012.",
};

/// An escape sequence is malformed.
pub const E0005: ErrorCode = ErrorCode {
    code: "E0005",
    title: "invalid escape sequence",
    explanation: "\
Escape sequences inside string and character literals must be well formed. Seal
supports `\\0`, `\\t`, `\\n`, `\\r`, `\\\"`, `\\'`, `\\\\`, `\\xNN` with exactly two
hexadecimal digits and a value up to 0x7F, and `\\u{...}` with one to six hex
digits naming a Unicode scalar value.

A backslash followed by a newline continues a string literal and skips the
following whitespace; it is not allowed in character literals. Unknown escapes,
missing braces, surrogates, and out-of-range code points are all reported here.",
};

/// A string literal is not terminated.
pub const E0006: ErrorCode = ErrorCode {
    code: "E0006",
    title: "unterminated string literal",
    explanation: "\
A string literal starts with a double quote and must end on the same line, unless
the newline is escaped with a trailing backslash. The parser reached a line break
or the end of the file before the closing quote.

Add the closing `\"`. A line break inside a string must be written as `\\n`, or
escaped with a backslash at the end of the line to continue the literal.",
};

/// A numeric literal is malformed.
pub const E0007: ErrorCode = ErrorCode {
    code: "E0007",
    title: "malformed numeric literal",
    explanation: "\
Numeric literals must follow one of the supported shapes: decimal digits, `0b`
binary, `0o` octal, or `0x` hexadecimal, optionally followed by a decimal part
and an exponent for decimal floats. Base prefixes are lowercase.

Separators (`_`) may appear only between two digits, never at the start, at the
end, next to a decimal point, next to an exponent marker, or next to a suffix.
A prefixed literal cannot have a decimal point, and an exponent needs at least
one digit after an optional sign.",
};

/// A numeric suffix is not a known type suffix.
pub const E0008: ErrorCode = ErrorCode {
    code: "E0008",
    title: "invalid numeric suffix",
    explanation: "\
A numeric literal may end with the name of a fixed-width numeric type: `i8`,
`i16`, `i32`, `i64`, `i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, `usize`,
`f32`, or `f64`. Integer literals accept integer suffixes, and float literals
accept `f32` or `f64`. Anything else attached to the digits is a syntax error,
not a separate identifier.

Remove the suffix or spell one of the supported type names.",
};

/// A numeric literal does not fit its type.
pub const E0009: ErrorCode = ErrorCode {
    code: "E0009",
    title: "numeric literal out of range",
    explanation: "\
Every integer literal must fit in 128 bits, the widest integer Seal supports,
and every float literal must be finite in `f64` (or in `f32` when the literal has
an `f32` suffix).

Use a smaller value, split the computation into several literals, or choose a
wider type. Values that overflow a narrower suffix such as `u8` are diagnosed
later, when the literal's type is known.",
};

/// A character literal is malformed.
pub const E0010: ErrorCode = ErrorCode {
    code: "E0010",
    title: "malformed character literal",
    explanation: "\
A character literal contains exactly one Unicode scalar value between single
quotes, for example `'a'`, `'é'`, or `'\\u{1F980}'`. It cannot be empty, cannot
contain more than one scalar value, and must be closed before the end of the
line.

For text with more than one character use a string literal (`\"ab\"`). A
grapheme that looks like one character may still be several scalar values, such
as a letter followed by a combining accent.",
};

/// A block comment is not terminated.
pub const E0011: ErrorCode = ErrorCode {
    code: "E0011",
    title: "unterminated block comment",
    explanation: "\
Block comments start with `/*` and end with `*/`, and they nest. The opening
comment of the report is the one still looking for its closing delimiter when
the end of the file was reached.

Add the missing `*/`. Nested comments are useful for commenting out code that
already contains block comments; every opener needs its own closer.",
};

/// A bidirectional control character appears in the source.
pub const E0012: ErrorCode = ErrorCode {
    code: "E0012",
    title: "bidirectional control character",
    explanation: "\
The source contains one of the Unicode bidirectional formatting characters
(U+202A to U+202E and U+2066 to U+2069). They reorder how text is displayed
without changing the bytes the compiler reads, which lets code display one way
and behave another (CVE-2021-42574, Trojan Source).

Seal rejects these characters everywhere, including comments and string
literals. If a string must contain one at runtime, write it with a Unicode escape
such as `\\u{202E}` instead of embedding the character.",
};

/// A literal contains a raw control character.
pub const E0013: ErrorCode = ErrorCode {
    code: "E0013",
    title: "invalid character in literal",
    explanation: "\
String and character literals cannot contain raw control characters such as NUL,
vertical tab, or DEL. They are invisible in most editors, hide what the literal
actually contains, and are easy to introduce by pasting data.

Write the character with an escape sequence instead: `\\0` for NUL, `\\t` for
tab, `\\n` for a newline, or `\\u{...}` for an arbitrary code point. A tab is
allowed literally inside a string, but `\\t` is clearer.",
};

/// The parser expected a top-level item.
pub const E1001: ErrorCode = ErrorCode {
    code: "E1001",
    title: "expected an item",
    explanation: "\
A source file is a sequence of items, and the only item Seal currently defines is
a function declaration. The parser found a token that cannot start one.

Start the declaration with `func`, or remove the stray text. Items are not nested:
functions are declared at the top level of the file, never inside another function.",
};

/// The parser expected a particular token.
pub const E1002: ErrorCode = ErrorCode {
    code: "E1002",
    title: "expected a token",
    explanation: "\
The grammar required a specific token at this point, such as a `;`, `)`, `}`, or
`->`, but the parser found something else. The message names the expected token
and the token that was found.

Check the surrounding delimiters. Because Seal statements are terminated by `;`,
a missing semicolon is the most common cause of this error.",
};

/// The parser expected an expression.
pub const E1003: ErrorCode = ErrorCode {
    code: "E1003",
    title: "expected an expression",
    explanation: "\
An expression was required but the parser found a token that cannot start one.
Expressions are literals, names, parenthesized expressions, calls, and unary or
binary operations on those.

A common cause is a missing operand, as in `let x = ;`, or an operator from
another language. Add an operand or remove the operator.",
};

/// The parser expected a type.
pub const E1004: ErrorCode = ErrorCode {
    code: "E1004",
    title: "expected a type",
    explanation: "\
A type annotation needs a type name after the `:` or `->`. Seal currently has the
integer types, `f32`, `f64`, `bool`, `char`, `str`, and pointer types written
`*T`.

Write one of those names, or a pointer to one. Names that are not built-in types
are reported here too, because user-defined types do not exist yet.",
};

/// The parser expected an identifier.
pub const E1005: ErrorCode = ErrorCode {
    code: "E1005",
    title: "expected an identifier",
    explanation: "\
This position needs a name: a function name, a parameter name, or a binding name.
The parser found a keyword, a literal, or punctuation instead.

Identifiers start with a letter or `_` and continue with letters, digits, and
underscores; Unicode identifiers are accepted.",
};

/// An assignment or update has a target that cannot be assigned.
pub const E1006: ErrorCode = ErrorCode {
    code: "E1006",
    title: "invalid assignment target",
    explanation: "\
The left side of an assignment, or the operand of `++`/`--`, must be a place: a
variable name (possibly parenthesized). Literals, calls, and arithmetic results
are values, not places, so they cannot be assigned to.

Assign to a `let` binding instead, or remove the operation.",
};

/// A `let` binding has neither a type nor an initializer.
pub const E1007: ErrorCode = ErrorCode {
    code: "E1007",
    title: "let binding needs a type or an initializer",
    explanation: "\
`let x;` leaves the compiler with no way to know the binding's type. A binding
must either have a type annotation (`let x: i32;`) or an initializer
(`let x = 1;`), or both.

Add one of them. The initializer can still be assigned later, so a type-only
binding is useful when the first assignment is conditional.",
};

/// Recursive syntax exceeds the parser's nesting limit.
pub const E1008: ErrorCode = ErrorCode {
    code: "E1008",
    title: "code is nested too deeply",
    explanation: "\
The parser protects the process stack with a nesting limit for expressions and
types. Valid programs stay far below it; hitting the limit usually means deeply
nested parentheses, unary operators, or pointer types, such as generated code.

Simplify the expression or split it into several `let` bindings. The limit exists
so malformed or hostile input reports an error instead of crashing the compiler.",
};

/// A definition is declared twice in the same scope.
pub const E2001: ErrorCode = ErrorCode {
    code: "E2001",
    title: "duplicate definition",
    explanation: "\
Names must be unique within their scope. Two functions with the same name, or two
parameters with the same name, make every reference ambiguous.

A `let` binding is different: `let x = 1; let x = 2;` is allowed and shadows the
first binding from the second declaration onwards. Shadowing is intentional,
while redeclaring a function or a parameter is not. Rename one of the
definitions.",
};

/// A name does not resolve in the current scope.
pub const E2002: ErrorCode = ErrorCode {
    code: "E2002",
    title: "cannot find name in this scope",
    explanation: "\
Every name used as a value must be declared before it is used, except functions,
which may be declared later in the file. A `let` binding is only visible after
its own declaration, and only inside the block that declares it; an `if` or
`while` body cannot leak names to the code after it.

Declare the name with `let`, define a function with `func`, or move the use into
the scope where the name exists. A binding is not visible in its own initializer,
so `let x = x;` reads an outer `x` and fails when there is none.",
};

/// A `break` appears outside any loop.
pub const E2003: ErrorCode = ErrorCode {
    code: "E2003",
    title: "break outside of a loop",
    explanation: "\
`break` exits the innermost `while` loop whose body contains it. A `break` at the
top level of a function body has no loop to exit, so the compiler rejects it.

Move the statement inside a `while` body, or remove it if the loop was deleted.
`break` inside an `if` that is itself inside a loop is fine: the enclosing loop
is still the target.",
};

/// A `continue` appears outside any loop.
pub const E2004: ErrorCode = ErrorCode {
    code: "E2004",
    title: "continue outside of a loop",
    explanation: "\
`continue` starts the next iteration of the innermost `while` loop whose body
contains it. A `continue` at the top level of a function body has no loop to
continue, so the compiler rejects it.

Move the statement inside a `while` body, or remove it. `continue` inside an `if`
that is itself inside a loop targets the enclosing loop and is accepted.",
};

/// Expression types do not match.
pub const E3001: ErrorCode = ErrorCode {
    code: "E3001",
    title: "mismatched types",
    explanation: "\
Seal has no implicit conversions: the type an expression has must be exactly the
type its context requires. This applies to initializers, assignments, arguments,
returns, conditions, and the two branches of an `if`.

A literal takes the type its context requires when that type is compatible, so
`let x: i64 = 1;` is accepted, while `let x: i64 = 1i32;` is not: the suffix
pins the literal to `i32`. Choose the same type on both sides, add a suffix, or
change the declaration. The diagnostic names the expected and the found type.",
};

/// An operator cannot be applied to the operand's type.
pub const E3002: ErrorCode = ErrorCode {
    code: "E3002",
    title: "invalid operands for operator",
    explanation: "\
Each operator accepts a fixed family of types. Arithmetic operators accept
integers and floats of one type; `&`, `|`, `^`, `~`, `<<`, and `>>` accept
integers; `&&`, `||`, and `!` accept `bool`; comparisons accept two equal
numeric or `char` values, and equality also accepts other values.

Operands are never converted, so mixed-width arithmetic such as `1i32 + 1i64` is
an error even though both sides are integers. Make both operands the same type.
Negating an unsigned integer is also rejected, because the result would not fit
the operand's type.",
};

/// An `if` or `while` condition is not `bool`.
pub const E3003: ErrorCode = ErrorCode {
    code: "E3003",
    title: "condition must be `bool`",
    explanation: "\
The condition of an `if` or `while` decides control flow, so it must have type
`bool`. Seal does not use truthiness: integers, floats, characters, and
references are not conditions on their own.

Write a comparison such as `value != 0`, or bind the condition to a `bool` and
use that binding.",
};

/// A binding that is not mutable is assigned to.
pub const E3004: ErrorCode = ErrorCode {
    code: "E3004",
    title: "assignment to immutable binding",
    explanation: "\
Bindings are immutable unless declared with `let mut` (decision D8 of the memory
model). Assignment, compound assignment, `++`, and `--` all change the place, so
they require a mutable binding. Parameters are immutable too; a function that
needs to change an argument must bind it again with `let mut`.

A binding declared without an initializer is the one exception: assigning to it
for the first time initializes it (decision D9), and only later writes require
`let mut`. Declare the binding with `let mut`, or remove the write.",
};

/// An assignment target is not a local place.
pub const E3005: ErrorCode = ErrorCode {
    code: "E3005",
    title: "invalid assignment target",
    explanation: "\
An assignment target must be a local place: a `let` binding or a parameter,
possibly written in parentheses. Functions are items, not storage, so assigning
to a function name is rejected even though the name resolves.

Assign to a local binding instead, or remove the assignment.",
};

/// The callee of a call is not a function.
pub const E3006: ErrorCode = ErrorCode {
    code: "E3006",
    title: "expression is not callable",
    explanation: "\
Only function items can be called. Seal has no first-class function type yet, so
a local binding never holds a function and cannot be used as a callee. Calling a
literal, an arithmetic result, or a local binding is therefore an error.

Name the function directly, for example `compute(1)`, or remove the call.",
};

/// A function item is used where a value is required.
pub const E3007: ErrorCode = ErrorCode {
    code: "E3007",
    title: "function item used as a value",
    explanation: "\
A function name is only meaningful in call position. Seal does not have
first-class function types, so a function item cannot be stored in a binding,
passed as an argument, returned, or used as an operand.

Call the function instead, or move the work into a function that returns the
value you need.",
};

/// A call supplies the wrong number of arguments.
pub const E3008: ErrorCode = ErrorCode {
    code: "E3008",
    title: "wrong number of arguments",
    explanation: "\
A call must supply exactly one argument for each parameter the function
declares. Seal has no default parameters, variadic parameters, or overloading,
so the count is fixed by the declaration.

Remove extra arguments or supply the missing ones. The diagnostic names both
counts, which makes an off-by-one easy to spot.",
};

/// A literal does not fit the type selected for it.
pub const E3009: ErrorCode = ErrorCode {
    code: "E3009",
    title: "literal out of range",
    explanation: "\
Integer literals take their type from a suffix or from the surrounding context,
and the value must fit that type. For example, `300u8` and `let x: i8 = 200;`
are rejected because the value is outside the type's range.

The range of `isize` and `usize` follows the target's pointer width, so the same
literal can be accepted for a 64-bit target and rejected for a 32-bit one. A
negative signed literal is allowed to name the type's minimum, as in `-128i8`.

Choose a wider type or a smaller value. Floating literals assigned to `f32` must
also remain finite when rounded to `f32`.",
};

/// A value with an unknown size is stored directly.
pub const E3010: ErrorCode = ErrorCode {
    code: "E3010",
    title: "value has an unsized type",
    explanation: "\
Locals, parameters, and return values are stored directly, so their type must
have a size known at compile time (decision D10 of the memory model). `str` is
the only unsized type in the current language: it is a sequence of bytes with a
runtime length, so it can only appear behind a reference.

Use `*str` for string data. String literals already have type `*str` and can be
stored in a binding without annotation.",
};

/// A local is read before it is definitely initialized.
pub const E3011: ErrorCode = ErrorCode {
    code: "E3011",
    title: "use of possibly-uninitialized binding",
    explanation: "\
A `let` declaration without an initializer creates storage but does not put a
value in it (decision D9 of the memory model). Reading the binding before every
path assigns a value would observe undefined contents, so the compiler rejects
it.

Assign a value on every path before the read. An `if` initializes a binding only
when both branches assign it, or when the branch that does not assign it never
reaches the code after the `if`. Assignments inside a `while` body do not count
after the loop, because the body may run zero times.",
};

/// The internal compiler error code.
pub const E9001: ErrorCode = ErrorCode {
    code: "E9001",
    title: "internal compiler error",
    explanation: "\
The compiler hit an internal invariant that should never fail. This is a bug in
Seal, not in your program. The report includes the message and, when enabled with
`SEAL_BACKTRACE=1`, a backtrace.

Please report the program that triggered the error together with the compiler
version and the backtrace.",
};

/// Too many errors were emitted.
pub const E9002: ErrorCode = ErrorCode {
    code: "E9002",
    title: "too many errors emitted",
    explanation: "\
The compiler stops after a fixed number of errors to keep the output usable. Later
errors are often consequences of the first ones, so fixing the reported errors
usually makes the rest disappear.

Fix the errors above and rebuild. The limit can be raised with `--max-errors`.",
};

/// Every registered code, in allocation order.
pub static ALL: &[ErrorCode] = &[
    E0001, E0002, E0003, E0004, E0005, E0006, E0007, E0008, E0009, E0010, E0011, E0012, E0013,
    E1001, E1002, E1003, E1004, E1005, E1006, E1007, E1008, E2001, E2002, E2003, E2004, E3001,
    E3002, E3003, E3004, E3005, E3006, E3007, E3008, E3009, E3010, E3011, E9001, E9002,
];

/// Looks up a code by its text, for example `E0001`.
pub fn lookup(code: &str) -> Option<&'static ErrorCode> {
    ALL.iter().find(|entry| entry.code == code)
}

/// Renders the `--explain` output for a code.
pub fn explain(code: &str) -> Option<String> {
    let entry = lookup(code)?;
    Some(format!(
        "{code}: {}\n\n{}\n",
        entry.title(),
        entry.explanation()
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{ALL, E0001, E0002, E0003, E0012, E1001, E1008, E9001, E9002, explain, lookup};

    #[test]
    fn codes_are_unique_and_well_formed() {
        let mut seen = BTreeSet::new();
        for entry in ALL {
            let code = entry.code();
            assert!(seen.insert(code), "duplicate code {code}");
            assert_eq!(code.len(), 5, "code {code} must have five characters");
            assert!(
                code.starts_with('E'),
                "error codes start with E, got {code}"
            );
            assert!(
                code[1..]
                    .chars()
                    .all(|character| character.is_ascii_digit()),
                "code {code} must end with four digits"
            );
            assert!(!entry.title().is_empty(), "code {code} has no title");
            assert!(
                entry.explanation().len() > 40,
                "code {code} needs a real explanation"
            );
            // The registry's format contract must match the diagnostics crate.
            assert_eq!(entry.diag_code().as_str(), code);
        }
    }

    #[test]
    fn lookup_finds_registered_codes() {
        assert_eq!(
            lookup("E0001").map(|entry| entry.title()),
            Some(E0001.title())
        );
        assert_eq!(
            lookup("E9002").map(|entry| entry.title()),
            Some(E9002.title())
        );
        assert!(lookup("E0000").is_none());
        assert!(lookup("e0001").is_none());
    }

    #[test]
    fn explain_renders_title_and_body() {
        let output = explain("E0003").expect("code exists");
        assert!(output.starts_with("E0003: could not read source file"));
        assert!(output.contains("permission"));
        assert!(explain("E7777").is_none());
    }

    #[test]
    fn codes_are_grouped_by_phase() {
        for code in [E0001, E0002, E0003, E0012] {
            assert_eq!(code.code()[1..2].parse::<u8>().expect("digit"), 0);
        }
        assert_eq!(E9001.code()[1..2].parse::<u8>().expect("digit"), 9);
        assert_eq!(E9002.code()[1..2].parse::<u8>().expect("digit"), 9);
    }

    #[test]
    fn parser_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E10"))
            .collect();
        assert_eq!(
            codes,
            [
                "E1001", "E1002", "E1003", "E1004", "E1005", "E1006", "E1007", "E1008",
            ]
        );
        assert_eq!(E1001.code()[1..2].parse::<u8>().expect("digit"), 1);
        assert_eq!(E1008.code()[1..2].parse::<u8>().expect("digit"), 1);
    }

    #[test]
    fn name_resolution_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E20"))
            .collect();
        assert_eq!(codes, ["E2001", "E2002", "E2003", "E2004"]);
    }

    #[test]
    fn type_checking_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E30"))
            .collect();
        assert_eq!(
            codes,
            [
                "E3001", "E3002", "E3003", "E3004", "E3005", "E3006", "E3007", "E3008", "E3009",
                "E3010", "E3011",
            ]
        );
    }

    #[test]
    fn lexer_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E00"))
            .collect();
        assert_eq!(
            codes,
            [
                "E0001", "E0002", "E0003", "E0004", "E0005", "E0006", "E0007", "E0008", "E0009",
                "E0010", "E0011", "E0012", "E0013",
            ]
        );
    }
}
