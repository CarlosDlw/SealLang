//! Reachable-instance collection.

use std::collections::VecDeque;

use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_index::{Idx, IndexVec};
use seal_mir::Mir;
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::Instance;

use crate::error::MonoError;
use crate::id::InstanceId;
use crate::substitute;
use crate::{MonoMir, MonoProgram};

/// Maximum call-chain depth while discovering instances.
pub const MAX_DEPTH: usize = 64;

/// Maximum number of distinct instances one program may collect.
pub const MAX_INSTANCES: usize = 4096;

/// Expansion limits.
///
/// Production uses [`collect`], which applies [`MAX_DEPTH`] and
/// [`MAX_INSTANCES`]. Tests lower the limits to exercise the diagnostics without
/// generating thousands of instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub(crate) depth: usize,
    pub(crate) instances: usize,
}

impl Limits {
    pub(crate) const DEFAULT: Self = Self {
        depth: MAX_DEPTH,
        instances: MAX_INSTANCES,
    };
}

/// One collected item.
///
/// Functions are the only items with an instance today; the enum keeps the door
/// open for global constants and statics without changing the collector's
/// output type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonoItem {
    /// One concrete function body.
    Function(Instance),
}

/// One discovered instance waiting for its body.
struct Pending {
    id: InstanceId,
    instance: Instance,
    depth: usize,
    /// Every step that reached this instance, with the call span of each step.
    chain: Vec<(Instance, Span)>,
}

/// Collects every instance reachable from the non-generic roots.
pub fn collect(hir: &Hir, mir: &Mir, dcx: &DiagCtxt, interner: &Interner) -> MonoProgram {
    collect_with(hir, mir, dcx, interner, Limits::DEFAULT)
}

/// Collects with explicit limits, for tests.
pub(crate) fn collect_with(
    hir: &Hir,
    mir: &Mir,
    dcx: &DiagCtxt,
    interner: &Interner,
    limits: Limits,
) -> MonoProgram {
    let mut collector = Collector {
        hir,
        mir,
        dcx,
        interner,
        limits,
        ids: FxHashMap::default(),
        queue: VecDeque::new(),
        bodies: Vec::new(),
        items: Vec::new(),
    };
    collector.seed_roots();
    while let Some(pending) = collector.queue.pop_front() {
        collector.process(pending);
    }
    collector.finish()
}

struct Collector<'a> {
    hir: &'a Hir,
    mir: &'a Mir,
    dcx: &'a DiagCtxt,
    interner: &'a Interner,
    limits: Limits,
    ids: FxHashMap<Instance, InstanceId>,
    queue: VecDeque<Pending>,
    bodies: Vec<Option<MonoMir>>,
    items: Vec<MonoItem>,
}

impl Collector<'_> {
    /// Enqueues every non-generic function body, rejecting a generic `main`.
    fn seed_roots(&mut self) {
        for body in self.mir.bodies().iter() {
            let owner = body.owner;
            let name = self.interner.resolve(self.hir.definitions().name(owner));
            let parameters = self.hir.flattened_generics(owner).len();
            if name == "main" && parameters > 0 {
                let span = self.hir.definitions().get(owner).span;
                self.dcx
                    .error(MonoError::GenericEntryPoint { parameters }.to_string())
                    .code(codes::E7003.diag_code())
                    .primary(span, "the entry point cannot be generic")
                    .emit();
                continue;
            }
            if parameters > 0 {
                continue;
            }
            let instance = Instance::new(owner, self.hir.ty_ctxt().empty_substs());
            let span = self.hir.definitions().get(owner).span;
            self.enqueue(instance, 0, vec![(instance, span)], span);
        }
    }

    /// Records one instance and schedules its body, or reports why it cannot.
    fn enqueue(
        &mut self,
        instance: Instance,
        depth: usize,
        chain: Vec<(Instance, Span)>,
        span: Span,
    ) {
        if self.ids.contains_key(&instance) {
            return;
        }
        if self.items.len() >= self.limits.instances {
            self.report_limit(
                codes::E7004,
                span,
                &format!(
                    "monomorphization instance limit of {} reached while instantiating `{}`",
                    self.limits.instances,
                    self.instance_name(instance)
                ),
                &chain,
            );
            return;
        }
        if depth > self.limits.depth {
            self.report_limit(
                codes::E7001,
                span,
                &format!(
                    "monomorphization recursion limit of {} reached while instantiating `{}`",
                    self.limits.depth,
                    self.instance_name(instance)
                ),
                &chain,
            );
            return;
        }
        if self.mir.body_of(instance.def).is_none() {
            self.dcx
                .error(
                    MonoError::MissingBody {
                        definition: self.def_name(instance.def),
                    }
                    .to_string(),
                )
                .code(codes::E7002.diag_code())
                .primary(span, "this call needs a function body")
                .emit();
            return;
        }
        let id = InstanceId::from_usize(self.items.len());
        self.ids.insert(instance, id);
        self.items.push(MonoItem::Function(instance));
        self.bodies.push(None);
        self.queue.push_back(Pending {
            id,
            instance,
            depth,
            chain,
        });
    }

    /// Specializes one body and enqueues its callees in statement order.
    fn process(&mut self, pending: Pending) {
        let Some(body) = self.mir.body_of(pending.instance.def) else {
            return;
        };
        let specialized = substitute::instantiate_body(self.hir, body, pending.instance);
        let (body, children) =
            substitute::resolve_calls(self.hir, self.dcx, self.interner, specialized);
        self.bodies[pending.id.index() as usize] = Some(MonoMir {
            instance: pending.instance,
            body,
        });
        for child in children {
            let mut chain = pending.chain.clone();
            chain.push((child.instance, child.span));
            self.enqueue(child.instance, pending.depth + 1, chain, child.span);
        }
    }

    fn finish(self) -> MonoProgram {
        let Self {
            ids, bodies, items, ..
        } = self;
        let instances: Vec<MonoMir> = bodies
            .into_iter()
            .map(|body| body.expect("every discovered instance has a body"))
            .collect();
        MonoProgram::new(IndexVec::from_raw(instances), ids, items)
    }

    /// Reports a limit diagnostic with one label per path step.
    fn report_limit(
        &self,
        code: codes::ErrorCode,
        span: Span,
        message: &str,
        chain: &[(Instance, Span)],
    ) {
        let mut diagnostic = self
            .dcx
            .error(message)
            .code(code.diag_code())
            .primary(span, "the limit was reached here");
        for (index, (instance, span)) in chain.iter().enumerate() {
            diagnostic = diagnostic.secondary(
                *span,
                format!("step {}: `{}`", index + 1, self.instance_name(*instance)),
            );
        }
        diagnostic.emit();
    }

    fn instance_name(&self, instance: Instance) -> String {
        crate::hash::instance_name(self.hir, self.interner, instance)
    }

    fn def_name(&self, def: seal_def::DefId) -> String {
        self.interner
            .resolve(self.hir.definitions().name(def))
            .to_owned()
    }
}
