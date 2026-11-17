//! Generates LLVM-IR and emits object files from the [`Mir`].
//!
//! The entry point to this crate is the [`Codegen`] type, and the [`codegen`][Codegen::codegen] method on it.
//! Use the [`create_ctx`] function to acquire a [`Context`] for use in [`Codegen::new`].

#![allow(
    clippy::unwrap_used,
    reason = "A large number of Inkwell functions return Results for error conditions we don't want to recover from"
)]

mod attrs;
mod config;
mod exprs;
mod funcs;

use foldhash::{HashMap, HashMapExt as _};
use inkwell::{
    AddressSpace, FloatPredicate, IntPredicate,
    basic_block::BasicBlock,
    builder::Builder,
    context::Context,
    llvm_sys::LLVMCallConv,
    module::{Linkage, Module},
    passes::PassBuilderOptions,
    targets::{
        FileType, InitializationConfig, Target as LLVMTarget, TargetMachine, TargetMachineOptions,
    },
    types::{BasicType, BasicTypeEnum, FunctionType},
    values::{
        BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue, FunctionValue, IntValue,
        PointerValue,
    },
};
use slotmap::SecondaryMap;

use irs::{
    Package,
    mir::{Func, LocalId, Mir},
    ty::{DefId, Param, Ty, TyContext, TyId, layout::StorageClass},
};

pub use config::*;

/// # Panics
/// Will panic if `target` is not a valid LLVM target triple.
pub fn emit(
    pkg: &Package,
    mir: &Mir,
    ty_ctx: &TyContext,
    opt_level: OptLevel,
    mode: CodegenMode,
    target: Target,
    package_name: &str,
) {
    let ctx = Context::create();
    CodegenState::new(pkg, mir, ty_ctx, &ctx, target, package_name).emit(opt_level, mode);
}

struct CodegenState<'ext, 'ctx> {
    pkg: &'ext Package,
    mir: &'ext Mir,
    ty_ctx: &'ext TyContext,
    ctx: &'ctx Context,
    builder: Builder<'ctx>,
    module: Module<'ctx>,
    target: TargetMachine,
    field_orders: HashMap<TyId, Vec<u32>>,
    funcs: SecondaryMap<DefId, FunctionValue<'ctx>>,
    vars: SecondaryMap<LocalId, PointerValue<'ctx>>,
}

impl<'ext, 'ctx> CodegenState<'ext, 'ctx> {
    /// Creates a new [`Codegen`] for a package with the given name.
    ///
    /// # Panics
    /// Panics if the target machine could not be created.
    fn new(
        pkg: &'ext Package,
        mir: &'ext Mir,
        ty_ctx: &'ext TyContext,
        ctx: &'ctx Context,
        target: Target,
        package_name: &str,
    ) -> Self {
        let module = ctx.create_module(package_name);

        LLVMTarget::initialize_all(&InitializationConfig::default());
        let triple = target.triple();
        let target = LLVMTarget::from_triple(&triple).unwrap();
        let target_machine = target
            .create_target_machine_from_options(&triple, TargetMachineOptions::default())
            .unwrap();

        Self {
            pkg,
            mir,
            ty_ctx,
            ctx,
            builder: ctx.create_builder(),
            module,
            target: target_machine,
            field_orders: HashMap::new(),
            funcs: SecondaryMap::new(),
            vars: SecondaryMap::new(),
        }
    }

    /// # Panics
    /// Panics if any functions are invalid, or if writing to the output file fails.
    fn emit(&mut self, opt_level: OptLevel, mode: CodegenMode) {
        for def in self.ty_ctx.defs() {
            let def_info = self.ty_ctx.def(def);

            let name = mangle::func_symbol(self.pkg, self.ty_ctx, def);
            let func_ty = self.func_ty(&def_info.params, def_info.ret_ty, false);

            let func = self.add_func(&name, func_ty, false);
            self.funcs.insert(def, func);
        }

        if let Some(main) = self.ty_ctx.main() {
            let i32_ty = self.ctx.i32_type();
            let ty = i32_ty.fn_type(&[i32_ty.into(), self.ptr_ty().into()], false);
            let func = self.add_func("main", ty, true);

            self.builder
                .position_at_end(self.ctx.append_basic_block(func, ""));
            self.build_call(self.funcs[main], &[]);
            self.builder
                .build_return(Some(&i32_ty.const_zero()))
                .unwrap();

            assert!(func.verify(true));
        }

        for def in self.ty_ctx.defs() {
            self.emit_func_body(def);
        }

        self.module.verify().unwrap();

        self.module
            .set_data_layout(&self.target.get_target_data().get_data_layout());
        self.module.set_triple(&self.target.get_triple());

        self.module
            .run_passes(
                &opt_level.opt_string(),
                &self.target,
                PassBuilderOptions::create(),
            )
            .unwrap();

        match mode {
            CodegenMode::IRDump => self.module.print_to_stderr(),
            CodegenMode::Emit(path) => {
                self.target
                    .write_to_file(&self.module, FileType::Object, &path)
                    .unwrap();
            }
            CodegenMode::Silent => {}
        }
    }

    fn emit_func_body(&mut self, def: DefId) {
        let func = self.mir.func(def);
        let llvm_func = self.funcs[def];

        let setup_block = self.ctx.append_basic_block(llvm_func, "");
        self.builder.position_at_end(setup_block);

        // Set up allocations for the locals.
        let mut args = llvm_func.get_param_iter();

        // First local is the return place.
        let ret_place = func.ret_place();
        if self.ty_ctx.is_indirect(self.ty_ctx.def(def).ret_ty) {
            self.vars
                .insert(ret_place, args.next().unwrap().into_pointer_value());
        } else {
            let alloc = self.build_local_alloca(func, ret_place);
            self.vars.insert(ret_place, alloc);
        }

        // The rest of the locals are either parameters, temporaries, or variables.
        for (local, info) in func.locals.iter().skip(1) {
            // We don't actually pass or allocate ZSTs.
            if self.ty_ctx.is_zst(info.ty) {
                continue;
            }

            let ptr = match args.next() {
                Some(arg) if info.mutable || self.ty_ctx.is_indirect(info.ty) => {
                    // Mutable or indirect-type parameters are already pointers.
                    arg.into_pointer_value()
                }
                Some(arg) => {
                    // Immutable, direct-type parameters need local allocations created.
                    let alloc = self.build_local_alloca(func, local);
                    self.build_store(info.ty, alloc, arg);
                    alloc
                }
                None => self.build_local_alloca(func, local),
            };

            self.vars.insert(local, ptr);
        }

        self.emit_blocks(def);

        assert!(llvm_func.verify(true));
    }

    #[expect(clippy::as_conversions, reason = "accessing enum discriminant")]
    fn add_func(&self, name: &str, ty: FunctionType<'ctx>, external: bool) -> FunctionValue<'ctx> {
        let (linkage, call_conv) = if external {
            (Linkage::External, LLVMCallConv::LLVMCCallConv)
        } else {
            (Linkage::Private, LLVMCallConv::LLVMFastCallConv)
        };
        let func = self.module.add_function(name, ty, Some(linkage));
        func.set_call_conventions(call_conv as u32);
        func.as_global_value().set_unnamed_addr(true);
        func
    }

    #[expect(clippy::as_conversions, reason = "accessing enum discriminant")]
    fn build_call(
        &self,
        func: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        let call = self.builder.build_call(func, args, "").unwrap();
        call.set_call_convention(LLVMCallConv::LLVMFastCallConv as u32);
        call.try_as_basic_value().basic()
    }

    #[expect(clippy::as_conversions, reason = "accessing enum discriminant")]
    fn build_c_call(
        &self,
        func: FunctionValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        let call = self.builder.build_call(func, args, "").unwrap();
        call.set_call_convention(LLVMCallConv::LLVMCCallConv as u32);
        call.try_as_basic_value().basic()
    }

    #[expect(clippy::as_conversions, reason = "accessing enum discriminant")]
    fn build_indirect_call(
        &self,
        func_ty: FunctionType<'ctx>,
        func_ptr: PointerValue<'ctx>,
        args: &[BasicMetadataValueEnum<'ctx>],
    ) -> Option<BasicValueEnum<'ctx>> {
        let call = self
            .builder
            .build_indirect_call(func_ty, func_ptr, args, "")
            .unwrap();
        call.set_call_convention(LLVMCallConv::LLVMFastCallConv as u32);
        call.try_as_basic_value().basic()
    }

    fn lower_ty(&mut self, ty: TyId) -> BasicTypeEnum<'ctx> {
        let lowered_ty = match self.ty_ctx.get_ty(ty) {
            Ty::Int | Ty::UInt => self.ctx.i64_type().as_basic_type_enum(),
            Ty::Byte => self.ctx.i8_type().as_basic_type_enum(),
            Ty::Float => self.ctx.f64_type().as_basic_type_enum(),
            Ty::Bool => self.ctx.bool_type().as_basic_type_enum(),
            Ty::Tuple(elem_tys) => {
                let field_order = self.get_field_order(ty);
                self.ctx
                    .struct_type(field_types, false)
                    .as_basic_type_enum()
            }
            Ty::Func(..) => self.closure_ty(),
            Ty::Named {
                ident,
                opaque,
                fields,
                ctor,
                module,
            } => todo!(),
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variables in codegen"),
        };

        assert_eq!(
            self.target.get_target_data().get_store_size(&lowered_ty),
            self.ty_ctx.size_of(ty).into(),
            "ty: {ty:?}"
        );
        assert_eq!(
            self.target.get_target_data().get_abi_alignment(&lowered_ty),
            self.ty_ctx.align_of(ty),
            "ty: {ty:?}"
        );

        lowered_ty
    }

    fn closure_ty(&self) -> BasicTypeEnum<'ctx> {
        self.ctx
            .struct_type(
                &[
                    // Function
                    self.ptr_ty(),
                    // Environment
                    self.ptr_ty(),
                    // Drop
                    self.ptr_ty(),
                    // Copy
                    self.ptr_ty(),
                ],
                false,
            )
            .as_basic_type_enum()
    }

    fn fields_ty(&self, fields: impl IntoIterator<Item = TyId>) -> BasicTypeEnum<'ctx> {
        let field_tys: Vec<_> = fields.into_iter().map(|ty| self.lower_ty(ty)).collect();
        self.ctx.struct_type(&field_tys, false).as_basic_type_enum()
    }

    fn func_ty(&mut self, params: &[Param], ret_ty: TyId, env: bool) -> FunctionType<'ctx> {
        let mut param_tys: Vec<_> = params
            .iter()
            .filter_map(|p| {
                if self.ty_ctx.is_zst(p.ty) {
                    None // Skip passing ZSTs.
                } else if p.mutable || self.ty_ctx.is_indirect(p.ty) {
                    Some(self.ptr_ty().into()) // Pass by pointer for mutable parameters or indirect types.
                } else {
                    Some(self.lower_ty(p.ty).into()) // Pass by value otherwise.
                }
            })
            .collect();

        // Add parameter for environment if needed.
        if env {
            param_tys.push(self.ptr_ty().into());
        }

        // Add parameter for return pointer if needed.
        match self.ty_ctx.storage_class(ret_ty) {
            StorageClass::Zst => self.ctx.void_type().fn_type(&param_tys, false),
            StorageClass::Indirect => {
                param_tys.insert(0, self.ptr_ty().into());
                self.ctx.void_type().fn_type(&param_tys, false)
            }
            StorageClass::Scalar => self.lower_ty(ret_ty).fn_type(&param_tys, false),
        }
    }

    /// Gets the cached field order for the given type, which must be [`Ty::Tuple`] or [`Ty::Named`].
    ///
    /// The returned slice has one element for each field of the type, corresponding to the in-memory index of that field.
    fn get_field_order(&mut self, ty: TyId) -> &[u32] {
        self.field_orders
            .entry(ty)
            .or_insert_with(|| Self::calc_field_order(self.ty_ctx, ty))
    }

    fn calc_field_order(ty_ctx: &TyContext, ty: TyId) -> Vec<u32> {
        match ty_ctx.get_ty(ty) {
            Ty::Tuple(elem_tys) => {
                todo!()
                // let mut order: Vec<u32> =
                //     (0..elem_tys.len().try_into().expect("too many fields")).collect();
                // order.sort_by_key(|idx| ty_ctx.size_of(elem_tys[usize::try_from(*idx).unwrap()]));
                // order
            }
            Ty::Named { fields, .. } => {
                todo!()
                // let mut order: Vec<u32> =
                //     (0..fields.len().try_into().expect("too many fields")).collect();
                // order.sort_by_key(|idx| ty_ctx.size_of(fields[usize::try_from(*idx).unwrap()]));
                // order
            }
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variable in codegen"),
            _ => panic!("ICE: Attempted to calculate field order for type without fields"),
        }
    }

    fn ptr_ty(&self) -> BasicTypeEnum<'ctx> {
        self.ctx
            .ptr_type(AddressSpace::default())
            .as_basic_type_enum()
    }

    fn const_int(&self, value: i64) -> IntValue<'ctx> {
        self.ctx.i64_type().const_int(value.cast_unsigned(), false)
    }

    fn const_uint(&self, value: u64) -> IntValue<'ctx> {
        self.ctx.i64_type().const_int(value, false)
    }

    fn const_byte(&self, value: u8) -> IntValue<'ctx> {
        self.ctx.i8_type().const_int(u64::from(value), false)
    }

    fn const_float(&self, value: f64) -> FloatValue<'ctx> {
        self.ctx.f64_type().const_float(value)
    }

    fn const_bool(&self, value: bool) -> IntValue<'ctx> {
        match value {
            true => self.ctx.bool_type().const_all_ones(),
            false => self.ctx.bool_type().const_zero(),
        }
    }

    fn const_null(&self) -> PointerValue<'ctx> {
        self.ctx.ptr_type(AddressSpace::default()).const_null()
    }

    fn dangling(&self, ty: TyId) -> PointerValue<'ctx> {
        self.builder
            .build_int_to_ptr(
                self.const_uint(self.ty_ctx.align_of(ty).into()),
                self.ptr_ty().into_pointer_type(),
                "",
            )
            .unwrap()
    }

    fn build_local_alloca(&mut self, func: &'ext Func, local: LocalId) -> PointerValue<'ctx> {
        let local = func.local(local);

        let name = match local.ident {
            Some(ident) => &ident.str(),
            None => "",
        };

        let curr_block = self.curr_block();
        let head_block = self.curr_function().get_first_basic_block().unwrap();

        if let Some(first_instr) = head_block.get_first_instruction() {
            self.builder.position_before(&first_instr);
        } else {
            self.builder.position_at_end(head_block);
        }

        let ty = self.lower_ty(local.ty);
        let alloc = self.builder.build_alloca(ty, name).unwrap();
        alloc
            .as_instruction_value()
            .unwrap()
            .set_alignment(self.ty_ctx.align_of(local.ty))
            .unwrap();

        self.builder.position_at_end(curr_block);

        alloc
    }

    fn build_field(
        &mut self,
        ty: TyId,
        base: PointerValue<'ctx>,
        field: usize,
    ) -> PointerValue<'ctx> {
        let idx = self.get_field_order(ty)[field];
        let ty = self.lower_ty(ty);
        self.builder.build_struct_gep(ty, base, idx, "").unwrap()
    }

    fn build_read(&mut self, ty: TyId, place: PointerValue<'ctx>) -> BasicValueEnum<'ctx> {
        match self.ty_ctx.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Byte | Ty::Bool | Ty::Float => {
                let ty = self.lower_ty(ty);
                self.builder.build_load(ty, place, "").unwrap()
            }
            Ty::Tuple(_) | Ty::Func(_, _) | Ty::Named { .. } => place.as_basic_value_enum(),
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variable in codegen"),
        }
    }

    fn build_store(&self, ty: TyId, dst: PointerValue<'ctx>, value: BasicValueEnum<'ctx>) {
        match self.ty_ctx.get_ty(ty) {
            // Avoid generating actual stores of ZST types.
            _ if self.ty_ctx.is_zst(ty) => {}
            Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool => {
                self.builder.build_store(dst, value).unwrap();
            }
            Ty::Tuple(_) | Ty::Func(_, _) | Ty::Named { .. } => {
                self.build_memcpy(ty, dst, value.into_pointer_value())
            }
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variable in codegen"),
        }
    }

    fn build_copy(&mut self, ty: TyId, src: PointerValue<'ctx>, dst: PointerValue<'ctx>) {
        match self.ty_ctx.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Byte | Ty::Bool | Ty::Float => {
                let value = self.build_read(ty, src);
                self.build_store(ty, dst, value);
            }
            Ty::Tuple(elem_tys) => {
                let func = self.fields_copy(ty, elem_tys);
                self.build_call(func, &[dst.into(), src.into()]);
            }
            Ty::Func(_, _) => {
                let copy_func = self
                    .builder
                    .build_struct_gep(self.closure_ty(), src, 3, "")
                    .unwrap();
                let copy_func = self
                    .builder
                    .build_load(self.ptr_ty(), copy_func, "")
                    .unwrap();
                self.builder
                    .build_indirect_call(
                        self.copy_func_ty(),
                        copy_func.into_pointer_value(),
                        &[dst.into(), src.into()],
                        "",
                    )
                    .unwrap();
            }
            Ty::Named { fields, .. } => {
                let func = self.fields_copy(ty, fields.values());
                self.build_call(func, &[dst.into(), src.into()]);
            }
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variable in codegen"),
        }
    }

    fn build_drop(&mut self, ty: TyId, place: PointerValue<'ctx>) {
        match self.ty_ctx.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool => {} // Trivial types.
            Ty::Tuple(elem_tys) => {
                let func = self.fields_drop(ty, elem_tys);
                self.build_call(func, &[place.into()]);
            }
            Ty::Func(_, _) => {
                let drop_func = self
                    .builder
                    .build_struct_gep(self.closure_ty(), place, 2, "")
                    .unwrap();
                let drop_func = self
                    .builder
                    .build_load(self.ptr_ty(), drop_func, "")
                    .unwrap();
                self.builder
                    .build_indirect_call(
                        self.drop_func_ty(),
                        drop_func.into_pointer_value(),
                        &[place.into()],
                        "",
                    )
                    .unwrap();
            }
            Ty::Named { fields, .. } => {
                let func = self.fields_drop(ty, fields.values());
                self.build_call(func, &[place.into()]);
            }
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variable in codegen"),
        }
    }

    fn build_equals(
        &mut self,
        ty: TyId,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
    ) -> IntValue<'ctx> {
        match self.ty_ctx.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Byte | Ty::Bool => self
                .builder
                .build_int_compare(
                    IntPredicate::EQ,
                    lhs.into_int_value(),
                    rhs.into_int_value(),
                    "",
                )
                .unwrap(),
            Ty::Float => self
                .builder
                .build_float_compare(
                    FloatPredicate::OEQ,
                    lhs.into_float_value(),
                    rhs.into_float_value(),
                    "",
                )
                .unwrap(),
            Ty::Tuple(elem_tys) => {
                let func = self.fields_equals(ty, elem_tys);
                self.build_call(func, &[lhs.into(), rhs.into()])
                    .unwrap()
                    .into_int_value()
            }
            Ty::Func(_, _) => {
                // Load the function pointers, which are stored first in the closures.
                let lhs_func = self
                    .builder
                    .build_load(self.ptr_ty(), lhs.into_pointer_value(), "")
                    .unwrap()
                    .into_pointer_value();
                let rhs_func = self
                    .builder
                    .build_load(self.ptr_ty(), rhs.into_pointer_value(), "")
                    .unwrap()
                    .into_pointer_value();

                // Test whether the function pointers are equal.
                self.builder
                    .build_int_compare(IntPredicate::EQ, lhs_func, rhs_func, "")
                    .unwrap()
            }
            Ty::Named { fields, .. } => {
                let func = self.fields_equals(ty, fields.values());
                self.build_call(func, &[lhs.into(), rhs.into()])
                    .unwrap()
                    .into_int_value()
            }
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variable in codegen"),
        }
    }

    fn build_panic(&mut self, msg: &str) {
        let msg = self
            .builder
            .build_global_string_ptr(msg, "")
            .unwrap()
            .as_pointer_value();
        let panic = self.panic();
        self.build_call(panic, &[msg.into()]);
        self.builder.build_unreachable().unwrap();
    }

    fn build_malloc(&mut self, ty: TyId) -> PointerValue<'ctx> {
        if self.ty_ctx.is_zst(ty) {
            self.dangling(ty)
        } else {
            let malloc = self.malloc();
            self.build_call(
                malloc,
                &[self.const_uint(self.ty_ctx.size_of(ty).into()).into()],
            )
            .unwrap()
            .into_pointer_value()
        }
    }

    fn build_memcpy(&self, ty: TyId, dst: PointerValue<'ctx>, src: PointerValue<'ctx>) {
        let align = self.ty_ctx.align_of(ty);
        let size = self
            .ctx
            .ptr_sized_int_type(&self.target.get_target_data(), None)
            .const_int(self.ty_ctx.size_of(ty).into(), false);
        self.builder
            .build_memcpy(dst, align, src, align, size)
            .unwrap();
    }

    /// # Panics
    /// Panics if the builder is not positioned.
    fn curr_block(&self) -> BasicBlock<'ctx> {
        self.builder.get_insert_block().unwrap()
    }

    /// # Panics
    /// Panics if the builder is not positioned, or is positioned but not within a function.
    fn curr_function(&self) -> FunctionValue<'ctx> {
        self.curr_block().get_parent().unwrap()
    }
}

struct FieldLayout {}
