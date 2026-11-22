module result;

// A success value or an error value. `Result` is not `Copy` and exposes no
// methods; callers use qualified variants and `match`.
public enum Result<T, E> {
  Ok(T),
  Err(E),
}
