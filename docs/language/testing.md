# Tests and benchmarks

Status: implemented in the current language version.

A test or a benchmark is an ordinary function with a configuration-aware
modifier. The compiler generates one harness entry point per target, and the
project tool runs each selected function in its own process. That design keeps
the language surface small: there is no `#[test]` attribute, no test runtime
visible to normal code, and no global registration step.

This chapter describes the language rules. The command-line options are in
[`tests and benchmarks`](../project/cli.md#tests-and-benchmarks); the target
layout rules build on [modules](modules.md).

## 1. Declaring a test

```seal
func double(value: i32) -> i32 {
  value * 2
}

test func doubles_positive_numbers() {
  assert(double(2) == 4);
}

test func doubles_zero() {
  assert(double(0) == 0, "zero must stay zero");
}
```

`test` is a contextual modifier. It is parsed only in the prefix of an item, so
existing code that uses `test` as an identifier keeps working. A function may
be declared with `test func`, or with the modifier followed by a visibility:

```seal
internal test func uses_a_private_helper() {
  assert(hidden() == 7);
}
```

A visibility on a test is accepted syntactically but has no API effect. In a
harness build the function is made `internal` so the generated entry point can
reach it; in every other build the item does not exist. Test functions never
enter the public API.

The modifier applies only to free functions. `test` on an import, module
declaration, or enum variant is rejected. `test` and `bench` are mutually
exclusive, and each may be written at most once:

| Declaration | Code |
|-------------|------|
| `test bench func f()` | `E1001` |
| `test test func f()` | `E1001` |
| `test module app::util;` | `E2079` |
| `test use app::util::f;` | `E2079` |
| `test import` on an enum variant | `E1001` |
| `test` on a trait method | `E1001` |

Functions associated with an `impl` block are not entry points. The harness
collects free functions whose module path is part of the target, so test code
belongs in a module file or in the package root.

The modifier is available in every package, including dependency packages. A
dependency compiled as part of an integration test does not receive
`cfg(test)`, so its test functions are omitted from that compilation.

## 2. Configuration gating

`test` and `bench` are configuration modifiers, not runtime flags. They behave
as if the declaration also carried the matching `cfg` condition:

- `test func` exists only when the package is compiled with `cfg(test)`;
- `bench func` exists only when the package is compiled with `cfg(bench)`.

A normal `seal build`, `seal check`, or `seal run` therefore never type-checks
or emits a test body. This is intentional: a test may use `assert`, private
setup helpers, or APIs that only make sense in a harness, and a production build
must not pay for them.

A test target sets `cfg(test)` for its primary package. A benchmark target sets
`cfg(bench)`. The two configurations are not active at the same time:

| Build | `cfg(test)` | `cfg(bench)` | Collected entries |
|-------|:-----------:|:------------:|-------------------|
| normal build/check | off | off | none |
| test target | on | off | `test func` |
| benchmark target | off | on | `bench func` |

Explicit configuration still works outside a harness:

```seal
cfg(test) func fixture() -> i32 {
  40 + 2
}
```

`cfg(test)` helpers are compiled into the test target and omitted from normal
builds. The same rule holds for `cfg(bench)`. Use an explicit condition when a
helper must exist in the harness but is not itself an entry point.

## 3. The `assert` intrinsic

`assert` is a built-in expression with one or two arguments:

```seal
assert(condition);
assert(condition, "message");
```

The condition must be `bool`; a non-boolean condition is a normal type mismatch
(`E3001`). The optional message must be a string literal. A non-literal message
is `E8106`, and the wrong arity is `E3008`. The message is deliberately
restricted to a literal so a failing test never needs formatting, allocation, or
a formatting subsystem.

```seal
test func valid_range() {
  let value = 7;
  assert(value > 0, "the value must be positive");
}
```

On success `assert` produces `()`. On failure it reaches the ordinary panic
path with the call site and the optional message. A failed assertion therefore
terminates that test process with a non-zero status; it does not unwind into the
runner or affect another test. The runner reports the captured panic text:

```text
FAIL valid_range (10 ms)
     panic: the value must be positive at src/lib.sl:4:3
```

`assert` is available only in `cfg(test)` and `cfg(bench)` builds. Using it in a
normal build is `E8105`:

```seal
func production_check() {
  assert(1 == 1); // E8105: no harness is present
}
```

If a helper is called by tests but is also compiled normally, the helper cannot
use `assert`. Either keep the helper test-only with `cfg(test)`, or write an
ordinary `if` in the production path.

`assert` is a built-in name only when no user item or local binding shadows it.
A user function named `assert` is called normally, exactly like a user function
named `drop` or `len`.

## 4. Entry-point contract

A `test` or `bench` function must be callable by the generated harness. The
harness validates the signature after type checking, so a malformed entry point
is reported even when its body type-checks:

- it must be a free function;
- it must have no generic parameters;
- it must take no parameters;
- it must return `()`.

Any violation is `E8107`. Move setup into the body or into a helper:

```seal
test func parser_accepts_empty_input() {
  let input = "";
  assert(input.len() == 0);
}
```

An entry point is not required to be `main`, and it may be in a module:

```seal
module math;

internal func triple(value: i32) -> i32 {
  value * 3
}

test func module_tests_are_discovered() {
  assert(triple(3) == 9);
}
```

The harness prints this entry as `math::module_tests_are_discovered`. A
function in the package root is printed without a `::` prefix. The package name
is not part of the test name; the package and target are separate selection
dimensions.

## 5. Unit and integration tests

The distinction is about package boundaries, not about syntax.

A **unit** test target reuses a library target. Its files are merged into the
library's package, so the test can call private items and see the same module
graph as the library:

```seal
func hidden() -> i32 {
  7
}

test func uses_private_helper() {
  assert(hidden() == 7);
}
```

The test function itself is still not public API; the harness is the only
caller outside the test body.

An **integration** test is a separate package. It depends on the library and
can use only the library's public API:

```seal
use fixture::double;

test func integration_uses_the_public_api() {
  assert(double(21) == 42);
}
```

Importing a private item from an integration test is an ordinary visibility
error (`E2070`/`E2072`); the test cannot bypass the package boundary. This is
the contract to rely on when checking that a library is usable from outside.

The integration package receives its own `cfg(test)` configuration. The library
it depends on does not. Consequently, `cfg(test)` items in the library are not
available to an integration test; the test must use the stable public API or
declare its own helpers.

## 6. Target discovery

When a package manifest has no `[targets]` table, the project tool infers the
usual targets:

| Package shape | Inferred test/bench targets |
|---------------|-----------------------------|
| `src/lib.sl` | one `unit` target reusing the library, each `tests/*.sl`, each `benches/*.sl` |
| `src/main.sl` only | each `tests/*.sl`, each `benches/*.sl`; no automatic unit target |
| library and binary | the binary is selected first; integration and benchmark files are still discovered |

The automatic unit target is named `unit`, except when the package itself is
named `unit`; then it is named `unit-tests`. Each `tests/<stem>.sl` becomes a
test target named `<stem>`, and each `benches/<stem>.sl` becomes a benchmark
target named `<stem>`. A file whose stem collides with an existing target is
skipped.

A unit target is created only for a library package. A binary has no library
target to merge, so its unit-style tests should live in `tests/*.sl` or in an
explicitly declared test target.

A manifest that declares `[targets]` keeps full control of discovery. Declare
the target explicitly and choose its sources:

```toml
[targets.math_unit]
kind = "test"
reuse = "math"

[targets.math_integration]
kind = "test"
sources = ["tests/integration.sl"]
include = ["tests/support.sl"]

[targets.math_bench]
kind = "bench"
sources = ["benches/math.sl"]
```

`reuse` names a library target and makes the result a unit-style target.
Without `reuse`, a test or benchmark target is an integration-style target: its
own primary file is compiled as a separate package, and `include` adds helper
files to that package. `include` is accepted only for test, benchmark, and
example targets.

## 7. Running tests

`seal test` builds every selected test target and asks each harness for its
entry list. Every test then runs in a separate process, so a panic, an abort, a
signal, or a timeout cannot stop the other tests. The test process receives only
its own entry name:

```text
test-harness --list
test-harness --run math::module_tests_are_discovered
```

The runner applies these selection rules in order:

1. target/package selection (`-p`, `--workspace`, `--exclude`, positional target
   names);
2. `--filter` substring matches (or `--exact` whole-name matches);
3. `--skip` substring exclusions;
4. de-duplication by package, target, and entry name;
5. `--partition` sharding;
6. `--stress` repetition.

`--partition` accepts `slice:INDEX/TOTAL` or `hash:INDEX/TOTAL`, and the short
form `INDEX/TOTAL` means a slice. A hash partition is stable for a given name,
so CI shards do not move when the test list changes order. `--stress N`
schedules each selected entry N times; each repetition is a separate process
and its report label carries `#1`, `#2`, and so on.

Execution is parallel by default. `--jobs` bounds it. Failures stop new tests
from being scheduled unless `--no-fail-fast` is given; tests already running
finish and are still reported. `--retries N` gives a failed or timed-out test N
extra attempts, and `--timeout` applies to each attempt. A timed-out test is
killed together with its process group, so a helper process cannot keep the
harness alive.

The command exits `0` when every selected test passed or was skipped, and `1`
when any test failed, timed out, or could not be executed. A missing test target
is `E8100`; a harness that cannot be started is `E8101`; an invalid runner
option is `E8102`; a report that cannot be written is `E8108`.

### Reports

`--format human` is the default. It prints one line per test and a final
summary. `--format json` emits NDJSON: one object per result, followed by a
`summary` object. `--format junit` writes one JUnit XML suite and defaults to
`seal/target/test-junit.xml`; `-o/--output` selects another path. Reports are
stable tool input; the human format is not.

## 8. Benchmarks

A benchmark is declared with `bench func` and follows the same signature,
discovery, selection, and configuration rules as a test:

```seal
bench func counting_loop() {
  let mut total = 0;
  let mut index = 0;
  while index < 1000 {
    total = total + index;
    index = index + 1;
  }
  assert(total == 499500);
}
```

The harness runs the benchmark body `--iterations` times in one process. The
runner then measures that process's wall time and divides by the iteration
count. `--samples` repeats the measurement in separate processes and reports
the fastest and mean result. A benchmark body is repeated in full, including
its setup; keep setup out of the body when it should not be measured.

`seal bench` reports nanoseconds per iteration and makes no statistical claim.
`--save` writes a JSON baseline; `--baseline` compares against it and exits `1`
when the fastest measurement is slower than the baseline by more than
`--threshold` percent. Benchmark failures (including a failed `assert`) are
reported as command failures, not as measurements.

## 9. Errors and diagnostics

| Situation | Code |
|-----------|------|
| `test` and `bench` on the same function, or a repeated modifier | `E1001` |
| `test`/`bench` on an import or module declaration | `E2079` |
| `test`/`bench` on a non-function item or variant | `E1001` |
| `assert` in a normal build | `E8105` |
| `assert` with a non-literal message | `E8106` |
| `assert` with the wrong number of arguments | `E3008` |
| `assert` condition is not `bool` | `E3001` |
| Test/benchmark has parameters, generics, or a non-unit return | `E8107` |
| No test target matches the selection | `E8100` |
| Harness cannot be executed or listed | `E8101` |
| Invalid runner option or partition | `E8102` |
| Report cannot be written | `E8108` |

Run `sealc explain <CODE>` for the long diagnostic. The normative compiler
rules remain in [`../architecture.md`](../architecture.md) and
[`../types.md`](../types.md).
