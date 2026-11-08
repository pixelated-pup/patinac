use std::range::Range;

use errors::Report;
use ident::Ident;
use irs::{
    hir::{self, ExprId, ExprKind, LitExpr, VarId},
    mir::{self, FieldAccess, Func, Local, LocalId, Place, Terminator, Value},
    ty::{DefInfo, Ty, TyId},
};

use crate::LowerInfo;

impl LowerInfo<'_, '_, '_> {
    /// The resulting expression has an indefinite lifetime.
    pub fn lower_as_expr(&mut self, expr_id: ExprId) -> mir::Expr {
        match &self.hir.expr(expr_id).kind {
            ExprKind::Infix(op, lhs, rhs)
                if !matches!(op, hir::InfixOp::And | hir::InfixOp::Or) =>
            {
                mir::Expr::Infix(
                    convert_infix_op(*op).unwrap(),
                    self.lower_as_value(*lhs, true),
                    self.lower_as_value(*rhs, true),
                )
            }
            ExprKind::Prefix(op, arg) => {
                mir::Expr::Prefix(convert_prefix_op(*op), self.lower_as_value(*arg, true))
            }
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
            | ExprKind::Call { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Assign { .. }
            | ExprKind::If { .. }
            | ExprKind::For { .. }
            | ExprKind::Loop(_)
            | ExprKind::Break
            | ExprKind::Continue
            | ExprKind::Return(_)
            | ExprKind::Block(_)
            | ExprKind::Print(_) => mir::Expr::Val(self.lower_as_value(expr_id, false)),
        }
    }

    /// `share` should be true when the result is not mutated and
    /// does not outlive the containing statement.
    ///
    /// Situations where the value outlive the containing statement include:
    /// - Returns
    /// - Assignments
    pub fn lower_as_value(&mut self, expr_id: ExprId, share: bool) -> Value {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Lit(lit) => Value::Lit(self.lower_lit(lit, expr.span, expr.ty)),
            ExprKind::Assign { place, value } => {
                let place = self.lower_as_place(*place);
                assert!(self.func.local(place.base).mutable);
                self.add_stmt(mir::Stmt::Drop(place.clone()));
                self.lower_into_place(*value, place);
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
                let value = self.lower_as_value(*expr, true);
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
                } else if share {
                    Value::Share(place)
                } else {
                    Value::Copy(place)
                }
            }
        }
    }

    /// Handles dropping the returned place if necessary.
    pub fn lower_as_place(&mut self, expr_id: ExprId) -> Place {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Var(var) => Place::local(self.lower_local(*var)),
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
            ExprKind::Lit(_)
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
                self.lower_into_place(expr_id, Place::local(temp));
                self.stage_drop(temp);
                Place::local(temp)
            }
        }
    }

    /// Lowers the provided expression, storing it's result in `place`.
    ///
    /// Caller's responsibility to ensure that `place` gets dropped.
    ///
    /// This function is guaranteed to only assign to `place` once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    pub fn lower_into_place(&mut self, expr_id: ExprId, place: Place) {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Tuple(elems) => {
                let mut place = place;
                for (index, elem) in elems.iter().enumerate() {
                    let ty = self.hir.expr(*elem).ty;
                    place.accesses.push(FieldAccess {
                        idx: index.try_into().expect("tuple too large"),
                        ty,
                    });
                    self.lower_into_place(*elem, place.clone());
                    place.accesses.pop();
                }
            }
            ExprKind::Infix(op, lhs, rhs) if matches!(op, hir::InfixOp::And | hir::InfixOp::Or) => {
                let lhs = self.lower_as_value(*lhs, true);

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
                    place: place.clone(),
                    expr: mir::Expr::Val(lhs),
                });
                self.set_term(Terminator::Goto(merge_block));

                self.block = cont_block;
                self.lower_into_place(*rhs, place);
                self.set_term(Terminator::Goto(merge_block));

                self.block = merge_block;
            }
            ExprKind::Call { callee, args } => {
                let callee = self.lower_as_value(*callee, true);
                let args = args
                    .iter()
                    .map(|arg| {
                        if arg.mutable {
                            mir::Arg::Place(self.lower_as_place(arg.value))
                        } else {
                            mir::Arg::Value(self.lower_as_value(arg.value, true))
                        }
                    })
                    .collect();
                self.add_stmt(mir::Stmt::Call {
                    callee,
                    args,
                    dst: place,
                });
            }
            ExprKind::MethodCall { base, method, args } => todo!("Method Lowering"),
            ExprKind::If { cond, th, el } => {
                let cond = self.lower_as_value(*cond, true);

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
                        self.lower_block_expr(th, Some(place.clone()));
                        self.set_term(Terminator::Goto(merge_block));

                        self.block = el_block;
                        self.lower_block_expr(el, Some(place));
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
            ExprKind::Block(block) => self.lower_block_expr(block, Some(place)),

            ExprKind::Var(_)
            | ExprKind::Lit(_)
            | ExprKind::Infix(_, _, _)
            | ExprKind::Prefix(_, _)
            | ExprKind::Field { .. }
            | ExprKind::Lambda { .. }
            | ExprKind::Assign { .. }
            | ExprKind::For { .. }
            | ExprKind::Loop(_)
            | ExprKind::Break
            | ExprKind::Continue
            | ExprKind::Return(_)
            | ExprKind::Print(_) => {
                let expr = self.lower_as_expr(expr_id);
                self.add_stmt(mir::Stmt::Assign { place, expr });
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
                expr: mir::Expr::Val(Value::Copy(Place::local(capture))),
            });
            env.accesses.pop();
        }

        // Create a distinct name for the lifted function.
        let func_name = format!("_{}::closure{}", self.def().ident, self.lambda_counter);
        self.lambda_counter += 1;

        // Construct the lifted function.
        let mut body_func = Func::new(params.len() + 1, self.hir.expr(body).ty);
        let def = self.ty_ctx.add_def(DefInfo {
            ident: Ident::new(&func_name),
            module: self.def().module,
        });
        let mut builder = LowerInfo::new(
            self.hir,
            self.ty_ctx,
            self.handler,
            self.mir,
            def,
            &mut body_func,
        );
        builder.lift_lambda_body(params, captures, body, env_ty);
        self.mir.add_func(def, body_func);

        mir::Expr::Closure {
            body: def,
            env: Value::Share(env),
        }
    }

    fn lift_lambda_body(
        &mut self,
        params: &[VarId],
        captures: &[VarId],
        body: ExprId,
        env_ty: TyId,
    ) {
        // Set up function parameters.
        for &param in params {
            let info = self.hir.var(param);
            let local = self.func.add_local(Local {
                ident: Some(info.ident.ident),
                mutable: info.mutable,
                ty: info.ty,
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
                expr: mir::Expr::Val(Value::Copy(env.clone())),
            });
            env.accesses.pop();
        }

        // Lower the function body and build the function return.
        let ret_value = self.lower_as_value(body, false);
        self.set_term(mir::Terminator::Return(ret_value));
    }

    /// Lowers a block expression, storing the result in `place`. If `place` is None, the result is instead dropped.
    ///
    /// This function is guaranteed to assign to `place` exactly once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    fn lower_block_expr(&mut self, block: &hir::BlockExpr, place: Option<Place>) {
        let mut locals = Vec::new();

        let [stmts @ .., last] = &block.stmts[..] else {
            return;
        };

        for stmt in stmts {
            if let Some(local) = self.lower_stmt(stmt, None) {
                locals.push(local);
            }
        }

        if let Some(local) = self.lower_stmt(last, place) {
            locals.push(local);
        }

        // Drop all local variables in this block, in reverse order.
        for local in locals.into_iter().rev() {
            self.add_stmt(mir::Stmt::Drop(Place::local(local)));
        }
    }

    /// Lowers a statement, storing any result in `place`. If `place` is None, the result is instead dropped.
    /// If the statement was a declaration, `place` is not assigned to and the local is returned.
    ///
    /// This function is guaranteed to assign to `place` at most once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    fn lower_stmt(&mut self, stmt: &hir::Stmt, place: Option<Place>) -> Option<LocalId> {
        match stmt {
            hir::Stmt::Decl { var, value, .. } => {
                let local = self.lower_local(*var);
                self.lower_into_place(*value, Place::local(local));
                self.commit_drops();
                Some(local)
            }
            hir::Stmt::Expr(expr) => {
                match place {
                    Some(place) => self.lower_into_place(*expr, place),
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
        hir::InfixOp::Exp => Some(mir::InfixOp::Exp),
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
