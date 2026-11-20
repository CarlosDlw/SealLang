//! Evaluation and validation of `cfg(...)` conditions for one package instance.
//!
//! The AST carries a typed expression; the project layer supplies the facts.
//! Inactive items and imports never reach name resolution, so code behind a
//! false condition is not typechecked. An `env("NAME")` condition is valid only
//! when the package manifest declared that value, which prevents a misspelled
//! environment variable from silently evaluating to false.

use std::collections::{BTreeMap, BTreeSet};

use seal_ast::{Ast, CfgExprId, CfgExprKind, CfgLiteral};
use seal_diagnostics::DiagCtxt;
use seal_error_codes as codes;
use seal_hash::FxHashSet;
use seal_span::FileId;
use seal_symbol::Interner;

use crate::PackagePlan;

/// The configuration facts for one package instance.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CfgEnvironment {
    /// Enabled features.
    pub features: BTreeSet<String>,
    /// Target operating system.
    pub target_os: String,
    /// Target architecture.
    pub target_arch: String,
    /// Target environment.
    pub target_env: String,
    /// Target pointer width.
    pub pointer_width: u8,
    /// Whether debug assertions are enabled.
    pub debug_assertions: bool,
    /// Whether this is a test instance.
    pub test: bool,
    /// Whether this is a benchmark instance.
    pub bench: bool,
    /// Declared environment values.
    pub env: BTreeMap<String, String>,
}

impl CfgEnvironment {
    /// Evaluates a condition.
    pub fn evaluate(&self, ast: &Ast, id: CfgExprId, interner: &Interner) -> bool {
        self.value(ast, id, interner)
            .is_some_and(|value| value.truthy())
    }

    fn value(&self, ast: &Ast, id: CfgExprId, interner: &Interner) -> Option<CfgValue> {
        match &ast.cfg_expr(id).kind {
            CfgExprKind::Feature(feature) => Some(CfgValue::Bool(
                self.features.contains(interner.resolve(*feature)),
            )),
            CfgExprKind::Env(name) => self
                .env
                .get(interner.resolve(*name))
                .cloned()
                .map(CfgValue::Str),
            CfgExprKind::Target(attribute) => {
                let value = match interner.resolve(*attribute) {
                    "os" => Some(self.target_os.clone()),
                    "arch" => Some(self.target_arch.clone()),
                    "env" => Some(self.target_env.clone()),
                    "pointer_width" => Some(self.pointer_width.to_string()),
                    _ => None,
                }?;
                Some(CfgValue::Str(value))
            }
            CfgExprKind::DebugAssertions => Some(CfgValue::Bool(self.debug_assertions)),
            CfgExprKind::Test => Some(CfgValue::Bool(self.test)),
            CfgExprKind::Bench => Some(CfgValue::Bool(self.bench)),
            CfgExprKind::Literal(literal) => Some(match literal {
                CfgLiteral::Str(value) => CfgValue::Str(interner.resolve(*value).to_owned()),
                CfgLiteral::Bool(value) => CfgValue::Bool(*value),
                CfgLiteral::Int(value) => CfgValue::Int(*value),
            }),
            CfgExprKind::Compare { lhs, op, rhs } => {
                let left = self.value(ast, *lhs, interner)?;
                let right = self.value(ast, *rhs, interner)?;
                let equal = left == right;
                Some(CfgValue::Bool(match op {
                    seal_ast::CfgCmpOp::Eq => equal,
                    seal_ast::CfgCmpOp::Ne => !equal,
                }))
            }
            CfgExprKind::And { lhs, rhs } => Some(CfgValue::Bool(
                self.evaluate(ast, *lhs, interner) && self.evaluate(ast, *rhs, interner),
            )),
            CfgExprKind::Or { lhs, rhs } => Some(CfgValue::Bool(
                self.evaluate(ast, *lhs, interner) || self.evaluate(ast, *rhs, interner),
            )),
            CfgExprKind::Not(operand) => {
                Some(CfgValue::Bool(!self.evaluate(ast, *operand, interner)))
            }
            CfgExprKind::Error => Some(CfgValue::Bool(false)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CfgValue {
    Bool(bool),
    Str(String),
    Int(u128),
}

impl CfgValue {
    fn truthy(&self) -> bool {
        match self {
            Self::Bool(value) => *value,
            Self::Str(value) => !value.is_empty(),
            Self::Int(value) => *value != 0,
        }
    }
}

/// Returns whether a file's `module` declaration is active.
///
pub(crate) fn module_enabled(ast: &Ast, cfg: &CfgEnvironment, interner: &Interner) -> bool {
    for &item_id in ast.program().items() {
        let item = ast.item(item_id);
        if let seal_ast::ItemKind::Module(_) = &item.kind {
            return item
                .modifiers
                .cfg
                .is_none_or(|condition| cfg.evaluate(ast, condition, interner));
        }
    }
    true
}

/// Renders a condition back to source form for API snapshots.
pub(crate) fn format(ast: &Ast, id: CfgExprId, interner: &Interner) -> String {
    match &ast.cfg_expr(id).kind {
        CfgExprKind::Feature(value) => {
            format!("feature({:?})", interner.resolve(*value))
        }
        CfgExprKind::Env(value) => format!("env({:?})", interner.resolve(*value)),
        CfgExprKind::Target(value) => format!("target.{}", interner.resolve(*value)),
        CfgExprKind::DebugAssertions => "debug_assertions".to_owned(),
        CfgExprKind::Test => "test".to_owned(),
        CfgExprKind::Bench => "bench".to_owned(),
        CfgExprKind::Literal(literal) => match literal {
            CfgLiteral::Str(value) => format!("{:?}", interner.resolve(*value)),
            CfgLiteral::Bool(value) => value.to_string(),
            CfgLiteral::Int(value) => value.to_string(),
        },
        CfgExprKind::Compare { lhs, op, rhs } => format!(
            "{} {} {}",
            format(ast, *lhs, interner),
            op.as_str(),
            format(ast, *rhs, interner)
        ),
        CfgExprKind::And { lhs, rhs } => format!(
            "{} && {}",
            format(ast, *lhs, interner),
            format(ast, *rhs, interner)
        ),
        CfgExprKind::Or { lhs, rhs } => format!(
            "{} || {}",
            format(ast, *lhs, interner),
            format(ast, *rhs, interner)
        ),
        CfgExprKind::Not(operand) => format!("!{}", format(ast, *operand, interner)),
        CfgExprKind::Error => "cfg(error)".to_owned(),
    }
}

/// Reports `env("NAME")` conditions that the manifest did not declare.
pub(crate) fn validate_env(
    plan: &PackagePlan,
    asts: &[(FileId, &Ast)],
    dcx: &DiagCtxt,
    interner: &Interner,
) {
    let by_file: BTreeMap<FileId, &CfgEnvironment> = plan
        .packages
        .iter()
        .flat_map(|package| {
            package
                .files
                .iter()
                .map(move |file| (file.file, &package.cfg))
        })
        .collect();
    let mut seen: FxHashSet<(u32, CfgExprId)> = FxHashSet::default();
    for (file, ast) in asts {
        let Some(environment) = by_file.get(file) else {
            continue;
        };
        for (id, expression) in ast.cfg_expr_arena().iter_enumerated() {
            if !seen.insert((file.index(), id)) {
                continue;
            }
            let CfgExprKind::Env(name) = &expression.kind else {
                continue;
            };
            let name = interner.resolve(*name);
            if !environment.env.contains_key(name) {
                dcx.error(format!(
                    "`cfg(env(\"{name}\"))` names an environment value that the manifest does not declare"
                ))
                .code(codes::E2084.diag_code())
                .primary(expression.span, "this condition cannot be evaluated")
                .help(format!(
                    "declare `{name}` in `[env]` with `tracked = true`, or remove the condition"
                ))
                .emit();
            }
        }
    }
}
