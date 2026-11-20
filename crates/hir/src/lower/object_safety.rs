//! Post-lowering validation of `dyn Trait` uses.
//!
//! Trait definitions are only complete after every declaration has been
//! lowered, so object safety is checked in one final pass over the recorded
//! trait-object uses. Results are cached per trait definition; a diamond in the
//! supertrait graph therefore contributes each violation exactly once.

use seal_ast::{ItemKind, TypeId as AstTypeId, TypeKind as AstTypeKind};
use seal_def::DefId;
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
use seal_span::Span;
use seal_ty::TyKind;

use super::Lowering;
use crate::ReceiverKind;

/// The kind of a dyn-compatibility violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ViolationKind {
    /// `self: Self` consumes the receiver by value.
    ValueReceiver,
    /// An associated function has no receiver to dispatch through.
    AssociatedFunction,
    /// A method declares type parameters of its own.
    GenericMethod,
    /// `Self` appears in a parameter or the return type.
    SelfOutsideReceiver,
    /// A supertrait argument mentions the declaring trait's `Self`.
    SupertraitSelfArgument,
}

/// The identity of one violation, independent of the path that reached it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ViolationKey {
    owner: DefId,
    kind: ViolationKind,
    slot: u32,
}

/// One deduplicated note attached to the `E2065` diagnostic.
#[derive(Clone, Debug)]
struct Violation {
    key: ViolationKey,
    span: Span,
    message: String,
}

impl Lowering<'_> {
    /// Reports one `E2065` per unsafe `dyn` use, with one note per violation.
    pub(super) fn validate_dyn_types(&mut self) {
        let uses = self.hir.dyn_types().to_vec();
        let mut cache = FxHashMap::default();
        for usage in uses {
            let TyKind::Dyn { trait_def, .. } = self.hir.ty_ctxt().kind(usage.ty) else {
                continue;
            };
            if let Some(kind) = self.hir.fn_trait_kind(trait_def) {
                self.error(
                    codes::E3096,
                    usage.span,
                    format!(
                        "the function trait `{}` cannot be used as a trait object",
                        kind.as_str()
                    ),
                )
                .help("use `F: Fn(...)` as a generic bound, or call a `func(...)` pointer")
                .emit();
                continue;
            }
            if self.hir.is_copy_trait(trait_def) {
                self.error(
                    codes::E3078,
                    usage.span,
                    "the built-in `Copy` marker cannot be used as a trait object",
                )
                .help("`Copy` is a marker bound for sized value types, not a trait object")
                .emit();
                continue;
            }
            if self.hir.is_drop_trait(trait_def) {
                self.error(
                    codes::E3066,
                    usage.span,
                    "the trait `Drop` cannot be used as a trait object",
                )
                .help(
                    "implement `Drop` for the concrete type; destructors are not dispatched through `dyn`",
                )
                .emit();
                continue;
            }
            let mut visiting = FxHashSet::default();
            let violations = self.object_safety(trait_def, &mut cache, &mut visiting);
            if violations.is_empty() {
                continue;
            }
            let name = self
                .interner
                .resolve(self.hir.definitions().name(trait_def))
                .to_owned();
            let mut diagnostic = self.error(
                codes::E2065,
                usage.span,
                format!("the trait `{name}` cannot be used as a trait object"),
            );
            for violation in violations {
                diagnostic = diagnostic.secondary(violation.span, violation.message);
            }
            diagnostic
                .help("use the trait as a bound for static dispatch, or make every method dispatchable")
                .emit();
        }
    }

    /// Returns every violation of `trait_def` and its reachable supertraits.
    fn object_safety(
        &mut self,
        trait_def: DefId,
        cache: &mut FxHashMap<DefId, Vec<Violation>>,
        visiting: &mut FxHashSet<DefId>,
    ) -> Vec<Violation> {
        if let Some(cached) = cache.get(&trait_def) {
            return cached.clone();
        }
        if !visiting.insert(trait_def) {
            return Vec::new();
        }

        let mut violations = Vec::new();
        let mut seen = FxHashSet::default();
        self.direct_violations(trait_def, &mut violations, &mut seen);
        if let Some(definition) = self.hir.trait_definition(trait_def) {
            for supertrait in &definition.supertraits {
                for violation in self.object_safety(supertrait.trait_def, cache, visiting) {
                    push_violation(&mut violations, &mut seen, violation);
                }
            }
        }

        visiting.remove(&trait_def);
        cache.insert(trait_def, violations.clone());
        violations
    }

    /// Collects the violations declared directly by one trait.
    fn direct_violations(
        &self,
        trait_def: DefId,
        violations: &mut Vec<Violation>,
        seen: &mut FxHashSet<ViolationKey>,
    ) {
        let Some(definition) = self.hir.trait_definition(trait_def) else {
            return;
        };
        let self_param = definition
            .generics
            .params
            .iter()
            .find(|parameter| parameter.is_self)
            .map(|parameter| parameter.param);
        let tcx = self.hir.ty_ctxt();

        for method in &definition.methods {
            let span = self.hir.definitions().get(method.def).name_span;
            let name = self.interner.resolve(method.name).to_owned();
            let violation = match self.trait_methods.get(&(trait_def, method.name)) {
                Some(raw) => match raw.receiver {
                    Some(ReceiverKind::Value) => Some(ViolationKind::ValueReceiver),
                    Some(ReceiverKind::Shared | ReceiverKind::Exclusive) => None,
                    // An invalid receiver was already reported by signature
                    // lowering; do not turn that error into an object-safety
                    // error.
                    None if raw.has_self_parameter => None,
                    None => Some(ViolationKind::AssociatedFunction),
                },
                // Built-in protocol traits have no source declaration in
                // `trait_methods`; their lowered receiver is authoritative.
                None => match method.receiver {
                    Some(receiver) if receiver.reference.is_none() => {
                        Some(ViolationKind::ValueReceiver)
                    }
                    Some(_) => None,
                    None => Some(ViolationKind::AssociatedFunction),
                },
            };
            match violation {
                Some(ViolationKind::ValueReceiver) => push_violation(
                    violations,
                    seen,
                    method_violation(
                        method.def,
                        ViolationKind::ValueReceiver,
                        span,
                        format!(
                            "method `{name}` takes `self` by value; dynamic dispatch requires `*Self` or `*mut Self`"
                        ),
                    ),
                ),
                Some(ViolationKind::AssociatedFunction) => push_violation(
                    violations,
                    seen,
                    method_violation(
                        method.def,
                        ViolationKind::AssociatedFunction,
                        span,
                        format!(
                            "associated function `{name}` has no receiver and cannot be dispatched"
                        ),
                    ),
                ),
                Some(
                    ViolationKind::GenericMethod
                    | ViolationKind::SelfOutsideReceiver
                    | ViolationKind::SupertraitSelfArgument,
                )
                | None => {}
            }
            if !method.generics.params.is_empty() {
                push_violation(
                    violations,
                    seen,
                    method_violation(
                        method.def,
                        ViolationKind::GenericMethod,
                        span,
                        format!("method `{name}` declares its own type parameters"),
                    ),
                );
            }
            if let Some(self_param) = self_param
                && method
                    .signature
                    .params
                    .iter()
                    .copied()
                    .chain(std::iter::once(method.signature.ret))
                    .any(|ty| tcx.free_params(ty).contains(&self_param))
            {
                push_violation(
                    violations,
                    seen,
                    method_violation(
                        method.def,
                        ViolationKind::SelfOutsideReceiver,
                        span,
                        format!("method `{name}` mentions `Self` outside its receiver"),
                    ),
                );
            }
        }

        self.supertrait_argument_violations(trait_def, violations, seen);
    }

    /// Reports supertrait arguments written in terms of the trait's `Self`.
    fn supertrait_argument_violations(
        &self,
        trait_def: DefId,
        violations: &mut Vec<Violation>,
        seen: &mut FxHashSet<ViolationKey>,
    ) {
        let Some((item, _)) = self
            .trait_items
            .iter()
            .find(|(_, definition)| *definition == trait_def)
        else {
            return;
        };
        let ItemKind::Trait(declaration) = &self.ast_of(*item).item(item.id).kind else {
            return;
        };
        for (slot, path) in declaration.supertraits.iter().enumerate() {
            let mentions_self = path.args.as_ref().is_some_and(|arguments| {
                arguments
                    .args
                    .iter()
                    .any(|argument| self.ast_type_mentions_self(*argument))
            });
            if !mentions_self {
                continue;
            }
            let name = self.interner.resolve(path.name).to_owned();
            push_violation(
                violations,
                seen,
                Violation {
                    key: ViolationKey {
                        owner: trait_def,
                        kind: ViolationKind::SupertraitSelfArgument,
                        slot: slot as u32,
                    },
                    span: path.span,
                    message: format!("the supertrait `{name}` mentions `Self` in its arguments"),
                },
            );
        }
    }

    /// Returns `true` when an AST type mentions `Self` at any depth.
    fn ast_type_mentions_self(&self, ty: AstTypeId) -> bool {
        match self.ast.ty(ty).kind.clone() {
            AstTypeKind::Path(path) if path.name == self.self_symbol => true,
            AstTypeKind::Path(path) => path.args.as_ref().is_some_and(|arguments| {
                arguments
                    .args
                    .iter()
                    .any(|argument| self.ast_type_mentions_self(*argument))
            }),
            AstTypeKind::Dyn(path) => path.args.as_ref().is_some_and(|arguments| {
                arguments
                    .args
                    .iter()
                    .any(|argument| self.ast_type_mentions_self(*argument))
            }),
            AstTypeKind::Pointer { pointee, .. } | AstTypeKind::RawPointer { pointee, .. } => {
                self.ast_type_mentions_self(pointee)
            }
            AstTypeKind::Array { element, .. } | AstTypeKind::Slice { element } => {
                self.ast_type_mentions_self(element)
            }
            AstTypeKind::Tuple(elements) => elements
                .iter()
                .any(|element| self.ast_type_mentions_self(*element)),
            _ => false,
        }
    }
}

fn method_violation(method: DefId, kind: ViolationKind, span: Span, message: String) -> Violation {
    Violation {
        key: ViolationKey {
            owner: method,
            kind,
            slot: 0,
        },
        span,
        message,
    }
}

fn push_violation(
    violations: &mut Vec<Violation>,
    seen: &mut FxHashSet<ViolationKey>,
    violation: Violation,
) {
    if seen.insert(violation.key) {
        violations.push(violation);
    }
}
