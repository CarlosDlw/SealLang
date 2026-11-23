# Type aliases

A type alias gives a second spelling to an existing type. It is transparent:
the compiler substitutes the target and then forgets that the alias was written.
No allocation, layout, vtable, destructor, or run-time check is attached to the
name.

```seal
type Score = i32;
type Pair<T> = (T, T);
type PointAlias = Point;
```

## Declaring an alias

An alias is a file-scope item:

```seal
public deprecated(since: "0.2.0", replacement: "Points")
type Points = *Point;
```

The declaration accepts the ordinary `public`, `internal`, `private`, `cfg`, and
`deprecated(...)` modifiers. It does not accept `test`, `bench`, `export`, or
`unsafe`; those report a syntax error. `type` is contextual, so it can still be
used as an identifier away from item-start position.

`extern type Name;` is a different declaration: it introduces an opaque C type.
A type alias always has a target after `=` and ends with `;`.

## Generics

```seal
type Pair<T> = (T, T);
type Nested<T> = Pair<Pair<T>>;

func first(pair: Pair<i32>) -> i32 {
  pair.0
}
```

Every parameter needs exactly one type argument at the use site. A missing or
extra argument is a normal generic-arity diagnostic. Aliases compose with
structs, enums, arrays, references, function pointers, and other aliases.

A transparent alias cannot enforce a bound on its parameters, so Seal rejects
one instead of silently ignoring it:

```seal
type Broken<T: Copy> = T; // E2096
```

Put the bound on the generic function, struct, enum, or trait that actually uses
the type.

## Nominal paths

An alias to a struct or enum is also usable wherever the nominal name is:

```seal
struct Point { x: Score, y: Score }
type P = Point;
enum Maybe { Some(Score), None }
type M = Maybe;

impl P {
  func origin() -> P {
    P { x: 0, y: 0 }
  }
}

func read(point: P) -> Score {
  match point {
    P { x, y } => x + y,
  }
}

func optional() -> Score {
  match M::Some(21) {
    M::Some(value) => value,
    M::None => 0,
  }
}
```

Constructors, variant paths, associated functions, patterns, and impl headers
resolve through the alias to the underlying declaration. An alias whose target
is a primitive, tuple, reference, or another non-nominal type cannot be used as
a constructor or pattern; that is `E2097`.

## Cycles and visibility

Expansion must terminate. Both `type A = A;` and `type A = B; type B = A;` are
`E2095`. Recursive data uses a struct or enum with a pointer field:

```seal
type Link = *Node;

struct Node {
  next: Link,
}
```

Aliases share the type namespace with every other type declaration, so this is
a duplicate-name error:

```seal
struct Id { value: i32 }
type Id = i32; // E2005
```

A public alias is part of the package API. Its target must also be public and
reachable from the same module; otherwise the API check reports `E3107`.
