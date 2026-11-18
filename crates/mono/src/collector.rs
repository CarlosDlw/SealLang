//! Reachable-instance collection.

use std::collections::{HashSet, VecDeque};

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
use crate::id::{InstanceId, VtableId};
use crate::substitute::{self, BodyEvent};
use crate::vtable::{self, MonoVTable, VtableKey};
use crate::{MonoMir, MonoProgram};

/// Maximum call-chain depth while discovering instances.
pub const MAX_DEPTH: usize = 64;

/// Maximum number of distinct instances one program may collect.
pub const MAX_INSTANCES: usize = 4096;

/// Maximum number of distinct vtables one program may collect.
pub const MAX_VTABLES: usize = 4096;

/// Expansion limits.
///
/// Production uses [`collect`], which applies [`MAX_DEPTH`] and
/// [`MAX_INSTANCES`]. Tests lower the limits to exercise the diagnostics without
/// generating thousands of instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub(crate) depth: usize,
    pub(crate) instances: usize,
    pub(crate) vtables: usize,
}

impl Limits {
    pub(crate) const DEFAULT: Self = Self {
        depth: MAX_DEPTH,
        instances: MAX_INSTANCES,
        vtables: MAX_VTABLES,
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
        vtables: IndexVec::new(),
        vtable_ids: FxHashMap::default(),
        rejected_vtables: HashSet::new(),
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
    vtables: IndexVec<VtableId, MonoVTable>,
    vtable_ids: FxHashMap<VtableKey, VtableId>,
    /// Keys already reported, so one broken coercion is not reported again.
    rejected_vtables: HashSet<VtableKey>,
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
        let (body, events) =
            substitute::resolve_body(self.hir, self.dcx, self.interner, specialized);
        self.bodies[pending.id.index() as usize] = Some(MonoMir {
            instance: pending.instance,
            body,
        });
        for event in events {
            match event {
                BodyEvent::Call(child) => {
                    let mut chain = pending.chain.clone();
                    chain.push((child.instance, child.span));
                    self.enqueue(child.instance, pending.depth + 1, chain, child.span);
                }
                BodyEvent::Vtable { key, span } => {
                    self.collect_vtable(key, span, pending.depth + 1, &pending.chain)
                }
            }
        }
    }

    /// Builds one vtable and schedules its method instances.
    ///
    /// The key is inserted before the methods are enqueued so a method body
    /// that coerces to the same vtable cannot request it recursively. Failures
    /// mark the key rejected, so repeated coercions do not repeat diagnostics.
    fn collect_vtable(
        &mut self,
        key: VtableKey,
        span: Span,
        depth: usize,
        chain: &[(Instance, Span)],
    ) {
        if self.vtable_ids.contains_key(&key) || self.rejected_vtables.contains(&key) {
            return;
        }
        if self.vtables.len() >= self.limits.vtables {
            self.dcx
                .error(
                    MonoError::VtableLimit {
                        limit: self.limits.vtables,
                    }
                    .to_string(),
                )
                .code(codes::E7006.diag_code())
                .primary(span, "the vtable limit was reached here")
                .emit();
            self.rejected_vtables.insert(key);
            return;
        }
        let table = match vtable::build_vtable(self.hir, self.interner, key) {
            Ok(table) => table,
            Err(error) => {
                vtable::report(self.dcx, span, &error);
                self.rejected_vtables.insert(key);
                return;
            }
        };
        let entries = table.entries.clone();
        let id = VtableId::from_usize(self.vtables.len());
        self.vtables.push(table);
        self.vtable_ids.insert(key, id);
        for entry in entries {
            let mut chain = chain.to_vec();
            chain.push((entry, span));
            self.enqueue(entry, depth, chain, span);
        }
    }

    fn finish(self) -> MonoProgram {
        let Self {
            ids,
            bodies,
            items,
            vtables,
            vtable_ids,
            ..
        } = self;
        let instances: Vec<MonoMir> = bodies
            .into_iter()
            .map(|body| body.expect("every discovered instance has a body"))
            .collect();
        MonoProgram::new(
            IndexVec::from_raw(instances),
            ids,
            items,
            vtables,
            vtable_ids,
        )
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
