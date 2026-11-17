//! Exhaustiveness and reachability of slice patterns.
//!
//! A slice pattern constrains a runtime length plus a fixed prefix and suffix of
//! element values. The length domain is split at every boundary a pattern
//! mentions; at one concrete length the pattern becomes an array pattern, so the
//! ordinary usefulness check answers reachability and exhaustiveness. Lengths
//! past every boundary behave alike, so one representative covers them all.

use seal_error_codes as codes;
use seal_hir::{HirPatternId, MatchArm, PatternKind};
use seal_span::Span;
use seal_ty::{TyId, TyKind};

use super::Checker;
use super::exhaustive::{
    Budget, CHAR_MAX, Cover, MAX_ALTERNATIVES, combine, is_useful, scalar_domain, witness,
};

/// One slice pattern, expressed as fixed prefix and suffix constraints.
#[derive(Clone, Debug)]
struct SliceCover {
    prefix: Vec<Cover>,
    suffix: Vec<Cover>,
    /// Smallest slice length this pattern accepts.
    min_length: u64,
    /// Exact accepted length, when the pattern has no `..`.
    exact: Option<u64>,
    /// Matches every slice value.
    wildcard: bool,
    /// Cannot match any value because the pattern already contained an error.
    matches_nothing: bool,
}

impl SliceCover {
    fn wildcard() -> Self {
        Self {
            prefix: Vec::new(),
            suffix: Vec::new(),
            min_length: 0,
            exact: None,
            wildcard: true,
            matches_nothing: false,
        }
    }

    fn nothing() -> Self {
        Self {
            prefix: Vec::new(),
            suffix: Vec::new(),
            min_length: 0,
            exact: None,
            wildcard: false,
            matches_nothing: true,
        }
    }

    /// Expands this slice pattern at one concrete slice length.
    fn cover_at(&self, length: u64) -> Option<Cover> {
        if self.matches_nothing {
            return None;
        }
        if self.wildcard {
            return Some(Cover::Wildcard);
        }
        if let Some(exact) = self.exact
            && exact != length
        {
            return None;
        }
        if length < self.min_length {
            return None;
        }
        let fixed = self.prefix.len() as u64 + self.suffix.len() as u64;
        let middle = length - fixed;
        let mut fields = Vec::with_capacity(length as usize);
        fields.extend(self.prefix.iter().cloned());
        fields.extend((0..middle).map(|_| Cover::Wildcard));
        fields.extend(self.suffix.iter().cloned());
        Some(Cover::Array { fields })
    }
}

/// Builds the usefulness matrix for one slice length.
fn slice_matrix(covered: &[SliceCover], length: u64) -> Vec<Vec<Cover>> {
    covered
        .iter()
        .filter_map(|cover| cover.cover_at(length))
        .map(|cover| vec![cover])
        .collect()
}

impl Checker<'_> {
    /// Returns the element type when `ty` is a reference to a slice.
    pub(super) fn slice_reference_element(&self, ty: TyId) -> Option<TyId> {
        let TyKind::Ref { pointee, .. } = self.hir.ty_ctxt().kind(ty) else {
            return None;
        };
        match self.hir.ty_ctxt().kind(pointee) {
            TyKind::Slice { element } => Some(element),
            _ => None,
        }
    }

    /// Reports unreachable arms and exhaustiveness for a slice scrutinee.
    pub(super) fn check_slice_coverage(
        &self,
        element: TyId,
        arms: &[MatchArm],
        span: Span,
    ) -> bool {
        let mut budget = Budget {
            remaining: MAX_ALTERNATIVES,
            exhausted: false,
        };
        let mut alternatives: Vec<Vec<SliceCover>> = Vec::with_capacity(arms.len());
        for arm in arms {
            alternatives.push(self.slice_pattern_covers(arm.pattern, element, &mut budget));
        }
        if budget.exhausted {
            self.error(
                codes::E3032,
                span,
                "this match has too many alternative combinations to check exhaustiveness",
            )
            .help("split the match into smaller matches")
            .emit();
            return true;
        }

        let mut representatives = vec![0u64, 1];
        let mut max_total = 0u64;
        for covers in &alternatives {
            for cover in covers {
                if cover.wildcard || cover.matches_nothing {
                    continue;
                }
                let fixed = cover.prefix.len() as u64 + cover.suffix.len() as u64;
                max_total = max_total.max(fixed);
                representatives.push(fixed);
                representatives.push(fixed.saturating_add(1));
                match cover.exact {
                    Some(exact) => {
                        max_total = max_total.max(exact);
                        representatives.push(exact);
                    }
                    None => {
                        max_total = max_total.max(cover.min_length);
                        representatives.push(cover.min_length);
                    }
                }
            }
        }
        representatives.push(max_total.saturating_add(1));
        representatives.retain(|length| *length <= max_total.saturating_add(1));
        representatives.sort_unstable();
        representatives.dedup();

        let tcx = self.hir.ty_ctxt();
        let mut covered: Vec<SliceCover> = Vec::new();
        for (index, arm) in arms.iter().enumerate() {
            let covers = &alternatives[index];
            let mut reachable = false;
            for &length in &representatives {
                let matrix = slice_matrix(&covered, length);
                let ty = tcx.array_ty(element, length);
                let candidates: Vec<Cover> = covers
                    .iter()
                    .filter_map(|cover| cover.cover_at(length))
                    .collect();
                if candidates.iter().any(|candidate| {
                    is_useful(
                        &matrix,
                        std::slice::from_ref(candidate),
                        &[ty],
                        tcx,
                        self.pointer_width,
                    )
                }) {
                    reachable = true;
                }
            }
            if !reachable && !covers.is_empty() {
                self.error(codes::E3033, arm.span, "this match arm is unreachable")
                    .help("remove the arm, or move it before the arm that covers it")
                    .emit();
            }
            if arm.guard.is_none() {
                covered.extend(covers.iter().cloned());
            }
        }

        for &length in &representatives {
            let matrix = slice_matrix(&covered, length);
            let ty = tcx.array_ty(element, length);
            let Some(witness) = witness(&matrix, &[ty], tcx, self.pointer_width) else {
                continue;
            };
            let Some(cover) = witness.first() else {
                continue;
            };
            let missing = self.slice_witness_text(cover, length);
            self.error(
                codes::E3032,
                span,
                format!("non-exhaustive match: the slice {missing} is not covered"),
            )
            .help("add an arm for it, or finish with `_ => ...`")
            .emit();
            return false;
        }
        true
    }

    /// Expands one slice pattern into its length and element constraints.
    fn slice_pattern_covers(
        &self,
        pattern: HirPatternId,
        element: TyId,
        budget: &mut Budget,
    ) -> Vec<SliceCover> {
        if budget.remaining == 0 {
            budget.exhausted = true;
            return Vec::new();
        }
        budget.remaining -= 1;

        match &self.hir.pattern(pattern).kind {
            PatternKind::Wildcard | PatternKind::Error => vec![SliceCover::wildcard()],
            PatternKind::Binding { pattern, .. } => match pattern {
                Some(pattern) => self.slice_pattern_covers(*pattern, element, budget),
                None => vec![SliceCover::wildcard()],
            },
            PatternKind::Array {
                prefix,
                has_rest,
                suffix,
            } => {
                let fixed = prefix.len() + suffix.len();
                if *has_rest && fixed == 0 {
                    return vec![SliceCover::wildcard()];
                }
                let prefixes = self.cover_sequence(prefix, element, budget);
                let suffixes = self.cover_sequence(suffix, element, budget);
                if budget.exhausted || prefixes.is_empty() || suffixes.is_empty() {
                    return Vec::new();
                }
                let fixed = fixed as u64;
                let mut covers = Vec::new();
                for prefix in &prefixes {
                    for suffix in &suffixes {
                        covers.push(SliceCover {
                            prefix: prefix.clone(),
                            suffix: suffix.clone(),
                            min_length: fixed,
                            exact: if *has_rest { None } else { Some(fixed) },
                            wildcard: false,
                            matches_nothing: false,
                        });
                    }
                }
                covers
            }
            PatternKind::Or(alternatives) => {
                let mut covers = Vec::new();
                for alternative in alternatives {
                    covers.extend(self.slice_pattern_covers(*alternative, element, budget));
                    if budget.exhausted {
                        return Vec::new();
                    }
                }
                covers
            }
            _ => vec![SliceCover::nothing()],
        }
    }

    /// Expands a sequence of element patterns into every alternative combination.
    pub(super) fn cover_sequence(
        &self,
        patterns: &[HirPatternId],
        element: TyId,
        budget: &mut Budget,
    ) -> Vec<Vec<Cover>> {
        let mut combinations: Vec<Vec<Cover>> = vec![Vec::new()];
        for pattern in patterns {
            let options = self.cover_pattern(*pattern, element, budget);
            if budget.exhausted || options.is_empty() {
                return Vec::new();
            }
            combinations = combine(combinations, options, budget);
            if budget.exhausted {
                return Vec::new();
            }
        }
        combinations
    }

    /// Renders one slice witness element, hiding constraints that only repeat
    /// the whole scalar domain.
    fn slice_field_text(&self, cover: &Cover) -> String {
        match cover {
            Cover::IntRange { ty, start, end } => {
                if let Some((low, high)) =
                    scalar_domain(*ty, self.hir.ty_ctxt(), self.pointer_width)
                    && *start == low
                    && *end == high
                {
                    return "_".to_owned();
                }
                self.cover_text(cover)
            }
            Cover::CharRange { start: 0, end } if *end == CHAR_MAX as u32 => "_".to_owned(),
            _ => self.cover_text(cover),
        }
    }

    /// Renders a slice witness, eliding the middle of a long slice.
    fn slice_witness_text(&self, witness: &Cover, length: u64) -> String {
        if length == 0 {
            return "[]".to_owned();
        }
        let Cover::Array { fields } = witness else {
            return "[..]".to_owned();
        };
        let rendered: Vec<String> = fields
            .iter()
            .map(|field| self.slice_field_text(field))
            .collect();
        if rendered.len() <= 4 {
            format!("[{}]", rendered.join(", "))
        } else {
            format!(
                "[{}, {}, .., {}]",
                rendered[0],
                rendered[1],
                rendered[rendered.len() - 1]
            )
        }
    }
}
