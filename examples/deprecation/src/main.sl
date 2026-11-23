deprecated(
    since: "0.1.0",
    note: "the current implementation replaces it",
    replacement: "current",
)
func old() -> i32 {
    40
}

func current() -> i32 {
    42
}

func main() -> i32 {
    old() + 2
}
