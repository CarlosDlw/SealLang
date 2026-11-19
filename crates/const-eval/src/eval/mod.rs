//! The const interpreter.
//!
//! Evaluation runs over MIR. It executes only items whose bodies have no
//! parameters (`const` and `static` initializers); calls to `const func`
//! instances are added by the call phase. Every place is backed by an
//! allocation, so taking a reference is uniform and a final value never holds a
//! pointer into a dead frame.

use seal_def::{DefId, DefKind};
use seal_diagnostics::DiagCtxt;
use seal_hash::FxHashMap;
use seal_hir::{Body as HirBody, Hir};
use seal_index::IndexVec;
use seal_layout::{Layout, LayoutCtxt};
use seal_mir::{
    AggregateKind, Body, BodyKind, Constant, Operand, Place, PlaceRoot, Projection, Rvalue,
    StatementKind, TerminatorKind, UnOp,
};
use seal_span::Span;
use seal_symbol::Interner;
use seal_ty::{Instance, TraitRef, TyCtxt, TyId, TyKind, VariantIdx};

use crate::error::{EvalError, EvalErrorKind};
use crate::limits::EvalLimits;
use crate::ops;
use crate::place::{self, PathStep, ResolvedPlace};
use crate::program::ConstProgram;
use crate::value::{
    AllocId, AllocSource, ConstAllocation, ConstValue, Dependency, DependencyKind, StaticData,
    TraitObjectKey,
};

mod call;
mod equality;
mod execution;
mod intrinsic;
mod memory;
mod rvalue;

/// Evaluates every `const` and `static` item of a file.
pub fn evaluate(
    hir: &Hir,
    mir: &seal_mir::Mir,
    dcx: &DiagCtxt,
    interner: &Interner,
    pointer_width: u8,
) -> ConstProgram {
    Interpreter::new(
        hir,
        mir,
        dcx,
        interner,
        pointer_width,
        EvalLimits::default(),
    )
    .run()
}

/// Evaluates every value item with explicit resource limits.
pub fn evaluate_with_limits(
    hir: &Hir,
    mir: &seal_mir::Mir,
    dcx: &DiagCtxt,
    interner: &Interner,
    pointer_width: u8,
    limits: EvalLimits,
) -> ConstProgram {
    Interpreter::new(hir, mir, dcx, interner, pointer_width, limits).run()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EvalState {
    Visiting,
    Done,
    Failed,
}

struct Alloc {
    ty: TyId,
    value: Option<ConstValue>,
    mutable: bool,
    source: AllocSource,
    size: u64,
}

struct Frame {
    owner: DefId,
    locals: Vec<AllocId>,
}

struct Interpreter<'a> {
    hir: &'a Hir,
    mir: &'a seal_mir::Mir,
    tcx: &'a TyCtxt,
    dcx: &'a DiagCtxt,
    interner: &'a Interner,
    limits: EvalLimits,
    layout: LayoutCtxt<'a>,
    memory: IndexVec<AllocId, Alloc>,
    values: FxHashMap<DefId, ConstValue>,
    static_allocs: FxHashMap<DefId, AllocId>,
    states: FxHashMap<DefId, EvalState>,
    stack: Vec<DefId>,
    dependencies: FxHashMap<DefId, Vec<Dependency>>,
    promoted: FxHashMap<DefId, u32>,
    vtables: Vec<TraitObjectKey>,
    frames: Vec<Frame>,
    steps: u64,
    allocated_bytes: u64,
}

impl<'a> Interpreter<'a> {
    fn new(
        hir: &'a Hir,
        mir: &'a seal_mir::Mir,
        dcx: &'a DiagCtxt,
        interner: &'a Interner,
        pointer_width: u8,
        limits: EvalLimits,
    ) -> Self {
        let tcx = hir.ty_ctxt();
        Self {
            hir,
            mir,
            tcx,
            dcx,
            interner,
            limits,
            layout: LayoutCtxt::new(tcx, pointer_width),
            memory: IndexVec::new(),
            values: FxHashMap::default(),
            static_allocs: FxHashMap::default(),
            states: FxHashMap::default(),
            stack: Vec::new(),
            dependencies: FxHashMap::default(),
            promoted: FxHashMap::default(),
            vtables: Vec::new(),
            frames: Vec::new(),
            steps: 0,
            allocated_bytes: 0,
        }
    }

    fn run(mut self) -> ConstProgram {
        let items: Vec<DefId> = self
            .hir
            .definitions()
            .iter_enumerated()
            .filter(|(_, data)| matches!(data.kind, DefKind::Const | DefKind::Static))
            .map(|(def, _)| def)
            .collect();
        for def in items {
            if self.states.contains_key(&def) {
                continue;
            }
            if let Err(error) = self.eval_def(def) {
                self.report(def, &error);
            }
        }
        self.finish()
    }

    fn finish(self) -> ConstProgram {
        let mut program = ConstProgram::default();
        for (def, value) in self.values {
            if self.states.get(&def) == Some(&EvalState::Done) {
                program.values.insert(def, value);
            }
        }
        for (def, alloc) in &self.static_allocs {
            if self.states.get(def) != Some(&EvalState::Done) {
                continue;
            }
            let data = &self.memory[*alloc];
            let index = program.statics.len();
            program.statics.push(StaticData {
                def: *def,
                ty: data.ty,
                mutable: data.mutable,
                alloc: *alloc,
            });
            program.static_by_def.insert(*def, index);
        }
        program.statics.sort_by_key(|data| data.def);
        for (index, data) in program.statics.iter().enumerate() {
            program.static_by_def.insert(data.def, index);
        }
        let mut allocations = IndexVec::new();
        for alloc in self.memory.into_raw() {
            allocations.push(ConstAllocation {
                ty: alloc.ty,
                value: alloc.value.unwrap_or(ConstValue::Unit),
                mutable: alloc.mutable,
                source: alloc.source,
            });
        }
        program.allocations = allocations;
        program.dependencies = self.dependencies;
        program.vtables = self.vtables;
        program
    }

    fn report(&self, def: DefId, error: &EvalError) {
        let name = self.def_name(def);
        let message = error.kind.message();
        let mut builder = self
            .dcx
            .error(message)
            .primary(error.span, "in this constant expression")
            .note(format!("while evaluating `{name}`"));
        if let Some(code) = error.kind.code() {
            builder = builder.code(code.diag_code());
        }
        if let Some(help) = error.kind.help() {
            builder = builder.help(help);
        }
        if !error.call_chain.is_empty() {
            let chain: Vec<String> = error
                .call_chain
                .iter()
                .map(|def| self.def_name(*def))
                .collect();
            builder = builder.note(format!("compile-time call chain: {}", chain.join(" -> ")));
        }
        builder.emit();
    }

    fn error(&self, kind: EvalErrorKind, span: Span) -> EvalError {
        EvalError {
            kind,
            span,
            call_chain: self.frames.iter().map(|frame| frame.owner).collect(),
        }
    }

    fn internal<T>(&self, message: impl Into<String>, span: Span) -> Result<T, EvalError> {
        Err(self.error(EvalErrorKind::Internal(message.into()), span))
    }

    fn eval_def(&mut self, def: DefId) -> Result<ConstValue, EvalError> {
        match self.states.get(&def) {
            Some(EvalState::Done) => {
                return self.values.get(&def).cloned().ok_or_else(|| {
                    self.error(
                        EvalErrorKind::Internal("missing value".into()),
                        self.def_span(def),
                    )
                });
            }
            Some(EvalState::Failed) => {
                return Err(self.error(
                    EvalErrorKind::Internal("dependency failed to evaluate".into()),
                    self.def_span(def),
                ));
            }
            Some(EvalState::Visiting) => {
                return Err(self.error(
                    EvalErrorKind::ValueCycle {
                        chain: self.cycle_chain(def),
                    },
                    self.def_span(def),
                ));
            }
            None => {}
        }
        let hir_body = match self.hir.body_of(def) {
            Some(body) => body.clone(),
            None => {
                self.states.insert(def, EvalState::Failed);
                return self.internal("constant item has no HIR body", self.def_span(def));
            }
        };
        let mir_body = match self.mir.body_of(def) {
            Some(body) => body.clone(),
            None => {
                self.states.insert(def, EvalState::Failed);
                return self.internal("constant item has no MIR body", self.def_span(def));
            }
        };
        self.states.insert(def, EvalState::Visiting);
        self.stack.push(def);
        self.dependencies.entry(def).or_default();
        let result = self.eval_value_body(def, &hir_body, &mir_body);
        self.stack.pop();
        match result {
            Ok(value) => {
                self.values.insert(def, value.clone());
                self.states.insert(def, EvalState::Done);
                Ok(value)
            }
            Err(error) => {
                self.states.insert(def, EvalState::Failed);
                Err(error)
            }
        }
    }

    fn eval_value_body(
        &mut self,
        def: DefId,
        hir_body: &HirBody,
        mir_body: &Body,
    ) -> Result<ConstValue, EvalError> {
        let ty = hir_body.ret.unwrap_or_else(|| self.tcx.error_ty());
        let (mutable, source) = match mir_body.kind {
            BodyKind::Static { mutable } => (mutable, AllocSource::Static(def)),
            BodyKind::Const => (false, AllocSource::Const(def)),
            BodyKind::Function => {
                return self.internal(
                    "const evaluation received a function body",
                    self.def_span(def),
                );
            }
        };
        if matches!(mir_body.kind, BodyKind::Static { .. }) {
            let size = self.layout.layout_of(ty).map(Layout::size).unwrap_or(0);
            let alloc = self.allocate(ty, None, mutable, source, size, self.def_span(def))?;
            self.static_allocs.insert(def, alloc);
        }
        let instance = Instance::new(def, self.tcx.empty_substs());
        let value = self.eval_body_frame(def, instance, mir_body, &[])?;
        if let Some(alloc) = self.static_allocs.get(&def).copied() {
            self.set_alloc(alloc, value.clone(), self.def_span(def))?;
        }
        Ok(value)
    }

    fn def_span(&self, def: DefId) -> Span {
        self.hir.definitions().get(def).span
    }

    fn cycle_chain(&self, def: DefId) -> Vec<String> {
        let start = self.stack.iter().position(|item| *item == def).unwrap_or(0);
        let mut chain: Vec<String> = self.stack[start..]
            .iter()
            .map(|item| self.def_name(*item))
            .collect();
        chain.push(self.def_name(def));
        chain
    }

    fn def_name(&self, def: DefId) -> String {
        self.interner
            .resolve(self.hir.definitions().name(def))
            .to_owned()
    }

    fn allocate(
        &mut self,
        ty: TyId,
        value: Option<ConstValue>,
        mutable: bool,
        source: AllocSource,
        size: u64,
        span: Span,
    ) -> Result<AllocId, EvalError> {
        if size > self.limits.value_bytes {
            return Err(self.error(EvalErrorKind::ValueTooLarge, span));
        }
        if self.memory.len() >= self.limits.allocations
            || self.allocated_bytes.saturating_add(size) > self.limits.value_bytes
        {
            return Err(self.error(EvalErrorKind::ResourceLimit, span));
        }
        self.allocated_bytes = self.allocated_bytes.saturating_add(size);
        Ok(self.memory.push(Alloc {
            ty,
            value,
            mutable,
            source,
            size,
        }))
    }

    fn set_alloc(
        &mut self,
        alloc: AllocId,
        value: ConstValue,
        span: Span,
    ) -> Result<(), EvalError> {
        let Some(slot) = self.memory.get_mut(alloc) else {
            return self.internal("allocation is missing", span);
        };
        slot.value = Some(value);
        Ok(())
    }

    fn record_dependency(&mut self, target: DefId, kind: DependencyKind) {
        if let Some(owner) = self.stack.last().copied() {
            if owner == target {
                return;
            }
            let entry = self.dependencies.entry(owner).or_default();
            if !entry.contains(&Dependency { target, kind }) {
                entry.push(Dependency { target, kind });
            }
        }
    }
}
