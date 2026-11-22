test func for_loop_yields_every_scalar_in_order() {
  let text: *str = "a🦀é";
  let expected: [char; 3] = ['a', '🦀', 'é'];
  let mut index: usize = 0;

  for value in text {
    assert(index < 3);
    assert(value == expected[index]);
    index += 1;
  }
  assert(index == 3);
}

test func chars_method_yields_every_scalar_in_order() {
  let text: *str = "a🦀é";
  let mut index: usize = 0;

  for value in text.chars() {
    if index == 0 {
      assert(value == 'a');
    } else if index == 1 {
      assert(value == '🦀');
    } else if index == 2 {
      assert(value == 'é');
    } else {
      assert(false);
    }
    index += 1;
  }
  assert(index == 3);
}

test func for_loop_over_an_empty_string_yields_nothing() {
  let text: *str = "";
  let mut count: usize = 0;

  for value in text {
    let _ = value;
    count += 1;
  }
  assert(count == 0);
}

test func chars_has_next_tracks_the_cursor() {
  let empty: *str = "";
  let text: *str = "a🦀";
  let mut empty_chars = empty.chars();
  let mut chars = text.chars();

  assert(!empty_chars.has_next());
  assert(chars.has_next());
  assert(chars.next() == 'a');
  assert(chars.has_next());
  assert(chars.next() == '🦀');
  assert(!chars.has_next());
}

test func bytes_yields_the_exact_utf8_sequence() {
  let text: *str = "a🦀é";
  let expected: [u8; 7] = [97u8, 240u8, 159u8, 166u8, 128u8, 195u8, 169u8];
  let mut index: usize = 0;

  for value in text.bytes() {
    assert(index < 7);
    assert(value == expected[index]);
    index += 1;
  }
  assert(index == 7);
}

test func bytes_has_next_on_empty_and_ascii_strings() {
  let empty: *str = "";
  let ascii: *str = "hi";
  let mut empty_bytes = empty.bytes();
  let mut bytes = ascii.bytes();

  assert(!empty_bytes.has_next());
  assert(bytes.has_next());
  assert(bytes.next() == 104);
  assert(bytes.has_next());
  assert(bytes.next() == 105);
  assert(!bytes.has_next());
}

test func char_indices_reports_byte_offsets() {
  let text: *str = "a🦀é";
  let offsets: [usize; 3] = [0, 1, 5];
  let values: [char; 3] = ['a', '🦀', 'é'];
  let mut index: usize = 0;

  for pair in text.char_indices() {
    assert(index < 3);
    assert(pair.0 == offsets[index]);
    assert(pair.1 == values[index]);
    index += 1;
  }
  assert(index == 3);
}

test func char_indices_reports_ascii_offsets() {
  let text: *str = "abc";
  let offsets: [usize; 3] = [0, 1, 2];
  let values: [char; 3] = ['a', 'b', 'c'];
  let mut index: usize = 0;

  for pair in text.char_indices() {
    assert(pair.0 == offsets[index]);
    assert(pair.1 == values[index]);
    index += 1;
  }
  assert(index == 3);
}

test func char_indices_on_an_empty_string_yields_nothing() {
  let text: *str = "";
  let mut count: usize = 0;

  for pair in text.char_indices() {
    let _ = pair;
    count += 1;
  }
  assert(count == 0);
}
