module app::math;

// `twice` lives in math_extra.sl, another fragment of `app::math`.
internal func add(left: i32, right: i32) -> i32 {
    twice(left) + right
}
