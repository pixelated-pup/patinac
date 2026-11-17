use ident::Ident;
use inkwell::{
    types::{BasicType, FunctionType},
    values::FunctionValue,
};

use irs::ty::{DefId, TyId};

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

    pub(crate) fn fields_drop<'fields>(
        &mut self,
        ty: TyId,
        fields: impl IntoIterator<Item = &'fields TyId>,
    ) -> FunctionValue<'ctx> {
        self.func(
            &mangle::assoc_item_symbol(self.pkg, self.ty_ctx, ty, Ident::new("_drop")),
            self.drop_func_ty(),
            |this, _, [base], [entry_block]| {
                this.builder.position_at_end(entry_block);
                // Drop each field.
                for (idx, &field_ty) in fields.into_iter().enumerate() {
                    let field_ptr = this.build_field(ty, base, idx);
                    this.build_drop(field_ty, field_ptr);
                }
                this.builder.build_return(None).unwrap();
            },
        )
    }

    pub(crate) fn fields_copy<'fields>(
        &mut self,
        ty: TyId,
        fields: impl IntoIterator<Item = &'fields TyId>,
    ) -> FunctionValue<'ctx> {
        self.func(
            &mangle::assoc_item_symbol(self.pkg, self.ty_ctx, ty, Ident::new("_copy")),
            self.copy_func_ty(),
            |this, _, [dst, src], [entry_block]| {
                this.builder.position_at_end(entry_block);
                if this.ty_ctx.is_trivial(ty) {
                    this.build_memcpy(ty, dst, src);
                } else {
                    // If the fields aren't all trivial, we need to copy each one individually.
                    for (idx, &field_ty) in fields.into_iter().enumerate() {
                        let dst = this.build_field(ty, dst, idx);
                        let src = this.build_field(ty, src, idx);

                        this.build_copy(field_ty, src, dst);
                    }
                }
                this.builder.build_return(None).unwrap();
            },
        )
    }

    pub(crate) fn fields_equals<'fields>(
        &mut self,
        ty: TyId,
        fields: impl IntoIterator<Item = &'fields TyId>,
    ) -> FunctionValue<'ctx> {
        self.func(
            &mangle::assoc_item_symbol(self.pkg, self.ty_ctx, ty, Ident::new("_eq")),
            self.equals_func_ty(),
            |this, func, [lhs, rhs], [entry_block, ne_block]| {
                this.builder.position_at_end(entry_block);

                for (idx, &field_ty) in fields.into_iter().enumerate() {
                    let lhs = this.build_field(ty, lhs, idx);
                    let lhs = this.build_read(field_ty, lhs);

                    let rhs = this.build_field(ty, rhs, idx);
                    let rhs = this.build_read(field_ty, rhs);

                    // If the fields are equal, continue to a new block for the next comparison, else branch to the not-equal block.
                    let equal = this.build_equals(field_ty, lhs, rhs);
                    let eq_block = this.ctx.append_basic_block(func, "");
                    this.builder
                        .build_conditional_branch(equal, eq_block, ne_block)
                        .unwrap();
                    this.builder.position_at_end(eq_block);
                }

                this.builder
                    .build_return(Some(&this.const_bool(true)))
                    .unwrap();

                ne_block.move_after(this.curr_block()).unwrap();
                this.builder.position_at_end(ne_block);
                this.builder
                    .build_return(Some(&this.const_bool(false)))
                    .unwrap();
            },
        )
    }

    pub(crate) fn closure_drop(&mut self, body: DefId, env_ty: TyId) -> FunctionValue<'ctx> {
        self.func(
            &mangle::nested_item_symbol(self.pkg, self.ty_ctx, body, Ident::new("_drop")),
            self.drop_func_ty(),
            |this, _, [closure], [entry_block]| {
                this.builder.position_at_end(entry_block);

                // Don't need to do anything if the environment is empty.
                if !this.ty_ctx.is_zst(env_ty) {
                    let ty = this.closure_ty();

                    // Get the environment.
                    let env = this.builder.build_struct_gep(ty, closure, 1, "").unwrap();
                    let env = this.builder.build_load(this.ptr_ty(), env, "").unwrap();

                    // Drop the environment.
                    this.build_drop(env_ty, env.into_pointer_value());

                    // Free the environment's memory.
                    this.build_c_call(this.free(), &[env.into()]);
                }

                this.builder.build_return(None).unwrap();
            },
        )
    }

    pub(crate) fn closure_copy(&mut self, body: DefId, env_ty: TyId) -> FunctionValue<'ctx> {
        self.func(
            &mangle::nested_item_symbol(self.pkg, self.ty_ctx, body, Ident::new("_copy")),
            self.copy_func_ty(),
            |this, _, [dst, src], [entry_block]| {
                this.builder.position_at_end(entry_block);
                let ty = this.closure_ty();

                // Copy the source into the destination as-is.
                let align = this.target.get_target_data().get_abi_alignment(&ty);
                let size = ty.size_of().unwrap();
                this.builder
                    .build_memcpy(dst, align, src, align, size)
                    .unwrap();

                // Don't need to do anything further if the environment is empty.
                if !this.ty_ctx.is_zst(env_ty) {
                    // Allocate the new environment.
                    let dst_env = this.build_malloc(env_ty);

                    // Get the source environment.
                    let src_env = this.builder.build_struct_gep(ty, src, 1, "").unwrap();
                    let src_env = this
                        .builder
                        .build_load(this.ptr_ty(), src_env, "")
                        .unwrap()
                        .into_pointer_value();

                    // Copy the environment.
                    this.build_copy(env_ty, src_env, dst_env);

                    // Store the new environment.
                    let env_ptr = this.builder.build_struct_gep(ty, dst, 1, "").unwrap();
                    this.builder.build_store(env_ptr, dst_env).unwrap();
                }

                this.builder.build_return(None).unwrap();
            },
        )
    }
}
