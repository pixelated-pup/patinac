use inkwell::{
    FloatPredicate, IntPredicate,
    basic_block::BasicBlock,
    builder::BuilderError,
    values::{
        BasicMetadataValueEnum, BasicValue, BasicValueEnum, FloatValue, IntValue, PointerValue,
    },
};

use irs::{
    mir::{
        Arg, BlockId, Callee, Expr, Func, InfixOp, LitValue, Place, PrefixOp, Stmt, Terminator,
        Value,
    },
    ty::{DefId, StorageClass, Ty, TyId},
};
use slotmap::SecondaryMap;

use crate::CodegenState;

impl<'mir, 'ty_ctx, 'ctx> CodegenState<'mir, 'ty_ctx, 'ctx> {
    fn place_ty(&self, func: &'mir Func, place: &Place) -> TyId {
        match place.accesses.last() {
            Some(access) => access.ty,
            None => func.local(place.base).ty,
        }
    }

    fn value_ty(&self, func: &'mir Func, value: &Value) -> TyId {
        match value {
            Value::Copy(place) | Value::Share(place) => self.place_ty(func, place),
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

    pub(crate) fn emit_blocks(&mut self, def: DefId) {
        let func = self.mir.func(def);

        let mut blocks = SecondaryMap::new();

        for block in func.blocks.keys() {
            let llvm_block = self.emit_block_stmts(func, block);
            blocks.insert(block, llvm_block);
        }
        for block in func.blocks.keys() {
            self.emit_block_term(&blocks, def, func, block);
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
        blocks: &SecondaryMap<BlockId, BasicBlock<'ctx>>,
        def: DefId,
        func: &'mir Func,
        block: BlockId,
    ) {
        self.builder.position_at_end(blocks[block]);

        match func.block(block).terminator() {
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
                let scrutinee = self.emit_value(func, scrutinee).into_int_value();
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
                let ret_ty = self.ty_ctx.def(def).ret_ty;
                let value = self.emit_value(func, value);
                match self.ty_ctx.storage_class(ret_ty) {
                    StorageClass::Zst => {
                        self.builder.build_return(None).unwrap();
                    }
                    StorageClass::Indirect => {
                        let dst = self
                            .curr_function()
                            .get_first_param()
                            .unwrap()
                            .into_pointer_value();
                        self.build_store(ret_ty, value, dst);
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
            Value::Share(place) => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                self.build_share(ty, place)
            }
            Value::Lit(lit) => self.emit_lit(lit),
            Value::Zst(_) => todo!(),
        }
    }

    fn emit_print(&mut self, func: &'mir Func, value: &Value) {
        let ty = self.value_ty(func, value);
        let format = match self.ty_ctx.get_ty(ty) {
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
            base = self.build_field(ty, base, access.idx);
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

    fn numeric_op<
        const N: usize,
        FRes: BasicValue<'ctx>,
        IRes: BasicValue<'ctx>,
        FOp: FnOnce([FloatValue<'ctx>; N]) -> Result<FRes, BuilderError>,
        IOp: FnOnce([IntValue<'ctx>; N], bool) -> Result<IRes, BuilderError>,
    >(
        &self,
        ty: TyId,
        values: [BasicValueEnum<'ctx>; N],
        float_op: FOp,
        int_op: IOp,
    ) -> BasicValueEnum<'ctx> {
        if ty == self.ty_ctx.float() {
            float_op(values.map(|v| v.into_float_value()))
                .unwrap()
                .as_basic_value_enum()
        } else {
            int_op(values.map(|v| v.into_int_value()), ty == self.ty_ctx.int())
                .unwrap()
                .as_basic_value_enum()
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
            InfixOp::Add => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| self.builder.build_float_add(lhs, rhs, ""),
                |[lhs, rhs], _| self.builder.build_int_add(lhs, rhs, ""),
            ),
            InfixOp::Sub => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| self.builder.build_float_sub(lhs, rhs, ""),
                |[lhs, rhs], _| self.builder.build_int_sub(lhs, rhs, ""),
            ),
            InfixOp::Mul => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| self.builder.build_float_mul(lhs, rhs, ""),
                |[lhs, rhs], _| self.builder.build_int_mul(lhs, rhs, ""),
            ),
            InfixOp::Div => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| self.builder.build_float_div(lhs, rhs, ""),
                |[lhs, rhs], signed| {
                    if signed {
                        self.builder.build_int_signed_div(lhs, rhs, "")
                    } else {
                        self.builder.build_int_unsigned_div(lhs, rhs, "")
                    }
                },
            ),
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
            InfixOp::Gt => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| {
                    self.builder
                        .build_float_compare(FloatPredicate::OGT, lhs, rhs, "")
                },
                |[lhs, rhs], signed| {
                    use IntPredicate as IP;
                    let op = if signed { IP::SGT } else { IP::UGT };
                    self.builder.build_int_compare(op, lhs, rhs, "")
                },
            ),
            InfixOp::Lt => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| {
                    self.builder
                        .build_float_compare(FloatPredicate::OLT, lhs, rhs, "")
                },
                |[lhs, rhs], signed| {
                    use IntPredicate as IP;
                    let op = if signed { IP::SLT } else { IP::ULT };
                    self.builder.build_int_compare(op, lhs, rhs, "")
                },
            ),
            InfixOp::Geq => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| {
                    self.builder
                        .build_float_compare(FloatPredicate::OGE, lhs, rhs, "")
                },
                |[lhs, rhs], signed| {
                    use IntPredicate as IP;
                    let op = if signed { IP::SGE } else { IP::UGE };
                    self.builder.build_int_compare(op, lhs, rhs, "")
                },
            ),
            InfixOp::Leq => self.numeric_op(
                ty,
                [lhs, rhs],
                |[lhs, rhs]| {
                    self.builder
                        .build_float_compare(FloatPredicate::OLE, lhs, rhs, "")
                },
                |[lhs, rhs], signed| {
                    use IntPredicate as IP;
                    let op = if signed { IP::SLE } else { IP::ULE };
                    self.builder.build_int_compare(op, lhs, rhs, "")
                },
            ),
        }
    }

    fn emit_prefix(&mut self, func: &'mir Func, op: PrefixOp, arg: &Value) -> BasicValueEnum<'ctx> {
        let ty = self.value_ty(func, arg);
        let arg = self.emit_value(func, arg);
        match op {
            PrefixOp::Not => self
                .builder
                .build_not(arg.into_int_value(), "")
                .unwrap()
                .as_basic_value_enum(),
            PrefixOp::Neg => self.numeric_op(
                ty,
                [arg],
                |[arg]| self.builder.build_float_neg(arg, ""),
                |[arg], _| self.builder.build_int_neg(arg, ""),
            ),
        }
    }

    fn emit_call(&mut self, func: &'mir Func, callee: &Callee, args: &[Arg], dst: &Place) {
        let ret_ty = self.place_ty(func, dst);
        let dst = self.emit_place(func, dst);

        let mut args: Vec<_> = args
            .iter()
            .map(|arg| match arg {
                Arg::Immut(value) => self.emit_value(func, value).into(),
                Arg::Mut(place) => self.emit_place(func, place).into(),
            })
            .collect();

        match self.ty_ctx.storage_class(ret_ty) {
            StorageClass::Zst => {
                self.emit_call_inner(func, callee, args);
            }
            StorageClass::Indirect => {
                args.insert(0, dst.into());
                self.emit_call_inner(func, callee, args);
            }
            StorageClass::Scalar => {
                let result = self.emit_call_inner(func, callee, args).unwrap();
                self.build_store(ret_ty, result, dst);
            }
        }
    }

    fn emit_call_inner(
        &mut self,
        func: &'mir Func,
        callee: &Callee,
        mut args: Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Option<BasicValueEnum<'ctx>> {
        match callee {
            Callee::Direct(def) => self.build_call(self.funcs[*def], &args),
            Callee::Closure(closure) => {
                let ty = self.closure_ty();
                let Ty::Func(params, ret_ty) = self.ty_ctx.get_ty(self.value_ty(func, closure))
                else {
                    unreachable!("ICE: Incorrect type for function callee");
                };
                let closure = self.emit_value(func, closure).into_pointer_value();

                // Extract environment from closure and add it to arguments.
                let env = self.builder.build_struct_gep(ty, closure, 1, "").unwrap();
                let env = self.builder.build_load(self.ptr_ty(), env, "").unwrap();
                args.push(env.as_basic_value_enum().into());

                // Extract function pointer from closure and call it.
                let func = self.builder.build_struct_gep(ty, closure, 0, "").unwrap();
                let func = self.builder.build_load(self.ptr_ty(), func, "").unwrap();
                self.build_indirect_call(
                    self.func_ty(params, *ret_ty, true),
                    func.into_pointer_value(),
                    &args,
                )
            }
        }
    }

    fn emit_closure(&mut self, func: &'mir Func, body: DefId, env: &Value) -> BasicValueEnum<'ctx> {
        // Create the environment, if one is needed
        let (env_ptr, env_ty) = if let Value::Zst(_) = env {
            (self.const_null(), None)
        } else {
            let env_ty = self.value_ty(func, env);
            let env_ptr = self.build_malloc(env_ty);
            let env = self.emit_value(func, env);
            self.build_store(env_ty, env, env_ptr);
            (env_ptr, Some(env_ty))
        };

        // Set up the closure.
        let name = self.ty_ctx.def(body).ident.str();
        let body = self.funcs[body];
        let ty = self.closure_ty();
        let closure = self.build_alloca(ty, "");

        // Create the witness functions.
        let drop = self
            .closure_drop(&name, env_ty)
            .as_global_value()
            .as_pointer_value();
        let copy = self
            .closure_copy(&name, env_ty)
            .as_global_value()
            .as_pointer_value();

        // Store everything into the closure, emitting the witness functions along the way.
        let store = |this: &Self, idx, val| {
            let ptr = this.builder.build_struct_gep(ty, closure, idx, "").unwrap();
            this.builder.build_store(ptr, val).unwrap();
        };
        store(self, 0, body.as_global_value().as_pointer_value());
        store(self, 1, env_ptr);
        store(self, 2, drop);
        store(self, 3, copy);

        closure.as_basic_value_enum()
    }
}
