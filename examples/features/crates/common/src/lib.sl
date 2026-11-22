// Each dependent activates exactly one of `a` and `b`. Feature sets are not
// unified across edges, so this package is instantiated twice and each
// instance keeps its own symbols.
cfg(feature("a")) public func which() -> i32 {
    1
}

cfg(feature("b")) public func which() -> i32 {
    2
}
