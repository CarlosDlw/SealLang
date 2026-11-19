use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_mir::{
    Body, CallTarget, IntrinsicReceiver, Operand, Rvalue, StatementKind, TerminatorKind,
};

use super::{BodyEffects, GlobalEffects, GlobalPath};

/// Collects one body's direct effects and call candidates.
pub(super) fn collect(
    hir: &Hir,
    body: &Body,
    trait_candidates: &FxHashMap<DefId, Vec<DefId>>,
    dynamic_candidates: &FxHashMap<DefId, Vec<DefId>>,
) -> BodyEffects {
    let mut direct = GlobalEffects::default();
    let mut calls = Vec::new();
    for block in body.blocks.iter() {
        for statement in &block.statements {
            match &statement.kind {
                StatementKind::Assign { place, rvalue } => {
                    if let Some(path) = GlobalPath::from_place(hir, place) {
                        direct.write(path);
                    }
                    collect_rvalue(
                        hir,
                        rvalue,
                        trait_candidates,
                        dynamic_candidates,
                        &mut direct,
                        &mut calls,
                    );
                }
                StatementKind::Drop { place, .. } | StatementKind::DropRemaining { place, .. } => {
                    // A destructor is user code that may read or write
                    // `static mut` storage. It runs from drop glue rather than
                    // from a call this analysis can follow, so a drop that can
                    // reach a destructor makes this body's effect summary
                    // unknown. A body that only drops `Copy`-like values keeps
                    // its precise summary.
                    if hir
                        .ty_ctxt()
                        .needs_drop(crate::origins::place_ty(hir, body, place))
                    {
                        direct.unknown = true;
                        direct.touches_raw = true;
                    }
                }
            }
        }
        match &block.terminator.kind {
            TerminatorKind::SwitchInt { discriminant, .. } => {
                collect_operand(hir, discriminant, &mut direct);
            }
            TerminatorKind::Goto(_)
            | TerminatorKind::Return
            | TerminatorKind::Panic { .. }
            | TerminatorKind::Unreachable => {}
        }
    }
    BodyEffects {
        owner: body.owner,
        direct,
        calls,
    }
}

/// Collects the effects of one rvalue and the callees of its calls.
pub(super) fn collect_rvalue(
    hir: &Hir,
    rvalue: &Rvalue,
    trait_candidates: &FxHashMap<DefId, Vec<DefId>>,
    dynamic_candidates: &FxHashMap<DefId, Vec<DefId>>,
    direct: &mut GlobalEffects,
    calls: &mut Vec<Vec<DefId>>,
) {
    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast { operand, .. } => {
            collect_operand(hir, operand, direct);
        }
        Rvalue::MakeTraitObject { source, .. } => collect_operand(hir, source, direct),
        Rvalue::TraitObjectReborrow { source, place, .. } => {
            collect_operand(hir, source, direct);
            if let Some(path) = GlobalPath::from_place(hir, place) {
                direct.read(path);
            }
        }
        Rvalue::Ref { place, kind } => {
            if let Some(path) = GlobalPath::from_place(hir, place) {
                if kind.is_exclusive() {
                    direct.write(path);
                } else {
                    direct.read(path);
                }
            }
        }
        Rvalue::SliceRef {
            base,
            start,
            end,
            kind,
            ..
        } => {
            if let Some(path) = GlobalPath::from_place(hir, base) {
                if kind.is_exclusive() {
                    direct.write(path);
                } else {
                    direct.read(path);
                }
            }
            for operand in start.iter().chain(end.iter()) {
                collect_operand(hir, operand, direct);
            }
        }
        Rvalue::Discriminant(place) => {
            if let Some(path) = GlobalPath::from_place(hir, place) {
                direct.read(path);
            }
        }
        Rvalue::Len { slice } => collect_operand(hir, slice, direct),
        Rvalue::Binary { lhs, rhs, .. } => {
            collect_operand(hir, lhs, direct);
            collect_operand(hir, rhs, direct);
        }
        Rvalue::Unary { operand, .. } => collect_operand(hir, operand, direct),
        Rvalue::Intrinsic {
            method,
            receiver,
            args,
            ..
        } => {
            if let IntrinsicReceiver::Place(place) = receiver
                && let Some(path) = GlobalPath::from_place(hir, place)
            {
                if method.def().receiver == seal_methods::ReceiverMode::Exclusive {
                    direct.write(path);
                } else {
                    direct.read(path);
                }
            }
            if let IntrinsicReceiver::Value(operand) = receiver {
                collect_operand(hir, operand, direct);
            }
            for arg in args {
                collect_operand(hir, arg, direct);
            }
        }
        Rvalue::DynamicCall {
            method,
            receiver,
            args,
            ..
        } => {
            collect_operand(hir, receiver, direct);
            for arg in args {
                collect_operand(hir, arg, direct);
            }
            if hir.is_unsafe_function(*method) {
                direct.touches_raw = true;
            }
            calls.push(dynamic_candidates.get(method).cloned().unwrap_or_default());
        }
        Rvalue::Call { target, args, .. } => {
            for arg in args {
                collect_operand(hir, arg, direct);
            }
            match target {
                CallTarget::Direct { instance, .. } | CallTarget::TraitMethod { instance, .. } => {
                    if hir.is_unsafe_function(instance.def) {
                        direct.touches_raw = true;
                    }
                }
            }
            let candidates = match target {
                CallTarget::Direct { instance, .. } => {
                    if hir.body_of(instance.def).is_some() {
                        vec![instance.def]
                    } else {
                        Vec::new()
                    }
                }
                CallTarget::TraitMethod { instance, .. } => trait_candidates
                    .get(&instance.def)
                    .cloned()
                    .unwrap_or_default(),
            };
            calls.push(candidates);
        }
        Rvalue::Aggregate { fields, .. } => {
            for (_, operand) in fields {
                collect_operand(hir, operand, direct);
            }
        }
        Rvalue::Tuple { elements, .. } | Rvalue::Array { elements, .. } => {
            for operand in elements {
                collect_operand(hir, operand, direct);
            }
        }
        Rvalue::Repeat { value, .. } => collect_operand(hir, value, direct),
    }
}

/// Records a static mut read when an operand names one.
fn collect_operand(hir: &Hir, operand: &Operand, direct: &mut GlobalEffects) {
    if let Some(place) = operand.place()
        && let Some(path) = GlobalPath::from_place(hir, &place)
    {
        direct.read(path);
    }
}
