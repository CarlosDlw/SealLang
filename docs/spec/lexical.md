# Lexical specification

This chapter defines how source text becomes a token stream. The lexer is
context-free: the same bytes always produce the same tokens, independent of the
target, the package, or the parser. Tokens never overlap, every token lies
inside one source file, and the stream always ends with `Eof`.

## Source text

**L1. Encoding.** Source files are UTF-8. A byte order mark (U+FEFF) is accepted
only at the start of a file and is removed before tokens are formed. A byte
order mark anywhere else is rejected (`E0004`).

**L2. Line breaks.** A line break is LF (`U+000A`) or CRLF (`U+000D U+000A`).
A bare CR is rejected (`E0004`). Columns are one-based Unicode scalar values;
the compiler also tracks UTF-16 columns for language servers, but those do not
change the language.

**L3. Whitespace.** The only whitespace characters accepted outside literals and
comments are ASCII space (`U+0020`), tab (`U+0009`), LF, and CRLF. Every other
Unicode whitespace character is rejected (`E0004`). Newlines are not
semicolons: Seal has no automatic semicolon insertion.

**L4. Rejected source characters.** Unicode bidirectional formatting characters
(`U+202A`..=`U+202E`, `U+2066`..=`U+2069`) are rejected everywhere, including
comments (`E0012`). Raw C0 control characters other than tab and line breaks are
rejected outside string and character literals (`E0013`/`E0004`). Source text is
not normalized: two identifiers that differ in Unicode normalization are
distinct.

## Tokens and trivia

**L5. Trivia.** Comments and whitespace are trivia and do not appear in the
significant token stream. Trivia is preserved in source order for tooling. A
newline inside a block comment still sets the `preceded_by_newline` flag of the
next significant token.

**L6. Comments.** Comments have these forms:

```text
line-comment      = "//" { not-line-break }
outer-line-doc    = "///" { not-line-break }
inner-line-doc    = "//!" { not-line-break }
block-comment     = "/*" { block-content } "*/"
outer-block-doc   = "/**" { block-content } "*/"
inner-block-doc   = "/*!" { block-content } "*/"
block-content     = { character | block-comment }
```

Block comments nest. `/**/` is an empty block comment, not an outer doc comment;
`/***/` is an outer doc comment with `*` content. An unterminated block comment
is `E0011`. A line comment ends before the line break.

**L7. Token stream.** Every token has a kind, an exact non-empty source span
(except `Eof`, whose span is empty and at the end of the file), and a
`preceded_by_newline` flag. Tokens never overlap, never cross a file boundary,
and cover every non-trivia byte. Malformed input produces exactly one `Error`
token covering the malformed lexeme so the parser can recover.

## Identifiers

**L8. Identifier.** An identifier starts with `_` or a character with Unicode
property `XID_Start`, and continues with zero or more characters with Unicode
property `XID_Continue`. It is interned as one symbol. `_` alone is a valid
pattern wildcard, but not a valid binding or item name where a name is required.

```text
IDENT = ( "_" | XID_Start ) { XID_Continue }
```

Seal does not apply NFC/NFD normalization and does not fold case. Keywords are
reserved exactly as spelled in L9; contextual keywords in L10 remain ordinary
identifiers outside their grammar position.

## Keywords

**L9. Reserved words.** These words are reserved and cannot be identifiers:

```text
func struct enum impl trait dyn for in if else let move mut while match
break continue return true false where as const static unsafe
```

A reserved word in a position that requires an identifier is `E1005`, with a
help that names the keyword.

**L10. Contextual words.** These spellings are ordinary identifiers except in the
listed grammar positions:

| Word | Reserved position |
|------|-------------------|
| `module` | first item of a file |
| `use` | item position before any non-import item |
| `public`, `internal`, `private` | prefix of an item, field, or method |
| `cfg` | prefix modifier, when followed by `(` |
| `test`, `bench` | prefix modifier of an item |
| `deprecated` | prefix modifier, when followed by `(` |
| `export` | prefix modifier, when followed by a string literal |
| `extern` | before a string, `struct`, or `type` |
| `raw` | immediately after `*` in a type |
| `link`, `link_name` | inside an `extern` item, in their grammar positions |
| `root` | first segment of an absolute import path |

`deprecated` without a following `(` is a normal identifier. The same is true
for `cfg`, `module`, `use`, and every other contextual word outside its listed
position.

## Punctuation and operators

**L11. Longest match.** Punctuation is matched longest first. The complete set
is:

```text
( ) { } [ ] , ; : :: . .. ..= ... @ -> =>
+ - * / % ++ -- = == != < > <= >= && || ! & | ^ ~ << >>
+= -= *= /= %= &= |= ^= <<= >>=
```

`...` is only valid in a C variadic signature. `..` and `..=` are only valid in
ranges. A punctuation character with no matching token is `E0004`.

## Numeric literals

**L12. Integer literal.** Decimal digits may be separated by `_` between
digits. `0b`, `0o`, and `0x` introduce binary, octal, and hexadecimal integers;
the base prefix is lowercase. At least one digit of the selected base is
required. The value must fit `u128` (`E0009`). A leading `-` is the unary
operator, not part of the literal.

```text
decimal-int   = decimal-digit { [ "_" ] decimal-digit }
binary-int    = "0b" bin-digit { [ "_" ] bin-digit }
octal-int     = "0o" oct-digit { [ "_" ] oct-digit }
hex-int       = "0x" hex-digit { [ "_" ] hex-digit }
int-literal   = ( decimal-int | binary-int | octal-int | hex-int ) [ int-suffix ]
```

Separators may appear between digits only. They may not start or end a number,
stand next to the base prefix, the decimal point, the exponent marker, or the
suffix. `1_`, `1__` (at the end), `0x_1`, and `1._0` are rejected (`E0007`).

**L13. Integer suffix.** The integer suffixes are `i8`, `i16`, `i32`, `i64`,
`i128`, `isize`, `u8`, `u16`, `u32`, `u64`, `u128`, and `usize`. An unsuffixed
integer literal has no type until type checking applies the default (`i32`) or
the expected type. The suffix always wins over an expected type. A suffix that
does not name a numeric type is `E0008`.

**L14. Floating literal.** A decimal float has an integer part, an optional
fraction, and/or an exponent. The fraction is `.` followed by zero or more
decimal digits; the exponent is `e` or `E`, an optional `+`/`-`, and at least
one digit. A float MUST contain either a fraction or an exponent. A leading dot
is not a float: `.5` is `.` followed by `5`.

```text
float-literal = decimal-int [ "." { decimal-digit } ] [ exponent ]
              | decimal-int exponent
exponent      = ( "e" | "E" ) [ "+" | "-" ] decimal-digit { [ "_" ] decimal-digit }
float-suffix  = "f32" | "f64"
```

A dot starts a fraction only when it is not the start of a field access, method
call, tuple index, or range. Consequently `1.foo`, `1..2`, and `pair.0.1` keep
the dot as punctuation, while `1.` and `1.0` are floats. An integer literal with
an `f32`/`f64` suffix is a float literal. A float must be finite in `f64`, and
an `f32` literal must be finite in `f32` (`E0009`). A float literal with an
integer suffix is `E0008`.

**L15. Literal typing.** Lexical form decides only whether a numeric literal is
an integer or a float. Type checking applies L12/L13/L14, the expected type, and
the default. A literal value that does not fit the selected type is rejected
during type checking, not truncated.

## Character literals

**L16. Character literal.** A character literal is `'` followed by exactly one
Unicode scalar value or one escape, then `'`. It has type `char`. An empty
literal, a multi-character literal, or a missing terminator is `E0010`.

```text
char-literal = "'" ( character | escape ) "'"
```

## String literals

**L17. String literal.** A string literal is `"` followed by zero or more
characters or escapes, then `"`. It has type `*str` and its bytes are the UTF-8
encoding of the cooked contents. A raw line break or bare CR inside the literal
is `E0006` unless it is escaped as a line continuation. A raw tab is allowed; a
raw control character other than tab is `E0013`.

**L18. Escape sequences.** These escapes are recognized in strings and
characters:

| Escape | Value |
|--------|-------|
| `\0` | NUL |
| `\t` | tab |
| `\n` | line feed |
| `\r` | carriage return |
| `\"` | double quote |
| `\'` | single quote |
| `\\` | backslash |
| `\xNN` | code point `0x00`..=`0x7F`, exactly two hex digits |
| `\u{H...}` | Unicode scalar value, one to six hex digits |

An unknown escape is `E0005`. `\x` above `0x7F`, a surrogate, a value above
`U+10FFFF`, or malformed braces are `E0005`. In a character literal, a line
continuation is `E0005`.

**L19. String continuation.** A backslash immediately followed by a line break,
then any run of spaces, tabs, and line breaks, removes the escaped line break and
the following whitespace from the cooked value. This is the only way a string
literal spans lines. A backslash before a bare CR is `E0005`.

**L20. C string literal.** `c"..."` is a C string literal. Its cooked value may
not contain an interior NUL (`E3090`); the compiler appends the terminator. Its
type is `*raw u8`. It is the only literal that produces a stable static address,
so it may initialize a `const` or `static`.

## Token kinds

**L21. Token kinds.** The lexer produces exactly these significant kinds:
`Eof`; `Ident`; `Int`, `Float`, `Str`, `CStr`, `Char`; the reserved words in
L9; the punctuation in L11; and `Error`. There is no separate boolean token:
`true` and `false` are reserved words and the parser converts them to boolean
literals. There is no comment token.

**L22. Total lexing.** Lexing always terminates and always advances at least one
byte per iteration. Every malformed lexeme becomes one `Error` token covering
it, and lexing continues after it. An internal invariant failure is an ICE, not
a loop or a silent truncation.

## Lexical diagnostics

The stable codes for this chapter are:

| Code | Meaning |
|------|---------|
| `E0004` | invalid source character, whitespace, BOM, or bare CR |
| `E0005` | malformed escape or Unicode escape |
| `E0006` | unterminated or multiline string |
| `E0007` | malformed numeric literal, base prefix, or separator |
| `E0008` | invalid numeric suffix |
| `E0009` | numeric value out of range |
| `E0010` | malformed character literal |
| `E0011` | unterminated block comment |
| `E0012` | bidirectional formatting character |
| `E0013` | raw control character in a literal |
| `E3090` | interior NUL in a C string literal |
