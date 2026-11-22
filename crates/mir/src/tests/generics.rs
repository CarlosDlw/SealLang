//! Generic bodies in MIR: parameters, instantiated aggregates, and calls.

use seal_def::DefKind;

use crate::body::Body;
use crate::operand::Operand;
use crate::rvalue::{AggregateKind, CallTarget, Rvalue};
use crate::statement::StatementKind;

use super::support::{assignment, function, lower_clean};

/// Returns the first call rvalue in `body`.
fn call(body: &Body) -> &Rvalue {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(|statement| assignment(statement).1)
        .find(|rvalue| matches!(rvalue, Rvalue::Call { .. }))
        .expect("the body contains a call")
}

/// Returns the first aggregate rvalue in `body`.
fn aggregate(body: &Body) -> &Rvalue {
    body.blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(|statement| assignment(statement).1)
        .find(|rvalue| matches!(rvalue, Rvalue::Aggregate { .. }))
        .expect("the body contains an aggregate")
}

#[test]
fn a_generic_body_keeps_its_parameter_type_and_has_no_inference_variable() {
    let source = "func identity<T>(value: T) -> T { value } \
                  func main() -> i32 { identity(1) }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "identity");
    let argument = body.args().next().expect("identity takes one argument");
    assert_eq!(fixture.ty_text(&hir, argument.ty), "T");

    let main = function(&fixture, &hir, &mir, "main");
    let Rvalue::Call { target, args, .. } = call(main) else {
        panic!("main calls identity");
    };
    assert!(matches!(target, CallTarget::Direct { .. }));
    assert_eq!(target.parameters(), 1);
    let instance = target.instance();
    let arguments: Vec<String> = hir
        .ty_ctxt()
        .substs_args(instance.args)
        .iter()
        .map(|argument| fixture.ty_text(&hir, *argument))
        .collect();
    assert_eq!(arguments, ["i32"], "the concrete call is instantiated");
    assert!(args.is_empty() || args.len() == 1);
}

#[test]
fn a_generic_struct_literal_carries_instantiated_arguments() {
    let source = "struct Wrapper<T> { value: T } \
                  func wrap<T>(value: T) -> Wrapper<T> { Wrapper { value: value } }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "wrap");
    let Rvalue::Aggregate { kind, .. } = aggregate(body) else {
        panic!("wrap builds a struct");
    };
    let AggregateKind::Struct { adt, args } = *kind else {
        panic!("expected a struct aggregate, got {kind:?}");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(adt)), "Wrapper");
    let arguments: Vec<String> = hir
        .ty_ctxt()
        .substs_args(args)
        .iter()
        .map(|argument| fixture.ty_text(&hir, *argument))
        .collect();
    assert_eq!(arguments, ["T"]);
}

#[test]
fn an_unsafe_cast_into_an_inferred_generic_literal_lowers_to_a_concrete_temporary() {
    let source = "struct Holder<T> { pointer: *raw mut T } \
                  func main() -> i32 { \
                      let holder = Holder { pointer: unsafe { 0usize as *raw mut i32 } }; \
                      if holder.pointer.is_null() { 0 } else { 1 } \
                  }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let Rvalue::Cast { to, .. } = body
        .blocks
        .iter()
        .flat_map(|block| &block.statements)
        .map(|statement| assignment(statement).1)
        .find(|rvalue| matches!(rvalue, Rvalue::Cast { .. }))
        .expect("main lowers the cast")
    else {
        panic!("expected a cast rvalue");
    };
    assert_eq!(fixture.ty_text(&hir, *to), "*raw mut i32");
}

#[test]
fn a_generic_variant_carries_instantiated_arguments() {
    let source = "enum Maybe<T> { Empty, Full(T) } \
                  func full<T>(value: T) -> Maybe<T> { Maybe::Full(value) }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "full");
    let Rvalue::Aggregate { kind, .. } = aggregate(body) else {
        panic!("full builds a variant");
    };
    let AggregateKind::Variant { adt, args, .. } = *kind else {
        panic!("expected a variant aggregate, got {kind:?}");
    };
    assert_eq!(fixture.symbol(hir.definitions().name(adt)), "Maybe");
    assert_eq!(
        hir.ty_ctxt()
            .substs_args(args)
            .iter()
            .map(|argument| fixture.ty_text(&hir, *argument))
            .collect::<Vec<_>>(),
        ["T"]
    );
}

#[test]
fn a_generic_field_projects_with_the_instantiated_field_type() {
    let source = "struct Wrapper<T> { value: T } \
                  func unwrap<T>(wrapper: *Wrapper<T>) -> T { (*wrapper).value }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "unwrap");
    let Rvalue::Use(Operand::Copy(place)) = assignment(&body.block(body.entry).statements[0]).1
    else {
        panic!("the tail expression reads the field");
    };
    assert_eq!(place.projections.len(), 2, "a dereference and a field");
    assert_eq!(
        fixture.ty_text(&hir, body.local(Body::return_local()).ty),
        "T",
        "the field type is resolved through the instantiated arguments"
    );
}

#[test]
fn nested_generic_fields_resolve_in_order() {
    let source = "struct Pair<A, B> { first: A, second: B } \
                  func nested<T, U>(pair: Pair<Pair<T, U>, T>) -> T { pair.first.first }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "nested");
    let Rvalue::Use(Operand::Copy(place) | Operand::Move(place)) =
        assignment(&body.block(body.entry).statements[0]).1
    else {
        panic!("the tail expression reads a nested field");
    };
    assert_eq!(place.projections.len(), 2);
    assert_eq!(
        fixture.ty_text(&hir, body.local(Body::return_local()).ty),
        "T"
    );
}

#[test]
fn a_generic_method_call_carries_its_instantiation() {
    let source = "struct Wrapper<T> { value: T } \
                  impl<T> Wrapper<T> { \
                      func get(self: *Wrapper<T>) -> T { (*self).value } \
                  } \
                  func main() -> i32 { \
                      let wrapper = Wrapper { value: 1 }; \
                      wrapper.get() \
                  }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    let Rvalue::Call { target, args, .. } = call(body) else {
        panic!("main calls the method");
    };
    assert!(matches!(target, CallTarget::Direct { .. }));
    assert_eq!(target.parameters(), 1, "the receiver is the only parameter");
    assert_eq!(args.len(), 1);
    assert_eq!(
        hir.ty_ctxt()
            .substs_args(target.instance().args)
            .iter()
            .map(|argument| fixture.ty_text(&hir, *argument))
            .collect::<Vec<_>>(),
        ["i32"]
    );
    assert!(
        body.blocks
            .iter()
            .flat_map(|block| &block.statements)
            .any(|statement| matches!(assignment(statement).1, Rvalue::Ref { .. })),
        "the shared receiver is an explicit borrow"
    );
}

#[test]
fn a_bound_method_call_stays_symbolic() {
    let source = "trait Show { func show(self: *Self) -> i32; } \
                  func show_one<T: Show>(value: *T) -> i32 { value.show() }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "show_one");
    let Rvalue::Call { target, args, .. } = call(body) else {
        panic!("show_one calls the bound method");
    };
    let CallTarget::TraitMethod {
        instance,
        parameters,
    } = target
    else {
        panic!("expected a symbolic target, got {target:?}");
    };
    assert_eq!(*parameters, 1);
    assert_eq!(hir.definitions().kind(instance.def), DefKind::TraitMethod);
    assert_eq!(args.len(), 1);
}

#[test]
fn a_type_parameter_receiver_is_valid_for_a_symbolic_call() {
    let source = "trait Take { func take(self: Self) -> i32; } \
                  func take_one<T: Take>(value: T) -> i32 { value.take() }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "take_one");
    let Rvalue::Call { target, .. } = call(body) else {
        panic!("take_one calls the bound method");
    };
    assert!(target.is_trait_method());
}

#[test]
fn a_generic_associated_function_may_take_a_parameter_by_value() {
    let source = "struct Holder<T> { value: T } \
                  impl<T> Holder<T> { \
                      func keep(value: T) -> T { value } \
                  } \
                  func keep_one<T>(value: T) -> T { Holder::keep(value) }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "keep_one");
    let Rvalue::Call { target, .. } = call(body) else {
        panic!("keep_one calls the associated function");
    };
    assert!(matches!(target, CallTarget::Direct { .. }));
}

#[test]
fn a_symbolic_associated_function_call_stays_symbolic() {
    let source = "trait Make { func make() -> i32; } \
                  func build<T: Make>() -> i32 { T::make() }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "build");
    let Rvalue::Call { target, args, .. } = call(body) else {
        panic!("build calls the associated function");
    };
    assert!(target.is_trait_method());
    assert_eq!(target.parameters(), 0);
    assert!(args.is_empty());
}

#[test]
fn a_unit_call_statement_keeps_the_argument_spans() {
    let source = "func sink<T>(value: T) { } func main() { sink(1); }";
    let (fixture, hir, mir) = lower_clean(source);
    let body = function(&fixture, &hir, &mir, "main");
    for statement in body.blocks.iter().flat_map(|block| &block.statements) {
        if let StatementKind::Assign {
            rvalue: Rvalue::Call {
                args, arg_spans, ..
            },
            ..
        } = &statement.kind
        {
            assert_eq!(args.len(), 1);
            assert_eq!(arg_spans.len(), 1);
            return;
        }
    }
    panic!("main has no call");
}
