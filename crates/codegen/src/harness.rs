//! Generated entry points for test and benchmark executables.
//!
//! A test target has no user-written `main`. Code generation synthesizes one
//! that speaks a small, stable protocol with the runner:
//!
//! - `--list` prints one fully qualified test name per line;
//! - `--run NAME` calls one test and exits zero;
//! - `--bench NAME COUNT` calls one benchmark `COUNT` times and exits zero.
//!
//! The protocol is implemented in the executable so every test runs in its own
//! process. That isolation is what lets the runner enforce timeouts, retries,
//! and parallel execution without trusting the test code.

use inkwell::IntPredicate;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{FunctionValue, PointerValue, ValueKind};
use seal_def::{DefId, DefKind};
use seal_hir::Hir;
use seal_symbol::Interner;
use seal_ty::Instance;

use crate::error::CodegenError;
use crate::options::HarnessKind;

/// One callable entry point the harness dispatches to.
struct Entry {
    /// The fully qualified name printed by `--list`.
    name: String,
    /// The monomorphized instance to call.
    instance: Instance,
}

/// Emits the harness `main` for `kind`.
///
/// The caller has already declared every function and lowered every body, so
/// each entry resolves to an existing `FunctionValue`.
pub(crate) fn emit<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    hir: &Hir,
    interner: &Interner,
    functions: &seal_hash::FxHashMap<Instance, FunctionValue<'ctx>>,
    kind: HarnessKind,
) -> Result<(), CodegenError> {
    let entries = entries(hir, interner, kind)?;
    let pointer = context.ptr_type(inkwell::AddressSpace::default());
    let i32_type = context.i32_type();
    let main_type = i32_type.fn_type(&[i32_type.into(), pointer.into()], false);
    let main = module.add_function("main", main_type, None);
    let argc = main
        .get_nth_param(0)
        .ok_or_else(|| CodegenError::internal("the harness entry has no argc"))?
        .into_int_value();
    let argv = main
        .get_nth_param(1)
        .ok_or_else(|| CodegenError::internal("the harness entry has no argv"))?
        .into_pointer_value();

    let entry = context.append_basic_block(main, "entry");
    let check_mode = context.append_basic_block(main, "check_mode");
    let list = context.append_basic_block(main, "list");
    let dispatch_mode = context.append_basic_block(main, "dispatch_mode");
    let dispatch = context.append_basic_block(main, "dispatch");
    let usage = context.append_basic_block(main, "usage");
    let selected = context.append_basic_block(main, "selected");
    let cases: Vec<_> = entries
        .iter()
        .map(|_| context.append_basic_block(main, "case"))
        .collect();

    builder.position_at_end(entry);
    let has_mode = builder
        .build_int_compare(
            IntPredicate::UGE,
            argc,
            i32_type.const_int(2, false),
            "has_mode",
        )
        .map_err(builder_error)?;
    builder
        .build_conditional_branch(has_mode, check_mode, usage)
        .map_err(branch_error)?;

    builder.position_at_end(check_mode);
    let first = load_arg(context, builder, pointer, argv, 1)?;
    let list_name = builder
        .build_global_string_ptr("--list", "harness.list")
        .map_err(builder_error)?
        .as_pointer_value();
    let is_list = strcmp(context, module, builder, first, list_name)?;
    builder
        .build_conditional_branch(is_list, list, dispatch_mode)
        .map_err(branch_error)?;

    builder.position_at_end(list);
    let puts = declare_puts(context, module);
    for (index, entry) in entries.iter().enumerate() {
        let text = builder
            .build_global_string_ptr(&entry.name, &format!("harness.name{index}"))
            .map_err(builder_error)?
            .as_pointer_value();
        builder
            .build_call(puts, &[text.into()], "harness.puts")
            .map_err(builder_error)?;
    }
    builder
        .build_return(Some(&i32_type.const_zero()))
        .map_err(builder_error)?;

    builder.position_at_end(dispatch_mode);
    let mode = match kind {
        HarnessKind::Test => "--run",
        HarnessKind::Bench => "--bench",
    };
    let mode_name = builder
        .build_global_string_ptr(mode, "harness.mode")
        .map_err(builder_error)?
        .as_pointer_value();
    let is_mode = strcmp(context, module, builder, first, mode_name)?;
    builder
        .build_conditional_branch(is_mode, dispatch, usage)
        .map_err(branch_error)?;

    builder.position_at_end(dispatch);
    let has_name = builder
        .build_int_compare(
            IntPredicate::UGE,
            argc,
            i32_type.const_int(3, false),
            "has_name",
        )
        .map_err(builder_error)?;
    builder
        .build_conditional_branch(has_name, selected, usage)
        .map_err(branch_error)?;
    builder.position_at_end(selected);
    let selected_name = load_arg(context, builder, pointer, argv, 2)?;
    for (entry, case) in entries.iter().zip(cases.iter()) {
        let name = builder
            .build_global_string_ptr(&entry.name, "harness.entry")
            .map_err(builder_error)?
            .as_pointer_value();
        let matches = strcmp(context, module, builder, selected_name, name)?;
        let next = context.append_basic_block(main, "next");
        builder
            .build_conditional_branch(matches, *case, next)
            .map_err(branch_error)?;
        builder.position_at_end(next);
    }
    builder
        .build_unconditional_branch(usage)
        .map_err(branch_error)?;

    for (entry, case) in entries.iter().zip(cases.iter()) {
        let function = functions.get(&entry.instance).ok_or_else(|| {
            CodegenError::internal(format!(
                "the harness entry `{}` has no declared function",
                entry.name
            ))
        })?;
        builder.position_at_end(*case);
        match kind {
            HarnessKind::Test => {
                builder
                    .build_call(*function, &[], "harness.call")
                    .map_err(builder_error)?;
                builder
                    .build_return(Some(&i32_type.const_zero()))
                    .map_err(builder_error)?;
            }
            HarnessKind::Bench => {
                // The runner measures wall time around this process. Repeating
                // inside the harness keeps process startup out of the
                // measurement while avoiding a clock dependency in the target.
                let has_count = builder
                    .build_int_compare(
                        IntPredicate::UGE,
                        argc,
                        i32_type.const_int(4, false),
                        "has_count",
                    )
                    .map_err(builder_error)?;
                let count_block = context.append_basic_block(main, "bench.count");
                let count_text = load_arg(context, builder, pointer, argv, 3)?;
                builder
                    .build_conditional_branch(has_count, count_block, usage)
                    .map_err(branch_error)?;
                builder.position_at_end(count_block);
                let count = atoi(context, module, builder, count_text)?;
                let counter = builder
                    .build_alloca(i32_type, "harness.counter")
                    .map_err(builder_error)?;
                builder
                    .build_store(counter, i32_type.const_zero())
                    .map_err(builder_error)?;
                let body = context.append_basic_block(main, "bench.body");
                let done = context.append_basic_block(main, "bench.done");
                builder
                    .build_unconditional_branch(body)
                    .map_err(branch_error)?;
                builder.position_at_end(body);
                builder
                    .build_call(*function, &[], "harness.bench_call")
                    .map_err(builder_error)?;
                let current = builder
                    .build_load(i32_type, counter, "harness.i")
                    .map_err(builder_error)?
                    .into_int_value();
                let next = builder
                    .build_int_add(current, i32_type.const_int(1, false), "harness.next")
                    .map_err(builder_error)?;
                builder.build_store(counter, next).map_err(builder_error)?;
                let more = builder
                    .build_int_compare(IntPredicate::SLT, next, count, "harness.more")
                    .map_err(builder_error)?;
                builder
                    .build_conditional_branch(more, body, done)
                    .map_err(branch_error)?;
                builder.position_at_end(done);
                builder
                    .build_return(Some(&i32_type.const_zero()))
                    .map_err(builder_error)?;
            }
        }
    }

    builder.position_at_end(usage);
    builder
        .build_return(Some(&i32_type.const_int(2, false)))
        .map_err(builder_error)?;
    Ok(())
}

/// Loads one `char *` argument from `argv[index]`.
fn load_arg<'ctx>(
    context: &'ctx Context,
    builder: &Builder<'ctx>,
    pointer: inkwell::types::PointerType<'ctx>,
    argv: PointerValue<'ctx>,
    index: u32,
) -> Result<PointerValue<'ctx>, CodegenError> {
    let slot = seal_llvm_ffi::gep(
        builder,
        argv,
        pointer,
        &[context.i32_type().const_int(u64::from(index), false)],
        false,
    )
    .map_err(builder_error)?;
    Ok(builder
        .build_load(pointer, slot, "harness.arg")
        .map_err(builder_error)?
        .into_pointer_value())
}

/// Compares two C strings and returns whether they are equal.
fn strcmp<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    left: PointerValue<'ctx>,
    right: PointerValue<'ctx>,
) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
    let function = match module.get_function("strcmp") {
        Some(function) => function,
        None => {
            let pointer = context.ptr_type(inkwell::AddressSpace::default());
            let function_type = context.i32_type().fn_type(
                &[
                    BasicMetadataTypeEnum::PointerType(pointer),
                    BasicMetadataTypeEnum::PointerType(pointer),
                ],
                false,
            );
            module.add_function("strcmp", function_type, None)
        }
    };
    let call = builder
        .build_call(function, &[left.into(), right.into()], "harness.strcmp")
        .map_err(builder_error)?;
    let value = match call.try_as_basic_value() {
        ValueKind::Basic(value) => value.into_int_value(),
        ValueKind::Instruction(_) => {
            return Err(CodegenError::internal("`strcmp` returned no value"));
        }
    };
    builder
        .build_int_compare(
            IntPredicate::EQ,
            value,
            context.i32_type().const_zero(),
            "harness.same",
        )
        .map_err(builder_error)
}

/// Declares `int puts(const char *)` once per module.
fn declare_puts<'ctx>(context: &'ctx Context, module: &Module<'ctx>) -> FunctionValue<'ctx> {
    if let Some(function) = module.get_function("puts") {
        return function;
    }
    let pointer = context.ptr_type(inkwell::AddressSpace::default());
    let function_type = context
        .i32_type()
        .fn_type(&[BasicMetadataTypeEnum::PointerType(pointer)], false);
    module.add_function("puts", function_type, None)
}

/// Calls `atoi` for the benchmark iteration count.
fn atoi<'ctx>(
    context: &'ctx Context,
    module: &Module<'ctx>,
    builder: &Builder<'ctx>,
    text: PointerValue<'ctx>,
) -> Result<inkwell::values::IntValue<'ctx>, CodegenError> {
    let function = match module.get_function("atoi") {
        Some(function) => function,
        None => {
            let pointer = context.ptr_type(inkwell::AddressSpace::default());
            let function_type = context
                .i32_type()
                .fn_type(&[BasicMetadataTypeEnum::PointerType(pointer)], false);
            module.add_function("atoi", function_type, None)
        }
    };
    let call = builder
        .build_call(function, &[text.into()], "harness.atoi")
        .map_err(builder_error)?;
    match call.try_as_basic_value() {
        ValueKind::Basic(value) => Ok(value.into_int_value()),
        ValueKind::Instruction(_) => Err(CodegenError::internal("`atoi` returned no value")),
    }
}

/// Returns the entries for `kind`, validating their signature.
fn entries(hir: &Hir, interner: &Interner, kind: HarnessKind) -> Result<Vec<Entry>, CodegenError> {
    let mut entries = Vec::new();
    for (definition, data) in hir.definitions().iter_enumerated() {
        let selected = match kind {
            HarnessKind::Test => data.is_test,
            HarnessKind::Bench => data.is_bench,
        };
        if !selected || data.kind != DefKind::Function || data.parent.is_some() {
            continue;
        }
        if data.package != hir.primary_package() {
            continue;
        }
        validate_signature(hir, interner, definition, kind)?;
        let module = hir.module(data.module);
        let mut segments: Vec<&str> = module
            .path
            .segments()
            .iter()
            .map(|segment| interner.resolve(*segment))
            .collect();
        segments.push(interner.resolve(data.name));
        entries.push(Entry {
            name: segments.join("::"),
            instance: Instance::new(definition, hir.ty_ctxt().empty_substs()),
        });
    }
    Ok(entries)
}

/// Checks that a test or benchmark can be called with no arguments.
fn validate_signature(
    hir: &Hir,
    interner: &Interner,
    definition: DefId,
    kind: HarnessKind,
) -> Result<(), CodegenError> {
    let data = hir.definitions().get(definition);
    let label = match kind {
        HarnessKind::Test => "test",
        HarnessKind::Bench => "benchmark",
    };
    if !hir.flattened_generics(definition).is_empty() {
        return Err(CodegenError::harness(format!(
            "the {label} `{}` declares generic parameters; a {label} cannot be generic",
            interner.resolve(data.name)
        )));
    }
    let Some(body) = hir.body_of(definition) else {
        return Err(CodegenError::harness(format!(
            "the {label} `{}` has no body",
            interner.resolve(data.name)
        )));
    };
    if !body.params.is_empty() {
        return Err(CodegenError::harness(format!(
            "the {label} `{}` takes parameters; a {label} must take none",
            interner.resolve(data.name)
        )));
    }
    if body.ret.is_some() {
        return Err(CodegenError::harness(format!(
            "the {label} `{}` must return `()`",
            interner.resolve(data.name)
        )));
    }
    Ok(())
}

fn builder_error(error: inkwell::builder::BuilderError) -> CodegenError {
    CodegenError::internal(format!("cannot build the test harness: {error}"))
}

fn branch_error(error: inkwell::builder::BuilderError) -> CodegenError {
    CodegenError::internal(format!("cannot branch in the test harness: {error}"))
}
