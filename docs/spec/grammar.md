# Concrete grammar

This chapter is the complete concrete syntax of Seal `0.1`. It is an
acceptance grammar: a source file is a Seal file only when it can be derived
from `file`. Static restrictions that cannot be expressed in EBNF — visibility
placement, `Sized` fields, impl coherence, and so on — are rules in
[`static-semantics.md`](static-semantics.md), not extra grammar.

Notation: `{ x }` is zero or more repetitions, `[ x ]` is optional, `x | y` is a
choice, `"x"` is a literal token, and `IDENT`, `INT`, `FLOAT`, `STR`, and `CSTR`
are the token classes from [`lexical.md`](lexical.md). Parentheses group.
Commas in the grammar are real tokens; a trailing comma is written explicitly
where it is accepted.

## Files and item position

```text
file          = [ module-decl ] { use-decl } { item } EOF
module-decl   = "module" IDENT { "::" IDENT } ";"
use-decl      = item-modifiers "use" use-path ";"
use-path      = use-root { "::" IDENT } [ "::" use-group ] [ "as" IDENT ]
use-root      = "root" | IDENT
use-group     = "{" use-item { "," use-item } [ "," ] "}"
use-item      = IDENT [ "as" IDENT ]
```

**G1.** A module declaration is at most one, and it MUST be the first item. A
second declaration, or one after an import or item, is `E2069`.

**G2.** Imports MUST appear before the first non-import item. A late import is
`E1001` but is still parsed.

**G3.** `root` is reserved as the first segment of an import and is not a valid
module or import segment (`E2075`). Globs (`*`) are not part of the grammar
(`E2076`).

**G4.** `module` and `use` are contextual words. They are declarations only in
the positions above; elsewhere they are ordinary identifiers.

## Modifiers

```text
item-modifiers = { visibility | cfg-modifier | deprecation | test-modifier
                 | bench-modifier | export-modifier }
visibility     = "public" | "internal" | "private"
cfg-modifier   = "cfg" "(" cfg-expr ")"
deprecation    = "deprecated" "(" deprecation-entry { "," deprecation-entry } [ "," ] ")"
deprecation-entry = "since" ":" STR
                  | "note" ":" STR
                  | "replacement" ":" STR
test-modifier  = "test"
bench-modifier = "bench"
export-modifier = "export" STR
```

**G5.** Each modifier occurs at most once. A duplicate is `E1001`. `test` and
`bench` are mutually exclusive (`E1001`). An empty `deprecated()` is rejected;
`since` is required and MUST be a semantic version; `note` and `replacement` are
optional and non-empty. Values are at most 256 bytes and contain no control
characters (`E1001`).

**G6.** `deprecated` is a modifier only when followed by `(`. Without it,
`deprecated` is an identifier.

**G7.** Modifiers are written before the item, field, or method they modify.
Whether a given modifier is meaningful for a construct is defined by the static
rules: for example, `export` applies only to functions and `impl` takes no
visibility.

## Items

```text
item = item-modifiers ( function-item | const-item | static-item | struct-item
     | enum-item | impl-item | trait-item | extern-item | opaque-type-item )

function-item = function-qualifiers "func" IDENT generic-params
                "(" [ params ] ")" [ "->" type ] where-clause block
function-qualifiers = [ "const" ] [ "unsafe" ] | [ "unsafe" ] [ "const" ]
params        = param { "," param } [ "," ]
param         = IDENT ":" type

const-item    = "const" IDENT ":" type "=" expression ";"
static-item   = "static" [ "mut" ] IDENT ":" type "=" expression ";"

struct-item   = "struct" IDENT generic-params
                where-clause "{" [ fields ] "}"
fields        = field { "," field } [ "," ]
field         = item-modifiers IDENT ":" type

enum-item     = "enum" IDENT generic-params where-clause
                "{" [ variants ] "}"
variants      = variant { "," variant } [ "," ]
variant       = variant-modifiers IDENT [ tuple-fields | named-fields ]
variant-modifiers = { cfg-modifier | deprecation }
tuple-fields  = "(" type { "," type } [ "," ] ")"
named-fields  = "{" [ fields ] "}"

trait-item    = "trait" IDENT generic-params
                [ ":" trait-bound { "+" trait-bound } ] where-clause
                "{" { trait-method } "}"
trait-method  = { cfg-modifier | deprecation }
                [ "unsafe" ] "func" IDENT generic-params
                "(" [ params ] ")" [ "->" type ] where-clause
                ( ";" | block )
trait-bound   = trait-path

impl-item     = "impl" generic-params impl-header
                where-clause "{" { item-modifiers function-item } "}"
impl-header   = trait-path "for" type
              | type [ "for" type ]

extern-item   = "extern" STR "{" { extern-member } "}"
              | "extern" STR extern-function
              | "extern" STR extern-static
              | "extern" "struct" IDENT generic-params
                where-clause "{" [ fields ] "}"
extern-member = "link" STR ";"
              | extern-function
              | extern-static
extern-function = "func" IDENT "(" [ extern-params ] ")"
                  [ "->" type ] [ "link_name" STR ] ";"
extern-params = extern-param { "," extern-param } [ "," ] [ "..." ]
extern-param  = IDENT ":" type
extern-static = "static" [ "mut" ] IDENT ":" type ";"

opaque-type-item = "extern" "type" IDENT ";"
```

**G8.** A `func` declaration has a body. A trait method has either `;` or a
default block; `impl` and `trait` bodies contain functions/methods only. An
`extern` function is a declaration: no body, no generic parameters.

**G9.** `const mut` is not a form (`E1013`). `static` takes at most one `mut`
(`E1013`). `const` and `static` require both a type and an initializer (`E1012`).

**G10.** A `variant` has exactly one field form: unit (neither suffix), positional
(`(T, ...)`, at least one type), or named (`{ field: T, ... }`). Mixing forms is
`E1010`. Named variant fields inherit the variant's visibility and cannot carry
`public`, `internal`, `private`, `test`, `bench`, or `export`.

**G11.** An `extern` block member inherits the block ABI. A member may carry
`deprecated(...)`; visibility, `cfg`, `test`, `bench`, and `export` on a member
are rejected (`E2079`/`E2085`). A nested `extern` is rejected (`E3092`).

**G12.** A variadic `extern` parameter list ends in `...` and has at least one
fixed named parameter (`E3088`). Seal functions and Seal function types are
never variadic.

**G13.** `extern type` declares a name only; it cannot be generic (`E3093`) and
ends with `;` (`E3081`). `extern struct` has C layout and cannot be generic
(`E3093`).

**G14.** A function declaration accepts `const` and `unsafe` in either order,
each at most once. `unsafe` without `func` is `E3071`. `static func` is
`E1013`; a function inside a block is not an item (`E1014`).

## Generics and bounds

```text
generic-params = [ "<" generic-param { "," generic-param } [ "," ] ">" ]
generic-param  = IDENT [ ":" bounds ]
bounds         = bound { "+" bound }
bound          = "Copy" | trait-path
trait-path     = IDENT [ generic-args ]
generic-args   = "<" type { "," type } [ "," ] ">"
               | "::" "<" type { "," type } [ "," ] ">"
where-clause   = [ "where" where-predicate { "," where-predicate } [ "," ] ]
where-predicate = type ":" bounds
```

**G15.** A generic parameter list is never empty (`E1002`). A type argument list
is never empty (`E1002`). A nested closing `>>` is split into two `>` tokens
when it closes two lists.

**G16.** `Copy` is a bound only without arguments. Every other bound names a
trait and may take generic arguments (`E1002`/`E3095`). `Fn`, `FnMut`, and
`FnOnce` accept the sugar `Fn(A, B) -> R`, which denotes `Fn<(A, B), R>`; it is
the same type as the explicit form.

**G17.** A `where` clause with no predicate is `E1002`. A bound after `+` that
is missing is `E1002`.

## Types

```text
type          = pointer-type | tuple-type | array-type | slice-type
              | dyn-type | fn-type | c-fn-type | type-path

pointer-type  = "*" [ "raw" ] [ "mut" ] ( type | c-fn-type )
tuple-type    = "(" ")" | "(" type ")" | "(" type "," [ type { "," type } [ "," ] ] ")"
array-type    = "[" type ";" INT "]"
slice-type    = "[" type "]"
dyn-type      = "dyn" trait-path
fn-type       = [ "unsafe" ] "func" "(" [ type { "," type } [ "," ] ] ")"
                [ "->" type ]
c-fn-type     = "extern" STR "func" "(" [ type { "," type } [ "," ] [ "..." ] ] ")"
                [ "->" type ]
type-path     = IDENT [ generic-args ] { "::" IDENT [ generic-args ] }
```

**G18.** `*T` is a shared reference, `*mut T` an exclusive reference, `*raw T`
a shared raw pointer, and `*raw mut T` an exclusive raw pointer. `raw` is
contextual and only recognized immediately after `*`; it remains an identifier
elsewhere.

**G19.** `(T)` is a parenthesized type, not a one-element tuple. A one-element
tuple is `(T,)`. `()` is the unit type. `[T; N]` requires a non-negative integer
literal whose type is `usize`; `N` must fit in a target-sized length (`E1009`,
`E3023`).

**G20.** A C function signature is written only as the pointee of a raw pointer:
`*raw extern "c" func(...)`. It is not a value type (`E3099`). A C function
pointer is read-only: `*raw mut extern ...` is `E3089`.

**G21.** A Seal function type is `func(T, ...) -> R` or
`unsafe func(T, ...) -> R`; it is never variadic. `dyn` takes exactly one trait
and no `+` composition (`E1002`). `dyn Fn(...)` is parsed and rejected later
because function traits are not object-safe.

**G22.** Primitive type names are identifiers in the grammar, not reserved
words: `i32`, `bool`, `str`, `c_int`, and so on. They are resolved by name
before ordinary type paths. `Self` is an identifier with the special meaning in
trait and impl signatures.

## Expressions

The precedence table is normative; the EBNF below groups the same levels.

| Level | Operators | Associativity |
|------:|-----------|---------------|
| 1 | `=` `+=` `-=` `*=` `/=` `%=` `<<=` `>>=` `&=` `|=` `^=` | right |
| 2 | `||` | left |
| 3 | `&&` | left |
| 4 | `|` | left |
| 5 | `^` | left |
| 6 | `&` | left |
| 7 | `==` `!=` | left |
| 8 | `<` `>` `<=` `>=` | left |
| 9 | `<<` `>>` | left |
| 10 | `+` `-` | left |
| 11 | `*` `/` `%` | left |
| 12 | `as` | left |
| 13 | prefix `-` `!` `~` `&` `&mut` `*` `++` `--` | right |
| 14 | postfix call, index, field, method, `++`, `--` | left |

```text
expression     = assignment
assignment     = logical-or [ assign-op assignment ]
assign-op      = "=" | "+=" | "-=" | "*=" | "/=" | "%="
               | "<<=" | ">>=" | "&=" | "|=" | "^="
logical-or     = logical-and { "||" logical-and }
logical-and    = bit-or { "&&" bit-or }
bit-or         = bit-xor { "|" bit-xor }
bit-xor        = bit-and { "^" bit-and }
bit-and        = equality { "&" equality }
equality       = comparison { ( "==" | "!=" ) comparison }
comparison     = shift { ( "<" | ">" | "<=" | ">=" ) shift }
shift          = additive { ( "<<" | ">>" ) additive }
additive       = multiplicative { ( "+" | "-" ) multiplicative }
multiplicative = cast { ( "*" | "/" | "%" ) cast }
cast           = prefix { "as" type }
prefix         = ( "-" | "!" | "~" | "&" [ "mut" ] | "*" | "++" | "--" ) prefix
               | postfix
postfix        = primary { call-suffix | index-suffix | field-suffix
                         | update-suffix }
call-suffix    = "(" [ arguments ] ")"
index-suffix   = "[" expression "]"
               | "[" [ expression ] ".." [ expression ] "]"
field-suffix   = "." ( INT
                     | IDENT [ "(" [ arguments ] ")" ]
                     | IDENT "::" generic-args "(" [ arguments ] ")" )
update-suffix  = "++" | "--"
arguments      = expression { "," expression } [ "," ]
```

**G23.** Assignment is right-associative and its left operand MUST be a place.
The parser syntactically restricts the target to a name, dereference, field,
tuple field, index, or parenthesized place; mutability is a static rule.

**G24.** `&` and `*` are prefix borrow/dereference in prefix position and binary
bitwise operators elsewhere. `&&`/`||` short-circuit. There is no ternary
operator.

**G25.** Postfix `.` is resolved by the next token: a decimal integer is a tuple
index; an identifier followed by `(` is a method call; otherwise it is field
access. A method may take explicit arguments only as `.name::<T>(...)`; the
shorter `.name<T>(...)` is parsed for recovery and diagnosed (`E1002`).

**G26.** `as` binds tighter than the multiplicative operators and looser than
prefix operators, and chains left to right: `-1 as i32 as i64` is
`((-1) as i32) as i64`. Numeric casts are defined in the static semantics; the
grammar accepts any type on the right.

### Primary expressions

```text
primary = literal
        | path
        | "(" ")"
        | "(" expression ")"
        | "(" expression "," [ expression { "," expression } [ "," ] ] ")"
        | array-literal
        | struct-literal
        | variant-literal
        | if-expression
        | match-expression
        | "unsafe" block
        | block
        | closure

literal = INT | FLOAT | STR | CSTR | CHAR | "true" | "false"

path = IDENT [ generic-args ] { "::" IDENT [ generic-args ] }
struct-literal = path "{" [ field-inits ] "}"
variant-literal = path "::" IDENT "{" [ field-inits ] "}"
field-inits = field-init { "," field-init } [ "," ]
field-init = IDENT ":" expression

array-literal = "[" "]"
              | "[" expression "]"
              | "[" expression "," [ expression { "," expression } [ "," ] ] "]"
              | "[" expression ";" INT "]"

if-expression = "if" if-condition block [ "else" ( block | if-expression ) ]
if-condition  = expression | "let" pattern "=" expression

match-expression = "match" expression "{" { match-arm } "}"
match-arm = pattern [ "if" expression ] "=>" expression [ "," ]
closure = [ "move" ] ( "||" | "|" [ closure-params ] "|" )
          [ "->" type ] ( block | expression )
closure-params = closure-param { "," closure-param } [ "," ]
closure-param  = IDENT [ ":" type ]
```

**G27.** A `match` needs at least one arm (`E1011`). An arm body that is a block
does not need a comma; every other body does. The parser accepts a trailing
comma.

**G28.** `if` is an expression. Without `else` its type is `()`; `else if` is an
`if` expression in the else block. `if let` matches a refutable pattern, and its
bindings are scoped to the then block only.

**G29.** A closure without parameters is written `||`; with parameters it is
`|a, b|`. `move` is reserved and must precede the pipes. A closure with an
explicit `-> Type` MUST have a block body (`E1002`).

**G30.** `{}` is a block expression. `();` is the unit literal. `(e)` groups;
`(e,)` is a one-element tuple.

**G31.** Struct and variant literals use `Name { field: value }`. There is no
field shorthand in the grammar. A unit variant is written as a path, not with
braces.

**G32.** `[a, b]` is an array literal, `[]` is an empty array, and `[value; N]`
evaluates `value` once and repeats it. `N` is a `usize` integer literal.

### Blocks and statements

```text
block = "{" { statement } [ expression ] "}"
statement = let-statement
          | local-const
          | local-static
          | return-statement
          | while-statement
          | while-let-statement
          | for-statement
          | "break" ";"
          | "continue" ";"
          | expression ";"
          | block-expression-statement

let-statement = "let" [ "mut" ] IDENT [ ":" type ] [ "=" expression ] ";"
local-const = "const" IDENT ":" type "=" expression ";"
local-static = "static" [ "mut" ] IDENT ":" type "=" expression ";"
return-statement = "return" [ expression ] ";"
while-statement = "while" expression block
while-let-statement = "while" "let" pattern "=" expression block
for-statement = "for" pattern "in" for-source block
for-source = expression [ ( ".." | "..=" ) expression ]
block-expression-statement = if-expression | match-expression | "unsafe" block
```

**G33.** A `let` needs a type or an initializer (`E1007`); a `let` binding is not
a pattern binding. A binding is immutable unless `mut` is written.

**G34.** `return`, `break`, and `continue` end with `;`. `break` and `continue`
may appear only inside a loop (`E2003`/`E2004`).

**G35.** `if`, `match`, and `unsafe { ... }` are block-like expressions and may
stand as statements without `;`. A plain block expression as a statement is not
part of the grammar; write `let _ = { ... };` or use it as a tail expression.

**G36.** An `if`, `while`, `for`, or `match` header ends at its body brace. A
struct literal in a condition, `if let` scrutinee, `while let` scrutinee, `for`
source, or `match` scrutinee MUST be parenthesized: `if (Point { x: 1 }).x > 0`.
Without parentheses the `{` starts the body/arm list.

## Patterns

```text
pattern        = pattern-alt { "|" pattern-alt }
pattern-alt    = "_"
               | IDENT [ "@" pattern-alt ]
               | scalar-literal [ range-op scalar-literal ]
               | "(" ")"
               | "(" pattern ")"
               | "(" pattern "," [ pattern { "," pattern } [ "," ] ] ")"
               | "[" [ pattern { "," pattern } ] [ "," ] [ ".." [ "," [ pattern { "," pattern } [ "," ] ] ] ] "]"
               | IDENT "::" IDENT [ variant-pattern-fields ]
               | IDENT "{" [ named-pattern-fields ] "}"
               | IDENT "::" IDENT "{" [ named-pattern-fields ] "}"
scalar-literal = INT | FLOAT | CHAR | "true" | "false"
               | "-" ( INT | FLOAT )
range-op       = ".." | "..="
variant-pattern-fields = "(" [ pattern { "," pattern } [ "," ] ] ")"
                       | "{" [ named-pattern-fields ] "}"
named-pattern-fields = named-pattern-field { "," named-pattern-field } [ "," ] [ ".." [ "," ] ]
named-pattern-field = IDENT [ ":" pattern ]
```

**G37.** `|` binds loosest, so `A | B` is one alternative list. `@` binds
tighter: `n @ 1..=5 | other` has two alternatives, the first binding `n`.
Alternatives MUST bind the same names (`E2041`).

**G38.** A range pattern has a literal start and end and is written `..` or
`..=`. String literals are not patterns (`E1011`). A rest pattern is `..`; in an
array it may appear at most once. In a named field list, `..` MUST be last.

**G39.** An enum variant pattern MUST be qualified with its enum
(`Enum::Variant`); an unqualified `Variant(...)` is `E1011`. A struct pattern is
`Struct { ... }`; a named variant pattern is `Enum::Variant { ... }`. Named
field shorthand `Point { x }` binds `x`.

**G40.** An `if let` or `while let` pattern may be refutable. A `for` pattern
over a built-in range, array, or slice MUST be irrefutable (`E3034`). A `match`
pattern may be refutable, and the whole match MUST be exhaustive (`E3032`).

## `cfg` expressions

```text
cfg-expr    = cfg-or
cfg-or      = cfg-and { "||" cfg-and }
cfg-and     = cfg-unary { "&&" cfg-unary }
cfg-unary   = "!" cfg-unary | "(" cfg-expr ")" | cfg-atom
cfg-atom    = cfg-primary [ ( "==" | "!=" ) cfg-value ]
cfg-primary = "feature" "(" STR ")"
            | "env" "(" STR ")"
            | "target" "." ( "os" | "arch" | "env" | "pointer_width" )
            | "debug_assertions" | "test" | "bench"
cfg-value   = STR | INT | "true" | "false"
```

**G41.** `cfg` has no arbitrary keys. A string literal in key position, an
unknown key, or an unknown `target` field is `E2084`. `feature` and `env` take
exactly one string argument.

**G42.** `!` binds tighter than `&&`, which binds tighter than `||`. Comparisons
are allowed only between a `cfg` atom and a literal; the static semantics
defines which pairs are well-typed. A malformed expression is `E2085` when it is
in a position where `cfg` is not meaningful, otherwise `E1002`/`E2084`.

## Precedence and ambiguity summary

**G43.** The precedence table above is exhaustive. Parentheses override it. `as`
is the only type-directed postfix operator; every other postfix has higher
precedence. Assignment and update targets are places; `&&`/`||` are control
flow, not arithmetic.

**G44.** A `>>` token closes two generic lists when nested:
`Wrapper<Wrapper<T>>` is parsed as two closing `>` tokens. Whitespace does not
change this.

**G45.** A file is parsed even when it contains syntax errors. The parser
recovers at item, statement, expression, type, and pattern boundaries and
returns a total tree with `Error` nodes. Recovery is an implementation
requirement, not a separate accepted language: a file with any syntax error is
not a valid Seal program, even though the compiler can still expose its tree to
an editor.

## Grammar diagnostics

The stable syntax codes are:

| Code | Meaning |
|------|---------|
| `E1001` | unexpected token or malformed item shape |
| `E1002` | expected a specific token or delimiter |
| `E1003` | expected an expression |
| `E1004` | expected a type |
| `E1005` | expected an identifier |
| `E1007` | `let` without a type or initializer |
| `E1008` | nesting limit exceeded |
| `E1009` | malformed array length |
| `E1010` | malformed enum or variant field form |
| `E1011` | malformed pattern, range, or match arm |
| `E1012` | `const`/`static` without a type or initializer |
| `E1013` | invalid `const`/`static` form |
| `E1014` | file-scope item inside a block |
| `E2069` | invalid module declaration position or duplicate |
| `E2075` | reserved `root` in a module/import position |
| `E2076` | glob import |
| `E2079` | modifier in an invalid position |
| `E2084` | unknown or malformed `cfg` key |
| `E2085` | `cfg` in an invalid position |
| `E3071` | `unsafe` on a non-function |
| `E3081` | malformed external declaration |
| `E3088` | malformed variadic parameter list |
| `E3089` | mutable C function pointer |
| `E3090` | interior NUL in a C string literal |
| `E3091` | malformed `link` directive |
| `E3092` | nested or statement-level `extern` |
| `E3093` | generic external declaration |
| `E3099` | C function signature used as a value type |
