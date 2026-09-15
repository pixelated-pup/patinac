use foldhash::HashSet;
use itertools::Itertools as _;

use errors::{Result, TryCollectEager as _};
use irs::{
    ast::{self, ExprKind},
    hir::{self, ExprId, Param, TyId, TyKind, VarId},
};

use crate::{Driver, ErrorKind};

impl Driver<'_, '_> {
    pub fn resolve_expr(&mut self, expr: &ast::Expr) -> Result<(ExprId, TyId)> {
        let (new_expr, ty) = match &expr.kind {
            ExprKind::Var(path) => match self.scopes.resolve_var(path.as_slice()) {
                Ok(id) => (hir::Expr::Var(id), self.hir.var_info(id).ty),
                Err(e) => return Err(self.err(e, expr.span)),
            },
            ExprKind::Lit(lit) => {
                let (lit, ty) = match lit {
                    ast::LitExpr::Int(i) => (hir::LitExpr::Int(*i), todo!()), //PartialTy::int_var(&mut self.table)
                    ast::LitExpr::Float(f) => (
                        hir::LitExpr::Float(*f),
                        self.hir.intern_ty(hir::TyKind::Float),
                    ),
                    ast::LitExpr::String(s) => {
                        (hir::LitExpr::String(s.clone()), todo!("String type"))
                    }
                    ast::LitExpr::Bool(b) => (
                        hir::LitExpr::Bool(*b),
                        self.hir.intern_ty(hir::TyKind::Bool),
                    ),
                };
                (hir::Expr::Lit(lit), ty)
            }
            ExprKind::Tuple(exprs) => {
                let (exprs, tys) = self.resolve_exprs(exprs)?;
                (
                    hir::Expr::Tuple(exprs),
                    self.hir.intern_ty(hir::TyKind::Tuple(tys)),
                )
            }
            ExprKind::Infix { op, lhs, rhs } => {
                let rhs = self.resolve_expr(rhs);
                let (lhs, lhs_ty) = self.resolve_expr(lhs)?;
                let (rhs, rhs_ty) = rhs?;
                match convert_infix_op(*op) {
                    Some(op) => {
                        let expr = hir::Expr::Infix { op, lhs, rhs };
                        let ty = match op {
                            hir::InfixOp::Add
                            | hir::InfixOp::Sub
                            | hir::InfixOp::Mul
                            | hir::InfixOp::Div => {
                                let int_var = PartialTy::int_var(&mut self.table);
                                self.constrain_eq(lhs_ty, int_var.clone(), self.hir.expr_span(lhs));
                                self.constrain_eq(rhs_ty, int_var.clone(), self.hir.expr_span(rhs));
                                int_var
                            }
                            hir::InfixOp::AddF
                            | hir::InfixOp::SubF
                            | hir::InfixOp::MulF
                            | hir::InfixOp::DivF => {
                                let float = self.hir.float();
                                self.constrain_eq(lhs_ty, float, self.hir.expr_span(lhs));
                                self.constrain_eq(rhs_ty, float, self.hir.expr_span(rhs));
                                float
                            }
                            hir::InfixOp::Exp => {
                                let float = self.hir.float();
                                self.constrain_eq(lhs_ty, float, self.hir.expr_span(lhs));
                                let int_var = PartialTy::int_var(&mut self.table);
                                self.constrain_eq(rhs_ty, int_var, self.hir.expr_span(rhs));
                                float
                            }
                            hir::InfixOp::And | hir::InfixOp::Or => {
                                let bool = self.hir.bool();
                                self.constrain_eq(lhs_ty, bool, self.hir.expr_span(lhs));
                                self.constrain_eq(rhs_ty, bool, self.hir.expr_span(rhs));
                                bool
                            }
                            hir::InfixOp::Eqq | hir::InfixOp::Neq => {
                                self.constrain_eq(rhs_ty, lhs_ty, self.hir.expr_span(rhs));
                                self.hir.bool()
                            }
                            hir::InfixOp::Gt
                            | hir::InfixOp::Lt
                            | hir::InfixOp::Geq
                            | hir::InfixOp::Leq => {
                                let float = self.hir.float();
                                self.constrain_eq(lhs_ty, float, self.hir.expr_span(lhs));
                                self.constrain_eq(rhs_ty, float, self.hir.expr_span(rhs));
                                self.hir.intern_ty(TyKind::Bool)
                            }
                        };
                        (expr, ty)
                    }
                    None => {
                        self.assert_is_place(lhs);
                        self.constrain_eq(rhs_ty, lhs_ty, self.hir.expr_span(lhs));
                        (
                            hir::Expr::Assign {
                                place: lhs,
                                value: rhs,
                            },
                            self.hir.unit(),
                        )
                    }
                }
            }
            ExprKind::Prefix { op, expr } => {
                let (expr, expr_ty) = self.resolve_expr(expr)?;
                let (ty, op) = match op {
                    ast::PrefixOp::Not => {
                        let bool = self.hir.bool();
                        self.constrain_eq(expr_ty, bool, self.hir.expr_span(expr));
                        (bool, hir::PrefixOp::Not)
                    }
                    ast::PrefixOp::Neg => {
                        let float = self.hir.float();
                        self.constrain_eq(expr_ty, float, self.hir.expr_span(expr));
                        (float, hir::PrefixOp::Neg)
                    }
                };
                (hir::Expr::Prefix { op, expr }, ty)
            }
            ExprKind::Field { base, field } => {
                let (base, base_ty) = self.resolve_expr(base)?;
                let field_ty = PartialTy::var(&mut self.table);
                self.constrain_field(base_ty, self.hir.expr_span(base), field_ty.clone(), *field);
                // TODO: de-stringify field name
                (
                    hir::Expr::Field {
                        base,
                        field: *field,
                    },
                    field_ty,
                )
            }
            ExprKind::Call { func, args } => {
                let func = self.resolve_expr(func);
                let (args, arg_tys) = self.resolve_call_args(args)?;

                let (func, func_ty) = func?;

                let ret_ty = PartialTy::var(&mut self.table);
                self.constrain_eq(
                    func_ty,
                    self.hir.intern_ty(TyKind::Func(arg_tys, ret_ty)),
                    self.hir.expr_span(func),
                );

                (hir::Expr::Call { func, args }, ret_ty)
            }
            ExprKind::MethodCall { base, method, args } => {
                let base = self.resolve_expr(base);
                let (args, arg_tys) = self.resolve_call_args(args)?;
                let (base, base_ty) = base?;

                let ret_ty = PartialTy::var(&mut self.table);
                self.constrain_method(
                    base_ty,
                    self.hir.expr_span(base),
                    self.hir.intern_ty(TyKind::Func(arg_tys, ret_ty)),
                    *method,
                );

                hir::Expr::MethodCall {
                    base,
                    method: *method,
                    args,
                }
            }
            ExprKind::Lambda { params, body } => {
                self.scopes.push_scope();

                // Rebind all captures within the lambda body, making them all immutable.
                let mut captures = HashSet::default();
                self.collect_captures(&mut captures, body);
                let captures = captures
                    .into_iter()
                    .map(|capture| {
                        let info = self.hir.var_info(capture);
                        let rebinding = self.hir.add_var(hir::VarInfo {
                            mutable: false,
                            ..info
                        });
                        self.scopes.add_var(info.ident.ident, rebinding);
                        (capture, rebinding)
                    })
                    .collect();

                let params = params
                    .iter()
                    .map(|param| {
                        let var = self.resolve_binding(param)?;
                        let info = self.hir.var_info(var);
                        Ok((
                            var,
                            hir::Param {
                                ty: info.ty,
                                mutable: info.mutable,
                            },
                        ))
                    })
                    .try_collect_eager();
                let (body, body_ty) = self.resolve_expr(body)?;
                let (params, param_tys) = params?;

                self.scopes.pop_scope();

                let ty = self.hir.intern_ty(TyKind::Func(param_tys, body_ty));
                (
                    hir::Expr::Lambda {
                        params,
                        body,
                        captures,
                    },
                    ty,
                )
            }
            ExprKind::If { cond, th, el } => {
                let cond = self.resolve_expr(cond);
                let th = self.resolve_block_expr(th);
                let el = el
                    .as_ref()
                    .map(|el| self.resolve_block_expr(el))
                    .transpose()?;
                let (th, th_ty) = th?;
                let (cond, cond_ty) = cond?;

                self.constrain_eq(cond_ty, self.hir.bool(), self.hir.expr_span(cond));

                if let Some((el, el_ty)) = el {
                    self.constrain_eq(el_ty, th_ty, el.span);
                }

                (
                    hir::Expr::If {
                        cond,
                        th,
                        el: el.map(|(el, _)| el),
                    },
                    th_ty,
                )
            }
            ExprKind::Match { .. } => todo!("Pattern Matching"),
            ExprKind::For { pat, iter, body } => todo!("For Loops"),
            ExprKind::Loop(body) => {
                let (body, _) = self.resolve_block_expr(body)?;
                (hir::Expr::Loop(body), self.hir.unit())
            }
            ExprKind::Break => todo!(),
            ExprKind::Continue => todo!(),
            ExprKind::Return(expr) => todo!(),
            ExprKind::Block(stmts) => {
                let (stmts, ty) = self.resolve_block_expr(stmts)?;
                (hir::Expr::Block(stmts), ty)
            }
            ExprKind::Print(expr) => {
                let (expr, _) = self.resolve_expr(expr)?;
                (hir::Expr::Print(expr), self.hir.unit())
            }
        };

        let expr = self.hir.add_expr(new_expr, expr.span, ty);
        Ok((expr, ty))
    }

    fn resolve_exprs(&mut self, exprs: &[ast::Expr]) -> Result<(Vec<ExprId>, Vec<TyId>)> {
        exprs
            .iter()
            .map(|expr| self.resolve_expr(expr))
            .try_collect_eager()
    }

    fn resolve_block_expr(&mut self, block: &ast::BlockExpr) -> Result<(hir::BlockExpr, TyId)> {
        self.scopes.push_scope();
        let stmts: Vec<_> = block
            .stmts
            .iter()
            .map(|stmt| match stmt {
                ast::Stmt::Decl {
                    binding,
                    value,
                    span,
                } => {
                    // The value must be resolved before the binding,
                    // to ensure the declared variable isn't in scope within it's own declaration
                    let value = self.resolve_expr(value);
                    let var = self.resolve_binding(binding)?;
                    let (value, value_ty) = value?;

                    let var_ty = self.hir.var_info(var).ty;
                    self.constrain_eq(value_ty, var_ty, self.hir.expr_span(value));

                    Ok(hir::Stmt::Decl {
                        var,
                        value,
                        span: *span,
                    })
                }
                ast::Stmt::Expr(expr) => self
                    .resolve_expr(expr)
                    .map(|(expr, _)| hir::Stmt::Expr(expr)),
            })
            .try_collect_eager()?;
        self.scopes.pop_scope();

        let ty = stmts
            .last()
            .and_then(|stmt| match stmt {
                hir::Stmt::Decl { .. } => None,
                hir::Stmt::Expr(expr) => Some(self.hir.expr_ty(*expr)),
            })
            .unwrap_or_else(|| self.hir.unit());

        Ok((
            hir::BlockExpr {
                stmts,
                span: block.span,
            },
            ty,
        ))
    }

    fn resolve_call_args(&mut self, args: &[ast::Arg]) -> Result<(Vec<hir::Arg>, Vec<hir::Param>)> {
        let args: Vec<_> = args
            .iter()
            .map(|arg| {
                let (val, val_ty) = self.resolve_expr(&arg.value)?;
                if arg.mutable {
                    self.assert_is_place(val);
                }
                let arg = hir::Arg {
                    value: val,
                    mutable: arg.mutable,
                    span: arg.span,
                };
                let param = Param {
                    ty: val_ty,
                    mutable: arg.mutable,
                };
                Ok((arg, param))
            })
            .try_collect_eager()?;

        let (args, arg_tys): (Vec<_>, Vec<_>) = args.into_iter().unzip();

        // Verify uniqueness of mutable arguments
        // TODO optimise???
        args.iter()
            .permutations(2)
            .map(|p| (p[0], p[1]))
            .filter(|(a, b)| a.mutable || b.mutable)
            .try_for_each(|(a, b)| {
                if self.overlaps(a.value, b.value) {
                    Err(self.err(ErrorKind::OverlappingArgs(a.span, b.span), a.span))
                } else {
                    Ok(())
                }
            })?;

        Ok((args, arg_tys))
    }

    fn assert_is_place(&mut self, place: ExprId) {
        match self.hir.expr(place) {
            hir::Expr::Var(id) => {
                let info = self.hir.var_info(*id);
                if !info.mutable {
                    self.err(
                        ErrorKind::Mutation(info.ident.ident),
                        self.hir.expr_span(place),
                    );
                }
            }
            hir::Expr::Field { base, .. } => self.assert_is_place(*base),
            _ => {
                self.err(ErrorKind::NotPlaceExpr, self.hir.expr_span(place));
            }
        }
    }

    fn overlaps(&self, a: ExprId, b: ExprId) -> bool {
        match (self.hir.expr(a), self.hir.expr(b)) {
            (hir::Expr::Var(a), hir::Expr::Var(b)) => a == b,
            (hir::Expr::Var(_), hir::Expr::Field { base, .. }) => self.overlaps(a, *base),
            (
                hir::Expr::Field {
                    base: base_a,
                    field: field_a,
                },
                hir::Expr::Field {
                    base: base_b,
                    field: field_b,
                },
            ) => (field_a.ident == field_b.ident) && self.overlaps(*base_a, *base_b),
            _ => false,
        }
    }

    fn collect_captures(&self, captures: &mut HashSet<VarId>, expr: &ast::Expr) {
        match &expr.kind {
            ExprKind::Var(path) => {
                // Unbound variables are either parameters, which don't need capturing, or actually unbound, which will produce an error anyway.
                if let Ok(id) = self.scopes.resolve_var(path.as_slice())
                    && !self.hir.var_info(id).global
                {
                    captures.insert(id);
                }
            }
            ExprKind::Lit(_) | ExprKind::Break | ExprKind::Continue => {}
            ExprKind::Tuple(exprs) => {
                for e in exprs {
                    self.collect_captures(captures, e);
                }
            }
            ExprKind::Lambda { body: e, .. }
            | ExprKind::Field { base: e, .. }
            | ExprKind::Prefix { expr: e, .. }
            | ExprKind::Print(e)
            | ExprKind::Return(e) => self.collect_captures(captures, e),
            ExprKind::Infix { lhs, rhs, .. } => {
                self.collect_captures(captures, lhs);
                self.collect_captures(captures, rhs);
            }
            ExprKind::Call { func, args } => {
                self.collect_captures(captures, func);
                for a in args {
                    self.collect_captures(captures, &a.value);
                }
            }
            ExprKind::MethodCall { base, args, .. } => {
                self.collect_captures(captures, base);
                for a in args {
                    self.collect_captures(captures, &a.value);
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.collect_captures(captures, scrutinee);
                for a in arms {
                    self.collect_captures(captures, &a.body);
                }
            }
            ExprKind::If { cond, th, el } => {
                self.collect_captures(captures, cond);
                self.collect_block_captures(captures, th);
                el.as_ref()
                    .inspect(|el| self.collect_block_captures(captures, el));
            }
            ExprKind::For { iter, body, .. } => {
                self.collect_captures(captures, iter);
                self.collect_block_captures(captures, body);
            }
            ExprKind::Loop(stmts) | ExprKind::Block(stmts) => {
                self.collect_block_captures(captures, stmts);
            }
        }
    }

    fn collect_block_captures(&self, captures: &mut HashSet<VarId>, block: &ast::BlockExpr) {
        for ast::Stmt::Decl { value: expr, .. } | ast::Stmt::Expr(expr) in &block.stmts {
            self.collect_captures(captures, expr);
        }
    }
}

/// Converts an ast infix operator to a hir infix operator, returning `None` if the ast operator was `Assign`.
const fn convert_infix_op(op: ast::InfixOp) -> Option<hir::InfixOp> {
    match op {
        ast::InfixOp::Assign => None,
        ast::InfixOp::Add => Some(hir::InfixOp::Add),
        ast::InfixOp::AddF => Some(hir::InfixOp::AddF),
        ast::InfixOp::Sub => Some(hir::InfixOp::Sub),
        ast::InfixOp::SubF => Some(hir::InfixOp::SubF),
        ast::InfixOp::Mul => Some(hir::InfixOp::Mul),
        ast::InfixOp::MulF => Some(hir::InfixOp::MulF),
        ast::InfixOp::Div => Some(hir::InfixOp::Div),
        ast::InfixOp::DivF => Some(hir::InfixOp::DivF),
        ast::InfixOp::Exp => Some(hir::InfixOp::Exp),
        ast::InfixOp::And => Some(hir::InfixOp::And),
        ast::InfixOp::Or => Some(hir::InfixOp::Or),
        ast::InfixOp::Eqq => Some(hir::InfixOp::Eqq),
        ast::InfixOp::Neq => Some(hir::InfixOp::Neq),
        ast::InfixOp::Gt => Some(hir::InfixOp::Gt),
        ast::InfixOp::Lt => Some(hir::InfixOp::Lt),
        ast::InfixOp::Geq => Some(hir::InfixOp::Geq),
        ast::InfixOp::Leq => Some(hir::InfixOp::Leq),
    }
}
