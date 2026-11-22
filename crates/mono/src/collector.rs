//! Reachable-instance collection.

use std::collections::{HashSet, VecDeque};

use seal_const_eval::{AllocId, AllocSource, ConstProgram, ConstValue};
use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hash::{FxHashMap, FxHashSet};
use seal_hir::Hir;
use seal_index::{Idx, IndexVec};
use seal_mir::{
    Body, BodyKind, Constant, IntrinsicReceiver, Mir, Operand, Place, PlaceRoot, Rvalue,
    StatementKind, TerminatorKind, place_ty,
};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{Instance, TyId};

use crate::drop::{self, DropGlueEntry};
use crate::error::MonoError;
use crate::id::{DropGlueId, InstanceId, VtableId};
use crate::substitute::{self, BodyEvent, SpecializedBody};
use crate::vtable::{self, MonoVTable, VtableKey};
use crate::{Collected, MonoMir, MonoProgram};

/// Maximum call-chain depth while discovering instances.
pub const MAX_DEPTH: usize = 64;

/// Maximum number of distinct instances one program may collect.
pub const MAX_INSTANCES: usize = 4096;

/// Maximum number of distinct vtables one program may collect.
pub const MAX_VTABLES: usize = 4096;

/// Maximum number of distinct drop glue functions one program may collect.
pub const MAX_GLUES: usize = 4096;

/// Maximum nesting depth of one drop glue shape.
pub const MAX_GLUE_DEPTH: usize = 64;

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
    pub(crate) glues: usize,
    pub(crate) glue_depth: usize,
}

impl Limits {
    pub(crate) const DEFAULT: Self = Self {
        depth: MAX_DEPTH,
        instances: MAX_INSTANCES,
        vtables: MAX_VTABLES,
        glues: MAX_GLUES,
        glue_depth: MAX_GLUE_DEPTH,
    };
}

/// One collected item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MonoItem {
    /// One concrete function body.
    Function(Instance),
    /// One reachable static, collected once per declaration.
    Static(seal_def::DefId),
    /// One reachable non-static allocation, addressed by a const or promotion.
    ConstAlloc(AllocId),
    /// One drop glue function for a concrete type.
    DropGlue(DropGlueId),
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
pub fn collect(
    hir: &Hir,
    mir: &Mir,
    constants: &ConstProgram,
    dcx: &DiagCtxt,
    interner: &Interner,
) -> MonoProgram {
    collect_with(hir, mir, constants, dcx, interner, Limits::DEFAULT)
}

/// Collects every reachable instance using a caller-provided specializer.
///
/// The callback receives the instance, its owning definition, and its generic
/// MIR body, and must return the concrete body plus its discovery events. The
/// definition lets an incremental caller name the exact body query the
/// specialization depends on. Diagnostics emitted by the specializer are the
/// caller's responsibility; the collector reports only graph-level errors.
pub fn collect_with_specializer<F>(
    hir: &Hir,
    mir: &Mir,
    constants: &ConstProgram,
    dcx: &DiagCtxt,
    interner: &Interner,
    mut specialize: F,
) -> MonoProgram
where
    F: FnMut(Instance, seal_def::DefId, &Body) -> SpecializedBody,
{
    collect_with_specializer_and_limits(
        hir,
        mir,
        constants,
        dcx,
        interner,
        Limits::DEFAULT,
        &mut specialize,
    )
}

/// Collects with explicit limits, for tests.
pub(crate) fn collect_with(
    hir: &Hir,
    mir: &Mir,
    constants: &ConstProgram,
    dcx: &DiagCtxt,
    interner: &Interner,
    limits: Limits,
) -> MonoProgram {
    collect_with_specializer_and_limits(
        hir,
        mir,
        constants,
        dcx,
        interner,
        limits,
        &mut |instance, _owner, body| substitute::specialize(hir, dcx, interner, body, instance),
    )
}

fn collect_with_specializer_and_limits(
    hir: &Hir,
    mir: &Mir,
    constants: &ConstProgram,
    dcx: &DiagCtxt,
    interner: &Interner,
    limits: Limits,
    specialize: &mut dyn FnMut(Instance, seal_def::DefId, &Body) -> SpecializedBody,
) -> MonoProgram {
    let mut collector = Collector {
        specialize,
        hir,
        mir,
        constants,
        dcx,
        interner,
        limits,
        ids: FxHashMap::default(),
        queue: VecDeque::new(),
        bodies: Vec::new(),
        items: Vec::new(),
        statics: Vec::new(),
        static_ids: FxHashSet::default(),
        extern_statics: Vec::new(),
        extern_static_ids: FxHashSet::default(),
        const_allocations: Vec::new(),
        alloc_ids: FxHashSet::default(),
        scanned_allocs: FxHashSet::default(),
        vtables: IndexVec::new(),
        vtable_ids: FxHashMap::default(),
        rejected_vtables: HashSet::new(),
        glues: IndexVec::new(),
        glue_ids: FxHashMap::default(),
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
    constants: &'a ConstProgram,
    dcx: &'a DiagCtxt,
    interner: &'a Interner,
    limits: Limits,
    specialize: &'a mut dyn FnMut(Instance, seal_def::DefId, &Body) -> SpecializedBody,
    ids: FxHashMap<Instance, InstanceId>,
    queue: VecDeque<Pending>,
    bodies: Vec<Option<MonoMir>>,
    items: Vec<MonoItem>,
    statics: Vec<seal_def::DefId>,
    static_ids: FxHashSet<seal_def::DefId>,
    extern_statics: Vec<seal_def::DefId>,
    extern_static_ids: FxHashSet<seal_def::DefId>,
    const_allocations: Vec<AllocId>,
    alloc_ids: FxHashSet<AllocId>,
    scanned_allocs: FxHashSet<AllocId>,
    vtables: IndexVec<VtableId, MonoVTable>,
    vtable_ids: FxHashMap<VtableKey, VtableId>,
    /// Keys already reported, so one broken coercion is not reported again.
    rejected_vtables: HashSet<VtableKey>,
    glues: IndexVec<DropGlueId, DropGlueEntry>,
    glue_ids: FxHashMap<TyId, DropGlueId>,
}

impl Collector<'_> {
    /// Enqueues every non-generic function body, rejecting a generic `main`.
    fn seed_roots(&mut self) {
        for body in self.mir.bodies().iter() {
            // Const and static initializers are data, not roots: they become
            // items only when reachable from a function or another global.
            if body.kind != BodyKind::Function {
                continue;
            }
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
        if self.ids.len() >= self.limits.instances {
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
        let id = InstanceId::from_usize(self.ids.len());
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
        let specialized = (self.specialize)(pending.instance, pending.instance.def, body);
        self.scan_body(&specialized.body, &pending.chain);
        self.bodies[pending.id.index() as usize] = Some(MonoMir {
            instance: pending.instance,
            body: specialized.body,
        });
        for event in specialized.events {
            match event {
                BodyEvent::Call(child) | BodyEvent::Closure(child) => {
                    let mut chain = pending.chain.clone();
                    chain.push((child.instance, child.span));
                    self.enqueue(child.instance, pending.depth + 1, chain, child.span);
                }
                BodyEvent::Vtable { key, span } => {
                    self.collect_vtable(key, span, pending.depth + 1, &pending.chain, false)
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
        from_const: bool,
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
                if from_const {
                    self.dcx
                        .error(
                            MonoError::ConstantVtable {
                                detail: error.to_string(),
                            }
                            .to_string(),
                        )
                        .code(codes::E7007.diag_code())
                        .primary(span, "this constant trait object needs a complete vtable")
                        .emit();
                } else {
                    vtable::report(self.dcx, span, &error);
                }
                self.rejected_vtables.insert(key);
                return;
            }
        };
        let entries = table.entries.clone();
        let id = VtableId::from_usize(self.vtables.len());
        self.vtables.push(table);
        self.vtable_ids.insert(key, id);
        self.request_vtable_glue(key.concrete, 0, chain, span);
        for entry in entries {
            let mut chain = chain.to_vec();
            chain.push((entry, span));
            self.enqueue(entry, depth, chain, span);
        }
    }

    /// Records the glue for a drop site and schedules its destructor and
    /// children.
    ///
    /// A type that needs no destructor gets no entry: its drop sites call
    /// nothing.
    fn request_glue(&mut self, ty: TyId, depth: usize, chain: &[(Instance, Span)], span: Span) {
        self.request_glue_inner(ty, depth, chain, span, false);
    }

    /// Records the glue a vtable names as its concrete type.
    ///
    /// The first slot always points at that type's own glue, even when dropping
    /// the type runs nothing, so no shared no-op function is ever emitted.
    fn request_vtable_glue(
        &mut self,
        ty: TyId,
        depth: usize,
        chain: &[(Instance, Span)],
        span: Span,
    ) {
        self.request_glue_inner(ty, depth, chain, span, true);
    }

    /// Records the glue for `ty` and schedules its destructor and children.
    ///
    /// The entry and its ID exist before any child is visited, so a type that
    /// reaches the same glue again stops instead of recursing forever.
    fn request_glue_inner(
        &mut self,
        ty: TyId,
        depth: usize,
        chain: &[(Instance, Span)],
        span: Span,
        required: bool,
    ) {
        if self.glue_ids.contains_key(&ty) {
            return;
        }
        let glue = match drop::glue_shape(self.hir, ty) {
            Some(glue) => glue,
            None if required => crate::DropGlue {
                destructor: None,
                children: crate::GlueChildren::None,
            },
            None => return,
        };
        if depth > self.limits.glue_depth {
            let message = format!(
                "the drop glue nesting depth limit of {} was reached while building the glue \
                 for `{}`",
                self.limits.glue_depth,
                self.type_name(ty)
            );
            self.report_limit(codes::E7008, span, &message, chain);
            return;
        }
        if self.glues.len() >= self.limits.glues {
            let message = format!(
                "the drop glue limit of {} was reached while building the glue for `{}`",
                self.limits.glues,
                self.type_name(ty)
            );
            self.report_limit(codes::E7008, span, &message, chain);
            return;
        }
        let destructor = glue.destructor;
        let children = glue.children.clone();
        let id = DropGlueId::from_usize(self.glues.len());
        self.glue_ids.insert(ty, id);
        self.glues.push(DropGlueEntry { ty, glue });
        self.items.push(MonoItem::DropGlue(id));
        if let Some(destructor) = destructor {
            let mut chain = chain.to_vec();
            chain.push((destructor, span));
            self.enqueue(destructor, depth + 1, chain, span);
        }
        match children {
            crate::GlueChildren::None => {}
            crate::GlueChildren::Fields(fields) => {
                for child in fields {
                    self.request_glue(child, depth + 1, chain, span);
                }
            }
            crate::GlueChildren::Variants(variants) => {
                for child in variants.into_iter().flatten() {
                    self.request_glue(child, depth + 1, chain, span);
                }
            }
            crate::GlueChildren::Elements { element, .. } => {
                self.request_glue(element, depth + 1, chain, span);
            }
        }
    }

    /// Records every static, const allocation, vtable, and drop glue a body
    /// references.
    fn scan_body(&mut self, body: &Body, chain: &[(Instance, Span)]) {
        for block in body.blocks.iter() {
            for statement in &block.statements {
                let span = statement.span;
                match &statement.kind {
                    StatementKind::Assign { place, rvalue } => {
                        self.scan_place(place, span);
                        self.scan_rvalue(rvalue, span);
                    }
                    StatementKind::Drop { place, .. } => {
                        self.scan_place(place, span);
                        self.request_glue(place_ty(self.hir, body, place), 0, chain, span);
                    }
                    StatementKind::DropRemaining { place, from } => {
                        self.scan_place(place, span);
                        self.scan_operand(from, span);
                        self.request_glue(place_ty(self.hir, body, place), 0, chain, span);
                    }
                }
            }
            match &block.terminator.kind {
                TerminatorKind::SwitchInt { discriminant, .. } => {
                    self.scan_operand(discriminant, block.terminator.span);
                }
                TerminatorKind::PanicValue { message } => {
                    self.scan_operand(message, block.terminator.span);
                }
                TerminatorKind::Exit { code } => {
                    self.scan_operand(code, block.terminator.span);
                }
                TerminatorKind::Goto(_)
                | TerminatorKind::Return
                | TerminatorKind::Panic { .. }
                | TerminatorKind::Unreachable => {}
            }
        }
    }

    fn scan_rvalue(&mut self, rvalue: &Rvalue, span: Span) {
        match rvalue {
            Rvalue::Use(operand) | Rvalue::Cast { operand, .. } => self.scan_operand(operand, span),
            Rvalue::MakeTraitObject { source, .. } => self.scan_operand(source, span),
            Rvalue::TraitObjectReborrow { source, place, .. } => {
                self.scan_operand(source, span);
                self.scan_place(place, span);
            }
            Rvalue::Ref { place, .. } => self.scan_place(place, span),
            Rvalue::SliceRef {
                base, start, end, ..
            } => {
                self.scan_place(base, span);
                for operand in start.iter().chain(end.iter()) {
                    self.scan_operand(operand, span);
                }
            }
            Rvalue::Discriminant(place) => self.scan_place(place, span),
            Rvalue::Len { slice } => self.scan_operand(slice, span),
            Rvalue::TypeInfo { arg, .. } => {
                if let Some(arg) = arg {
                    self.scan_operand(arg, span);
                }
            }
            Rvalue::Binary { lhs, rhs, .. } => {
                self.scan_operand(lhs, span);
                self.scan_operand(rhs, span);
            }
            Rvalue::Unary { operand, .. } => self.scan_operand(operand, span),
            Rvalue::Intrinsic { receiver, args, .. } => {
                match receiver {
                    IntrinsicReceiver::Place(place) => self.scan_place(place, span),
                    IntrinsicReceiver::Value(operand) => self.scan_operand(operand, span),
                }
                for arg in args {
                    self.scan_operand(arg, span);
                }
            }
            Rvalue::DynamicCall { receiver, args, .. } => {
                self.scan_operand(receiver, span);
                for arg in args {
                    self.scan_operand(arg, span);
                }
            }
            Rvalue::Call { args, .. } => {
                for arg in args {
                    self.scan_operand(arg, span);
                }
            }
            Rvalue::IndirectCall { callee, args, .. } => {
                self.scan_operand(callee, span);
                for arg in args {
                    self.scan_operand(arg, span);
                }
            }
            Rvalue::Aggregate { fields, .. } => {
                for (_, operand) in fields {
                    self.scan_operand(operand, span);
                }
            }
            Rvalue::Tuple { elements, .. } | Rvalue::Array { elements, .. } => {
                for operand in elements {
                    self.scan_operand(operand, span);
                }
            }
            Rvalue::Repeat { value, .. } => self.scan_operand(value, span),
        }
    }

    fn scan_operand(&mut self, operand: &Operand, span: Span) {
        match operand {
            Operand::Copy(place) | Operand::Move(place) => self.scan_place(place, span),
            Operand::Const(def) => {
                if let Some(value) = self.constants.value(*def).cloned() {
                    self.scan_value(&value, span);
                }
            }
            Operand::Constant(
                Constant::FunctionAddr { instance, .. } | Constant::ClosureFnPtr { instance, .. },
            ) => {
                // The address can be called from C, or from a function pointer
                // value, so the body must be monomorphized even when no Seal
                // call site names it.
                if self.hir.body_of(instance.def).is_some() {
                    let chain = vec![(*instance, span)];
                    self.enqueue(*instance, 0, chain, span);
                }
            }
            Operand::Constant(_) => {}
        }
    }

    fn scan_place(&mut self, place: &Place, span: Span) {
        match place.root {
            PlaceRoot::Local(_) => {}
            // An external static is defined by C: there is no Seal allocation
            // to collect, and the linker resolves the symbol.
            PlaceRoot::Static(def) => {
                if self.hir.is_extern_static(def) {
                    if self.extern_static_ids.insert(def) {
                        self.extern_statics.push(def);
                    }
                } else {
                    self.add_static(def);
                }
            }
            PlaceRoot::Const(def) => {
                if let Some(alloc) = self.constants.const_allocation(def) {
                    self.scan_allocation(alloc);
                } else if let Some(value) = self.constants.value(def).cloned() {
                    self.scan_value(&value, span);
                }
            }
        }
    }

    /// Adds a static item and scans its allocation once.
    fn add_static(&mut self, def: seal_def::DefId) {
        if !self.static_ids.insert(def) {
            return;
        }
        let Some(data) = self.constants.static_data(def).copied() else {
            self.dcx
                .bug("MIR references a static that constant evaluation did not produce");
        };
        self.statics.push(def);
        self.items.push(MonoItem::Static(def));
        self.scan_allocation(data.alloc);
    }

    /// Adds a non-static allocation item; the caller scans its value.
    fn add_const_allocation(&mut self, id: AllocId) {
        if self.alloc_ids.insert(id) {
            self.const_allocations.push(id);
            self.items.push(MonoItem::ConstAlloc(id));
        }
    }

    /// Resolves one allocation, recording the owning static or a const item.
    fn scan_allocation(&mut self, id: AllocId) {
        if !self.scanned_allocs.insert(id) {
            return;
        }
        let allocation = self.constants.allocation(id);
        let source = allocation.source;
        let value = allocation.value.clone();
        let span = self.source_span(source);
        match source {
            AllocSource::Static(def) => self.add_static(def),
            AllocSource::Const(_)
            | AllocSource::Promoted { .. }
            | AllocSource::ConstFnLocal { .. } => self.add_const_allocation(id),
        }
        self.scan_value(&value, span);
    }

    /// Scans a constant value for allocation references and trait objects.
    fn scan_value(&mut self, value: &ConstValue, span: Span) {
        match value {
            ConstValue::Ref { target, .. }
            | ConstValue::Slice { target, .. }
            | ConstValue::RawPtr { target, .. }
            | ConstValue::RawSlice { target, .. } => {
                self.scan_allocation(*target);
            }
            ConstValue::Dyn {
                data,
                concrete,
                trait_ref,
                ..
            }
            | ConstValue::RawDyn {
                data,
                concrete,
                trait_ref,
                ..
            } => {
                self.scan_allocation(*data);
                self.collect_vtable(
                    VtableKey {
                        concrete: *concrete,
                        trait_ref: *trait_ref,
                    },
                    span,
                    0,
                    &[],
                    true,
                );
            }
            ConstValue::Tuple(elements) | ConstValue::Array(elements) => {
                for element in elements {
                    self.scan_value(element, span);
                }
            }
            ConstValue::Struct { fields, .. } | ConstValue::Variant { fields, .. } => {
                for (_, field) in fields {
                    self.scan_value(field, span);
                }
            }
            ConstValue::Int { .. }
            | ConstValue::Float { .. }
            | ConstValue::Bool(_)
            | ConstValue::Char(_)
            | ConstValue::Unit
            | ConstValue::Str { .. }
            | ConstValue::RawStr { .. }
            | ConstValue::RawStrData { .. } => {}
        }
    }

    fn source_span(&self, source: AllocSource) -> Span {
        let def = match source {
            AllocSource::Static(def) | AllocSource::Const(def) => def,
            AllocSource::Promoted { owner, .. } => owner,
            AllocSource::ConstFnLocal { instance, .. } => instance.def,
        };
        self.hir.definitions().get(def).span
    }

    fn finish(self) -> MonoProgram {
        let Self {
            ids,
            bodies,
            items,
            statics,
            extern_statics,
            const_allocations,
            vtables,
            vtable_ids,
            glues,
            glue_ids,
            ..
        } = self;
        let instances: Vec<MonoMir> = bodies
            .into_iter()
            .map(|body| body.expect("every discovered instance has a body"))
            .collect();
        MonoProgram::new(Collected {
            instances: IndexVec::from_raw(instances),
            ids,
            items,
            statics,
            extern_statics,
            const_allocations,
            vtables,
            vtable_ids,
            drop_glues: glues,
            drop_glue_ids: glue_ids,
        })
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

    fn type_name(&self, ty: TyId) -> String {
        self.hir
            .ty_ctxt()
            .display_with(ty, self.interner)
            .to_string()
    }

    fn def_name(&self, def: seal_def::DefId) -> String {
        self.interner
            .resolve(self.hir.definitions().name(def))
            .to_owned()
    }
}
