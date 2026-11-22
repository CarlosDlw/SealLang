module prelude;

// Names in this module are implicitly in scope in packages that select the
// core prelude. Variants stay qualified, for example `Option::Some`.
public use option::Option;
public use result::Result;
