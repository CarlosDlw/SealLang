//! End-to-end behavior of owned views over thin, slice, and string pointees.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

fn assert_returns(source: &str, expected: i32, suffix: &str) {
    for opt_level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new(suffix);
        compile(source, output.path(), Emit::Executable, opt_level, false)
            .unwrap_or_else(|error| panic!("compilation failed at {opt_level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "unexpected result at {opt_level:?}"
        );
    }
}

#[test]
fn a_thin_owned_view_reads_and_writes_the_pointee() {
    let source = "\
struct Buffer { pointer: *raw mut i32 }
func shared(buffer: *Buffer) -> *i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer as *raw i32) }
}
func exclusive(buffer: *mut Buffer) -> *mut i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}
func main() -> i32 {
  let mut value = 40;
  let mut buffer = Buffer { pointer: &mut value as *raw mut i32 };
  let view = shared(&buffer);
  if *view != 40 { return 1; }
  let edit = exclusive(&mut buffer);
  let copy = edit;
  unsafe { *copy = 42; }
  value
}";
    assert_returns(source, 42, "owned-view-thin");
}

#[test]
fn a_temporary_owner_is_promoted_and_used_after_its_statement() {
    let source = "\
struct Buffer { pointer: *raw i32 }
func identity(buffer: *Buffer) -> *Buffer { buffer }
func main() -> i32 {
  let value = 41;
  let buffer = Buffer { pointer: &value as *raw i32 };
  let source = identity(&buffer);
  let view = unsafe {
    owned_view(*identity(&buffer), (*source).pointer)
  };
  *view + 1
}";
    assert_returns(source, 42, "owned-view-temporary-owner");
}

#[test]
fn a_static_owner_lends_a_view_for_the_static_lifetime() {
    let source = "\
struct Buffer { pointer: *raw i32 }
static VALUE: i32 = 41;
static BUFFER: Buffer = Buffer { pointer: &VALUE as *raw i32 };
func view() -> *i32 {
  unsafe { owned_view(BUFFER, BUFFER.pointer) }
}
func main() -> i32 { *view() + 1 }";
    assert_returns(source, 42, "owned-view-static-owner");
}

#[test]
fn an_owned_slice_preserves_length_and_elements() {
    let source = "\
struct SliceBuffer { pointer: *raw [i32] }
func as_slice(buffer: *SliceBuffer) -> *[i32] {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}
func main() -> i32 {
  let values = [40, 41, 42];
  let buffer = SliceBuffer { pointer: &values[..] as *raw [i32] };
  let view = as_slice(&buffer);
  if len(view) != 3 { return 1; }
  if view[0] + view[2] != 82 { return 2; }
  42
}";
    assert_returns(source, 42, "owned-view-slice");
}

#[test]
fn an_owned_mutable_slice_writes_through_the_view() {
    let source = "\
struct SliceBuffer { pointer: *raw mut [i32] }
func as_slice(buffer: *mut SliceBuffer) -> *mut [i32] {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}
func main() -> i32 {
  let mut values = [7, 8, 9];
  let mut buffer = SliceBuffer { pointer: &mut values[..] as *raw mut [i32] };
  let view = as_slice(&mut buffer);
  if len(view) != 3 { return 1; }
  unsafe { view[1] = 41; }
  if values[1] != 41 { return 2; }
  42
}";
    assert_returns(source, 42, "owned-view-mut-slice");
}

#[test]
fn an_owned_slice_can_be_empty() {
    let source = "\
struct SliceBuffer { pointer: *raw [i32] }
func as_slice(buffer: *SliceBuffer) -> *[i32] {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}
func main() -> i32 {
  let values = [7];
  let buffer = SliceBuffer { pointer: &values[0..0] as *raw [i32] };
  let view = as_slice(&buffer);
  if len(view) != 0 { 1 } else { 42 }
}";
    assert_returns(source, 42, "owned-view-empty-slice");
}

#[test]
fn an_owned_string_preserves_contents() {
    let source = "\
struct TextBuffer { pointer: *raw str }
func as_str(buffer: *TextBuffer) -> *str {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}
func main() -> i32 {
  let text = \"hello\";
  let buffer = TextBuffer { pointer: text as *raw str };
  let view = as_str(&buffer);
  if view == \"hello\" { 42 } else { 1 }
}";
    assert_returns(source, 42, "owned-view-str");
}

#[test]
fn an_owned_string_can_be_empty() {
    let source = "\
struct TextBuffer { pointer: *raw str }
func as_str(buffer: *TextBuffer) -> *str {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}
func main() -> i32 {
  let text = \"\";
  let buffer = TextBuffer { pointer: text as *raw str };
  let view = as_str(&buffer);
  if view == \"\" { 42 } else { 1 }
}";
    assert_returns(source, 42, "owned-view-empty-str");
}
