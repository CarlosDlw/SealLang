//! Debug metadata survives the `.sealmono` round trip.

mod support;

use inkwell::context::Context;
use seal_codegen::Emit;
use seal_session::OptLevel;

#[test]
fn debug_info_round_trips_through_the_frontend_archive() {
    let output = support::TempOutput::new("debug.sealmono");
    support::compile(
        "func main() -> i32 { 0 }\n",
        output.path(),
        Emit::Mono,
        OptLevel::O0,
        true,
    )
    .expect("the frontend archive is written");
    let archive = seal_metadata::read_archive_file(output.path()).expect("valid archive");
    let context = Context::create();
    let module = seal_llvm_ffi::parse_bitcode(&context, archive.module(0).expect("one CGU"))
        .expect("the unit parses");
    assert!(
        module.get_flag("Debug Info Version").is_some(),
        "LLVM strips all debug metadata when the module flag is absent"
    );
    assert!(module.get_flag("Dwarf Version").is_some());
}
