//! Closure expression lowering.
//!
//! A closure is lowered completely at the expression that creates it: its
//! definition, generic scope, body, synthetic environment aggregate, and
//! captures all exist before the enclosing body moves on. Lowering the body
//! here, rather than in a later queue, is what lets name resolution see the
//! lexical scopes that were active at the closure's source position.
//!
//! The body is lowered while the closure's environment frame is on top of the
//! stack, so every outer name it mentions is recorded as a capture. When the
//! body ends, the frame's captures become the environment fields; type checking
//! fills their real types later.

use seal_ast::{ClosureParam, ExprId as AstExprId, TypeId as AstTypeId};
use seal_def::{DefData, DefId, DefKind};
use seal_hash::FxHashMap;
use seal_span::Span;
use seal_ty::{AdtKind, FieldDef, Generics};

use super::Lowering;
use crate::body::{Block, Body, BodyKind, ParamData};
use crate::closure::{CaptureDef, CaptureSource, ClosureDef, ClosureEnv};
use crate::expr::ExprKind;

impl Lowering<'_> {
    /// Lowers one closure expression into [`ExprKind::Closure`].
    ///
    /// The returned node carries the capture list so later phases can read the
    /// environment without another lookup. Every closure gets a definition, a
    /// [`BodyKind::Closure`] body, and a synthetic [`AdtKind::Closure`]
    /// aggregate; [`crate::Hir::closure_for_adt`] maps the aggregate back.
    pub(super) fn lower_closure(
        &mut self,
        span: Span,
        is_move: bool,
        params: &[ClosureParam],
        ret: Option<AstTypeId>,
        body: AstExprId,
    ) -> ExprKind {
        let parent = self.function;
        let name = self.interner.intern("closure");
        let closure = self.define(DefData::new(
            DefKind::Closure,
            name,
            span,
            span,
            parent,
            false,
        ));

        // A closure declares no parameters of its own, but it sees every
        // parameter of the enclosing item. Registering an empty list with the
        // parent link keeps `flattened_generics` working unchanged.
        self.hir
            .ty_ctxt_mut()
            .insert_generics(Generics::new(closure, parent, Vec::new()));
        let env = parent
            .map(|parent| self.hir.param_env(parent))
            .unwrap_or_default();
        self.hir.ty_ctxt_mut().set_param_env(closure, env);

        let adt = self.hir.ty_ctxt().push_adt(closure, name, AdtKind::Closure);
        let generic_count = parent
            .map(|parent| self.hir.flattened_generics(parent).len())
            .unwrap_or(0);
        self.hir
            .ty_ctxt()
            .set_generic_count(adt, generic_count as u32);
        self.hir.insert_adt(closure, adt);
        self.hir.insert_closure(ClosureDef {
            def: closure,
            adt,
            is_move,
            captures: Vec::new(),
        });

        let saved_function = self.function.replace(closure);
        let saved_loop_depth = std::mem::replace(&mut self.loop_depth, 0);
        self.closure_env.push(ClosureEnv {
            closure,
            captures: Vec::new(),
            indices: FxHashMap::default(),
        });

        self.scopes.push();
        let params = params
            .iter()
            .map(|param| self.lower_closure_param(param.clone(), closure))
            .collect();
        let ret = ret.map(|ty| self.lower_type(ty));
        let value = self.lower_expr(body);
        let value_span = self.hir.expr(value).span;
        let root = self.hir.push_block(Block {
            statements: Vec::new(),
            tail: Some(value),
            span: value_span,
        });
        self.scopes.pop();

        let frame = self
            .closure_env
            .pop()
            .expect("the closure frame is on top of the stack");
        let placeholder = self.hir.ty_ctxt().error_ty();
        let fields = frame
            .captures
            .iter()
            .map(|capture| FieldDef {
                def: capture.field,
                name: None,
                ty: placeholder,
            })
            .collect();
        self.hir.ty_ctxt().fill_adt(adt, fields);

        let body = self.hir.push_body(Body {
            owner: closure,
            kind: BodyKind::Closure,
            ret,
            is_const: false,
            is_unsafe: false,
            params,
            root,
        });
        self.hir.set_body_of(closure, body);

        self.loop_depth = saved_loop_depth;
        self.function = saved_function;

        ExprKind::Closure {
            closure,
            captures: frame.captures,
        }
    }

    /// Defines one closure parameter, with the error type when the source
    /// omits its annotation.
    fn lower_closure_param(&mut self, param: ClosureParam, closure: DefId) -> ParamData {
        let ty = match param.ty {
            Some(ty) => self.lower_type(ty),
            None => self.hir.ty_ctxt().error_ty(),
        };
        self.define_param(closure, param.name, param.name_span, param.name_span, ty)
    }

    /// Records `definition` as a capture of the innermost closure.
    ///
    /// Returns `None` when the value belongs to the innermost closure's own
    /// body, which is an ordinary local. Otherwise every closure between the
    /// value's owner and the innermost one receives a capture, so a nested
    /// closure can read the value through its parent's environment.
    pub(super) fn capture_value(&mut self, definition: DefId, span: Span) -> Option<(DefId, u32)> {
        let innermost = self.closure_env.len().checked_sub(1)?;
        let parent = self.hir.definitions().get(definition).parent;
        if parent == Some(self.closure_env[innermost].closure) {
            return None;
        }

        // The frame whose body declares the value, if any. Without one, the
        // value belongs to the function that contains the outermost closure.
        let owner = self
            .closure_env
            .iter()
            .position(|frame| parent == Some(frame.closure));
        let start = owner.map_or(0, |owner| owner + 1);
        debug_assert!(
            start <= innermost,
            "the innermost closure cannot declare the value it captures"
        );

        let mut slot = None;
        for frame in start..=innermost {
            let source = match slot {
                Some(index) => CaptureSource::Outer(index),
                None => CaptureSource::Local(definition),
            };
            slot = Some(self.capture_into(frame, definition, source, span));
        }

        Some((
            self.closure_env[innermost].closure,
            slot.expect("the innermost frame receives the capture"),
        ))
    }

    /// Appends one capture to `frame`, reusing the slot of an equal origin.
    fn capture_into(
        &mut self,
        frame: usize,
        origin: DefId,
        source: CaptureSource,
        span: Span,
    ) -> u32 {
        if let Some(index) = self.closure_env[frame].index_of(origin) {
            debug_assert_eq!(
                self.closure_env[frame].captures[index as usize].source, source,
                "one origin keeps the same source in one environment"
            );
            return index;
        }

        let closure = self.closure_env[frame].closure;
        let name = self.hir.definitions().name(origin);
        let field = self.define(DefData::new(
            DefKind::Field,
            name,
            span,
            span,
            Some(closure),
            false,
        ));
        let binding = self.define(DefData::new(
            DefKind::Capture,
            name,
            span,
            span,
            Some(closure),
            false,
        ));
        let capture = CaptureDef {
            field,
            binding,
            origin,
            source,
        };
        let index = self.hir.push_closure_capture(closure, capture);
        self.closure_env[frame].captures.push(capture);
        self.closure_env[frame].indices.insert(origin, index);
        index
    }

    /// Returns the capture barrier of the body currently being lowered.
    ///
    /// The owners cover the enclosing function and every active closure, so a
    /// static initializer cannot name any of their parameters or bindings.
    pub(crate) fn capture_context(&self) -> CaptureContext {
        let mut owners = Vec::new();
        let mut add = |owner: DefId| {
            if !owners.contains(&owner) {
                owners.push(owner);
            }
        };
        if let Some(function) = self.function {
            add(function);
        }
        for frame in &self.closure_env {
            add(frame.closure);
            if let Some(parent) = self.hir.definitions().get(frame.closure).parent {
                add(parent);
            }
        }
        CaptureContext { owners }
    }
}

/// One local static's capture barrier.
///
/// The barrier is created at the declaration and travels with the value body,
/// which clears the closure stack: a static initializer never captures.
#[derive(Clone)]
pub(crate) struct CaptureContext {
    /// The body owners whose parameters and bindings cannot be named from the
    /// initializer: the enclosing function and every active closure.
    owners: Vec<DefId>,
}

impl CaptureContext {
    /// Returns `true` when a definition owned by `owner` cannot be named.
    pub(crate) fn forbids(&self, owner: DefId) -> bool {
        self.owners.contains(&owner)
    }
}
