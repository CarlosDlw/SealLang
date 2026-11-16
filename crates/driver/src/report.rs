//! Reporting code generation failures.

use seal_session::Session;

/// Emits a code generation failure through the session diagnostics.
pub(crate) fn codegen_error(session: &Session, error: &seal_codegen::CodegenError) {
    let message = error.to_string();
    match error.diag_code() {
        Some(code) => {
            let _ = session.diag().error(message).code(code).emit();
        }
        None => session.diag().bug(message),
    }
}
