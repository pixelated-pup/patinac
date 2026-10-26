use inkwell::{
    FloatPredicate,
    values::{BasicMetadataValueEnum, BasicValue as _, BasicValueEnum, PointerValue},
};

use irs::mir::{Arg, BlockExpr, Expr, ExprId, InfixOp, LitValue, PrefixOp, Stmt, Ty, VarId};

use crate::{
    CodegenState,
    layout::{self, IntSize, LayoutValue, ScalarKind, ScalarLayout, StorageClass},
};

impl<'mir, 'ctx> CodegenState<'mir, 'ctx> {
    pub(crate) fn emit_expr(&mut self, expr: ExprId) -> LayoutValue<'mir, 'ctx> {
        match self.mir.expr(expr) {
            Expr::Var(id) => self.emit_ident(*id),
            Expr::Lit(lit) => self.emit_lit(lit),
            Expr::Construct(field_tys, values) => self.emit_construct(field_tys, values),
            Expr::Infix { op, lhs, rhs } => self.emit_infix(*op, *lhs, *rhs),
            Expr::Prefix { op, expr } => self.emit_prefix(*op, *expr),
            Expr::Field { base, field } => self.emit_field(*base, *field),
            Expr::Call { func, args, ret_ty } => self.emit_call(*func, args, ret_ty),
            Expr::Closure { func, captures } => self.emit_closure(*func, captures),
            Expr::Assign { place, value } => self.emit_assign(*place, *value),
            Expr::If { ty, cond, th, el } => self.emit_if(ty, *cond, th, el.as_ref()),
            Expr::Loop(body) => self.emit_loop(body),
            Expr::Block(stmts) => self.emit_block_expr(stmts),

            Expr::Print(ty, expr) => self.emit_print(ty, *expr),
        }
    }

    fn emit_print(&mut self, ty: &'mir Ty, expr: ExprId) -> LayoutValue<'mir, 'ctx> {
        let format = match ty {
            Ty::Int => "%lld\n",
            Ty::UInt => "%llu\n",
            Ty::Byte => "%hhu\n",
            Ty::Float => "%f\n",
            Ty::Bool => "%hhd\n",
            Ty::Fields(_) => todo!(),
            Ty::Func(_, _) => panic!("can't print this type"),
        };
        let format = self
            .builder
            .build_global_string_ptr(format, "format_string")
            .unwrap()
            .as_pointer_value();

        let expr = self.emit_expr(expr);
        self.build_c_call(self.printf(), &[format.into(), expr.as_scalar().into()]);

        LayoutValue::Zst
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

    fn emit_ident(&self, id: VarId) -> LayoutValue<'mir, 'ctx> {
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

    fn emit_construct(
        &mut self,
        field_tys: &'mir [Ty],
        values: &[ExprId],
    ) -> LayoutValue<'mir, 'ctx> {
        // Unit.
        if field_tys.is_empty() {
            return LayoutValue::Zst;
        }

        let lowered_ty = self.fields_ty(field_tys);
        let out = self.build_alloca_entry(lowered_ty, "");
        for (idx, value) in values.iter().enumerate() {
            let value = self.emit_expr(*value);
            let ptr = self
                .builder
                .build_struct_gep(lowered_ty, out, u32::try_from(idx).unwrap(), "")
                .unwrap();
            self.build_move(value, ptr);
        }
        LayoutValue::Fields(field_tys, out)
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

    fn emit_field(&mut self, base: ExprId, field: u32) -> LayoutValue<'mir, 'ctx> {
        let (base, needs_drop) = if self.is_place(base) {
            (self.emit_place(base), false)
        } else {
            (self.emit_expr(base), true)
        };

        let (fields, ptr) = base.as_fields();
        let field_ptr = self
            .builder
            .build_struct_gep(self.fields_ty(fields), ptr, field, "")
            .unwrap();

        let result = self
            .build_dup(self.layout_indirect(&fields[usize::try_from(field).unwrap()], field_ptr));
        if needs_drop {
            self.build_drop(base);
        }
        result
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

    fn emit_closure(&self, func: VarId, captures: &[VarId]) -> LayoutValue<'mir, 'ctx> {
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

    fn emit_if(
        &mut self,
        ty: &'mir Ty,
        cond: ExprId,
        th: &BlockExpr,
        el: Option<&BlockExpr>,
    ) -> LayoutValue<'mir, 'ctx> {
        match el {
            Some(el) => self.emit_if_else(ty, cond, th, el),
            None => self.emit_if_no_else(cond, th),
        }
    }

    fn emit_if_else(
        &mut self,
        ty: &'mir Ty,
        cond: ExprId,
        th: &BlockExpr,
        el: &BlockExpr,
    ) -> LayoutValue<'mir, 'ctx> {
        let function = self.curr_function();

        // Set up blocks and result value alloc.
        let result = match layout::storage_class(ty) {
            StorageClass::Zst => None,
            StorageClass::Indirect => Some(self.build_alloca_entry(self.ptr_ty(), "if_result")),
            StorageClass::Scalar => Some(self.build_alloca_entry(self.lower_ty(ty), "if_result")),
        };
        let th_block = self.ctx.append_basic_block(function, "then");
        let el_block = self.ctx.append_basic_block(function, "else");
        let merge_block = self.ctx.append_basic_block(function, "merge");

        // Branch.
        let cond = self.emit_expr(cond);
        self.builder
            .build_conditional_branch(cond.as_int(), th_block, el_block)
            .unwrap();

        // Then block.
        {
            self.builder.position_at_end(th_block);
            let th = self.emit_block_expr(th);
            if let Some(result) = result {
                self.builder.build_store(result, th.as_value()).unwrap();
            }
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();
        }

        // Else block.
        {
            // Reposition after any sub-blocks of the then block.
            el_block
                .move_after(function.get_last_basic_block().unwrap())
                .unwrap();
            self.builder.position_at_end(el_block);
            let el = self.emit_block_expr(el);
            if let Some(result) = result {
                self.builder.build_store(result, el.as_value()).unwrap();
            }
            self.builder
                .build_unconditional_branch(merge_block)
                .unwrap();
        }

        // Extract result value.
        {
            // Reposition after any sub-blocks of the else block.
            merge_block
                .move_after(function.get_last_basic_block().unwrap())
                .unwrap();
            self.builder.position_at_end(merge_block);
            result.map_or(LayoutValue::Zst, |result| self.layout_direct(ty, result))
        }
    }

    fn emit_if_no_else(&mut self, cond: ExprId, th: &BlockExpr) -> LayoutValue<'mir, 'ctx> {
        let func = self.curr_function();

        // Append blocks to current function.
        let th_block = self.ctx.append_basic_block(func, "then");
        let merge_block = self.ctx.append_basic_block(func, "merge");

        // Branch on the condition.
        let cond = self.emit_expr(cond);
        self.builder
            .build_conditional_branch(cond.as_int(), th_block, merge_block)
            .unwrap();

        // Emit the then block.
        self.builder.position_at_end(th_block);
        let _ = self.emit_block_expr(th);
        self.builder
            .build_unconditional_branch(merge_block)
            .unwrap();

        // Reposition the merge block after any sub-blocks of the then block.
        merge_block
            .move_after(func.get_last_basic_block().unwrap())
            .unwrap();
        self.builder.position_at_end(merge_block);

        LayoutValue::Zst
    }

    fn emit_loop(&mut self, body: &BlockExpr) -> LayoutValue<'mir, 'ctx> {
        let function = self.curr_function();

        let body_block = self.ctx.append_basic_block(function, "body");
        self.builder.build_unconditional_branch(body_block).unwrap();

        self.builder.position_at_end(body_block);
        let _ = self.emit_block_expr(body);
        self.builder.build_unconditional_branch(body_block).unwrap();

        let post_block = self.ctx.append_basic_block(function, "post");
        self.builder.position_at_end(post_block);

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
