//! Query-backed frontend phases.
//!
//! Lexing and parsing are pure functions of the source text and the target
//! options. Routing them through [`seal_query::QueryDb`] gives the session a
//! single place to validate content fingerprints and record whether a phase ran
//! or was reused. Lowering a single-file program is queried too; package plans
//! keep their existing multi-file lowering because the query value would be one
//! package-wide HIR rather than a per-file body.
//!
//! A query failure is a compiler bug, not a user diagnostic. It is reported as
//! an ICE so a corrupt internal dependency can never be silently ignored.

use std::sync::{Arc, Mutex};

use seal_ast::Ast;
use seal_hir::Hir;
use seal_lexer::Lexed;
use seal_query::{InputKind, QueryError, QueryKey, QueryKind, QueryOutput};
use seal_session::Session;
use seal_span::{FileId, SourceFile};

/// A source file's stable key inside one session.
///
/// The file index distinguishes two package instances that load the same path
/// as separate compilation units. It is not a filesystem identity and is never
/// used as an invalidation timestamp.
fn source_key(file_id: FileId, file: &SourceFile) -> String {
    format!("{}#{}", file.name(), file_id.index())
}

fn input_dependency(
    session: &Session,
    file_id: FileId,
    file: &SourceFile,
) -> Result<(String, seal_query::Dependency), QueryError> {
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
        Ok(QueryOutput::new(
            seal_lexer::lex(file, session.diag(), session.interner()),
            fingerprint,
        ))
    });
    match output {
        Ok(output) => Some(Arc::clone(output.value())),
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
            Ok(QueryOutput::new(
                seal_parser::parse(tokens, session.diag(), session.interner()),
                fingerprint,
            ))
        });
    match output {
        Ok(output) => Some(Arc::clone(output.value())),
        Err(error) => {
            report(session, &error, "parse");
            None
        }
    }
}

/// Lowers a single-file program through the session query database.
pub(crate) fn lower_single(
    session: &Session,
    file_id: FileId,
    file: &SourceFile,
    ast: &Ast,
) -> Option<Arc<Mutex<Hir>>> {
    let (source, input) = match input_dependency(session, file_id, file) {
        Ok(value) => value,
        Err(error) => {
            report(session, &error, "hir");
            return None;
        }
    };
    let ast_query = match session.queries().query_dependency(&ast_key(&source)) {
        Ok(dependency) => dependency,
        Err(error) => {
            report(session, &error, "hir");
            return None;
        }
    };
    let name = format!("hir:{}:{}", session.target().triple(), source);
    let fingerprint = file.content_hash();
    let output = session.queries().query(
        QueryKey::new(QueryKind::ModuleItems, name),
        [input, ast_query],
        || {
            Ok(QueryOutput::new(
                Mutex::new(seal_hir::lower_with_c_abi(
                    ast,
                    session.diag(),
                    session.interner(),
                    session.target().c_abi(),
                )),
                fingerprint,
            ))
        },
    );
    match output {
        Ok(output) => Some(Arc::clone(output.value())),
        Err(error) => {
            report(session, &error, "hir");
            None
        }
    }
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

    use super::{lex, lower_single, parse};

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

        let first_hir = lower_single(&session, file_id, &file, &first_ast).expect("first hir");
        let second_hir = lower_single(&session, file_id, &file, &second_ast).expect("second hir");
        assert!(Arc::ptr_eq(&first_hir, &second_hir));

        let stats = session.queries().stats();
        assert_eq!(stats.by_kind[&QueryKind::Tokens].misses, 1);
        assert_eq!(stats.by_kind[&QueryKind::Tokens].hits, 1);
        assert_eq!(stats.by_kind[&QueryKind::Ast].misses, 1);
        assert_eq!(stats.by_kind[&QueryKind::Ast].hits, 1);
        assert_eq!(stats.by_kind[&QueryKind::ModuleItems].misses, 1);
        assert_eq!(stats.by_kind[&QueryKind::ModuleItems].hits, 1);
    }
}
