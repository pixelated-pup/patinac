use inkwell::{
    FloatPredicate,
    basic_block::BasicBlock,
    types::BasicType as _,
    values::{BasicMetadataValueEnum, BasicValue as _, BasicValueEnum, PointerValue},
};

use irs::{
    mir::{
        Arg, BlockId, Expr, Func, InfixOp, ItemId, LitValue, LocalId, Place, PrefixOp, Stmt,
        Terminator, Value,
    },
    ty::{Ty, TyId},
};
use slotmap::SecondaryMap;

use crate::{CodegenState, StorageClass};

impl<'mir, 'ty_ctx, 'ctx> CodegenState<'mir, 'ty_ctx, 'ctx> {
    fn place_ty(&self, func: &'mir Func, place: &Place) -> TyId {
        match place.accesses.last() {
            Some(access) => access.ty,
            None => func.local(place.base).ty,
        }
    }

    fn value_ty(&self, func: &'mir Func, value: &Value) -> TyId {
        match value {
            Value::Copy(place) | Value::Move(place) => self.place_ty(func, place),
            Value::Lit(lit_value) => match lit_value {
                LitValue::Int(_) => self.ty_ctx.int(),
                LitValue::UInt(_) => self.ty_ctx.uint(),
                LitValue::Byte(_) => self.ty_ctx.byte(),
                LitValue::Float(_) => self.ty_ctx.float(),
                LitValue::Bool(_) => self.ty_ctx.bool(),
            },
            Value::Zst(ty) => *ty,
        }
    }

    pub(crate) fn emit_blocks(&mut self, func: &'mir Func) {
        let mut blocks = SecondaryMap::new();
        for block in func.blocks.keys() {
            let llvm_block = self.emit_block_stmts(func, block);
            blocks.insert(block, llvm_block);
        }
        for block in func.blocks.keys() {
            self.emit_block_term(func, block, &blocks);
        }
    }

    fn emit_block_stmts(&mut self, func: &'mir Func, mir_block: BlockId) -> BasicBlock<'ctx> {
        let llvm_block = self.ctx.append_basic_block(self.curr_function(), "");
        self.builder.position_at_end(llvm_block);

        for stmt in &func.block(mir_block).stmts {
            self.emit_stmt(func, stmt);
        }

        llvm_block
    }

    fn emit_block_term(
        &mut self,
        mir_func: &'mir Func,
        mir_block: BlockId,
        blocks: &SecondaryMap<BlockId, BasicBlock<'ctx>>,
    ) {
        self.builder.position_at_end(blocks[mir_block]);

        match mir_func.block(mir_block).terminator() {
            Terminator::Goto(next_block) => {
                self.builder
                    .build_unconditional_branch(blocks[*next_block])
                    .unwrap();
            }
            Terminator::Switch {
                scrutinee,
                targets,
                otherwise,
            } => {
                let scrutinee = self.emit_value(mir_func, scrutinee).into_int_value();
                let targets: Vec<_> = targets
                    .iter()
                    .map(|(value, block)| {
                        let value = self.ctx.i64_type().const_int(*value, false);
                        (value, blocks[*block])
                    })
                    .collect();
                self.builder
                    .build_switch(scrutinee, blocks[*otherwise], &targets)
                    .unwrap();
            }
            Terminator::Return(value) => {
                let value = self.emit_value(mir_func, value);
                match self.storage_class(mir_func.ret_ty) {
                    StorageClass::Zst => {
                        self.builder.build_return(None).unwrap();
                    }
                    StorageClass::Indirect => {
                        let dst = self
                            .curr_function()
                            .get_first_param()
                            .unwrap()
                            .into_pointer_value();
                        self.build_store(mir_func.ret_ty, value, dst);
                        self.builder.build_return(None).unwrap();
                    }
                    StorageClass::Scalar => {
                        self.builder.build_return(Some(&value)).unwrap();
                    }
                }
            }
            Terminator::Unreachable => {
                self.builder.build_unreachable().unwrap();
            }
        }
    }

    pub(crate) fn emit_stmt(&mut self, func: &'mir Func, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { place, expr } => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                let expr = self.emit_expr(func, expr);
                self.build_store(ty, expr, place);
            }
            Stmt::Call { callee, args, dst } => self.emit_call(func, callee, args, dst),
            Stmt::Drop(place) => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                self.build_drop(ty, place);
            }
            Stmt::Print(value) => self.emit_print(func, value),
        }
    }

    fn emit_expr(&mut self, func: &'mir Func, expr: &Expr) -> BasicValueEnum<'ctx> {
        match expr {
            Expr::Val(value) => self.emit_value(func, value),
            Expr::Infix(op, lhs, rhs) => self.emit_infix(func, *op, lhs, rhs),
            Expr::Prefix(op, arg) => self.emit_prefix(func, *op, arg),
            Expr::Closure { body, env } => self.emit_closure(func, *body, env),
        }
    }

    fn emit_value(&mut self, func: &'mir Func, value: &Value) -> BasicValueEnum<'ctx> {
        match value {
            Value::Copy(place) => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                self.build_copy(ty, place)
            }
            Value::Move(place) => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                self.build_move(ty, place)
            }
            Value::Lit(lit) => self.emit_lit(lit),
            Value::Zst(_) => todo!(),
        }
    }

    fn emit_print(&mut self, func: &'mir Func, value: &Value) {
        let ty = self.value_ty(func, value);
        let format = match self.ty_ctx.get(ty) {
            Ty::Int => "%lld\n",
            Ty::UInt => "%llu\n",
            Ty::Byte => "%hhu\n",
            Ty::Float => "%f\n",
            Ty::Bool => "%hhd\n",
            _ => todo!(),
        };
        let format = self
            .builder
            .build_global_string_ptr(format, "format_string")
            .unwrap()
            .as_pointer_value();

        let value = self.emit_value(func, value);
        self.build_c_call(self.printf(), &[format.into(), value.into()]);
    }

    fn emit_place(&mut self, func: &'mir Func, place: &Place) -> PointerValue<'ctx> {
        let mut base = self.vars[place.base];
        let mut ty = func.local(place.base).ty;

        for access in &place.accesses {
            base = self
                .builder
                .build_struct_gep(self.lower_ty(ty), base, access.index, "")
                .unwrap();
            ty = access.ty;
        }

        base
    }

    fn emit_lit(&self, lit: &LitValue) -> BasicValueEnum<'ctx> {
        match lit {
            LitValue::Int(value) => self.const_int(*value).as_basic_value_enum(),
            LitValue::UInt(value) => self.const_uint(*value).as_basic_value_enum(),
            LitValue::Byte(value) => self.const_byte(*value).as_basic_value_enum(),
            LitValue::Float(value) => self.const_float(*value).as_basic_value_enum(),
            LitValue::Bool(value) => self.const_bool(*value).as_basic_value_enum(),
        }
    }

    fn emit_infix(
        &mut self,
        func: &'mir Func,
        op: InfixOp,
        lhs: &Value,
        rhs: &Value,
    ) -> BasicValueEnum<'ctx> {
        let ty = self.value_ty(func, lhs);
        let lhs = self.emit_value(func, lhs);
        let rhs = self.emit_value(func, rhs);
        match op {
            InfixOp::Add => {
                if ty == self.ty_ctx.float() {
                    self.builder
                        .build_float_add(lhs.into_float_value(), rhs.into_float_value(), "")
                        .unwrap()
                        .as_basic_value_enum()
                } else {
                    self.builder
                        .build_int_add(lhs.into_int_value(), rhs.into_int_value(), "")
                        .unwrap()
                        .as_basic_value_enum()
                }
            }

            InfixOp::Sub => {
                if ty == self.ty_ctx.float() {
                    self.builder
                        .build_float_sub(lhs.into_float_value(), rhs.into_float_value(), "")
                        .unwrap()
                        .as_basic_value_enum()
                } else {
                    self.builder
                        .build_int_sub(lhs.into_int_value(), rhs.into_int_value(), "")
                        .unwrap()
                        .as_basic_value_enum()
                }
            }
            InfixOp::Mul => {
                if ty == self.ty_ctx.float() {
                    self.builder
                        .build_float_mul(lhs.into_float_value(), rhs.into_float_value(), "")
                        .unwrap()
                        .as_basic_value_enum()
                } else {
                    self.builder
                        .build_int_mul(lhs.into_int_value(), rhs.into_int_value(), "")
                        .unwrap()
                        .as_basic_value_enum()
                }
            }
            InfixOp::Div => {
                if ty == self.ty_ctx.float() {
                    self.builder
                        .build_float_div(lhs.into_float_value(), rhs.into_float_value(), "")
                        .unwrap()
                        .as_basic_value_enum()
                } else {
                    todo!()
                }
            }
            InfixOp::Exp => todo!(),
            op @ (InfixOp::Eqq | InfixOp::Neq) => {
                let equals = self.build_equals(ty, lhs, rhs);
                if op == InfixOp::Neq {
                    self.builder.build_not(equals, "").unwrap()
                } else {
                    equals
                }
                .as_basic_value_enum()
            }
            InfixOp::Gt => self
                .builder
                .build_float_compare(
                    FloatPredicate::OGT,
                    lhs.into_float_value(),
                    rhs.into_float_value(),
                    "",
                )
                .unwrap()
                .as_basic_value_enum(),
            InfixOp::Lt => self
                .builder
                .build_float_compare(
                    FloatPredicate::OLT,
                    lhs.into_float_value(),
                    rhs.into_float_value(),
                    "",
                )
                .unwrap()
                .as_basic_value_enum(),
            InfixOp::Geq => self
                .builder
                .build_float_compare(
                    FloatPredicate::OGE,
                    lhs.into_float_value(),
                    rhs.into_float_value(),
                    "",
                )
                .unwrap()
                .as_basic_value_enum(),
            InfixOp::Leq => self
                .builder
                .build_float_compare(
                    FloatPredicate::OLE,
                    lhs.into_float_value(),
                    rhs.into_float_value(),
                    "",
                )
                .unwrap()
                .as_basic_value_enum(),
        }
    }

    fn emit_prefix(&mut self, func: &'mir Func, op: PrefixOp, arg: &Value) -> BasicValueEnum<'ctx> {
        let arg = self.emit_value(func, arg);
        match op {
            PrefixOp::Not => self
                .builder
                .build_not(arg.into_int_value(), "")
                .unwrap()
                .as_basic_value_enum(),
            PrefixOp::Neg => self
                .builder
                .build_float_neg(arg.into_float_value(), "")
                .unwrap()
                .as_basic_value_enum(),
        }
    }

    fn emit_call(&mut self, func: &'mir Func, callee: &Value, args: &[Arg], dst: &Place) {
        let ret_ty = self.place_ty(func, dst);
        let dst = self.emit_place(func, dst);

        let callee_ty = self.value_ty(func, callee);
        let callee = self.emit_value(func, callee);

        let mut args: Vec<_> = args
            .iter()
            .map(|arg| match arg {
                Arg::Value(value) => self.emit_value(func, value).into(),
                Arg::Place(place) => self.emit_place(func, place).into(),
            })
            .collect();

        match self.storage_class(ret_ty) {
            StorageClass::Zst => {
                self.emit_call_inner(callee_ty, callee, args);
            }
            StorageClass::Indirect => {
                args.insert(0, dst.into());
                self.emit_call_inner(callee_ty, callee, args);
            }
            StorageClass::Scalar => {
                let result = self.emit_call_inner(callee_ty, callee, args).unwrap();
                self.build_store(ret_ty, result, dst);
            }
        }
    }

    fn emit_call_inner(
        &mut self,
        callee_ty: TyId,
        callee: BasicValueEnum<'ctx>,
        mut args: Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Option<BasicValueEnum<'ctx>> {
        match self.ty_ctx.get(callee_ty) {
            Ty::Func(params, ret_ty) => {
                let ty = self.closure_ty();
                let closure = callee.into_pointer_value();

                // Extract environment from closure and add it to arguments.
                let env = self.builder.build_struct_gep(ty, closure, 1, "").unwrap();
                let env = self.builder.build_load(self.ptr_ty(), env, "").unwrap();
                args.push(env.as_basic_value_enum().into());

                // Extract function pointer from closure and call it.
                let func = self.builder.build_struct_gep(ty, closure, 0, "").unwrap();
                let func = self.builder.build_load(self.ptr_ty(), func, "").unwrap();
                self.build_indirect_call(func_ty, func.into_pointer_value(), &args)
            }
            // TODO: Direct functoin calls
            _ => unreachable!("wrong type for function"),
        }
    }

    fn emit_closure(&self, func: &'mir Func, body: ItemId, env: &Value) -> BasicValueEnum<'ctx> {
        // Create the environment, if one is needed
        let (env_ptr, env_ty) = if let Value::Zst(_) = env {
            (self.const_null(), None)
        } else {
            let env_ty = self.value_ty(func, env);
            let lowered_env_ty = self.lower_ty(env_ty);
            let env_ptr = self
                .build_call(
                    self.malloc(),
                    &[lowered_env_ty
                        .size_of()
                        .unwrap()
                        .as_basic_value_enum()
                        .into()],
                )
                .unwrap()
                .into_pointer_value();
            let env = self.emit_value(func, env);
            self.build_store(env_ty, env, env_ptr);
            (env_ptr, Some(env_ty))
        };

        // Create the final closure
        let name = self.mir.item(body).ident.str();
        let func = self.funcs[func];
        let ty = self.closure_ty();
        let closure = self.build_alloca_entry(ty, "");

        // Store everything into the closure, emitting the witness functions along the way.
        let store = |idx, val: PointerValue<'ctx>| {
            let ptr = self.builder.build_struct_gep(ty, closure, idx, "").unwrap();
            self.builder.build_store(ptr, val).unwrap();
        };
        store(0, func.as_global_value().as_pointer_value());
        store(1, env_ptr);
        store(
            2,
            self.closure_drop(&name, env_ty)
                .as_global_value()
                .as_pointer_value(),
        );
        store(
            3,
            self.closure_copy(&name, env, env_ty)
                .as_global_value()
                .as_pointer_value(),
        );
        store(
            4,
            self.closure_equals(&name)
                .as_global_value()
                .as_pointer_value(),
        );

        closure.as_basic_value_enum()
    }
}
