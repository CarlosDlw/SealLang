//! The Seal high-level intermediate representation.
//!
//! Lowering turns the parser's [`Ast`](seal_ast::Ast) into a tree where every
//! name is resolved and every syntactic convenience is gone: parentheses are
//! dropped, and each function owns exactly one [`Body`]. A closure owns its own
//! [`BodyKind::Closure`] body and a synthetic environment aggregate whose fields
//! are its captures.
//!
//! The result is total. A name that does not resolve, or a `break` outside a
//! loop, produces a diagnostic (`E2xxx`) and an `Error` node, so consumers can
//! keep walking without special cases.
//!
//! # Resolution
//!
//! Functions are collected before any body is lowered, so a function can call
//! one declared later. Inside a body, names resolve through a stack of ribs:
//! the innermost block wins, `let` bindings shadow parameters and outer
//! bindings, and a binding is not visible in its own initializer. Parameters and
//! bindings become [`DefId`](seal_def::DefId)s carrying their mutability, which
//! later phases use instead of re-reading the source. Inside a closure body, a
//! name from an enclosing body resolves to [`ExprKind::Capture`], so no consumer
//! ever binds an outer local through the scope stack.
//!
//! # Example
//!
//! ```ignore
//! let ast = seal_parser::parse(&lexed, session.diag(), session.interner());
//! let hir = seal_hir::lower(&ast, session.diag(), session.interner());
//! ```

#![warn(missing_docs)]

mod body;
mod builtin;
mod cfg;
mod closure;
mod export;
mod expr;
mod extern_decl;
mod generics;
mod id;
mod lower;
mod module;
mod pattern;
mod primitives;
mod res;
mod traits;

#[cfg(test)]
mod tests;

pub use body::{Block, Body, BodyKind, ForSource, ParamData, Stmt, StmtKind};
pub use builtin::protocols::{Protocol, ProtocolDefs, ProtocolMethod};
pub use cfg::CfgEnvironment;
pub use closure::{CaptureDef, CaptureSource, ClosureDef};
pub use export::Export;
pub use expr::{Expr, ExprKind, Place, Projection};
pub use extern_decl::{ExternFunction, ExternStatic, LinkDirective};
pub use generics::InherentImpl;
pub use id::{BodyId, HirBlockId, HirExprId, HirPatternId, HirStmtId};
pub use module::{
    ApiTable, ImportBinding, ImportTable, ImportTarget, ModuleData, ModuleFragment, ModulePath,
    Namespace, PackageData, PackageDependency, PackagePlan, PackagePlanDependency,
    PackagePlanEntry, PackagePlanFile, PreludeBinding, PreludeSource, PreludeTable,
};
pub use pattern::{MatchArm, Pattern, PatternKind};
pub use primitives::primitive_kind;
pub use res::Res;
pub use traits::TraitMethodLookup;

use seal_ast::Ast;
use seal_def::{DefData, DefId, DefKind, Definitions, ModuleId, PackageId, Visibility};
use seal_diagnostics::DiagCtxt;
use seal_hash::FxHashMap;
use seal_index::{Idx, IndexVec};
use seal_span::{FileId, Span};
use seal_symbol::{Interner, Symbol};
use seal_ty::{
    AdtId, CAbiProperties, GenericParamDef, Generics, ParamEnv, ParamTyId, SubstsId, TraitDef,
    TraitImplDef, TyCtxt, TyId, TyKind, VariantIdx,
};

/// How an instance method takes its receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiverKind {
    /// `self: Type` takes the value, moving or copying it.
    Value,
    /// `self: *Type` borrows the receiver for reading.
    Shared,
    /// `self: *mut Type` borrows the receiver for writing.
    Exclusive,
}

/// One source-level use of a `dyn Trait` type.
///
/// The type is recorded during lowering, but object safety can only be checked
/// once every trait definition exists, so validation runs as a final pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynTypeUse {
    /// The lowered trait-object type.
    pub ty: TyId,
    /// The source span of the complete `dyn ...` type.
    pub span: Span,
}

/// One source-level use of a C function signature.
///
/// The signature is recorded during lowering so the checker can validate every
/// parameter and the return type with the same FFI-safety predicate it uses for
/// external declarations. The span points at the complete signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FnSigUse {
    /// The lowered signature type, normally the pointee of a raw pointer.
    pub ty: TyId,
    /// The source span of the signature.
    pub span: Span,
}

/// The lowered representation of one file.
#[derive(Debug)]
pub struct Hir {
    definitions: Definitions,
    bodies: IndexVec<BodyId, Body>,
    blocks: IndexVec<HirBlockId, Block>,
    stmts: IndexVec<HirStmtId, Stmt>,
    exprs: IndexVec<HirExprId, Expr>,
    patterns: IndexVec<HirPatternId, Pattern>,
    ty_ctxt: TyCtxt,
    body_of: FxHashMap<DefId, BodyId>,
    adts: FxHashMap<DefId, AdtId>,
    variants: FxHashMap<(DefId, Symbol), VariantIdx>,
    traits: FxHashMap<Symbol, DefId>,
    builtin: Option<builtin::BuiltinTraits>,
    trait_impls: traits::TraitImpls,
    /// Inherent associated functions, in declaration order per type and name.
    ///
    /// Several impl blocks may declare the same name for disjoint instances of
    /// one generic type, so a name maps to every candidate. Choosing one needs
    /// the type arguments or the receiver, which only the caller has.
    methods: FxHashMap<(DefId, Symbol), Vec<DefId>>,
    /// Inherent functions declared by `impl str`, in declaration order per
    /// method name. `str` has no `DefId`, so its methods need a separate
    /// registry from the nominal one.
    str_methods: FxHashMap<Symbol, Vec<DefId>>,
    receivers: FxHashMap<DefId, ReceiverKind>,
    inherent_impls: Vec<InherentImpl>,
    dyn_types: Vec<DynTypeUse>,
    fn_sigs: Vec<FnSigUse>,
    extern_functions: FxHashMap<DefId, extern_decl::ExternFunction>,
    extern_function_order: Vec<DefId>,
    exports: FxHashMap<DefId, Export>,
    export_order: Vec<DefId>,
    extern_statics: FxHashMap<DefId, extern_decl::ExternStatic>,
    extern_static_order: Vec<DefId>,
    link_directives: Vec<extern_decl::LinkDirective>,
    extern_structs: Vec<AdtId>,
    closures: FxHashMap<DefId, ClosureDef>,
    closure_adt: FxHashMap<AdtId, DefId>,
    packages: IndexVec<PackageId, module::PackageData>,
    modules: IndexVec<ModuleId, module::ModuleData>,
    file_modules: FxHashMap<FileId, ModuleId>,
    module_children: FxHashMap<(ModuleId, seal_symbol::Symbol), ModuleId>,
    module_exports: FxHashMap<(ModuleId, seal_symbol::Symbol), module::ExportBinding>,
    preludes: FxHashMap<PackageId, module::PreludeTable>,
    imports: FxHashMap<FileId, module::ImportTable>,
    api: module::ApiTable,
    primary_package: PackageId,
}

impl Hir {
    /// Lowers a parsed file, reporting name-resolution errors through `dcx`.
    ///
    /// The returned tree is always complete; syntax or names that failed are
    /// represented by `Error` nodes after their diagnostic was emitted.
    pub fn lower(ast: &Ast, dcx: &DiagCtxt, interner: &Interner) -> Self {
        Self::lower_with_c_abi(ast, dcx, interner, CAbiProperties::default())
    }

    /// Lowers a parsed file whose C scalar names resolve for `c_abi`.
    ///
    /// The compiler passes the target's C ABI so `c_char`, `c_long`, and
    /// `c_wchar` have the width and signedness C uses on that target.
    pub fn lower_with_c_abi(
        ast: &Ast,
        dcx: &DiagCtxt,
        interner: &Interner,
        c_abi: CAbiProperties,
    ) -> Self {
        Self::lower_file_with_c_abi(ast, FileId::from_usize(0), dcx, interner, c_abi)
    }

    /// Lowers a parsed file registered under an explicit source-file id.
    ///
    /// A persistent session may compile several generations of one path, each
    /// with its own [`FileId`]. The id must identify the file that produced
    /// `ast`, otherwise every span in the HIR points at the wrong generation.
    pub fn lower_file_with_c_abi(
        ast: &Ast,
        file: FileId,
        dcx: &DiagCtxt,
        interner: &Interner,
        c_abi: CAbiProperties,
    ) -> Self {
        lower::lower_ast(ast, file, dcx, interner, c_abi)
    }

    /// Returns every C-exported function in declaration order.
    pub fn exports(&self) -> &[DefId] {
        &self.export_order
    }

    /// Returns the export record of `def`, if it is C-exported.
    pub fn export(&self, def: DefId) -> Option<&Export> {
        self.exports.get(&def)
    }

    /// Returns the definition table.
    pub fn definitions(&self) -> &Definitions {
        &self.definitions
    }

    /// Returns every package in the compilation.
    pub fn packages(&self) -> &IndexVec<PackageId, module::PackageData> {
        &self.packages
    }

    /// Returns the resolved prelude of `package`, if it has one.
    pub fn prelude(&self, package: PackageId) -> Option<&module::PreludeTable> {
        self.preludes.get(&package)
    }

    /// Returns every module in the compilation.
    pub fn modules(&self) -> &IndexVec<ModuleId, module::ModuleData> {
        &self.modules
    }

    /// Returns every source file that contributed to this compilation.
    ///
    /// The list is sorted by [`FileId`] so fingerprints are deterministic. A
    /// persistent session may have older generations of the same path in its
    /// source map; those files are not part of this HIR and are not returned.
    pub fn source_files(&self) -> Vec<FileId> {
        let mut files: Vec<FileId> = self.file_modules.keys().copied().collect();
        files.sort_unstable();
        files
    }

    /// Returns one module.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this compilation.
    pub fn module(&self, id: ModuleId) -> &module::ModuleData {
        &self.modules[id]
    }

    /// Returns one package.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this compilation.
    pub fn package(&self, id: PackageId) -> &module::PackageData {
        &self.packages[id]
    }

    /// Returns the module of the file `file`, if it is part of the compilation.
    pub fn module_of_file(&self, file: FileId) -> Option<ModuleId> {
        self.file_modules.get(&file).copied()
    }

    /// Returns the module that declares `def`.
    pub fn module_of_def(&self, def: DefId) -> ModuleId {
        self.definitions.module(def)
    }

    /// Returns the package that declares `def`.
    pub fn package_of_def(&self, def: DefId) -> PackageId {
        self.definitions.package(def)
    }

    /// Returns the child module `name` of `parent`.
    ///
    /// A resolved module re-export is a child for path resolution purposes.
    pub fn child_module(&self, parent: ModuleId, name: seal_symbol::Symbol) -> Option<ModuleId> {
        if let Some(module) = self.module_children.get(&(parent, name)).copied() {
            return Some(module);
        }
        let export = self.module_exports.get(&(parent, name))?;
        let module::ExportTarget::Module { module } = export.target else {
            return None;
        };
        Some(module)
    }

    /// Returns the re-export binding under `parent::name`, when any.
    pub fn module_export(
        &self,
        parent: ModuleId,
        name: seal_symbol::Symbol,
    ) -> Option<&module::ExportBinding> {
        self.module_exports.get(&(parent, name))
    }

    /// Returns every module re-export with its declaring module and name.
    pub fn module_exports(
        &self,
    ) -> impl Iterator<Item = (ModuleId, seal_symbol::Symbol, &module::ExportBinding)> + '_ {
        self.module_exports
            .iter()
            .map(|((module, name), binding)| (*module, *name, binding))
    }

    /// Returns the imports of one file.
    pub fn imports(&self, file: FileId) -> Option<&module::ImportTable> {
        self.imports.get(&file)
    }

    /// Returns the public API table.
    pub fn api(&self) -> &module::ApiTable {
        &self.api
    }

    pub(crate) fn api_mut(&mut self) -> &mut module::ApiTable {
        &mut self.api
    }

    /// Returns the package this compilation produced.
    pub fn primary_package(&self) -> PackageId {
        self.primary_package
    }

    /// Returns `true` when `def` is part of the package's public API.
    pub fn is_public_api(&self, def: DefId) -> bool {
        self.definitions.visibility(def) == Visibility::Public
            && self.modules[self.definitions.module(def)].exposed
    }

    /// Returns every top-level item declared in `module`, in source order.
    pub fn items_in_module(&self, module: ModuleId) -> Vec<DefId> {
        let mut items: Vec<DefId> = self
            .definitions
            .iter_enumerated()
            .filter(|(_, data)| data.module == module && data.parent.is_none())
            .map(|(id, _)| id)
            .collect();
        items.sort_by_key(|def| self.definitions.get(*def).span.lo());
        items
    }

    /// Returns `true` when `def` is reachable from `from`.
    ///
    /// A private item is reachable in its own module and in descendants of that
    /// module. An internal item is reachable anywhere in its package. A public
    /// item is reachable from another package only when every module on its path
    /// is exposed.
    pub fn is_visible_from(&self, def: DefId, from: ModuleId) -> bool {
        // Compiler-owned traits are part of the language surface and are
        // installed before user modules exist, so they have no exposed module
        // to navigate. Their names and protocol methods are always visible.
        if self.is_builtin_def(def) {
            return true;
        }
        let target = self.definitions.module(def);
        let target_package = self.definitions.package(def);
        let from_package = self.modules[from].package;
        if target_package == from_package {
            return match self.definitions.visibility(def) {
                Visibility::Private => self.modules[from]
                    .path
                    .starts_with(&self.modules[target].path),
                Visibility::Internal | Visibility::Public => true,
            };
        }
        self.definitions.visibility(def) == Visibility::Public && self.modules[target].exposed
    }

    /// Returns the public items of the package's API, in declaration order.
    pub fn public_items(&self) -> &[DefId] {
        self.api.items()
    }

    /// Returns the type context that owns every lowered type.
    pub fn ty_ctxt(&self) -> &TyCtxt {
        &self.ty_ctxt
    }

    /// Returns all bodies, indexable by [`BodyId`].
    pub fn bodies(&self) -> &IndexVec<BodyId, Body> {
        &self.bodies
    }

    /// Returns a body.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn body(&self, id: BodyId) -> &Body {
        &self.bodies[id]
    }

    /// Returns every lowered block, for structural fingerprints.
    #[inline]
    pub fn blocks(&self) -> &IndexVec<HirBlockId, Block> {
        &self.blocks
    }

    /// Returns every lowered statement, for structural fingerprints.
    #[inline]
    pub fn stmts(&self) -> &IndexVec<HirStmtId, Stmt> {
        &self.stmts
    }

    /// Returns every lowered expression, for structural fingerprints.
    #[inline]
    pub fn exprs(&self) -> &IndexVec<HirExprId, Expr> {
        &self.exprs
    }

    /// Returns every lowered pattern, for structural fingerprints.
    #[inline]
    pub fn patterns(&self) -> &IndexVec<HirPatternId, Pattern> {
        &self.patterns
    }

    /// Returns the ID of the body owned by `owner`, if it has one.
    pub fn body_id_of(&self, owner: DefId) -> Option<BodyId> {
        self.body_of.get(&owner).copied()
    }

    /// Returns the body owned by `owner`, if it has one.
    pub fn body_of(&self, owner: DefId) -> Option<&Body> {
        self.body_id_of(owner).map(|id| &self.bodies[id])
    }

    /// Returns whether calling `function` requires an `unsafe` block.
    ///
    /// A function with a body carries the marker on that body. A trait method
    /// without a body has no body of its own, so its declaration supplies the
    /// marker instead.
    pub fn is_unsafe_function(&self, function: DefId) -> bool {
        if self.definitions.get(function).kind == DefKind::ExternFunction {
            return true;
        }
        if let Some(body) = self.body_of(function) {
            return body.is_unsafe;
        }
        let definition = self.definitions.get(function);
        if definition.kind != DefKind::TraitMethod {
            return false;
        }
        let Some(parent) = definition.parent else {
            return false;
        };
        self.trait_definition(parent).is_some_and(|declaration| {
            declaration
                .methods
                .iter()
                .any(|method| method.def == function && method.is_unsafe)
        })
    }

    /// Returns a block.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn block(&self, id: HirBlockId) -> &Block {
        &self.blocks[id]
    }

    /// Returns a statement.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn stmt(&self, id: HirStmtId) -> &Stmt {
        &self.stmts[id]
    }

    /// Returns an expression.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn expr(&self, id: HirExprId) -> &Expr {
        &self.exprs[id]
    }

    /// Returns a pattern.
    ///
    /// # Panics
    ///
    /// Panics if `id` did not come from this tree.
    pub fn pattern(&self, id: HirPatternId) -> &Pattern {
        &self.patterns[id]
    }

    /// Returns all patterns, indexable by [`HirPatternId`].
    pub fn pattern_arena(&self) -> &IndexVec<HirPatternId, Pattern> {
        &self.patterns
    }

    /// Returns the place denoted by `expr`, when it denotes storage.
    ///
    /// A local path is a place rooted at its definition; a capture is a place
    /// rooted at its synthetic environment binding; `*reference` is the place
    /// reached by one dereference; a field access projects from its base place.
    /// Borrows, calls, and literals are not places. Assignment and update
    /// targets are expressions, so this is how a consumer asks whether the
    /// target is assignable without matching on the tree.
    pub fn place(&self, expr: HirExprId) -> Option<Place> {
        match self.exprs[expr].kind {
            ExprKind::Path {
                res: Res::Local(def) | Res::Const(def) | Res::Static(def),
                ..
            } => Some(Place::local(def)),
            ExprKind::Deref { operand } => self.place(operand).map(Place::deref),
            ExprKind::Field { base, name, .. } => self.place(base).map(|place| place.field(name)),
            ExprKind::TupleField { base, index, .. } => self
                .place(base)
                .map(|place| place.tuple_field(index as usize)),
            ExprKind::Index { base, index } => self.place(base).map(|place| place.index(index)),
            // A capture is a place rooted at the synthetic binding that names
            // its environment slot, so field and index projections work without
            // a new place root.
            ExprKind::Capture { closure, index } => self
                .closures
                .get(&closure)
                .and_then(|closure| closure.captures.get(index as usize))
                .map(|capture| Place::local(capture.binding)),
            _ => None,
        }
    }

    /// Returns the aggregate reserved for the nominal type definition `def`.
    ///
    /// Structs and enums both reserve one aggregate; use the type context to
    /// distinguish them when the distinction matters.
    pub fn adt(&self, def: DefId) -> Option<AdtId> {
        self.adts.get(&def).copied()
    }

    /// Returns the variant of the enum `type_def` named `name`.
    ///
    /// The registry is keyed by the enum definition, so a variant name is only
    /// found through its own enum. Structs have no entries.
    pub fn variant(&self, type_def: DefId, name: Symbol) -> Option<VariantIdx> {
        self.variants.get(&(type_def, name)).copied()
    }

    /// Returns the impl function named `name` attached to `type_def`.
    ///
    /// The lookup covers methods and associated functions. Use
    /// [`Hir::receiver`] to distinguish them.
    pub fn method(&self, type_def: DefId, name: Symbol) -> Option<DefId> {
        let candidates = self.methods(type_def, name);
        (candidates.len() == 1).then(|| candidates[0])
    }

    /// Returns every inherent function `name` declared for `type_def`.
    ///
    /// The candidates are in declaration order. Each one belongs to an impl
    /// block whose self type may restrict the instances where it applies.
    pub fn methods(&self, type_def: DefId, name: Symbol) -> &[DefId] {
        self.methods
            .get(&(type_def, name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the inherent `impl str` method named `name`.
    ///
    /// The lookup succeeds only when exactly one method has the name. Use
    /// [`Hir::str_methods`] when every declaration is needed, such as when
    /// reporting a duplicate or choosing among candidates.
    pub fn str_method(&self, name: Symbol) -> Option<DefId> {
        let candidates = self.str_methods(name);
        (candidates.len() == 1).then(|| candidates[0])
    }

    /// Returns every inherent `impl str` method named `name`.
    ///
    /// The candidates are in declaration order. `str` is a primitive, so no
    /// type arguments or impl generics are involved in the lookup.
    pub fn str_methods(&self, name: Symbol) -> &[DefId] {
        self.str_methods
            .get(&name)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Returns the receiver kind of `function`, or `None` when it is associated.
    pub fn receiver(&self, function: DefId) -> Option<ReceiverKind> {
        self.receivers.get(&function).copied()
    }

    /// Returns the trait declared with `name`, if any.
    pub fn trait_def(&self, name: Symbol) -> Option<DefId> {
        self.traits.get(&name).copied()
    }

    /// Returns the semantic declaration of a trait.
    pub fn trait_definition(&self, def: DefId) -> Option<TraitDef> {
        self.ty_ctxt.trait_def(def)
    }

    /// Returns the definition IDs of the compiler-provided protocols.
    ///
    /// # Panics
    ///
    /// Panics only if the HIR was constructed without installing its built-in
    /// protocols, which is an internal invariant violation.
    pub fn protocol_defs(&self) -> ProtocolDefs {
        self.builtin_traits().protocols
    }

    /// Returns the `Drop` trait declaration installed into every HIR.
    pub fn drop_trait(&self) -> DefId {
        self.builtin_traits().drop.trait_def
    }

    /// Returns the `Drop::drop` method declaration installed into every HIR.
    pub fn drop_method(&self) -> DefId {
        self.builtin_traits().drop.method
    }

    /// Returns `true` when `def` is the `Drop` trait declaration.
    pub fn is_drop_trait(&self, def: DefId) -> bool {
        self.drop_trait() == def
    }

    /// Returns the `Copy` trait declaration installed into every HIR.
    pub fn copy_trait(&self) -> DefId {
        self.builtin_traits().copy.trait_def
    }

    /// Returns `true` when `def` is the `Copy` trait declaration.
    pub fn is_copy_trait(&self, def: DefId) -> bool {
        self.copy_trait() == def
    }

    /// Returns the built-in trait declaration for `kind`.
    pub fn fn_trait(&self, kind: seal_ty::ClosureKind) -> DefId {
        self.builtin_traits().fn_traits.trait_def(kind)
    }

    /// Returns the closure kind named by `trait_def`, if it is a function
    /// trait.
    pub fn fn_trait_kind(&self, trait_def: DefId) -> Option<seal_ty::ClosureKind> {
        self.builtin.as_ref()?.fn_traits.kind(trait_def)
    }

    /// Returns the closure kind named by `name`, if it is a function trait.
    pub fn fn_trait_named(&self, name: Symbol) -> Option<seal_ty::ClosureKind> {
        seal_ty::ClosureKind::ALL
            .into_iter()
            .find(|kind| self.definitions().name(self.fn_trait(*kind)) == name)
    }

    /// Returns `true` when `def` is a function trait declaration.
    pub fn is_fn_trait(&self, def: DefId) -> bool {
        self.fn_trait_kind(def).is_some()
    }

    /// Returns the call method declared by the function trait `kind`.
    pub fn fn_trait_method(&self, kind: seal_ty::ClosureKind) -> DefId {
        self.builtin_traits().fn_traits.method(kind)
    }

    /// Returns the definition of the built-in opaque `c_void` type.
    pub fn c_void(&self) -> DefId {
        self.builtin_traits().c_void
    }

    /// Returns `true` when `def` is the built-in `c_void` type.
    pub fn is_c_void(&self, def: DefId) -> bool {
        self.c_void() == def
    }

    /// Returns every `extern struct` aggregate, in declaration order.
    pub fn extern_structs(&self) -> &[AdtId] {
        &self.extern_structs
    }

    /// Returns the semantic data of the closure `def`, if it is a closure.
    pub fn closure(&self, def: DefId) -> Option<&ClosureDef> {
        self.closures.get(&def)
    }

    /// Returns every closure in the file, keyed by its definition.
    pub fn closures(&self) -> &FxHashMap<DefId, ClosureDef> {
        &self.closures
    }

    /// Returns the closure whose environment is the aggregate `adt`.
    pub fn closure_for_adt(&self, adt: AdtId) -> Option<DefId> {
        self.closure_adt.get(&adt).copied()
    }

    /// Returns `true` when `def` is a closure expression.
    pub fn is_closure(&self, def: DefId) -> bool {
        self.definitions.kind(def) == DefKind::Closure
    }

    fn builtin_traits(&self) -> &builtin::BuiltinTraits {
        self.builtin
            .as_ref()
            .expect("every HIR installs its built-in traits")
    }

    /// Returns the trait definition for `protocol`.
    pub fn protocol_trait(&self, protocol: Protocol) -> DefId {
        self.protocol_defs().trait_def(protocol)
    }

    /// Returns the protocol owned by the trait definition `def`.
    pub fn protocol_of_trait(&self, def: DefId) -> Option<Protocol> {
        self.builtin.as_ref()?.protocols.protocol_of_trait(def)
    }

    /// Returns the protocol and method named by a method definition.
    pub fn protocol_method_of(&self, def: DefId) -> Option<(Protocol, ProtocolMethod)> {
        self.builtin.as_ref()?.protocols.protocol_method_of(def)
    }

    /// Returns `true` when `def` is a compiler-provided protocol definition or
    /// one of its generic parameters.
    pub fn is_builtin_def(&self, def: DefId) -> bool {
        if self.protocol_of_trait(def).is_some()
            || self.is_drop_trait(def)
            || self.is_copy_trait(def)
            || self.is_fn_trait(def)
            || self.is_c_void(def)
        {
            return true;
        }
        self.definitions
            .get(def)
            .parent
            .is_some_and(|parent| self.is_builtin_def(parent))
    }

    /// Returns the generic parameters declared by `owner`.
    ///
    /// An impl method reports the impl as its parent; use
    /// [`Hir::flattened_generics`] for every parameter visible from its body.
    pub fn generics(&self, owner: DefId) -> Option<Generics> {
        self.ty_ctxt.generics(owner)
    }

    /// Returns every parameter visible from `owner`, outermost first.
    pub fn flattened_generics(&self, owner: DefId) -> Vec<GenericParamDef> {
        self.ty_ctxt.flattened_generics(owner)
    }

    /// Returns the bounds visible while checking `owner`.
    pub fn param_env(&self, owner: DefId) -> ParamEnv {
        self.ty_ctxt.param_env(owner)
    }

    /// Returns every trait implementation in declaration order.
    pub fn trait_impls(&self) -> Vec<TraitImplDef> {
        self.ty_ctxt.trait_impls()
    }

    /// Returns every `dyn Trait` use recorded during lowering.
    pub fn dyn_types(&self) -> &[DynTypeUse] {
        &self.dyn_types
    }

    /// Returns every C function-signature use recorded during lowering.
    pub fn fn_sigs(&self) -> &[FnSigUse] {
        &self.fn_sigs
    }

    /// Returns the external declaration for `def`, if it is an external function.
    pub fn extern_function(&self, def: DefId) -> Option<&extern_decl::ExternFunction> {
        self.extern_functions.get(&def)
    }

    /// Returns the external declaration for `def`, if it is an external static.
    pub fn extern_static(&self, def: DefId) -> Option<&extern_decl::ExternStatic> {
        self.extern_statics.get(&def)
    }

    /// Returns every external function definition, in declaration order.
    pub fn extern_functions(&self) -> &[DefId] {
        &self.extern_function_order
    }

    /// Returns every external static definition, in declaration order.
    pub fn extern_statics(&self) -> &[DefId] {
        &self.extern_static_order
    }

    /// Returns every `link` directive, in source order.
    pub fn link_directives(&self) -> &[extern_decl::LinkDirective] {
        &self.link_directives
    }

    /// Returns `true` when `def` is an external function declaration.
    pub fn is_extern_function(&self, def: DefId) -> bool {
        self.definitions.get(def).kind == DefKind::ExternFunction
    }

    /// Returns `true` when `def` is an external static declaration.
    pub fn is_extern_static(&self, def: DefId) -> bool {
        self.definitions.get(def).kind == DefKind::ExternStatic
    }

    pub(crate) fn insert_export(&mut self, def: DefId, export: Export) {
        if self.exports.insert(def, export).is_none() {
            self.export_order.push(def);
        }
    }

    pub(crate) fn insert_extern_function(&mut self, def: DefId, data: extern_decl::ExternFunction) {
        self.extern_function_order.push(def);
        self.extern_functions.insert(def, data);
    }

    pub(crate) fn insert_extern_static(&mut self, def: DefId, data: extern_decl::ExternStatic) {
        self.extern_static_order.push(def);
        self.extern_statics.insert(def, data);
    }

    pub(crate) fn push_link_directive(&mut self, directive: extern_decl::LinkDirective) {
        self.link_directives.push(directive);
    }

    /// Returns every inherent implementation in declaration order.
    pub fn inherent_impls(&self) -> &[InherentImpl] {
        &self.inherent_impls
    }

    /// Returns the impl blocks whose self type is the nominal type `type_def`.
    ///
    /// Both inherent and trait implementations are included, in declaration
    /// order. Generic impls match their nominal head, not a particular
    /// instantiation.
    pub fn impls_for(&self, type_def: DefId) -> Vec<DefId> {
        let matches = |ty: seal_ty::TyId| match self.ty_ctxt.kind(ty) {
            TyKind::Adt { adt, .. } => self.ty_ctxt.adt(adt).def == type_def,
            _ => false,
        };
        let mut impls: Vec<DefId> = self
            .inherent_impls
            .iter()
            .filter(|implementation| matches(implementation.self_ty))
            .map(|implementation| implementation.def)
            .collect();
        impls.extend(
            self.ty_ctxt
                .trait_impls()
                .into_iter()
                .filter(|implementation| matches(implementation.trait_ref.self_ty))
                .map(|implementation| implementation.def),
        );
        impls.sort_unstable();
        impls
    }

    /// Returns the resolved type arguments attached to `expr`, if written.
    ///
    /// For a qualified associated function these belong to the aggregate; for
    /// a free function they are the function's own arguments.
    pub fn expr_args(&self, expr: HirExprId) -> Option<SubstsId> {
        match self.expr(expr).kind {
            ExprKind::Path { args, .. }
            | ExprKind::Struct { args, .. }
            | ExprKind::Variant { args, .. } => args,
            ExprKind::MethodCall { generic_args, .. } => generic_args,
            _ => None,
        }
    }

    /// Returns the explicit type arguments written on a function name.
    ///
    /// For an associated function this is the list after its name, separate
    /// from any aggregate arguments written before it. A method call stores
    /// its arguments here too.
    pub fn expr_function_args(&self, expr: HirExprId) -> Option<SubstsId> {
        match self.expr(expr).kind {
            ExprKind::Path { function_args, .. } => function_args,
            ExprKind::MethodCall { generic_args, .. } => generic_args,
            _ => None,
        }
    }

    pub(crate) fn insert_inherent_impl(&mut self, implementation: InherentImpl) {
        self.inherent_impls.push(implementation);
    }

    /// Returns the trait method `name` implemented by `type_def`.
    ///
    /// Inherent functions are not considered here: callers try [`Hir::method`]
    /// first, so an inherent method shadows a trait method with the same name.
    pub fn trait_method(&self, type_def: DefId, name: Symbol) -> TraitMethodLookup {
        self.trait_impls.lookup(type_def, name, |trait_ref, name| {
            self.trait_definition(trait_ref.trait_def)
                .and_then(|definition| definition.method(name).cloned())
                .filter(|method| method.has_body)
                .map(|method| method.def)
        })
    }

    pub(crate) fn insert_adt(&mut self, def: DefId, adt: AdtId) {
        self.adts.insert(def, adt);
    }

    pub(crate) fn push_extern_struct(&mut self, adt: AdtId) {
        self.extern_structs.push(adt);
    }

    pub(crate) fn insert_variant(&mut self, type_def: DefId, name: Symbol, variant: VariantIdx) {
        self.variants.insert((type_def, name), variant);
    }

    pub(crate) fn insert_trait_name(&mut self, name: Symbol, def: DefId) {
        self.traits.insert(name, def);
    }

    pub(crate) fn insert_trait_impl(
        &mut self,
        trait_ref: seal_ty::TraitRef,
        type_def: DefId,
        functions: FxHashMap<Symbol, DefId>,
    ) {
        self.trait_impls.insert(trait_ref, type_def, functions);
    }

    pub(crate) fn insert_method(&mut self, type_def: DefId, name: Symbol, function: DefId) {
        self.methods
            .entry((type_def, name))
            .or_default()
            .push(function);
    }

    pub(crate) fn insert_str_method(&mut self, name: Symbol, function: DefId) {
        self.str_methods.entry(name).or_default().push(function);
    }

    pub(crate) fn insert_receiver(&mut self, function: DefId, receiver: ReceiverKind) {
        self.receivers.insert(function, receiver);
    }

    pub(crate) fn insert_dyn_type(&mut self, usage: DynTypeUse) {
        self.dyn_types.push(usage);
    }

    pub(crate) fn insert_fn_sig(&mut self, usage: FnSigUse) {
        self.fn_sigs.push(usage);
    }

    pub(crate) fn insert_closure(&mut self, closure: ClosureDef) {
        self.closure_adt.insert(closure.adt, closure.def);
        self.closures.insert(closure.def, closure);
    }

    /// Appends one capture to a closure and returns its environment slot.
    ///
    /// The closure exists before its body is lowered, so the slot is available
    /// to every expression that mentions the captured value, including the
    /// `Hir::place` lookups performed while that body is still being built.
    pub(crate) fn push_closure_capture(&mut self, closure: DefId, capture: CaptureDef) -> u32 {
        let definition = self
            .closures
            .get_mut(&closure)
            .expect("captures belong to an inserted closure");
        let index = definition.captures.len() as u32;
        definition.captures.push(capture);
        index
    }

    /// Returns all blocks, indexable by [`HirBlockId`].
    pub fn block_arena(&self) -> &IndexVec<HirBlockId, Block> {
        &self.blocks
    }

    /// Returns all statements, indexable by [`HirStmtId`].
    pub fn stmt_arena(&self) -> &IndexVec<HirStmtId, Stmt> {
        &self.stmts
    }

    /// Returns all expressions, indexable by [`HirExprId`].
    pub fn expr_arena(&self) -> &IndexVec<HirExprId, Expr> {
        &self.exprs
    }

    pub(crate) fn empty(mut ty_ctxt: TyCtxt, interner: &Interner) -> Self {
        lower::intern_primitives(&mut ty_ctxt);
        let mut hir = Self {
            definitions: Definitions::new(),
            bodies: IndexVec::new(),
            blocks: IndexVec::new(),
            stmts: IndexVec::new(),
            exprs: IndexVec::new(),
            patterns: IndexVec::new(),
            ty_ctxt,
            body_of: FxHashMap::default(),
            adts: FxHashMap::default(),
            variants: FxHashMap::default(),
            traits: FxHashMap::default(),
            builtin: None,
            trait_impls: traits::TraitImpls::default(),
            methods: FxHashMap::default(),
            str_methods: FxHashMap::default(),
            receivers: FxHashMap::default(),
            inherent_impls: Vec::new(),
            dyn_types: Vec::new(),
            fn_sigs: Vec::new(),
            extern_functions: FxHashMap::default(),
            extern_function_order: Vec::new(),
            exports: FxHashMap::default(),
            export_order: Vec::new(),
            extern_statics: FxHashMap::default(),
            extern_static_order: Vec::new(),
            link_directives: Vec::new(),
            extern_structs: Vec::new(),
            closures: FxHashMap::default(),
            closure_adt: FxHashMap::default(),
            packages: IndexVec::new(),
            modules: IndexVec::new(),
            file_modules: FxHashMap::default(),
            module_children: FxHashMap::default(),
            module_exports: FxHashMap::default(),
            preludes: FxHashMap::default(),
            imports: FxHashMap::default(),
            api: module::ApiTable::default(),
            primary_package: PackageId::from_usize(0),
        };
        hir.builtin = Some(builtin::install(&mut hir, interner));
        hir
    }

    pub(crate) fn define(&mut self, data: DefData) -> DefId {
        self.definitions.push(data)
    }

    pub(crate) fn definitions_mut(&mut self) -> &mut Definitions {
        &mut self.definitions
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn install_graph(
        &mut self,
        modules: IndexVec<ModuleId, module::ModuleData>,
        packages: IndexVec<PackageId, module::PackageData>,
        file_modules: FxHashMap<FileId, ModuleId>,
        api: module::ApiTable,
        module_children: FxHashMap<(ModuleId, seal_symbol::Symbol), ModuleId>,
        module_exports: FxHashMap<(ModuleId, seal_symbol::Symbol), module::ExportBinding>,
        preludes: FxHashMap<PackageId, module::PreludeTable>,
        imports: FxHashMap<FileId, module::ImportTable>,
        primary_package: PackageId,
    ) {
        self.primary_package = primary_package;
        self.modules = modules;
        self.packages = packages;
        self.file_modules = file_modules;
        self.api = api;
        self.module_children = module_children;
        self.module_exports = module_exports;
        self.preludes = preludes;
        self.imports = imports;
    }

    pub(crate) fn push_body(&mut self, body: Body) -> BodyId {
        self.bodies.push(body)
    }

    pub(crate) fn set_body_of(&mut self, owner: DefId, body: BodyId) {
        self.body_of.insert(owner, body);
    }

    pub(crate) fn push_block(&mut self, block: Block) -> HirBlockId {
        self.blocks.push(block)
    }

    pub(crate) fn push_stmt(&mut self, stmt: Stmt) -> HirStmtId {
        self.stmts.push(stmt)
    }

    pub(crate) fn push_expr(&mut self, expr: Expr) -> HirExprId {
        self.exprs.push(expr)
    }

    pub(crate) fn push_pattern(&mut self, pattern: Pattern) -> HirPatternId {
        self.patterns.push(pattern)
    }

    pub(crate) fn ty_ctxt_mut(&mut self) -> &mut TyCtxt {
        &mut self.ty_ctxt
    }

    #[allow(dead_code)]
    pub(crate) fn generic_param_owner(&self, param: ParamTyId) -> DefId {
        self.ty_ctxt.param_data(param).owner
    }
}

/// Lowers a parsed file, reporting name-resolution errors through `dcx`.
pub fn lower(ast: &Ast, dcx: &DiagCtxt, interner: &Interner) -> Hir {
    Hir::lower(ast, dcx, interner)
}

/// Lowers a parsed file whose C scalar names resolve for `c_abi`.
pub fn lower_with_c_abi(
    ast: &Ast,
    dcx: &DiagCtxt,
    interner: &Interner,
    c_abi: CAbiProperties,
) -> Hir {
    Hir::lower_with_c_abi(ast, dcx, interner, c_abi)
}

/// Lowers every file of a package plan into one HIR.
///
/// The plan names the packages, their files, their dependencies, and the
/// modules their public API exposes. Each file must name a module whose path is
/// either the package root or a source-declared path.
pub fn lower_package(
    plan: &PackagePlan,
    asts: &[(FileId, Ast)],
    dcx: &DiagCtxt,
    interner: &Interner,
    c_abi: CAbiProperties,
) -> Hir {
    lower::lower_package(plan, asts, dcx, interner, c_abi)
}
