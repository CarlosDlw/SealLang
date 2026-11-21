//! Measures invalidation counts and time for a body/signature/feature graph.
//!
//! This is a deterministic model of the compiler query graph, not a claim about
//! the whole compiler. It validates that a body edit recomputes only the body
//! chain while a signature edit fans out, and it reports the measured costs of
//! the database itself.

use std::sync::Arc;
use std::time::{Duration, Instant};

use seal_hash::{Fingerprint, StableHasher};
use seal_query::{Dependency, InputId, InputKind, QueryDb, QueryKey, QueryKind, QueryOutput};

const ITERATIONS: usize = 500;

fn fingerprint(text: &str) -> Fingerprint {
    let mut hasher = StableHasher::new();
    hasher.write_str(text);
    hasher.finish()
}

fn input_dep(db: &QueryDb, input: InputId) -> Dependency {
    db.input_dependency(input).expect("input dependency")
}

fn query_dep(db: &QueryDb, kind: QueryKind, name: &str) -> Dependency {
    db.query_dependency(&QueryKey::new(kind, name))
        .expect("query dependency")
}

fn query_text(
    db: &QueryDb,
    kind: QueryKind,
    name: &str,
    dependencies: Vec<Dependency>,
    value: &str,
) -> Arc<String> {
    let output = db
        .query(QueryKey::new(kind, name), dependencies, || {
            Ok(QueryOutput::new(value.to_owned(), fingerprint(value)))
        })
        .expect("query value");
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
        .expect("query input");
    Arc::clone(output.value())
}

fn run_graph(db: &QueryDb, decls: InputId, a: InputId, b: InputId) {
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
        |text| format!("body-a:{}", strip_comments(text)),
    );
    let checked_a = query_text(
        db,
        QueryKind::TypeckBody,
        "app::run",
        vec![
            query_dep(db, QueryKind::Signature, "app::run"),
            query_dep(db, QueryKind::BodyHir, "app::run"),
        ],
        &format!("{signature}|{body_a}"),
    );
    let mir_a = query_text(
        db,
        QueryKind::MirBody,
        "app::run",
        vec![query_dep(db, QueryKind::TypeckBody, "app::run")],
        &format!("mir|{checked_a}"),
    );
    let mono_a = query_text(
        db,
        QueryKind::MonoInstance,
        "app::run",
        vec![query_dep(db, QueryKind::MirBody, "app::run")],
        &format!("mono|{mir_a}"),
    );
    query_text(
        db,
        QueryKind::CodegenCgu,
        "cgu0",
        vec![query_dep(db, QueryKind::MonoInstance, "app::run")],
        &format!("cgu|{mono_a}"),
    );
    let body_b = query_input(
        db,
        QueryKind::BodyHir,
        "app::helper",
        vec![input_dep(db, decls), input_dep(db, b)],
        b,
        |text| format!("body-b:{}", strip_comments(text)),
    );
    let checked_b = query_text(
        db,
        QueryKind::TypeckBody,
        "app::helper",
        vec![
            query_dep(db, QueryKind::Signature, "app::run"),
            query_dep(db, QueryKind::BodyHir, "app::helper"),
        ],
        &format!("{signature}|{body_b}"),
    );
    query_text(
        db,
        QueryKind::CodegenCgu,
        "cgu1",
        vec![query_dep(db, QueryKind::TypeckBody, "app::helper")],
        &format!("cgu|{checked_b}"),
    );
}

fn strip_comments(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn direct_graph(decls: &str, a: &str, b: &str) {
    let signature = format!("sig:{decls}");
    let body_a = format!("body-a:{a}");
    let body_b = format!("body-b:{b}");
    let checked_a = format!("{signature}|{body_a}");
    let checked_b = format!("{signature}|{body_b}");
    let _mir = format!("mir|{checked_a}");
    let _mono = format!("mono|{checked_a}");
    let _cgu_a = format!("cgu|{checked_a}");
    let _cgu_b = format!("cgu|{checked_b}");
}

fn per_iteration(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1_000_000.0 / ITERATIONS as f64
}

fn misses(stats: &seal_query::QueryStats, kind: QueryKind) -> u64 {
    stats.by_kind.get(&kind).map_or(0, |kind| kind.misses)
}

fn main() {
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
    run_graph(&db, decls, a, b);

    let direct_start = Instant::now();
    for _ in 0..ITERATIONS {
        direct_graph("fn run; fn helper", "x + 1", "y + 2");
    }
    let direct = per_iteration(direct_start.elapsed());

    db.reset_stats();
    let noop_start = Instant::now();
    for _ in 0..ITERATIONS {
        run_graph(&db, decls, a, b);
    }
    let noop = per_iteration(noop_start.elapsed());
    let noop_stats = db.stats();

    db.reset_stats();
    let body_start = Instant::now();
    for index in 0..ITERATIONS {
        db.set_input(InputKind::Source, "src/run.sl", format!("x + {index}"))
            .expect("edited body");
        run_graph(&db, decls, a, b);
    }
    let body = per_iteration(body_start.elapsed());
    let body_stats = db.stats();

    db.reset_stats();
    let signature_start = Instant::now();
    for index in 0..ITERATIONS {
        db.set_input(
            InputKind::Source,
            "src/app.sl",
            format!("fn run; fn helper; const V: i32 = {index}"),
        )
        .expect("edited signature");
        run_graph(&db, decls, a, b);
    }
    let signature = per_iteration(signature_start.elapsed());
    let signature_stats = db.stats();

    db.set_input(InputKind::Source, "src/run.sl", b"x + 1")
        .expect("reset body");
    run_graph(&db, decls, a, b);
    db.reset_stats();
    let backdate_start = Instant::now();
    for index in 0..ITERATIONS {
        db.set_input(
            InputKind::Source,
            "src/run.sl",
            format!("// comment {index}\nx + 1"),
        )
        .expect("commented body");
        run_graph(&db, decls, a, b);
    }
    let backdate = per_iteration(backdate_start.elapsed());
    let backdate_stats = db.stats();

    let features = db
        .set_input(InputKind::Features, "app", b"serde")
        .expect("features");
    query_text(
        &db,
        QueryKind::ModuleItems,
        "app",
        vec![input_dep(&db, features)],
        "serde",
    );
    db.reset_stats();
    for index in 0..ITERATIONS {
        db.set_input(InputKind::Features, "app", format!("serde{index}"))
            .expect("edited features");
        query_text(
            &db,
            QueryKind::ModuleItems,
            "app",
            vec![input_dep(&db, features)],
            &format!("serde{index}"),
        );
    }
    let feature_stats = db.stats();

    let cfg = db
        .set_input(InputKind::Cfg, "app", b"target_os=linux")
        .expect("cfg");
    query_text(
        &db,
        QueryKind::ModuleItems,
        "cfg-profile",
        vec![input_dep(&db, cfg)],
        "linux",
    );
    db.reset_stats();
    for index in 0..ITERATIONS {
        db.set_input(InputKind::Cfg, "app", format!("target_os=linux{index}"))
            .expect("edited cfg");
        query_text(
            &db,
            QueryKind::ModuleItems,
            "cfg-profile",
            vec![input_dep(&db, cfg)],
            &format!("linux{index}"),
        );
    }
    let cfg_stats = db.stats();

    let constant = db
        .set_input(InputKind::Source, "src/const.sl", b"const N: i32 = 1")
        .expect("const");
    query_text(
        &db,
        QueryKind::MonoInstance,
        "const-value",
        vec![input_dep(&db, constant)],
        "1",
    );
    db.reset_stats();
    for index in 0..ITERATIONS {
        db.set_input(
            InputKind::Source,
            "src/const.sl",
            format!("const N: i32 = {index}"),
        )
        .expect("edited const");
        query_text(
            &db,
            QueryKind::MonoInstance,
            "const-value",
            vec![input_dep(&db, constant)],
            &format!("{index}"),
        );
    }
    let const_stats = db.stats();

    println!("queries direct_graph_us={direct:.3}");
    println!("queries noop_us={noop:.3} misses={}", noop_stats.misses);
    println!(
        "queries body_edit_us={body:.3} misses={} typeck={} mir={} mono={} cgu={}",
        body_stats.misses,
        misses(&body_stats, QueryKind::TypeckBody),
        misses(&body_stats, QueryKind::MirBody),
        misses(&body_stats, QueryKind::MonoInstance),
        misses(&body_stats, QueryKind::CodegenCgu)
    );
    println!(
        "queries signature_edit_us={signature:.3} misses={} typeck={} cgu={}",
        signature_stats.misses,
        misses(&signature_stats, QueryKind::TypeckBody),
        misses(&signature_stats, QueryKind::CodegenCgu)
    );
    println!(
        "queries backdate_us={backdate:.3} misses={} backdated={}",
        backdate_stats.misses, backdate_stats.backdated
    );
    println!("queries feature_edit_misses={}", feature_stats.misses);
    println!("queries cfg_edit_misses={}", cfg_stats.misses);
    println!("queries const_edit_misses={}", const_stats.misses);

    assert_eq!(noop_stats.misses, 0);
    assert_eq!(body_stats.misses, 5 * ITERATIONS as u64);
    assert_eq!(misses(&body_stats, QueryKind::Signature), 0);
    assert_eq!(misses(&body_stats, QueryKind::MirBody), ITERATIONS as u64);
    assert_eq!(feature_stats.misses, ITERATIONS as u64);
    assert_eq!(cfg_stats.misses, ITERATIONS as u64);
    assert_eq!(const_stats.misses, ITERATIONS as u64);
    assert_eq!(signature_stats.misses, 9 * ITERATIONS as u64);
    assert_eq!(backdate_stats.misses, ITERATIONS as u64);
    assert_eq!(backdate_stats.backdated, ITERATIONS as u64);
}
