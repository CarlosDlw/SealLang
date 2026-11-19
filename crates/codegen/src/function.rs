//! Lowering one MIR body into an LLVM function.

use inkwell::basic_block::BasicBlock;
use inkwell::builder::{Builder, BuilderError};
use inkwell::context::Context;
use inkwell::debug_info::DISubprogram;
use inkwell::module::Module;
use inkwell::types::{BasicTypeEnum, IntType};
use inkwell::values::{FunctionValue, IntValue, PointerValue};
use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_mir::{Body, LocalId, LocalKind, StatementKind, SwitchTarget, TerminatorKind};
use seal_span::{SourceMap, Span};
use seal_symbol::Interner;
use seal_ty::{Instance, TyCtxt, TyId};

use crate::const_value::ConstValues;
use crate::constant::Constants;
use crate::debug::DebugContext;
use crate::error::CodegenError;
use crate::global::Globals;
use crate::names;
use crate::relocation::Pointers;
use crate::types::TypeMapper;
use crate::vtable::Vtables;

/// Everything needed to lower one body.
pub(crate) struct FunctionRequest<'a, 'ctx> {
    /// LLVM context.
    pub(crate) context: &'ctx Context,
    /// Module receiving the function.
    pub(crate) module: &'a Module<'ctx>,
    /// Shared IR builder.
    pub(crate) builder: &'a Builder<'ctx>,
    /// The function being lowered.
    pub(crate) function: FunctionValue<'ctx>,
    /// The body to lower.
    pub(crate) body: &'a Body,
    /// HIR, used to resolve aggregate definitions.
    pub(crate) hir: &'a Hir,
    /// The interned type context.
    pub(crate) tcx: &'a TyCtxt,
    /// Type mapping.
    pub(crate) mapper: &'a TypeMapper<'ctx>,
    /// Constants for this module.
    pub(crate) constants: &'a Constants<'a, 'ctx>,
    /// Constant encoding, including the storage of values read by operand.
    pub(crate) values: &'a ConstValues<'a, 'ctx>,
    /// Resolver for the pointers inside constant data.
    pub(crate) pointers: &'a Pointers<'a, 'ctx>,
    /// Static and const-allocation storage.
    pub(crate) globals: &'a Globals<'ctx>,
    /// Declared functions, keyed by instance.
    pub(crate) functions: &'a FxHashMap<Instance, FunctionValue<'ctx>>,
    /// Emitted vtable globals, keyed by coercion.
    pub(crate) vtables: &'a Vtables<'ctx>,
    /// Symbol interner.
    pub(crate) interner: &'a Interner,
    /// Source map, used for runtime panic locations.
    pub(crate) source_map: &'a SourceMap,
    /// Debug information, when requested.
    pub(crate) debug: Option<&'a DebugContext<'a, 'ctx>>,
    /// This function's debug subprogram.
    pub(crate) scope: Option<DISubprogram<'ctx>>,
}

/// Lowers a single MIR body.
pub(crate) struct FunctionLowerer<'a, 'ctx> {
    pub(crate) context: &'ctx Context,
    pub(crate) module: &'a Module<'ctx>,
    pub(crate) builder: &'a Builder<'ctx>,
    pub(crate) function: FunctionValue<'ctx>,
    pub(crate) body: &'a Body,
    pub(crate) hir: &'a Hir,
    pub(crate) tcx: &'a TyCtxt,
    pub(crate) mapper: &'a TypeMapper<'ctx>,
    pub(crate) constants: &'a Constants<'a, 'ctx>,
    pub(crate) values: &'a ConstValues<'a, 'ctx>,
    pub(crate) pointers: &'a Pointers<'a, 'ctx>,
    pub(crate) globals: &'a Globals<'ctx>,
    pub(crate) functions: &'a FxHashMap<Instance, FunctionValue<'ctx>>,
    pub(crate) vtables: &'a Vtables<'ctx>,
    pub(crate) interner: &'a Interner,
    pub(crate) source_map: &'a SourceMap,
    pub(crate) debug: Option<&'a DebugContext<'a, 'ctx>>,
    pub(crate) scope: Option<DISubprogram<'ctx>>,
    pub(crate) allocas: Vec<PointerValue<'ctx>>,
    pub(crate) blocks: Vec<BasicBlock<'ctx>>,
    entry: BasicBlock<'ctx>,
}

impl<'a, 'ctx> FunctionRequest<'a, 'ctx> {
    /// Lowers the request's body.
    pub(crate) fn lower(self) -> Result<(), CodegenError> {
        let entry = self.context.append_basic_block(self.function, "entry");
        let mut blocks = Vec::with_capacity(self.body.blocks.len());
        for (id, _) in self.body.blocks.iter_enumerated() {
            let block = if id == self.body.entry {
                entry
            } else {
                self.context
                    .append_basic_block(self.function, &format!("bb{}", id.index()))
            };
            blocks.push(block);
        }
        FunctionLowerer {
            context: self.context,
            module: self.module,
            builder: self.builder,
            function: self.function,
            body: self.body,
            hir: self.hir,
            tcx: self.tcx,
            mapper: self.mapper,
            constants: self.constants,
            values: self.values,
            pointers: self.pointers,
            globals: self.globals,
            functions: self.functions,
            vtables: self.vtables,
            interner: self.interner,
            source_map: self.source_map,
            debug: self.debug,
            scope: self.scope,
            allocas: Vec::with_capacity(self.body.locals.len()),
            blocks,
            entry,
        }
        .run()
    }
}

impl<'a, 'ctx> FunctionLowerer<'a, 'ctx> {
    fn run(mut self) -> Result<(), CodegenError> {
        self.emit_prologue()?;
        for (id, block) in self.body.blocks.iter_enumerated() {
            self.builder
                .position_at_end(self.blocks[id.index() as usize]);
            for statement in &block.statements {
                self.set_span(statement.span);
                let (place, rvalue) = match &statement.kind {
                    StatementKind::Assign { place, rvalue } => (place, rvalue),
                    // Drop glue is emitted per type, and lowering does not emit
                    // drop sites yet, so reaching one here means the pipeline
                    // handed code generation a statement its shape cannot
                    // represent.
                    StatementKind::Drop { .. } | StatementKind::DropRemaining { .. } => {
                        return Err(CodegenError::internal(
                            "drop statements require drop glue, which is not emitted yet",
                        ));
                    }
                };
                let value = self.emit_rvalue(rvalue)?;
                let destination = self.place_ptr(place)?;
                self.build(self.builder.build_store(destination, value), "assignment")?;
            }
            self.set_span(block.terminator.span);
            self.emit_terminator(&block.terminator)?;
        }
        if self.debug.is_some() {
            self.builder.unset_current_debug_location();
        }
        Ok(())
    }

    fn emit_prologue(&mut self) -> Result<(), CodegenError> {
        self.builder.position_at_end(self.entry);
        let definition = self.hir.definitions().get(self.body.owner);
        self.set_span(definition.span);
        for (id, local) in self.body.locals.iter_enumerated() {
            let ty = self.mapper.basic(local.ty)?;
            let name = names::local(
                local.name.map(|name| self.interner.resolve(name)),
                id.index() as usize,
            );
            let alloca = self.build(self.builder.build_alloca(ty, &name), "local storage")?;
            self.allocas.push(alloca);
        }
        for (id, local) in self.body.locals.iter_enumerated() {
            if id.index() == 0 || id.index() as usize > self.body.arg_count {
                continue;
            }
            let parameter = self.function.get_nth_param(id.index() - 1).ok_or_else(|| {
                CodegenError::internal(format!(
                    "function {:?} is missing parameter {id:?}",
                    self.body.owner
                ))
            })?;
            let storage = self.allocas[id.index() as usize];
            self.build(
                self.builder.build_store(storage, parameter),
                "parameter store",
            )?;
            let _ = local;
        }
        if let (Some(debug), Some(scope)) = (self.debug, self.scope) {
            for (id, local) in self.body.locals.iter_enumerated() {
                if local.name.is_none() || matches!(local.kind, LocalKind::Temp | LocalKind::Return)
                {
                    continue;
                }
                let argument = if id.index() >= 1 && id.index() as usize <= self.body.arg_count {
                    Some(id.index())
                } else {
                    None
                };
                debug.declare_local(
                    scope,
                    self.allocas[id.index() as usize],
                    local,
                    argument,
                    self.entry,
                )?;
            }
        }
        Ok(())
    }

    fn emit_terminator(&self, terminator: &seal_mir::Terminator) -> Result<(), CodegenError> {
        match &terminator.kind {
            TerminatorKind::Goto(target) => {
                self.build(
                    self.builder
                        .build_unconditional_branch(self.blocks[target.index() as usize]),
                    "branch",
                )?;
            }
            TerminatorKind::SwitchInt {
                discriminant,
                targets,
                otherwise,
            } => {
                let value = self.load_operand(discriminant)?;
                let value = value.into_int_value();
                let int_ty = value.get_type();
                let mut cases = Vec::with_capacity(targets.len());
                for SwitchTarget {
                    value: case,
                    target,
                } in targets
                {
                    cases.push((
                        self.int_constant(int_ty, *case),
                        self.blocks[target.index() as usize],
                    ));
                }
                self.build(
                    self.builder.build_switch(
                        value,
                        self.blocks[otherwise.index() as usize],
                        &cases,
                    ),
                    "switch",
                )?;
            }
            TerminatorKind::Return => {
                let return_local = Body::return_local();
                let ty = self.mapper.basic(self.body.local(return_local).ty)?;
                let value = self.build(
                    self.builder.build_load(ty, self.allocas[0], "return value"),
                    "return load",
                )?;
                self.build(self.builder.build_return(Some(&value)), "return")?;
            }
            TerminatorKind::Panic { message } => {
                self.panic_at(terminator.span, message)?;
            }
            TerminatorKind::Unreachable => {
                self.build(self.builder.build_unreachable(), "unreachable")?;
            }
        }
        Ok(())
    }

    pub(crate) fn set_span(&self, span: Span) {
        if let Some(location) = self.location(span) {
            self.builder.set_current_debug_location(location);
        }
    }

    pub(crate) fn location(&self, span: Span) -> Option<inkwell::debug_info::DILocation<'ctx>> {
        match (self.debug, self.scope) {
            (Some(debug), Some(scope)) => debug.location(scope, span),
            _ => None,
        }
    }

    pub(crate) fn build<T>(
        &self,
        result: Result<T, BuilderError>,
        operation: &str,
    ) -> Result<T, CodegenError> {
        result
            .map_err(|error| CodegenError::internal(format!("cannot build {operation}: {error:?}")))
    }

    pub(crate) fn int_constant(&self, ty: IntType<'ctx>, value: u128) -> IntValue<'ctx> {
        if ty.get_bit_width() > 64 {
            ty.const_int_arbitrary_precision(&[value as u64, (value >> 64) as u64])
        } else {
            ty.const_int(value as u64, false)
        }
    }

    /// Returns the local slot's alloca.
    pub(crate) fn alloca(&self, local: LocalId) -> PointerValue<'ctx> {
        self.allocas[local.index() as usize]
    }

    /// Allocates a temporary in the entry block and restores the builder.
    ///
    /// Temps created while lowering an expression must not grow the stack each
    /// time the surrounding loop runs, so their storage belongs to the function
    /// prologue even though their value is computed in place.
    pub(crate) fn entry_alloca(
        &self,
        ty: BasicTypeEnum<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let current = self.builder.get_insert_block();
        if let Some(first) = self.entry.get_first_instruction() {
            self.builder.position_before(&first);
        } else {
            self.builder.position_at_end(self.entry);
        }
        let alloca = self.build(self.builder.build_alloca(ty, name), "entry storage")?;
        if let Some(current) = current {
            self.builder.position_at_end(current);
        }
        Ok(alloca)
    }

    /// Returns the type of a local.
    pub(crate) fn local_type(&self, local: LocalId) -> TyId {
        self.body.local(local).ty
    }

    /// Resolves an aggregate definition.
    pub(crate) fn adt_of(&self, def: DefId) -> Result<seal_ty::AdtId, CodegenError> {
        self.hir
            .adt(def)
            .ok_or_else(|| CodegenError::internal(format!("{def:?} is not a struct definition")))
    }
}
