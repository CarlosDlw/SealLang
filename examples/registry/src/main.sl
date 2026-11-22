// `foo` is resolved, checksum-verified, and vendored by the project layer.
use foo::answer;

func main() -> i32 {
    answer()
}
