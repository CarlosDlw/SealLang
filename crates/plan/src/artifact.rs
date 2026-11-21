//! Platform artifact names shared by the project, build, and driver crates.
//!
//! Keeping the suffixes in one place prevents a library produced by the build
//! graph from disagreeing with the path the CLI prints or the plan records.

/// Returns the suffix of a static C library.
pub const fn static_library_suffix() -> &'static str {
    if cfg!(windows) { ".lib" } else { ".a" }
}

/// Returns the suffix of a shared C library.
pub const fn shared_library_suffix() -> &'static str {
    if cfg!(windows) {
        ".dll"
    } else if cfg!(target_os = "macos") {
        ".dylib"
    } else {
        ".so"
    }
}

/// Returns the file name of a static C library for `name`.
pub fn static_library_name(name: &str) -> String {
    format!("lib{name}{}", static_library_suffix())
}

/// Returns the file name of a shared C library for `name`.
///
/// On Windows the conventional DLL name has no `lib` prefix.
pub fn shared_library_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}{}", shared_library_suffix())
    } else {
        format!("lib{name}{}", shared_library_suffix())
    }
}

#[cfg(test)]
mod tests {
    use super::{shared_library_name, static_library_name};

    #[test]
    fn library_names_follow_the_host_convention() {
        assert!(static_library_name("math").starts_with("libmath."));
        if cfg!(windows) {
            assert_eq!(shared_library_name("math"), "math.dll");
        } else {
            assert!(shared_library_name("math").starts_with("libmath."));
        }
    }
}
