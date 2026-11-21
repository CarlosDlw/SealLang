//! Compares direct lexing/parsing with query-cached frontend phases.

use std::sync::Arc;
use std::time::{Duration, Instant};

use seal_diagnostics::{DiagCtxt, SilentEmitter};
use seal_query::{InputKind, QueryDb, QueryKey, QueryKind, QueryOutput};
use seal_span::{FileName, SourceMap};
use seal_symbol::Interner;

const SOURCE: &str = r#"
public func double(value: i32) -> i32 {
  value * 2
}

func main() -> i32 {
  double(21)
}
"#;

const ITERATIONS: usize = 500;

fn per_iteration(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1_000_000.0 / ITERATIONS as f64
}

fn main() {
    let map = Arc::new(SourceMap::new());
    let file_id = map
        .add_file(FileName::virtual_name("bench.sl"), SOURCE.to_owned())
        .expect("bench source");
    let file = map.file(file_id).expect("registered bench source");
    let dcx = DiagCtxt::new(Box::new(SilentEmitter::default()), Arc::clone(&map));
    let interner = Interner::new();

    let direct_start = Instant::now();
    for _ in 0..ITERATIONS {
        let tokens = seal_lexer::lex(&file, &dcx, &interner);
        let _ast = seal_parser::parse(&tokens, &dcx, &interner);
    }
    let direct = per_iteration(direct_start.elapsed());

    let db = QueryDb::new();
    let input = db
        .set_input(InputKind::Source, "bench.sl", SOURCE.as_bytes())
        .expect("source input");
    let token_key = QueryKey::new(QueryKind::Tokens, "bench.sl");
    let ast_key = QueryKey::new(QueryKind::Ast, "bench.sl");
    let input_dep = db.input_dependency(input).expect("input dependency");
    db.query(token_key.clone(), [input_dep], || {
        Ok(QueryOutput::new(
            seal_lexer::lex(&file, &dcx, &interner),
            file.content_hash(),
        ))
    })
    .expect("warm token query");
    let token_dep = db.query_dependency(&token_key).expect("token dependency");
    let tokens = seal_lexer::lex(&file, &dcx, &interner);
    db.query(ast_key.clone(), [token_dep], || {
        Ok(QueryOutput::new(
            seal_parser::parse(&tokens, &dcx, &interner),
            file.content_hash(),
        ))
    })
    .expect("warm AST query");
    db.reset_stats();

    let cached_start = Instant::now();
    for _ in 0..ITERATIONS {
        let input = db.input_dependency(input).expect("input dependency");
        db.query(token_key.clone(), [input], || {
            Ok(QueryOutput::new(
                seal_lexer::lex(&file, &dcx, &interner),
                file.content_hash(),
            ))
        })
        .expect("cached tokens");
        let token_dep = db.query_dependency(&token_key).expect("token dependency");
        db.query(ast_key.clone(), [token_dep], || {
            Ok(QueryOutput::new(
                seal_parser::parse(&tokens, &dcx, &interner),
                file.content_hash(),
            ))
        })
        .expect("cached AST");
    }
    let cached = per_iteration(cached_start.elapsed());
    let stats = db.stats();

    println!("frontend direct_lex_parse_us={direct:.3}");
    println!("frontend cached_lex_parse_us={cached:.3}");
    println!(
        "frontend speedup={:.2}x",
        if cached > 0.0 { direct / cached } else { 0.0 }
    );
    println!("frontend hits={} misses={}", stats.hits, stats.misses);
}
