use ident::Ident;
use inkwell::{
    IntPredicate,
    types::{BasicType, FunctionType},
    values::{BasicValue as _, FunctionValue},
};

use irs::ty::TyId;

use crate::CodegenState;

impl<'ctx> CodegenState<'_, 'ctx> {
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

    pub fn tuple_drop(&mut self, elem_tys: &[TyId]) -> FunctionValue<'ctx> {
        let name = self.mangle_tuple(fields);
        self.fields_drop(elem_tys.iter().copied(), lowered_ty, &name)
    }

    pub fn struct_drop(
        &mut self,
        name: &str,
        fields: impl IntoIterator<Item = TyId>,
    ) -> FunctionValue<'ctx> {
        self.fields_drop(fields, lowered_ty, name)
    }

    pub fn fields_drop(
        &mut self,
        ty: TyId,
        fields: impl IntoIterator<Item = TyId>,
    ) -> FunctionValue<'ctx> {
        let name = mangle::mangle_assoc_func(self.pkg, self.ty_ctx, ty, Ident::new("drop"));

        // Check if we already built this function
        if let Some(func) = self.module.get_function(&name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the function.
        let func = self.add_func(&name, self.drop_func_ty(), false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        let base = func.get_nth_param(0).unwrap().into_pointer_value();

        // Drop each field.
        for (idx, field_ty) in fields.into_iter().enumerate() {
            let field_ptr = self.build_field(ty, base, idx);
            self.build_drop(field_ty, field_ptr);
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub fn fields_copy(&mut self, ty: TyId, fields: &[TyId]) -> FunctionValue<'ctx> {
        let func_name = format!("{}.copy", self.mangle_tuple(fields));

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
        let dst = func.get_nth_param(0).unwrap().into_pointer_value();
        let src = func.get_nth_param(1).unwrap().into_pointer_value();

        if self.ty_ctx.is_trivial(ty) {
            self.build_memcpy(ty, dst, src);
        } else {
            // If the fields aren't all trivial, we need to copy each one individually.
            for (idx, field_ty) in fields.iter().enumerate() {
                let dst = self.build_field(ty, dst, idx);
                let src = self.build_field(ty, src, idx);

                self.build_copy_to(*field_ty, src, dst);
            }
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub fn fields_equals(&mut self, ty: TyId, fields: &[TyId]) -> FunctionValue<'ctx> {
        let func_name = format!("{}.equals", self.mangle_tuple(fields));

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
        let lhs = func.get_nth_param(0).unwrap().into_pointer_value();
        let rhs = func.get_nth_param(1).unwrap().into_pointer_value();
        let ne_block = self.ctx.append_basic_block(func, "ne");

        for (idx, field_ty) in fields.iter().enumerate() {
            let lhs = self.build_field(ty, lhs, idx);
            let rhs = self.build_field(ty, rhs, idx);

            let (lhs, rhs) = if self.ty_ctx.is_indirect(*field_ty) {
                (lhs.as_basic_value_enum(), rhs.as_basic_value_enum())
            } else {
                let field_ty = self.lower_ty(*field_ty);
                let lhs = self.builder.build_load(field_ty, lhs, "").unwrap();
                let rhs = self.builder.build_load(field_ty, rhs, "").unwrap();
                (lhs, rhs)
            };

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
        let lhs = func.get_nth_param(0).unwrap().into_pointer_value();
        let rhs = func.get_nth_param(1).unwrap().into_pointer_value();

        // Load the function pointers, which are stored first in the closures.
        let lhs_func = self
            .builder
            .build_load(self.ptr_ty(), lhs, "")
            .unwrap()
            .into_pointer_value();
        let rhs_func = self
            .builder
            .build_load(self.ptr_ty(), rhs, "")
            .unwrap()
            .into_pointer_value();

        // Test whether the function pointers are equal.
        let eq = self
            .builder
            .build_int_compare(IntPredicate::EQ, lhs_func, rhs_func, "")
            .unwrap();
        self.builder.build_return(Some(&eq)).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn closure_drop(&mut self, name: &str, env_ty: Option<TyId>) -> FunctionValue<'ctx> {
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

    pub(crate) fn closure_copy(&mut self, name: &str, env_ty: Option<TyId>) -> FunctionValue<'ctx> {
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
        let dst = func.get_nth_param(0).unwrap().into_pointer_value();
        let src = func.get_nth_param(1).unwrap().into_pointer_value();
        let ty = self.closure_ty();

        // Copy the source into the destination as-is.
        let align = self.target.get_target_data().get_abi_alignment(&ty);
        let size = ty.size_of().unwrap();
        self.builder
            .build_memcpy(dst, align, src, align, size)
            .unwrap();

        if let Some(env_ty) = env_ty {
            // Allocate the new environment.
            let dst_env = self.build_malloc(env_ty);

            // Get the source environment.
            let src_env = self.builder.build_struct_gep(ty, src, 1, "").unwrap();
            let src_env = self
                .builder
                .build_load(self.ptr_ty(), src_env, "")
                .unwrap()
                .into_pointer_value();

            // Copy the environment.
            self.build_copy_to(env_ty, src_env, dst_env);

            // Store the new environment.
            let env_ptr = self.builder.build_struct_gep(ty, dst, 1, "").unwrap();
            self.builder.build_store(env_ptr, dst_env).unwrap();
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }
}
