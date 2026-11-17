//! Exhaustiveness and reachability of `match` arms.
//!
//! The algorithm is the classic usefulness check: an arm is reachable when its
//! pattern can match a value that no earlier unguarded pattern matches. The same
//! machinery, asked about a wildcard, produces a witness for a non-exhaustive
//! match. Guards never contribute coverage, because a guard can fail.
//!
//! Scalar ranges are handled by partitioning the value domain at every range
//! endpoint. Each resulting interval is a constructor, so two adjacent ranges
//! cover their type and an uncovered gap is reported with a concrete value.

use seal_ast::PatternLiteral;
use seal_error_codes as codes;
use seal_hir::{HirPatternId, MatchArm, PatternKind};
use seal_span::Span;
use seal_ty::{AdtId, FloatTy, TyCtxt, TyId, TyKind, VariantIdx};

use super::Checker;

/// Upper bound on the cartesian product of nested or-patterns.
pub(super) const MAX_ALTERNATIVES: usize = 65_536;

const SIGN_BIT: u128 = 1 << 127;
pub(super) const CHAR_MAX: u128 = 0x10FFFF;

/// A pattern shape used by the usefulness algorithm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Cover {
    Wildcard,
    /// A pattern whose coverage cannot be represented in this lattice. It
    /// matches no constructor when it appears in a matrix, and behaves like a
    /// wildcard when it is the candidate being tested for reachability.
    Opaque,
    /// A slice pattern inside a larger pattern: a length plus prefix and suffix
    /// element constraints.
    Slice {
        prefix: Vec<Cover>,
        suffix: Vec<Cover>,
        min_length: u64,
        exact: Option<u64>,
        wildcard: bool,
        matches_nothing: bool,
    },
    Bool(bool),
    IntRange {
        ty: TyId,
        start: u128,
        end: u128,
    },
    CharRange {
        start: u32,
        end: u32,
    },
    Float(u64),
    Variant {
        adt: AdtId,
        variant: VariantIdx,
        fields: Vec<Cover>,
    },
    Struct {
        adt: AdtId,
        fields: Vec<Cover>,
    },
    Array {
        fields: Vec<Cover>,
    },
}

/// A constructor of a type, used to expand wildcards.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ctor {
    Bool(bool),
    IntRange { start: u128, end: u128 },
    CharRange { start: u32, end: u32 },
    Float(u64),
    Variant { adt: AdtId, variant: VariantIdx },
    Struct { adt: AdtId },
    Array { element: TyId, length: u64 },
    SliceLength { element: TyId, length: u64 },
}

/// Tracks the or-pattern expansion budget.
pub(super) struct Budget {
    pub(super) remaining: usize,
    pub(super) exhausted: bool,
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
        if let Some(element) = self.slice_reference_element(scrutinee_ty) {
            return self.check_slice_coverage(element, arms, span);
        }

        let mut covered: Vec<Vec<Cover>> = Vec::new();

        for arm in arms {
            let mut budget = Budget {
                remaining: MAX_ALTERNATIVES,
                exhausted: false,
            };
            let alternatives = self.cover_pattern(arm.pattern, scrutinee_ty, &mut budget);
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
                    self.pointer_width,
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
        let Some(witness) = witness(&covered, &[scrutinee_ty], tcx, self.pointer_width) else {
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
    pub(super) fn cover_pattern(
        &self,
        pattern: HirPatternId,
        ty: TyId,
        budget: &mut Budget,
    ) -> Vec<Cover> {
        if budget.remaining == 0 {
            budget.exhausted = true;
            return Vec::new();
        }
        budget.remaining -= 1;

        match &self.hir.pattern(pattern).kind {
            PatternKind::Wildcard | PatternKind::Error => vec![Cover::Wildcard],
            PatternKind::Binding { pattern, .. } => match pattern {
                Some(pattern) => self.cover_pattern(*pattern, ty, budget),
                None => vec![Cover::Wildcard],
            },
            PatternKind::Literal(literal) => self.cover_literal(*literal, ty).into_iter().collect(),
            PatternKind::Range {
                start,
                end,
                inclusive,
            } => self
                .cover_range(*start, *end, *inclusive, ty)
                .into_iter()
                .collect(),
            PatternKind::Variant {
                adt,
                variant,
                fields,
            } => {
                let Some(adt_id) = self.hir.adt(*adt) else {
                    return vec![Cover::Wildcard];
                };
                let definition = self.hir.ty_ctxt().adt(adt_id);
                let Some(variant_def) = definition.variant(*variant) else {
                    return vec![Cover::Wildcard];
                };
                let field_types: Vec<TyId> =
                    variant_def.fields.iter().map(|field| field.ty).collect();
                let mut combinations: Vec<Vec<Cover>> = vec![Vec::new()];
                for (field, field_ty) in fields.iter().zip(&field_types) {
                    let options = self.cover_pattern(field.1, *field_ty, budget);
                    if budget.exhausted || options.is_empty() {
                        return Vec::new();
                    }
                    combinations = combine(combinations, options, budget);
                    if budget.exhausted {
                        return Vec::new();
                    }
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
            PatternKind::Struct { adt, fields } => {
                let Some(adt_id) = self.hir.adt(*adt) else {
                    return vec![Cover::Wildcard];
                };
                let definition = self.hir.ty_ctxt().adt(adt_id);
                let mut combinations: Vec<Vec<Cover>> = vec![Vec::new()];
                for (field, field_pattern) in fields {
                    let Some(field_def) = definition.field(*field) else {
                        return vec![Cover::Wildcard];
                    };
                    let options = self.cover_pattern(*field_pattern, field_def.ty, budget);
                    if budget.exhausted || options.is_empty() {
                        return Vec::new();
                    }
                    combinations = combine(combinations, options, budget);
                    if budget.exhausted {
                        return Vec::new();
                    }
                }
                combinations
                    .into_iter()
                    .map(|fields| Cover::Struct {
                        adt: adt_id,
                        fields,
                    })
                    .collect()
            }
            PatternKind::Array {
                prefix,
                has_rest,
                suffix,
            } => {
                let shape = match self.hir.ty_ctxt().kind(ty) {
                    TyKind::Array { element, length } => Some((element, length)),
                    TyKind::Ref { pointee, .. } => match self.hir.ty_ctxt().kind(pointee) {
                        TyKind::Slice { element } => {
                            let fixed = prefix.len() + suffix.len();
                            if *has_rest && fixed == 0 {
                                return vec![Cover::Wildcard];
                            }
                            let prefixes = self.cover_sequence(prefix, element, budget);
                            let suffixes = self.cover_sequence(suffix, element, budget);
                            if budget.exhausted || prefixes.is_empty() || suffixes.is_empty() {
                                return vec![Cover::Opaque];
                            }
                            let fixed = fixed as u64;
                            let mut covers = Vec::new();
                            for prefix in &prefixes {
                                for suffix in &suffixes {
                                    covers.push(Cover::Slice {
                                        prefix: prefix.clone(),
                                        suffix: suffix.clone(),
                                        min_length: fixed,
                                        exact: if *has_rest { None } else { Some(fixed) },
                                        wildcard: false,
                                        matches_nothing: false,
                                    });
                                }
                            }
                            return covers;
                        }
                        TyKind::Array { element, length } => Some((element, length)),
                        _ => None,
                    },
                    _ => None,
                };
                let Some((element, length)) = shape else {
                    return vec![Cover::Wildcard];
                };
                let fixed = prefix.len() + suffix.len();
                if fixed as u64 > length || (!*has_rest && fixed as u64 != length) {
                    // Type checking already reported the length mismatch; a
                    // guessed arity would make the usefulness check unsound.
                    return vec![Cover::Wildcard];
                }
                let rest = if *has_rest { length - fixed as u64 } else { 0 };
                let mut slots: Vec<Option<HirPatternId>> =
                    prefix.iter().copied().map(Some).collect();
                slots.extend((0..rest).map(|_| None));
                slots.extend(suffix.iter().copied().map(Some));

                let mut combinations: Vec<Vec<Cover>> = vec![Vec::new()];
                for slot in slots {
                    let options = match slot {
                        Some(pattern) => self.cover_pattern(pattern, element, budget),
                        None => vec![Cover::Wildcard],
                    };
                    if budget.exhausted || options.is_empty() {
                        return Vec::new();
                    }
                    combinations = combine(combinations, options, budget);
                    if budget.exhausted {
                        return Vec::new();
                    }
                }
                combinations
                    .into_iter()
                    .map(|fields| Cover::Array { fields })
                    .collect()
            }
            PatternKind::Or(alternatives) => {
                let mut covers = Vec::new();
                for alternative in alternatives {
                    covers.extend(self.cover_pattern(*alternative, ty, budget));
                    if budget.exhausted {
                        return Vec::new();
                    }
                }
                covers
            }
        }
    }

    fn cover_literal(&self, literal: PatternLiteral, ty: TyId) -> Option<Cover> {
        match literal {
            PatternLiteral::Int {
                value, negative, ..
            } => {
                let key = int_key(ty, negative, value, self.hir.ty_ctxt())?;
                Some(Cover::IntRange {
                    ty,
                    start: key,
                    end: key,
                })
            }
            PatternLiteral::Char(value) => Some(Cover::CharRange {
                start: value as u32,
                end: value as u32,
            }),
            PatternLiteral::Bool(value) => Some(Cover::Bool(value)),
            PatternLiteral::Float { value, .. } => Some(Cover::Float(value.to_bits())),
        }
    }

    fn cover_range(
        &self,
        start: PatternLiteral,
        end: PatternLiteral,
        inclusive: bool,
        ty: TyId,
    ) -> Option<Cover> {
        match (start, end) {
            (
                PatternLiteral::Int {
                    value: start_value,
                    negative: start_negative,
                    ..
                },
                PatternLiteral::Int {
                    value: end_value,
                    negative: end_negative,
                    ..
                },
            ) => {
                let mut start = int_key(ty, start_negative, start_value, self.hir.ty_ctxt())?;
                let mut end = int_key(ty, end_negative, end_value, self.hir.ty_ctxt())?;
                let (low, high) = scalar_domain(ty, self.hir.ty_ctxt(), self.pointer_width)?;
                if !inclusive {
                    end = end.checked_sub(1)?;
                }
                start = start.max(low);
                end = end.min(high);
                (start <= end).then_some(Cover::IntRange { ty, start, end })
            }
            (PatternLiteral::Char(start), PatternLiteral::Char(end)) => {
                let start = start as u32;
                let mut end = end as u32;
                if !inclusive {
                    end = end.checked_sub(1)?;
                }
                let low = 0u32;
                let high = (CHAR_MAX as u32).min(0x10FFFF);
                let start = start.max(low);
                let end = end.min(high);
                (start <= end).then_some(Cover::CharRange { start, end })
            }
            _ => None,
        }
    }

    /// Renders one witness value for the diagnostic.
    pub(super) fn cover_text(&self, cover: &Cover) -> String {
        match cover {
            Cover::Wildcard | Cover::Opaque => "_".to_owned(),
            Cover::Bool(value) => value.to_string(),
            Cover::IntRange { ty, start, end } => {
                if start == end {
                    render_int(*ty, *start, self.hir.ty_ctxt())
                } else {
                    format!(
                        "{}..={}",
                        render_int(*ty, *start, self.hir.ty_ctxt()),
                        render_int(*ty, *end, self.hir.ty_ctxt())
                    )
                }
            }
            Cover::CharRange { start, end } => {
                let start = char::from_u32(*start).unwrap_or('\u{FFFD}');
                if start as u32 == *end {
                    format!("{start:?}")
                } else {
                    let end = char::from_u32(*end).unwrap_or('\u{FFFD}');
                    format!("{start:?}..={end:?}")
                }
            }
            Cover::Float(bits) => format!("{}", f64::from_bits(*bits)),
            Cover::Slice { .. } => "[..]".to_owned(),
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
            Cover::Struct { adt, fields } => {
                let definition = self.hir.ty_ctxt().adt(*adt);
                let name = self.interner.resolve(definition.name);
                let fields: Vec<String> = definition
                    .fields
                    .iter()
                    .zip(fields)
                    .map(|(field, value)| {
                        let field_name = field
                            .name
                            .map(|name| self.interner.resolve(name))
                            .unwrap_or("_");
                        format!("{field_name}: {}", self.cover_text(value))
                    })
                    .collect();
                format!("{name} {{ {} }}", fields.join(", "))
            }
            Cover::Array { fields } => {
                let rendered: Vec<String> =
                    fields.iter().map(|field| self.cover_text(field)).collect();
                format!("[{}]", rendered.join(", "))
            }
        }
    }
}
pub(super) fn combine(
    prefixes: Vec<Vec<Cover>>,
    options: Vec<Cover>,
    budget: &mut Budget,
) -> Vec<Vec<Cover>> {
    let mut next = Vec::new();
    for prefix in &prefixes {
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
    next
}

/// Converts an integer pattern literal into its ordering key.
fn int_key(ty: TyId, negative: bool, magnitude: u128, tcx: &TyCtxt) -> Option<u128> {
    match tcx.kind(ty) {
        TyKind::Int(_) => Some(signed_key(signed_value(negative, magnitude))),
        TyKind::Uint(_) if !negative => Some(magnitude),
        _ => None,
    }
}

fn signed_value(negative: bool, magnitude: u128) -> i128 {
    let magnitude = magnitude.min(i128::MAX as u128) as i128;
    if negative { -magnitude } else { magnitude }
}

fn signed_key(value: i128) -> u128 {
    (value as u128) ^ SIGN_BIT
}

fn render_int(ty: TyId, key: u128, tcx: &TyCtxt) -> String {
    match tcx.kind(ty) {
        TyKind::Int(_) => ((key ^ SIGN_BIT) as i128).to_string(),
        TyKind::Uint(_) => key.to_string(),
        _ => key.to_string(),
    }
}

/// Returns the closed key range of a scalar type.
pub(super) fn scalar_domain(ty: TyId, tcx: &TyCtxt, pointer_width: u8) -> Option<(u128, u128)> {
    match tcx.kind(ty) {
        TyKind::Int(kind) => {
            let bits = kind
                .bits()
                .map(u32::from)
                .unwrap_or(u32::from(pointer_width));
            if bits >= 128 {
                Some((0, u128::MAX))
            } else {
                let max = (1i128 << (bits - 1)) - 1;
                let min = -(1i128 << (bits - 1));
                Some((signed_key(min), signed_key(max)))
            }
        }
        TyKind::Uint(kind) => {
            let bits = kind
                .bits()
                .map(u32::from)
                .unwrap_or(u32::from(pointer_width));
            let max = if bits >= 128 {
                u128::MAX
            } else {
                (1u128 << bits) - 1
            };
            Some((0, max))
        }
        TyKind::Char => Some((0, CHAR_MAX)),
        _ => None,
    }
}

/// Returns every constructor of a type, or `None` when the type needs a
/// wildcard to be exhaustive.
fn constructors(tcx: &TyCtxt, ty: TyId, covers: &[&Cover], pointer_width: u8) -> Option<Vec<Ctor>> {
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
                Some(vec![Ctor::Struct { adt }])
            }
        }
        TyKind::Array { element, length } => Some(vec![Ctor::Array { element, length }]),
        TyKind::Ref { pointee, .. } => match tcx.kind(pointee) {
            TyKind::Slice { element } => Some(slice_constructors(element, covers)),
            _ => None,
        },
        TyKind::Int(_) | TyKind::Uint(_) => Some(interval_ctors(ty, covers, tcx, pointer_width)),
        TyKind::Char => Some(char_ctors(covers)),
        TyKind::Never | TyKind::Error => Some(Vec::new()),
        _ => None,
    }
}

/// Returns the length constructors for a slice pattern column.
fn slice_constructors(element: TyId, covers: &[&Cover]) -> Vec<Ctor> {
    let mut lengths = vec![0u64, 1];
    let mut max_total = 0u64;
    for cover in covers {
        let Cover::Slice {
            prefix,
            suffix,
            min_length,
            exact,
            wildcard,
            matches_nothing,
        } = cover
        else {
            continue;
        };
        if *wildcard || *matches_nothing {
            continue;
        }
        let fixed = prefix.len() as u64 + suffix.len() as u64;
        max_total = max_total.max(fixed);
        lengths.push(fixed);
        lengths.push(fixed.saturating_add(1));
        match exact {
            Some(exact) => {
                max_total = max_total.max(*exact);
                lengths.push(*exact);
            }
            None => {
                max_total = max_total.max(*min_length);
                lengths.push(*min_length);
            }
        }
    }
    lengths.push(max_total.saturating_add(1));
    lengths.retain(|length| *length <= max_total.saturating_add(1));
    lengths.sort_unstable();
    lengths.dedup();
    lengths
        .into_iter()
        .map(|length| Ctor::SliceLength { element, length })
        .collect()
}

/// Partitions a scalar domain at the endpoints of every observed range.
fn interval_ctors(ty: TyId, covers: &[&Cover], tcx: &TyCtxt, pointer_width: u8) -> Vec<Ctor> {
    let Some((low, high)) = scalar_domain(ty, tcx, pointer_width) else {
        return Vec::new();
    };
    let mut starts = vec![low];
    for cover in covers {
        if let Cover::IntRange {
            ty: cover_ty,
            start,
            end,
        } = cover
            && *cover_ty == ty
        {
            let start = (*start).max(low);
            let end = (*end).min(high);
            if start <= end {
                starts.push(start);
                if end < high {
                    starts.push(end + 1);
                }
            }
        }
    }
    starts.sort_unstable();
    starts.dedup();
    let mut ctors = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map(|next| next - 1).unwrap_or(high);
        if *start <= end {
            ctors.push(Ctor::IntRange { start: *start, end });
        }
    }
    ctors
}

fn char_ctors(covers: &[&Cover]) -> Vec<Ctor> {
    let high = CHAR_MAX as u32;
    let mut starts = vec![0u32];
    for cover in covers {
        if let Cover::CharRange { start, end } = cover {
            let start = *start;
            let end = (*end).min(high);
            if start <= end {
                starts.push(start);
                if end < high {
                    starts.push(end + 1);
                }
            }
        }
    }
    starts.sort_unstable();
    starts.dedup();
    let mut ctors = Vec::with_capacity(starts.len());
    for (index, start) in starts.iter().enumerate() {
        let end = starts.get(index + 1).map(|next| next - 1).unwrap_or(high);
        if *start <= end {
            ctors.push(Ctor::CharRange { start: *start, end });
        }
    }
    ctors
}

fn ctor_fields(tcx: &TyCtxt, ctor: Ctor) -> Vec<TyId> {
    match ctor {
        Ctor::Bool(_) | Ctor::IntRange { .. } | Ctor::CharRange { .. } | Ctor::Float(_) => {
            Vec::new()
        }
        Ctor::Variant { adt, variant } => tcx
            .adt(adt)
            .variant(variant)
            .map(|variant| variant.fields.iter().map(|field| field.ty).collect())
            .unwrap_or_default(),
        Ctor::Struct { adt } => tcx.adt(adt).fields.iter().map(|field| field.ty).collect(),
        Ctor::Array { element, length } => vec![element; length as usize],
        Ctor::SliceLength { element, length } => vec![element; length as usize],
    }
}

fn ctor_matches(cover: &Cover, ctor: Ctor) -> bool {
    match (cover, ctor) {
        (Cover::Bool(left), Ctor::Bool(right)) => *left == right,
        (
            Cover::IntRange {
                start: left_start,
                end: left_end,
                ..
            },
            Ctor::IntRange { start, end },
        ) => *left_start <= start && end <= *left_end,
        (
            Cover::CharRange {
                start: left_start,
                end: left_end,
            },
            Ctor::CharRange { start, end },
        ) => *left_start <= start && end <= *left_end,
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
        (Cover::Struct { adt: left, .. }, Ctor::Struct { adt: right }) => *left == right,
        (Cover::Array { .. }, Ctor::Array { .. }) => true,
        (
            Cover::Slice {
                min_length,
                exact,
                wildcard,
                matches_nothing,
                ..
            },
            Ctor::SliceLength { length, .. },
        ) => {
            if *matches_nothing {
                false
            } else if *wildcard {
                true
            } else if let Some(exact) = exact {
                *exact == length
            } else {
                length >= *min_length
            }
        }
        _ => false,
    }
}

fn cover_fields(cover: &Cover) -> &[Cover] {
    match cover {
        Cover::Variant { fields, .. } | Cover::Struct { fields, .. } | Cover::Array { fields } => {
            fields
        }
        _ => &[],
    }
}

fn cover_from_ctor(ctor: Ctor, fields: Vec<Cover>, ty: Option<TyId>) -> Cover {
    match ctor {
        Ctor::Bool(value) => Cover::Bool(value),
        Ctor::IntRange { start, end } => Cover::IntRange {
            ty: ty.expect("a scalar constructor carries its type"),
            start,
            end,
        },
        Ctor::CharRange { start, end } => Cover::CharRange { start, end },
        Ctor::Float(value) => Cover::Float(value),
        Ctor::Variant { adt, variant } => Cover::Variant {
            adt,
            variant,
            fields,
        },
        Ctor::Struct { adt } => Cover::Struct { adt, fields },
        Ctor::Array { .. } | Ctor::SliceLength { .. } => Cover::Array { fields },
    }
}

/// Expands a slice cover at the length selected by `ctor`, or returns its
/// ordinary constructor fields.
fn expanded_fields(cover: &Cover, ctor: Ctor) -> Vec<Cover> {
    if let Cover::Slice { prefix, suffix, .. } = cover
        && let Ctor::SliceLength { length, .. } = ctor
    {
        let fixed = prefix.len() as u64 + suffix.len() as u64;
        let mut fields = prefix.clone();
        fields.extend((0..length - fixed).map(|_| Cover::Wildcard));
        fields.extend(suffix.iter().cloned());
        return fields;
    }
    cover_fields(cover).to_vec()
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
            let mut expanded = expanded_fields(first, ctor);
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
pub(super) fn is_useful(
    matrix: &[Vec<Cover>],
    candidate: &[Cover],
    types: &[TyId],
    tcx: &TyCtxt,
    pointer_width: u8,
) -> bool {
    let Some(first) = candidate.first() else {
        return matrix.is_empty();
    };
    let Some(ty) = types.first().copied() else {
        return false;
    };

    let mut covers: Vec<&Cover> = matrix.iter().filter_map(|row| row.first()).collect();
    covers.push(first);
    let Some(constructors) = constructors(tcx, ty, &covers, pointer_width) else {
        let specialized = default_matrix(matrix);
        return is_useful(
            &specialized,
            &candidate[1..],
            &types[1..],
            tcx,
            pointer_width,
        );
    };

    if matches!(first, Cover::Slice { .. }) {
        for ctor in constructors {
            if !ctor_matches(first, ctor) {
                continue;
            }
            let fields = expanded_fields(first, ctor);
            let specialized = specialize(matrix, ctor, fields.len());
            let mut nested = fields;
            nested.extend_from_slice(&candidate[1..]);
            let mut nested_types = ctor_fields(tcx, ctor);
            nested_types.extend_from_slice(&types[1..]);
            if is_useful(&specialized, &nested, &nested_types, tcx, pointer_width) {
                return true;
            }
        }
        return false;
    }

    if matches!(first, Cover::Wildcard | Cover::Opaque) {
        for ctor in constructors {
            let fields = ctor_fields(tcx, ctor);
            let specialized = specialize(matrix, ctor, fields.len());
            let mut nested = vec![Cover::Wildcard; fields.len()];
            nested.extend_from_slice(&candidate[1..]);
            let mut nested_types = fields;
            nested_types.extend_from_slice(&types[1..]);
            if is_useful(&specialized, &nested, &nested_types, tcx, pointer_width) {
                return true;
            }
        }
        return false;
    }

    let Some(first_ctor) = cover_ctor(first, ty, tcx) else {
        return false;
    };
    let fields = ctor_fields(tcx, first_ctor);
    let specialized = specialize(matrix, first_ctor, fields.len());
    let mut nested = cover_fields(first).to_vec();
    nested.extend_from_slice(&candidate[1..]);
    let mut nested_types = fields;
    nested_types.extend_from_slice(&types[1..]);
    is_useful(&specialized, &nested, &nested_types, tcx, pointer_width)
}

fn cover_ctor(cover: &Cover, ty: TyId, tcx: &TyCtxt) -> Option<Ctor> {
    match cover {
        Cover::Bool(value) => Some(Ctor::Bool(*value)),
        Cover::IntRange { start, end, .. } => Some(Ctor::IntRange {
            start: *start,
            end: *end,
        }),
        Cover::CharRange { start, end } => Some(Ctor::CharRange {
            start: *start,
            end: *end,
        }),
        Cover::Float(value) => Some(Ctor::Float(*value)),
        Cover::Slice { .. } => None,
        Cover::Variant { adt, variant, .. } => Some(Ctor::Variant {
            adt: *adt,
            variant: *variant,
        }),
        Cover::Struct { adt, .. } => Some(Ctor::Struct { adt: *adt }),
        Cover::Array { .. } => match tcx.kind(ty) {
            TyKind::Array { element, length } => Some(Ctor::Array { element, length }),
            _ => None,
        },
        Cover::Opaque | Cover::Wildcard => None,
    }
}

/// Returns a value that the matrix does not cover, or `None` when it covers
/// every value of `types`.
pub(super) fn witness(
    matrix: &[Vec<Cover>],
    types: &[TyId],
    tcx: &TyCtxt,
    pointer_width: u8,
) -> Option<Vec<Cover>> {
    let Some(ty) = types.first().copied() else {
        return matrix.is_empty().then(Vec::new);
    };

    let covers: Vec<&Cover> = matrix.iter().filter_map(|row| row.first()).collect();
    if let Some(constructors) = constructors(tcx, ty, &covers, pointer_width) {
        for ctor in constructors {
            let fields = ctor_fields(tcx, ctor);
            let specialized = specialize(matrix, ctor, fields.len());
            let mut nested_types = fields.clone();
            nested_types.extend_from_slice(&types[1..]);
            if let Some(nested) = witness(&specialized, &nested_types, tcx, pointer_width) {
                let (field_witnesses, rest) = nested.split_at(fields.len());
                let mut result = vec![cover_from_ctor(ctor, field_witnesses.to_vec(), Some(ty))];
                result.extend_from_slice(rest);
                return Some(result);
            }
        }
        return None;
    }

    let specialized = default_matrix(matrix);
    let nested = witness(&specialized, &types[1..], tcx, pointer_width)?;
    let mut result = vec![fresh_cover(tcx, ty, matrix)];
    result.extend(nested);
    Some(result)
}

/// Chooses a representative value for a type without a finite constructor set.
fn fresh_cover(tcx: &TyCtxt, ty: TyId, matrix: &[Vec<Cover>]) -> Cover {
    match tcx.kind(ty) {
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
