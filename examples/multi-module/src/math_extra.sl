module app::math;

// Both files declare the same module; their items are merged.
internal func twice(value: i32) -> i32 {
    value + value
}
