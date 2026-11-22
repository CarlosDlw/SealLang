//! Runtime behavior of string and byte-slice views.

use seal_codegen::Emit;
use seal_session::OptLevel;

mod support;

use support::{TempOutput, compile, run, run_captured};

/// Compiles `source` at both optimization levels and asserts the exit status.
fn assert_returns(source: &str, expected: i32) {
    for level in [OptLevel::O0, OptLevel::O2] {
        for debug_info in [false, true] {
            let output = TempOutput::new("str-views");
            compile(source, output.path(), Emit::Executable, level, debug_info)
                .unwrap_or_else(|error| panic!("compilation failed at {level:?}: {error}"));
            assert_eq!(
                run(output.path()).code(),
                Some(expected),
                "unexpected status at {level:?} with debug info {debug_info}"
            );
        }
    }
}

/// Compiles and runs a panicking program at every supported codegen setting.
fn assert_panics(source: &str, message: &str, line: u32) {
    for level in [OptLevel::O0, OptLevel::O2] {
        for debug_info in [false, true] {
            let output = TempOutput::new("str-view-panic");
            compile(source, output.path(), Emit::Executable, level, debug_info)
                .unwrap_or_else(|error| panic!("compilation failed at {level:?}: {error}"));
            let result = run_captured(output.path());
            assert_eq!(
                result.status.code(),
                Some(101),
                "unexpected status at {level:?} with debug info {debug_info}"
            );
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                stderr.contains(&format!("panic: {message} at <test.sl>:{line}:")),
                "unexpected stderr at {level:?} with debug info {debug_info}: {stderr}"
            );
        }
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
fn type_info_names_string_views_without_dereferencing() {
    assert_returns(
        r#"
func main() -> i32 {
  let text: *str = "abc";
  let bytes: *[u8] = text.as_bytes();
  let text_name: *str = type_name::<*str>();
  let bytes_name: *str = type_name::<*[u8]>();
  if text_name != "*str" { return 1; }
  if bytes_name != "*[u8]" { return 2; }
  0
}
"#,
        0,
    );
}

#[test]
fn slice_accepts_every_boundary_range() {
    assert_returns(
        r#"
func main() -> i32 {
  let text: *str = "héllo🦀";
  let whole: *str = text.slice(0, text.len());
  let empty_at_one: *str = text.slice(1, 1);
  let empty_at_zero: *str = text.slice(0, 0);
  let empty_at_end: *str = text.slice(text.len(), text.len());
  if whole != text { return 1; }
  if empty_at_one != "" { return 2; }
  if empty_at_zero != "" { return 3; }
  if empty_at_end != "" { return 4; }
  0
}
"#,
        0,
    );
}

#[test]
fn multibyte_slices_select_whole_scalars() {
    assert_returns(
        "\
func main() -> i32 {
  let text: *str = \"é🦀\";
  let first: *str = text.slice(0, 2);
  let second: *str = text.slice(2, 6);
  if first != \"é\" { return 1; }
  if second != \"🦀\" { return 2; }
  0
}",
        0,
    );
}

#[test]
fn as_bytes_length_matches_len_for_every_kind_of_string() {
    assert_returns(
        r#"
func main() -> i32 {
  let ascii: *str = "abc";
  let multibyte: *str = "aé🦀";
  let empty: *str = "";
  let nul: *str = "a\0b";
  if ascii.as_bytes().len() != ascii.len() { return 1; }
  if multibyte.as_bytes().len() != multibyte.len() { return 2; }
  if empty.as_bytes().len() != empty.len() { return 3; }
  if nul.as_bytes().len() != nul.len() { return 4; }
  if multibyte.len() != 7 { return 5; }
  0
}
"#,
        0,
    );
}

#[test]
fn char_at_decodes_every_utf8_width() {
    assert_returns(
        r#"
func main() -> i32 {
  if "A".char_at(0) != 'A' { return 1; }
  if "\u{7f}".char_at(0) != '\u{7f}' { return 2; }
  if "\u{80}".char_at(0) != '\u{80}' { return 3; }
  if "\u{7ff}".char_at(0) != '\u{7ff}' { return 4; }
  if "\u{800}".char_at(0) != '\u{800}' { return 5; }
  if "\u{ffff}".char_at(0) != '\u{ffff}' { return 6; }
  if "\u{10000}".char_at(0) != '\u{10000}' { return 7; }
  if "\u{10ffff}".char_at(0) != '\u{10ffff}' { return 8; }
  if "a\0b".char_at(1) != '\0' { return 9; }
  0
}
"#,
        0,
    );
}

#[test]
fn char_at_decodes_a_scalar_after_a_prefix() {
    assert_returns(
        r#"
func main() -> i32 {
  let text: *str = "a🦀é";
  if text.char_at(1) != '🦀' { return 1; }
  if text.char_at(5) != 'é' { return 2; }
  0
}
"#,
        0,
    );
}

#[test]
fn char_boundaries_cover_every_scalar_start() {
    assert_returns(
        r#"
func main() -> i32 {
  let text: *str = "a🦀é";
  if !text.is_char_boundary(0) { return 1; }
  if !text.is_char_boundary(1) { return 2; }
  if text.is_char_boundary(2) { return 3; }
  if text.is_char_boundary(3) { return 4; }
  if text.is_char_boundary(4) { return 5; }
  if !text.is_char_boundary(5) { return 6; }
  if text.is_char_boundary(6) { return 7; }
  if !text.is_char_boundary(7) { return 8; }
  0
}
"#,
        0,
    );
}

#[test]
fn slice_panics_when_start_follows_end() {
    assert_panics(
        "func main() -> i32 {\n  let text: *str = \"abc\";\n  let view: *str = text.slice(2, 1);\n  0\n}",
        "slice range starts after it ends",
        3,
    );
}

#[test]
fn slice_panics_when_end_is_past_the_length() {
    assert_panics(
        "func main() -> i32 {\n  let text: *str = \"abc\";\n  let view: *str = text.slice(0, 4);\n  0\n}",
        "slice range end is out of bounds",
        3,
    );
}

#[test]
fn slice_panics_inside_two_three_and_four_byte_scalars() {
    for (text, end) in [("é", 2), ("€", 3), ("🦀", 4)] {
        let source = format!(
            "func main() -> i32 {{\n  let text: *str = \"{text}\";\n  let view: *str = text.slice(1, {end});\n  0\n}}"
        );
        assert_panics(&source, "slice range starts inside a character", 3);
    }
    assert_panics(
        "func main() -> i32 {\n  let text: *str = \"é\";\n  let view: *str = text.slice(0, 1);\n  0\n}",
        "slice range ends inside a character",
        3,
    );
}

#[test]
fn char_at_panics_on_a_continuation_index() {
    assert_panics(
        "func main() -> i32 {\n  let text: *str = \"é\";\n  let value: char = text.char_at(1);\n  0\n}",
        "char_at index is not a char boundary",
        3,
    );
}

#[test]
fn char_at_panics_at_or_past_the_length() {
    assert_panics(
        "func main() -> i32 {\n  let text: *str = \"abc\";\n  let value: char = text.char_at(3);\n  0\n}",
        "char_at index is out of bounds",
        3,
    );
    assert_panics(
        "func main() -> i32 {\n  let text: *str = \"\";\n  let value: char = text.char_at(0);\n  0\n}",
        "char_at index is out of bounds",
        3,
    );
}

#[test]
fn char_at_rejects_invalid_utf8() {
    assert_panics(
        "\
func main() -> i32 {
  let data: [u8; 2] = [192u8, 128u8];
  let bytes: *[u8] = &data[..];
  let text: *str = unsafe { bytes.as_str_unchecked() };
  let value: char = text.char_at(0);
  0
}",
        "string contains invalid UTF-8",
        5,
    );
    assert_panics(
        "\
func main() -> i32 {
  let data: [u8; 2] = [226u8, 130u8];
  let bytes: *[u8] = &data[..];
  let text: *str = unsafe { bytes.as_str_unchecked() };
  let value: char = text.char_at(0);
  0
}",
        "string contains invalid UTF-8",
        5,
    );
    assert_panics(
        "\
func main() -> i32 {
  let data: [u8; 3] = [237u8, 160u8, 128u8];
  let bytes: *[u8] = &data[..];
  let text: *str = unsafe { bytes.as_str_unchecked() };
  let value: char = text.char_at(0);
  0
}",
        "string contains invalid UTF-8",
        5,
    );
    assert_panics(
        "\
func main() -> i32 {
  let data: [u8; 4] = [244u8, 144u8, 128u8, 128u8];
  let bytes: *[u8] = &data[..];
  let text: *str = unsafe { bytes.as_str_unchecked() };
  let value: char = text.char_at(0);
  0
}",
        "string contains invalid UTF-8",
        5,
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
