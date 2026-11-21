//! Query-backed per-body type checking and MIR lowering.
//!
//! Declarations are checked once per HIR unit. Each non-closure body then has
//! its own query with three dependencies: the HIR unit, the declarations table,
//! and an input containing that body's source text. A body edit changes only its
//! own input; the HIR value is recomputed (and backdated), while every other
//! body keeps its checked tree and MIR.
//!
//! The fingerprints are semantic identities, not timestamps: a body result is
//! identified by its source text and the declarations fingerprint, and MIR by
//! that result. A signature edit therefore invalidates dependent bodies even
//! when their own text did not change.

use std::sync::Arc;

use seal_hash::{Fingerprint, StableHasher, stable_hash_of};
use seal_hir::{BodyId, Hir};
use seal_mir::Body;
use seal_query::{Dependency, InputKind, QueryError, QueryKey, QueryKind, QueryOutput};
use seal_session::Session;
use seal_typeck::TypeckResults;

use crate::query_cache::{self, Cached};

/// The shared declarations computed for one HIR unit.
pub(crate) struct Declarations {
    /// Signatures and extern declarations.
    pub(crate) results: Arc<TypeckResults>,
    /// The query stamp used by body queries.
    pub(crate) dependency: Dependency,
    /// The declarations' semantic fingerprint.
    pub(crate) fingerprint: Fingerprint,
}

/// The shared inputs of every body query in one unit.
pub(crate) struct BodyUnit<'a> {
    /// The session owning the query database.
    pub(crate) session: &'a Session,
    /// Stable identity of the HIR unit.
    pub(crate) unit_key: &'a str,
    /// The unit's HIR.
    pub(crate) hir: &'a Hir,
    /// The HIR query stamp every body depends on.
    pub(crate) hir_dependency: Dependency,
    /// The shared declarations table.
    pub(crate) declarations: &'a Declarations,
}

/// Type-checks the declarations of `hir` once.
pub(crate) fn declarations(
    session: &Session,
    unit_key: &str,
    hir: &Hir,
    hir_dependency: Dependency,
) -> Option<Declarations> {
    let key = QueryKey::new(QueryKind::Signature, format!("declarations:{unit_key}"));
    let output = session.queries().query(key.clone(), [hir_dependency], || {
        let (results, diagnostics) = query_cache::with_diagnostics(session, |dcx| {
            seal_typeck::check_declarations(
                hir,
                dcx,
                session.interner(),
                session.target().pointer_width(),
            )
        });
        let fingerprint = seal_typeck::declarations_fingerprint(&results);
        Ok(QueryOutput::new(
            Cached::new(Arc::new(results), diagnostics),
            fingerprint,
        ))
    });
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            report(session, &error, "declarations");
            return None;
        }
    };
    query_cache::replay(session, output.value());
    let dependency = match session.queries().query_dependency(&key) {
        Ok(dependency) => dependency,
        Err(error) => {
            report(session, &error, "declarations");
            return None;
        }
    };
    Some(Declarations {
        results: Arc::clone(&output.value().value),
        dependency,
        fingerprint: output.fingerprint(),
    })
}

/// Type-checks one non-closure body.
pub(crate) fn typeck_body(
    unit: &BodyUnit<'_>,
    body: BodyId,
) -> Option<(TypeckResults, Dependency)> {
    let session = unit.session;
    let structure = body_hir_fingerprint(session, unit.hir, body);
    let body_input = match session.queries().set_input(
        InputKind::Source,
        format!("body:{}:{}", unit.unit_key, body.index()),
        structure.as_u128().to_be_bytes(),
    ) {
        Ok(input) => input,
        Err(error) => {
            report(session, &error, "typeck-body");
            return None;
        }
    };
    let body_dependency = match session.queries().input_dependency(body_input) {
        Ok(dependency) => dependency,
        Err(error) => {
            report(session, &error, "typeck-body");
            return None;
        }
    };
    let fingerprint = stable_hash_of(&(unit.declarations.fingerprint, structure));
    let key = QueryKey::new(
        QueryKind::TypeckBody,
        format!("typeck-body:{}:{}", unit.unit_key, body.index()),
    );
    let output = session.queries().query(
        key.clone(),
        [
            unit.hir_dependency.clone(),
            unit.declarations.dependency.clone(),
            body_dependency,
        ],
        || {
            let (results, diagnostics) = query_cache::with_diagnostics(session, |dcx| {
                seal_typeck::check_body(
                    unit.hir,
                    dcx,
                    session.interner(),
                    session.target().pointer_width(),
                    &unit.declarations.results,
                    body,
                )
            });
            Ok(QueryOutput::new(
                Cached::new(results, diagnostics),
                fingerprint,
            ))
        },
    );
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            report(session, &error, "typeck-body");
            return None;
        }
    };
    query_cache::replay(session, output.value());
    let dependency = match session.queries().query_dependency(&key) {
        Ok(dependency) => dependency,
        Err(error) => {
            report(session, &error, "typeck-body");
            return None;
        }
    };
    Some((output.value().value.clone(), dependency))
}

/// Lowers one checked body to MIR.
pub(crate) fn mir_body(
    unit: &BodyUnit<'_>,
    body: BodyId,
    typeck: &TypeckResults,
    typeck_dependency: Dependency,
) -> Option<(Body, Dependency)> {
    let session = unit.session;
    let structure = body_hir_fingerprint(session, unit.hir, body);
    let fingerprint = stable_hash_of(&(structure, typeck_dependency.fingerprint()));
    let key = QueryKey::new(
        QueryKind::MirBody,
        format!("mir-body:{}:{}", unit.unit_key, body.index()),
    );
    let output = session.queries().query(
        key.clone(),
        [
            unit.hir_dependency.clone(),
            unit.declarations.dependency.clone(),
            typeck_dependency,
        ],
        || {
            let lowered = seal_mir::lower_body_with_declarations(
                unit.hir,
                typeck,
                &unit.declarations.results,
                &unit.hir.bodies()[body],
            );
            Ok(QueryOutput::new(
                Cached::new(lowered, Vec::new()),
                fingerprint,
            ))
        },
    );
    let output = match output {
        Ok(output) => output,
        Err(error) => {
            report(session, &error, "mir-body");
            return None;
        }
    };
    let dependency = match session.queries().query_dependency(&key) {
        Ok(dependency) => dependency,
        Err(error) => {
            report(session, &error, "mir-body");
            return None;
        }
    };
    Some((output.value().value.clone(), dependency))
}

/// Hashes the HIR nodes that belong to one body, including their arena IDs.
///
/// Source text alone is not enough: HIR arena indices are assigned in lowering
/// order, so editing an earlier body can shift this body's expression IDs even
/// though its text is unchanged. Including the IDs makes a shifted body miss,
/// while an unrelated body edit leaves this fingerprint untouched.
fn body_hir_fingerprint(session: &Session, hir: &Hir, body: BodyId) -> Fingerprint {
    let body_data = &hir.bodies()[body];
    let span = hir.block(body_data.root).span;
    // A persistent session places a new source generation at a new global
    // offset. HIR arena IDs are unaffected by that offset, but span positions
    // are. Normalize them to file-local positions so an unchanged body keeps
    // its fingerprint across generations, while a changed body or a shifted
    // arena index still misses.
    let base = session
        .source_map()
        .file_of(span)
        .and_then(|file| session.source_map().file(file))
        .map_or(0, |file| file.start().to_u32());
    let mut hasher = StableHasher::new();
    hasher.write_u32(body.index());
    hasher.write_u32(body_data.owner.index());
    hasher.write_str(&normalized_debug(&format!("{:?}", body_data.kind), base));
    hasher.write_u32(body_data.root.index());
    for (index, block) in hir.blocks().iter().enumerate() {
        if span.contains_span(block.span) {
            hasher.write_u32(index as u32);
            hasher.write_str(&normalized_debug(&format!("{block:?}"), base));
        }
    }
    for (index, statement) in hir.stmts().iter().enumerate() {
        if span.contains_span(statement.span) {
            hasher.write_u32(index as u32);
            hasher.write_str(&normalized_debug(&format!("{statement:?}"), base));
        }
    }
    for (index, expression) in hir.exprs().iter().enumerate() {
        if span.contains_span(expression.span) {
            hasher.write_u32(index as u32);
            hasher.write_str(&normalized_debug(&format!("{expression:?}"), base));
        }
    }
    for (index, pattern) in hir.patterns().iter().enumerate() {
        if span.contains_span(pattern.span) {
            hasher.write_u32(index as u32);
            hasher.write_str(&normalized_debug(&format!("{pattern:?}"), base));
        }
    }
    hasher.finish()
}

/// Rewrites every span in a debug rendering to file-local positions.
///
/// `Span` renders as `lo..hi`, so both endpoints of each numeric range are
/// shifted by the file's starting position. Local positions are what the body
/// actually contains; the global base changes when a persistent session adds a
/// new source generation.
fn normalized_debug(value: &str, base: u32) -> String {
    if base == 0 {
        return value.to_owned();
    }
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while let Some(relative) = value[cursor..].find("..") {
        let dots = cursor + relative;
        let mut start = dots;
        while start > cursor && value.as_bytes()[start - 1].is_ascii_digit() {
            start -= 1;
        }
        let mut end = dots + 2;
        while end < value.len() && value.as_bytes()[end].is_ascii_digit() {
            end += 1;
        }
        output.push_str(&value[cursor..start]);
        if start < dots
            && end > dots + 2
            && let (Ok(lo), Ok(hi)) = (
                value[start..dots].parse::<u32>(),
                value[dots + 2..end].parse::<u32>(),
            )
        {
            output.push_str(&lo.saturating_sub(base).to_string());
            output.push_str("..");
            output.push_str(&hi.saturating_sub(base).to_string());
        } else {
            output.push_str(&value[start..end]);
        }
        cursor = end;
    }
    output.push_str(&value[cursor..]);
    output
}

fn report(session: &Session, error: &QueryError, phase: &str) {
    session
        .diag()
        .bug(format!("the {phase} query failed: {error}"));
}

#[cfg(test)]
mod tests {
    use super::normalized_debug;

    #[test]
    fn debug_spans_become_file_local() {
        assert_eq!(
            normalized_debug("Block { span: 74..79, tail: 76..77 }", 54),
            "Block { span: 20..25, tail: 22..23 }"
        );
        assert_eq!(
            normalized_debug("value without a span", 54),
            "value without a span"
        );
        assert_eq!(normalized_debug("Name(7)..(9)", 2), "Name(7)..(9)");
    }
}
