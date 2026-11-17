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
A trait is a complete list of signatures: implementing it means supplying one
function for each method, with the name and the signature the trait declares.
Seal has no default methods, so there is nothing to inherit when a method is
missing.\n\nAdd the missing function to the impl block, or remove the method from the trait
declaration.",
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
type. Seal has no overloading and no implicit conversions, so a mismatch would
make the trait's guarantee untrue for one implementor.\n\nIn the trait, write the implementing type as `Self`; in the impl, write the
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
field the arm ignores.",
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
have a size known at compile time (decision D10 of the memory model). `str` and
slices are unsized: they carry a runtime length, so they can only appear behind
a reference.

Use `*str` for string data and `*[T]`/`*mut [T]` for a borrowed sequence.
String literals already have type `*str`, and a range such as `&values[..]`
produces a slice reference, so both can be stored in a binding without an
annotation.",
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
a new value.",
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
The `base[index]` syntax selects an element of a fixed-size array. Seal has no
slices, strings are not indexable, and there is no overloading or automatic
dereference, so the base must have an array type such as `[i32; 4]`.

Dereference a reference first, as in `(*array)[index]`, and make sure the base
is an array rather than a scalar or a struct.",
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
A struct method comes from an `impl` block for that struct; an intrinsic method\ncomes from the compiler-owned scalar, array, or slice catalog. A name that\nneither source defines for the receiver's type cannot be called.\n\nCheck the spelling and the impl block, or dereference the receiver until it\nreaches the type that defines the method. The intrinsic catalog documents every\ncompiler-owned name.",
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
A method with an exclusive receiver, and an intrinsic such as `reverse`, `fill`,\nor `toggle`, modifies its receiver. It requires a mutable binding, or a place\nreached through a `*mut T` reference. A shared reference and an immutable\nbinding do not permit the write.\n\nDeclare the binding with `let mut`, or call the method through a mutable\nreference.",
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
sysroot for that target; `--emit=obj` avoids the linker requirement.",
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
compiler host. Use `--runtime-archive <PATH>` to supply a runtime built for the
target, or `--emit=obj` to link the object separately.

Check that the output directory exists and is writable, that a C toolchain is
installed, and that the target's linker and sysroot are available. Nothing is
left behind: artifacts are written through a temporary file.",
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
    E1001, E1002, E1003, E1004, E1005, E1006, E1007, E1008, E1009, E1010, E1011, E2001, E2002,
    E2003, E2004, E2005, E2006, E2007, E2008, E2009, E2010, E2011, E2012, E2013, E2014, E2015,
    E2016, E2017, E2018, E2019, E2020, E2021, E2022, E2023, E2024, E2025, E2026, E2027, E2028,
    E2029, E2030, E2031, E2032, E2033, E2034, E2035, E2036, E2037, E2038, E2039, E2040, E2041,
    E2042, E2043, E2044, E2045, E3001, E3002, E3003, E3004, E3005, E3006, E3007, E3008, E3009,
    E3010, E3011, E3012, E3013, E3014, E3015, E3016, E3017, E3018, E3019, E3020, E3021, E3022,
    E3023, E3024, E3025, E3026, E3027, E3028, E3029, E3030, E3031, E3032, E3033, E5001, E5002,
    E5003, E5004, E5005, E5006, E5007, E5008, E5009, E5010, E6001, E6002, E6003, E9001, E9002,
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
                "E1010", "E1011",
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
                "E3028", "E3029", "E3030", "E3031", "E3032", "E3033",
            ]
        );
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
                "E5010"
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
        assert_eq!(codes, ["E6001", "E6002", "E6003"]);
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
