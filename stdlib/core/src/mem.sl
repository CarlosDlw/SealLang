module mem;

/// Consumes `value` without running its destructor.
///
/// `forget` takes ownership of the value, so a non-`Copy` argument cannot be
/// used afterward. No destructor, drop glue, allocation, or free runs for the
/// forgotten value: any resources it owns are leaked on purpose.
///
/// The operation is safe. A leaked value cannot make memory unsafe, although
/// forgetting an owner of a heap allocation leaks that allocation.
///
/// `forget` is not a `const` function. A direct call is compiled to a no-op
/// after evaluating its argument; an indirect call still consumes the argument
/// without dropping it.
public func forget<T>(value: T) {
  __forget(value);
}
