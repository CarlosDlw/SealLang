//! Compiler-backed standard-library functions.
//!
//! A few standard-library functions cannot be ordinary Seal functions because
//! their body needs an operation that has no source syntax: `mem::forget` must
//! consume a value without running its destructor. The compiler recognizes the
//! canonical declaration by package, module, and name, and lowers calls to the
//! corresponding HIR node. The public function still exists, so function values
//! and explicit type arguments keep a normal body to call.
//!
//! The classifier lives here so future compiler-backed functions extend one
//! enum instead of scattering string comparisons across lowering, checking, and
//! code generation.

use seal_def::{DefId, DefKind};
use seal_symbol::Interner;

use crate::Hir;

/// A standard-library function whose behavior is implemented by the compiler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinFunction {
    /// `core::mem::forget`: consume a value without running its destructor.
    Forget,
}

impl Hir {
    /// Returns the compiler-backed function named by `def`, if any.
    ///
    /// Only the canonical `core` declaration is recognized. A user function
    /// named `forget`, or a module named `mem` in another package, is an
    /// ordinary function. Project validation reserves the package name `core`
    /// for the standard library, so a user package cannot impersonate it.
    pub fn builtin_function(&self, def: DefId, interner: &Interner) -> Option<BuiltinFunction> {
        let definition = self.definitions().get(def);
        if definition.kind != DefKind::Function {
            return None;
        }
        if interner.resolve(self.package(definition.package).name) != "core"
            || interner.resolve(definition.name) != "forget"
        {
            return None;
        }
        let module = self.module(definition.module);
        let [segment] = module.path.segments() else {
            return None;
        };
        if interner.resolve(*segment) != "mem" {
            return None;
        }
        Some(BuiltinFunction::Forget)
    }
}
