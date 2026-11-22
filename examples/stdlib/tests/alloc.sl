// Phase 1 coverage for the allocation layout vocabulary. Allocation itself is
// intentionally absent; these tests exercise layout construction, validation,
// checked array sizing, and the Copy value semantics of `Layout`.

use std::alloc::alloc;
use std::alloc::alloc_zeroed;
use std::alloc::dealloc;
use std::alloc::AllocError;
use std::alloc::Layout;
use std::alloc::LayoutError;

struct AllocLayoutSample {
  byte: u8,
  word: u64,
}

func max_isize() -> usize {
  (0usize.wrapping_sub(1)) >> 1
}

func expect_layout(result: Result<Layout, LayoutError>) -> Layout {
  match result {
    Result::Ok(layout) => layout,
    Result::Err(_) => panic("expected a valid allocation layout"),
  }
}

func is_size_overflow(result: Result<Layout, LayoutError>) -> bool {
  match result {
    Result::Ok(_) => false,
    Result::Err(error) => match error {
      LayoutError::SizeOverflow => true,
      LayoutError::AlignmentNotPowerOfTwo => false,
    },
  }
}

func is_alignment_error(result: Result<Layout, LayoutError>) -> bool {
  match result {
    Result::Ok(_) => false,
    Result::Err(error) => match error {
      LayoutError::AlignmentNotPowerOfTwo => true,
      LayoutError::SizeOverflow => false,
    },
  }
}

func layout_size(layout: Layout) -> usize {
  layout.size()
}

func expect_pointer(result: Result<*raw mut u8, AllocError>) -> *raw mut u8 {
  match result {
    Result::Ok(pointer) => pointer,
    Result::Err(_) => panic("expected a successful allocation"),
  }
}

test func layout_new_matches_the_type_introspection_builtins() {
  assert(Layout::new::<u8>().size() == size_of::<u8>());
  assert(Layout::new::<u8>().align() == align_of::<u8>());
  assert(Layout::new::<u16>().size() == size_of::<u16>());
  assert(Layout::new::<u16>().align() == align_of::<u16>());
  assert(Layout::new::<u32>().size() == size_of::<u32>());
  assert(Layout::new::<u32>().align() == align_of::<u32>());
  assert(Layout::new::<u64>().size() == size_of::<u64>());
  assert(Layout::new::<u64>().align() == align_of::<u64>());
  assert(Layout::new::<usize>().size() == size_of::<usize>());
  assert(Layout::new::<usize>().align() == align_of::<usize>());
  assert(Layout::new::<AllocLayoutSample>().size() == size_of::<AllocLayoutSample>());
  assert(Layout::new::<AllocLayoutSample>().align() == align_of::<AllocLayoutSample>());
}

test func layout_array_reports_the_expected_byte_sizes() {
  assert(expect_layout(Layout::array::<u8>(0)).size() == 0);
  assert(expect_layout(Layout::array::<u8>(1)).size() == 1);
  assert(expect_layout(Layout::array::<u8>(17)).size() == 17);
  assert(expect_layout(Layout::array::<u64>(0)).size() == 0);
  assert(expect_layout(Layout::array::<u64>(3)).size() == 24);
  assert(expect_layout(Layout::array::<u64>(3)).align() == align_of::<u64>());
}

test func layout_array_checks_maximum_size_without_a_hardcoded_width() {
  let max = max_isize();
  let count = max / 8;

  assert(expect_layout(Layout::array::<u64>(count)).size() == count * 8);
  assert(is_size_overflow(Layout::array::<u64>(count + 1)));
  assert(is_size_overflow(Layout::array::<u8>(max + 1)));
}

test func layout_from_size_align_rejects_non_power_of_two_alignments() {
  let sizes: [usize; 6] = [0, 1, 8, 64, 4096, 4096];
  let alignments: [usize; 6] = [0, 3, 5, 6, 7, 12];

  let mut index: usize = 0;
  while index < 6 {
    assert(is_alignment_error(Layout::from_size_align(sizes[index], alignments[index])));
    index += 1;
  }
}

test func layout_from_size_align_enforces_the_maximum_object_size() {
  let max = max_isize();

  assert(expect_layout(Layout::from_size_align(max, 1)).size() == max);
  assert(is_size_overflow(Layout::from_size_align(max, 2)));
  assert(expect_layout(Layout::from_size_align(1, 4096)).align() == 4096);
}

test func layout_is_copy_by_value() {
  let layout = Layout::new::<u64>();

  assert(layout_size(layout) == size_of::<u64>());
  assert(layout_size(layout) == size_of::<u64>());
}

test func alloc_returns_aligned_non_null_blocks() {
  let sizes: [usize; 4] = [1, 16, 64, 4096];
  let alignments: [usize; 4] = [1, 16, 64, 4096];
  let mut index: usize = 0;

  while index < 4 {
    let layout = expect_layout(Layout::from_size_align(sizes[index], alignments[index]));

    unsafe {
      let pointer = expect_pointer(alloc(layout));
      assert(!pointer.is_null());
      assert((pointer as usize) % alignments[index] == 0);
      dealloc(pointer, layout);
    }

    index += 1;
  }
}

test func alloc_supports_writing_and_reading_bytes() {
  let layout = expect_layout(Layout::from_size_align(4, 1));

  unsafe {
    let pointer = expect_pointer(alloc(layout));
    *pointer = 10;
    *(pointer.add(1)) = 20;
    *(pointer.add(2)) = 30;
    *(pointer.add(3)) = 40;

    assert(*pointer == 10);
    assert(*(pointer.add(1)) == 20);
    assert(*pointer + *(pointer.add(3)) == 50);

    dealloc(pointer, layout);
  }
}

test func alloc_zeroed_clears_every_byte_at_high_alignment() {
  let layout = expect_layout(Layout::from_size_align(64, 4096));

  unsafe {
    let pointer = expect_pointer(alloc_zeroed(layout));
    let mut index: usize = 0;

    while index < 64 {
      assert(*(pointer.add(index)) == 0);
      index += 1;
    }

    dealloc(pointer, layout);
  }
}

test func zero_size_allocation_is_a_non_null_dangling_pointer() {
  let layout = expect_layout(Layout::from_size_align(0, 4096));

  unsafe {
    let pointer = expect_pointer(alloc(layout));
    let zeroed = expect_pointer(alloc_zeroed(layout));

    assert(!pointer.is_null());
    assert(!zeroed.is_null());

    dealloc(pointer, layout);
    dealloc(zeroed, layout);
  }
}

test func dealloc_accepts_null_for_zero_and_nonzero_layouts() {
  let zero = expect_layout(Layout::from_size_align(0, 1));
  let non_zero = expect_layout(Layout::from_size_align(16, 8));
  let pointer: *raw mut u8 = 0usize as *raw mut u8;

  unsafe {
    dealloc(pointer, zero);
    dealloc(pointer, non_zero);
  }
}
