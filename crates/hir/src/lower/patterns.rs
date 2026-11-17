//! Pattern lowering.
//!
//! A pattern is first read from the AST into a private tree that keeps binding
//! names. Lowering then computes one binding set for the whole arm: every
//! alternative of an or-pattern must bind the same names, and a name may appear
//! once per path. Only after that set is known are definitions created and the
//! public, fully resolved pattern tree built.

use seal_ast::{
    PatternId as AstPatternId, PatternKind as AstPatternKind, PatternLiteral, VariantPatternFields,
};
use seal_def::{DefData, DefId, DefKind};
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
use seal_span::Span;
use seal_symbol::Symbol;
use seal_ty::VariantIdx;

use super::Lowering;
use crate::id::HirPatternId;
use crate::pattern::{Pattern, PatternKind};

/// A pattern whose binding names are not resolved yet.
struct RawPattern {
    kind: RawPatternKind,
    span: Span,
}

/// The unresolved shape of a pattern.
enum RawPatternKind {
    Wildcard,
    Binding {
        name: Symbol,
        span: Span,
    },
    Literal(PatternLiteral),
    Variant {
        adt: DefId,
        variant: VariantIdx,
        fields: Vec<(DefId, RawPattern)>,
    },
    Or(Vec<RawPattern>),
    Error,
}

/// Definitions available while one arm is lowered.
#[derive(Default)]
struct ArmBindings {
    /// Names that the arm body may use, one definition per name.
    visible: FxHashMap<Symbol, DefId>,
    /// Names that only a later alternative binds. They are errors, but the tree
    /// still needs a definition for the binding node.
    hidden: FxHashMap<Symbol, DefId>,
}

impl Lowering<'_> {
    /// Lowers one match arm pattern and installs its bindings in the current
    /// scope.
    pub(super) fn lower_arm_pattern(&mut self, pattern: AstPatternId) -> HirPatternId {
        let raw = self.build_raw_pattern(pattern);

        let mut bindings = Vec::new();
        let mut seen = FxHashSet::default();
        self.collect_pattern_bindings(&raw, &mut bindings, &mut seen);

        let mut context = ArmBindings::default();
        for (name, name_span) in bindings {
            let definition = self.hir.define(DefData {
                kind: DefKind::Binding,
                name,
                name_span,
                span: raw.span,
                parent: self.function,
                mutable: false,
            });
            context.visible.insert(name, definition);
            self.scopes.insert(name, definition);
        }

        self.instantiate_pattern(&raw, &mut context)
    }

    fn build_raw_pattern(&mut self, pattern: AstPatternId) -> RawPattern {
        let (span, kind) = {
            let pattern = self.ast.pattern(pattern);
            (pattern.span, pattern.kind.clone())
        };
        let kind = match kind {
            AstPatternKind::Wildcard => RawPatternKind::Wildcard,
            AstPatternKind::Binding(name) => RawPatternKind::Binding { name, span },
            AstPatternKind::Literal(literal) => RawPatternKind::Literal(literal),
            AstPatternKind::Variant {
                type_name,
                type_span,
                name,
                name_span,
                fields,
            } => self.build_raw_variant(type_name, type_span, name, name_span, fields),
            AstPatternKind::Or(alternatives) => RawPatternKind::Or(
                alternatives
                    .into_iter()
                    .map(|alternative| self.build_raw_pattern(alternative))
                    .collect(),
            ),
            AstPatternKind::Error => RawPatternKind::Error,
        };
        RawPattern { kind, span }
    }

    fn build_raw_variant(
        &mut self,
        type_name: Symbol,
        type_span: Span,
        name: Symbol,
        name_span: Span,
        fields: VariantPatternFields,
    ) -> RawPatternKind {
        let Some(definition) = self.types.get(&type_name).copied() else {
            let missing = self.interner.resolve(type_name);
            if self.traits.contains_key(&type_name) {
                self.error(
                    codes::E2027,
                    type_span,
                    format!("`{missing}` is a trait, not an enum"),
                )
                .help("match a value of an enum type")
                .emit();
            } else if self.globals.contains_key(&type_name) {
                self.error(
                    codes::E2015,
                    type_span,
                    format!("`{missing}` is a value, not a type"),
                )
                .help("a variant pattern starts with an enum name")
                .emit();
            } else {
                self.error(
                    codes::E2002,
                    type_span,
                    format!("cannot find type `{missing}` in this scope"),
                )
                .help("declare it with `enum`, or check the spelling")
                .emit();
            }
            self.discard_variant_fields(fields);
            return RawPatternKind::Error;
        };

        let adt = self
            .hir
            .adt(definition)
            .expect("every type definition reserves an aggregate");
        let adt_def = self.hir.ty_ctxt().adt(adt).clone();
        if !adt_def.is_enum() {
            self.error(
                codes::E2042,
                type_span,
                format!(
                    "`{}` is a struct, not an enum",
                    self.interner.resolve(type_name)
                ),
            )
            .help("a struct is matched by binding it, with `_`, or by a literal")
            .emit();
            self.discard_variant_fields(fields);
            return RawPatternKind::Error;
        }

        let Some(variant) = self.hir.variant(definition, name) else {
            let type_name = self.interner.resolve(type_name).to_owned();
            let variant_name = self.interner.resolve(name).to_owned();
            let declared: Vec<String> = adt_def
                .variants
                .iter()
                .map(|variant| format!("`{}`", self.interner.resolve(variant.name)))
                .collect();
            let help = if declared.is_empty() {
                "this enum declares no variants".to_owned()
            } else {
                format!("the enum declares {}", declared.join(", "))
            };
            self.error(
                codes::E2032,
                name_span,
                format!("the enum `{type_name}` has no variant `{variant_name}`"),
            )
            .help(help)
            .emit();
            self.discard_variant_fields(fields);
            return RawPatternKind::Error;
        };
        let variant_def = adt_def
            .variant(variant)
            .expect("a registered variant belongs to its enum")
            .clone();

        match fields {
            VariantPatternFields::Unit => {
                if variant_def.fields.is_empty() {
                    RawPatternKind::Variant {
                        adt: definition,
                        variant,
                        fields: Vec::new(),
                    }
                } else {
                    self.error(
                        codes::E2043,
                        name_span,
                        format!(
                            "the variant `{}::{}` has {} field(s), but the pattern has none",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name),
                            variant_def.fields.len()
                        ),
                    )
                    .help(if variant_def.is_tuple() {
                        format!(
                            "write the fields in parentheses, as in `{}::{}(...)`",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name)
                        )
                    } else {
                        format!(
                            "write the fields in braces, as in `{}::{} {{ ... }}`",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name)
                        )
                    })
                    .emit();
                    RawPatternKind::Error
                }
            }
            VariantPatternFields::Positional(patterns) => {
                if !variant_def.is_tuple() {
                    self.error(
                        codes::E2042,
                        name_span,
                        format!(
                            "the variant `{}::{}` does not have positional fields",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name)
                        ),
                    )
                    .help(if variant_def.is_unit() {
                        format!(
                            "write `{}::{}` without field patterns",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name)
                        )
                    } else {
                        format!(
                            "write the fields in braces, as in `{}::{} {{ ... }}`",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name)
                        )
                    })
                    .emit();
                    for pattern in patterns {
                        self.build_raw_pattern(pattern);
                    }
                    return RawPatternKind::Error;
                }
                if patterns.len() != variant_def.fields.len() {
                    self.error(
                        codes::E2043,
                        name_span,
                        format!(
                            "the variant `{}::{}` takes {} field pattern(s), but {} were supplied",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name),
                            variant_def.fields.len(),
                            patterns.len()
                        ),
                    )
                    .help("write one pattern per field, using `_` for ignored fields")
                    .emit();
                }
                let fields = variant_def
                    .fields
                    .iter()
                    .zip(patterns)
                    .map(|(field, pattern)| (field.def, self.build_raw_pattern(pattern)))
                    .collect();
                RawPatternKind::Variant {
                    adt: definition,
                    variant,
                    fields,
                }
            }
            VariantPatternFields::Named(named) => {
                if variant_def.is_tuple() {
                    self.error(
                        codes::E2042,
                        name_span,
                        format!(
                            "the variant `{}::{}` has positional fields",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name)
                        ),
                    )
                    .help(format!(
                        "write the fields in parentheses, as in `{}::{}(...)`",
                        self.interner.resolve(type_name),
                        self.interner.resolve(name)
                    ))
                    .emit();
                    for field in named {
                        self.build_raw_pattern(field.pattern);
                    }
                    return RawPatternKind::Error;
                }
                if variant_def.is_unit() {
                    self.error(
                        codes::E2042,
                        name_span,
                        format!(
                            "the unit variant `{}::{}` has no fields",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name)
                        ),
                    )
                    .help(format!(
                        "write `{}::{}` without field patterns",
                        self.interner.resolve(type_name),
                        self.interner.resolve(name)
                    ))
                    .emit();
                    for field in named {
                        self.build_raw_pattern(field.pattern);
                    }
                    return RawPatternKind::Error;
                }

                let mut matched: FxHashMap<DefId, RawPattern> = FxHashMap::default();
                for field in named {
                    let pattern = self.build_raw_pattern(field.pattern);
                    match variant_def
                        .fields
                        .iter()
                        .find(|declared| declared.name == Some(field.name))
                    {
                        Some(declared) => {
                            if matched.insert(declared.def, pattern).is_some() {
                                self.error(
                                    codes::E2044,
                                    field.name_span,
                                    format!(
                                        "the field `{}` is matched twice",
                                        self.interner.resolve(field.name)
                                    ),
                                )
                                .help("write each field exactly once")
                                .emit();
                            }
                        }
                        None => {
                            self.error(
                                codes::E2044,
                                field.name_span,
                                format!(
                                    "the variant `{}::{}` has no field `{}`",
                                    self.interner.resolve(type_name),
                                    self.interner.resolve(name),
                                    self.interner.resolve(field.name)
                                ),
                            )
                            .help("check the field name against the declaration")
                            .emit();
                        }
                    }
                }
                let missing: Vec<String> = variant_def
                    .fields
                    .iter()
                    .filter(|field| !matched.contains_key(&field.def))
                    .filter_map(|field| {
                        field
                            .name
                            .map(|name| format!("`{}`", self.interner.resolve(name)))
                    })
                    .collect();
                if !missing.is_empty() {
                    self.error(
                        codes::E2043,
                        name_span,
                        format!(
                            "the pattern for `{}::{}` is missing {}",
                            self.interner.resolve(type_name),
                            self.interner.resolve(name),
                            missing.join(", ")
                        ),
                    )
                    .help("write every field, or use `_` for the ones the arm ignores")
                    .emit();
                }
                let fields = variant_def
                    .fields
                    .iter()
                    .filter_map(|field| {
                        matched
                            .remove(&field.def)
                            .map(|pattern| (field.def, pattern))
                    })
                    .collect();
                RawPatternKind::Variant {
                    adt: definition,
                    variant,
                    fields,
                }
            }
        }
    }

    /// Lowers child patterns of a variant that could not be resolved, so their
    /// own diagnostics still appear, and discards the result.
    fn discard_variant_fields(&mut self, fields: VariantPatternFields) {
        match fields {
            VariantPatternFields::Unit => {}
            VariantPatternFields::Positional(patterns) => {
                for pattern in patterns {
                    self.build_raw_pattern(pattern);
                }
            }
            VariantPatternFields::Named(fields) => {
                for field in fields {
                    self.build_raw_pattern(field.pattern);
                }
            }
        }
    }

    /// Collects one arm's bindings, checking duplicates and or-pattern
    /// consistency.
    fn collect_pattern_bindings(
        &mut self,
        pattern: &RawPattern,
        out: &mut Vec<(Symbol, Span)>,
        seen: &mut FxHashSet<Symbol>,
    ) {
        match &pattern.kind {
            RawPatternKind::Wildcard | RawPatternKind::Literal(_) | RawPatternKind::Error => {}
            RawPatternKind::Binding { name, span } => {
                if seen.insert(*name) {
                    out.push((*name, *span));
                } else {
                    self.error(
                        codes::E2040,
                        *span,
                        format!(
                            "the name `{}` is bound twice in one pattern",
                            self.interner.resolve(*name)
                        ),
                    )
                    .help("rename one binding, or replace it with `_`")
                    .emit();
                }
            }
            RawPatternKind::Variant { fields, .. } => {
                for (_, field) in fields {
                    self.collect_pattern_bindings(field, out, seen);
                }
            }
            RawPatternKind::Or(alternatives) => {
                let mut collected = Vec::with_capacity(alternatives.len());
                for alternative in alternatives {
                    let mut names = Vec::new();
                    let mut alternative_seen = FxHashSet::default();
                    self.collect_pattern_bindings(alternative, &mut names, &mut alternative_seen);
                    collected.push(names);
                }

                let Some(first) = collected.first().cloned() else {
                    return;
                };
                let first_names: FxHashSet<Symbol> = first.iter().map(|(name, _)| *name).collect();
                let consistent = collected.iter().skip(1).all(|names| {
                    let names: FxHashSet<Symbol> = names.iter().map(|(name, _)| *name).collect();
                    names == first_names
                });
                if !consistent {
                    self.error(
                        codes::E2041,
                        pattern.span,
                        "the alternatives of this pattern bind different names",
                    )
                    .help(
                        "bind the same names in every alternative, and use `_` for ignored values",
                    )
                    .emit();
                }

                for (name, span) in first {
                    if seen.insert(name) {
                        out.push((name, span));
                    } else {
                        self.error(
                            codes::E2040,
                            span,
                            format!(
                                "the name `{name}` is bound twice in one pattern",
                                name = self.interner.resolve(name)
                            ),
                        )
                        .help("rename one binding, or replace it with `_`")
                        .emit();
                    }
                }
            }
        }
    }

    fn instantiate_pattern(
        &mut self,
        pattern: &RawPattern,
        bindings: &mut ArmBindings,
    ) -> HirPatternId {
        let kind = match &pattern.kind {
            RawPatternKind::Wildcard => PatternKind::Wildcard,
            RawPatternKind::Binding { name, span } => PatternKind::Binding {
                name: *name,
                def: self.binding_def(*name, *span, bindings),
            },
            RawPatternKind::Literal(literal) => PatternKind::Literal(*literal),
            RawPatternKind::Variant {
                adt,
                variant,
                fields,
            } => PatternKind::Variant {
                adt: *adt,
                variant: *variant,
                fields: fields
                    .iter()
                    .map(|(field, pattern)| (*field, self.instantiate_pattern(pattern, bindings)))
                    .collect(),
            },
            RawPatternKind::Or(alternatives) => PatternKind::Or(
                alternatives
                    .iter()
                    .map(|alternative| self.instantiate_pattern(alternative, bindings))
                    .collect(),
            ),
            RawPatternKind::Error => PatternKind::Error,
        };
        self.hir.push_pattern(Pattern {
            kind,
            span: pattern.span,
        })
    }

    fn binding_def(&mut self, name: Symbol, span: Span, bindings: &mut ArmBindings) -> DefId {
        if let Some(definition) = bindings.visible.get(&name) {
            return *definition;
        }
        if let Some(definition) = bindings.hidden.get(&name) {
            return *definition;
        }
        let definition = self.hir.define(DefData {
            kind: DefKind::Binding,
            name,
            name_span: span,
            span,
            parent: self.function,
            mutable: false,
        });
        bindings.hidden.insert(name, definition);
        definition
    }
}
