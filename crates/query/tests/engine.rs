//! Incremental invalidation and backdating tests.

use std::sync::Arc;

use seal_hash::{Fingerprint, StableHasher};
use seal_query::{
    Dependency, InputId, InputKind, QueryDb, QueryError, QueryKey, QueryKind, QueryOutput,
};

fn fingerprint(text: &str) -> Fingerprint {
    let mut hasher = StableHasher::new();
    hasher.write_str(text);
    hasher.finish()
}

fn input_dep(db: &QueryDb, input: InputId) -> Dependency {
    db.input_dependency(input).expect("input exists")
}

fn query_dep(db: &QueryDb, kind: QueryKind, name: &str) -> Dependency {
    db.query_dependency(&QueryKey::new(kind, name))
        .expect("dependency exists")
}

fn query_value(
    db: &QueryDb,
    kind: QueryKind,
    name: &str,
    dependencies: Vec<Dependency>,
    value: String,
) -> Arc<String> {
    let output = db
        .query(QueryKey::new(kind, name), dependencies, || {
            Ok(QueryOutput::new(value.clone(), fingerprint(&value)))
        })
        .expect("query succeeds");
    Arc::clone(output.value())
}

fn query_input(
    db: &QueryDb,
    kind: QueryKind,
    name: &str,
    dependencies: Vec<Dependency>,
    input: InputId,
    transform: impl FnOnce(&str) -> String,
) -> Arc<String> {
    let output = db
        .query(QueryKey::new(kind, name), dependencies, || {
            let bytes = db.input_bytes(input)?;
            let text = String::from_utf8_lossy(&bytes);
            let value = transform(&text);
            Ok(QueryOutput::new(value.clone(), fingerprint(&value)))
        })
        .expect("query succeeds");
    Arc::clone(output.value())
}

struct BodyGraph {
    signature: Arc<String>,
    body_a: Arc<String>,
    typeck_a: Arc<String>,
    mir_a: Arc<String>,
    mono_a: Arc<String>,
    cgu_a: Arc<String>,
    body_b: Arc<String>,
    typeck_b: Arc<String>,
    cgu_b: Arc<String>,
}

fn run_body_graph(db: &QueryDb, decls: InputId, a: InputId, b: InputId) -> BodyGraph {
    let signature = query_input(
        db,
        QueryKind::Signature,
        "app::run",
        vec![input_dep(db, decls)],
        decls,
        |text| format!("sig:{text}"),
    );
    let body_a = query_input(
        db,
        QueryKind::BodyHir,
        "app::run",
        vec![input_dep(db, decls), input_dep(db, a)],
        a,
        |text| format!("body-a:{text}"),
    );
    let typeck_a = query_value(
        db,
        QueryKind::TypeckBody,
        "app::run",
        vec![
            query_dep(db, QueryKind::Signature, "app::run"),
            query_dep(db, QueryKind::BodyHir, "app::run"),
        ],
        format!("{signature}|{body_a}"),
    );
    let mir_a = query_value(
        db,
        QueryKind::MirBody,
        "app::run",
        vec![query_dep(db, QueryKind::TypeckBody, "app::run")],
        format!("mir|{typeck_a}"),
    );
    let mono_a = query_value(
        db,
        QueryKind::MonoInstance,
        "app::run",
        vec![query_dep(db, QueryKind::MirBody, "app::run")],
        format!("mono|{mir_a}"),
    );
    let cgu_a = query_value(
        db,
        QueryKind::CodegenCgu,
        "cgu0",
        vec![query_dep(db, QueryKind::MonoInstance, "app::run")],
        format!("cgu|{mono_a}"),
    );
    let body_b = query_input(
        db,
        QueryKind::BodyHir,
        "app::helper",
        vec![input_dep(db, decls), input_dep(db, b)],
        b,
        |text| format!("body-b:{text}"),
    );
    let typeck_b = query_value(
        db,
        QueryKind::TypeckBody,
        "app::helper",
        vec![
            query_dep(db, QueryKind::Signature, "app::run"),
            query_dep(db, QueryKind::BodyHir, "app::helper"),
        ],
        format!("{signature}|{body_b}"),
    );
    let cgu_b = query_value(
        db,
        QueryKind::CodegenCgu,
        "cgu1",
        vec![query_dep(db, QueryKind::TypeckBody, "app::helper")],
        format!("cgu|{typeck_b}"),
    );
    BodyGraph {
        signature,
        body_a,
        typeck_a,
        mir_a,
        mono_a,
        cgu_a,
        body_b,
        typeck_b,
        cgu_b,
    }
}

#[test]
fn editing_a_body_reuses_signature_and_sibling_body() {
    let db = QueryDb::new();
    let decls = db
        .set_input(InputKind::Source, "src/app.sl", b"fn run; fn helper")
        .expect("declarations");
    let a = db
        .set_input(InputKind::Source, "src/run.sl", b"x + 1")
        .expect("body a");
    let b = db
        .set_input(InputKind::Source, "src/helper.sl", b"y + 2")
        .expect("body b");
    run_body_graph(&db, decls, a, b);
    db.reset_stats();

    db.set_input(InputKind::Source, "src/run.sl", b"x + 10")
        .expect("edited body");
    let graph = run_body_graph(&db, decls, a, b);
    let stats = db.stats();

    assert_eq!(*graph.signature, "sig:fn run; fn helper");
    assert_eq!(*graph.body_a, "body-a:x + 10");
    assert_eq!(*graph.body_b, "body-b:y + 2");
    assert_eq!(stats.by_kind[&QueryKind::Signature].misses, 0);
    assert_eq!(stats.by_kind[&QueryKind::Signature].hits, 1);
    assert_eq!(stats.by_kind[&QueryKind::BodyHir].misses, 1);
    assert_eq!(stats.by_kind[&QueryKind::BodyHir].hits, 1);
    assert_eq!(stats.by_kind[&QueryKind::TypeckBody].misses, 1);
    assert_eq!(stats.by_kind[&QueryKind::TypeckBody].hits, 1);
    assert_eq!(stats.by_kind[&QueryKind::MirBody].misses, 1);
    assert_eq!(stats.by_kind[&QueryKind::MonoInstance].misses, 1);
    assert_eq!(stats.by_kind[&QueryKind::CodegenCgu].misses, 1);
    assert_eq!(*graph.typeck_a, "sig:fn run; fn helper|body-a:x + 10");
    assert!(graph.mir_a.contains("body-a:x + 10"));
    assert!(graph.mono_a.contains("body-a:x + 10"));
    assert!(graph.cgu_a.contains("body-a:x + 10"));
    assert_eq!(*graph.typeck_b, "sig:fn run; fn helper|body-b:y + 2");
    assert!(graph.cgu_b.contains("body-b:y + 2"));
}

#[test]
fn editing_a_signature_invalidates_dependent_bodies() {
    let db = QueryDb::new();
    let decls = db
        .set_input(InputKind::Source, "src/app.sl", b"fn run; fn helper")
        .expect("declarations");
    let a = db
        .set_input(InputKind::Source, "src/run.sl", b"x + 1")
        .expect("body a");
    let b = db
        .set_input(InputKind::Source, "src/helper.sl", b"y + 2")
        .expect("body b");
    run_body_graph(&db, decls, a, b);
    db.reset_stats();

    db.set_input(InputKind::Source, "src/app.sl", b"fn run; fn helper; use a")
        .expect("edited declarations");
    let graph = run_body_graph(&db, decls, a, b);
    let stats = db.stats();

    assert!(graph.signature.contains("use a"));
    assert_eq!(stats.by_kind[&QueryKind::Signature].misses, 1);
    assert_eq!(stats.by_kind[&QueryKind::BodyHir].misses, 2);
    assert_eq!(stats.by_kind[&QueryKind::BodyHir].backdated, 2);
    assert_eq!(stats.by_kind[&QueryKind::TypeckBody].misses, 2);
    assert_eq!(stats.by_kind[&QueryKind::CodegenCgu].misses, 2);
}

#[test]
fn equal_result_is_backdated_and_stops_propagation() {
    let db = QueryDb::new();
    let decls = db
        .set_input(InputKind::Source, "src/app.sl", b"fn run; fn helper")
        .expect("declarations");
    let a = db
        .set_input(InputKind::Source, "src/run.sl", b"x + 1")
        .expect("body a");
    let b = db
        .set_input(InputKind::Source, "src/helper.sl", b"y + 2")
        .expect("body b");
    run_body_graph(&db, decls, a, b);
    db.reset_stats();

    db.set_input(InputKind::Source, "src/run.sl", b"// comment\nx + 1")
        .expect("commented body");
    // The model normalization removes comments before computing the fingerprint,
    // which is what a real parser does for trivia.
    let body = query_input(
        &db,
        QueryKind::BodyHir,
        "app::run",
        vec![input_dep(&db, decls), input_dep(&db, a)],
        a,
        |text| format!("body-a:{}", text.replace("// comment\n", "")),
    );
    let graph = run_body_graph(&db, decls, a, b);
    let stats = db.stats();

    assert_eq!(*body, "body-a:x + 1");
    assert_eq!(*graph.typeck_a, "sig:fn run; fn helper|body-a:x + 1");
    assert_eq!(stats.by_kind[&QueryKind::BodyHir].misses, 1);
    assert_eq!(stats.by_kind[&QueryKind::BodyHir].backdated, 1);
    assert_eq!(stats.by_kind[&QueryKind::TypeckBody].misses, 0);
    assert_eq!(stats.by_kind[&QueryKind::MirBody].misses, 0);
    assert_eq!(stats.by_kind[&QueryKind::MonoInstance].misses, 0);
    assert_eq!(stats.by_kind[&QueryKind::CodegenCgu].misses, 0);
}

#[test]
fn features_and_cfg_are_inputs_and_do_not_reuse_old_results() {
    let db = QueryDb::new();
    let features = db
        .set_input(InputKind::Features, "app", b"serde")
        .expect("features");
    let cfg = db
        .set_input(InputKind::Cfg, "app", b"target_os=linux")
        .expect("cfg");
    let key = QueryKey::new(QueryKind::ModuleItems, "app");
    let output = db
        .query(
            key.clone(),
            vec![input_dep(&db, features), input_dep(&db, cfg)],
            || Ok(QueryOutput::new("serde".to_owned(), fingerprint("serde"))),
        )
        .expect("module query");
    assert_eq!(output.value().as_str(), "serde");
    db.reset_stats();

    db.set_input(InputKind::Features, "app", b"serde,backtrace")
        .expect("new features");
    let output = db
        .query(
            key,
            vec![input_dep(&db, features), input_dep(&db, cfg)],
            || {
                Ok(QueryOutput::new(
                    "serde,backtrace".to_owned(),
                    fingerprint("serde,backtrace"),
                ))
            },
        )
        .expect("updated module query");
    assert_eq!(output.value().as_str(), "serde,backtrace");
    assert_eq!(db.stats().by_kind[&QueryKind::ModuleItems].misses, 1);

    db.set_input(InputKind::Cfg, "app", b"target_os=windows")
        .expect("new cfg");
    let output = db
        .query(
            QueryKey::new(QueryKind::ModuleItems, "app"),
            vec![input_dep(&db, features), input_dep(&db, cfg)],
            || {
                Ok(QueryOutput::new(
                    "serde,backtrace,windows".to_owned(),
                    fingerprint("serde,backtrace,windows"),
                ))
            },
        )
        .expect("updated cfg query");
    assert_eq!(output.value().as_str(), "serde,backtrace,windows");
    assert_eq!(db.stats().by_kind[&QueryKind::ModuleItems].misses, 2);
}

#[test]
fn installing_identical_input_does_not_advance_the_revision() {
    let db = QueryDb::new();
    let source = db
        .set_input(InputKind::Source, "src/main.sl", b"func main() {}")
        .expect("source");
    let revision = db.revision();
    db.set_input(InputKind::Source, "src/main.sl", b"func main() {}")
        .expect("same source");
    assert_eq!(db.revision(), revision);
    assert_eq!(db.stats().input_changes, 1);
    assert_eq!(
        String::from_utf8(db.input_bytes(source).expect("source").to_vec()).expect("text"),
        "func main() {}"
    );
}

#[test]
fn missing_dependencies_and_type_mismatches_are_errors() {
    let db = QueryDb::new();
    let key = QueryKey::new(QueryKind::Tokens, "missing");
    assert_eq!(
        db.query_dependency(&key),
        Err(QueryError::MissingDependency(key.clone()))
    );
    db.query(key.clone(), [], || {
        Ok(QueryOutput::new(1_u32, fingerprint("one")))
    })
    .expect("numeric query");
    assert!(matches!(
        db.query(key, [], || Ok(QueryOutput::new(
            String::from("text"),
            fingerprint("text")
        ))),
        Err(QueryError::TypeMismatch { .. })
    ));
    assert!(matches!(
        db.query::<String, _>(QueryKey::new(QueryKind::Ast, "bad"), [], || {
            Ok(QueryOutput::new(String::new(), Fingerprint::ZERO))
        }),
        Err(QueryError::InvalidFingerprint { .. })
    ));
}

#[test]
fn recursive_queries_report_a_cycle_instead_of_deadlocking() {
    let db = QueryDb::new();
    let key = QueryKey::new(QueryKind::Ast, "loop");
    let result = db.query::<String, _>(key.clone(), [], || {
        db.query::<String, _>(key.clone(), [], || {
            Ok(QueryOutput::new("inner".to_owned(), fingerprint("inner")))
        })?;
        Ok(QueryOutput::new("outer".to_owned(), fingerprint("outer")))
    });
    assert!(matches!(result, Err(QueryError::Cycle { .. })));
    assert!(db.is_empty());
}
