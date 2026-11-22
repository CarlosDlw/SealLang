// `panic(message)` reports the message with the call's source location and
// terminates the process with status 101. The message is a `*str` value, so a
// binding can be used instead of a literal. No code after the call runs.
// The documented result is the panic status 101.

func main() -> i32 {
  let reason: *str = "invalid configuration";
  panic(reason)
}
