//! Query-backed frontend phases.
//!
//! Lexing and parsing are pure functions of the source text and target options.
//! Lowering is pure after parsing and target selection. Routing them through
//! [`seal_query::QueryDb`] gives the session a single place to validate content
//! fingerprints and to record whether a phase ran or was reused.
//!
//! Lowering a body is not part of this module: HIR lowering keeps the complete
//! body arena for its unit. The queries here return a whole HIR value, but its
//! fingerprint covers declarations only. A body edit recomputes the HIR value
//! (and therefore visible spans and IDs) while backdating the query, so body
//! queries that do not read the edited span stay valid.
//!
//! Diagnostics travel with each cached value. A query failure is a compiler
//! bug, not a user diagnostic, and is reported as an ICE.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use seal_ast::Ast;
use seal_hash::{Fingerprint, StableHasher};
use seal_hir::{Hir, PackagePlan};
use seal_lexer::Lexed;
use seal_query::{Dependency, InputKind, QueryError, QueryKey, QueryKind, QueryOutput};
use seal_session::Session;
use seal_span::{FileId, SourceFile};

use crate::query_cache::{self, Cached};

/// A lowered HIR value and the dependency stamp that names it.
pub(crate) struct Lowered {
    /// The shared HIR value.
    pub(crate) hir: Arc<Mutex<Hir>>,
    /// The query stamp for use as a dependency of later phases.
    pub(crate) dependency: Dependency,
}

/// A source file's stable key inside one session.
///
/// The file index distinguishes two package instances that load the same path
/// as separate compilation units. It is not a filesystem identity and is never
/// used as an invalidation timestamp.
fn source_key(file_id: FileId, file: &SourceFile) -> String {
    format!("{}#{}", file.name(), file_id.index())
}

pub(crate) fn input_dependency(
    session: &Session,
    file_id: FileId,
    file: &SourceFile,
) -> Result<(String, Dependency), QueryError> {
    let key = source_key(file_id, file);
    let input =
        session
            .queries()
            .set_input(InputKind::Source, key.clone(), file.text().as_bytes())?;
    let dependency = session.queries().input_dependency(input)?;
    Ok((key, dependency))
}

fn tokens_key(source: &str) -> QueryKey {
    QueryKey::new(QueryKind::Tokens, source)
}

fn ast_key(source: &str) -> QueryKey {
    QueryKey::new(QueryKind::Ast, source)
}

/// Returns the dependency stamp of a parsed file.
pub(crate) fn ast_dependency(
    session: &Session,
    file_id: FileId,
    file: &SourceFile,
) -> Result<Dependency, QueryError> {
    let source = source_key(file_id, file);
    session.queries().query_dependency(&ast_key(&source))
}

/// Lexes `file` through the session query database.
pub(crate) fn lex(session: &Session, file_id: FileId, file: &SourceFile) -> Option<Arc<Lexed>> {
    let (source, input) = match input_dependency(session, file_id, file) {
        Ok(value) => value,
        Err(error) => {
            report(session, &error, "lex");
            return None;
        }
    };
    let fingerprint = file.content_hash();
    let output = session.queries().query(tokens_key(&source), [input], || {
        let (lexed, diagnostics) = query_cache::with_diagnostics(session, |dcx| {
            seal_lexer::lex(file, dcx, session.interner())
        });
        Ok(QueryOutput::new(
            Cached::new(Arc::new(lexed), diagnostics),
            fingerprint,
        ))
    });
    match output {
        Ok(output) => {
            query_cache::replay(session, output.value());
            Some(Arc::clone(&output.value().value))
        }
        Err(error) => {
            report(session, &error, "lex");
            None
        }
    }
}

/// Parses a lexed file through the session query database.
pub(crate) fn parse(
    session: &Session,
    file_id: FileId,
    file: &SourceFile,
    tokens: &Lexed,
) -> Option<Arc<Ast>> {
    let (source, input) = match input_dependency(session, file_id, file) {
        Ok(value) => value,
        Err(error) => {
            report(session, &error, "parse");
            return None;
        }
    };
    let token_query = match session.queries().query_dependency(&tokens_key(&source)) {
        Ok(dependency) => dependency,
        Err(error) => {
            report(session, &error, "parse");
            return None;
        }
    };
    let fingerprint = file.content_hash();
    let output = session
        .queries()
        .query(ast_key(&source), [input, token_query], || {
            let (ast, diagnostics) = query_cache::with_diagnostics(session, |dcx| {
                seal_parser::parse(tokens, dcx, session.interner())
            });
            Ok(QueryOutput::new(
                Cached::new(Arc::new(ast), diagnostics),
                fingerprint,
            ))
        });
    match output {
        Ok(output) => {
            query_cache::replay(session, output.value());
            Some(Arc::clone(&output.value().value))
        }
        Err(error) => {
            report(session, &error, "parse");
            None
        }
    }
}

/// Lowers a complete package through the session query database.
pub(crate) fn lower_package(
    session: &Session,
    unit_key: &str,
    plan: &PackagePlan,
    loaded: &[(FileId, Ast)],
    dependencies: impl IntoIterator<Item = Dependency>,
) -> Option<Lowered> {
    let name = format!("hir-package:{}:{unit_key}", session.target().triple());
    query_hir(session, &name, dependencies, || {
        query_cache::with_diagnostics(session, |dcx| {
            seal_hir::lower_package(
                plan,
                loaded,
                dcx,
                session.interner(),
                session.target().c_abi(),
            )
        })
    })
}

fn query_hir<F>(
    session: &Session,
    name: &str,
    dependencies: impl IntoIterator<Item = Dependency>,
    compute: F,
) -> Option<Lowered>
where
    F: FnOnce() -> (Hir, Vec<seal_diagnostics::Diagnostic>),
{
    let key = QueryKey::new(QueryKind::ModuleItems, name);
    let output = session.queries().query(key.clone(), dependencies, || {
        let (hir, diagnostics) = compute();
        let fingerprint = declaration_source_fingerprint(session, &hir);
        Ok(QueryOutput::new(
            Cached::new(Arc::new(Mutex::new(hir)), diagnostics),
            fingerprint,
        ))
    });
    match output {
        Ok(output) => {
            query_cache::replay(session, output.value());
            let dependency = match session.queries().query_dependency(&key) {
                Ok(dependency) => dependency,
                Err(error) => {
                    report(session, &error, "hir");
                    return None;
                }
            };
            Some(Lowered {
                hir: Arc::clone(&output.value().value),
                dependency,
            })
        }
        Err(error) => {
            report(session, &error, "hir");
            None
        }
    }
}

/// Returns a fingerprint of every declaration in `hir`, excluding body text.
///
/// Body content is removed from each source file before hashing. The remaining
/// text includes signatures, generic bounds, ADT fields, impl and trait
/// headers, and module declarations. A body edit therefore changes the HIR
/// value and its body spans but not this fingerprint.
pub(crate) fn declaration_source_fingerprint(session: &Session, hir: &Hir) -> Fingerprint {
    let mut ranges: Vec<(FileId, Range<u32>)> = Vec::new();
    for body in hir.bodies() {
        let span = hir.block(body.root).span;
        let Some(snippet) = session.source_map().snippet(span) else {
            continue;
        };
        if let Some(file) = session.source_map().file_of(span) {
            ranges.push((file, snippet.range()));
        }
    }
    ranges.sort_by_key(|(file, range)| (*file, range.start));

    // A persistent session keeps previous generations of an edited file so old
    // spans stay valid. Only files this HIR actually references are part of its
    // fingerprint; hashing every file in the map would make each edit change
    // the declaration fingerprint through the stale generation alone.
    let mut files = hir.source_files();
    // File identifiers depend on load order, and the standard library is loaded
    // before the user sources on some paths. Sorting by the stable display name
    // keeps the declaration fingerprint identical across equivalent runs.
    files.sort_by_key(|file_id| {
        session
            .source_map()
            .display_name(*file_id)
            .unwrap_or_default()
    });
    let mut hasher = StableHasher::new();
    hasher.write_usize(files.len());
    for file_id in files {
        let Some(file) = session.source_map().file(file_id) else {
            continue;
        };
        let mut body_ranges: Vec<Range<u32>> = ranges
            .iter()
            .filter(|(id, _)| *id == file_id)
            .map(|(_, range)| range.clone())
            .collect();
        body_ranges.sort_by_key(|range| range.start);
        let text = file.text();
        let mut cursor = 0usize;
        for range in body_ranges {
            let start = range.start as usize;
            let end = (range.end as usize).min(text.len());
            if start > cursor {
                hasher.write_bytes(&text.as_bytes()[cursor..start]);
            }
            cursor = cursor.max(end);
        }
        if cursor < text.len() {
            hasher.write_bytes(&text.as_bytes()[cursor..]);
        }
        if let Some(name) = session.source_map().display_name(file_id) {
            hasher.write_str(&name);
        }
    }
    hasher.finish()
}

fn report(session: &Session, error: &QueryError, phase: &str) {
    session
        .diag()
        .bug(format!("the {phase} query failed: {error}"));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use seal_diagnostics::SilentEmitter;
    use seal_query::QueryKind;
    use seal_session::{Options, Session};
    use seal_span::FileName;

    use seal_hir::PackagePlan;

    use super::{ast_dependency, lex, lower_package, parse};

    #[test]
    fn repeated_frontend_requests_hit_the_queries() {
        let session =
            Session::with_emitter(Options::default(), Some(Box::new(SilentEmitter::default())))
                .expect("session");
        let file_id = session
            .add_file(
                FileName::virtual_name("memo.sl"),
                "func main() -> i32 { 0 }".to_owned(),
            )
            .expect("source");
        let file = session.source_map().file(file_id).expect("source file");

        let first_tokens = lex(&session, file_id, &file).expect("first lex");
        let second_tokens = lex(&session, file_id, &file).expect("second lex");
        assert!(Arc::ptr_eq(&first_tokens, &second_tokens));

        let first_ast = parse(&session, file_id, &file, &first_tokens).expect("first parse");
        let second_ast = parse(&session, file_id, &file, &second_tokens).expect("second parse");
        assert!(Arc::ptr_eq(&first_ast, &second_ast));

        let plan = PackagePlan::single(session.interner().intern("main"), file_id);
        let dependency =
            ast_dependency(&session, file_id, &file).expect("the parse dependency exists");
        let loaded = |ast: Arc<seal_ast::Ast>| vec![(file_id, Arc::unwrap_or_clone(ast))];
        let first_hir = lower_package(
            &session,
            "memo",
            &plan,
            &loaded(first_ast),
            [dependency.clone()],
        )
        .expect("first hir");
        let second_hir = lower_package(&session, "memo", &plan, &loaded(second_ast), [dependency])
            .expect("second hir");
        assert!(Arc::ptr_eq(&first_hir.hir, &second_hir.hir));

        let stats = session.queries().stats();
        assert_eq!(stats.by_kind[&QueryKind::Tokens].misses, 1);
        assert_eq!(stats.by_kind[&QueryKind::Tokens].hits, 1);
        assert_eq!(stats.by_kind[&QueryKind::Ast].misses, 1);
        assert_eq!(stats.by_kind[&QueryKind::Ast].hits, 1);
        assert_eq!(stats.by_kind[&QueryKind::ModuleItems].misses, 1);
        assert_eq!(stats.by_kind[&QueryKind::ModuleItems].hits, 1);
    }
}
