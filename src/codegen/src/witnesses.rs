use inkwell::{
    IntPredicate,
    types::{FunctionType, StructType},
    values::{BasicValue as _, FunctionValue},
};

use irs::mir::{Ty, LocalId};

use crate::{CodegenState, layout};

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

    pub fn fields_drop(&self, fields: &[Ty]) -> FunctionValue<'ctx> {
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
            .filter(|(_, ty)| !layout::trivial(ty))
        {
            let field_ptr = self
                .builder
                .build_struct_gep(ty, out, u32::try_from(idx).unwrap(), "fieldptr")
                .unwrap();
            self.build_drop(self.layout_direct(field_ty, field_ptr));
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub fn fields_copy(&self, fields: &[Ty]) -> FunctionValue<'ctx> {
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

        if layout::all_trivial(fields) {
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

                self.build_copy(self.layout_direct(field_ty, src), dst);
            }
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub fn fields_equals(&self, fields: &[Ty]) -> FunctionValue<'ctx> {
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
            let equal = self.build_equals(
                self.layout_direct(field_ty, lhs),
                self.layout_direct(field_ty, rhs),
            );
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

    // pub(crate) fn array_drop(&self, elem_ty: &Ty) -> FunctionValue<'ctx> {
    //     let func_name = format!("{}.drop", self.mangle_array_ty(elem_ty));

    //     // Check if we already built this function.
    //     if let Some(func) = self.module.get_function(&func_name) {
    //         return func;
    //     }

    //     // Save the builder's current insertion block to restore at the end.
    //     let old_insert_block = self.builder.get_insert_block().unwrap();

    //     // Create the function and blocks, and extract the arguments.
    //     let func = self.add_func(&func_name, self.drop_func_ty(), false);
    //     let entry_block = self.ctx.append_basic_block(func, "entry");
    //     let decr_block = self.ctx.append_basic_block(func, "decr");
    //     let drop_block = self.ctx.append_basic_block(func, "drop");
    //     let loop_block = self.ctx.append_basic_block(func, "loop");
    //     let free_block = self.ctx.append_basic_block(func, "free");
    //     let ret_block = self.ctx.append_basic_block(func, "return");
    //     let array = func.get_first_param().unwrap().into_pointer_value();

    //     // Return immediately if the array hasn't been allocated.
    //     // Also set up a stack allocation for later.
    //     let (header, index) = {
    //         self.builder.position_at_end(entry_block);
    //         let index = self
    //             .builder
    //             .build_alloca(self.ctx.i64_type(), "index")
    //             .unwrap();
    //         let header = self.get_array_header(array);
    //         let is_null = self
    //             .builder
    //             .build_int_compare(IntPredicate::EQ, header, self.const_null(), "")
    //             .unwrap();
    //         self.builder
    //             .build_conditional_branch(is_null, ret_block, decr_block)
    //             .unwrap();
    //         (header, index)
    //     };

    //     // Decrement refcount and branch to drop block if it hits 0 and the element type needs to be dropped.
    //     // We rely on LLVM to optimise out the drop block if it isn't needed for the element type.
    //     {
    //         self.builder.position_at_end(decr_block);
    //         let refc = self
    //             .builder
    //             .build_struct_gep(self.array_header_ty(), header, 0, "refc")
    //             .unwrap();
    //         let old_refc = self
    //             .builder
    //             .build_atomicrmw(
    //                 AtomicRMWBinOp::Sub,
    //                 refc,
    //                 self.const_int(1),
    //                 AtomicOrdering::AcquireRelease,
    //             )
    //             .unwrap();
    //         let no_refs = self
    //             .builder
    //             .build_int_compare(IntPredicate::EQ, old_refc, self.const_int(1), "")
    //             .unwrap();

    //         let target_block = if layout::trivial(elem_ty) {
    //             free_block
    //         } else {
    //             drop_block
    //         };
    //         self.builder
    //             .build_conditional_branch(no_refs, target_block, ret_block)
    //             .unwrap();
    //     }

    //     // Initialise the loop to drop all the elements.
    //     let (count, index) = {
    //         self.builder.position_at_end(drop_block);
    //         let count = self
    //             .builder
    //             .build_struct_gep(self.array_header_ty(), header, 1, "count")
    //             .unwrap();
    //         let count = self
    //             .builder
    //             .build_load(self.ctx.i64_type(), count, "")
    //             .unwrap()
    //             .into_int_value();
    //         self.builder
    //             .build_store(index, self.ctx.i64_type().const_zero())
    //             .unwrap();
    //         let empty = self
    //             .builder
    //             .build_int_compare(
    //                 IntPredicate::EQ,
    //                 count,
    //                 self.ctx.i64_type().const_zero(),
    //                 "",
    //             )
    //             .unwrap();
    //         self.builder
    //             .build_conditional_branch(empty, free_block, loop_block)
    //             .unwrap();
    //         (count, index)
    //     };

    //     // Loop over each element and free it.
    //     {
    //         self.builder.position_at_end(loop_block);
    //         let curr_index = self
    //             .builder
    //             .build_load(self.ctx.i64_type(), index, "")
    //             .unwrap()
    //             .into_int_value();
    //         let elem_ptr = self.build_index(
    //             LayoutValue::array(elem_ty, array),
    //             LayoutValue::int(IntSize::Bits64, curr_index),
    //         );
    //         self.build_drop(elem_ptr);
    //         let new_index = self
    //             .builder
    //             .build_int_add(curr_index, self.const_int(1), "")
    //             .unwrap();
    //         self.builder.build_store(index, new_index).unwrap();
    //         let done = self
    //             .builder
    //             .build_int_compare(IntPredicate::UGE, new_index, count, "")
    //             .unwrap();
    //         self.builder
    //             .build_conditional_branch(done, free_block, loop_block)
    //             .unwrap();
    //     }

    //     // Free the memory allocation.
    //     {
    //         self.builder.position_at_end(free_block);
    //         self.build_c_call(self.free(), &[header.into()]);
    //         self.builder.build_unconditional_branch(ret_block).unwrap();
    //     }

    //     // Return.
    //     {
    //         self.builder.position_at_end(ret_block);
    //         self.builder.build_return(None).unwrap();
    //     }

    //     self.builder.position_at_end(old_insert_block);

    //     func
    // }

    // pub(crate) fn array_equals(&self, elem_ty: &Ty) -> FunctionValue<'ctx> {
    //     let func_name = format!("{}.equals", self.mangle_array_ty(elem_ty));

    //     // Check if we already built this function
    //     if let Some(func) = self.module.get_function(&func_name) {
    //         return func;
    //     }

    //     // Save the builder's current insertion block to restore at the end
    //     let old_insert_block = self.builder.get_insert_block().unwrap();

    //     // Create the copy function
    //     let func = self.add_func(&func_name, self.equals_func_ty(), false);
    //     let entry_block = self.ctx.append_basic_block(func, "entry");
    //     let null_block = self.ctx.append_basic_block(func, "null");
    //     let count_block = self.ctx.append_basic_block(func, "count");
    //     let empty_block = self.ctx.append_basic_block(func, "empty");
    //     let loop_block = self.ctx.append_basic_block(func, "loop");
    //     let ret_block = self.ctx.append_basic_block(func, "return");
    //     let lhs = func.get_nth_param(0).unwrap().into_pointer_value();
    //     let rhs = func.get_nth_param(1).unwrap().into_pointer_value();

    //     // Always equal if the arrays share storage. This will also handle both arrays being unallocated.
    //     // Also set up a stack allocation for later.
    //     let index = {
    //         self.builder.position_at_end(entry_block);
    //         let index = self
    //             .builder
    //             .build_alloca(self.ctx.i64_type(), "index")
    //             .unwrap();
    //         self.builder
    //             .build_store(index, self.ctx.i64_type().const_zero())
    //             .unwrap();

    //         let equal = self
    //             .builder
    //             .build_int_compare(IntPredicate::EQ, lhs, lhs, "")
    //             .unwrap();
    //         self.builder
    //             .build_conditional_branch(equal, ret_block, null_block)
    //             .unwrap();
    //         index
    //     };

    //     // If either array is unallocated, they're not equal (both being unallocated is handled in the entry block).
    //     {
    //         self.builder.position_at_end(null_block);
    //         let lhs_null = self
    //             .builder
    //             .build_int_compare(IntPredicate::EQ, lhs, self.const_null(), "")
    //             .unwrap();
    //         let rhs_null = self
    //             .builder
    //             .build_int_compare(IntPredicate::EQ, rhs, self.const_null(), "")
    //             .unwrap();
    //         let either_null = self.builder.build_or(lhs_null, rhs_null, "").unwrap();
    //         self.builder
    //             .build_conditional_branch(either_null, ret_block, count_block)
    //             .unwrap();
    //     }

    //     // If the arrays' counts aren't equal, they're not equal.
    //     let count = {
    //         self.builder.position_at_end(count_block);
    //         let lhs_header = self.get_array_header(lhs);
    //         let lhs_count = self
    //             .builder
    //             .build_struct_gep(self.array_header_ty(), lhs_header, 1, "")
    //             .unwrap();
    //         let lhs_count = self
    //             .builder
    //             .build_load(self.ctx.i64_type(), lhs_count, "")
    //             .unwrap()
    //             .into_int_value();
    //         let rhs_header = self.get_array_header(rhs);
    //         let rhs_count = self
    //             .builder
    //             .build_struct_gep(self.array_header_ty(), rhs_header, 1, "")
    //             .unwrap();
    //         let rhs_count = self
    //             .builder
    //             .build_load(self.ctx.i64_type(), rhs_count, "")
    //             .unwrap()
    //             .into_int_value();
    //         let counts_eq = self
    //             .builder
    //             .build_int_compare(IntPredicate::EQ, lhs_count, rhs_count, "")
    //             .unwrap();
    //         self.builder
    //             .build_conditional_branch(counts_eq, empty_block, ret_block)
    //             .unwrap();
    //         lhs_count
    //     };

    //     // If the arrays are empty, they're equal.
    //     {
    //         self.builder.position_at_end(empty_block);
    //         let empty = self
    //             .builder
    //             .build_int_compare(
    //                 IntPredicate::EQ,
    //                 count,
    //                 self.ctx.i64_type().const_zero(),
    //                 "",
    //             )
    //             .unwrap();
    //         self.builder
    //             .build_conditional_branch(empty, ret_block, loop_block)
    //             .unwrap();
    //     }

    //     // Loop over each pair of elements to check their equality.
    //     let equal = {
    //         self.builder.position_at_end(loop_block);
    //         let curr_index = self
    //             .builder
    //             .build_load(self.ctx.i64_type(), index, "")
    //             .unwrap()
    //             .into_int_value();
    //         let curr_index_val = LayoutValue::int(IntSize::Bits64, curr_index);
    //         let lhs_elem = self.build_index(LayoutValue::array(elem_ty, lhs), curr_index_val);
    //         let rhs_elem = self.build_index(LayoutValue::array(elem_ty, rhs), curr_index_val);
    //         // Need to load the elements if they're direct.
    //         let equal = self.build_equals(
    //             self.layout_direct(elem_ty, lhs_elem.as_pointer()),
    //             self.layout_direct(elem_ty, rhs_elem.as_pointer()),
    //         );
    //         let new_index = self
    //             .builder
    //             .build_int_add(curr_index, self.const_uint(1), "")
    //             .unwrap();
    //         self.builder.build_store(index, new_index).unwrap();
    //         let done = self
    //             .builder
    //             .build_int_compare(IntPredicate::UGE, new_index, count, "")
    //             .unwrap();
    //         let should_cont = self
    //             .builder
    //             .build_select(done, self.ctx.bool_type().const_zero(), equal, "")
    //             .unwrap()
    //             .into_int_value();
    //         self.builder
    //             .build_conditional_branch(should_cont, loop_block, ret_block)
    //             .unwrap();
    //         equal
    //     };

    //     // Return the equality value.
    //     {
    //         self.builder.position_at_end(ret_block);
    //         let ret_val = self.builder.build_phi(self.ctx.bool_type(), "").unwrap();
    //         ret_val.add_incoming(&[
    //             (&self.const_bool(true), entry_block),
    //             (&self.const_bool(false), null_block),
    //             (&self.const_bool(false), count_block),
    //             (&self.const_bool(true), empty_block),
    //             (&equal, loop_block),
    //         ]);
    //         self.builder
    //             .build_return(Some(&ret_val.as_basic_value()))
    //             .unwrap();
    //     }

    //     self.builder.position_at_end(old_insert_block);

    //     func
    // }

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

    pub(crate) fn closure_drop(
        &self,
        name: &str,
        captures: &[LocalId],
        env_ty: Option<StructType<'ctx>>,
    ) -> FunctionValue<'ctx> {
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

            // Drop each non-trivial capture
            for (idx, id) in captures.iter().enumerate() {
                let ty = &self.mir.var(*id).ty;
                if layout::trivial(ty) {
                    continue;
                }
                let capture_ptr = self
                    .builder
                    .build_struct_gep(env_ty, env, u32::try_from(idx).unwrap(), "")
                    .unwrap();
                self.build_drop(self.layout_direct(ty, capture_ptr));
            }

            // Free the environment's memory
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
        env_ty: Option<StructType<'ctx>>,
    ) -> FunctionValue<'ctx> {
        let func_name = format!("{name}.copy");

        // Check if we already built this function
        if let Some(func) = self.module.get_function(&func_name) {
            return func;
        }

        // Save the builder's current insertion block to restore at the end
        let old_insert_block = self.builder.get_insert_block().unwrap();

        // Create the copy function
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
            // Allocate the new target environment
            let size = env_ty.size_of().unwrap();
            let dst_env = self
                .build_call(self.malloc(), &[size.as_basic_value_enum().into()])
                .unwrap()
                .into_pointer_value();

            // Get the source environment
            let src_env_ptr = self
                .builder
                .build_struct_gep(ty, src, 1, "srcenvptr")
                .unwrap();
            let src_env = self
                .builder
                .build_load(self.ptr_ty(), src_env_ptr, "srcenv")
                .unwrap()
                .into_pointer_value();

            if captures
                .iter()
                .all(|id| layout::trivial(&self.mir.var(*id).ty))
            {
                // If all of the captures are trivial, we can memcpy the whole environment
                self.build_memcpy(dst_env, src_env, &env_ty);
            } else {
                // If some of the captures aren't trivial, we need to copy each of them individually
                for (idx, ty) in captures.iter().map(|id| &self.mir.var(*id).ty).enumerate() {
                    let idx = u32::try_from(idx).unwrap();
                    let dst_capture = self
                        .builder
                        .build_struct_gep(env_ty, dst_env, idx, "dstcapture")
                        .unwrap();
                    let src_capture = self
                        .builder
                        .build_struct_gep(env_ty, src_env, idx, "srccapture")
                        .unwrap();
                    self.build_copy(self.layout_direct(ty, src_capture), dst_capture);
                }
            }

            // Store the target environment
            let dst_env_ptr = self
                .builder
                .build_struct_gep(ty, dst, 1, "dstenvptr")
                .unwrap();
            self.builder.build_store(dst_env_ptr, dst_env).unwrap();
        }
        self.builder.build_return(None).unwrap();

        self.builder.position_at_end(old_insert_block);

        func
    }

    pub(crate) fn closure_equals(
        &self,
        name: &str,
        captures: &[LocalId],
        env_ty: Option<StructType<'ctx>>,
    ) -> FunctionValue<'ctx> {
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
        let ty = self.closure_ty();
        // Test whether the lifted closure pointers are equal.
        let lhs_fn = self.builder.build_struct_gep(ty, lhs, 0, "lhsfn").unwrap();
        let lhs_fn = self
            .builder
            .build_load(self.ptr_ty(), lhs_fn, "lhsfn")
            .unwrap()
            .into_pointer_value();
        let rhs_fn = self.builder.build_struct_gep(ty, rhs, 0, "rhsfn").unwrap();
        let rhs_fn = self
            .builder
            .build_load(self.ptr_ty(), rhs_fn, "rhsfn")
            .unwrap()
            .into_pointer_value();
        let test = self
            .builder
            .build_int_compare(IntPredicate::EQ, lhs_fn, rhs_fn, "fntest")
            .unwrap();
        // If there's no env, we can just return whether the pointers are equal
        match env_ty {
            None => {
                self.builder.build_return(Some(&test)).unwrap();
            }
            Some(env_ty) => {
                let ne_block = self.ctx.append_basic_block(func, "ne");
                let eq_block = self.ctx.append_basic_block(func, "eq");
                self.builder
                    .build_conditional_branch(test, eq_block, ne_block)
                    .unwrap();
                self.builder.position_at_end(eq_block);

                let lhs_env = self.builder.build_struct_gep(ty, lhs, 1, "lhsenv").unwrap();
                let lhs_env = self
                    .builder
                    .build_load(self.ptr_ty(), lhs_env, "lhs_env")
                    .unwrap()
                    .into_pointer_value();
                let rhs_env = self.builder.build_struct_gep(ty, rhs, 1, "rhsenv").unwrap();
                let rhs_env = self
                    .builder
                    .build_load(self.ptr_ty(), rhs_env, "rhs_env")
                    .unwrap()
                    .into_pointer_value();

                for (idx, ty) in captures.iter().map(|id| &self.mir.var(*id).ty).enumerate() {
                    let idx = u32::try_from(idx).unwrap();
                    let lhs_capture = self
                        .builder
                        .build_struct_gep(env_ty, lhs_env, idx, "lhscapture")
                        .unwrap();
                    let rhs_capture = self
                        .builder
                        .build_struct_gep(env_ty, rhs_env, idx, "rhscapture")
                        .unwrap();
                    let equal = self.build_equals(
                        self.layout_direct(ty, lhs_capture),
                        self.layout_direct(ty, rhs_capture),
                    );
                    // Bail out if we found a difference.
                    let eq_block = self.ctx.append_basic_block(func, "eq");
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
            }
        }

        self.builder.position_at_end(old_insert_block);

        func
    }
}
