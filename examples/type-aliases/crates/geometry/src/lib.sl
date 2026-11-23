// A public re-export makes a dependency alias nameable from the package root.
public use root::shapes::MaybeAlias;
public use root::shapes::PointAlias;

// Aliases are transparent: every use expands to the target type.
public type Score = i32;
public type Pair<T> = (T, T);
