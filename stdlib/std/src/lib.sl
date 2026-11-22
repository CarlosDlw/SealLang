// The std package root is a facade over the core modules. Re-exporting the
// modules keeps `std::option::Option` and `std::result::Result` as canonical
// aliases of the core declarations rather than new types.
public use core::option as option;
public use core::result as result;
