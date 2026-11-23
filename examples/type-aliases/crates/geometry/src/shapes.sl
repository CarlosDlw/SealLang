module shapes;

public struct Point {
    public x: i32,
    public y: i32,
}

public enum Maybe {
    Some(i32),
    None,
}

public type PointAlias = Point;
public type MaybeAlias = Maybe;
