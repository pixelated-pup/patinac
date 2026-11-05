use inkwell::{
    FloatPredicate,
    basic_block::BasicBlock,
    values::{BasicMetadataValueEnum, BasicValue as _, BasicValueEnum, PointerValue},
};

use irs::{
    mir::{
        Arg, Block, BlockId, Expr, Func, InfixOp, LitValue, LocalId, PrefixOp, Stmt, Terminator,
        Value,
    },
    ty::{Ty, TyId},
};

use crate::{
    CodegenState,
    layout::{self, IntSize, LayoutValue, ScalarKind, ScalarLayout, StorageClass},
};

impl<'mir, 'ty_ctx, 'ctx> CodegenState<'mir, 'ty_ctx, 'ctx> {
    pub(crate) fn emit_block(
        &mut self,
        mir_func: &'mir Func,
        mir_block: BlockId,
    ) -> BasicBlock<'ctx> {
        let mir_block = mir_func.block(mir_block);
        let llvm_block = self.ctx.append_basic_block(self.curr_function(), "");
        self.builder.position_at_end(llvm_block);

        for stmt in &mir_block.stmts {
            self.emit_stmt(stmt);
        }

        match mir_block.terminator() {
            Terminator::Goto(next_block) => {
                let next_block = self.emit_block(mir_func, *next_block);
                self.builder.position_at_end(llvm_block);
                self.builder.build_unconditional_branch(next_block).unwrap();
            }
            Terminator::Switch {
                scrutinee,
                targets,
                otherwise,
            } => {
                let scrutinee = self.emit_value(scrutinee).as_int();

                let targets: Vec<_> = targets
                    .iter()
                    .map(|(value, block)| {
                        let value = self.ctx.i64_type().const_int(*value, false);
                        let block = self.emit_block(mir_func, *block);
                        (value, block)
                    })
                    .collect();
                let otherwise = self.emit_block(mir_func, *otherwise);

                self.builder.position_at_end(llvm_block);
                self.builder
                    .build_switch(scrutinee, otherwise, &targets)
                    .unwrap();
            }
            Terminator::Return(value) => {
                // match layout::storage_class(ret_ty) {
                //     StorageClass::Zst => self.builder.build_return(None).unwrap(),
                //     StorageClass::Indirect => {
                //         let out_ptr = func.get_first_param().unwrap().into_pointer_value();
                //         self.build_move(body, out_ptr);
                //         self.builder.build_return(None).unwrap()
                //     }
                //     StorageClass::Scalar => {
                //         self.builder.build_return(Some(&body.as_scalar())).unwrap()
                //     }
                // };
                todo!();
            }
            Terminator::Unreachable => {
                self.builder.build_unreachable().unwrap();
            }
        }

        llvm_block
    }

    pub(crate) fn emit_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Assign { place, expr } => todo!(),
            Stmt::Call { callee, args, dst } => todo!(),
            Stmt::Drop(place) => todo!(),
            Stmt::Print(value) => self.emit_print(value),
        }
    }

    fn emit_expr(&mut self, expr: &Expr) -> LayoutValue<'mir, 'ctx> {
        match expr {
            Expr::Val(value) => self.emit_value(value),
            Expr::Infix(infix_op, value, value1) => todo!(),
            Expr::Prefix(prefix_op, value) => todo!(),
            Expr::Closure { func, captures } => todo!(),
        }
    }

    fn emit_value(&mut self, value: &Value) -> LayoutValue<'mir, 'ctx> {
        match value {
            Value::Copy(place) => todo!(),
            Value::Move(place) => todo!(),
            Value::Lit(lit) => self.emit_lit(lit),
            Value::Item(item_id) => todo!(),
            Value::Zst(_) => LayoutValue::Zst,
        }
    }

    // pub(crate) fn emit_expr(&mut self, expr: ExprId) -> LayoutValue<'mir, 'ctx> {
    //     match self.mir.expr(expr) {
    //         Expr::Var(id) => self.emit_ident(*id),
    //         Expr::Lit(lit) => self.emit_lit(lit),
    //         Expr::Construct(field_tys, values) => self.emit_construct(field_tys, values),
    //         Expr::Infix(op, lhs, rhs) => self.emit_infix(*op, *lhs, *rhs),
    //         Expr::Prefix(op, expr) => self.emit_prefix(*op, *expr),
    //         Expr::Field { base, field } => self.emit_field(*base, *field),
    //         Expr::Call { func, args, ret_ty } => self.emit_call(*func, args, ret_ty),
    //         Expr::Closure { func, captures } => self.emit_closure(*func, captures),
    //         Expr::Assign { place, value } => self.emit_assign(*place, *value),
    //         Expr::If { ty, cond, th, el } => self.emit_if(ty, *cond, th, el.as_ref()),
    //         Expr::Loop(body) => self.emit_loop(body),
    //         Expr::Block(stmts) => self.emit_block_expr(stmts),

    //         Expr::Print(ty, expr) => self.emit_print(ty, *expr),
    //     }
    // }

    fn emit_print(&mut self, value: &Value) {
        let ty = match Value {};
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

        let value = self.emit_value(value);
        self.build_c_call(self.printf(), &[format.into(), value.as_scalar().into()]);
    }

    fn is_place(&self, expr: ExprId) -> bool {
        match self.mir.expr(expr) {
            Expr::Var(id) => match self.vars[*id] {
                LayoutValue::Scalar(_, ScalarLayout::Direct(_)) | LayoutValue::Zst => false,
                LayoutValue::Scalar(_, ScalarLayout::Indirect(_))
                | LayoutValue::Closure(_, _)
                | LayoutValue::Fields(_, _) => true,
            },
            Expr::Field { base, .. } => self.is_place(*base),
            _ => false,
        }
    }

    fn emit_place(&mut self, expr: ExprId) -> LayoutValue<'mir, 'ctx> {
        match self.mir.expr(expr) {
            Expr::Var(id) => self.vars[*id],
            Expr::Field { base, field } => {
                let (fields, base) = self.emit_place(*base).as_fields();
                let field_ptr = self
                    .builder
                    .build_struct_gep(self.fields_ty(fields), base, *field, "")
                    .unwrap();
                self.layout_indirect(&fields[usize::try_from(*field).unwrap()], field_ptr)
            }
            _ => unreachable!("not a place"),
        }
    }

    fn emit_unique_place(&mut self, expr: ExprId) -> LayoutValue<'mir, 'ctx> {
        match self.mir.expr(expr) {
            Expr::Var(id) => self.vars[*id],
            Expr::Field { base, field } => {
                let (fields, base) = self.emit_unique_place(*base).as_fields();
                let field_ptr = self
                    .builder
                    .build_struct_gep(self.fields_ty(fields), base, *field, "")
                    .unwrap();
                self.layout_indirect(&fields[usize::try_from(*field).unwrap()], field_ptr)
            }
            _ => unreachable!("not a place"),
        }
    }

    fn emit_ident(&self, id: LocalId) -> LayoutValue<'mir, 'ctx> {
        self.build_dup(self.vars[id])
    }

    fn emit_lit(&self, lit: &LitValue) -> LayoutValue<'mir, 'ctx> {
        match lit {
            LitValue::Int(value) => LayoutValue::int(IntSize::Bits64, self.const_int(*value)),
            LitValue::UInt(value) => LayoutValue::int(IntSize::Bits64, self.const_uint(*value)),
            LitValue::Byte(value) => LayoutValue::int(IntSize::Bits8, self.const_byte(*value)),
            LitValue::Float(value) => LayoutValue::float(self.const_float(*value)),
            LitValue::Bool(value) => LayoutValue::int(IntSize::Bits8, self.const_bool(*value)),
        }
    }

    fn emit_infix(&mut self, op: InfixOp, lhs: ExprId, rhs: ExprId) -> LayoutValue<'mir, 'ctx> {
        let lhs = self.emit_expr(lhs);
        let rhs = self.emit_expr(rhs);
        match op {
            InfixOp::Add => LayoutValue::int_op(lhs, rhs, |l, r| {
                self.builder.build_int_add(l, r, "").unwrap()
            }),
            InfixOp::AddF => LayoutValue::float(
                self.builder
                    .build_float_add(lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
            InfixOp::Sub => LayoutValue::int_op(lhs, rhs, |l, r| {
                self.builder.build_int_sub(l, r, "").unwrap()
            }),
            InfixOp::SubF => LayoutValue::float(
                self.builder
                    .build_float_sub(lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
            InfixOp::Mul => LayoutValue::int_op(lhs, rhs, |l, r| {
                self.builder.build_int_mul(l, r, "").unwrap()
            }),
            InfixOp::MulF => LayoutValue::float(
                self.builder
                    .build_float_mul(lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
            InfixOp::Div => todo!(),
            InfixOp::DivF => LayoutValue::float(
                self.builder
                    .build_float_div(lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
            InfixOp::Exp => todo!(),
            // FIXME: Short-circuiting
            InfixOp::And => {
                LayoutValue::int_op(lhs, rhs, |l, r| self.builder.build_and(l, r, "").unwrap())
            }
            InfixOp::Or => {
                LayoutValue::int_op(lhs, rhs, |l, r| self.builder.build_or(l, r, "").unwrap())
            }
            op @ (InfixOp::Eqq | InfixOp::Neq) => {
                let equals = self.build_equals(lhs, rhs);
                if op == InfixOp::Neq {
                    LayoutValue::int(IntSize::Bits8, self.builder.build_not(equals, "").unwrap())
                } else {
                    LayoutValue::int(IntSize::Bits8, equals)
                }
            }
            InfixOp::Gt => LayoutValue::int(
                IntSize::Bits8,
                self.builder
                    .build_float_compare(FloatPredicate::OGT, lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
            InfixOp::Lt => LayoutValue::int(
                IntSize::Bits8,
                self.builder
                    .build_float_compare(FloatPredicate::OLT, lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
            InfixOp::Geq => LayoutValue::int(
                IntSize::Bits8,
                self.builder
                    .build_float_compare(FloatPredicate::OGE, lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
            InfixOp::Leq => LayoutValue::int(
                IntSize::Bits8,
                self.builder
                    .build_float_compare(FloatPredicate::OLE, lhs.as_float(), rhs.as_float(), "")
                    .unwrap(),
            ),
        }
    }

    fn emit_prefix(&mut self, op: PrefixOp, expr: ExprId) -> LayoutValue<'mir, 'ctx> {
        let expr = self.emit_expr(expr);
        match op {
            PrefixOp::Not => LayoutValue::int(
                IntSize::Bits8,
                self.builder.build_not(expr.as_int(), "").unwrap(),
            ),
            PrefixOp::Neg => {
                LayoutValue::float(self.builder.build_float_neg(expr.as_float(), "").unwrap())
            }
        }
    }

    fn emit_call(
        &mut self,
        func: ExprId,
        args: &[Arg],
        ret_ty: &'mir Ty,
    ) -> LayoutValue<'mir, 'ctx> {
        let mut tmps = Vec::new();
        let mut args: Vec<_> = args
            .iter()
            .filter_map(|arg| {
                if layout::zst(&arg.ty) {
                    // Erase ZSTs.
                    None
                } else if arg.mutable {
                    // Mutable arguments.
                    Some(self.emit_unique_place(arg.value).as_pointer().into())
                } else if layout::indirect(&arg.ty) && self.is_place(arg.value) {
                    // Immutable aliasing optimisation.
                    Some(self.emit_place(arg.value).as_pointer().into())
                } else {
                    let tmp = self.emit_expr(arg.value);
                    tmps.push(tmp);
                    Some(tmp.as_value().into())
                }
            })
            .collect();

        let result = match layout::storage_class(ret_ty) {
            StorageClass::Zst => {
                self.emit_call_inner(func, args);
                LayoutValue::Zst
            }
            StorageClass::Indirect => {
                let ret_ptr = self.build_alloca_entry(self.lower_ty(ret_ty), "out");
                args.insert(0, ret_ptr.into());
                self.emit_call_inner(func, args);
                self.layout_direct(ret_ty, ret_ptr)
            }
            StorageClass::Scalar => {
                let result = self.emit_call_inner(func, args).unwrap();
                let kind = match ret_ty {
                    Ty::Int | Ty::UInt => ScalarKind::Int(IntSize::Bits64),
                    Ty::Byte | Ty::Bool => ScalarKind::Int(IntSize::Bits8),
                    Ty::Float => ScalarKind::Float,
                    _ => unreachable!("not a scalar"),
                };
                LayoutValue::Scalar(kind, ScalarLayout::Direct(result))
            }
        };

        for tmp in tmps {
            self.build_drop(tmp);
        }

        result
    }

    fn emit_call_inner(
        &mut self,
        func: ExprId,
        mut args: Vec<BasicMetadataValueEnum<'ctx>>,
    ) -> Option<BasicValueEnum<'ctx>> {
        if let Expr::Var(id) = self.mir.expr(func)
            && let Some(func) = self.funcs.get(*id)
        {
            return self.build_call(*func, &args);
        }
        match self.emit_expr(func) {
            LayoutValue::Scalar(ScalarKind::FuncPtr(func_ty), ScalarLayout::Direct(func)) => {
                self.build_indirect_call(func_ty, func.into_pointer_value(), &args)
            }
            LayoutValue::Scalar(ScalarKind::FuncPtr(func_ty), ScalarLayout::Indirect(ptr)) => {
                let func = self.builder.build_load(self.ptr_ty(), ptr, "").unwrap();
                self.build_indirect_call(func_ty, func.into_pointer_value(), &args)
            }
            LayoutValue::Closure(func_ty, closure) => {
                let ty = self.closure_ty();

                // Extract environment from closure and add it to arguments.
                let env = self.builder.build_struct_gep(ty, closure, 1, "").unwrap();
                let env = self.builder.build_load(self.ptr_ty(), env, "").unwrap();
                args.push(env.as_basic_value_enum().into());

                // Extract function pointer from closure and call it.
                let func = self.builder.build_struct_gep(ty, closure, 0, "").unwrap();
                let func = self.builder.build_load(self.ptr_ty(), func, "").unwrap();
                self.build_indirect_call(func_ty, func.into_pointer_value(), &args)
            }
            _ => unreachable!("wrong type for function"),
        }
    }

    fn emit_closure(&self, func: LocalId, captures: &[LocalId]) -> LayoutValue<'mir, 'ctx> {
        // Create the environment, if one is needed
        let (env, env_ty) = if captures.is_empty() {
            (self.const_null(), None)
        } else {
            // Allocate the environment.
            let capture_tys: Vec<_> = captures
                .iter()
                .map(|id| self.lower_ty(&self.mir.var(*id).ty))
                .collect();
            let env_ty = self.ctx.struct_type(&capture_tys, false);
            let env = self
                .build_call(
                    self.malloc(),
                    &[env_ty.size_of().unwrap().as_basic_value_enum().into()],
                )
                .unwrap()
                .into_pointer_value();

            // Initialize the environment
            for (index, capture) in captures.iter().enumerate() {
                let dst = self
                    .builder
                    .build_struct_gep(env_ty, env, u32::try_from(index).unwrap(), "")
                    .unwrap();
                self.build_copy(self.vars[*capture], dst);
            }

            (env, Some(env_ty))
        };

        // Create the final closure
        let name = self.mir.var(func).ident.str();
        let func = self.funcs[func];
        let ty = self.closure_ty();
        let closure = self.build_alloca_entry(ty, "");

        // Store everything into the closure, emitting the witness functions along the way.
        let store = |idx, val: PointerValue<'ctx>| {
            let ptr = self.builder.build_struct_gep(ty, closure, idx, "").unwrap();
            self.builder.build_store(ptr, val).unwrap();
        };
        store(0, func.as_global_value().as_pointer_value());
        store(1, env);
        store(
            2,
            self.closure_drop(&name, captures, env_ty)
                .as_global_value()
                .as_pointer_value(),
        );
        store(
            3,
            self.closure_copy(&name, captures, env_ty)
                .as_global_value()
                .as_pointer_value(),
        );
        store(
            4,
            self.closure_equals(&name, captures, env_ty)
                .as_global_value()
                .as_pointer_value(),
        );

        LayoutValue::Closure(func.get_type(), closure)
    }

    fn emit_assign(&mut self, place: ExprId, value: ExprId) -> LayoutValue<'mir, 'ctx> {
        let place = self.emit_unique_place(place);
        let value = self.emit_expr(value);

        // Don't have to do anything further if it's a ZST.
        if place == LayoutValue::Zst {
            return LayoutValue::Zst;
        }

        // Drop the current value in the assigned-to variable
        self.build_drop(place);
        // Move the temporary value into the variable
        self.build_move(value, place.as_pointer());
        LayoutValue::Zst
    }

    fn emit_block_expr(&mut self, block: &BlockExpr) -> LayoutValue<'mir, 'ctx> {
        let mut locals = Vec::new();
        let mut last_expr = None;

        for stmt in &block.0 {
            // Drop the previous expression, if there was one.
            if let Some(expr) = last_expr.take() {
                self.build_drop(expr);
            }
            match stmt {
                Stmt::Decl { var, value, .. } => {
                    let ty = &self.mir.var(*var).ty;
                    let tmp = self.emit_expr(*value);

                    // ZSTs and non-mutable values can be referenced directly, without a pointer.
                    // Sized, mutable values must be behind pointers for SSA reasons.
                    // Indirect values are already behind pointers, so they don't need a new allocation.
                    let val = if layout::zst(ty)
                        || layout::indirect(ty)
                        || !self.mir.var(*var).mutable
                    {
                        tmp
                    } else {
                        let alloc = self
                            .build_alloca_entry(self.lower_ty(ty), &self.mir.var(*var).ident.str());
                        self.build_move(tmp, alloc);
                        self.layout_indirect(ty, alloc)
                    };

                    self.vars.insert(*var, val);
                    locals.push(*var);
                }
                Stmt::Expr(expr) => {
                    last_expr = Some(self.emit_expr(*expr));
                }
            }
        }

        // Drop all local variables.
        for var in locals {
            let var = self.vars.remove(var).expect("variable was just added");
            self.build_drop(var);
        }

        last_expr.unwrap_or(LayoutValue::Zst)
    }
}
