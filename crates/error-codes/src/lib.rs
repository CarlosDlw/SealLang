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
//! | `E7xxx` | monomorphization |
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

A keyword is reserved by the language and cannot name anything, so a program
that used one as an identifier must rename it. Identifiers start with a letter
or `_` and continue with letters, digits, and underscores; Unicode identifiers
are accepted.",
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

/// An array length is not a supported literal.
pub const E1009: ErrorCode = ErrorCode {
    code: "E1009",
    title: "invalid array length",
    explanation: "\
A fixed-size array is written `[T; N]`, where `N` is the number of elements.
Seal lengths are compile-time constants, so the grammar requires a non-negative
integer literal; at this stage there are no `const` items or constant
expressions to evaluate.

Write a literal such as `[i32; 4]` or `[0u8; 16]`. The literal may be unsuffixed
or suffixed `usize`, and must fit in a 32-bit unsigned value because the backend
represents array lengths with 32 bits.",
};

/// A variant field list is malformed.
pub const E1010: ErrorCode = ErrorCode {
    code: "E1010",
    title: "invalid variant field list",
    explanation: "\
A variant declares either positional fields (`Some(i32)`), named fields
(`Circle { radius: i32 }`), or no fields at all (`Empty`). The three forms cannot
be mixed: positional fields have no names, so a braced list after them would
have nothing to bind, and a positional list without a type has no field.

Write one form. An empty positional list is a unit variant: write `Empty` rather
than `Empty()`. A positional field is written as its type alone; names belong in
the braced form.",
};

/// A pattern is malformed.
pub const E1011: ErrorCode = ErrorCode {
    code: "E1011",
    title: "invalid pattern",
    explanation: "\
A match arm starts with a pattern. Seal patterns are `_`, a binding name, a
scalar literal (`0`, `-1`, `'x'`, `true`), a qualified enum variant such as
`Option::Some(value)`, or alternatives joined by `|`.\n\nA string cannot be a pattern because comparing text needs a runtime operation
that Seal does not define. An enum variant must always be written with its enum
name, as in `Mode::On`; a bare variant name is a binding.\n\nFix the pattern syntax, or use `_` when the arm ignores the value.",
};

/// A `const` or `static` declaration is incomplete.
pub const E1012: ErrorCode = ErrorCode {
    code: "E1012",
    title: "incomplete const or static declaration",
    explanation: "\
A `const` or `static` declaration always states its type and its initializer: the
type is the contract every use relies on, and the initializer is the value the
compiler evaluates. There is no inference from a later assignment, and the value
is never left uninitialized.

Write both, as in `const LIMIT: usize = 1024;` or `static FLAG: bool = false;`.",
};

/// A mutable static is written in an invalid form.
pub const E1013: ErrorCode = ErrorCode {
    code: "E1013",
    title: "invalid mutable static form",
    explanation: "\
`mut` is a modifier of `static`, not a separate item keyword. Seal has no
`const mut`, no `mut static`, and no second `mut`; only `static mut NAME: T =
value;` declares mutable global storage.

Write `static mut` for mutable storage and plain `static` for read-only storage.
A block-scope declaration follows the same spelling.",
};

/// A const or static declaration appears in an unsupported position.
pub const E1014: ErrorCode = ErrorCode {
    code: "E1014",
    title: "declaration is not allowed here",
    explanation: "\
A `const` or `static` item appears at file scope, and may also be a declaration
statement inside a block. `const func` is a file-scope item only, and no
const/static declaration may appear inside an expression, a parameter list, or
a trait implementation body.

Move the declaration to file scope, or use a block-scope `const`/`static`
statement where a local value is intended.",
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

/// Two nominal types share one name.
pub const E2005: ErrorCode = ErrorCode {
    code: "E2005",
    title: "type is defined twice",
    explanation: "\
Struct names live in the type namespace and must be unique in a program. Two
structs with the same name would make every written type ambiguous.

Rename one of the declarations. A struct and a function may share a name,
because they live in different namespaces.",
};

/// A struct declares the same field name twice.
pub const E2006: ErrorCode = ErrorCode {
    code: "E2006",
    title: "field is declared twice",
    explanation: "\
Each field name identifies exactly one field inside a struct. Declaring the name
twice would make field access and struct literals depend on declaration order.

Rename or remove one of the fields.",
};

/// A field access or literal names a field the struct does not have.
pub const E2007: ErrorCode = ErrorCode {
    code: "E2007",
    title: "no such field",
    explanation: "\
Field access and struct literals name fields of a known struct type. A name that
the struct does not declare cannot refer to storage.

Check the spelling against the struct declaration. Fields are resolved by name,
so a misspelled field is rejected even when another field has a similar type.",
};

/// A struct literal mentions one field more than once.
pub const E2008: ErrorCode = ErrorCode {
    code: "E2008",
    title: "field is specified twice",
    explanation: "\
A struct literal builds one value per field. Specifying a field twice would make
the result depend on which initializer wins, so the compiler rejects it.

Write each field exactly once.",
};

/// A struct literal omits a field.
pub const E2009: ErrorCode = ErrorCode {
    code: "E2009",
    title: "missing field in struct literal",
    explanation: "\
A struct value must initialize every field, because safe code never observes
uninitialized storage. Seal has no default values and no `..base` update syntax,
so each field appears in the literal.

Add the missing initializer.",
};

/// A type name is used where a value is required.
pub const E2010: ErrorCode = ErrorCode {
    code: "E2010",
    title: "type name used as a value",
    explanation: "\
A struct name introduces a type, not a value: it has no size and cannot be
stored. Constructing a value requires a struct literal such as `Point { x: 1 }`.

Use the name in a type position, or write a literal in a value position.",
};

/// A value name is used where a type is required.
pub const E2011: ErrorCode = ErrorCode {
    code: "E2011",
    title: "expected a type, found a value",
    explanation: "\
Type positions accept primitive types, references, and struct names. A function
or binding is a value and does not name a type, so it cannot appear in an
annotation, parameter, field, or return type.

Replace the name with a type, such as `i32` or `*Point`.",
};

/// A field is accessed on a value that is not a place.
pub const E2012: ErrorCode = ErrorCode {
    code: "E2012",
    title: "field access requires a place",
    explanation: "\
Fields live inside storage: a binding, a parameter, a dereference, or another
field. A computed value such as a struct literal or a call result has no storage
to project, and Seal does not create hidden temporaries for field access.

Store the value in a `let` binding and access the field through that binding.",
};

/// An index expression is applied to a value with no storage.
pub const E2013: ErrorCode = ErrorCode {
    code: "E2013",
    title: "indexing requires a place",
    explanation: "\
An array index selects storage inside an array: a binding, a parameter, a
dereference, a field, or another index. A computed value such as an array
literal or a call result has no storage to project, and Seal does not create
hidden temporaries for indexing.

Store the array in a `let` binding and index that binding.",
};

/// The built-in `len` received the wrong number of arguments.
pub const E2014: ErrorCode = ErrorCode {
    code: "E2014",
    title: "wrong number of arguments to `len`",
    explanation: "\
`len` is a built-in operator that reads the length stored in a slice reference.
It takes exactly one argument, the `*[T]` or `*mut [T]` value whose length is
needed; it has no second form and no optional arguments.

Pass one slice reference, as in `len(values)`, or remove the extra arguments.
A user function named `len` shadows the built-in and is called normally.",
};

/// An `impl` block names a type that cannot receive inherent methods.
pub const E2015: ErrorCode = ErrorCode {
    code: "E2015",
    title: "invalid impl target",
    explanation: "\
An implementation attaches methods to one nominal struct declared in the same
module. The name after `impl` must be that struct, not a primitive, a reference,
a function, or an undeclared name. The `impl Trait for Type` form names the
trait first and the struct after `for`; the struct is the target validated here.\n\nDeclare the struct with `struct`, then write `impl StructName { ... }` or
`impl TraitName for StructName { ... }`.",
};

/// A method receiver has an invalid declaration.
pub const E2016: ErrorCode = ErrorCode {
    code: "E2016",
    title: "invalid method receiver",
    explanation: "\
A method is an impl function whose first parameter is named `self`. The receiver
must be written with its type: `self: Point` takes the value, `self: *Point`
borrows it for reading, and `self: *mut Point` borrows it for writing. The type
must be the struct named by the impl block. `self` cannot appear in any other
position.\n\nWrite the receiver as the first parameter, or remove `self` to make the
function associated.",
};

/// One impl block declares the same function name twice.
pub const E2017: ErrorCode = ErrorCode {
    code: "E2017",
    title: "associated function is defined twice",
    explanation: "\
Methods and associated functions share one namespace per type. Two functions
with the same name in an impl block, or across several impl blocks for the same
type, are ambiguous: a call could not choose one.\n\nRename one of the functions, or remove it.",
};

/// A qualified path names a function the type does not define.
pub const E2018: ErrorCode = ErrorCode {
    code: "E2018",
    title: "no such associated function",
    explanation: "\
A qualified path such as `Point::new` names an associated function declared in
an impl block for `Point`, either inherent or as part of a trait implementation.
The name must be one of those functions, and it must not take a `self`
receiver.\n\nCheck the name and the impl blocks, or call an instance method on a value.",
};

/// A qualified path names an instance method and omits its receiver.
pub const E2019: ErrorCode = ErrorCode {
    code: "E2019",
    title: "method called as an associated function",
    explanation: "\
The function named by this qualified path is an instance method, because its
first parameter is `self`. It needs a receiver value and is called with `.`, as
in `point.sum()`.\n\nCall the method on a value, or make the function associated by removing its
`self` parameter.",
};

/// An `impl Trait for Type` block names a trait that is not declared.
pub const E2020: ErrorCode = ErrorCode {
    code: "E2020",
    title: "cannot find the trait for this impl",
    explanation: "\
An impl block with the `Trait for Type` form names a trait declared in the same
file. The name after `impl` must be that trait, not a struct, a function, or an
undeclared name.\n\nDeclare the trait with `trait`, then write `impl Trait for Type { ... }`. An
inherent impl omits the trait and keeps only the type: `impl Type { ... }`.",
};

/// One trait is implemented twice for the same type.
pub const E2021: ErrorCode = ErrorCode {
    code: "E2021",
    title: "trait is implemented twice for one type",
    explanation: "\
A type has exactly one implementation of each trait. Two `impl Trait for Type`
blocks would define the trait's methods twice, and a call would have no way to
choose between them.\n\nRemove one of the impl blocks, or merge their functions into a single block.",
};

/// An impl block omits a method the trait declares.
pub const E2022: ErrorCode = ErrorCode {
    code: "E2022",
    title: "trait method is not implemented",
    explanation: "\
A trait method may declare a default body. An impl may omit only those methods
and inherits the body; a method without a default must be supplied, with the
name and the signature the trait declares.\n\nAdd the missing function to the impl block, give the trait method a default
body, or remove the method from the trait declaration.",
};

/// An impl block defines a function the trait does not declare.
pub const E2023: ErrorCode = ErrorCode {
    code: "E2023",
    title: "function is not declared by the trait",
    explanation: "\
An `impl Trait for Type` block implements exactly the methods of that trait. A
function the trait does not declare does not belong to the trait implementation:
other code would never reach it through the trait.\n\nMove the function into an inherent `impl Type` block, or declare it in the trait
first.",
};

/// An impl function does not match the trait signature.
pub const E2024: ErrorCode = ErrorCode {
    code: "E2024",
    title: "trait method does not match its declaration",
    explanation: "\
An implementation must reproduce every method signature exactly: the same
receiver kind, the same parameter types in the same order, and the same return
type. Seal has no overloading and no implicit conversions between method
parameter types, so a mismatch would make the trait's guarantee untrue for one
implementor.\n\nIn the trait, write the implementing type as `Self`; in the impl, write the
concrete struct name. Adjust the receiver or the parameter and return types to
match the declaration.",
};

/// `Self` appears outside a trait method signature.
pub const E2025: ErrorCode = ErrorCode {
    code: "E2025",
    title: "`Self` used outside a trait declaration",
    explanation: "\
`Self` names the type that implements a trait. It only has a meaning inside a
trait method signature, where it is replaced with the concrete type when an impl
is checked. Free functions, inherent impls, struct fields, and local annotations
have no implementing type, so `Self` there has nothing to name.\n\nWrite the concrete type instead: `Point::new()` or `self: *Point`.",
};

/// One trait declaration lists the same method name twice.
pub const E2026: ErrorCode = ErrorCode {
    code: "E2026",
    title: "trait method is declared twice",
    explanation: "\
A trait is a set of method names, and each name identifies exactly one signature.
Declaring a name twice would make the trait's requirement depend on declaration
order and would ask implementors for two functions with one name.\n\nRename one of the methods, or remove the duplicate declaration.",
};

/// A trait name is used as a value or as a type.
pub const E2027: ErrorCode = ErrorCode {
    code: "E2027",
    title: "trait name used as a value or type",
    explanation: "\
A trait declares method signatures: it is not a value, and it is not a type a
value can have. Trait calls resolve statically to the implementation of the
receiver's concrete type, so there is no trait object to store or pass.\n\nName a struct in type position and call a trait method on a value of that
struct. Associated functions declared by a trait are called as `Type::name()`.",
};

/// A call names a method declared by more than one implemented trait.
pub const E2028: ErrorCode = ErrorCode {
    code: "E2028",
    title: "ambiguous trait method",
    explanation: "\
A type may implement several traits, and two of them may declare a method with
the same name. A call through the value's type then has no single target, and
Seal has no syntax for naming the trait, so the call is rejected instead of
picking one by declaration order.\n\nRename one of the trait methods, or remove one of the impl blocks. An inherent
method with the same name takes precedence over every trait method and is not
ambiguous.",
};

/// An enum declares the same variant name twice.
pub const E2029: ErrorCode = ErrorCode {
    code: "E2029",
    title: "enum variant is declared twice",
    explanation: "\
A variant name identifies exactly one variant inside an enum. Declaring the name
twice would make construction, layout order, and pattern matching depend on
declaration order, so the compiler rejects it.\n\nRename or remove one of the variants. Different enums may reuse a variant name,
and a variant name is independent of the field names it carries.",
};

/// A variant name collides with an associated function of the same enum.
pub const E2030: ErrorCode = ErrorCode {
    code: "E2030",
    title: "variant name conflicts with an associated function",
    explanation: "\
`Type::name` resolves through one namespace: it either names an enum variant or
an associated function. Allowing both would make `Type::name` ambiguous and the
result depend on lookup order, so the compiler rejects the collision.\n\nRename the variant or the function.",
};

/// A variant literal starts with a struct name.
pub const E2031: ErrorCode = ErrorCode {
    code: "E2031",
    title: "variant literal names a struct",
    explanation: "\
`Type::Variant { ... }` constructs an enum value: the name before `::` must be an
enum and the name after it one of its variants. A struct has no variants, so the
literal cannot name a value.\n\nConstruct the struct with a plain literal such as `Point { x: 1 }`, or check
that the type before `::` is the enum you meant.",
};

/// A variant literal names a variant the enum does not declare.
pub const E2032: ErrorCode = ErrorCode {
    code: "E2032",
    title: "no such enum variant",
    explanation: "\
A variant literal names a variant of the enum written before `::`. The enum does
not declare that name, so there is no discriminant or field list to use.\n\nCheck the spelling against the enum declaration. Variants are resolved by name,
not by position.",
};

/// A unit variant is constructed with fields.
pub const E2033: ErrorCode = ErrorCode {
    code: "E2033",
    title: "unit variant cannot have fields",
    explanation: "\
A unit variant carries no data: its value is just the discriminant. Writing
`{ ... }` after it would silently discard the initializers and the expressions
that produce them, so the compiler rejects it.\n\nRemove the braces, or declare fields on the variant if it needs to carry data.",
};

/// A variant with fields is written without an initializer list.
pub const E2034: ErrorCode = ErrorCode {
    code: "E2034",
    title: "variant fields are missing",
    explanation: "\
A variant that declares fields must be constructed with its field list: a braced
initializer list for named fields (`Shape::Circle { radius: 1 }`) or
parenthesized arguments for positional fields (`Option::Some(1)`). The bare path
would leave the fields uninitialized, and safe Seal code never observes
uninitialized storage.\n\nAdd the matching field list, or use the variant that carries no fields.",
};

/// A struct literal names an enum.
pub const E2035: ErrorCode = ErrorCode {
    code: "E2035",
    title: "struct literal names an enum",
    explanation: "\
`Name { field: value }` builds a struct. An enum value always carries a
discriminant, so it is built one variant at a time; a literal naming the enum
itself has no variant to store and is rejected.\n\nWrite `Name::Variant { ... }` for named fields, `Name::Variant(...)` for
positional fields, or `Name::Variant` for a unit variant.",
};

/// A positional variant is constructed with named fields.
pub const E2036: ErrorCode = ErrorCode {
    code: "E2036",
    title: "positional variant uses named fields",
    explanation: "\
A variant declared with positional fields, such as `Option::Some(i32)`, takes
positional arguments at construction. Writing `{ ... }` would bind them by name
that the declaration does not have, so the compiler rejects the expression.\n\nWrite the values between parentheses, as in `Option::Some(1)`.",
};

/// A named variant is constructed with positional arguments.
pub const E2037: ErrorCode = ErrorCode {
    code: "E2037",
    title: "named variant uses positional arguments",
    explanation: "\
A variant declared with named fields, such as `Shape::Circle { radius: i32 }`,
takes a braced initializer list. Parenthesized arguments would have no field
names to bind them to, so the compiler rejects the expression.\n\nWrite the initializer list, as in `Shape::Circle { radius: 1 }`.",
};

/// A positional variant receives the wrong number of arguments.
pub const E2038: ErrorCode = ErrorCode {
    code: "E2038",
    title: "wrong number of variant fields",
    explanation: "\
A positional variant is a constructor whose arity is fixed by its declaration.
Every field must receive exactly one argument, in declaration order, because
there are no names to supply them by.\n\nSupply one value per declared field. The diagnostic names the expected and the
actual count.",
};

/// A unit variant is called with parentheses.
pub const E2039: ErrorCode = ErrorCode {
    code: "E2039",
    title: "unit variant is written without parentheses",
    explanation: "\
A unit variant carries no payload, so it is a value rather than a constructor.
Writing `Variant()` would suggest a callable that takes no arguments and would
give the same value two spellings.\n\nWrite the variant without parentheses, as in `Mode::On`.",
};

/// A binding name appears twice in one pattern.
pub const E2040: ErrorCode = ErrorCode {
    code: "E2040",
    title: "binding name appears twice in one pattern",
    explanation: "\
A pattern introduces each binding name at most once. `Some(value, value)` would
ask one name to refer to two fields, so the compiler cannot tell which value a
later use means. A duplicate is rejected even when both fields have the same
type.\n\nRename one binding, or replace it with `_` if the field is not used.",
};

/// Or-pattern alternatives bind different names.
pub const E2041: ErrorCode = ErrorCode {
    code: "E2041",
    title: "or-pattern alternatives bind different names",
    explanation: "\
Every alternative of an or-pattern must bind the same set of names with the same
types, because the arm body runs after whichever alternative matched and has one
set of names available. `Some(x) | None` is rejected: `None` has no `x` to
supply.\n\nUse `_` in alternatives that do not need a binding, or bind the same names on
every side.",
};

/// A variant pattern uses the wrong field form.
pub const E2042: ErrorCode = ErrorCode {
    code: "E2042",
    title: "variant pattern uses the wrong field form",
    explanation: "\
A variant pattern mirrors the variant declaration. A unit variant is written
bare (`Mode::Off`), a positional variant with parentheses
(`Option::Some(value)`), and a named variant with braces
(`Shape::Circle { radius }`).\n\nWriting fields on a unit variant, or the wrong delimiter for the declared form,
is rejected instead of silently ignoring a field pattern.",
};

/// A variant pattern has the wrong number of fields.
pub const E2043: ErrorCode = ErrorCode {
    code: "E2043",
    title: "wrong number of fields in variant pattern",
    explanation: "\
Every field of a positional variant must be matched by exactly one pattern.
Omitting a field would leave part of the value unaccounted for, and supplying
extra patterns would have nothing to match.\n\nWrite one pattern per declared field, in declaration order, and use `_` for a
field the arm ignores. A named variant pattern must name every field unless it
ends with `..`, which ignores the remaining fields.",
};

/// A named variant pattern names an unknown field.
pub const E2044: ErrorCode = ErrorCode {
    code: "E2044",
    title: "no such field in variant pattern",
    explanation: "\
A named pattern must use the field names the variant declares. A name that the
declaration does not contain cannot match anything and is usually a typo.\n\nCheck the field spelling against the variant declaration.",
};

/// A guard reads a binding that is not `Copy`.
pub const E2045: ErrorCode = ErrorCode {
    code: "E2045",
    title: "guard cannot use a non-`Copy` binding",
    explanation: "\
A guard runs after a pattern matches but before the arm commits. If the guard
fails, matching continues to the next arm, so a value the guard consumed could
not be put back. Seal therefore allows only bindings whose type is `Copy` in the
guard expression of an arm.\n\nMove the check into the arm body with an `if`, match the value structurally
without a guard, or use a `Copy` field.",
};

/// A struct pattern names a field the struct does not declare.
///
/// A struct pattern mirrors the declaration: every field it names must exist.
/// A misspelled field is rejected instead of silently ignored.
pub const E2046: ErrorCode = ErrorCode {
    code: "E2046",
    title: "no such field in struct pattern",
    explanation: "\
A struct pattern names fields of a known struct type. A name the struct does
not declare cannot select storage and is usually a typo.\n\nCheck the spelling against the struct declaration, or use `_` for a field the
arm ignores.",
};

/// A struct pattern names the same field twice.
pub const E2047: ErrorCode = ErrorCode {
    code: "E2047",
    title: "field is matched twice in a struct pattern",
    explanation: "\
A struct pattern selects each field exactly once. Matching a field twice would
make the arm depend on which pattern wins, so the compiler rejects it.\n\nRemove one of the patterns.",
};

/// A struct pattern omits a declared field.
pub const E2048: ErrorCode = ErrorCode {
    code: "E2048",
    title: "missing field in struct pattern",
    explanation: "\
A struct pattern matches a whole struct value, so it must account for every
field. Write the field pattern, use `_` to ignore the field without binding it,
or end the pattern with `..` to ignore every field it did not name.\n\nThis rule matches struct literals: a `let` binding cannot read a field, only a
pattern decides what a field does.",
};

/// An array pattern does not match the array length.
pub const E2049: ErrorCode = ErrorCode {
    code: "E2049",
    title: "array pattern has the wrong length",
    explanation: "\
A fixed-size array has a length that is part of its type, so an array pattern
must write exactly one pattern per element. A pattern with a different number of
elements cannot match any value of the array.\n\nWrite one pattern per element, using `_` for the elements the arm ignores.",
};

/// A range pattern is empty.
pub const E2050: ErrorCode = ErrorCode {
    code: "E2050",
    title: "empty range pattern",
    explanation: "\
A range pattern matches the values between its endpoints. When the start is
greater than the end (or equal, for an exclusive `..` range), no value can match,
so the arm is dead code.\n\nSwap the endpoints, or remove the arm.",
};

/// A generic parameter is declared twice in one scope.
pub const E2051: ErrorCode = ErrorCode {
    code: "E2051",
    title: "duplicate generic parameter",
    explanation: "\
A generic parameter name must be unique in the scope where it is visible. A
method's parameters are flattened after its impl's parameters, so a method may
not redeclare an impl parameter with the same name.\n\nRename one of the parameters, or remove the duplicate declaration.",
};

/// A generic argument list has the wrong number of arguments.
pub const E2052: ErrorCode = ErrorCode {
    code: "E2052",
    title: "wrong number of generic arguments",
    explanation: "\
A generic declaration fixes how many type arguments every use must supply. The
diagnostic points at the argument list, not at the declaration.\n\nSupply exactly one argument per declared parameter, or remove the argument list
when the declaration is not generic.",
};

/// Generic arguments were written on a non-generic declaration.
pub const E2053: ErrorCode = ErrorCode {
    code: "E2053",
    title: "generic arguments on a non-generic declaration",
    explanation: "\
Only declarations with type parameters accept type arguments. Writing `::<T>`
on a function, method, type, or literal that declares no parameters has no
meaning.\n\nRemove the type arguments, or add the parameters to the declaration.",
};

/// A generic argument is not a known type parameter.
pub const E2054: ErrorCode = ErrorCode {
    code: "E2054",
    title: "unknown type parameter",
    explanation: "\
A bound or type argument referred to a name that is not a declared type
parameter and not a known type. Type parameters are only visible in the item
that declares them and in its nested methods.\n\nDeclare the parameter, or use a concrete type instead.",
};

/// A type parameter is used outside the scope that declares it.
pub const E2055: ErrorCode = ErrorCode {
    code: "E2055",
    title: "type parameter used outside its scope",
    explanation: "\
A type parameter belongs to one declaration. A parameter declared by a struct,
enum, trait, impl, function, or method is visible only inside that declaration
and, for an impl or trait, its methods.\n\nUse a parameter declared by the current item, or pass the type through a\ngeneric argument.",
};

/// An impl block has an invalid target type.
pub const E2056: ErrorCode = ErrorCode {
    code: "E2056",
    title: "invalid impl target",
    explanation: "\
An impl block must target a type that can receive methods. Inherent impls may
target a struct or enum declared in this file. Trait impls may also target a
sized primitive, array, tuple, or reference.\n\nUse a valid target type, or declare the type with `struct` or `enum`.",
};

/// A bound or impl names an invalid trait path.
pub const E2057: ErrorCode = ErrorCode {
    code: "E2057",
    title: "invalid trait path",
    explanation: "\
A trait path must name a trait declared in this file. A struct or enum in a
bound position is not a trait, and a trait with missing or extra type arguments
cannot be used as a bound.\n\nName the trait, and supply exactly its declared type arguments.",
};

/// Two impls overlap or a trait is implemented twice.
pub const E2058: ErrorCode = ErrorCode {
    code: "E2058",
    title: "overlapping implementations",
    explanation: "\
Each trait implementation must be distinct. Two impl blocks that provide the
same trait for the same self type overlap, so a call could have more than one
candidate.\n\nRemove one implementation, or change its self type or trait arguments.",
};

/// `Self` appears where no self type exists.
pub const E2059: ErrorCode = ErrorCode {
    code: "E2059",
    title: "`Self` outside a trait or impl",
    explanation: "\
`Self` names the implementing type of a trait or impl. It has no meaning in a
free function or in a declaration that is not inside a trait or impl.\n\nWrite the concrete type instead, or move the declaration into an impl.",
};

/// A bound target is not a trait.
pub const E2060: ErrorCode = ErrorCode {
    code: "E2060",
    title: "bound target is not a trait",
    explanation: "\
A bound states that a type implements a trait. A struct, enum, primitive, or
other non-trait name cannot appear in bound position.\n\nDeclare the requirement as a trait, then implement it for the type.",
};

/// A generic parameter shadows an incompatible namespace.
pub const E2061: ErrorCode = ErrorCode {
    code: "E2061",
    title: "generic parameter shadows a built-in type",
    explanation: "\
A generic parameter lives in the type namespace. A parameter cannot reuse the
spelling of a built-in type such as `i32`, `bool`, or `str`, because every use
of that spelling would resolve to the built-in type instead of the parameter.\n\nRename the parameter.",
};

/// A where-clause predicate targets something other than a type parameter.
pub const E2062: ErrorCode = ErrorCode {
    code: "E2062",
    title: "where-clause target is not a type parameter",
    explanation: "\
The generic phase records where-clause bounds on type parameters. A predicate
whose target is a concrete type cannot be stored in the parameter environment
and is rejected here until the trait solver can prove it.\n\nMove the bound to a type parameter, or remove the predicate.",
};

/// An associated function is declared by several impls and cannot be chosen.
pub const E2063: ErrorCode = ErrorCode {
    code: "E2063",
    title: "ambiguous associated function",
    explanation: "\
Several inherent impl blocks for one generic type declare an associated
function with this name. The blocks apply to different instances, so the call
needs type arguments to select one; without them there is no single target.
\n\nWrite the type arguments on the aggregate, as in `Type::<i32>::function()`, or
remove one of the declarations.",
};

/// A trait's supertrait relation contains a cycle.
pub const E2064: ErrorCode = ErrorCode {
    code: "E2064",
    title: "cyclic supertrait relation",
    explanation: "\
A trait may require its implementors to implement other traits. When the
requirement graph contains a cycle, proving one bound would require proving the
same bound again, so no implementation could ever satisfy it.\n\nRemove one of the supertrait requirements that closes the cycle.",
};

/// A trait cannot be used as a trait object.
pub const E2065: ErrorCode = ErrorCode {
    code: "E2065",
    title: "trait is not dyn-compatible",
    explanation: "\
A `dyn Trait` type needs a finite vtable, so every method must be callable
without knowing the concrete type. A trait is dyn-compatible only when all of
its supertraits are dyn-compatible and every method can be dispatched. A method
is dispatchable when it takes `*Self` or `*mut Self`, has no type parameters of
its own, does not mention `Self` outside the receiver, and is not an associated
function. A supertrait argument may not mention `Self` either.\n\nA trait that is not dyn-compatible can still be used for static dispatch. To
form a trait object, change the offending methods to pointer receivers, remove
method type parameters and `Self` from parameter and return types, make every
supertrait dyn-compatible, and remove associated functions from the
dispatchable surface.",
};

/// A user trait redeclares a compiler-provided protocol trait.
pub const E2066: ErrorCode = ErrorCode {
    code: "E2066",
    title: "reserved protocol trait name",
    explanation: "\
`Iterator`, `IntoIterator`, `Index`, and `IndexMut` are provided by the
compiler so that `for` loops and `[]` have the same meaning in every file.
Declaring a trait with one of those names would replace or conflict with the
operator contract.\n\nImplement the existing protocol trait for your type instead of declaring a new
trait with the same name. If the behavior is unrelated to operators, choose a
different trait name.",
};

/// A protocol implementation targets a compiler-owned sequence type.
pub const E2067: ErrorCode = ErrorCode {
    code: "E2067",
    title: "protocol implementation targets a built-in sequence",
    explanation: "\
`for` loops and `[]` already define iteration and indexing for arrays and
slices, including references to them. A protocol implementation for one of
those targets would have no effect and could make the operator's behavior
depend on an implementation that the compiler deliberately ignores.\n\nWrap the sequence in a struct or enum and implement the protocol for the
wrapper. The built-in behavior remains available through the wrapped value.",
};

/// A value name is declared twice in the same scope.
pub const E2068: ErrorCode = ErrorCode {
    code: "E2068",
    title: "duplicate value name",
    explanation: "\
Functions, `const` names, `static` names, parameters, and bindings share one
value namespace. Declaring two of them with the same name in one scope makes
every use ambiguous, so the compiler reports the second declaration instead of
letting declaration order decide.

A block-scope declaration may shadow a name from an outer scope; only a
duplicate inside the same scope is rejected. Rename one of the two definitions.",
};

/// Expression types do not match.
pub const E3001: ErrorCode = ErrorCode {
    code: "E3001",
    title: "mismatched types",
    explanation: "\
Seal has one implicit conversion: a thin pointer to a sized concrete value may
become a trait-object pointer of the same reference kind. Apart from that, the
type an expression has must be exactly the type its context requires. This
applies to initializers, assignments, arguments, returns, conditions, and the
two branches of an `if`.

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
integers; `&&`, `||`, and `!` accept `bool`; ordering comparisons accept two
equal numeric or `char` values. `==` and `!=` also accept `bool`, `()`, arrays,
tuples, structs, and enums whose fields are themselves comparable, plus `*str`
and `*[T]`, which compare contents. Thin references and trait objects have no
equality: a reference would compare addresses, and a trait object erases its
concrete type.

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
have a size known at compile time (decision D10 of the memory model). `str`,
slices, and `dyn Trait` are unsized: a string or slice carries a runtime length,
and a trait object has no known concrete layout, so each can only appear behind
a reference.

Use `*str` for string data, `*[T]`/`*mut [T]` for a borrowed sequence, and
`*dyn Trait`/`*mut dyn Trait` for dynamic dispatch. String literals already have
type `*str`, and a range such as `&values[..]` produces a slice reference, so
both can be stored in a binding without an annotation. A trait-object pointer is
created by coercing a pointer to a concrete implementation.",
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

/// A dereference is applied to a value that is not a reference.
pub const E3012: ErrorCode = ErrorCode {
    code: "E3012",
    title: "cannot dereference this type",
    explanation: "\
The `*` prefix reads the value a reference points to, so its operand must have a
reference type `*T`. Seal has no raw pointers, no nullable references, and no
implicit dereference of other types.

Write `*reference` where `reference` is a parameter or binding of type `*T`, or
remove the `*`.",
};

/// A borrow is applied to an expression that is not a place.
pub const E3013: ErrorCode = ErrorCode {
    code: "E3013",
    title: "cannot borrow this expression",
    explanation: "\
`&` borrows a place: a binding, a parameter, or a dereference of a reference.
Borrowing a computed value such as `a + b` or a literal would name storage that
no binding owns, and Seal does not create hidden temporaries for borrows.

Store the value in a `let` binding and borrow that binding instead.",
};

/// An assignment writes through a shared reference.
pub const E3014: ErrorCode = ErrorCode {
    code: "E3014",
    title: "cannot write through a shared reference",
    explanation: "\
`*T` is a shared, read-only reference (decision D4 of the memory model), so
assigning through it is rejected. Only a `*mut T` reference permits writes.

Write through a `*mut T` reference, or write to the borrowed place directly by
name.",
};

/// A field is accessed on a type that has no fields.
pub const E3015: ErrorCode = ErrorCode {
    code: "E3015",
    title: "cannot access a field on this type",
    explanation: "\
Only struct values have named fields. Accessing `.x` on a scalar, a reference,
or a type with no such field cannot name storage. Seal has no automatic
dereference: a reference must be dereferenced explicitly before a field is
accessed.

Write `(*reference).field` for a reference, and check the field name against the
struct declaration.",
};

/// A struct contains itself by value.
pub const E3016: ErrorCode = ErrorCode {
    code: "E3016",
    title: "recursive type has infinite size",
    explanation: "\
A struct stored by value contains its fields inline, so a struct that contains
itself by value would need infinite storage. Recursion is allowed through a
reference, because the reference has a fixed size and the value lives elsewhere.

Store the recursive field behind a reference such as `*Node`, or break the cycle
by removing one of the fields.",
};

/// A borrow asks for exclusive access to a read-only place.
pub const E3017: ErrorCode = ErrorCode {
    code: "E3017",
    title: "cannot borrow as mutable",
    explanation: "\
`&mut place` promises exclusive access: the borrower may write the place, and no
other loan of it may overlap. A binding that is not declared `mut` does not
permit writes, and a place behind a shared reference (`*T`) is read-only
(decision D4), so neither can be borrowed exclusively.

Declare the binding with `let mut`, or borrow through a `*mut T` reference so the
place itself permits writes.",
};

/// A value is moved out of a referenced place.
pub const E3018: ErrorCode = ErrorCode {
    code: "E3018",
    title: "cannot move out of a reference",
    explanation: "\
Reading `*reference` by value moves the pointee when its type is not `Copy`, but
the pointee belongs to whoever created the reference: moving it would leave that
storage uninitialized behind the owner's back. Only `Copy` values may be read
through a reference by value.

Borrow the pointee instead, as in `&*reference`, or copy the fields you need into
a new value. The same rule applies to an element bound by a slice pattern: only
`Copy` elements may be bound, because the slice still owns its storage. Struct
and variant patterns that match through a reference follow the same rule: only
`Copy` fields may be bound by name.",
};

/// An empty array literal has no element type to infer.
pub const E3019: ErrorCode = ErrorCode {
    code: "E3019",
    title: "cannot infer array element type",
    explanation: "\
An empty array literal `[]` contains no value that could determine its element
type. Unlike a non-empty literal, there is no first element whose type the rest
of the array can follow.

Give the binding, parameter, or return position a type annotation such as
`[i32; 0]`, or write at least one element.",
};

/// A repeated array literal would copy a non-`Copy` value.
pub const E3020: ErrorCode = ErrorCode {
    code: "E3020",
    title: "cannot repeat a non-`Copy` value",
    explanation: "\
`[value; N]` builds an array by copying `value` into every element. A type that
is not `Copy` owns its contents and cannot be duplicated bitwise, so repeating
it would create several owners of the same storage.

Use an explicit literal when each element can be constructed separately. Struct
values are not `Copy` in Seal, while scalars, references, and arrays of `Copy`
elements are.",
};

/// An index expression is applied to a type that is not an array.
pub const E3021: ErrorCode = ErrorCode {
    code: "E3021",
    title: "cannot index this type",
    explanation: "\
The `base[index]` syntax selects an element of a fixed-size array or a slice, or
calls a custom `Index<Key, Value>`/`IndexMut<Key, Value>` implementation. A
scalar, a struct without the protocol, and a string have no element to select.

Use an array, slice, or a type with a matching protocol implementation. Built-in
arrays and slices use a `usize` index with a bounds check; a custom
implementation chooses its key and value types. If the base is a reference,
indexing follows it before selecting the built-in or protocol path.",
};

/// A constant array index is outside the array bounds.
pub const E3022: ErrorCode = ErrorCode {
    code: "E3022",
    title: "array index is out of bounds",
    explanation: "\
An array of length `N` has valid indices `0..N`. When the index is a literal,
the compiler knows the value and rejects it before the program runs, instead of
leaving a guaranteed panic in the generated code.

The last valid index is `length - 1`. For an empty array, no index is valid.",
};

/// An array is too large for the target's address space.
pub const E3023: ErrorCode = ErrorCode {
    code: "E3023",
    title: "array is too large for the target",
    explanation: "\
The size of an array is the element size multiplied by its length. On a target
with 32-bit pointers, a type whose size exceeds `usize::MAX` cannot be stored in
memory or addressed by one pointer.

Use a smaller length or a smaller element type. The check runs against the
selected target, not the host.",
};

/// A slice operation is invalid for the value's type or mutability.
pub const E3024: ErrorCode = ErrorCode {
    code: "E3024",
    title: "invalid slice operation",
    explanation: "\
A slice is an unsized view over contiguous elements. It is created by borrowing
a range, such as `&values[start..end]`, and only a `*[T]` or `*mut [T]` can be
stored or passed. A bare range is unsized, only arrays and slice references can
be sliced, and a mutable slice requires a writable source.

Borrow the range, make sure the base is an array or slice, and use a `*mut`
source or a `mut` binding when the slice is mutable.",
};

/// The `len` built-in received a value that is not a slice reference.
pub const E3025: ErrorCode = ErrorCode {
    code: "E3025",
    title: "`len` requires a slice reference",
    explanation: "\
`len` reads the length word of a slice reference. Values of other types do not
carry a runtime length, so the compiler cannot answer without a slice.

Create a slice first, as in `len(&values[..])`, and pass that reference.",
};

/// A constant slice range has its start after its end.
pub const E3026: ErrorCode = ErrorCode {
    code: "E3026",
    title: "slice range is empty because start exceeds end",
    explanation: "\
A range selects elements from `start` up to, but not including, `end`. When both
bounds are literals and `start > end`, the range can never select anything, so
the compiler reports it instead of leaving a guaranteed runtime panic.

Write a range whose start is at most its end, or swap the bounds.",
};

/// A method name is not available for the receiver type.
pub const E3027: ErrorCode = ErrorCode {
    code: "E3027",
    title: "no such method",
    explanation: "\
A struct method comes from an `impl` block for that struct, a trait method from\nan implemented trait, and an intrinsic method from the compiler-owned scalar,\narray, or slice catalog. On a trait object, lookup covers the principal trait\nand every supertrait; a name none of them declares cannot be called.\n\nCheck the spelling, the impl block, and the traits in scope, or dereference the\nreceiver until it reaches the type that defines the method. The intrinsic\ncatalog documents every compiler-owned name. If a supertrait declares the name\nbut the principal trait does not, call it through a `*dyn` pointer whose\nprincipal trait is that supertrait.",
};

/// A method call supplies the wrong number of arguments.
pub const E3028: ErrorCode = ErrorCode {
    code: "E3028",
    title: "wrong number of method arguments",
    explanation: "\
Methods have fixed signatures. A call must supply one argument for each\nparameter after the receiver; there are no defaults or overloads. An associated\nfunction has no receiver, so all of its parameters are call arguments.\n\nPass exactly the number of arguments in the selected declaration or intrinsic\nsignature.",
};

/// A modifying method has a receiver that cannot be written.
pub const E3029: ErrorCode = ErrorCode {
    code: "E3029",
    title: "method receiver is not writable",
    explanation: "\
A method with an exclusive receiver, and an intrinsic such as `reverse`, `fill`,\nor `toggle`, modifies its receiver. It requires a mutable binding, or a place\nreached through a `*mut T` reference. A shared reference, an immutable binding,\nand a shared trait object (`*dyn Trait`) do not permit the write.\n\nDeclare the binding with `let mut`, call the method through a mutable reference,\nor use a `*mut dyn Trait` receiver when dispatching dynamically.",
};

/// `fill` is called on a sequence whose element type is not `Copy`.
pub const E3030: ErrorCode = ErrorCode {
    code: "E3030",
    title: "`fill` requires a `Copy` element",
    explanation: "\
`fill(value)` writes the same value into every element. Writing a non-`Copy`\nvalue into several elements would duplicate ownership, which Seal forbids.\n\nUse a `Copy` element type, or assign each element separately. `swap` and\n`reverse` move elements between positions and do not require `Copy`.",
};

/// A method receiver does not have the type or adjustment the method expects.
pub const E3031: ErrorCode = ErrorCode {
    code: "E3031",
    title: "method receiver has an incompatible type",
    explanation: "\
A user method declares exactly how it takes its receiver. `self: Point` takes
the value, `self: *Point` takes a shared reference, and `self: *mut Point` takes
an exclusive reference. The receiver expression must already have that type, or
be a `Point` place that Seal can borrow automatically for a reference receiver.
\nAn exclusive method also requires a writable place. A shared reference cannot
be passed to an exclusive receiver, and a reference receiver cannot consume a
value. Adjust the receiver or the method signature.",
};

/// A match expression does not cover every value.
pub const E3032: ErrorCode = ErrorCode {
    code: "E3032",
    title: "non-exhaustive match",
    explanation: "\
A match expression must handle every value its scrutinee type can have, because
Seal has no fallback that would panic or leave the result undefined. The
diagnostic lists a value that no arm covers.\n\nAdd an arm for the missing value, or finish with `_ => ...` to handle everything
not written explicitly. Guards do not make an arm exhaustive, because a guard
can fail.\n\nSeal checks usefulness rather than just counting arms: alternatives inside one
arm are considered together, and impossible combinations are ignored.",
};

/// A match arm can never be selected.
pub const E3033: ErrorCode = ErrorCode {
    code: "E3033",
    title: "unreachable match arm",
    explanation: "\
An arm is unreachable when every value it matches is already matched by an
earlier arm. Seal reports the arm instead of compiling code that can never run,
because an unreachable arm usually hides a mistake in the patterns or their
order.\n\nRemove the arm, or move it before the arm that covers it. An arm with a guard
is reachable while the guard can succeed, even when its pattern overlaps an
earlier one; the guard is what can still make it select.\n\n`_` after an exhaustive list of variants is the most common case: the previous
arms already cover every value.",
};

/// A place is assigned while a shared reference to it is live.
pub const E5001: ErrorCode = ErrorCode {
    code: "E5001",
    title: "cannot assign to a borrowed place",
    explanation: "\
A shared reference (`*T`) promises that the place it points to keeps its value
and stays alive for as long as the reference is used. Assigning to that place
while the reference is still live would break that promise, so the borrow
checker rejects it.

A borrow ends at its last use: once the reference and every copy of it are dead,
the place becomes writable again. Move the assignment after the last use, or end
the borrow by overwriting the reference first.",
};

/// A place is moved while a shared reference to it is live.
pub const E5002: ErrorCode = ErrorCode {
    code: "E5002",
    title: "cannot move out of a borrowed place",
    explanation: "\
Moving a value transfers ownership and leaves the place uninitialized. A live
shared reference (`*T`) to that place would then point at storage without a
value, so the borrow checker rejects the move.

Use the reference only after the place is initialized again, or finish using the
reference before moving the value.",
};

/// A local is read before every path initializes it.
pub const E5003: ErrorCode = ErrorCode {
    code: "E5003",
    title: "use of possibly-uninitialized place",
    explanation: "\
The borrow checker tracks initialization over the MIR control-flow graph. A
place is readable only when every path that reaches the read has assigned it a
value. This is the same guarantee the type checker gives at the source level,
re-checked on explicit MIR places after lowering.

Assign the place on every path before the read. A branch that diverges does not
need to initialize it, because it never reaches the read.",
};

/// A reference to function-local storage escapes through the return value.
pub const E5004: ErrorCode = ErrorCode {
    code: "E5004",
    title: "returned reference points to local storage",
    explanation: "\
A local lives only until its function returns, so a reference to it cannot be
handed to the caller. The borrow checker rejects returning `&local`, including a
reference loaded from a local that borrowed another local.

Returning a reference to a parameter is fine: the caller owns the referenced
data and keeps it alive. Borrowing through a reference, as in `&*parameter`,
points outside this frame and is accepted too. The loans the reference itself
carries are checked as well, so `&*identity(&local)` is rejected: the pointee
still belongs to this frame. To return derived data, the caller must own the
storage or the value must be copied.",
};

/// A local is read after its value was moved out.
pub const E5005: ErrorCode = ErrorCode {
    code: "E5005",
    title: "use of moved value",
    explanation: "\
Moving a value out of a place leaves the place uninitialized: the new owner is
responsible for it, and the old place must not be read, borrowed, or moved
again. Copy types are duplicated instead of moved and are never affected.

Assign a new value to the place before using it again, or borrow the value
instead of moving it.",
};

/// A partially moved place is borrowed as a whole.
pub const E5006: ErrorCode = ErrorCode {
    code: "E5006",
    title: "cannot borrow a partially moved place",
    explanation: "\
Moving a field out of a struct leaves the remaining storage valid, but the whole
value is no longer intact. A shared reference to the whole place would promise
access to a value that is missing one of its fields.

Borrow only the fields that are still initialized, or reinitialize the moved
field before borrowing the whole place.",
};

/// A place is read while an exclusive reference to it is live.
pub const E5007: ErrorCode = ErrorCode {
    code: "E5007",
    title: "cannot read an exclusively borrowed place",
    explanation: "\
An exclusive reference (`*mut T`) promises that the borrower has sole access to
the pointee for as long as the reference is used (decision D6). Reading the place
directly while that loan is live would observe the value concurrently with the
borrower's writes, so the borrow checker rejects it.

A borrow ends at its last use: read the place after the reference is dead, or
read through the reference itself. The diagnostic labels that last use.",
};

/// Two loans of the same place conflict.
pub const E5008: ErrorCode = ErrorCode {
    code: "E5008",
    title: "conflicting borrow of a borrowed place",
    explanation: "\
Two loans overlap when one borrowed place is a prefix of the other. Shared loans
may overlap, and any number of them may be live at once, but an exclusive loan
(`*mut T`) forbids every other loan of the same place while it is live. Creating
a second exclusive loan, an exclusive loan beside a shared one, or writing or
moving the place while an exclusive loan is live all conflict.

End the live borrow before creating the new one. A borrow ends at its last use,
which the diagnostic labels, so moving the new borrow or access after that point
usually resolves the conflict.",
};

/// An exclusive argument conflicts with a live borrow of its pointee.
pub const E5009: ErrorCode = ErrorCode {
    code: "E5009",
    title: "cannot pass a borrowed place as an exclusive argument",
    explanation: "\
An exclusive argument — a `*mut T`, or an aggregate that carries one — gives the
callee sole access to its pointee for the whole call (decision D6). A live
borrow of that storage promises the caller the opposite, so the borrow checker
rejects the call.

A borrow ends at its last use, which the diagnostic labels. End the borrow
before the call, or pass a shared reference (`*T`) when the callee only reads.",
};

/// Two reference arguments of one call may name the same storage.
pub const E5010: ErrorCode = ErrorCode {
    code: "E5010",
    title: "reference arguments may alias",
    explanation: "\
A reference argument lets the callee access its pointee. When one of the
arguments is exclusive (`*mut T`, or an aggregate that carries one), the callee
may write through it, so it must not be able to reach the same storage through
another argument.

Reference arguments are compared by the storage they may name, not by syntax,
so copies and reborrows of one pointer are caught as well; two shared arguments
may still name the same place. Pass references to distinct places, or use a
shared reference when the callee does not write.",
};

/// A call may read an exclusively borrowed static.
pub const E5011: ErrorCode = ErrorCode {
    code: "E5011",
    title: "call may read an exclusively borrowed static",
    explanation: "\
An exclusive loan of a mutable static promises that no other access can reach
its storage while the loan is live. The called function may read that static,
directly or through another function, because Seal cannot see across the call.

End the exclusive borrow before the call, or pass the value the function needs
instead of letting it reach the global.",
};

/// A reference to a mutable static escapes its function.
pub const E5012: ErrorCode = ErrorCode {
    code: "E5012",
    title: "mutable static reference escapes",
    explanation: "\
A reference to a `static mut` is only valid while the function that created it
can reason about the borrow. Returning it, storing it in a longer-lived place, or
passing it to code that keeps it would let another function use the global after
the borrow checker stopped tracking it.

Use the reference only inside the function, and reborrow the static where it is
needed.",
};

/// A call may write a borrowed static.
pub const E5013: ErrorCode = ErrorCode {
    code: "E5013",
    title: "call may write a borrowed static",
    explanation: "\
The called function may write a mutable static, directly or transitively. A live
loan of that static in the caller would then observe storage changing through a
path the loan did not expect.

End the loan before the call, or restructure the code so the caller does not hold
a reference to the global across it.",
};

/// A field is moved out of a value whose type implements `Drop`.
pub const E5014: ErrorCode = ErrorCode {
    code: "E5014",
    title: "cannot move out of a type that implements `Drop`",
    explanation: "\
A value with a declared destructor is dropped as a whole: its destructor runs,
and then the glue drops the fields that are still there. Moving one field out
would leave the destructor looking at storage that no longer owns a value, and
the field would be dropped twice if the destructor still assumes it is present.

Borrow the field (`&value.field`), move the whole value, or drop it explicitly
with `drop(value)`.",
};

/// An array element is moved out through an index.
pub const E5015: ErrorCode = ErrorCode {
    code: "E5015",
    title: "cannot move an element out of an array",
    explanation: "\
An array owns a fixed sequence of values. Indexing selects one element, and
moving it out would leave a hole the array cannot describe: the array still
exists, but one of its positions holds no value.

Copy the element when its type is `Copy`, borrow it (`&values[i]`), or iterate
over the array by value with `for value in values`, which moves each element in
turn and drops the rest.",
};

/// A field is initialized on a value that needs a destructor.
pub const E5016: ErrorCode = ErrorCode {
    code: "E5016",
    title: "cannot initialize one field of a value that needs `Drop`",
    explanation: "\
A value that needs a destructor must be whole before it is dropped, because its
glue runs the destructor and then visits every field. Writing one field at a
time would produce a value the destructor can observe in a half-built state,
and a path that stops halfway would leave a partially drop-able value behind.

Build the whole value in one expression, such as `point = Point { x, y }`, or
store the fields in separate variables.",
};

/// The program has no usable entry point.
pub const E6001: ErrorCode = ErrorCode {
    code: "E6001",
    title: "invalid entry point",
    explanation: "\
A native executable needs a `main` function with no parameters that returns
`i32` or `()`. The code generator refuses to invent an entry point or to discard
a value the language says `main` returns.

Add `func main() -> i32 { ... }` or `func main() { ... }`. Object and assembly
output do not require an entry point.",
};

/// The requested target cannot be used for code generation.
pub const E6002: ErrorCode = ErrorCode {
    code: "E6002",
    title: "unsupported code generation target",
    explanation: "\
The target triple parsed correctly, but LLVM was built without the backend for
that architecture, or the triple does not name a target LLVM can lower.

Use a target supported by the LLVM installation, or rebuild LLVM with the
required target enabled. Cross-compiling an executable also needs a linker and
sysroot for that target; `sealc build --emit=obj` avoids the linker requirement.",
};

/// Writing or linking an artifact failed.
pub const E6003: ErrorCode = ErrorCode {
    code: "E6003",
    title: "failed to emit artifact",
    explanation: "\
The compiler produced valid machine code but could not write the requested file
or could not run the system linker. The linker error is included with the
diagnostic. Linking an executable also needs the embedded runtime to match the
target architecture and operating system; the bundled runtime is built for the
compiler host. Use `sealc build --runtime-archive <PATH>` to supply a runtime
built for the target, or `sealc build --emit=obj` to link the object separately.

Check that the output directory exists and is writable, that a C toolchain is
installed, and that the target's linker and sysroot are available. Nothing is
left behind: artifacts are written through a temporary file.",
};

/// A reachable static or constant cannot be emitted.
pub const E6004: ErrorCode = ErrorCode {
    code: "E6004",
    title: "cannot emit a global value",
    explanation: "\
Code generation emits every reachable static and constant allocation as an LLVM
global. A value whose layout is malformed, whose relocation targets a missing
symbol, or whose trait object has no vtable cannot be represented in the object
file.

The message names the item and the offending type. This is usually an internal
compiler error, unless the program uses a construct the target cannot represent.",
};

/// Monomorphization exceeded its recursion depth.
pub const E7001: ErrorCode = ErrorCode {
    code: "E7001",
    title: "monomorphization recursion limit reached",
    explanation: "\
Monomorphization expands a generic function once per distinct set of type
arguments. A function that calls itself with a growing instantiation, such as
`f<T>` calling `f<[T; 1]>`, creates a new instance at every level and would
never finish. Seal stops at a fixed depth and reports the instance chain that
reached it.

Rewrite the recursion so every recursive call uses the same type arguments, or
move the growing part to a non-generic helper. The limit is high enough for any
ordinary program; hitting it usually means generated code.",
};

/// A trait implementation needed during monomorphization does not exist.
pub const E7002: ErrorCode = ErrorCode {
    code: "E7002",
    title: "missing trait implementation during monomorphization",
    explanation: "\
A call through a trait bound keeps the trait method symbolic until
monomorphization selects the implementation for the concrete type arguments. The
selection failed: no implementation of the trait matches the concrete self type
and trait arguments, or the selected implementation requires a bound the
arguments do not satisfy.

Type checking usually reports this at the call site. If it reaches
monomorphization, check that an `impl Trait for Type` exists for every concrete
instantiation the program uses and that its own bounds hold.",
};

/// The entry point declares generic parameters.
pub const E7003: ErrorCode = ErrorCode {
    code: "E7003",
    title: "entry point cannot be generic",
    explanation: "\
`main` is the root of monomorphization: there is no caller to supply its type
arguments. A generic `main` has no concrete instance and no command-line syntax
would choose one.

Remove the type parameters from `main` and let it call generic functions with
concrete arguments.",
};

/// Monomorphization produced too many instances.
pub const E7004: ErrorCode = ErrorCode {
    code: "E7004",
    title: "monomorphization instance limit reached",
    explanation: "\
Each distinct set of type arguments creates one function instance. A program that
instantiates generic functions with very many combinations can exhaust the
collection limit before code generation starts.

Reduce the number of distinct instantiations, for example by using one type
argument per generic function, or split the program so each part uses fewer
combinations. The limit exists so such a program reports an error instead of
consuming unbounded memory.",
};

/// A vtable slot has no concrete method implementation.
pub const E7005: ErrorCode = ErrorCode {
    code: "E7005",
    title: "vtable method has no implementation",
    explanation: "\
Monomorphization builds one vtable for each concrete type and trait reference.
Every method slot must resolve to the concrete method instance selected by the
implementation. A slot with no implementation means the vtable would contain a
missing entry, so code generation cannot produce a safe indirect call.\n\nType checking normally rejects this before monomorphization. If it reaches
this stage, make sure the trait is implemented for the concrete type and that
the trait arguments match the implementation. Report the program if the
implementation exists and the compiler still cannot find it.",
};

/// The vtable collection limit was reached.
pub const E7006: ErrorCode = ErrorCode {
    code: "E7006",
    title: "vtable limit reached",
    explanation: "\
Every distinct concrete type and trait reference pair needs its own vtable. A
program that creates trait objects for very many combinations can exhaust the
vtable collection limit before code generation starts.\n\nReduce the number of concrete type and trait combinations, for example by
grouping implementations behind fewer traits, or split the program so each part
uses fewer trait objects. The limit exists so such a program reports an error
instead of consuming unbounded memory.",
};

/// A constant trait object has no collected vtable.
pub const E7007: ErrorCode = ErrorCode {
    code: "E7007",
    title: "constant trait object has no vtable",
    explanation: "\
A constant that holds a trait object names a concrete implementation and a
trait. Monomorphization collects one vtable for every reachable pair, and code
generation needs that table to initialize the constant.

Implement the trait for the concrete type, or remove the constant that needs the
missing table. The compiler reports this rather than emitting a null vtable.",
};

/// Drop glue collection reached its limit.
pub const E7008: ErrorCode = ErrorCode {
    code: "E7008",
    title: "drop glue limit reached",
    explanation: "\
Dropping a value runs one glue function per concrete type involved, and a type
that contains other droppable types nests their glue. A program with very many
distinct types that need dropping, or with types nested deeper than the glue
builder follows, can exhaust the drop glue limit before code generation starts.

Reduce the number of distinct types that need a destructor, for example by
grouping values behind a single aggregate, or split the program so each part
uses fewer of them. The limit exists so such a program reports an error instead
of consuming unbounded memory.",
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

/// A `for` loop pattern does not match every element.
pub const E3034: ErrorCode = ErrorCode {
    code: "E3034",
    title: "refutable pattern in for loop",
    explanation: "\
A for loop runs its body only when the iteration value matches the pattern. A
pattern that does not match every value of the element type would silently skip
iterations, so it is rejected.\n\nWrite an irrefutable pattern, such as a binding, `_`, or a destructuring pattern
whose fields are all irrefutable.",
};

/// A `for` loop source has no iteration protocol.
pub const E3035: ErrorCode = ErrorCode {
    code: "E3035",
    title: "value is not iterable",
    explanation: "\
A for loop iterates an integer range, an array by value, or a reference to an
array or slice. A custom source may instead implement `IntoIterator` with an
`Iterator` for the resulting cursor; a source that implements `Iterator`
directly uses the fallback. `IntoIterator` takes precedence when both are
available. Other values have no elements to visit, so the loop cannot run.\n\nIterate an integer range such as `0..len(values)`, an array, `&array`, or a
slice reference. For a custom type, implement `IntoIterator` and `Iterator`, or
implement `Iterator` on the source itself. The cursor's item type must match the
`IntoIterator` item, and ambiguous implementations are reported separately.",
};

/// A tuple element index is outside the tuple.
pub const E3036: ErrorCode = ErrorCode {
    code: "E3036",
    title: "tuple index out of bounds",
    explanation: "\
Tuple elements are numbered from zero, in the order they appear in the tuple
type. Accessing an element that the tuple does not have is rejected, because
the resulting type would not exist.\n\nCheck the tuple's arity and use an index below it. For example, `(a, b)` has
fields `0` and `1`, not `2`.",
};

/// A tuple pattern has the wrong number of element patterns.
pub const E3037: ErrorCode = ErrorCode {
    code: "E3037",
    title: "tuple pattern has the wrong arity",
    explanation: "\
A tuple pattern must describe every element of the tuple exactly once. Unlike a
struct pattern, a tuple has no field names and no `..` shorthand: omitting an
element would leave its value unaccounted for.\n\nWrite one pattern per tuple element, using `_` for elements the arm ignores.",
};

/// A generic argument could not be inferred.
pub const E3038: ErrorCode = ErrorCode {
    code: "E3038",
    title: "cannot infer a generic type argument",
    explanation: "\
A generic parameter has no value and nothing in the call determines it. Seal
does not guess defaults for type parameters, because guessing would make the
meaning of a program depend on an implementation detail.\n\nAnnotate the binding, pass the type argument explicitly, or pass an argument
whose type determines the parameter.",
};

/// A trait bound is not satisfied.
pub const E3039: ErrorCode = ErrorCode {
    code: "E3039",
    title: "trait bound is not satisfied",
    explanation: "\
A generic argument must implement every bound declared by its parameter. The
solver looked at explicit impls and at the bounds visible on type parameters,
and found no implementation for this argument.\n\nImplement the trait for the argument, or pass a different type.",
};

/// A trait goal has more than one candidate.
pub const E3040: ErrorCode = ErrorCode {
    code: "E3040",
    title: "ambiguous trait bound or method",
    explanation: "\
More than one bound or implementation can satisfy this request, so the compiler
cannot choose without guessing. Coherence should reject most overlapping impls
before they reach this point.\n\nRemove one candidate, or make the request more specific.",
};

/// An operation needs a capability a type parameter does not declare.
pub const E3041: ErrorCode = ErrorCode {
    code: "E3041",
    title: "operation is not available for a type parameter",
    explanation: "\
A type parameter is opaque inside a generic body: it is only known to implement
the bounds written on it. Arithmetic, comparison, field access, and indexing
need a concrete representation, so they cannot be applied to an unconstrained
`T`.\n\nUse an operation provided by a bound, or make the function concrete.",
};

/// Trait proof search exceeded its recursion limit.
pub const E3042: ErrorCode = ErrorCode {
    code: "E3042",
    title: "trait proof recursion limit exceeded",
    explanation: "\
Conditional implementations may depend on other bounds. The solver follows
those dependencies with a depth limit and an active-goal stack, so a cycle is
reported instead of overflowing the compiler stack.\n\nBreak the cycle between the conditional implementations.",
};

/// An argument does not satisfy a `Copy` bound.
pub const E3043: ErrorCode = ErrorCode {
    code: "E3043",
    title: "type argument does not satisfy `Copy`",
    explanation: "\
A parameter declared with `T: Copy` accepts only types that can be duplicated
bitwise. User-defined aggregates are not `Copy` unless the language provides an
explicit opt-in.\n\nPass a scalar, reference, array, or tuple of `Copy` values.",
};

/// A generic method call has the wrong number of type arguments.
pub const E3044: ErrorCode = ErrorCode {
    code: "E3044",
    title: "wrong number of method type arguments",
    explanation: "\
A method declares its own type parameters after the parameters of its impl.
Explicit `.method::<...>()` arguments must match exactly the method's own
parameters.\n\nSupply one type argument per method parameter, or omit the list.",
};

/// No unique inherent impl matches a method receiver or type arguments.
pub const E3045: ErrorCode = ErrorCode {
    code: "E3045",
    title: "no unique inherent implementation",
    explanation: "\
Selecting an inherent method requires exactly one impl whose self type matches
the receiver or the type arguments written on the aggregate. Two overlapping
impl blocks, or type arguments that match no impl of the aggregate, leave the
call without a unique target.\n\nRemove one of the overlapping methods, or use type arguments that match the
impl self type.",
};

/// A trait-object coercion has no unique implementation.
pub const E3046: ErrorCode = ErrorCode {
    code: "E3046",
    title: "no unique implementation for trait object",
    explanation: "\
Coercing `*S` to `*dyn Trait` or `*mut S` to `*mut dyn Trait` needs exactly one
implementation of the target trait for `S`, with the exact trait arguments
written in the `dyn` type. The coercion cannot choose between several
candidates, and it cannot invent an implementation that does not exist.\n\nImplement the trait for the concrete type, correct the trait arguments, or
remove the conflicting implementation. If the value should keep static
dispatch, use the concrete pointer type instead of a trait object.",
};

/// A trait-object coercion is not valid.
pub const E3047: ErrorCode = ErrorCode {
    code: "E3047",
    title: "invalid trait object coercion",
    explanation: "\
A trait object can only be formed from a pointer to a sized concrete value. The
allowed coercions are `*S` to `*dyn Trait` and `*mut S` to `*mut dyn Trait`.
The reference kind is preserved: a mutable pointer does not silently become a
shared trait object, and a shared pointer cannot become mutable. Nested
coercions such as `*[S]` to `*[dyn Trait]` are also rejected because they would
change the element layout, not just the pointer metadata.\n\nUse a pointer with the same mutability, make the pointee sized, or reborrow
explicitly with `&*pointer` when a shared trait object is required. Trait-object
upcasting from one trait to a supertrait is not a coercion in this language.",
};

/// An explicit cast is not a numeric conversion.
pub const E3048: ErrorCode = ErrorCode {
    code: "E3048",
    title: "invalid cast",
    explanation: "\
The `as` operator converts between primitive numeric types only: any integer
to any other integer, an integer to `f32` or `f64`, a floating-point value to
an integer, and `f32` to `f64` or back. Every other pair is rejected, including
`bool`, `char`, `str`, references, slices, arrays, tuples, structs, enums,
trait objects, and the never type.\n\nNumeric casts may lose information. Integers narrow by truncating the high bits
and widen by sign- or zero-extending; integer-to-float conversion rounds to
nearest, ties to even; float-to-int conversion truncates toward zero, maps NaN
to zero, and saturates at the destination bounds.\n\nUse the named conversions `bool.to_i32()` and `char.to_u32()` for those scalar
types. Converting an enum to its discriminant, reinterpreting aggregates, and
pointer casts are separate features, and fallible conversions belong in a
checked API rather than in `as`.",
};

/// A cast involves a type parameter.
pub const E3049: ErrorCode = ErrorCode {
    code: "E3049",
    title: "cast of a type parameter",
    explanation: "\
An explicit cast must select a concrete numeric conversion when it is checked.
Seal has no numeric-bound trait, so the compiler cannot decide which integer or
floating-point conversion a type parameter `T` performs, and monomorphization
cannot postpone that choice: the cast kind is recorded before instantiation.\n\nMake the cast concrete, for example by casting a value whose type is known,
or perform the conversion inside the impl that supplies the concrete type.",
};

/// A `for` source matches more than one `IntoIterator` implementation.
pub const E3050: ErrorCode = ErrorCode {
    code: "E3050",
    title: "ambiguous IntoIterator protocol",
    explanation: "\
A `for` source resolves through the `IntoIterator` protocol before falling back
to a direct `Iterator` implementation. If the exact source type matches more
than one reachable `IntoIterator` implementation, the loop cannot choose the
iterator type or item type, and the compiler does not guess from declaration
order.\n\nUse a source type with exactly one implementation, add a more specific type, or
wrap the source in a newtype with one iteration behavior.",
};

/// An iterator type does not implement `Iterator<T>`.
pub const E3051: ErrorCode = ErrorCode {
    code: "E3051",
    title: "iterator protocol is not implemented",
    explanation: "\
`IntoIterator::iter` returns the iterator type `I`, and the loop then requires
`I: Iterator<T>` for the item type declared by the `IntoIterator`
implementation. Without that second protocol, the compiler cannot call
`has_next` or `next`.\n\nImplement `Iterator<T>` for the iterator type, or change the
`IntoIterator` implementation to return a type that already implements it.",
};

/// An iterator type matches more than one `Iterator` implementation.
pub const E3052: ErrorCode = ErrorCode {
    code: "E3052",
    title: "ambiguous Iterator protocol",
    explanation: "\
A custom loop or a direct-`Iterator` source needs exactly one reachable
`Iterator` implementation for the iterator type. Several implementations with
different item types would make the loop binding and every `next` call
ambiguous, so the compiler does not choose one by declaration order.\n\nUse a type with one `Iterator` implementation, a newtype, or an explicit
conversion that selects the intended item type.",
};

/// An index expression matches more than one protocol implementation.
pub const E3053: ErrorCode = ErrorCode {
    code: "E3053",
    title: "ambiguous indexing protocol",
    explanation: "\
Custom indexing resolves through the `Index` protocol for reads and `IndexMut`
for writes. A type may implement a protocol for several index types, and the
index expression chooses one by its own type. When the expression cannot choose
exactly one implementation, the compiler does not guess: silently preferring the
first candidate would make the program depend on declaration order.\n\nMake the index expression's type select one implementation, add a more specific
implementation, or wrap the base type in a newtype with a single indexing
behavior.",
};

/// A write indexes a value that does not implement `IndexMut`.
pub const E3054: ErrorCode = ErrorCode {
    code: "E3054",
    title: "indexing is not mutable",
    explanation: "\
Writes and exclusive borrows of an indexed value resolve through the `IndexMut`
protocol, which returns an exclusive reference to the selected element. `Index`
only returns a shared reference, and implementing it does not grant write
access; the two protocols are independent.\n\nImplement `IndexMut` for the base type, or use the indexed value only in read
positions. When both protocols are needed, implement both.",
};

/// A protocol type argument cannot be inferred.
pub const E3055: ErrorCode = ErrorCode {
    code: "E3055",
    title: "cannot infer a protocol type argument",
    explanation: "\
An indexing or iteration protocol names type parameters that the surrounding
expression must determine. The index expression supplies the protocol's index
type, and the expected value type or the implementation supplies the element or
item type. If one of those parameters remains unresolved, the compiler cannot
choose a concrete implementation or generate a well-typed call.\n\nAnnotate the resulting value, use an implementation that fixes the parameter,
or pass a concrete type where the protocol is used.",
};

/// A static's type is not `Sized`.
pub const E3056: ErrorCode = ErrorCode {
    code: "E3056",
    title: "static type is not sized",
    explanation: "\
Every static owns one allocation whose size must be known when the program is
built. `str`, `[T]`, and bare trait objects are unsized, so they cannot be the
type of a static.

Store the value behind a reference (`*str`, `*[T]`, `*dyn Trait`) or use a sized
aggregate that contains the data.",
};

/// A const's type is not `Sized`.
pub const E3057: ErrorCode = ErrorCode {
    code: "E3057",
    title: "const type is not sized",
    explanation: "\
A `const` is inlined wherever it is used, so its type must have a fixed size.
Unsized types have no value to copy: `str`, `[T]`, and bare trait objects exist
only behind a reference.

Use a sized type, or declare the value as a reference such as `*str`.",
};

/// A non-`Copy` value would be moved out of a static.
pub const E3058: ErrorCode = ErrorCode {
    code: "E3058",
    title: "cannot move out of a static",
    explanation: "\
A static is one allocation shared by the whole program, so reading a non-`Copy`
value out of it would move storage that other code can still reach. The compiler
rejects the move instead of duplicating or invalidating the global.

Read a `Copy` field, borrow the static (`*T` or `*mut T`), or copy the value
into a local before using it.",
};

/// An immutable static is assigned.
pub const E3059: ErrorCode = ErrorCode {
    code: "E3059",
    title: "cannot assign to an immutable static",
    explanation: "\
Only `static mut` is writable. A plain `static` is initialized before the
program starts and never changes afterwards, so an assignment, a compound
assignment, or an exclusive borrow of it is rejected.

Declare the storage with `static mut` and perform the access inside `unsafe`, or
keep the value immutable and compute a new local value instead.",
};

/// A const or static mentions a generic parameter.
pub const E3060: ErrorCode = ErrorCode {
    code: "E3060",
    title: "const or static item mentions a generic parameter",
    explanation: "\
A `const` or `static` has exactly one value for the whole program, so its type
and initializer cannot depend on a type parameter that varies per call or per
instantiation. A declaration inside a generic function has the same rule: the
storage is shared by every instantiation.

Use a concrete type, move the value into a local, or compute it through a
generic `const func` whose result is materialized per call.",
};

/// A const function mentions a mutable raw pointer.
pub const E3062: ErrorCode = ErrorCode {
    code: "E3062",
    title: "const function mentions a mutable raw pointer",
    explanation: "\
Compile-time evaluation has no mutable memory to hand out, so a `const func`
cannot declare, accept, return, or construct a `*mut T` value, directly or
through an aggregate it names.

Use a shared reference (`*T`) for data the function only reads, or move the
pointer manipulation into an ordinary function that runs only at runtime.",
};

/// A mutable static place is accessed outside `unsafe`.
pub const E3063: ErrorCode = ErrorCode {
    code: "E3063",
    title: "mutable static access requires unsafe",
    explanation: "\
Reading, writing, borrowing, or projecting a `static mut` place is only allowed
inside an `unsafe { ... }` block. Mutable global storage can be reached from
anywhere, so the marker records that the programmer, not the compiler, is
responsible for the parts of aliasing that a single function cannot see.

Wrap the access in `unsafe { ... }`, or use an immutable `static` when the value
does not need to change.",
};

/// `Drop` is used as a trait bound or supertrait.
pub const E3064: ErrorCode = ErrorCode {
    code: "E3064",
    title: "Drop cannot be used as a trait bound",
    explanation: "\
`Drop` is not a requirement a generic function can ask for. The destructor is
tied to one concrete declaration and is called automatically when a value of
that type goes out of scope; a bound would promise the compiler could call it
for a value whose declaration it does not know yet.

Implement `Drop` for the concrete type. The destructor is then selected by the
compiler; no bound or generic function can name it.",
};

/// `Drop::drop` is called like an ordinary method.
pub const E3065: ErrorCode = ErrorCode {
    code: "E3065",
    title: "the destructor cannot be called directly",
    explanation: "\
`Drop::drop` is not part of the callable interface of a type. It takes a raw
mutable pointer to a value the compiler has already decided to release, and
calling it by hand would run the destructor while the value is still alive,
leading to a second run when the value actually goes out of scope.

Remove the call and let the end of the value's scope run the destructor at the
usual time.",
};

/// `Drop` is used as a trait object.
pub const E3066: ErrorCode = ErrorCode {
    code: "E3066",
    title: "Drop cannot be used as a trait object",
    explanation: "\
A destructor is selected from the concrete declaration of the value being
released, so it cannot be dispatched through a `dyn Drop` table. `Drop` also
declares a raw-pointer receiver, which is not a dispatchable calling
convention for a trait object.

Implement `Drop` for the concrete type, or use an ordinary trait for the
behavior a `dyn` value should expose.",
};

/// An `impl Drop` is specialized or duplicated.
pub const E3067: ErrorCode = ErrorCode {
    code: "E3067",
    title: "invalid Drop implementation",
    explanation: "\
Every aggregate has at most one destructor. The implementation must also cover
the whole type declaration: `impl Drop for Pair<T>` is valid for a `Pair` with
one parameter, but `impl Drop for Pair<i32>` and `impl<T> Drop for Pair<T, T>`
are not, because a destructor cannot be selected per instantiation.

Write one `impl Drop for TypeName<...>` that repeats the declaration's
parameters in order, and remove any other implementation for the same type.",
};

/// A `Copy` type implements `Drop`.
pub const E3068: ErrorCode = ErrorCode {
    code: "E3068",
    title: "a Copy type cannot implement Drop",
    explanation: "\
A `Copy` value is duplicated by assignment, by passing it to a function, and by
returning it. Each duplicate would run the same destructor, so the resource the
destructor releases would be released more than once.

Remove `Copy` from the type and pass references where a copy was used, or keep
the type `Copy` and release its resource without a destructor.",
};

/// A destructor is declared as a const function.
pub const E3069: ErrorCode = ErrorCode {
    code: "E3069",
    title: "a destructor cannot be const",
    explanation: "\
A destructor runs when a value goes out of scope, which only happens while the
program is running. Compile-time evaluation has no scope to leave, so a
`const func drop` could never run at the time it exists for.

Remove `const` from the destructor. If the value also needs a compile-time
constructor, give it an ordinary `const func` with a different name.",
};

/// An unsafe operation is performed outside `unsafe`.
pub const E3070: ErrorCode = ErrorCode {
    code: "E3070",
    title: "an unsafe operation requires unsafe",
    explanation: "\
Dereferencing a raw pointer, calling an unsafe function, or performing unsafe
pointer arithmetic assumes a contract the compiler cannot check. The operation
is only allowed inside an `unsafe { ... }` block, which records that the
programmer has established that contract.

Move the operation into an `unsafe` block and document why its preconditions
hold.",
};

/// `unsafe` is applied to an item that is not a function.
pub const E3071: ErrorCode = ErrorCode {
    code: "E3071",
    title: "unsafe applies only to functions",
    explanation: "\
`unsafe` marks a callable contract: the caller must establish a precondition
before invoking it. A struct, enum, constant, static, field, or type alias has
no call site where that contract could be checked, so the marker has no meaning
there.

Remove `unsafe` from the item. Unsafe behavior belongs in a function or method,
where callers can see the requirement.",
};

/// A destructor is declared unsafe.
pub const E3072: ErrorCode = ErrorCode {
    code: "E3072",
    title: "a destructor cannot be unsafe",
    explanation: "\
A destructor runs automatically when a value goes out of scope, outside any
`unsafe` block. If `drop` were unsafe, every scope exit would either need an
`unsafe` block it cannot have or would silently perform an unchecked operation.

Keep the destructor safe and move any unsafe work into an explicit method that
the destructor's owner calls before the value is dropped.",
};

/// The entry point is declared unsafe.
pub const E3073: ErrorCode = ErrorCode {
    code: "E3073",
    title: "the entry point cannot be unsafe",
    explanation: "\
The C runtime calls the process entry point directly, so no Seal caller exists
to establish an `unsafe` contract or wrap the call in an `unsafe` block. An
unsafe entry point would be invoked without anyone satisfying its precondition.

Make `main` safe and keep unsafe work in a function that `main` calls inside an
`unsafe { ... }` block.",
};

/// A cast between pointer types is not supported.
pub const E3074: ErrorCode = ErrorCode {
    code: "E3074",
    title: "unsupported cast involving a raw pointer",
    explanation: "\
An explicit `as` cast converts only between the pointer shapes the memory model
supports. A reference becomes a raw pointer with the same pointee and no more
permission than the reference already had; a raw pointer may change its pointee
or its mutability only between compatible metadata; and only `usize` and
`isize` convert to and from a pointer.

Writing through a shared reference, forging a slice or trait-object length,
and narrowing a wide integer are all rejected. Use `data()` to discard fat
metadata, and keep the pointer's displacement inside the allocation it came
from.",
};

/// A constant asks for a pointer address chosen at link time.
pub const E4030: ErrorCode = ErrorCode {
    code: "E4030",
    title: "a pointer address is not computable at compile time",
    explanation: "\
A pointer's numeric address is chosen when the program is linked, so it is not
available while a constant is evaluated. Converting a pointer to `usize` or
`isize` in a constant would make the value depend on the linker's layout.

Compare pointers with `==`, inspect nullness with `is_null()`, or move the
address computation to a runtime function.",
};

/// Constant evaluation failed.
pub const E4001: ErrorCode = ErrorCode {
    code: "E4001",
    title: "constant evaluation failed",
    explanation: "\
The compiler evaluates every `const` and `static` initializer before the program
runs. Evaluation can fail for a specific rule, reported by a more precise code;
this code is the fallback that names the item when no specific reason applies.

The diagnostic includes the item and the operation that failed. Fix the
initializer so it can be computed from the values available at compile time.",
};

/// A constant expression divides by zero.
pub const E4002: ErrorCode = ErrorCode {
    code: "E4002",
    title: "division by zero in a constant",
    explanation: "\
Constant evaluation performs integer arithmetic exactly, so dividing by zero has
no value to produce and cannot be deferred to runtime: the initializer would
never have a well-defined result.

Guard the divisor, or compute the value in ordinary code where the division runs
at runtime.",
};

/// A constant expression takes a remainder by zero.
pub const E4003: ErrorCode = ErrorCode {
    code: "E4003",
    title: "remainder by zero in a constant",
    explanation: "\
The remainder operation is undefined when the divisor is zero, exactly like
division. A constant must have one well-defined value, so the evaluator reports
the operation instead of producing a result that depends on the target.

Guard the divisor, or perform the remainder at runtime.",
};

/// A constant expression overflows signed division.
pub const E4004: ErrorCode = ErrorCode {
    code: "E4004",
    title: "signed division overflow in a constant",
    explanation: "\
Signed division overflow is the one integer operation with no representable
result: the minimum value divided by `-1` exceeds the maximum. Runtime code
would trap, and constant evaluation reports it while the item is checked.

Use a wider type, avoid the negative divisor, or handle the boundary case
explicitly.",
};

/// A constant expression indexes outside an array or slice.
pub const E4005: ErrorCode = ErrorCode {
    code: "E4005",
    title: "index out of bounds in a constant",
    explanation: "\
Constant evaluation checks every array, slice, and tuple index against the
actual length, exactly like runtime indexing. An index outside that range has no
element, and the constant cannot be built.

Use an index inside the range, or change the array length or slice extent.",
};

/// A constant expression creates an invalid slice range.
pub const E4006: ErrorCode = ErrorCode {
    code: "E4006",
    title: "invalid slice range in a constant",
    explanation: "\
A slice range must start and end inside the source and must not be reversed.
Constant evaluation checks both bounds before creating the slice, so an empty or
malformed extent is reported instead of producing a reference outside the
allocation.

Adjust the bounds so `start <= end` and both lie within the slice.",
};

/// Constants depend on each other in a cycle.
pub const E4007: ErrorCode = ErrorCode {
    code: "E4007",
    title: "constant evaluation cycle",
    explanation: "\
Constants may refer to other constants, statics, and const functions. Each value
must be computable from values that do not depend on it, so a dependency cycle
has no starting point. The diagnostic lists the edges of the cycle rather than
only the repeated name.

Break the cycle by making one value independent, or compute it at runtime.",
};

/// Constant evaluation exceeded its step limit.
pub const E4008: ErrorCode = ErrorCode {
    code: "E4008",
    title: "constant evaluation step limit reached",
    explanation: "\
Constant evaluation is bounded so a hostile or accidental initializer cannot
keep the compiler running forever. The step budget counts interpreted
operations, including calls made from a `const func`.

Simplify the computation, move the work to runtime, or split the value into
smaller constants. The limit exists to fail deterministically rather than hang.",
};

/// Constant evaluation exceeded its call-depth limit.
pub const E4009: ErrorCode = ErrorCode {
    code: "E4009",
    title: "constant evaluation call depth reached",
    explanation: "\
A `const func` may call other `const func`s, including itself. Recursion is
allowed only while the interpreter's call-depth budget lasts; exceeding it means
the evaluation would not terminate within the guaranteed resources.

Reduce the recursion depth, or perform the computation at runtime.",
};

/// Constant evaluation exceeded its allocation or value-size limit.
pub const E4010: ErrorCode = ErrorCode {
    code: "E4010",
    title: "constant evaluation allocation limit reached",
    explanation: "\
Constant evaluation allocates values from a bounded budget. An initializer that
builds a very large array or string, or that creates many intermediate values,
can exhaust it before the item is complete.

Reduce the size of the constant, split it into several items, or build the value
at runtime.",
};

/// A constant context calls a non-const function.
pub const E4011: ErrorCode = ErrorCode {
    code: "E4011",
    title: "constant context calls a non-const function",
    explanation: "\
A `const` or `static` initializer runs at compile time, so every function it
calls must be marked `const func` and must itself be evaluable. An ordinary
function needs runtime state that does not exist during compilation.

Mark the callee `const func` and remove operations that only work at runtime, or
move the call into ordinary code that runs when the program executes.",
};

/// A constant value contains a mutable reference.
pub const E4012: ErrorCode = ErrorCode {
    code: "E4012",
    title: "constant value contains a mutable reference",
    explanation: "\
A `*mut T` grants exclusive access to storage. Constant values are shared,
immutable, and may be merged with identical values, so they cannot carry a
mutable reference: two uses could otherwise write to the same constant data.

Use a shared reference (`*T`) in the constant, or keep the mutable access in
runtime code.",
};

/// A reference produced by evaluation escapes its temporary storage.
pub const E4013: ErrorCode = ErrorCode {
    code: "E4013",
    title: "constant reference escapes evaluation",
    explanation: "\
A reference produced while evaluating a `const` or `static` may point at
temporary storage created by the interpreter. Once evaluation ends, that storage
no longer exists, so the reference would dangle.

Store the referent in a `static`, which has program lifetime, or return a value
instead of a reference.",
};

/// A constant context reads a mutable static.
pub const E4014: ErrorCode = ErrorCode {
    code: "E4014",
    title: "constant context reads a mutable static",
    explanation: "\
Reading a `static mut` is an unsafe operation whose value can change between
compilation and execution. A constant must have one value fixed at compile time,
so its initializer cannot observe mutable global storage.

Use an immutable `static`, pass the value as a parameter, or read the mutable
static at runtime inside `unsafe`.",
};

/// A constant context writes to a static.
pub const E4015: ErrorCode = ErrorCode {
    code: "E4015",
    title: "constant context writes to a static",
    explanation: "\
A constant initializer cannot change global state: the write would happen during
compilation, not when the program runs, and the result would depend on
evaluation order.

Move the write to runtime code, and use `static mut` with an `unsafe` block if
mutable global storage is really intended.",
};

/// A constant context uses dynamic dispatch.
pub const E4016: ErrorCode = ErrorCode {
    code: "E4016",
    title: "constant context uses dynamic dispatch",
    explanation: "\
A constant must be built from a call target known at compile time. Dynamic
dispatch reads a function pointer from a vtable that only exists at runtime, so
the evaluator cannot follow it.

Call a concrete function, or use a const function whose calls resolve
statically.",
};

/// A constant context uses a custom iteration or indexing protocol.
pub const E4017: ErrorCode = ErrorCode {
    code: "E4017",
    title: "constant context uses a custom protocol",
    explanation: "\
`for` loops and `[]` may resolve through user-defined `IntoIterator`, `Iterator`,
`Index`, or `IndexMut` protocols whose bodies are ordinary functions. Constant
evaluation cannot run those bodies at compile time unless they are const, and
the protocol contract does not require it.

Use a built-in array or slice in the constant, or compute the value in a
`const func` without custom protocols.",
};

/// A `const func` calls a non-const function.
pub const E4018: ErrorCode = ErrorCode {
    code: "E4018",
    title: "const function calls a non-const function",
    explanation: "\
A `const func` may run at compile time and at runtime. To keep that promise,
every function it calls must also be `const func`; otherwise a call during
constant evaluation would need runtime state.

Mark the callee `const func`, or move the non-const call out of the `const
func` and into an ordinary function.",
};

/// A `const func` uses a mutable raw pointer.
pub const E4019: ErrorCode = ErrorCode {
    code: "E4019",
    title: "const function uses a mutable raw pointer",
    explanation: "\
Compile-time evaluation has no mutable memory for a `*mut T` to point at. A
`const func` that mentions one could not be interpreted, so the restriction is
checked in the signature and the body before any call is evaluated.

Use a shared reference, or move the pointer work into a non-const function.",
};

/// A `const func` accesses a mutable static.
pub const E4020: ErrorCode = ErrorCode {
    code: "E4020",
    title: "const function accesses a mutable static",
    explanation: "\
A `const func` may execute during compilation, when mutable global storage has
no defined state and cannot be written. Allowing the access would make the
constant depend on when it was evaluated.

Read only immutable statics from a `const func`, and perform mutable-global
access in runtime functions inside `unsafe`.",
};

/// A `const func` uses dynamic dispatch or a trait-bound call.
pub const E4021: ErrorCode = ErrorCode {
    code: "E4021",
    title: "const function uses dynamic dispatch",
    explanation: "\
Calls inside a `const func` must be resolvable before the program runs. Dynamic
dispatch needs a vtable built at runtime, and trait-bound calls need a concrete
implementation that the evaluator cannot select.

Call a concrete function, or make the type argument concrete so a static call is
possible.",
};

/// A constant context uses an intrinsic outside the whitelist.
pub const E4022: ErrorCode = ErrorCode {
    code: "E4022",
    title: "intrinsic is not available in a constant",
    explanation: "\
Seal's intrinsic methods have a fixed list of operations that constant
evaluation implements. An intrinsic outside that list would need runtime
behavior, allocation, or an environment that the evaluator does not provide.

Use an operation from the constant whitelist, or compute the value at runtime.",
};

/// A constant reference is out of bounds or invalid.
pub const E4023: ErrorCode = ErrorCode {
    code: "E4023",
    title: "invalid reference in a constant",
    explanation: "\
A reference inside a constant must name an allocation that exists in the
evaluated program: a static, a promoted value, or a string. A reference produced
by arithmetic or by an index outside every allocation has no valid target.

Build the reference from an existing allocation, or use an index inside its
bounds.",
};

/// A trait-object constant cannot build its vtable.
pub const E4024: ErrorCode = ErrorCode {
    code: "E4024",
    title: "trait-object constant has no vtable",
    explanation: "\
A `*dyn Trait` constant needs a vtable describing the concrete implementation.
The evaluator records the concrete type and trait, and monomorphization must
collect the matching table; a missing or malformed implementation leaves the
constant without dispatch data.

Make sure the concrete type implements the trait, and that the implementation is
reachable from the constant.",
};

/// A constant value is too large.
pub const E4025: ErrorCode = ErrorCode {
    code: "E4025",
    title: "constant value is too large",
    explanation: "\
Every constant has a maximum representable size so a program cannot exhaust
memory at compile time or produce an object the target cannot address. The limit
counts the whole value, including nested aggregates and referenced allocations.

Reduce the constant's size, split it into several items, or construct it at
runtime.",
};

/// A const or static initializer reads a mutable static.
pub const E4026: ErrorCode = ErrorCode {
    code: "E4026",
    title: "initializer reads a mutable static",
    explanation: "\
A `const` or `static` initializer is computed once, before the program runs.
Reading a `static mut` there would capture a value that does not yet have a
defined state and can change later.

Use an immutable static in the initializer, or read the mutable static at
runtime.",
};

/// A compile-time call panicked or had non-concrete type arguments.
pub const E4027: ErrorCode = ErrorCode {
    code: "E4027",
    title: "constant evaluation could not complete the call",
    explanation: "\
Constant evaluation reports this code when a `const func` call cannot be
completed: either the interpreted body reached a panic path, or a compile-time
call supplied type arguments that are still generic. Both cases mean the value
cannot be produced from the information available while compiling.

Make the type arguments concrete, or avoid the failing operation in the
compile-time path. The message states which of the two cases applied.",
};

/// Dropping a value whose type declares a destructor during constant evaluation.
pub const E4028: ErrorCode = ErrorCode {
    code: "E4028",
    title: "a destructor cannot run at compile time",
    explanation: "\
Constant evaluation only runs the fixed set of operations the compiler models.
Dropping a value whose type declares a destructor would run user code, which is
outside that set, so a compile-time path that needs to drop such a value is
rejected.

Avoid owning values with destructors in constants: build the value at run time
instead. Dropping a value whose type has no destructor is always allowed,
because it does nothing at run time either.",
};

/// Every registered code, in allocation order.
pub static ALL: &[ErrorCode] = &[
    E0001, E0002, E0003, E0004, E0005, E0006, E0007, E0008, E0009, E0010, E0011, E0012, E0013,
    E1001, E1002, E1003, E1004, E1005, E1006, E1007, E1008, E1009, E1010, E1011, E1012, E1013,
    E1014, E2001, E2002, E2003, E2004, E2005, E2006, E2007, E2008, E2009, E2010, E2011, E2012,
    E2013, E2014, E2015, E2016, E2017, E2018, E2019, E2020, E2021, E2022, E2023, E2024, E2025,
    E2026, E2027, E2028, E2029, E2030, E2031, E2032, E2033, E2034, E2035, E2036, E2037, E2038,
    E2039, E2040, E2041, E2042, E2043, E2044, E2045, E2046, E2047, E2048, E2049, E2050, E2051,
    E2052, E2053, E2054, E2055, E2056, E2057, E2058, E2059, E2060, E2061, E2062, E2063, E2064,
    E2065, E2066, E2067, E2068, E3001, E3002, E3003, E3004, E3005, E3006, E3007, E3008, E3009,
    E3010, E3011, E3012, E3013, E3014, E3015, E3016, E3017, E3018, E3019, E3020, E3021, E3022,
    E3023, E3024, E3025, E3026, E3027, E3028, E3029, E3030, E3031, E3032, E3033, E3034, E3035,
    E3036, E3037, E3038, E3039, E3040, E3041, E3042, E3043, E3044, E3045, E3046, E3047, E3048,
    E3049, E3050, E3051, E3052, E3053, E3054, E3055, E3056, E3057, E3058, E3059, E3060, E3062,
    E3063, E3064, E3065, E3066, E3067, E3068, E3069, E3070, E3071, E3072, E3073, E3074, E4001,
    E4002, E4003, E4004, E4005, E4006, E4007, E4008, E4009, E4010, E4011, E4012, E4013, E4014,
    E4015, E4016, E4017, E4018, E4019, E4020, E4021, E4022, E4023, E4024, E4025, E4026, E4027,
    E4028, E4030, E5001, E5002, E5003, E5004, E5005, E5006, E5007, E5008, E5009, E5010, E5011,
    E5012, E5013, E5014, E5015, E5016, E6001, E6002, E6003, E6004, E7001, E7002, E7003, E7004,
    E7005, E7006, E7007, E7008, E9001, E9002,
];

/// Looks up a code by its text, for example `E0001`.
pub fn lookup(code: &str) -> Option<&'static ErrorCode> {
    ALL.iter().find(|entry| entry.code == code)
}

/// Renders the `sealc explain` output for a code.
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

    use super::{ALL, E0001, E0002, E0003, E0012, E1001, E1009, E9001, E9002, explain, lookup};

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
    fn trait_object_codes_have_explanations() {
        for (code, fragment) in [
            ("E2065", "needs a finite vtable"),
            ("E3046", "cannot choose between several"),
            ("E3047", "reference kind is preserved"),
            ("E3048", "primitive numeric types only"),
            ("E3049", "no numeric-bound trait"),
            ("E7005", "missing entry"),
            ("E7006", "vtable collection limit"),
        ] {
            let entry = lookup(code).expect("trait-object code is registered");
            assert!(
                entry.explanation().contains(fragment),
                "{code} explanation is missing {fragment:?}"
            );
        }
    }

    #[test]
    fn protocol_codes_have_explanations() {
        for (code, fragment) in [
            ("E3021", "custom `Index"),
            ("E3035", "directly uses the fallback"),
            ("E3050", "does not guess"),
            ("E3051", "cannot call"),
            ("E3052", "does not choose one"),
            ("E3053", "does not guess"),
            ("E3054", "protocols are independent"),
            ("E3055", "remains unresolved"),
        ] {
            let entry = lookup(code).expect("protocol code is registered");
            assert!(
                entry.explanation().contains(fragment),
                "{code} explanation is missing {fragment:?}"
            );
        }
    }

    #[test]
    fn global_data_codes_have_explanations() {
        for (code, fragment) in [
            ("E1012", "always states its type"),
            ("E1013", "modifier of `static`"),
            (
                "E1014",
                "may also be a declaration statement inside a block",
            ),
            ("E2068", "bindings share one value namespace"),
            ("E3056", "one allocation whose size"),
            ("E3057", "inlined wherever it is used"),
            ("E3058", "move storage that other code"),
            ("E3059", "Only `static mut` is writable"),
            ("E3060", "exactly one value for the whole program"),
            ("E3062", "no mutable memory to hand out"),
            ("E3063", "inside an `unsafe { ... }` block"),
            ("E3064", "not a requirement a generic function can ask for"),
            ("E3065", "not part of the callable interface"),
            ("E3066", "selected from the concrete declaration"),
            ("E3067", "at most one destructor"),
            ("E3068", "duplicated by assignment"),
            ("E3069", "no scope to leave"),
            ("E3070", "records that the programmer"),
            ("E3071", "no call site where that contract"),
            ("E3072", "outside any `unsafe` block"),
            ("E3073", "no Seal caller exists"),
            ("E3074", "explicit `as` cast"),
            ("E4001", "fallback that names the item"),
            ("E4002", "dividing by zero"),
            ("E4003", "remainder operation is undefined"),
            ("E4004", "no representable result"),
            ("E4005", "outside that range has no element"),
            ("E4006", "must not be reversed"),
            ("E4007", "edges of the cycle"),
            ("E4008", "step budget"),
            ("E4009", "call-depth budget"),
            ("E4010", "bounded budget"),
            ("E4011", "must be marked `const func`"),
            ("E4012", "may be merged with identical values"),
            ("E4013", "that storage no longer exists"),
            ("E4014", "cannot observe mutable global storage"),
            ("E4015", "depend on evaluation order"),
            ("E4016", "vtable that only exists at runtime"),
            ("E4017", "protocol contract does not require it"),
            ("E4018", "every function it calls must also be `const func`"),
            ("E4019", "no mutable memory for a `*mut T`"),
            ("E4020", "when mutable global storage has no defined state"),
            ("E4021", "vtable built at runtime"),
            ("E4022", "fixed list of operations"),
            ("E4023", "must name an allocation that exists"),
            ("E4024", "monomorphization must collect"),
            ("E4025", "maximum representable size"),
            ("E4026", "does not yet have a defined state"),
            ("E4027", "which of the two cases"),
            ("E4028", "has no destructor is always allowed"),
            ("E4030", "chosen when the program is linked"),
            ("E5011", "cannot see across the call"),
            ("E5012", "reborrow the static where it is needed"),
            ("E5013", "directly or transitively"),
            ("E6004", "as an LLVM global"),
            ("E7007", "rather than emitting a null vtable"),
        ] {
            let entry = lookup(code).expect("global-data code is registered");
            let explanation = entry
                .explanation()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            assert!(
                explanation.contains(fragment),
                "{code} explanation is missing {fragment:?}"
            );
        }
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
                "E1001", "E1002", "E1003", "E1004", "E1005", "E1006", "E1007", "E1008", "E1009",
                "E1010", "E1011", "E1012", "E1013", "E1014",
            ]
        );
        assert_eq!(E1001.code()[1..2].parse::<u8>().expect("digit"), 1);
        assert_eq!(E1009.code()[1..2].parse::<u8>().expect("digit"), 1);
    }

    #[test]
    fn name_resolution_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E20"))
            .collect();
        assert_eq!(
            codes,
            [
                "E2001", "E2002", "E2003", "E2004", "E2005", "E2006", "E2007", "E2008", "E2009",
                "E2010", "E2011", "E2012", "E2013", "E2014", "E2015", "E2016", "E2017", "E2018",
                "E2019", "E2020", "E2021", "E2022", "E2023", "E2024", "E2025", "E2026", "E2027",
                "E2028", "E2029", "E2030", "E2031", "E2032", "E2033", "E2034", "E2035", "E2036",
                "E2037", "E2038", "E2039", "E2040", "E2041", "E2042", "E2043", "E2044", "E2045",
                "E2046", "E2047", "E2048", "E2049", "E2050", "E2051", "E2052", "E2053", "E2054",
                "E2055", "E2056", "E2057", "E2058", "E2059", "E2060", "E2061", "E2062", "E2063",
                "E2064", "E2065", "E2066", "E2067", "E2068",
            ]
        );
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
                "E3010", "E3011", "E3012", "E3013", "E3014", "E3015", "E3016", "E3017", "E3018",
                "E3019", "E3020", "E3021", "E3022", "E3023", "E3024", "E3025", "E3026", "E3027",
                "E3028", "E3029", "E3030", "E3031", "E3032", "E3033", "E3034", "E3035", "E3036",
                "E3037", "E3038", "E3039", "E3040", "E3041", "E3042", "E3043", "E3044", "E3045",
                "E3046", "E3047", "E3048", "E3049", "E3050", "E3051", "E3052", "E3053", "E3054",
                "E3055", "E3056", "E3057", "E3058", "E3059", "E3060", "E3062", "E3063", "E3064",
                "E3065", "E3066", "E3067", "E3068", "E3069", "E3070", "E3071", "E3072", "E3073",
                "E3074",
            ]
        );
        // `E3061` stays unallocated: codes are never reused, so a gap is
        // cheaper than giving an unrelated diagnostic a number out of order.
    }

    #[test]
    fn ownership_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E50"))
            .collect();
        assert_eq!(
            codes,
            [
                "E5001", "E5002", "E5003", "E5004", "E5005", "E5006", "E5007", "E5008", "E5009",
                "E5010", "E5011", "E5012", "E5013", "E5014", "E5015", "E5016"
            ]
        );
    }

    #[test]
    fn codegen_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E60"))
            .collect();
        assert_eq!(codes, ["E6001", "E6002", "E6003", "E6004"]);
    }

    #[test]
    fn mono_codes_are_contiguous() {
        let codes: Vec<&str> = ALL
            .iter()
            .map(|entry| entry.code())
            .filter(|code| code.starts_with("E70"))
            .collect();
        assert_eq!(
            codes,
            [
                "E7001", "E7002", "E7003", "E7004", "E7005", "E7006", "E7007", "E7008"
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
