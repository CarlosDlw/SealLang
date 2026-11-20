//! Cross-package implementation rules.

use crate::tests::support::{Fixture, PackageSpec};

fn geometry() -> PackageSpec {
    PackageSpec {
        name: "geometry",
        files: &[(
            "geometry.sl",
            "public struct Point { public x: i32 }\n\
             public trait Marker {}",
        )],
        exposed: &[],
        dependencies: &[],
    }
}

#[test]
fn an_inherent_impl_for_a_foreign_type_is_an_orphan() {
    let fixture = Fixture::from_packages(&[
        geometry(),
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "use geometry::Point;\n\
                 impl geometry::Point { func get(self: *Point) -> i32 { (*self).x } }\n\
                 func main() {}",
            )],
            exposed: &[],
            dependencies: &[("geometry", 0)],
        },
    ]);
    let _ = fixture.lower_packages();
    fixture.expect("E2082", "this implementation is an orphan");
}

#[test]
fn a_trait_impl_for_a_foreign_type_and_trait_is_an_orphan() {
    let fixture = Fixture::from_packages(&[
        geometry(),
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "use geometry::{Marker, Point};\n\
                 impl Marker for geometry::Point {}\n\
                 func main() {}",
            )],
            exposed: &[],
            dependencies: &[("geometry", 0)],
        },
    ]);
    let _ = fixture.lower_packages();
    fixture.expect("E2082", "neither the type nor the trait");
}

#[test]
fn a_foreign_trait_may_be_implemented_for_a_local_type() {
    let fixture = Fixture::from_packages(&[
        geometry(),
        PackageSpec {
            name: "app",
            files: &[(
                "main.sl",
                "use geometry::Marker;\n\
                 struct Local {}\n\
                 impl Marker for Local {}\n\
                 func main() {}",
            )],
            exposed: &[],
            dependencies: &[("geometry", 0)],
        },
    ]);
    let _ = fixture.lower_packages();
    fixture.assert_clean();
}
