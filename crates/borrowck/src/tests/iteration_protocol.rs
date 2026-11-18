//! Borrow checking through custom `IntoIterator` and `Iterator` loops.

use super::support::{check_clean, check_errors};

const CHAIN: &str = "\
struct Source { value: i32 } \
struct Cursor { value: i32 } \
impl Iterator<i32> for Cursor { \
    func has_next(self: *Cursor) -> bool { (*self).value < 3 } \
    func next(self: *mut Cursor) -> i32 { \
        let value = (*self).value; \
        (*self).value += 1; \
        value \
    } \
} \
impl IntoIterator<Cursor, i32> for Source { \
    func iter(self: Source) -> Cursor { Cursor { value: self.value } } \
}";

const BORROWED: &str = "\
struct Source { value: i32 } \
struct Cursor { value: i32 } \
impl Iterator<i32> for Cursor { \
    func has_next(self: *Cursor) -> bool { (*self).value < 3 } \
    func next(self: *mut Cursor) -> i32 { (*self).value } \
} \
impl IntoIterator<Cursor, i32> for *Source { \
    func iter(self: *Source) -> Cursor { Cursor { value: (*self).value } } \
}";

const EXCLUSIVE: &str = "\
struct Source { value: i32 } \
struct Cursor { source: *mut Source } \
impl Iterator<i32> for Cursor { \
    func has_next(self: *Cursor) -> bool { (*(*self).source).value < 3 } \
    func next(self: *mut Cursor) -> i32 { \
        let value = (*(*self).source).value; \
        (*(*self).source).value += 1; \
        value \
    } \
} \
impl IntoIterator<Cursor, i32> for *mut Source { \
    func iter(self: *mut Source) -> Cursor { Cursor { source: self } } \
}";

#[test]
fn a_custom_loop_is_clean() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let source = Source {{ value: 0 }}; \
           let mut total = 0; \
           for value in source {{ total += value; }} \
           total \
         }}"
    );
    check_clean(&source);
}

#[test]
fn moving_a_source_into_iter_ends_its_use() {
    let source = format!(
        "{CHAIN} func main() -> i32 {{ \
           let source = Source {{ value: 0 }}; \
           for value in source {{ }} \
           source.value \
         }}"
    );
    let fixture = check_errors(&source);
    fixture.expect("E5005", "moved");
}

#[test]
fn a_borrowed_source_remains_usable_after_the_loop() {
    let source = format!(
        "{BORROWED} func main() -> i32 {{ \
           let source = Source {{ value: 0 }}; \
           for value in &source {{ let _ = value; }} \
           source.value \
         }}"
    );
    check_clean(&source);
}

#[test]
fn the_iterators_exclusive_source_loan_conflicts_with_body_access() {
    let source = format!(
        "{EXCLUSIVE} func main() {{ \
           let mut source = Source {{ value: 0 }}; \
           for value in &mut source {{ \
               let _ = value; \
               source.value += 1; \
           }} \
         }}"
    );
    let fixture = check_errors(&source);
    assert!(
        fixture.diagnostics().iter().any(|diagnostic| matches!(
            diagnostic.code().map(|code| code.as_str()),
            Some("E5007" | "E5008")
        )),
        "the body access must conflict with the iterator's source loan: {:?}",
        fixture
            .diagnostics()
            .iter()
            .map(|diagnostic| (diagnostic.code(), diagnostic.message()))
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_shared_source_loan_allows_reads_in_the_body() {
    let source = "\
        struct Source { value: i32 } \
        struct Cursor { source: *Source } \
        impl Iterator<i32> for Cursor { \
            func has_next(self: *Cursor) -> bool { (*(*self).source).value < 3 } \
            func next(self: *mut Cursor) -> i32 { (*(*self).source).value } \
        } \
        impl IntoIterator<Cursor, i32> for *Source { \
            func iter(self: *Source) -> Cursor { Cursor { source: self } } \
        } \
        func main() { \
            let source = Source { value: 0 }; \
            for value in &source { \
                let _ = value; \
                let read = source.value; \
                let _ = read; \
            } \
        }";
    check_clean(source);
}

#[test]
fn break_and_continue_do_not_leave_iterator_loans_live() {
    let source = format!(
        "{EXCLUSIVE} func main() {{ \
           let mut source = Source {{ value: 0 }}; \
           for value in &mut source {{ \
               if value == 1 {{ continue; }} \
               if value == 2 {{ break; }} \
           }} \
           source.value += 1; \
         }}"
    );
    check_clean(&source);
}
