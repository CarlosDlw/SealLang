func option_holds(view: Option<*str>, expected: *str) -> bool {
  match view {
    Option::Some(inner) => inner == expected,
    Option::None => false,
  }
}

func option_missing(view: Option<*str>) -> bool {
  match view {
    Option::Some(_) => false,
    Option::None => true,
  }
}

test func is_empty_is_true_only_for_an_empty_string() {
  let empty: *str = "";
  let ascii: *str = "a";
  let nul: *str = "\0";
  let multibyte: *str = "é";

  assert(empty.is_empty());
  assert(!ascii.is_empty());
  assert(!nul.is_empty());
  assert(!multibyte.is_empty());
}

test func get_accepts_every_boundary_pair() {
  let empty: *str = "";
  let text: *str = "a🦀é";

  assert(option_holds(empty.get(0, 0), ""));
  assert(option_missing(empty.get(0, 1)));
  assert(option_missing(empty.get(1, 1)));

  assert(option_holds(text.get(0, 0), ""));
  assert(option_holds(text.get(0, 1), "a"));
  assert(option_holds(text.get(0, 5), "a🦀"));
  assert(option_holds(text.get(0, 7), text));
  assert(option_holds(text.get(1, 1), ""));
  assert(option_holds(text.get(1, 5), "🦀"));
  assert(option_holds(text.get(1, 7), "🦀é"));
  assert(option_holds(text.get(5, 5), ""));
  assert(option_holds(text.get(5, 7), "é"));
  assert(option_holds(text.get(7, 7), ""));
}

test func get_returns_none_for_invalid_ranges_and_boundaries() {
  let text: *str = "a🦀é";

  assert(option_missing(text.get(5, 1)));
  assert(option_missing(text.get(0, 8)));
  assert(option_missing(text.get(2, 5)));
  assert(option_missing(text.get(3, 5)));
  assert(option_missing(text.get(4, 5)));
  assert(option_missing(text.get(0, 6)));
  assert(option_missing(text.get(6, 7)));
  assert(option_missing(text.get(999usize, 1000usize)));
  assert(option_missing(text.get(999usize, 999usize)));
  assert(option_missing(text.get(0, 999usize)));
  assert(option_missing(text.get(999usize, 0)));
}

test func get_and_slice_agree_on_valid_ranges() {
  let text: *str = "a🦀é";

  assert(option_holds(text.get(0, 7), text.slice(0, 7)));
  assert(option_holds(text.get(0, 1), text.slice(0, 1)));
  assert(option_holds(text.get(1, 5), text.slice(1, 5)));
  assert(option_holds(text.get(5, 7), text.slice(5, 7)));
  assert(option_holds(text.get(7, 7), text.slice(7, 7)));
}

test func is_char_boundary_is_false_outside_the_string() {
  let empty: *str = "";
  let text: *str = "a🦀é";

  assert(empty.is_char_boundary(0));
  assert(!empty.is_char_boundary(1));
  assert(text.is_char_boundary(0));
  assert(text.is_char_boundary(1));
  assert(!text.is_char_boundary(2));
  assert(!text.is_char_boundary(3));
  assert(!text.is_char_boundary(4));
  assert(text.is_char_boundary(5));
  assert(!text.is_char_boundary(6));
  assert(text.is_char_boundary(7));
  assert(!text.is_char_boundary(8));
  assert(!text.is_char_boundary(999usize));
}

test func from_utf8_accepts_every_supported_encoding() {
  let empty: [u8; 0] = [];
  let ascii: [u8; 2] = [104u8, 105u8];
  let two_byte_min: [u8; 2] = [0xc2u8, 0x80u8];
  let two_byte: [u8; 2] = [0xc3u8, 0xa9u8];
  let three_byte_min: [u8; 3] = [0xe0u8, 0xa0u8, 0x80u8];
  let three_byte: [u8; 3] = [0xe2u8, 0x82u8, 0xacu8];
  let before_surrogate: [u8; 3] = [0xedu8, 0x9fu8, 0xbfu8];
  let four_byte_min: [u8; 4] = [0xf0u8, 0x90u8, 0x80u8, 0x80u8];
  let four_byte: [u8; 4] = [0xf0u8, 0x9fu8, 0xa6u8, 0x80u8];
  let nul: [u8; 3] = [97u8, 0u8, 98u8];
  let maximum: [u8; 4] = [0xf4u8, 0x8fu8, 0xbfu8, 0xbfu8];

  assert(option_holds(std::str::from_utf8(&empty[..]), ""));
  assert(option_holds(std::str::from_utf8(&ascii[..]), "hi"));
  assert(option_holds(std::str::from_utf8(&two_byte_min[..]), "\u{80}"));
  assert(option_holds(std::str::from_utf8(&two_byte[..]), "é"));
  assert(option_holds(std::str::from_utf8(&three_byte_min[..]), "\u{800}"));
  assert(option_holds(std::str::from_utf8(&three_byte[..]), "€"));
  assert(option_holds(std::str::from_utf8(&before_surrogate[..]), "\u{d7ff}"));
  assert(option_holds(std::str::from_utf8(&four_byte_min[..]), "\u{10000}"));
  assert(option_holds(std::str::from_utf8(&four_byte[..]), "🦀"));
  assert(option_holds(std::str::from_utf8(&nul[..]), "a\0b"));
  assert(option_holds(std::str::from_utf8(&maximum[..]), "\u{10ffff}"));
}

test func from_utf8_rejects_malformed_input() {
  let lone_continuation: [u8; 1] = [0x80u8];
  let bad_trailing: [u8; 2] = [0x61u8, 0x80u8];
  let truncated_two: [u8; 1] = [0xc2u8];
  let truncated_three: [u8; 2] = [0xe2u8, 0x82u8];
  let truncated_four: [u8; 3] = [0xf0u8, 0x9fu8, 0xa6u8];
  let overlong_two: [u8; 2] = [0xc0u8, 0x80u8];
  let overlong_three: [u8; 3] = [0xe0u8, 0x80u8, 0x80u8];
  let overlong_four: [u8; 4] = [0xf0u8, 0x80u8, 0x80u8, 0x80u8];
  let surrogate: [u8; 3] = [0xedu8, 0xa0u8, 0x80u8];
  let above_maximum: [u8; 4] = [0xf4u8, 0x90u8, 0x80u8, 0x80u8];
  let invalid_lead: [u8; 4] = [0xf5u8, 0x80u8, 0x80u8, 0x80u8];
  let bad_continuation: [u8; 2] = [0xc2u8, 0x20u8];

  assert(option_missing(std::str::from_utf8(&lone_continuation[..])));
  assert(option_missing(std::str::from_utf8(&bad_trailing[..])));
  assert(option_missing(std::str::from_utf8(&truncated_two[..])));
  assert(option_missing(std::str::from_utf8(&truncated_three[..])));
  assert(option_missing(std::str::from_utf8(&truncated_four[..])));
  assert(option_missing(std::str::from_utf8(&overlong_two[..])));
  assert(option_missing(std::str::from_utf8(&overlong_three[..])));
  assert(option_missing(std::str::from_utf8(&overlong_four[..])));
  assert(option_missing(std::str::from_utf8(&surrogate[..])));
  assert(option_missing(std::str::from_utf8(&above_maximum[..])));
  assert(option_missing(std::str::from_utf8(&invalid_lead[..])));
  assert(option_missing(std::str::from_utf8(&bad_continuation[..])));
}

test func from_utf8_unchecked_builds_the_same_view() {
  let data: [u8; 7] = [97u8, 240u8, 159u8, 166u8, 128u8, 195u8, 169u8];
  let bytes: *[u8] = &data[..];
  let checked = match std::str::from_utf8(bytes) {
    Option::Some(view) => view,
    Option::None => "",
  };
  let unchecked = unsafe { std::str::from_utf8_unchecked(bytes) };

  assert(checked == "a🦀é");
  assert(unchecked == checked);
}

test func option_views_keep_the_source_loan() {
  let data: [u8; 7] = [97u8, 240u8, 159u8, 166u8, 128u8, 195u8, 169u8];
  let bytes: *[u8] = &data[..];
  let text = match std::str::from_utf8(bytes) {
    Option::Some(view) => view,
    Option::None => "",
  };
  let scalar = match text.get(1, 5) {
    Option::Some(view) => view,
    Option::None => "",
  };

  assert(scalar == "🦀");
}

test func split_at_returns_both_halves_at_every_boundary() {
  let text: *str = "a🦀é";

  let after_ascii = text.split_at(1);
  assert(after_ascii.0 == "a");
  assert(after_ascii.1 == "🦀é");

  let after_crab = text.split_at(5);
  assert(after_crab.0 == "a🦀");
  assert(after_crab.1 == "é");

  let at_start = text.split_at(0);
  assert(at_start.0 == "");
  assert(at_start.1 == text);

  let at_end = text.split_at(text.len());
  assert(at_end.0 == text);
  assert(at_end.1 == "");

  let empty: *str = "";
  let empty_parts = empty.split_at(0);
  assert(empty_parts.0 == "");
  assert(empty_parts.1 == "");
}
