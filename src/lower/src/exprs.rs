use std::range::Range;

use errors::Report;
use ident::Ident;
use irs::{
    hir::{self, ExprId, ExprKind, LitExpr, VarId},
    mir::{self, Callee, FieldAccess, Func, Local, LocalId, Place, Terminator, Value},
    ty::{DefInfo, Param, Parent, Ty, TyId},
};

use crate::LowerInfo;

impl LowerInfo<'_, '_, '_> {
    /// The resulting expression has an indefinite lifetime.
    pub fn lower_as_expr(&mut self, expr_id: ExprId) -> mir::Expr {
        match &self.hir.expr(expr_id).kind {
            ExprKind::Var(var) if let Some(def) = self.hir.var(*var).global_id => {
                mir::Expr::Closure {
                    body: def,
                    env: Value::Zst(self.ty_ctx.unit()),
                }
            }
            ExprKind::Infix(op, lhs, rhs)
                if !matches!(op, hir::InfixOp::And | hir::InfixOp::Or) =>
            {
                // TODO: Check for div-by-zero.
                mir::Expr::Infix(
                    convert_infix_op(*op).unwrap(),
                    self.lower_as_value(*lhs),
                    self.lower_as_value(*rhs),
                )
            }
            ExprKind::Prefix(op, arg) => {
                mir::Expr::Prefix(convert_prefix_op(*op), self.lower_as_value(*arg))
            }
            ExprKind::Call { callee, args } => {
                let callee = match &self.hir.expr(*callee).kind {
                    ExprKind::Var(var) if let Some(def) = self.hir.var(*var).global_id => {
                        Callee::Direct(def)
                    }
                    _ => Callee::Closure(self.lower_as_value(*callee)),
                };
                let args = args
                    .iter()
                    .map(|arg| {
                        if arg.mutable {
                            mir::Arg::Mut(self.lower_as_place(arg.value))
                        } else {
                            mir::Arg::Immut(self.lower_as_value(arg.value))
                        }
                    })
                    .collect();
                mir::Expr::Call { callee, args }
            }
            ExprKind::MethodCall { base, method, args } => todo!("Method Lowering"),
            ExprKind::Lambda {
                params,
                captures,
                body,
            } => self.lower_lambda(params, captures, *body),

            ExprKind::Var(_)
            | ExprKind::Lit(_)
            | ExprKind::Tuple(_)
            | ExprKind::Infix(_, _, _)
            | ExprKind::Field { .. }
            | ExprKind::Assign { .. }
            | ExprKind::If { .. }
            | ExprKind::For { .. }
            | ExprKind::Loop(_)
            | ExprKind::Break
            | ExprKind::Continue
            | ExprKind::Return(_)
            | ExprKind::Block(_)
            | ExprKind::Print(_) => mir::Expr::Copy(self.lower_as_place(expr_id)),
        }
    }

    /// The returned value may alias temporaries and local variables, so it must not be mutated and must not outlive the containing statement.
    ///
    /// Situations where a value outlives the containing statement include:
    /// - Returns
    /// - Assignments
    pub fn lower_as_value(&mut self, expr_id: ExprId) -> Value {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Lit(lit) => Value::Lit(self.lower_lit(lit, expr.span, expr.ty)),
            ExprKind::Assign { place, value } => {
                let place = self.lower_as_place(*place);
                assert!(self.func.local(place.base).mutable);
                self.add_stmt(mir::Stmt::Drop(place.clone()));
                self.lower_into_place(place, *value);
                self.unit()
            }
            ExprKind::For { .. } => todo!("For Loops"),
            ExprKind::Loop(body) => {
                let body_block = self.func.add_block();

                self.set_term(Terminator::Goto(body_block));

                self.block = body_block;
                self.lower_block_expr(body, None);
                self.set_term(Terminator::Goto(body_block));

                let merge_block = self.func.add_block();
                self.block = merge_block;

                self.unit()
            }
            ExprKind::Break => todo!("Unconditional Control Flow"),
            ExprKind::Continue => todo!("Unconditional Control Flow"),
            ExprKind::Return(_) => todo!("Unconditional Control Flow"),
            ExprKind::Print(expr) => {
                let value = self.lower_as_value(*expr);
                self.add_stmt(mir::Stmt::Print(value));
                self.unit()
            }

            ExprKind::Var(_)
            | ExprKind::Field { .. }
            | ExprKind::Tuple(_)
            | ExprKind::Infix(_, _, _)
            | ExprKind::Prefix(_, _)
            | ExprKind::Call { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Lambda { .. }
            | ExprKind::If { .. }
            | ExprKind::Block(_) => {
                let place = self.lower_as_place(expr_id);
                if self.ty_ctx.is_zst(expr.ty) {
                    Value::Zst(expr.ty)
                } else {
                    Value::Read(place)
                }
            }
        }
    }

    /// Handles dropping the returned place if necessary.
    pub fn lower_as_place(&mut self, expr_id: ExprId) -> Place {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Var(var) if self.hir.var(*var).global_id.is_none() => {
                Place::local(self.lower_local(*var))
            }
            ExprKind::Field { base, field } => {
                let Ty::Named { fields, .. } = self.ty_ctx.get_ty(self.hir.expr(*base).ty) else {
                    unreachable!("field access of non-record type")
                };
                let index = fields
                    .get_index_of(&field.ident)
                    .expect("ICE: Missing Field")
                    .try_into()
                    .expect("ICE: Too Many Fields");
                let mut place = self.lower_as_place(*base);
                place.accesses.push(FieldAccess {
                    idx: index,
                    ty: expr.ty,
                });
                place
            }
            ExprKind::Var(_)
            | ExprKind::Lit(_)
            | ExprKind::Tuple(_)
            | ExprKind::Infix(_, _, _)
            | ExprKind::Prefix(_, _)
            | ExprKind::Call { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Lambda { .. }
            | ExprKind::Assign { .. }
            | ExprKind::If { .. }
            | ExprKind::For { .. }
            | ExprKind::Loop(_)
            | ExprKind::Break
            | ExprKind::Continue
            | ExprKind::Return(_)
            | ExprKind::Block(_)
            | ExprKind::Print(_) => {
                let temp = self.func.add_temp(expr.ty);
                self.lower_into_place(Place::local(temp), expr_id);
                self.stage_drop(temp);
                Place::local(temp)
            }
        }
    }

    /// Lowers the provided expression, storing it's result in `dst`.
    ///
    /// Caller's responsibility to ensure that `place` gets dropped.
    ///
    /// This function is guaranteed to only assign to `place` once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    pub fn lower_into_place(&mut self, dst: Place, expr_id: ExprId) {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Tuple(elems) => {
                let mut place = dst;
                for (index, elem) in elems.iter().enumerate() {
                    let ty = self.hir.expr(*elem).ty;
                    place.accesses.push(FieldAccess {
                        idx: index.try_into().expect("tuple too large"),
                        ty,
                    });
                    self.lower_into_place(place.clone(), *elem);
                    place.accesses.pop();
                }
            }
            ExprKind::Infix(op, lhs, rhs) if matches!(op, hir::InfixOp::And | hir::InfixOp::Or) => {
                let lhs = self.lower_as_value(*lhs);

                let th_block = self.func.add_block();
                let el_block = self.func.add_block();
                let merge_block = self.func.add_block();

                self.set_term(Terminator::Switch {
                    scrutinee: lhs.clone(),
                    targets: vec![(1, th_block)],
                    otherwise: el_block,
                });

                let (done_block, cont_block) = match op {
                    hir::InfixOp::And => (el_block, th_block),
                    hir::InfixOp::Or => (th_block, el_block),
                    _ => unreachable!(),
                };

                self.block = done_block;
                self.add_stmt(mir::Stmt::Assign {
                    place: dst.clone(),
                    expr: mir::Expr::Val(lhs),
                });
                self.set_term(Terminator::Goto(merge_block));

                self.block = cont_block;
                self.lower_into_place(dst, *rhs);
                self.set_term(Terminator::Goto(merge_block));

                self.block = merge_block;
            }
            ExprKind::If { cond, th, el } => {
                let cond = self.lower_as_value(*cond);

                let th_block = self.func.add_block();
                let el_block = self.func.add_block();

                self.set_term(Terminator::Switch {
                    scrutinee: cond,
                    targets: vec![(1, th_block)],
                    otherwise: el_block,
                });

                match el {
                    Some(el) => {
                        let merge_block = self.func.add_block();

                        self.block = th_block;
                        self.lower_block_expr(th, Some(dst.clone()));
                        self.set_term(Terminator::Goto(merge_block));

                        self.block = el_block;
                        self.lower_block_expr(el, Some(dst));
                        self.set_term(Terminator::Goto(merge_block));

                        self.block = merge_block;
                    }
                    None => {
                        self.block = th_block;
                        self.lower_block_expr(th, None);
                        self.set_term(Terminator::Goto(el_block));
                        self.block = el_block;
                    }
                }
            }
            ExprKind::Block(block) => self.lower_block_expr(block, Some(dst)),

            ExprKind::Var(_)
            | ExprKind::Lit(_)
            | ExprKind::Infix(_, _, _)
            | ExprKind::Prefix(_, _)
            | ExprKind::Field { .. }
            | ExprKind::Call { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Lambda { .. }
            | ExprKind::Assign { .. }
            | ExprKind::For { .. }
            | ExprKind::Loop(_)
            | ExprKind::Break
            | ExprKind::Continue
            | ExprKind::Return(_)
            | ExprKind::Print(_) => {
                let expr = self.lower_as_expr(expr_id);
                self.add_stmt(mir::Stmt::Assign { place: dst, expr });
            }
        }
    }

    fn lower_lit(&self, lit: &LitExpr, span: Range<u32>, ty: TyId) -> mir::LitValue {
        let warn = |msg| {
            self.handler
                .report(Report::warning(msg), span, self.def().module);
        };
        match lit {
            &LitExpr::Int(value) => match self.ty_ctx.get_ty(ty) {
                Ty::Int => {
                    let i = i64::try_from(value).unwrap_or_else(|_| {
                        warn(format!(
                            "int literal {value} overflowed and was clamped to {}",
                            i64::MAX
                        ));
                        i64::MAX
                    });
                    mir::LitValue::Int(i)
                }
                Ty::UInt => mir::LitValue::UInt(value),
                Ty::Byte => {
                    let b = u8::try_from(value).unwrap_or_else(|_| {
                        warn(format!(
                            "byte literal {value} overflowed and was clamped to {}",
                            u8::MAX
                        ));
                        u8::MAX
                    });
                    mir::LitValue::Byte(b)
                }
                _ => unreachable!("not an int type"),
            },
            &LitExpr::Float(value) => mir::LitValue::Float(value),
            hir::LitExpr::String(_) => todo!("Strings"),
            &LitExpr::Bool(value) => mir::LitValue::Bool(value),
        }
    }

    fn lower_lambda(&mut self, params: &[VarId], captures: &[VarId], body: ExprId) -> mir::Expr {
        // Create a type for the environment.
        let capture_tys = captures
            .iter()
            .map(|&capture| self.hir.var(capture).ty)
            .collect();
        let env_ty = self.ty_ctx.intern_ty(Ty::Tuple(capture_tys));

        // Construct the environment.
        let env = self.func.add_temp(env_ty);
        let mut env = Place::local(env);
        for (index, &capture) in captures.iter().enumerate() {
            let capture = self.lower_local(capture);
            let ty = self.func.local(capture).ty;
            if self.ty_ctx.is_zst(ty) {
                continue;
            }
            env.accesses.push(FieldAccess {
                idx: index.try_into().expect("tuple too large"),
                ty,
            });
            self.add_stmt(mir::Stmt::Assign {
                place: env.clone(),
                expr: mir::Expr::Copy(Place::local(capture)),
            });
            env.accesses.pop();
        }

        // Create a locally distinct name for the lifted function.
        let func_name = format!("_closure{}", self.lambda_counter);
        self.lambda_counter += 1;

        // Construct the lifted function.
        let mut func = Func::new(params.len() + 1, self.hir.expr(body).ty);
        let def = self.ty_ctx.add_def(DefInfo {
            ident: Ident::new(&func_name),
            module: self.def().module,
            parent: Some(Parent::Def(self.def)),
            params: params
                .iter()
                .map(|var| {
                    let var = self.hir.var(*var);
                    Param {
                        ty: var.ty,
                        mutable: var.mutable,
                    }
                })
                .collect(),
            ret_ty: self.hir.expr(body).ty,
        });
        let mut builder = LowerInfo::new(
            self.hir,
            self.ty_ctx,
            self.handler,
            self.mir,
            def,
            &mut func,
        );
        builder.lower_lambda_body(params, captures, body, env_ty);
        self.mir.add_func(def, func);

        mir::Expr::Closure {
            body: def,
            env: Value::Read(env),
        }
    }

    fn lower_lambda_body(
        &mut self,
        params: &[VarId],
        captures: &[VarId],
        body: ExprId,
        env_ty: TyId,
    ) {
        // Set up function parameters.
        for &param in params {
            let var = self.hir.var(param);
            let local = self.func.add_local(Local {
                ident: Some(var.ident.ident),
                mutable: var.mutable,
                ty: var.ty,
            });
            self.var_map.insert(param, local);
        }
        let env = self.func.add_local(Local {
            ident: None,
            mutable: false,
            ty: env_ty,
        });

        // Extract each capture from the environment.
        let mut env = Place::local(env);
        for (index, &capture) in captures.iter().enumerate() {
            let capture = self.lower_local(capture);
            let ty = self.func.local(capture).ty;
            if self.ty_ctx.is_zst(ty) {
                continue;
            }
            env.accesses.push(FieldAccess {
                idx: index.try_into().expect("tuple too large"),
                ty,
            });
            self.add_stmt(mir::Stmt::Assign {
                place: Place::local(capture),
                expr: mir::Expr::Copy(env.clone()),
            });
            env.accesses.pop();
        }

        // Lower the function body and build the function return.
        self.lower_into_place(Place::local(self.func.ret_place()), body);
        self.set_term(mir::Terminator::Return);
    }

    /// Lowers a block expression, storing the result in `dst`. If `dst` is None, the result is instead dropped.
    ///
    /// This function is guaranteed to assign to `dst` exactly once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    fn lower_block_expr(&mut self, block: &hir::BlockExpr, dst: Option<Place>) {
        let mut locals = Vec::new();

        let [stmts @ .., last] = &block.stmts[..] else {
            return;
        };

        for stmt in stmts {
            if let Some(local) = self.lower_stmt(stmt, None) {
                locals.push(local);
            }
        }

        if let Some(local) = self.lower_stmt(last, dst) {
            locals.push(local);
        }

        // Drop all local variables in this block, in reverse order.
        for local in locals.into_iter().rev() {
            self.add_stmt(mir::Stmt::Drop(Place::local(local)));
        }
    }

    /// Lowers a statement, storing any result in `dst`. If `dst` is None, the result is instead dropped.
    /// If the statement was a declaration, `dst` is not assigned to and the local is returned.
    ///
    /// This function is guaranteed to assign to `place` at most once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    fn lower_stmt(&mut self, stmt: &hir::Stmt, dst: Option<Place>) -> Option<LocalId> {
        match stmt {
            hir::Stmt::Decl { var, value, .. } => {
                let local = self.lower_local(*var);
                self.lower_into_place(Place::local(local), *value);
                self.commit_drops();
                Some(local)
            }
            hir::Stmt::Expr(expr) => {
                match dst {
                    Some(place) => self.lower_into_place(place, *expr),
                    None => _ = self.lower_as_place(*expr),
                }
                self.commit_drops();
                None
            }
        }
    }
}

const fn convert_prefix_op(op: hir::PrefixOp) -> mir::PrefixOp {
    match op {
        hir::PrefixOp::Not => mir::PrefixOp::Not,
        hir::PrefixOp::Neg => mir::PrefixOp::Neg,
    }
}

const fn convert_infix_op(op: hir::InfixOp) -> Option<mir::InfixOp> {
    match op {
        hir::InfixOp::Add | hir::InfixOp::AddF => Some(mir::InfixOp::Add),
        hir::InfixOp::Sub | hir::InfixOp::SubF => Some(mir::InfixOp::Sub),
        hir::InfixOp::Mul | hir::InfixOp::MulF => Some(mir::InfixOp::Mul),
        hir::InfixOp::Div | hir::InfixOp::DivF => Some(mir::InfixOp::Div),
        hir::InfixOp::Exp => panic!("To be removed"),
        hir::InfixOp::And => None,
        hir::InfixOp::Or => None,
        hir::InfixOp::Eqq => Some(mir::InfixOp::Eqq),
        hir::InfixOp::Neq => Some(mir::InfixOp::Neq),
        hir::InfixOp::Gt => Some(mir::InfixOp::Gt),
        hir::InfixOp::Lt => Some(mir::InfixOp::Lt),
        hir::InfixOp::Geq => Some(mir::InfixOp::Geq),
        hir::InfixOp::Leq => Some(mir::InfixOp::Leq),
    }
}
