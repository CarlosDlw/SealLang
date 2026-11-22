// The std package root is a facade over the core modules. Re-exporting the
// modules keeps `std::option::Option`, `std::result::Result`, `std::str`, and
// `std::mem::forget` as canonical aliases of the core declarations rather than
// new items.
public use core::option as option;
public use core::result as result;
public use core::str as str;
public use core::mem as mem;
