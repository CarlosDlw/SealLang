// Enums are nominal sum types. A unit variant is written `Enum::Variant`; a
// variant with named fields is a braced literal. Constructing a variant always
// initializes its discriminant and exactly its own fields.
//
// An enum value owns its payload and is moved as a whole, just like a struct.
// It may be stored in a struct field, passed to a function, and returned from
// one. `match` is not part of the language yet, so this program observes the
// values through construction, moves, and method calls.

enum Status {
  Ready,
  Busy,
  Failed { code: i32 },
}

struct Job {
  status: Status,
  retries: i32,
}

impl Status {
  // An associated function on an enum can build one of its variants.
  func ready() -> Status {
    Status::Ready
  }
}

func enqueue(status: Status, retries: i32) -> Job {
  Job { status: status, retries: retries }
}

func discard(status: Status) -> i32 {
  // `status` is moved into this function and owns its payload until it returns.
  1
}

func main() -> i32 {
  let ready = Status::ready();
  let failed = Status::Failed { code: 7 };
  let busy = Status::Busy;

  // The struct owns `ready`; constructing it moves the enum value into `job`.
  let mut job = enqueue(ready, 2);
  job.retries += 1;

  // Both payload and unit variants move into the call.
  let first = discard(failed);
  let second = discard(busy);

  if job.retries == 3 && first + second == 2 { 42 } else { 1 }
}
