use inkwell::{IntPredicate, types::BasicType as _, values::FunctionValue};

use crate::CodegenState;

impl<'ctx> CodegenState<'_, 'ctx> {
    pub(crate) fn malloc(&mut self) -> FunctionValue<'ctx> {
        let c_malloc = self.extern_func(
            "malloc",
            self.ptr_ty().fn_type(&[self.ctx.i64_type().into()], false),
        );

        self.func(
            &symbol("malloc"),
            self.ptr_ty().fn_type(&[self.ctx.i64_type().into()], false),
            |this, _, [size], [entry_block, panic_block, ret_block]| {
                this.builder.position_at_end(entry_block);
                let ptr = this
                    .build_c_call(c_malloc, &[size])
                    .unwrap()
                    .into_pointer_value();
                let is_null = this
                    .builder
                    .build_int_compare(IntPredicate::EQ, ptr, this.const_null(), "")
                    .unwrap();
                this.builder
                    .build_conditional_branch(is_null, panic_block, ret_block)
                    .unwrap();

                this.builder.position_at_end(panic_block);
                this.build_panic("allocation failed");

                this.builder.position_at_end(ret_block);
                this.builder.build_return(Some(&ptr)).unwrap();
            },
        )
    }

    pub(crate) fn free(&self) -> FunctionValue<'ctx> {
        self.extern_func(
            "free",
            self.ctx.void_type().fn_type(&[self.ptr_ty().into()], false),
        )
    }

    pub(crate) fn printf(&self) -> FunctionValue<'ctx> {
        self.extern_func(
            "printf",
            self.ctx.i32_type().fn_type(&[self.ptr_ty().into()], true),
        )
    }

    pub(crate) fn panic(&mut self) -> FunctionValue<'ctx> {
        let exit = self.extern_func(
            "exit",
            self.ctx
                .void_type()
                .fn_type(&[self.ctx.i32_type().into()], false),
        );

        self.func(
            &symbol("panic"),
            self.ctx.void_type().fn_type(&[self.ptr_ty().into()], false),
            |this, _, [msg], [entry_block]| {
                this.builder.position_at_end(entry_block);
                this.build_c_call(this.printf(), &[msg]);
                this.build_c_call(exit, &[this.ctx.i32_type().const_int(1, false).into()]);
                this.builder.build_unreachable().unwrap();
            },
        )
    }
}

fn symbol(name: &str) -> String {
    format!("_PNR4_ptn{}{name}", name.len())
}
