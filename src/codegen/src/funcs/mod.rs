pub mod runtime;
pub mod witnesses;

use std::{array, fmt::Debug};

use inkwell::{
    basic_block::BasicBlock,
    types::FunctionType,
    values::{BasicValueEnum, FunctionValue},
};

use crate::CodegenState;

impl<'ctx> CodegenState<'_, 'ctx> {
    fn func<const BLOCKS: usize, const ARGS: usize, ARG>(
        &mut self,
        name: &str,
        ty: FunctionType<'ctx>,
        body: impl FnOnce(&mut Self, FunctionValue<'ctx>, [ARG; ARGS], [BasicBlock<'ctx>; BLOCKS]),
    ) -> FunctionValue<'ctx>
    where
        ARG: TryFrom<BasicValueEnum<'ctx>, Error: Debug>,
    {
        if let Some(func) = self.module.get_function(name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end.
        let prev_block = self.builder.get_insert_block().unwrap();

        let func = self.add_func(name, ty, false);
        let args = array::from_fn(|i| {
            func.get_nth_param(i.try_into().unwrap())
                .unwrap()
                .try_into()
                .unwrap()
        });
        let blocks = array::from_fn(|_| self.ctx.append_basic_block(func, ""));
        body(self, func, args, blocks);

        self.builder.position_at_end(prev_block);

        func
    }

    fn extern_func(&self, name: &str, ty: FunctionType<'ctx>) -> FunctionValue<'ctx> {
        if let Some(func) = self.module.get_function(name) {
            return func;
        }
        self.add_func(name, ty, true)
    }
}
