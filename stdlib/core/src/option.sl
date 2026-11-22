module option;

// A value that is either present or absent. `Option` is not `Copy`: moving it
// moves the payload, and the language has no methods or `?` operator.
public enum Option<T> {
  Some(T),
  None,
}
