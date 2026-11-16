//! `noalias` emission tests.
//!
//! The attribute is an optimization gated by borrow checking's facts, so these
//! tests inspect the declared LLVM functions directly instead of running the
//! program.

use inkwell::context::Context;
use seal_session::{Options, Session};
use seal_span::FileName;

/// Runs the front end and returns the IR of every declared function.
fn module_ir(source: &str) -> String {
    let session = Session::new(Options::default()).expect("session starts");
    let file = session
        .add_file(FileName::virtual_name("test.sl"), source.to_owned())
        .expect("source registers");
    let source_file = session
        .source_map()
        .file(file)
        .expect("registered source exists");

    let tokens = seal_lexer::lex(&source_file, session.diag(), session.interner());
    let ast = seal_parser::parse(&tokens, session.diag(), session.interner());
    let hir = seal_hir::lower(&ast, session.diag(), session.interner());
    let typeck = seal_typeck::check(
        &hir,
        session.diag(),
        session.interner(),
        session.target().pointer_width(),
    );
    let mir = seal_mir::lower(&hir, &typeck);
    mir.validate()
        .expect("MIR lowering preserves its invariants");
    let facts = seal_borrowck::check(&hir, &mir, session.diag(), session.interner());
    assert!(!session.diag().has_errors(), "the test program must check");

    let context = Context::create();
    let module = context.create_module("test");
    let mapper = crate::types::TypeMapper::new(
        &context,
        hir.ty_ctxt(),
        session.interner(),
        session.target().pointer_width(),
    )
    .expect("types map to LLVM");
    crate::backend::declare_functions(
        &context,
        &module,
        &hir,
        &mir,
        session.interner(),
        &mapper,
        facts.noalias_functions(),
    )
    .expect("functions declare");
    module.print_to_string().to_string()
}

#[test]
fn an_exclusive_parameter_is_noalias_when_the_proof_closes() {
    let ir = module_ir(
        "func bump(count: *mut i32) {\n  *count = *count + 1;\n}\n\n\
         func main() -> i32 {\n  let mut value = 1;\n  bump(&mut value);\n  value\n}\n",
    );
    assert!(ir.contains("noalias"), "expected `noalias` in:\n{ir}");
}

#[test]
fn an_unknown_pointer_in_the_callee_withholds_noalias() {
    let ir = module_ir(
        "struct Node {\n  next: *mut i32,\n}\n\n\
         func follow(node: *mut Node) -> i32 {\n  let next = (*node).next;\n  *next\n}\n\n\
         func main() -> i32 {\n  let mut value = 7;\n  let mut node = Node { next: &mut value };\n  follow(&mut node)\n}\n",
    );
    assert!(!ir.contains("noalias"), "unexpected `noalias` in:\n{ir}");
}

#[test]
fn an_unproven_call_argument_withholds_noalias() {
    let ir = module_ir(
        "struct Node {\n  next: *mut i32,\n}\n\n\
         func bump(count: *mut i32) {\n  *count = *count + 1;\n}\n\n\
         func main() -> i32 {\n  let mut value = 7;\n  let mut node = Node { next: &mut value };\n  let r = &node;\n  bump((*r).next);\n  0\n}\n",
    );
    assert!(!ir.contains("noalias"), "unexpected `noalias` in:\n{ir}");
}
