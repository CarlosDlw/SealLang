// A freestanding target has no runtime startup: `_start` is the entry point.
// The host test reaches `exit` through libc only to observe the exit status.
extern "c" {
    func exit(code: c_int) -> ();
}

func _start() {
    unsafe {
        exit(42);
    }
}
