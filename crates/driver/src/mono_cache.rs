//! Query-backed monomorphization specialization.
//!
//! The collector walks the reachable instance graph, but the expensive part of
//! each step is substituting the generic MIR body and resolving its trait
//! calls. That value depends only on the instance, the owning body, and the
//! declarations, so it is cached per instance. The walk itself stays explicit:
//! reachability is graph state, not a memoized query.

use seal_def::DefId;
use seal_hash::StableHasher;
use seal_hir::Hir;
use seal_mir::Body;
use seal_query::{Dependency, InputKind, QueryError, QueryKey, QueryKind, QueryOutput};
use seal_session::Session;
use seal_ty::Instance;

use crate::query_cache::{self, Cached};

/// The shared inputs of every mono-instance query in one unit.
pub(crate) struct MonoContext<'a> {
    /// The session owning the query database.
    pub(crate) session: &'a Session,
    /// Stable identity of the HIR unit.
    pub(crate) unit_key: &'a str,
    /// The unit's HIR.
    pub(crate) hir: &'a Hir,
    /// The HIR query stamp.
    pub(crate) hir_dependency: Dependency,
    /// The declarations query stamp.
    pub(crate) declarations_dependency: Dependency,
}

/// Specializes one instance through the session query database.
pub(crate) fn specialize(
    context: &MonoContext<'_>,
    mir_dependency: Option<Dependency>,
    instance: Instance,
    owner: DefId,
    body: &Body,
) -> seal_mono::SpecializedBody {
    let session = context.session;
    let hir = context.hir;
    let fingerprint = seal_mono::instance_hash(hir, instance);
    let body_fingerprint = body_fingerprint(body);
    let body_input = match session.queries().set_input(
        InputKind::Source,
        format!("mono-body:{}:{}", context.unit_key, owner.index()),
        body_fingerprint.as_u128().to_be_bytes(),
    ) {
        Ok(input) => input,
        Err(error) => {
            report(session, &error);
            unreachable!("a diagnostic context bug is fatal");
        }
    };
    let body_dependency = match session.queries().input_dependency(body_input) {
        Ok(dependency) => dependency,
        Err(error) => {
            report(session, &error);
            unreachable!("a diagnostic context bug is fatal");
        }
    };
    let mut dependencies = vec![
        context.hir_dependency.clone(),
        context.declarations_dependency.clone(),
        body_dependency,
    ];
    if let Some(dependency) = mir_dependency {
        dependencies.push(dependency);
    }
    let key = QueryKey::new(
        QueryKind::MonoInstance,
        format!("mono-instance:{}:{fingerprint}", context.unit_key),
    );
    let dependencies_for_query = dependencies.clone();
    let output = session.queries().query(key, dependencies_for_query, || {
        let (specialized, diagnostics) = query_cache::with_diagnostics(session, |dcx| {
            seal_mono::specialize(hir, dcx, session.interner(), body, instance)
        });
        Ok(QueryOutput::new(
            Cached::new(specialized, diagnostics),
            fingerprint,
        ))
    });
    match output {
        Ok(output) => {
            query_cache::replay(session, output.value());
            output.value().value.clone()
        }
        Err(error) => {
            report(session, &error);
            unreachable!("a diagnostic context bug is fatal");
        }
    }
}

/// Hashes a MIR body's complete structure.
///
/// Arena IDs are included through `Debug`: a body edit that shifts later IDs
/// must not reuse a specialization keyed by the old body.
fn body_fingerprint(body: &Body) -> seal_hash::Fingerprint {
    let mut hasher = StableHasher::new();
    hasher.write_str(&format!("{body:?}"));
    hasher.finish()
}

fn report(session: &Session, error: &QueryError) {
    session
        .diag()
        .bug(format!("the mono-instance query failed: {error}"));
}
