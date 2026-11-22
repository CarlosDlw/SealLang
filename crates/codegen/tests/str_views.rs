//! Runtime behavior of string and byte-slice views.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run};

/// Compiles `source` at both optimization levels and asserts the exit status.
fn assert_returns(source: &str, expected: i32) {
    for level in [OptLevel::O0, OptLevel::O2] {
        let output = TempOutput::new("str-views");
        compile(source, output.path(), Emit::Executable, level, false)
            .unwrap_or_else(|error| panic!("compilation failed at {level:?}: {error}"));
        assert_eq!(
            run(output.path()).code(),
            Some(expected),
            "unexpected status at {level:?}"
        );
    }
}

#[test]
fn slice_returns_the_selected_bytes() {
    assert_returns(
        "\
func main() -> i32 {
  let text: *str = \"hello\";
  let view: *str = text.slice(1, 4);
  if view == \"ell\" { 0 } else { 1 }
}",
        0,
    );
}

#[test]
fn as_bytes_keeps_the_data_and_length() {
    assert_returns(
        "\
func main() -> i32 {
  let text: *str = \"hello\";
  let bytes: *[u8] = text.as_bytes();
  if bytes.len() == 5 && bytes[0] == 104 && bytes[4] == 111 { 0 } else { 1 }
}",
        0,
    );
}

#[test]
fn as_str_unchecked_keeps_the_data_and_length() {
    assert_returns(
        "\
func main() -> i32 {
  let text: *str = \"hello\";
  let bytes: *[u8] = text.as_bytes();
  let view: *str = unsafe { bytes.as_str_unchecked() };
  if view == text { 0 } else { 1 }
}",
        0,
    );
}

#[test]
fn char_boundaries_follow_utf8_encoding() {
    assert_returns(
        "\
func main() -> i32 {
  let text: *str = \"\u{e9}\";
  if text.is_char_boundary(0)
      && !text.is_char_boundary(1)
      && text.is_char_boundary(2)
      && !text.is_char_boundary(3) {
    0
  } else {
    1
  }
}",
        0,
    );
}

#[test]
fn the_end_of_an_empty_string_is_a_boundary() {
    assert_returns(
        "\
func main() -> i32 {
  let text: *str = \"\";
  if text.is_char_boundary(0) && !text.is_char_boundary(1) { 0 } else { 1 }
}",
        0,
    );
}
