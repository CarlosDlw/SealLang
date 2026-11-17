// A program that exercises generic functions, aggregates, enums, methods,
// trait bounds, recursion through a pointer, and a fat pointer field.
// The documented result is 39.

enum Option<T> {
    Some(T),
    None,
}

struct Pair<T, U> {
    first: T,
    second: U,
}

struct Node<T> {
    value: T,
    next: Option<*mut Node<T>>,
}

struct Matrix<T> {
    cells: [T; 4],
    pair: (T, T),
}

struct Box<T> {
    value: T,
}

struct Labeled<T> {
    label: *str,
    value: T,
}

trait Show {
    func show(self: *Self) -> i32;
}

impl Show for Pair<i32, bool> {
    func show(self: *Pair<i32, bool>) -> i32 {
        (*self).first
    }
}

impl<T> Box<T> {
    func new(value: T) -> Box<T> {
        Box { value: value }
    }

    func get(self: *Box<T>) -> T {
        (*self).value
    }
}

func identity<T>(value: T) -> T {
    value
}

func describe<T: Show>(value: *T) -> i32 {
    value.show()
}

func sum_nodes(node: *mut Node<i32>) -> i32 {
    let mut total = (*node).value;
    match (*node).next {
        Option::Some(next) => {
            total = total + sum_nodes(next);
        }
        Option::None => {}
    }
    total
}

func next(counter: *mut i32) -> i32 {
    *counter = *counter + 1;
    *counter
}

func main() -> i32 {
    let mut second = Node { value: 2, next: Option::None };
    let mut first = Node { value: 3, next: Option::Some(&mut second) };
    let total = sum_nodes(&mut first);

    let some = Option::Some(4);
    let extra = match some {
        Option::Some(value) => value,
        Option::None => 0,
    };

    let matrix = Matrix { cells: [1, 2, 3, 4], pair: (5, 6) };
    let nested = Pair { first: Pair { first: 1, second: true }, second: 2 };
    let shown = describe(&nested.first);

    let mut boxed = Box::new(7);
    let boxed_value = boxed.get();

    let labeled = Labeled { label: "seal", value: 2 };
    let labeled_value = labeled.value;

    let mut counter = 0;
    let order = next(&mut counter) * 10 + identity(next(&mut counter));

    let flag = identity(false);
    let flag_extra = if flag { 1 } else { 1 };

    total + extra + matrix.cells[0] + matrix.pair.1 + shown + boxed_value + labeled_value
        + order + flag_extra
}
