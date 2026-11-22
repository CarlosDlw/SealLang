// The root file has no `module` declaration; it contributes to `root`.
// Imports are absolute and file-scoped.

use app::math::add;
use app::numbers::base;

func main() -> i32 {
    add(base(), 2)
}
