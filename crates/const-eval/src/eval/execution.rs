//! Frame execution: statements, terminators, and operand reads.

use super::*;

impl<'a> Interpreter<'a> {
    pub(super) fn eval_body_frame(
        &mut self,
        owner: DefId,
        instance: Instance,
        body: &Body,
        args: &[ConstValue],
    ) -> Result<ConstValue, EvalError> {
        if self.frames.len() >= self.limits.call_depth {
            return Err(self.error(
                EvalErrorKind::DepthLimit {
                    limit: self.limits.call_depth,
                },
                self.def_span(owner),
            ));
        }
        let mut locals = Vec::with_capacity(body.locals.len());
        for (index, local) in body.locals.iter_enumerated() {
            let size = self
                .layout
                .layout_of(local.ty)
                .map(Layout::size)
                .unwrap_or(0);
            let alloc = self.allocate(
                local.ty,
                None,
                true,
                AllocSource::ConstFnLocal {
                    instance,
                    index: index.index(),
                },
                size,
                local.span,
            )?;
            locals.push(alloc);
        }
        for (index, value) in args.iter().enumerate() {
            let local = locals.get(index + 1).copied().ok_or_else(|| {
                self.error(
                    EvalErrorKind::Internal("argument count exceeds locals".into()),
                    self.def_span(owner),
                )
            })?;
            self.set_alloc(local, value.clone(), self.def_span(owner))?;
        }
        self.frames.push(Frame { owner, locals });
        let result = self.run_body(body);
        let frame = self.frames.pop();
        let value = result?;
        let frame = frame.ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("missing evaluation frame".into()),
                self.def_span(owner),
            )
        })?;
        if self.escapes(&value, frame.owner) {
            return Err(self.error(EvalErrorKind::EscapingReference, self.def_span(owner)));
        }
        Ok(value)
    }

    pub(super) fn run_body(&mut self, body: &Body) -> Result<ConstValue, EvalError> {
        let mut block = body.entry;
        loop {
            let basic = &body.blocks[block];
            for statement in &basic.statements {
                self.steps = self.steps.saturating_add(1);
                if self.steps > self.limits.steps {
                    return Err(self.error(
                        EvalErrorKind::StepLimit {
                            limit: self.limits.steps,
                        },
                        statement.span,
                    ));
                }
                let StatementKind::Assign { place, rvalue } = &statement.kind;
                let destination = self.resolve_place(place, statement.span)?;
                let value = self.eval_rvalue(rvalue, statement.span, destination.ty)?;
                self.write_place(&destination, value, statement.span)?;
            }
            self.steps = self.steps.saturating_add(1);
            if self.steps > self.limits.steps {
                return Err(self.error(
                    EvalErrorKind::StepLimit {
                        limit: self.limits.steps,
                    },
                    basic.terminator.span,
                ));
            }
            match &basic.terminator.kind {
                TerminatorKind::Goto(target) => block = *target,
                TerminatorKind::SwitchInt {
                    discriminant,
                    targets,
                    otherwise,
                } => {
                    let value = self.read_operand(discriminant, basic.terminator.span)?;
                    let value = self.as_u128(&value, basic.terminator.span)?;
                    block = targets
                        .iter()
                        .find(|target| target.value == value)
                        .map(|target| target.target)
                        .unwrap_or(*otherwise);
                }
                TerminatorKind::Return => {
                    return self.read_local(Body::return_local(), basic.terminator.span);
                }
                TerminatorKind::Panic { message } => {
                    return Err(
                        self.error(EvalErrorKind::Panic(message.clone()), basic.terminator.span)
                    );
                }
                TerminatorKind::Unreachable => {
                    return Err(self.error(
                        EvalErrorKind::Panic("entered unreachable code".to_owned()),
                        basic.terminator.span,
                    ));
                }
            }
        }
    }

    pub(super) fn read_local(
        &self,
        local: seal_mir::LocalId,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        let frame = self.frames.last().ok_or_else(|| {
            self.error(EvalErrorKind::Internal("no frame for local".into()), span)
        })?;
        let alloc = *frame.locals.get(local.index() as usize).ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("local has no allocation".into()),
                span,
            )
        })?;
        let value = &self.memory[alloc].value;
        value.clone().ok_or_else(|| {
            self.error(
                EvalErrorKind::Internal("read of an uninitialized local".into()),
                span,
            )
        })
    }

    pub(super) fn escapes(&self, value: &ConstValue, owner: DefId) -> bool {
        match value {
            ConstValue::Ref { target, .. }
            | ConstValue::Slice { target, .. }
            | ConstValue::Dyn { data: target, .. } => {
                if let AllocSource::ConstFnLocal { instance, .. } = self.memory[*target].source {
                    if instance.def == owner {
                        return true;
                    }
                }
                false
            }
            ConstValue::Tuple(elements) | ConstValue::Array(elements) => {
                elements.iter().any(|element| self.escapes(element, owner))
            }
            ConstValue::Struct { fields, .. } | ConstValue::Variant { fields, .. } => {
                fields.iter().any(|(_, field)| self.escapes(field, owner))
            }
            ConstValue::Int { .. }
            | ConstValue::Float { .. }
            | ConstValue::Bool(_)
            | ConstValue::Char(_)
            | ConstValue::Unit
            | ConstValue::Str { .. } => false,
        }
    }

    pub(super) fn read_operand(
        &mut self,
        operand: &Operand,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match operand {
            Operand::Constant(constant) => self.constant_value(*constant, span),
            Operand::Const(def) => self.read_const(*def, span),
            Operand::Copy(place) | Operand::Move(place) => {
                let resolved = self.resolve_place(place, span)?;
                self.read_place(&resolved, span)
            }
        }
    }

    pub(super) fn read_const(&mut self, def: DefId, span: Span) -> Result<ConstValue, EvalError> {
        self.record_dependency(def, DependencyKind::Value);
        if self.states.get(&def) == Some(&EvalState::Visiting) {
            return Err(self.error(
                EvalErrorKind::ValueCycle {
                    chain: self.cycle_chain(def),
                },
                span,
            ));
        }
        if let Some(alloc) = self.static_allocs.get(&def).copied() {
            if let Some(value) = self.memory[alloc].value.clone() {
                return Ok(value);
            }
        }
        self.eval_def(def)
    }

    pub(super) fn constant_value(
        &self,
        constant: Constant,
        span: Span,
    ) -> Result<ConstValue, EvalError> {
        match constant {
            Constant::Int { value, ty } => {
                let (bits, _) = self.integer_info(ty, span)?;
                Ok(ConstValue::Int {
                    bits: ops::truncate(value, bits),
                    ty,
                })
            }
            Constant::Float { value, ty } => {
                let width = self.float_width(ty, span)?;
                Ok(ConstValue::Float {
                    bits: ops::float_bits(value, width),
                    ty,
                })
            }
            Constant::Bool(value) => Ok(ConstValue::Bool(value)),
            Constant::Char(value) => Ok(ConstValue::Char(value)),
            Constant::Str { value, ty } => Ok(ConstValue::Str { value, ty }),
            Constant::Unit => Ok(ConstValue::Unit),
        }
    }

    pub(super) fn integer_info(&self, ty: TyId, span: Span) -> Result<(u32, bool), EvalError> {
        let signed = match self.tcx.kind(ty) {
            TyKind::Int(_) => true,
            TyKind::Uint(_) => false,
            _ => return self.internal("expected an integer type", span),
        };
        let bits = self
            .layout
            .layout_of(ty)
            .map(|layout| (layout.size() * 8) as u32)
            .map_err(|error| self.error(EvalErrorKind::Internal(error.to_string()), span))?;
        Ok((bits, signed))
    }

    pub(super) fn float_width(&self, ty: TyId, span: Span) -> Result<u32, EvalError> {
        match self.tcx.kind(ty) {
            TyKind::Float(seal_ty::FloatTy::F32) => Ok(32),
            TyKind::Float(seal_ty::FloatTy::F64) => Ok(64),
            _ => self.internal("expected a floating-point type", span),
        }
    }

    pub(super) fn as_u128(&self, value: &ConstValue, span: Span) -> Result<u128, EvalError> {
        match value {
            ConstValue::Int { bits, .. } => Ok(*bits),
            ConstValue::Char(value) => Ok(u128::from(*value as u32)),
            ConstValue::Bool(value) => Ok(u128::from(*value)),
            _ => self.internal("expected an integer value", span),
        }
    }
}

impl ConstValue {
    pub(super) fn bits(&self) -> u64 {
        match self {
            Self::Int { bits, .. } => *bits as u64,
            Self::Bool(value) => u64::from(*value),
            Self::Char(value) => u64::from(*value as u32),
            _ => 0,
        }
    }
}
