use inkwell::{IntPredicate, types::BasicType as _, values::FunctionValue};

use crate::CodegenState;

impl<'ctx> CodegenState<'_, 'ctx> {
    pub(crate) fn printf(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("printf") {
            return func;
        }
        let ty = self.ctx.i32_type().fn_type(&[self.ptr_ty().into()], true);
        self.add_func("printf", ty, true)
    }

    pub(crate) fn malloc(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("ptn_malloc") {
            return func;
        }

        // Save the builder's current insertion block to restore at the end.
        let old_insert_block = self.builder.get_insert_block().unwrap();

        let ty = self.ptr_ty().fn_type(&[self.ctx.i64_type().into()], false);
        let func = self.add_func("ptn_malloc", ty, false);
        let entry_block = self.ctx.append_basic_block(func, "entry");
        let panic_block = self.ctx.append_basic_block(func, "panic");
        let ret_block = self.ctx.append_basic_block(func, "return");

        self.builder.position_at_end(entry_block);
        let ptr = self
            .build_c_call(self.c_malloc(), &[func.get_first_param().unwrap().into()])
            .unwrap()
            .into_pointer_value();
        let is_null = self
            .builder
            .build_int_compare(IntPredicate::EQ, ptr, self.const_null(), "")
            .unwrap();
        self.builder
            .build_conditional_branch(is_null, panic_block, ret_block)
            .unwrap();

        self.builder.position_at_end(panic_block);
        self.build_panic("allocation failed");

        self.builder.position_at_end(ret_block);
        self.builder.build_return(Some(&ptr)).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    fn c_malloc(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("malloc") {
            return func;
        }
        let ty = self.ptr_ty().fn_type(&[self.ctx.i64_type().into()], false);
        self.add_func("malloc", ty, true)
    }

    pub(crate) fn free(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("free") {
            return func;
        }
        let ty = self.ctx.void_type().fn_type(&[self.ptr_ty().into()], false);
        self.add_func("free", ty, true)
    }

    pub(crate) fn panic(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("panic") {
            return func;
        }

        // Save the builder's current insertion block to restore at the end.
        let old_insert_block = self.builder.get_insert_block().unwrap();

        let ty = self.ctx.void_type().fn_type(&[self.ptr_ty().into()], false);
        let func = self.add_func("panic", ty, false);
        self.builder
            .position_at_end(self.ctx.append_basic_block(func, "entry"));
        self.build_c_call(self.printf(), &[func.get_first_param().unwrap().into()]);
        self.build_c_call(
            self.exit(),
            &[self.ctx.i32_type().const_int(1, false).into()],
        );
        self.builder.build_unreachable().unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    fn exit(&self) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function("exit") {
            return func;
        }
        let ty = self
            .ctx
            .void_type()
            .fn_type(&[self.ctx.i32_type().into()], false);
        self.add_func("exit", ty, true)
    }
}
