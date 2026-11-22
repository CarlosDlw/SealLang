use seal_def::DefId;
use seal_hash::FxHashMap;
use seal_hir::Hir;
use seal_mir::{
    Body, CallTarget, IntrinsicReceiver, Operand, Place, PlaceRoot, Projection, Rvalue,
    StatementKind, TerminatorKind,
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
                    mark_raw_place(place, &mut direct);
                    // A write to an external static is a write C observes: the
                    // checker owns neither the storage nor the value read back.
                    if matches!(place.root, PlaceRoot::Static(def) if hir.is_extern_static(def)) {
                        direct.unknown = true;
                        direct.touches_raw = true;
                    }
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
                    mark_raw_place(place, &mut direct);
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
            TerminatorKind::PanicValue { message } => {
                collect_operand(hir, message, &mut direct);
            }
            TerminatorKind::Exit { code } => collect_operand(hir, code, &mut direct),
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
            mark_raw_place(place, direct);
            if let Some(path) = GlobalPath::from_place(hir, place) {
                direct.read(path);
            }
        }
        Rvalue::Ref { place, kind } => {
            mark_raw_place(place, direct);
            if let Some(path) = GlobalPath::from_place(hir, place) {
                if kind.is_exclusive() {
                    direct.write(path);
                } else {
                    direct.read(path);
                }
            }
        }
        Rvalue::OwnedView {
            owner,
            source,
            kind,
            ..
        } => {
            mark_raw_place(owner, direct);
            if let Some(path) = GlobalPath::from_place(hir, owner) {
                if kind.is_exclusive() {
                    direct.write(path);
                } else {
                    direct.read(path);
                }
            }
            collect_operand(hir, source, direct);
            // The pointer may name storage the reference model does not track,
            // so this body cannot vouch for `noalias` on its parameters.
            direct.touches_raw = true;
        }
        Rvalue::SliceRef {
            base,
            start,
            end,
            kind,
            ..
        } => {
            mark_raw_place(base, direct);
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
        Rvalue::StrView {
            base, start, end, ..
        } => {
            mark_raw_place(base, direct);
            if let Some(path) = GlobalPath::from_place(hir, base) {
                direct.read(path);
            }
            for operand in start.iter().chain(end.iter()) {
                collect_operand(hir, operand, direct);
            }
        }
        Rvalue::Discriminant(place) => {
            mark_raw_place(place, direct);
            if let Some(path) = GlobalPath::from_place(hir, place) {
                direct.read(path);
            }
        }
        Rvalue::Len { slice } => collect_operand(hir, slice, direct),
        Rvalue::TypeInfo { arg, .. } => {
            if let Some(arg) = arg {
                collect_operand(hir, arg, direct);
            }
        }
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
            if let IntrinsicReceiver::Place(place) = receiver {
                mark_raw_place(place, direct);
                if let Some(path) = GlobalPath::from_place(hir, place) {
                    if method.def().receiver == seal_methods::ReceiverMode::Exclusive {
                        direct.write(path);
                    } else {
                        direct.read(path);
                    }
                }
            }
            if method.def().unsafe_op {
                direct.touches_raw = true;
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
        Rvalue::IndirectCall { callee, args, .. } => {
            collect_operand(hir, callee, direct);
            for arg in args {
                collect_operand(hir, arg, direct);
            }
            // The callable body is outside Seal, or was chosen at run time, so
            // the callee may read and write any storage reachable through the
            // raw arguments.
            direct.unknown = true;
            direct.touches_raw = true;
            calls.push(Vec::new());
        }
        Rvalue::Call { target, args, .. } => {
            for arg in args {
                collect_operand(hir, arg, direct);
            }
            match target {
                CallTarget::Direct { instance, .. }
                | CallTarget::TraitMethod { instance, .. }
                | CallTarget::Closure { instance } => {
                    if hir.is_unsafe_function(instance.def) {
                        direct.touches_raw = true;
                    }
                }
                CallTarget::Extern { .. } => {
                    direct.unknown = true;
                    direct.touches_raw = true;
                }
                // The callee is chosen at run time: its body may reach any
                // static and any raw-memory contract.
                CallTarget::FnPtr { .. } | CallTarget::FnTrait { .. } => {
                    direct.unknown = true;
                    direct.touches_raw = true;
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
                CallTarget::Closure { instance } => {
                    if hir.body_of(instance.def).is_some() {
                        vec![instance.def]
                    } else {
                        Vec::new()
                    }
                }
                CallTarget::Extern { .. }
                | CallTarget::FnPtr { .. }
                | CallTarget::FnTrait { .. } => Vec::new(),
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
    if let Some(place) = operand.place() {
        mark_raw_place(&place, direct);
        // An external static is storage the checker does not own: it may be
        // changed by C between any two Seal operations.
        if matches!(place.root, PlaceRoot::Static(def) if hir.is_extern_static(def)) {
            direct.unknown = true;
            direct.touches_raw = true;
            return;
        }
        if let Some(path) = GlobalPath::from_place(hir, &place) {
            direct.read(path);
        }
    }
}

/// Marks a body that reaches storage through a raw-pointer dereference.
///
/// Raw dereferences are not ordered by the reference model, so a body that
/// uses one cannot lend its exclusive parameters the `noalias` guarantee even
/// when the may-name analysis happens to know the target.
fn mark_raw_place(place: &Place, direct: &mut GlobalEffects) {
    if place
        .projections
        .iter()
        .any(|projection| matches!(projection, Projection::RawDeref))
    {
        direct.touches_raw = true;
    }
}
