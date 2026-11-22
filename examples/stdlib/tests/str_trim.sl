test func trim_removes_the_entire_unicode_white_space_table() {
  let text: *str = "\u{9}\u{a}\u{b}\u{c}\u{d}\u{20}\u{85}\u{a0}\u{1680}\u{2000}\u{2001}\u{2002}\u{2003}\u{2004}\u{2005}\u{2006}\u{2007}\u{2008}\u{2009}\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}";
  let wrapped: *str = "\u{2003}hello\u{3000}";

  assert(text.trim() == "");
  assert(text.trim_start() == "");
  assert(text.trim_end() == "");
  assert(wrapped.trim() == "hello");
  assert(wrapped.trim_start() == "hello\u{3000}");
  assert(wrapped.trim_end() == "\u{2003}hello");
}

test func trim_preserves_non_whitespace_and_interior_whitespace() {
  let text: *str = " a b ";
  let zero_width: *str = " \u{200b} ";

  assert(text.trim() == "a b");
  assert(text.trim_start() == "a b ");
  assert(text.trim_end() == " a b");
  assert(zero_width.trim() == "\u{200b}");
  assert(zero_width.trim_start() == "\u{200b} ");
  assert(zero_width.trim_end() == " \u{200b}");
}

test func trim_on_empty_and_whitespace_only_strings_returns_empty() {
  let empty: *str = "";
  let spaces: *str = " \t\u{a0}";

  assert(empty.trim() == "");
  assert(empty.trim_start() == "");
  assert(empty.trim_end() == "");
  assert(spaces.trim() == "");
  assert(spaces.trim_start() == "");
  assert(spaces.trim_end() == "");
}

test func eq_ignore_ascii_case_folds_only_ascii_letters() {
  let upper: *str = "HeLLo";
  let lower: *str = "hello";
  let punctuation: *str = "a-b_c!";
  let non_ascii_lower: *str = "é";
  let non_ascii_upper: *str = "É";

  assert(upper.eq_ignore_ascii_case(lower));
  assert(lower.eq_ignore_ascii_case(upper));
  assert(punctuation.eq_ignore_ascii_case("A-B_C!"));
  assert(!punctuation.eq_ignore_ascii_case("a-b_c?"));
  assert(!non_ascii_lower.eq_ignore_ascii_case(non_ascii_upper));
  assert(!non_ascii_lower.eq_ignore_ascii_case("e"));
  assert(!lower.eq_ignore_ascii_case("hell"));
  assert("".eq_ignore_ascii_case(""));
}
