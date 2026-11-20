//! DWARF debug information.
//!
//! Debug information is built alongside the IR rather than reconstructed later:
//! every function gets a `DISubprogram`, every parameter and binding gets a
//! `DILocalVariable` declared at its `alloca`, every aggregate gets a
//! `DICompositeType` with member offsets from the language layout, and every
//! statement and terminator gets a `DILocation`.
//!
//! Recursive aggregates are handled with forward declarations. A struct cannot
//! contain itself by value, so recursion goes through a reference; the pointer's
//! pointee is then the forward declaration, which is exactly how DWARF models
//! recursive types.

use std::cell::RefCell;
use std::path::Path;

use inkwell::AddressSpace;
use inkwell::basic_block::BasicBlock;
use inkwell::context::Context;
use inkwell::debug_info::{
    AsDIScope, DICompileUnit, DIFile, DIFlags, DIFlagsConstants, DILocation, DISubprogram, DIType,
    DWARFEmissionKind, DWARFSourceLanguage, DebugInfoBuilder,
};
use inkwell::module::Module;
use inkwell::values::{GlobalValue, PointerValue};
use seal_def::{DefId, Definitions};
use seal_hash::{FxHashMap, FxHashSet};
use seal_mir::{Body, LocalDecl};
use seal_span::{FileId, SourceMap, Span};
use seal_symbol::Interner;
use seal_ty::{AdtId, AdtKind, SubstsId, TyCtxt, TyId, TyKind};

use crate::error::CodegenError;
use crate::types::TypeMapper;

/// DWARF base type encodings (`DW_ATE_*`).
mod encoding {
    /// Boolean.
    pub(super) const BOOLEAN: u32 = 0x02;
    /// Floating point.
    pub(super) const FLOAT: u32 = 0x04;
    /// Signed integer.
    pub(super) const SIGNED: u32 = 0x05;
    /// Unsigned integer.
    pub(super) const UNSIGNED: u32 = 0x07;
    /// Unicode character.
    pub(super) const UTF: u32 = 0x10;
}

const PRODUCER: &str = concat!("Seal ", env!("CARGO_PKG_VERSION"));

/// Owns the debug builder and the metadata caches for one module.
pub(crate) struct DebugContext<'a, 'ctx> {
    context: &'ctx Context,
    builder: DebugInfoBuilder<'ctx>,
    unit: DICompileUnit<'ctx>,
    unit_file: DIFile<'ctx>,
    source_map: &'a SourceMap,
    definitions: &'a Definitions,
    tcx: &'a TyCtxt,
    mapper: &'a TypeMapper<'ctx>,
    interner: &'a Interner,
    optimized: bool,
    files: RefCell<FxHashMap<FileId, DIFile<'ctx>>>,
    types: RefCell<FxHashMap<TyId, DIType<'ctx>>>,
    adts: RefCell<FxHashMap<(AdtId, SubstsId), DIType<'ctx>>>,
    forwards: RefCell<FxHashMap<(AdtId, SubstsId), DIType<'ctx>>>,
    in_progress: RefCell<FxHashSet<(AdtId, SubstsId)>>,
}

impl<'a, 'ctx> DebugContext<'a, 'ctx> {
    /// Starts a compile unit for `module`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        context: &'ctx Context,
        module: &Module<'ctx>,
        source_map: &'a SourceMap,
        definitions: &'a Definitions,
        tcx: &'a TyCtxt,
        mapper: &'a TypeMapper<'ctx>,
        interner: &'a Interner,
        file: FileId,
        optimized: bool,
    ) -> Result<Self, CodegenError> {
        let (file_name, directory) = source_map
            .display_name(file)
            .map(|display| split_display(&display))
            .unwrap_or_else(|| ("seal-input".to_owned(), String::new()));
        let (builder, unit) = module.create_debug_info_builder(
            false,
            DWARFSourceLanguage::Rust,
            &file_name,
            &directory,
            PRODUCER,
            optimized,
            "",
            0,
            "",
            DWARFEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );
        let unit_file = builder.create_file(&file_name, &directory);
        Ok(Self {
            context,
            builder,
            unit,
            unit_file,
            source_map,
            definitions,
            tcx,
            mapper,
            interner,
            optimized,
            files: RefCell::new(FxHashMap::default()),
            types: RefCell::new(FxHashMap::default()),
            adts: RefCell::new(FxHashMap::default()),
            forwards: RefCell::new(FxHashMap::default()),
            in_progress: RefCell::new(FxHashSet::default()),
        })
    }

    /// Creates the `DISubprogram` describing one instance's function.
    ///
    /// `name` is the readable instance name, such as `identity<i32>`, and
    /// `linkage_name` is the unique linker symbol. Every instance gets its own
    /// subprogram; they share source locations but not types or names.
    pub(crate) fn subprogram(
        &self,
        body: &Body,
        name: &str,
        linkage_name: &str,
    ) -> Result<DISubprogram<'ctx>, CodegenError> {
        let (file, line, _) = self.definition_location(body.owner);

        let return_type = body.local(Body::return_local()).ty;
        let return_type = if self.mapper.is_unit_like(return_type) {
            None
        } else {
            Some(self.di_type(return_type)?)
        };
        let mut parameter_types = Vec::with_capacity(body.arg_count);
        for parameter in body.args() {
            parameter_types.push(self.di_type(parameter.ty)?);
        }
        let subroutine = self.builder.create_subroutine_type(
            self.unit_file,
            return_type,
            &parameter_types,
            DIFlags::ZERO,
        );
        Ok(self.builder.create_function(
            self.unit.as_debug_info_scope(),
            name,
            Some(linkage_name),
            file,
            line,
            subroutine,
            false,
            true,
            line,
            DIFlags::ZERO,
            self.optimized,
        ))
    }

    /// Creates the debug subprogram of an external function declaration.
    ///
    /// The declaration has no body, so the subprogram is marked as a
    /// declaration and no scope is opened for it.
    pub(crate) fn declare_extern_function(
        &self,
        def: DefId,
        function: inkwell::values::FunctionValue<'ctx>,
        parameters: &[TyId],
        returns: Option<TyId>,
    ) -> Result<(), CodegenError> {
        let (file, line, _) = self.definition_location(def);
        let return_type = match returns {
            Some(ty) if !self.mapper.is_unit_like(ty) => Some(self.di_type(ty)?),
            _ => None,
        };
        let mut parameter_types = Vec::with_capacity(parameters.len());
        for parameter in parameters {
            parameter_types.push(self.di_type(*parameter)?);
        }
        let subroutine = self.builder.create_subroutine_type(
            self.unit_file,
            return_type,
            &parameter_types,
            DIFlags::ZERO,
        );
        let name = self.interner.resolve(self.definitions.name(def)).to_owned();
        let scope = self.builder.create_function(
            self.unit.as_debug_info_scope(),
            &name,
            None,
            file,
            line,
            subroutine,
            false,
            false,
            line,
            DIFlags::ZERO,
            self.optimized,
        );
        function.set_subprogram(scope);
        Ok(())
    }

    /// Declares a source-level local at its storage.
    pub(crate) fn declare_local(
        &self,
        scope: DISubprogram<'ctx>,
        storage: PointerValue<'ctx>,
        local: &LocalDecl,
        argument: Option<u32>,
        entry: BasicBlock<'ctx>,
    ) -> Result<(), CodegenError> {
        let Some(name) = local.name else {
            return Ok(());
        };
        let name = self.interner.resolve(name);
        let ty = self.di_type(local.ty)?;
        let (file, line, _) = self.definition_location_for_span(local.span);
        let scope = scope.as_debug_info_scope();
        let variable = match argument {
            Some(number) => self.builder.create_parameter_variable(
                scope,
                name,
                number,
                file,
                line,
                ty,
                false,
                DIFlags::ZERO,
            ),
            None => {
                let align = bits_u32(self.mapper.layout(local.ty)?.align() * 8)?;
                self.builder.create_auto_variable(
                    scope,
                    name,
                    file,
                    line,
                    ty,
                    false,
                    DIFlags::ZERO,
                    align,
                )
            }
        };
        let location = self.location_for(scope, local.span).unwrap_or_else(|| {
            self.builder
                .create_debug_location(self.context, 1, 1, scope, None)
        });
        let expression = self.builder.create_expression(Vec::new());
        seal_llvm_ffi::declare_at_end(
            &self.builder,
            storage,
            variable,
            expression,
            location,
            entry,
        );
        Ok(())
    }

    /// Declares a program-lifetime static at its global storage.
    ///
    /// Every static has internal or private linkage, so its symbol is local to
    /// the unit. Local statics use the same record: their scope is lexical, but
    /// their storage and lifetime are still the module's.
    pub(crate) fn declare_static(
        &self,
        def: seal_def::DefId,
        global: GlobalValue<'ctx>,
        ty: TyId,
    ) -> Result<(), CodegenError> {
        let definition = self.definitions.get(def);
        let name = self.interner.resolve(definition.name);
        let linkage = global.get_name().to_str().unwrap_or(name).to_owned();
        let (file, line, _) = self.definition_location(def);
        let di_type = self.di_type(ty)?;
        let align = bits_u32(self.mapper.layout(ty)?.align().max(1) * 8)?;
        let expression = self.builder.create_global_variable_expression(
            self.unit.as_debug_info_scope(),
            name,
            &linkage,
            file,
            line,
            di_type,
            true,
            None,
            None,
            align,
        );
        // LLVM requires the `!dbg` attachment of a global to be the global
        // variable expression itself, not a node wrapping it.
        global.set_metadata(expression.as_metadata_value(self.context), 0);
        Ok(())
    }

    /// Creates a debug location for `span` inside `scope`, if the span is real.
    pub(crate) fn location(
        &self,
        scope: DISubprogram<'ctx>,
        span: Span,
    ) -> Option<DILocation<'ctx>> {
        self.location_for(scope.as_debug_info_scope(), span)
    }

    fn location_for(
        &self,
        scope: inkwell::debug_info::DIScope<'ctx>,
        span: Span,
    ) -> Option<DILocation<'ctx>> {
        let (_, line, column) = self.file_line_column(span)?;
        Some(
            self.builder
                .create_debug_location(self.context, line, column, scope, None),
        )
    }

    /// Finalizes debug metadata. Must run before verification and emission.
    pub(crate) fn finalize(&self) {
        self.builder.finalize();
    }

    fn di_type(&self, ty: TyId) -> Result<DIType<'ctx>, CodegenError> {
        if let Some(found) = self.types.borrow().get(&ty).copied() {
            return Ok(found);
        }
        let kind = self.tcx.kind(ty);
        let created = match kind {
            TyKind::Bool => self.basic_type(ty, "bool", 8, encoding::BOOLEAN)?,
            TyKind::Char => self.basic_type(ty, "char", 32, encoding::UTF)?,
            TyKind::Int(int) => {
                let bits = u64::from(crate::types::int_width(int, self.mapper.pointer_width()));
                self.basic_type(ty, int.as_str(), bits, encoding::SIGNED)?
            }
            TyKind::Uint(uint) => {
                let bits = u64::from(crate::types::uint_width(uint, self.mapper.pointer_width()));
                self.basic_type(ty, uint.as_str(), bits, encoding::UNSIGNED)?
            }
            TyKind::Float(float) => {
                self.basic_type(ty, float.as_str(), u64::from(float.bits()), encoding::FLOAT)?
            }
            TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. }
                if matches!(self.tcx.kind(pointee), TyKind::Dyn { .. }) =>
            {
                let name = self.tcx.display_with(ty, self.interner).to_string();
                self.dyn_type(&name)?
            }
            TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. }
                if matches!(self.tcx.kind(pointee), TyKind::Str) =>
            {
                self.string_type()?
            }
            TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. }
                if matches!(self.tcx.kind(pointee), TyKind::Slice { .. }) =>
            {
                let TyKind::Slice { element } = self.tcx.kind(pointee) else {
                    unreachable!("the guard checked the kind")
                };
                self.slice_type(element)?
            }
            TyKind::Ref { pointee, .. } | TyKind::RawPtr { pointee, .. } => {
                let pointer_bits = u64::from(self.mapper.pointer_width());
                let pointee = self.di_type(pointee)?;
                self.builder
                    .create_pointer_type(
                        "",
                        pointee,
                        pointer_bits,
                        bits_u32(pointer_bits)?,
                        AddressSpace::default(),
                    )
                    .as_type()
            }
            TyKind::Array { element, length } => {
                let layout = self.mapper.layout(ty)?;
                let element = self.di_type(element)?;
                // One subscript describes the whole dimension. Building one
                // range per element would allocate memory proportional to the
                // array length while emitting debug information.
                let mut subscripts = Vec::new();
                if length != 0 {
                    let upper = i64::try_from(length).map_err(|_| {
                        CodegenError::internal(format!(
                            "array length {length} does not fit debug information"
                        ))
                    })?;
                    subscripts.push(0..upper);
                }
                let size_bits = layout.size().checked_mul(8).ok_or_else(|| {
                    CodegenError::internal(format!(
                        "array type {ty:?} is too large for debug information"
                    ))
                })?;
                self.builder
                    .create_array_type(
                        element,
                        size_bits,
                        bits_u32(layout.align() * 8)?,
                        &subscripts,
                    )
                    .as_type()
            }
            TyKind::Tuple(tuple) => {
                let layout = self.mapper.layout(ty)?;
                let size_bits = layout.size().checked_mul(8).ok_or_else(|| {
                    CodegenError::internal(format!(
                        "tuple type {ty:?} is too large for debug information"
                    ))
                })?;
                let scope = self.unit.as_debug_info_scope();
                let element_layouts = self.mapper.tuple_field_layouts(ty)?;
                let mut members = Vec::with_capacity(element_layouts.len());
                for (index, (element, element_layout)) in self
                    .tcx
                    .tuple_elements(tuple)
                    .into_iter()
                    .zip(&element_layouts)
                    .enumerate()
                {
                    let member_type = self.di_type(element)?;
                    let name = index.to_string();
                    let member = self.builder.create_member_type(
                        scope,
                        &name,
                        self.unit_file,
                        0,
                        element_layout.layout.size() * 8,
                        bits_u32(element_layout.layout.align() * 8)?,
                        element_layout.offset * 8,
                        DIFlags::ZERO,
                        member_type,
                    );
                    members.push(member.as_type());
                }
                let name = self.tcx.display(ty).to_string();
                self.builder
                    .create_struct_type(
                        scope,
                        &name,
                        self.unit_file,
                        0,
                        size_bits,
                        bits_u32(layout.align() * 8)?,
                        DIFlags::ZERO,
                        None,
                        &members,
                        0,
                        None,
                        "",
                    )
                    .as_type()
            }
            TyKind::Adt { adt, args } => self.adt_type(adt, args)?,
            TyKind::Str => self.basic_type(ty, "str", 0, encoding::UNSIGNED)?,
            TyKind::Slice { .. } => self.basic_type(ty, "slice", 0, encoding::UNSIGNED)?,
            // A bare `dyn Trait` is unsized and never a value, but debug
            // information may still ask about it while describing a pointer.
            // A zero-sized basic type keeps `-g` total without inventing a
            // storage layout for an unsized type.
            TyKind::Dyn { .. } => self.basic_type(ty, "dyn", 0, encoding::UNSIGNED)?,
            TyKind::Unit | TyKind::Never | TyKind::Error => {
                self.basic_type(ty, "()", 0, encoding::UNSIGNED)?
            }
            TyKind::Infer(_) => {
                return Err(CodegenError::internal(
                    "an unresolved inference type reached debug information",
                ));
            }
            TyKind::Param(_) => {
                return Err(CodegenError::internal(
                    "an uninstantiated type parameter reached debug information",
                ));
            }
        };
        self.types.borrow_mut().insert(ty, created);
        Ok(created)
    }

    fn basic_type(
        &self,
        _ty: TyId,
        name: &str,
        size_in_bits: u64,
        encoding: u32,
    ) -> Result<DIType<'ctx>, CodegenError> {
        self.builder
            .create_basic_type(name, size_in_bits, encoding, DIFlags::ZERO)
            .map(|basic| basic.as_type())
            .map_err(|error| {
                CodegenError::internal(format!("cannot describe `{name}` in debug info: {error:?}"))
            })
    }

    fn string_type(&self) -> Result<DIType<'ctx>, CodegenError> {
        let byte = self
            .builder
            .create_basic_type("u8", 8, encoding::UNSIGNED, DIFlags::ZERO)
            .map_err(|error| {
                CodegenError::internal(format!("cannot describe `u8` in debug info: {error:?}"))
            })?
            .as_type();
        self.fat_type("&str", byte)
    }

    fn slice_type(&self, element: TyId) -> Result<DIType<'ctx>, CodegenError> {
        let element = self.di_type(element)?;
        self.fat_type("&[T]", element)
    }

    /// Describes a trait-object pointer as its data and vtable pointers.
    fn dyn_type(&self, name: &str) -> Result<DIType<'ctx>, CodegenError> {
        let pointer_bits = u64::from(self.mapper.pointer_width());
        let align = bits_u32(pointer_bits)?;
        let scope = self.unit.as_debug_info_scope();
        let byte = self
            .builder
            .create_basic_type("u8", 8, encoding::UNSIGNED, DIFlags::ZERO)
            .map_err(|error| {
                CodegenError::internal(format!("cannot describe `u8` in debug info: {error:?}"))
            })?
            .as_type();
        let data = self
            .builder
            .create_pointer_type("data", byte, pointer_bits, align, AddressSpace::default())
            .as_type();
        let vtable = self
            .builder
            .create_pointer_type("vtable", byte, pointer_bits, align, AddressSpace::default())
            .as_type();
        let data_member = self
            .builder
            .create_member_type(
                scope,
                "data",
                self.unit_file,
                0,
                pointer_bits,
                align,
                0,
                DIFlags::ZERO,
                data,
            )
            .as_type();
        let vtable_member = self
            .builder
            .create_member_type(
                scope,
                "vtable",
                self.unit_file,
                0,
                pointer_bits,
                align,
                pointer_bits,
                DIFlags::ZERO,
                vtable,
            )
            .as_type();
        Ok(self
            .builder
            .create_struct_type(
                scope,
                name,
                self.unit_file,
                0,
                pointer_bits * 2,
                align,
                DIFlags::ZERO,
                None,
                &[data_member, vtable_member],
                0,
                None,
                "",
            )
            .as_type())
    }

    fn fat_type(&self, name: &str, data: DIType<'ctx>) -> Result<DIType<'ctx>, CodegenError> {
        let pointer_bits = u64::from(self.mapper.pointer_width());
        let align = bits_u32(pointer_bits)?;
        let scope = self.unit.as_debug_info_scope();
        let data = self
            .builder
            .create_pointer_type("data", data, pointer_bits, align, AddressSpace::default())
            .as_type();
        let length = self
            .builder
            .create_basic_type("usize", pointer_bits, encoding::UNSIGNED, DIFlags::ZERO)
            .map_err(|error| {
                CodegenError::internal(format!("cannot describe `usize` in debug info: {error:?}"))
            })?
            .as_type();
        let data_member = self
            .builder
            .create_member_type(
                scope,
                "data",
                self.unit_file,
                0,
                pointer_bits,
                align,
                0,
                DIFlags::ZERO,
                data,
            )
            .as_type();
        let length_member = self
            .builder
            .create_member_type(
                scope,
                "length",
                self.unit_file,
                0,
                pointer_bits,
                align,
                pointer_bits,
                DIFlags::ZERO,
                length,
            )
            .as_type();
        Ok(self
            .builder
            .create_struct_type(
                scope,
                name,
                self.unit_file,
                0,
                pointer_bits * 2,
                align,
                DIFlags::ZERO,
                None,
                &[data_member, length_member],
                0,
                None,
                "",
            )
            .as_type())
    }

    fn adt_type(&self, adt: AdtId, args: SubstsId) -> Result<DIType<'ctx>, CodegenError> {
        let key = (adt, args);
        if let Some(found) = self.adts.borrow().get(&key).copied() {
            return Ok(found);
        }
        if self.in_progress.borrow().contains(&key) {
            return self.forward(key);
        }
        self.in_progress.borrow_mut().insert(key);

        let definition = self.tcx.adt(adt);
        let name = self.aggregate_name(adt, args);
        let (file, line, _) = self.definition_location(definition.def);
        let layout = self.mapper.layout(self.tcx.adt_ty_with(adt, args))?;
        let scope = self.unit.as_debug_info_scope();
        let members = match definition.kind {
            AdtKind::Struct => {
                let field_layouts = self.mapper.field_layouts(adt, args)?;
                let mut members = Vec::with_capacity(definition.fields.len());
                for (field, field_layout) in definition.fields.iter().zip(&field_layouts) {
                    let member_type = self.di_type(self.tcx.substitute(field.ty, args))?;
                    let field_name = field.name.ok_or_else(|| {
                        CodegenError::internal(format!("struct field {field:?} has no name"))
                    })?;
                    let member = self.builder.create_member_type(
                        scope,
                        self.interner.resolve(field_name),
                        file,
                        line,
                        field_layout.layout.size() * 8,
                        bits_u32(field_layout.layout.align() * 8)?,
                        field_layout.offset * 8,
                        DIFlags::ZERO,
                        member_type,
                    );
                    members.push(member.as_type());
                }
                members
            }
            AdtKind::Enum => self.enum_members(adt, args, file, line)?,
        };
        let created = self
            .builder
            .create_struct_type(
                scope,
                &name,
                file,
                line,
                layout.size() * 8,
                bits_u32(layout.align() * 8)?,
                DIFlags::ZERO,
                None,
                &members,
                0,
                None,
                "",
            )
            .as_type();
        self.in_progress.borrow_mut().remove(&key);
        self.adts.borrow_mut().insert(key, created);
        Ok(created)
    }

    /// Returns the readable debug name of one aggregate instantiation.
    ///
    /// Non-generic aggregates keep their declared name. Generic instantiations
    /// render their arguments (`Pair<i32, bool>`), which makes a debugger's type
    /// list tell instances apart.
    fn aggregate_name(&self, adt: AdtId, args: SubstsId) -> String {
        self.tcx
            .display_with(self.tcx.adt_ty_with(adt, args), self.interner)
            .to_string()
    }

    /// Returns the forward declaration reserved before the aggregate is filled.
    ///
    /// Recursive aggregates reach themselves through a reference and must find a
    /// type record before their members are complete, exactly like the LLVM
    /// opaque reservation in the type mapper.
    fn forward(&self, key: (AdtId, SubstsId)) -> Result<DIType<'ctx>, CodegenError> {
        if let Some(found) = self.forwards.borrow().get(&key).copied() {
            return Ok(found);
        }
        let (adt, args) = key;
        let definition = self.tcx.adt(adt);
        let name = self.aggregate_name(adt, args);
        let (file, line, _) = self.definition_location(definition.def);
        let forward = self
            .builder
            .create_struct_type(
                self.unit.as_debug_info_scope(),
                &name,
                file,
                line,
                0,
                0,
                DIFlags::FWD_DECL,
                None,
                &[],
                0,
                None,
                "",
            )
            .as_type();
        self.forwards.borrow_mut().insert(key, forward);
        Ok(forward)
    }

    /// Describes an enum as its discriminant plus its payload bytes.
    ///
    /// DWARF variant parts would let a debugger name the active variant, but the
    /// layout stays faithful: the tag is a real unsigned member and the payload
    /// is a byte array at the language payload offset, so every field can be
    /// inspected through its offset.
    fn enum_members(
        &self,
        adt: AdtId,
        args: SubstsId,
        file: DIFile<'ctx>,
        line: u32,
    ) -> Result<Vec<DIType<'ctx>>, CodegenError> {
        let layout = self.mapper.enum_layout(adt, args)?;
        let scope = self.unit.as_debug_info_scope();
        let tag_bits = layout.tag.size() * 8;
        let tag = self
            .builder
            .create_basic_type("tag", tag_bits, encoding::UNSIGNED, DIFlags::ZERO)
            .map_err(|error| {
                CodegenError::internal(format!(
                    "cannot describe an enum tag in debug info: {error:?}"
                ))
            })?
            .as_type();
        let mut members = vec![
            self.builder
                .create_member_type(
                    scope,
                    "tag",
                    file,
                    line,
                    tag_bits,
                    bits_u32(tag_bits)?,
                    0,
                    DIFlags::ZERO,
                    tag,
                )
                .as_type(),
        ];

        if layout.payload_size > 0 {
            let payload_bits = layout.payload_size.checked_mul(8).ok_or_else(|| {
                CodegenError::internal(format!(
                    "enum {adt:?} payload is too large for debug information"
                ))
            })?;
            let byte = self
                .builder
                .create_basic_type("u8", 8, encoding::UNSIGNED, DIFlags::ZERO)
                .map_err(|error| {
                    CodegenError::internal(format!(
                        "cannot describe enum payload bytes in debug info: {error:?}"
                    ))
                })?
                .as_type();
            let upper = i64::try_from(layout.payload_size).map_err(|_| {
                CodegenError::internal(format!(
                    "enum {adt:?} payload is too large for a DWARF subscript"
                ))
            })?;
            let subscript = 0..upper;
            let payload = self
                .builder
                .create_array_type(
                    byte,
                    payload_bits,
                    bits_u32(layout.payload_align * 8)?,
                    std::slice::from_ref(&subscript),
                )
                .as_type();
            let member = self.builder.create_member_type(
                scope,
                "payload",
                file,
                line,
                payload_bits,
                bits_u32(layout.payload_align * 8)?,
                layout.payload_offset * 8,
                DIFlags::ZERO,
                payload,
            );
            members.push(member.as_type());
        }

        Ok(members)
    }

    fn definition_location(&self, def: seal_def::DefId) -> (DIFile<'ctx>, u32, u32) {
        self.definition_location_for_span(self.definitions.get(def).span)
    }

    fn definition_location_for_span(&self, span: Span) -> (DIFile<'ctx>, u32, u32) {
        self.file_line_column(span)
            .unwrap_or((self.unit_file, 0, 0))
    }

    fn file_line_column(&self, span: Span) -> Option<(DIFile<'ctx>, u32, u32)> {
        let file_id = self.source_map.file_of(span)?;
        let file = self.file(file_id)?;
        let source = self.source_map.file(file_id)?;
        let line_col = source.line_col(span.lo()).ok()?;
        Some((file, line_col.line, line_col.column))
    }

    fn file(&self, file_id: FileId) -> Option<DIFile<'ctx>> {
        if let Some(found) = self.files.borrow().get(&file_id).copied() {
            return Some(found);
        }
        let display = self.source_map.display_name(file_id)?;
        let (name, directory) = split_display(&display);
        let file = self.builder.create_file(&name, &directory);
        self.files.borrow_mut().insert(file_id, file);
        Some(file)
    }
}

fn split_display(display: &str) -> (String, String) {
    let path = Path::new(display);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| display.to_owned());
    let directory = path
        .parent()
        .map(|parent| parent.to_string_lossy().into_owned())
        .unwrap_or_default();
    (name, directory)
}

fn bits_u32(bits: u64) -> Result<u32, CodegenError> {
    u32::try_from(bits)
        .map_err(|_| CodegenError::internal(format!("{bits} bits do not fit a DWARF alignment")))
}
