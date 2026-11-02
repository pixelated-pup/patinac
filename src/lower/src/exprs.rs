use std::{iter, range::Range};

use errors::Report;
use ident::Ident;
use irs::{
    hir::{self, ExprId, ExprKind, LitExpr},
    mir::{self, BlockId, FieldAccess, Func, Item, Place, Terminator, Value},
    ty::{Ty, TyId},
};

use crate::LowerInfo;

impl LowerInfo<'_, '_> {
    pub fn lower_as_expr(&mut self, expr_id: ExprId) -> mir::Expr {
        match &self.hir.expr(expr_id).kind {
            &ExprKind::Infix(op, lhs, rhs) => mir::Expr::Infix(
                convert_infix_op(op),
                self.lower_as_value(lhs),
                self.lower_as_value(rhs),
            ),
            &ExprKind::Prefix(op, arg) => {
                mir::Expr::Prefix(convert_prefix_op(op), self.lower_as_value(arg))
            }
            ExprKind::Lambda {
                params,
                captures,
                body,
            } => todo!(), //self.lower_lambda(expr, params, captures, *body),

            ExprKind::Var(_)
            | ExprKind::Lit(_)
            | ExprKind::Tuple(_)
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
            | ExprKind::Print(_) => mir::Expr::Val(self.lower_as_value(expr_id)),
        }
    }

    pub fn lower_as_value(&mut self, expr_id: ExprId) -> Value {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Var(var) if self.hir.var_info(*var).global => {
                Value::Item(self.lower_global(*var))
            }
            ExprKind::Lit(lit) => Value::Lit(self.lower_lit(lit, expr.span, expr.ty)),
            ExprKind::Assign { place, value } => {
                let place = self.lower_as_place(*place);
                assert!(self.func.local(place.base).mutable);
                self.add_stmt(mir::Stmt::Drop(place.clone()));
                self.lower_into_place(*value, place);
                mir::Value::Zst
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

                Value::Zst
            }
            ExprKind::Break => todo!("Unconditional Control Flow"),
            ExprKind::Continue => todo!("Unconditional Control Flow"),
            ExprKind::Return(_) => todo!("Unconditional Control Flow"),
            ExprKind::Print(expr) => {
                let value = self.lower_as_value(*expr);
                self.add_stmt(mir::Stmt::Print(value));
                Value::Zst
            }

            ExprKind::Var(_) | ExprKind::Field { .. } => Value::Copy(self.lower_as_place(expr_id)),
            ExprKind::Tuple(_)
            | ExprKind::Infix(_, _, _)
            | ExprKind::Prefix(_, _)
            | ExprKind::Call { .. }
            | ExprKind::MethodCall { .. }
            | ExprKind::Lambda { .. }
            | ExprKind::If { .. }
            | ExprKind::Block(_) => Value::Move(self.lower_as_place(expr_id)),
        }
    }

    pub fn lower_as_place(&mut self, expr_id: ExprId) -> Place {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Var(var) if !self.hir.var_info(*var).global => {
                Place::var(self.lower_local(*var))
            }
            ExprKind::Field { base, field } => {
                let Ty::Named { fields, .. } = self.ty_ctx.get(self.hir.expr(*base).ty) else {
                    unreachable!("field access of non-record type")
                };
                let mut place = self.lower_as_place(*base);
                let index = fields
                    .get_index_of(&field.ident)
                    .expect("ICE: Missing Field")
                    .try_into()
                    .expect("ICE: Too Many Fields");
                place.accesses.push(FieldAccess { index, ty: expr.ty });
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
                self.lower_into_place(expr_id, Place::var(temp));
                Place::var(temp)
            }
        }
    }

    /// Lowers the provided expression, storing it's result in `place`.
    ///
    /// This function is guaranteed to only assign to `place` once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    /// The only exception is if it evaluates to a ZST, in which case `place` may never be assigned.
    pub fn lower_into_place(&mut self, expr_id: ExprId, place: Place) {
        let expr = self.hir.expr(expr_id);
        match &expr.kind {
            ExprKind::Tuple(elems) => {
                let mut place = place;
                for (index, elem) in elems.iter().enumerate() {
                    let ty = self.hir.expr(*elem).ty;
                    place.accesses.push(FieldAccess {
                        index: index.try_into().expect("tuple too large"),
                        ty,
                    });
                    self.lower_into_place(*elem, place.clone());
                    place.accesses.pop();
                }
            }
            ExprKind::Call { callee, args } => {
                let callee = self.lower_as_value(*callee);
                let args = args
                    .iter()
                    .map(|arg| {
                        if arg.mutable {
                            mir::Arg::Place(self.lower_as_place(arg.value))
                        } else {
                            mir::Arg::Value(self.lower_as_value(arg.value))
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
                        self.lower_block_expr(th, Some(place.clone()));
                        self.set_term(Terminator::Goto(merge_block));

                        self.block = el_block;
                        self.lower_block_expr(el, Some(place));
                        self.set_term(Terminator::Goto(merge_block));

                        self.block = merge_block;
                    }
                    // No else branch means the overall expression evaluates to unit,
                    // so we don't need to write to the destination place
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
        match lit {
            &LitExpr::Int(value) => match self.ty_ctx.get(ty) {
                Ty::Int => {
                    let i = i64::try_from(value).unwrap_or_else(|_| {
                        self.handler.report(
                            Report::warning(format!(
                                "int literal {value} overflowed and was clamped to {}",
                                i64::MAX
                            )),
                            span,
                            self.module,
                        );
                        i64::MAX
                    });
                    mir::LitValue::Int(i)
                }
                Ty::UInt => mir::LitValue::UInt(value),
                Ty::Byte => {
                    let b = u8::try_from(value).unwrap_or_else(|_| {
                        self.handler.report(
                            Report::warning(format!(
                                "byte literal {value} overflowed and was clamped to {}",
                                u8::MAX
                            )),
                            span,
                            self.module,
                        );
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

    fn lower_lambda(
        &mut self,
        expr: ExprId,
        params: &[hir::VarId],
        captures: &[(hir::VarId, hir::VarId)],
        body: ExprId,
    ) -> mir::Expr {
        // Create a distinct name for the lifted function.
        let func_name = format!("_lambda{}", self.lambda_counter);
        self.lambda_counter += 1;

        let (captures, rebindings): (Vec<_>, Vec<_>) = captures
            .iter()
            .map(|(capture, rebinding)| (self.lower_local(*capture), self.lower_local(*rebinding)))
            .collect();

        // Create a type for the environment.
        let capture_tys = captures
            .iter()
            .map(|var| self.mir.var(*var).ty.clone())
            .collect();
        let env_ty = mir::Ty::Fields(capture_tys);

        // Create the variable representing the lifted function.
        let mir::Ty::Func(mut param_tys, ret_ty) = self.lower_expr_ty(expr) else {
            unreachable!("lambda expression with non-function type")
        };
        param_tys.push(mir::Param {
            ty: env_ty.clone(),
            mutable: false,
        });
        let func_var = self.mir.add_var(mir::Local {
            ident: Ident::new(&func_name),
            ty: mir::Ty::Func(param_tys, ret_ty),
            mutable: false,
        });

        // Add the environment to the parameter list.
        let env = self.mir.add_var(mir::Local {
            ident: Ident::new("env"),
            ty: env_ty,
            mutable: false,
        });
        let param_vars = params
            .iter()
            .map(|var| self.lower_local(*var))
            .chain(iter::once(env))
            .collect();

        // Create the body, formed by extracting each capture from the environment before executing the original body.
        let env = self.mir.add_expr(mir::Expr::Var(env));
        let mut stmts: Vec<_> = rebindings
            .iter()
            .enumerate()
            .map(|(index, var)| {
                let field = u32::try_from(index).expect("too many captures");
                let value = self.mir.add_expr(mir::Expr::Field { base: env, field });
                mir::Stmt::Decl { var: *var, value }
            })
            .collect();
        stmts.push(mir::Stmt::Expr(self.lower_expr(body)));
        let body = self.mir.add_expr(mir::Expr::Block(mir::BlockExpr(stmts)));

        self.mir.add_item(Item {
            var: func_var,
            kind: ItemKind::Func {
                params: param_vars,
                body,
            },
        });

        mir::Expr::Closure {
            func: func_var,
            captures,
        }
    }

    /// Lowers a block expression, storing the result in `place`. If `place` is None, the result is instead dropped.
    ///
    /// This function is guaranteed to assign to `place` exactly once and to never read from it,
    /// so it can safely be used with a fresh immutable temporary.
    /// The only exception is if it evaluates to a ZST, in which case `place` may never be assigned.
    fn lower_block_expr(&mut self, block: &hir::BlockExpr, place: Option<Place>) {
        let mut locals = Vec::new();

        let [stmts @ .., last] = &block.stmts[..] else {
            // Empty block, nothing to do.
            return;
        };

        for stmt in stmts {
            match stmt {
                hir::Stmt::Decl { var, value, .. } => {
                    let local = self.lower_local(*var);
                    self.lower_into_place(*value, Place::var(local));
                    locals.push(local);
                }
                hir::Stmt::Expr(expr) => {
                    let place = self.lower_as_place(*expr);
                    self.add_stmt(mir::Stmt::Drop(place));
                }
            }
        }

        match last {
            hir::Stmt::Decl { var, value, .. } => {
                let local = self.lower_local(*var);
                self.lower_into_place(*value, Place::var(local));
                locals.push(local);
            }
            hir::Stmt::Expr(expr) => match place {
                Some(place) => self.lower_into_place(*expr, place),
                None => {
                    let place = self.lower_as_place(*expr);
                    self.add_stmt(mir::Stmt::Drop(place));
                }
            },
        }

        // Drop all local variables in this block, in reverse order.
        for local in locals.into_iter().rev() {
            self.add_stmt(mir::Stmt::Drop(Place::var(local)));
        }
    }
}

macro_rules! convert_op {
    ($op:ident, $enum_name:ident, $($variant:ident),*) => {
        match $op {
            $(hir::$enum_name::$variant => mir::$enum_name::$variant),*
        }
    };
}

const fn convert_prefix_op(op: hir::PrefixOp) -> mir::PrefixOp {
    convert_op!(op, PrefixOp, Not, Neg)
}

const fn convert_infix_op(op: hir::InfixOp) -> mir::InfixOp {
    convert_op!(
        op, InfixOp, Add, AddF, Sub, SubF, Mul, MulF, Div, DivF, Exp, And, Or, Eqq, Neq, Gt, Lt,
        Geq, Leq
    )
}
