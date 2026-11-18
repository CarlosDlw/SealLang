//! Tests for deterministic vtable layout.

use seal_def::DefId;
use seal_symbol::Interner;
use seal_ty::{
    GenericParamDef, GenericSignature, Generics, IntTy, TraitDef, TraitMethodDef, TraitRef, TyCtxt,
    TyId,
};

use super::{VTABLE_HEADER_SLOTS, VtableEntryKind, VtableLayout, VtableLayoutError, VtableSlot};

fn def(index: u32) -> DefId {
    DefId::from_u32(index).expect("test definitions are small")
}

fn insert_trait(
    ctxt: &TyCtxt,
    interner: &Interner,
    index: u32,
    name: &str,
    methods: &[(&str, u32)],
) -> (DefId, TyId) {
    let owner = def(index);
    let self_param = ctxt.push_param(def(index + 1), owner, 0, interner.intern("Self"));
    let generic = vec![
        GenericParamDef::new(def(index + 1), self_param, interner.intern("Self"), 0).as_self(),
    ];
    let method_defs = methods
        .iter()
        .map(|(name, method)| {
            TraitMethodDef::new(
                def(*method),
                interner.intern(name),
                GenericSignature::new(Vec::new(), ctxt.unit_ty()),
                Generics::new(def(*method), Some(owner), Vec::new()),
                None,
            )
        })
        .collect();
    ctxt.insert_trait_def(TraitDef::new(
        owner,
        interner.intern(name),
        Generics::new(owner, None, generic),
        Vec::new(),
        method_defs,
    ));
    (owner, ctxt.param_ty(self_param))
}

fn set_supertraits(ctxt: &TyCtxt, trait_def: DefId, supertraits: Vec<TraitRef>) {
    let mut definition = ctxt.trait_def(trait_def).expect("the test trait exists");
    definition.supertraits = supertraits;
    ctxt.insert_trait_def(definition);
}

fn reference(ctxt: &TyCtxt, trait_def: DefId, self_ty: TyId) -> TraitRef {
    TraitRef::new(trait_def, self_ty, ctxt.empty_substs())
}

fn int(ctxt: &TyCtxt) -> TyId {
    ctxt.int_ty(IntTy::I32)
}

#[test]
fn vtable_header_has_three_slots() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (shape, self_ty) = insert_trait(&ctxt, &interner, 100, "Shape", &[]);
    let layout = VtableLayout::new(&ctxt, reference(&ctxt, shape, self_ty))
        .expect("an empty trait has a header-only vtable");

    assert_eq!(layout.entries().len(), VTABLE_HEADER_SLOTS as usize);
    assert_eq!(layout.entries()[0].slot, VtableSlot::DROP);
    assert_eq!(layout.entries()[1].slot, VtableSlot::SIZE);
    assert_eq!(layout.entries()[2].slot, VtableSlot::ALIGN);
    assert_eq!(layout.entries()[0].kind, VtableEntryKind::Drop);
    assert_eq!(layout.entries()[1].kind, VtableEntryKind::Size);
    assert_eq!(layout.entries()[2].kind, VtableEntryKind::Align);
    assert_eq!(layout.method_count(), 0);
}

#[test]
fn principal_methods_keep_declaration_order() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (shape, self_ty) = insert_trait(
        &ctxt,
        &interner,
        100,
        "Shape",
        &[("area", 200), ("scale", 201), ("name", 202)],
    );
    let layout = VtableLayout::new(&ctxt, reference(&ctxt, shape, self_ty))
        .expect("the principal is object safe");

    let methods: Vec<DefId> = layout
        .method_entries()
        .map(|entry| entry.method().expect("a method entry"))
        .collect();
    assert_eq!(methods, vec![def(200), def(201), def(202)]);
    assert_eq!(layout.slot_of(def(200)), Some(VtableSlot::new(3)));
    assert_eq!(layout.slot_of(def(201)), Some(VtableSlot::new(4)));
    assert_eq!(layout.slot_of(def(202)), Some(VtableSlot::new(5)));
}

#[test]
fn supertrait_methods_follow_the_principal() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (shape, self_ty) = insert_trait(&ctxt, &interner, 100, "Shape", &[("area", 200)]);
    let (named, named_self) = insert_trait(&ctxt, &interner, 150, "Named", &[("name", 300)]);
    set_supertraits(&ctxt, shape, vec![reference(&ctxt, named, named_self)]);
    let layout = VtableLayout::new(&ctxt, reference(&ctxt, shape, self_ty))
        .expect("the supertrait is object safe");

    let methods: Vec<DefId> = layout
        .method_entries()
        .map(|entry| entry.method().expect("a method entry"))
        .collect();
    assert_eq!(methods, vec![def(200), def(300)]);
    assert_eq!(layout.slot_of(def(200)), Some(VtableSlot::new(3)));
    assert_eq!(layout.slot_of(def(300)), Some(VtableSlot::new(4)));
}

#[test]
fn diamond_supertraits_share_one_method_slot() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (root, _root_self) = insert_trait(&ctxt, &interner, 100, "Root", &[("root", 400)]);
    let (left, left_self) = insert_trait(&ctxt, &interner, 150, "Left", &[]);
    let (right, right_self) = insert_trait(&ctxt, &interner, 200, "Right", &[]);
    let (leaf, leaf_self) = insert_trait(&ctxt, &interner, 250, "Leaf", &[("leaf", 500)]);
    set_supertraits(&ctxt, left, vec![reference(&ctxt, root, left_self)]);
    set_supertraits(&ctxt, right, vec![reference(&ctxt, root, right_self)]);
    set_supertraits(
        &ctxt,
        leaf,
        vec![
            reference(&ctxt, left, leaf_self),
            reference(&ctxt, right, leaf_self),
        ],
    );

    let layout = VtableLayout::new(&ctxt, reference(&ctxt, leaf, leaf_self))
        .expect("the diamond is well formed");
    let methods: Vec<DefId> = layout
        .method_entries()
        .map(|entry| entry.method().expect("a method entry"))
        .collect();
    assert_eq!(methods, vec![def(500), def(400)]);
    assert_eq!(layout.method_count(), 2);
    assert_eq!(
        layout
            .method_entries()
            .filter(|entry| entry.method() == Some(def(400)))
            .count(),
        1
    );
}

#[test]
fn principal_shadow_keeps_both_method_slots() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (shape, self_ty) = insert_trait(&ctxt, &interner, 100, "Shape", &[("name", 200)]);
    let (named, named_self) = insert_trait(&ctxt, &interner, 150, "Named", &[("name", 300)]);
    set_supertraits(&ctxt, shape, vec![reference(&ctxt, named, named_self)]);

    let layout = VtableLayout::new(&ctxt, reference(&ctxt, shape, self_ty))
        .expect("both declarations are distinct");
    assert_eq!(layout.method_count(), 2);
    assert_eq!(layout.slot_of(def(200)), Some(VtableSlot::new(3)));
    assert_eq!(layout.slot_of(def(300)), Some(VtableSlot::new(4)));
}

#[test]
fn method_slots_are_stable_for_the_same_trait() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (shape, self_ty) = insert_trait(&ctxt, &interner, 100, "Shape", &[("area", 200)]);
    let principal = reference(&ctxt, shape, self_ty);

    let first = VtableLayout::new(&ctxt, principal).expect("the trait is complete");
    let second = VtableLayout::new(&ctxt, principal).expect("the trait is complete");
    assert_eq!(first, second);
    assert_eq!(first.slot_of(def(200)), second.slot_of(def(200)));
}

#[test]
fn generic_trait_arguments_do_not_change_the_slot_count() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let owner = def(100);
    let self_param = ctxt.push_param(def(101), owner, 0, interner.intern("Self"));
    let argument = ctxt.push_param(def(102), owner, 1, interner.intern("T"));
    let method = TraitMethodDef::new(
        def(200),
        interner.intern("convert"),
        GenericSignature::new(Vec::new(), ctxt.unit_ty()),
        Generics::new(def(200), Some(owner), Vec::new()),
        None,
    );
    ctxt.insert_trait_def(TraitDef::new(
        owner,
        interner.intern("Convert"),
        Generics::new(
            owner,
            None,
            vec![
                GenericParamDef::new(def(101), self_param, interner.intern("Self"), 0).as_self(),
                GenericParamDef::new(def(102), argument, interner.intern("T"), 1),
            ],
        ),
        Vec::new(),
        vec![method],
    ));

    let self_ty = int(&ctxt);
    let int_object = VtableLayout::new(
        &ctxt,
        TraitRef::new(owner, self_ty, ctxt.substs(&[int(&ctxt)])),
    )
    .expect("the instantiation is complete");
    let bool_object = VtableLayout::new(
        &ctxt,
        TraitRef::new(owner, self_ty, ctxt.substs(&[ctxt.bool_ty()])),
    )
    .expect("the instantiation is complete");

    assert_eq!(int_object.method_count(), 1);
    assert_eq!(int_object.method_count(), bool_object.method_count());
    assert_eq!(int_object.slot_of(def(200)), bool_object.slot_of(def(200)));
    let declaring = int_object
        .method_entries()
        .next()
        .and_then(|entry| entry.declaring_trait())
        .expect("the method entry names its trait");
    assert_eq!(declaring.args, ctxt.substs(&[int(&ctxt)]));
}

#[test]
fn missing_trait_is_a_typed_error_not_a_panic() {
    let ctxt = TyCtxt::new();
    let principal = TraitRef::new(def(999), int(&ctxt), ctxt.empty_substs());
    assert_eq!(
        VtableLayout::new(&ctxt, principal),
        Err(VtableLayoutError::MissingTrait(def(999)))
    );
}

#[test]
fn cyclic_supertraits_are_a_typed_error() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (first, first_self) = insert_trait(&ctxt, &interner, 100, "First", &[]);
    let (second, second_self) = insert_trait(&ctxt, &interner, 150, "Second", &[]);
    set_supertraits(&ctxt, first, vec![reference(&ctxt, second, first_self)]);
    set_supertraits(&ctxt, second, vec![reference(&ctxt, first, second_self)]);

    assert_eq!(
        VtableLayout::new(&ctxt, reference(&ctxt, first, first_self)),
        Err(VtableLayoutError::CyclicSupertrait(first))
    );
}

#[test]
fn malformed_supertrait_arity_is_a_typed_error() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (shape, self_ty) = insert_trait(&ctxt, &interner, 100, "Shape", &[]);
    let (named, _) = insert_trait(&ctxt, &interner, 150, "Named", &[]);
    set_supertraits(&ctxt, shape, vec![reference(&ctxt, named, self_ty)]);

    let mut definition = ctxt.trait_def(shape).expect("the test trait exists");
    definition.supertraits[0].args = ctxt.substs(&[int(&ctxt)]);
    ctxt.insert_trait_def(definition);

    assert_eq!(
        VtableLayout::new(&ctxt, reference(&ctxt, shape, self_ty)),
        Err(VtableLayoutError::MalformedTraitRef {
            trait_def: named,
            expected: 0,
            found: 1,
        })
    );
}

#[test]
fn for_object_uses_the_principal_trait_reference() {
    let ctxt = TyCtxt::new();
    let interner = Interner::new();
    let (shape, _) = insert_trait(&ctxt, &interner, 100, "Shape", &[("area", 200)]);
    let object = ctxt.dyn_ty(shape, ctxt.empty_substs());
    let layout = VtableLayout::for_object(&ctxt, object).expect("the object has a layout");
    assert_eq!(layout.principal().trait_def, shape);
    assert_eq!(layout.principal().self_ty, object);
    assert_eq!(layout.slot_of(def(200)), Some(VtableSlot::new(3)));
}
