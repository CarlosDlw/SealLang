//! Alpha contains the shared numeric helpers.
//!
//! The module is public and forms the base of the workspace graph.

/// Returns the neutral value.
public func one() -> i32 {
    1
}

/// A two-dimensional point.
public struct Point {
    /// The horizontal coordinate.
    public x: i32,
    /// The vertical coordinate.
    public y: i32,
}
