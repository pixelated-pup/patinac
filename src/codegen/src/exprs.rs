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
    ty::{DefId, Ty, TyId, layout::StorageClass},
};
use slotmap::SecondaryMap;

use crate::CodegenState;

impl<'ext, 'ctx> CodegenState<'ext, 'ctx> {
    fn place_ty(&self, func: &'ext Func, place: &Place) -> TyId {
        match place.accesses.last() {
            Some(access) => access.ty,
            None => func.local(place.base).ty,
        }
    }

    fn value_ty(&self, func: &'ext Func, value: &Value) -> TyId {
        match value {
            Value::Read(place) => self.place_ty(func, place),
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

    pub fn emit_block_stmts(&mut self, func: &'ext Func, mir_block: BlockId) -> BasicBlock<'ctx> {
        let llvm_block = self.ctx.append_basic_block(self.curr_function(), "");
        self.builder.position_at_end(llvm_block);

        for stmt in &func.block(mir_block).stmts {
            self.emit_stmt(func, stmt);
        }

        llvm_block
    }

    pub fn emit_block_term(
        &mut self,
        blocks: &SecondaryMap<BlockId, BasicBlock<'ctx>>,
        def: DefId,
        func: &'ext Func,
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
            Terminator::Return => {
                let ret_ty = self.ty_ctx.def(def).ret_ty;
                match self.ty_ctx.storage_class(ret_ty) {
                    StorageClass::Zst | StorageClass::Indirect => {
                        self.builder.build_return(None).unwrap();
                    }
                    StorageClass::Scalar => {
                        let ret_place = Place::local(func.ret_place());
                        let value = self.emit_value(func, &Value::Read(ret_place));
                        self.builder.build_return(Some(&value)).unwrap();
                    }
                }
            }
            Terminator::Unreachable => {
                self.builder.build_unreachable().unwrap();
            }
        }
    }

    pub(crate) fn emit_stmt(&mut self, func: &'ext Func, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { place, expr } => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                self.emit_expr(func, ty, place, expr);
            }
            Stmt::Drop(place) => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                self.build_drop(ty, place);
            }
            Stmt::Print(value) => self.emit_print(func, value),
        }
    }

    fn emit_expr(&mut self, func: &'ext Func, ty: TyId, dst: PointerValue<'ctx>, expr: &Expr) {
        match expr {
            Expr::Val(value) => {
                let value = self.emit_value(func, value);
                self.build_store(ty, dst, value);
            }
            Expr::Copy(place) => {
                let place = self.emit_place(func, place);
                self.build_copy(ty, place, dst);
            }
            Expr::Infix(op, lhs, rhs) => {
                let value = self.emit_infix(func, *op, lhs, rhs);
                self.build_store(ty, dst, value);
            }
            Expr::Prefix(op, arg) => {
                let value = self.emit_prefix(func, *op, arg);
                self.build_store(ty, dst, value);
            }
            Expr::Call { callee, args } => self.emit_call(func, dst, callee, args, ty),
            Expr::Closure { body, env } => self.emit_closure(func, dst, *body, env),
        }
    }

    fn emit_value(&mut self, func: &'ext Func, value: &Value) -> BasicValueEnum<'ctx> {
        match value {
            Value::Read(place) => {
                let ty = self.place_ty(func, place);
                let place = self.emit_place(func, place);
                self.build_read(ty, place)
            }
            Value::Lit(lit) => self.emit_lit(lit),
            Value::Zst(_) => self
                .ctx
                .struct_type(&[], false)
                .get_poison()
                .as_basic_value_enum(),
        }
    }

    fn emit_print(&mut self, func: &'ext Func, value: &Value) {
        let ty = self.value_ty(func, value);
        let format = match self.ty_ctx.get_ty(ty) {
            Ty::Int => "%lld\n",
            Ty::UInt => "%llu\n",
            Ty::Byte => "%hhu\n",
            Ty::Float => "%f\n",
            Ty::Bool => "%hhd\n",
            _ => todo!("Proper printing"),
        };
        let format = self
            .builder
            .build_global_string_ptr(format, "")
            .unwrap()
            .as_pointer_value();

        let value = self.emit_value(func, value);
        self.build_c_call(self.printf(), &[format.into(), value.into()]);
    }

    fn emit_place(&mut self, func: &'ext Func, place: &Place) -> PointerValue<'ctx> {
        let ty = self.place_ty(func, place);
        if self.ty_ctx.is_zst(ty) {
            return self.dangling(ty);
        }

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
        func: &'ext Func,
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

    fn emit_prefix(&mut self, func: &'ext Func, op: PrefixOp, arg: &Value) -> BasicValueEnum<'ctx> {
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

    fn emit_call(
        &mut self,
        func: &'ext Func,
        dst: PointerValue<'ctx>,
        callee: &Callee,
        args: &[Arg],
        ret_ty: TyId,
    ) {
        let mut args: Vec<BasicMetadataValueEnum<'_>> = args
            .iter()
            .filter_map(|arg| match arg {
                Arg::Immut(value) => {
                    if self.ty_ctx.is_zst(self.value_ty(func, value)) {
                        None
                    } else {
                        Some(self.emit_value(func, value).into())
                    }
                }
                Arg::Mut(place) => {
                    if self.ty_ctx.is_zst(self.place_ty(func, place)) {
                        None
                    } else {
                        Some(self.emit_place(func, place).into())
                    }
                }
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
                self.build_store(ret_ty, dst, result);
            }
        }
    }

    fn emit_call_inner(
        &mut self,
        func: &'ext Func,
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
                let func_ty = self.func_ty(params, *ret_ty, true);
                self.build_indirect_call(func_ty, func.into_pointer_value(), &args)
            }
        }
    }

    fn emit_closure(
        &mut self,
        func: &'ext Func,
        dst: PointerValue<'ctx>,
        body: DefId,
        env: &Value,
    ) {
        // Allocate the environment.
        let env_ty = self.value_ty(func, env);
        let env_ptr = self.build_malloc(env_ty);
        if !self.ty_ctx.is_zst(env_ty) {
            let env = self.emit_value(func, env);
            self.build_store(env_ty, env_ptr, env);
        }

        // Create the witness functions.
        let drop = self
            .closure_drop(body, env_ty)
            .as_global_value()
            .as_pointer_value();
        let copy = self
            .closure_copy(body, env_ty)
            .as_global_value()
            .as_pointer_value();

        // Build the closure in the destination
        let ty = self.closure_ty();
        let store = |idx, val| {
            let ptr = self.builder.build_struct_gep(ty, dst, idx, "").unwrap();
            self.builder.build_store(ptr, val).unwrap();
        };
        store(0, self.funcs[body].as_global_value().as_pointer_value());
        store(1, env_ptr);
        store(2, drop);
        store(3, copy);
    }
}
