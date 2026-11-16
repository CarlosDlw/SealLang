//! Compilation and behavior tests for `#[derive(StableHash)]`.

use seal_hash::{StableHash, StableHasher, stable_hash_of};
use seal_macros::StableHash;

#[derive(StableHash)]
struct Pair {
    left: u32,
    right: u32,
}

#[derive(StableHash)]
struct Tuple(u8, bool);

#[derive(StableHash)]
struct Unit;

#[derive(StableHash)]
enum Shape {
    Empty,
    Circle { radius: u32 },
    Rect(u32, u32),
}

#[derive(StableHash)]
struct Wrapper<T> {
    value: T,
}

fn manual_pair(value: &Pair) -> u128 {
    let mut hasher = StableHasher::new();
    value.left.stable_hash(&mut hasher);
    value.right.stable_hash(&mut hasher);
    hasher.finish().as_u128()
}

#[test]
fn named_struct_matches_manual_order() {
    let pair = Pair { left: 1, right: 2 };
    assert_eq!(stable_hash_of(&pair).as_u128(), manual_pair(&pair));

    let swapped = Pair { left: 2, right: 1 };
    assert_ne!(stable_hash_of(&pair), stable_hash_of(&swapped));
}

#[test]
fn tuple_struct_hashes_fields_in_order() {
    let value = Tuple(7, true);
    let mut hasher = StableHasher::new();
    7u8.stable_hash(&mut hasher);
    true.stable_hash(&mut hasher);
    assert_eq!(stable_hash_of(&value), hasher.finish());
}

#[test]
fn unit_struct_is_hashable() {
    let _ = stable_hash_of(&Unit);
}

#[test]
fn enum_variants_are_distinct() {
    let empty = stable_hash_of(&Shape::Empty);
    let circle = stable_hash_of(&Shape::Circle { radius: 0 });
    let rect = stable_hash_of(&Shape::Rect(0, 0));
    assert_ne!(empty, circle);
    assert_ne!(empty, rect);
    assert_ne!(circle, rect);
}

#[test]
fn enum_payload_changes_hash() {
    let a = stable_hash_of(&Shape::Circle { radius: 1 });
    let b = stable_hash_of(&Shape::Circle { radius: 2 });
    assert_ne!(a, b);
}

#[test]
fn generic_struct_hashes_content() {
    let a: Wrapper<u32> = Wrapper { value: 9 };
    let b: Wrapper<u32> = Wrapper { value: 10 };
    assert_ne!(stable_hash_of(&a), stable_hash_of(&b));
}
