use inkwell::{
    IntPredicate,
    types::{FunctionType, StructType},
    values::{BasicValue as _, FunctionValue},
};

use irs::{
    mir::LocalId,
    ty::{Ty, TyId},
};

use crate::CodegenState;

impl<'ctx> CodegenState<'_, '_, 'ctx> {
    pub(crate) fn drop_func_ty(&self) -> FunctionType<'ctx> {
        self.ctx.void_type().fn_type(&[self.ptr_ty().into()], false)
    }

    pub(crate) fn copy_func_ty(&self) -> FunctionType<'ctx> {
        let ptr = self.ptr_ty().into();
        self.ctx.void_type().fn_type(&[ptr, ptr], false)
    }

    pub(crate) fn equals_func_ty(&self) -> FunctionType<'ctx> {
        let ptr = self.ptr_ty().into();
        self.ctx.bool_type().fn_type(&[ptr, ptr], false)
    }

    pub fn tuple_drop(&self, elem_tys: &[TyId]) -> FunctionValue<'ctx> {}

    pub fn struct_drop(&self, ident: Ident, fields: &OrderMap<Ident, TyId>) -> FunctionValue<'ctx> {
    }

    pub fn fields_drop(&self, fields: &[TyId]) -> FunctionValue<'ctx> {
        let func_name = format!("{}.drop", self.mangle_fields_ty(fields));

        // Check if we already built this function
        if let Some(func) = self.module.get_function(&func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the function.
        let func = self.add_func(&func_name, self.drop_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let ty = self.fields_ty(fields);
        let out = func.get_nth_param(0).unwrap().into_pointer_value();

        // Drop each non-trivial field.
        for (idx, field_ty) in fields
            .iter()
            .enumerate()
            .filter(|(_, ty)| !self.is_trivial(**ty))
        {
            let field_ptr = self
                .builder
                .build_struct_gep(ty, out, u32::try_from(idx).unwrap(), "fieldptr")
                .unwrap();
            self.build_drop(*field_ty, field_ptr);
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub fn fields_copy(&self, fields: &[TyId]) -> FunctionValue<'ctx> {
        let func_name = format!("{}.copy", self.mangle_fields_ty(fields));

        // Check if we already built this function
        if let Some(func) = self.module.get_function(&func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end.
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the function.
        let func = self.add_func(&func_name, self.copy_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let ty = self.fields_ty(fields);
        let dst = func.get_nth_param(0).unwrap().into_pointer_value();
        let src = func.get_nth_param(1).unwrap().into_pointer_value();

        if self.all_trivial(fields) {
            self.build_memcpy(dst, src, &ty);
        } else {
            // If the fields are not all trivial, we need to copy each field individually.
            for (idx, field_ty) in fields.iter().enumerate() {
                let idx = u32::try_from(idx).unwrap();

                let dst = self
                    .builder
                    .build_struct_gep(ty, dst, idx, "dstfieldptr")
                    .unwrap();
                let src = self
                    .builder
                    .build_struct_gep(ty, src, idx, "srcfieldptr")
                    .unwrap();

                self.build_copy_to(*field_ty, src, dst);
            }
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub fn fields_equals(&self, fields: &[TyId]) -> FunctionValue<'ctx> {
        let func_name = format!("{}.equals", self.mangle_fields_ty(fields));

        // Check if we already built this function
        if let Some(func) = self.module.get_function(&func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end.
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the equality function
        let func = self.add_func(&func_name, self.equals_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let ty = self.fields_ty(fields);
        let lhs = func.get_nth_param(0).unwrap().into_pointer_value();
        let rhs = func.get_nth_param(1).unwrap().into_pointer_value();
        let ne_block = self.ctx.append_basic_block(func, "ne");

        for (idx, field_ty) in fields.iter().enumerate() {
            let idx = u32::try_from(idx).unwrap();

            let lhs = self
                .builder
                .build_struct_gep(ty, lhs, idx, "lhsfieldptr")
                .unwrap();
            let rhs = self
                .builder
                .build_struct_gep(ty, rhs, idx, "rhsfieldptr")
                .unwrap();

            // If the fields are equal, continue to a new block for the next comparison, else branch to the not-equal block
            let eq_block = self.ctx.append_basic_block(func, "eq");
            let equal = self.build_equals(*field_ty, lhs, rhs);
            self.builder
                .build_conditional_branch(equal, eq_block, ne_block)
                .unwrap();
            self.builder.position_at_end(eq_block);
        }

        self.builder
            .build_return(Some(&self.const_bool(true)))
            .unwrap();

        self.builder.position_at_end(ne_block);
        self.builder
            .build_return(Some(&self.const_bool(false)))
            .unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn any_closure_drop(&self) -> FunctionValue<'ctx> {
        let func_name = "Closure.drop";

        // Check if we already built this function
        if let Some(func) = self.module.get_function(func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the drop function
        let func = self.add_func(func_name, self.drop_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let val = func.get_first_param().unwrap();
        let drop_func = self
            .builder
            .build_struct_gep(self.closure_ty(), val.into_pointer_value(), 2, "dropfn")
            .unwrap();
        let drop_func = self
            .builder
            .build_load(self.ptr_ty(), drop_func, "dropfn")
            .unwrap();
        self.builder
            .build_indirect_call(
                self.drop_func_ty(),
                drop_func.into_pointer_value(),
                &[val.into()],
                "",
            )
            .unwrap();
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn any_closure_copy(&self) -> FunctionValue<'ctx> {
        let func_name = "Closure.copy";

        // Check if we already built this function
        if let Some(func) = self.module.get_function(func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the function
        let func = self.add_func(func_name, self.copy_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let dst = func.get_nth_param(0).unwrap();
        let src = func.get_nth_param(1).unwrap();
        let copy_func = self
            .builder
            .build_struct_gep(self.closure_ty(), src.into_pointer_value(), 3, "")
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
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn any_closure_equals(&self) -> FunctionValue<'ctx> {
        let func_name = "Closure.equals";

        // Check if we already built this function
        if let Some(func) = self.module.get_function(func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the function
        let func = self.add_func(func_name, self.equals_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let lhs = func.get_nth_param(0).unwrap();
        let rhs = func.get_nth_param(1).unwrap();
        let equals_func = self
            .builder
            .build_struct_gep(self.closure_ty(), lhs.into_pointer_value(), 4, "equalsfn")
            .unwrap();
        let equals_func = self
            .builder
            .build_load(self.ptr_ty(), equals_func, "equalsfn")
            .unwrap();
        let result = self
            .builder
            .build_indirect_call(
                self.equals_func_ty(),
                equals_func.into_pointer_value(),
                &[lhs.into(), rhs.into()],
                "",
            )
            .unwrap()
            .try_as_basic_value()
            .unwrap_basic();
        self.builder.build_return(Some(&result)).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn closure_drop(&self, name: &str, env_ty: Option<TyId>) -> FunctionValue<'ctx> {
        let func_name = format!("{name}.drop");

        // Check if we already built this function
        if let Some(func) = self.module.get_function(&func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the drop function
        let func = self.add_func(&func_name, self.drop_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        // Don't need to do anything if there's no captures
        if let Some(env_ty) = env_ty {
            let closure = func.get_first_param().unwrap().into_pointer_value();
            let ty = self.closure_ty();

            // Get the environment.
            let env_ptr = self
                .builder
                .build_struct_gep(ty, closure, 1, "envptr")
                .unwrap();
            let env = self
                .builder
                .build_load(self.ptr_ty(), env_ptr, "env")
                .unwrap()
                .into_pointer_value();

            // Drop the environment.
            self.build_drop(env_ty, env);

            // Free the environment's memory.
            self.build_c_call(self.free(), &[env.as_basic_value_enum().into()]);
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn closure_copy(
        &self,
        name: &str,
        captures: &[LocalId],
        env_ty: Option<TyId>,
    ) -> FunctionValue<'ctx> {
        let func_name = format!("{name}.copy");

        // Check if we already built this function.
        if let Some(func) = self.module.get_function(&func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end.
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the copy function.
        let func = self.add_func(&func_name, self.copy_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        // Copy the source into the target.
        let dst = func.get_nth_param(0).unwrap().into_pointer_value();
        let src = func.get_nth_param(1).unwrap().into_pointer_value();
        let ty = self.closure_ty();
        self.build_memcpy(dst, src, &ty);
        // Don't need to clone the environment if there isn't one
        if let Some(env_ty) = env_ty {
            todo!();
            // // Allocate the new target environment
            // let size = env_ty.size_of().unwrap();
            // let dst_env = self
            //     .build_call(self.malloc(), &[size.as_basic_value_enum().into()])
            //     .unwrap()
            //     .into_pointer_value();

            // // Get the source environment
            // let src_env_ptr = self
            //     .builder
            //     .build_struct_gep(ty, src, 1, "srcenvptr")
            //     .unwrap();
            // let src_env = self
            //     .builder
            //     .build_load(self.ptr_ty(), src_env_ptr, "srcenv")
            //     .unwrap()
            //     .into_pointer_value();

            // if captures
            //     .iter()
            //     .all(|id| layout::trivial(&self.mir.var(*id).ty))
            // {
            //     // If all of the captures are trivial, we can memcpy the whole environment
            //     self.build_memcpy(dst_env, src_env, &env_ty);
            // } else {
            //     // If some of the captures aren't trivial, we need to copy each of them individually
            //     for (idx, ty) in captures.iter().map(|id| &self.mir.var(*id).ty).enumerate() {
            //         let idx = u32::try_from(idx).unwrap();
            //         let dst_capture = self
            //             .builder
            //             .build_struct_gep(env_ty, dst_env, idx, "dstcapture")
            //             .unwrap();
            //         let src_capture = self
            //             .builder
            //             .build_struct_gep(env_ty, src_env, idx, "srccapture")
            //             .unwrap();
            //         self.build_copy_to(self.layout_direct(ty, src_capture), dst_capture);
            //     }
            // }

            // // Store the target environment
            // let dst_env_ptr = self
            //     .builder
            //     .build_struct_gep(ty, dst, 1, "dstenvptr")
            //     .unwrap();
            // self.builder.build_store(dst_env_ptr, dst_env).unwrap();
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn closure_equals(&self, name: &str) -> FunctionValue<'ctx> {
        let func_name = format!("{name}.equals");

        // Check if we already built this function
        if let Some(func) = self.module.get_function(&func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the equality function
        let func = self.add_func(&func_name, self.equals_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let lhs = func.get_nth_param(0).unwrap().into_pointer_value();
        let rhs = func.get_nth_param(1).unwrap().into_pointer_value();
        // Load the function pointers, which are stored first in the closures.
        let lhs_fn = self
            .builder
            .build_load(self.ptr_ty(), lhs, "")
            .unwrap()
            .into_pointer_value();
        let rhs_fn = self
            .builder
            .build_load(self.ptr_ty(), rhs, "")
            .unwrap()
            .into_pointer_value();
        // Test whether the function pointers are equal.
        let eq = self
            .builder
            .build_int_compare(IntPredicate::EQ, lhs_fn, rhs_fn, "")
            .unwrap();
        self.builder.build_return(Some(&eq)).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }
}
