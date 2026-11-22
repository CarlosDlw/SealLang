func split_once_is(value: Option<(*str, *str)>, left: *str, right: *str) -> bool {
  match value {
    Option::Some(pair) => pair.0 == left && pair.1 == right,
    Option::None => false,
  }
}

func split_once_is_none(value: Option<(*str, *str)>) -> bool {
  match value {
    Option::Some(_) => false,
    Option::None => true,
  }
}

test func split_preserves_empty_pieces() {
  let text: *str = ",a,,b,";
  let expected: [*str; 5] = ["", "a", "", "b", ""];
  let mut index: usize = 0;

  for piece in text.split(",") {
    assert(index < 5);
    assert(piece == expected[index]);
    index += 1;
  }
  assert(index == 5);
}

test func split_without_a_separator_yields_the_whole_string() {
  let text: *str = "abc";
  let mut count: usize = 0;

  for piece in text.split(",") {
    assert(piece == "abc");
    count += 1;
  }
  assert(count == 1);
}

test func split_on_an_empty_source_yields_one_empty_piece() {
  let text: *str = "";
  let mut count: usize = 0;

  for piece in text.split(",") {
    assert(piece == "");
    count += 1;
  }
  assert(count == 1);
}

test func split_on_the_whole_string_yields_two_empty_pieces() {
  let text: *str = "abc";
  let expected: [*str; 2] = ["", ""];
  let mut index: usize = 0;

  for piece in text.split("abc") {
    assert(piece == expected[index]);
    index += 1;
  }
  assert(index == 2);
}

test func split_accepts_a_multibyte_separator() {
  let text: *str = "a🦀b🦀c";
  let expected: [*str; 3] = ["a", "b", "c"];
  let mut index: usize = 0;

  for piece in text.split("🦀") {
    assert(piece == expected[index]);
    index += 1;
  }
  assert(index == 3);
}

test func split_on_an_empty_separator_visits_every_scalar() {
  let text: *str = "a🦀";
  let expected: [*str; 4] = ["", "a", "🦀", ""];
  let mut index: usize = 0;

  for piece in text.split("") {
    assert(piece == expected[index]);
    index += 1;
  }
  assert(index == 4);
}

test func split_on_an_empty_separator_and_empty_source_yields_two_pieces() {
  let text: *str = "";
  let mut count: usize = 0;

  for piece in text.split("") {
    assert(piece == "");
    count += 1;
  }
  assert(count == 2);
}

test func split_once_returns_only_the_first_split() {
  let text: *str = "a,b,c";

  assert(split_once_is(text.split_once(","), "a", "b,c"));
  assert(split_once_is("a,".split_once(","), "a", ""));
  assert(split_once_is(",a".split_once(","), "", "a"));
  assert(split_once_is("abc".split_once(""), "", "abc"));
  assert(split_once_is("".split_once(""), "", ""));
  assert(split_once_is_none(text.split_once("z")));
  assert(split_once_is_none("".split_once(",")));
}

test func lines_handle_trailing_newlines_like_rust() {
  let no_trailing: *str = "a\nb\nc";
  let trailing: *str = "a\n";
  let blank_trailing: *str = "a\n\n";
  let expected_no_trailing: [*str; 3] = ["a", "b", "c"];
  let expected_trailing: [*str; 1] = ["a"];
  let expected_blank: [*str; 2] = ["a", ""];
  let mut index: usize = 0;

  for line in no_trailing.lines() {
    assert(line == expected_no_trailing[index]);
    index += 1;
  }
  assert(index == 3);

  index = 0;
  for line in trailing.lines() {
    assert(line == expected_trailing[index]);
    index += 1;
  }
  assert(index == 1);

  index = 0;
  for line in blank_trailing.lines() {
    assert(line == expected_blank[index]);
    index += 1;
  }
  assert(index == 2);
}

test func lines_strip_carriage_returns_only_before_newlines() {
  let crlf: *str = "a\r\nb\r\n";
  let lone_cr: *str = "a\r";
  let leading_newline: *str = "\n";
  let expected_crlf: [*str; 2] = ["a", "b"];
  let mut index: usize = 0;

  for line in crlf.lines() {
    assert(line == expected_crlf[index]);
    index += 1;
  }
  assert(index == 2);

  for line in lone_cr.lines() {
    assert(line == "a\r");
  }

  index = 0;
  for line in leading_newline.lines() {
    assert(line == "");
    index += 1;
  }
  assert(index == 1);
}

test func lines_skip_the_empty_source_and_keep_interior_empty_lines() {
  let empty: *str = "";
  let interior: *str = "a\n\nb";
  let expected: [*str; 3] = ["a", "", "b"];
  let mut empty_count: usize = 0;
  let mut index: usize = 0;

  for line in empty.lines() {
    let _ = line;
    empty_count += 1;
  }
  assert(empty_count == 0);

  for line in interior.lines() {
    assert(line == expected[index]);
    index += 1;
  }
  assert(index == 3);
}

test func split_whitespace_skips_ascii_and_unicode_whitespace() {
  let ascii: *str = "  a\tb\nc  ";
  let unicode: *str = "a\u{a0}b\u{2003}c\u{3000}d";
  let expected_ascii: [*str; 3] = ["a", "b", "c"];
  let expected_unicode: [*str; 4] = ["a", "b", "c", "d"];
  let mut index: usize = 0;

  for word in ascii.split_whitespace() {
    assert(word == expected_ascii[index]);
    index += 1;
  }
  assert(index == 3);

  index = 0;
  for word in unicode.split_whitespace() {
    assert(word == expected_unicode[index]);
    index += 1;
  }
  assert(index == 4);
}

test func split_whitespace_yields_nothing_for_whitespace_only_input() {
  let text: *str = " \t\n\u{a0}\u{3000}";
  let mut count: usize = 0;

  for word in text.split_whitespace() {
    let _ = word;
    count += 1;
  }
  assert(count == 0);
}
