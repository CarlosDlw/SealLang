func find_is(value: Option<usize>, expected: usize) -> bool {
  match value {
    Option::Some(index) => index == expected,
    Option::None => false,
  }
}

func find_is_none(value: Option<usize>) -> bool {
  match value {
    Option::Some(_) => false,
    Option::None => true,
  }
}

func same_find(left: Option<usize>, right: Option<usize>) -> bool {
  match left {
    Option::Some(left_index) => match right {
      Option::Some(right_index) => left_index == right_index,
      Option::None => false,
    },
    Option::None => match right {
      Option::Some(_) => false,
      Option::None => true,
    },
  }
}

func naive_find(text: *str, needle: *str) -> Option<usize> {
  let haystack = text.as_bytes();
  let pattern = needle.as_bytes();
  let haystack_len = haystack.len();
  let pattern_len = pattern.len();

  if pattern_len == 0 {
    return Option::Some(0);
  }
  if pattern_len > haystack_len {
    return Option::None;
  }

  let mut start: usize = 0;
  while start + pattern_len <= haystack_len {
    let mut index: usize = 0;
    let mut matched = true;
    while index < pattern_len {
      if haystack[start + index] != pattern[index] {
        matched = false;
        break;
      }
      index += 1;
    }
    if matched {
      return Option::Some(start);
    }
    start += 1;
  }
  Option::None
}

test func starts_with_handles_empty_prefix_and_equal_strings() {
  let text: *str = "a🦀é";
  let empty: *str = "";

  assert(text.starts_with(""));
  assert(text.starts_with("a"));
  assert(text.starts_with("a🦀"));
  assert(text.starts_with(text));
  assert(empty.starts_with(""));
  assert(!text.starts_with("🦀"));
  assert(!text.starts_with("é"));
  assert(!text.starts_with("a🦀éx"));
  assert(!empty.starts_with("a"));
}

test func ends_with_handles_empty_suffix_and_equal_strings() {
  let text: *str = "a🦀é";
  let empty: *str = "";

  assert(text.ends_with(""));
  assert(text.ends_with("é"));
  assert(text.ends_with("🦀é"));
  assert(text.ends_with(text));
  assert(empty.ends_with(""));
  assert(!text.ends_with("🦀"));
  assert(!text.ends_with("a"));
  assert(!text.ends_with("xa🦀é"));
  assert(!empty.ends_with("a"));
}

test func contains_handles_every_needle_position() {
  let text: *str = "a🦀é";

  assert(text.contains(""));
  assert(text.contains("a"));
  assert(text.contains("🦀"));
  assert(text.contains("é"));
  assert(text.contains("a🦀é"));
  assert(text.contains("🦀é"));
  assert(!text.contains("z"));
  assert(!"".contains("a"));
}

test func find_returns_byte_indices() {
  let text: *str = "a🦀é";

  assert(find_is(text.find(""), 0));
  assert(find_is(text.find("a"), 0));
  assert(find_is(text.find("🦀"), 1));
  assert(find_is(text.find("é"), 5));
  assert(find_is(text.find("🦀é"), 1));
  assert(find_is_none(text.find("z")));
  assert(find_is_none("".find("a")));
}

test func find_returns_the_first_overlapping_candidate() {
  assert(find_is("aaaa".find("aa"), 0));
  assert(find_is("abababa".find("aba"), 0));
  assert(find_is("baabaa".find("aabaa"), 1));
  assert(find_is("abcabcabc".find("cab"), 2));
  assert(find_is("aab".find("ab"), 1));
  assert(find_is("aaab".find("aab"), 1));
  assert(find_is("abba".find("bba"), 1));
}

test func two_way_matches_the_naive_reference_on_a_corpus() {
  let haystacks: [*str; 12] = [
    "",
    "a",
    "aa",
    "aab",
    "aaab",
    "aaaaa",
    "abababab",
    "abba",
    "banana",
    "a🦀éa🦀é",
    "aaaaaaaaab",
    "abcabcabc",
  ];
  let patterns: [*str; 16] = [
    "",
    "a",
    "aa",
    "aaa",
    "ab",
    "aab",
    "aba",
    "ba",
    "bba",
    "abc",
    "🦀",
    "é",
    "a🦀",
    "zz",
    "aaaaab",
    "abab",
  ];

  for haystack in &haystacks {
    for needle in &patterns {
      let text: *str = *haystack;
      let pattern: *str = *needle;
      assert(same_find(text.find(pattern), naive_find(text, pattern)));
    }
  }
}
