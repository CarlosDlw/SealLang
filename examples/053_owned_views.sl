// `owned_view(owner, source)` turns a raw pointer into a checked reference and
// attaches the reference's loan to `owner`. The owner is the storage that
// guarantees the allocation, not the pointer field itself.
//
// This program builds a view over a thin value, a shared slice, a mutable
// slice, and a string, then exercises a promoted temporary owner and a static
// owner. Every construction is `unsafe`; every use of the resulting reference
// is checked by the borrow checker like any other reference.
//
// The documented result is 0. A wrong pointee or a wrong mutation returns a
// non-zero status; a missing loan is rejected at compile time.

struct ThinBuffer {
  pointer: *raw mut i32,
}

func read_thin(buffer: *ThinBuffer) -> *i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer as *raw i32) }
}

func write_thin(buffer: *mut ThinBuffer) -> *mut i32 {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}

struct SliceBuffer {
  pointer: *raw [i32],
}

func read_slice(buffer: *SliceBuffer) -> *[i32] {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}

struct MutableSliceBuffer {
  pointer: *raw mut [i32],
}

func write_slice(buffer: *mut MutableSliceBuffer) -> *mut [i32] {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}

struct TextBuffer {
  pointer: *raw str,
}

func read_text(buffer: *TextBuffer) -> *str {
  unsafe { owned_view(*buffer, (*buffer).pointer) }
}

func identity(buffer: *TextBuffer) -> *TextBuffer {
  buffer
}

struct StaticBuffer {
  pointer: *raw i32,
}

static VALUE: i32 = 7;
static STATIC_BUFFER: StaticBuffer = StaticBuffer {
  pointer: &VALUE as *raw i32,
};

func read_static() -> *i32 {
  unsafe { owned_view(STATIC_BUFFER, STATIC_BUFFER.pointer) }
}

func main() -> i32 {
  let mut value = 41;
  let mut thin = ThinBuffer {
    pointer: &mut value as *raw mut i32,
  };
  let shared = read_thin(&thin);
  if *shared != 41 {
    return 1;
  }

  // The shared loan ends after its last use, so the same owner can be viewed
  // exclusively next.
  let exclusive = write_thin(&mut thin);
  unsafe { *exclusive = 42; }
  if value != 42 {
    return 2;
  }

  let values = [10, 20, 30];
  let slice_buffer = SliceBuffer {
    pointer: &values[..] as *raw [i32],
  };
  let slice = read_slice(&slice_buffer);
  if len(slice) != 3 || slice[1] != 20 {
    return 3;
  }

  let mut elements = [1, 2, 3];
  let mut mutable_buffer = MutableSliceBuffer {
    pointer: &mut elements[..] as *raw mut [i32],
  };
  let mutable_slice = write_slice(&mut mutable_buffer);
  unsafe { mutable_slice[2] = 42; }
  if elements[2] != 42 {
    return 4;
  }

  let text = "seal";
  let text_buffer = TextBuffer {
    pointer: text as *raw str,
  };
  let view = read_text(&text_buffer);
  if view != "seal" {
    return 5;
  }

  // A dereference of a temporary reference is a checked owner. Lowering
  // promotes the reference to the enclosing block, so the view stays valid
  // after this statement without naming a separate local.
  let source = identity(&text_buffer);
  let temporary = unsafe {
    owned_view(*identity(&text_buffer), (*source).pointer)
  };
  if temporary != "seal" {
    return 6;
  }

  let static_number = read_static();
  if *static_number != 7 {
    return 7;
  }

  0
}
