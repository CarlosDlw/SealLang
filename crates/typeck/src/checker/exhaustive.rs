//! Exhaustiveness and reachability of `match` arms.
//!
//! The algorithm is the classic usefulness check: an arm is reachable when its
//! pattern can match a value that no earlier unguarded pattern matches. The same
//! machinery, asked about a wildcard, produces a witness for a non-exhaustive
//! match. Guards never contribute coverage, because a guard can fail.

use seal_ast::PatternLiteral;
use seal_error_codes as codes;
use seal_hir::{HirPatternId, MatchArm, PatternKind};
use seal_span::Span;
use seal_ty::{AdtId, FloatTy, TyCtxt, TyId, TyKind, VariantIdx};

use super::Checker;

/// Upper bound on the cartesian product of nested or-patterns.
///
/// A pattern is a finite tree, but or-patterns in several nested fields multiply.
/// Normal programs are far below this bound; the check exists so hostile input
/// cannot exhaust memory. Exceeding it is reported as a diagnostic, never as a
/// silent wrong answer.
const MAX_ALTERNATIVES: usize = 65_536;

/// A pattern shape used by the usefulness algorithm.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Cover {
    Wildcard,
    Bool(bool),
    Int {
        negative: bool,
        value: u128,
    },
    Char(char),
    Float(u64),
    Variant {
        adt: AdtId,
        variant: VariantIdx,
        fields: Vec<Cover>,
    },
}

/// A constructor of a type, used to expand wildcards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctor {
    Bool(bool),
    Int { negative: bool, value: u128 },
    Char(char),
    Float(u64),
    Variant { adt: AdtId, variant: VariantIdx },
}

/// Tracks the or-pattern expansion budget.
struct Budget {
    remaining: usize,
    exhausted: bool,
}

impl Checker<'_> {
    /// Reports unreachable arms and a non-exhaustive match. Returns `true` when
    /// every value of `scrutinee_ty` is covered by an unguarded arm.
    pub(super) fn check_match_coverage(
        &self,
        scrutinee_ty: TyId,
        arms: &[MatchArm],
        span: Span,
    ) -> bool {
        let mut covered: Vec<Vec<Cover>> = Vec::new();

        for arm in arms {
            let mut budget = Budget {
                remaining: MAX_ALTERNATIVES,
                exhausted: false,
            };
            let alternatives = self.cover_pattern(arm.pattern, &mut budget);
            if budget.exhausted {
                self.error(
                    codes::E3032,
                    arm.span,
                    "this pattern has too many alternative combinations to check exhaustiveness",
                )
                .help("split the match into smaller matches")
                .emit();
                return true;
            }
            if alternatives.is_empty() {
                continue;
            }

            let reachable = alternatives.iter().any(|alternative| {
                is_useful(
                    &covered,
                    std::slice::from_ref(alternative),
                    &[scrutinee_ty],
                    self.hir.ty_ctxt(),
                )
            });
            if !reachable {
                self.error(codes::E3033, arm.span, "this match arm is unreachable")
                    .help("remove the arm, or move it before the arm that covers it")
                    .emit();
            }

            if arm.guard.is_none() {
                for alternative in alternatives {
                    covered.push(vec![alternative]);
                }
            }
        }

        let tcx = self.hir.ty_ctxt();
        let Some(witness) = witness(&covered, &[scrutinee_ty], tcx) else {
            return true;
        };
        if witness.is_empty() {
            return false;
        }

        let missing = witness
            .first()
            .map(|cover| self.cover_text(cover))
            .unwrap_or_else(|| "_".to_owned());
        self.error(
            codes::E3032,
            span,
            format!("non-exhaustive match: the value `{missing}` is not covered"),
        )
        .help("add an arm for it, or finish with `_ => ...`")
        .emit();
        false
    }

    /// Expands a pattern into constructor shapes, distributing or-patterns.
    fn cover_pattern(&self, pattern: HirPatternId, budget: &mut Budget) -> Vec<Cover> {
        if budget.remaining == 0 {
            budget.exhausted = true;
            return Vec::new();
        }
        budget.remaining -= 1;

        match &self.hir.pattern(pattern).kind {
            PatternKind::Wildcard | PatternKind::Binding { .. } | PatternKind::Error => {
                vec![Cover::Wildcard]
            }
            PatternKind::Literal(literal) => vec![match *literal {
                PatternLiteral::Int {
                    value, negative, ..
                } => Cover::Int { negative, value },
                PatternLiteral::Float { value, .. } => Cover::Float(value.to_bits()),
                PatternLiteral::Char(value) => Cover::Char(value),
                PatternLiteral::Bool(value) => Cover::Bool(value),
            }],
            PatternKind::Variant {
                adt,
                variant,
                fields,
            } => {
                let Some(adt_id) = self.hir.adt(*adt) else {
                    return vec![Cover::Wildcard];
                };
                let mut combinations: Vec<Vec<Cover>> = vec![Vec::new()];
                for (_, field) in fields {
                    let options = self.cover_pattern(*field, budget);
                    if budget.exhausted || options.is_empty() {
                        return Vec::new();
                    }
                    let mut next = Vec::new();
                    for prefix in &combinations {
                        for option in &options {
                            if budget.remaining == 0 {
                                budget.exhausted = true;
                                return Vec::new();
                            }
                            budget.remaining -= 1;
                            let mut combination = prefix.clone();
                            combination.push(option.clone());
                            next.push(combination);
                        }
                    }
                    combinations = next;
                }
                combinations
                    .into_iter()
                    .map(|fields| Cover::Variant {
                        adt: adt_id,
                        variant: *variant,
                        fields,
                    })
                    .collect()
            }
            PatternKind::Or(alternatives) => {
                let mut covers = Vec::new();
                for alternative in alternatives {
                    covers.extend(self.cover_pattern(*alternative, budget));
                    if budget.exhausted {
                        return Vec::new();
                    }
                }
                covers
            }
        }
    }

    /// Renders one witness value for the diagnostic.
    fn cover_text(&self, cover: &Cover) -> String {
        match cover {
            Cover::Wildcard => "_".to_owned(),
            Cover::Bool(value) => value.to_string(),
            Cover::Int { negative, value } => {
                if *negative {
                    format!("-{value}")
                } else {
                    value.to_string()
                }
            }
            Cover::Char(value) => format!("{value:?}"),
            Cover::Float(bits) => format!("{}", f64::from_bits(*bits)),
            Cover::Variant {
                adt,
                variant,
                fields,
            } => {
                let definition = self.hir.ty_ctxt().adt(*adt);
                let enum_name = self.interner.resolve(definition.name);
                let Some(variant_def) = definition.variant(*variant) else {
                    return format!("{enum_name}::_");
                };
                let variant_name = self.interner.resolve(variant_def.name);
                if fields.is_empty() {
                    return format!("{enum_name}::{variant_name}");
                }
                let rendered: Vec<String> =
                    fields.iter().map(|field| self.cover_text(field)).collect();
                if variant_def.is_tuple() {
                    format!("{enum_name}::{variant_name}({})", rendered.join(", "))
                } else {
                    let fields: Vec<String> = variant_def
                        .fields
                        .iter()
                        .zip(rendered)
                        .map(|(field, value)| {
                            let name = field
                                .name
                                .map(|name| self.interner.resolve(name))
                                .unwrap_or("_");
                            format!("{name}: {value}")
                        })
                        .collect();
                    format!("{enum_name}::{variant_name} {{ {} }}", fields.join(", "))
                }
            }
        }
    }
}

/// Returns every constructor of a type, or `None` when the type has an infinite
/// constructor space and therefore needs a wildcard to be exhaustive.
fn constructors(tcx: &TyCtxt, ty: TyId) -> Option<Vec<Ctor>> {
    match tcx.kind(ty) {
        TyKind::Bool => Some(vec![Ctor::Bool(false), Ctor::Bool(true)]),
        TyKind::Adt(adt) => {
            let definition = tcx.adt(adt);
            if definition.is_enum() {
                Some(
                    definition
                        .variants
                        .iter()
                        .enumerate()
                        .map(|(index, _)| Ctor::Variant {
                            adt,
                            variant: VariantIdx::from_u32(
                                u32::try_from(index).expect("variant count fits in u32"),
                            )
                            .expect("enum variant index is in range"),
                        })
                        .collect(),
                )
            } else {
                None
            }
        }
        TyKind::Never | TyKind::Error => Some(Vec::new()),
        _ => None,
    }
}

fn ctor_fields(tcx: &TyCtxt, ctor: Ctor) -> Vec<TyId> {
    match ctor {
        Ctor::Bool(_) | Ctor::Int { .. } | Ctor::Char(_) | Ctor::Float(_) => Vec::new(),
        Ctor::Variant { adt, variant } => tcx
            .adt(adt)
            .variant(variant)
            .map(|variant| variant.fields.iter().map(|field| field.ty).collect())
            .unwrap_or_default(),
    }
}

fn ctor_matches(cover: &Cover, ctor: Ctor) -> bool {
    match (cover, ctor) {
        (Cover::Bool(left), Ctor::Bool(right)) => *left == right,
        (
            Cover::Int {
                negative: left_negative,
                value: left_value,
            },
            Ctor::Int {
                negative: right_negative,
                value: right_value,
            },
        ) => *left_negative == right_negative && *left_value == right_value,
        (Cover::Char(left), Ctor::Char(right)) => *left == right,
        (Cover::Float(left), Ctor::Float(right)) => *left == right,
        (
            Cover::Variant {
                adt: left_adt,
                variant: left_variant,
                ..
            },
            Ctor::Variant {
                adt: right_adt,
                variant: right_variant,
            },
        ) => *left_adt == right_adt && *left_variant == right_variant,
        _ => false,
    }
}

fn cover_fields(cover: &Cover) -> &[Cover] {
    match cover {
        Cover::Variant { fields, .. } => fields,
        _ => &[],
    }
}

fn cover_from_ctor(ctor: Ctor, fields: Vec<Cover>) -> Cover {
    match ctor {
        Ctor::Bool(value) => Cover::Bool(value),
        Ctor::Int { negative, value } => Cover::Int { negative, value },
        Ctor::Char(value) => Cover::Char(value),
        Ctor::Float(value) => Cover::Float(value),
        Ctor::Variant { adt, variant } => Cover::Variant {
            adt,
            variant,
            fields,
        },
    }
}

/// Keeps rows that can match `ctor`, expanding wildcards into its fields.
fn specialize(matrix: &[Vec<Cover>], ctor: Ctor, arity: usize) -> Vec<Vec<Cover>> {
    let mut specialized = Vec::new();
    for row in matrix {
        let Some(first) = row.first() else {
            continue;
        };
        if matches!(first, Cover::Wildcard) {
            let mut expanded = vec![Cover::Wildcard; arity];
            expanded.extend_from_slice(&row[1..]);
            specialized.push(expanded);
        } else if ctor_matches(first, ctor) {
            let mut expanded = cover_fields(first).to_vec();
            expanded.extend_from_slice(&row[1..]);
            specialized.push(expanded);
        }
    }
    specialized
}

/// Keeps rows whose first pattern is a wildcard, dropping that column.
fn default_matrix(matrix: &[Vec<Cover>]) -> Vec<Vec<Cover>> {
    matrix
        .iter()
        .filter(|row| matches!(row.first(), Some(Cover::Wildcard)))
        .map(|row| row[1..].to_vec())
        .collect()
}

/// Returns `true` when `candidate` matches a value that no matrix row matches.
fn is_useful(matrix: &[Vec<Cover>], candidate: &[Cover], types: &[TyId], tcx: &TyCtxt) -> bool {
    let Some(first) = candidate.first() else {
        return matrix.is_empty();
    };
    let Some(ty) = types.first().copied() else {
        return false;
    };

    let Some(constructors) = constructors(tcx, ty) else {
        let specialized = default_matrix(matrix);
        return is_useful(&specialized, &candidate[1..], &types[1..], tcx);
    };

    if matches!(first, Cover::Wildcard) {
        for ctor in constructors {
            let fields = ctor_fields(tcx, ctor);
            let specialized = specialize(matrix, ctor, fields.len());
            let mut nested = vec![Cover::Wildcard; fields.len()];
            nested.extend_from_slice(&candidate[1..]);
            let mut nested_types = fields;
            nested_types.extend_from_slice(&types[1..]);
            if is_useful(&specialized, &nested, &nested_types, tcx) {
                return true;
            }
        }
        return false;
    }

    let Some(first_ctor) = cover_ctor(first) else {
        return false;
    };
    let fields = ctor_fields(tcx, first_ctor);
    let specialized = specialize(matrix, first_ctor, fields.len());
    let mut nested = cover_fields(first).to_vec();
    nested.extend_from_slice(&candidate[1..]);
    let mut nested_types = fields;
    nested_types.extend_from_slice(&types[1..]);
    is_useful(&specialized, &nested, &nested_types, tcx)
}

fn cover_ctor(cover: &Cover) -> Option<Ctor> {
    match cover {
        Cover::Bool(value) => Some(Ctor::Bool(*value)),
        Cover::Int { negative, value } => Some(Ctor::Int {
            negative: *negative,
            value: *value,
        }),
        Cover::Char(value) => Some(Ctor::Char(*value)),
        Cover::Float(value) => Some(Ctor::Float(*value)),
        Cover::Variant { adt, variant, .. } => Some(Ctor::Variant {
            adt: *adt,
            variant: *variant,
        }),
        Cover::Wildcard => None,
    }
}

/// Returns a value that the matrix does not cover, or `None` when it covers
/// every value of `types`.
fn witness(matrix: &[Vec<Cover>], types: &[TyId], tcx: &TyCtxt) -> Option<Vec<Cover>> {
    let Some(ty) = types.first().copied() else {
        return matrix.is_empty().then(Vec::new);
    };

    if let Some(constructors) = constructors(tcx, ty) {
        for ctor in constructors {
            let fields = ctor_fields(tcx, ctor);
            let specialized = specialize(matrix, ctor, fields.len());
            let mut nested_types = fields.clone();
            nested_types.extend_from_slice(&types[1..]);
            if let Some(nested) = witness(&specialized, &nested_types, tcx) {
                let (field_witnesses, rest) = nested.split_at(fields.len());
                let mut result = vec![cover_from_ctor(ctor, field_witnesses.to_vec())];
                result.extend_from_slice(rest);
                return Some(result);
            }
        }
        return None;
    }

    let specialized = default_matrix(matrix);
    let nested = witness(&specialized, &types[1..], tcx)?;
    let mut result = vec![fresh_cover(tcx, ty, matrix)];
    result.extend(nested);
    Some(result)
}

/// Chooses a representative value for an infinite scalar type.
fn fresh_cover(tcx: &TyCtxt, ty: TyId, matrix: &[Vec<Cover>]) -> Cover {
    match tcx.kind(ty) {
        TyKind::Int(_) | TyKind::Uint(_) => {
            let mut value = 0u128;
            while matrix.iter().any(|row| {
                matches!(
                    row.first(),
                    Some(Cover::Int {
                        negative: false,
                        value: used,
                    }) if *used == value
                )
            }) {
                value += 1;
            }
            Cover::Int {
                negative: false,
                value,
            }
        }
        TyKind::Char => {
            let mut code = 0u32;
            loop {
                let Some(candidate) = char::from_u32(code) else {
                    code += 1;
                    continue;
                };
                let used = matrix.iter().any(
                    |row| matches!(row.first(), Some(Cover::Char(value)) if *value == candidate),
                );
                if !used {
                    return Cover::Char(candidate);
                }
                code += 1;
            }
        }
        TyKind::Float(kind) => {
            let candidates: [f64; 4] = [0.0, 1.0, -1.0, 2.0];
            for candidate in candidates {
                let bits = match kind {
                    FloatTy::F32 => f64::from(candidate as f32).to_bits(),
                    FloatTy::F64 => candidate.to_bits(),
                };
                let used = matrix
                    .iter()
                    .any(|row| matches!(row.first(), Some(Cover::Float(used)) if *used == bits));
                if !used {
                    return Cover::Float(bits);
                }
            }
            Cover::Float(3.0f64.to_bits())
        }
        _ => Cover::Wildcard,
    }
}
