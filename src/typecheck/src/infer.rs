use std::range::Range;

use ident::SpanIdent;
use irs::hir::{
    BlockExpr, ExprId, ExprKind, InfixOp, LitExpr, PrefixOp, Stmt,
    ty::{Param, Ty, TyId},
};

use crate::{TypeChecker, error::ErrorKind};

impl TypeChecker<'_, '_, '_> {
    pub(super) fn infer_expr(&mut self, expr: ExprId) -> TyId {
        let expr = self.hir.expr(expr);
        let module = expr.module;
        let ty = match &expr.kind {
            ExprKind::Var(id) => self.hir.var_info(*id).ty,
            ExprKind::Lit(lit) => match lit {
                LitExpr::Int(_) => self.ty_ctx.fresh_int_var(),
                LitExpr::Float(_) => self.ty_ctx.float(),
                LitExpr::String(_) => todo!("String type"),
                LitExpr::Bool(_) => self.ty_ctx.bool(),
            },
            ExprKind::Tuple(exprs) => {
                let elem_tys = self.infer_exprs(exprs);
                self.ty_ctx.intern_ty(Ty::Tuple(elem_tys))
            }
            ExprKind::Infix { op, lhs, rhs } => self.infer_expr_infix(*op, *lhs, *rhs),
            ExprKind::Prefix { op, expr } => self.infer_expr_prefix(*op, *expr),
            ExprKind::Field { base, field } => self.infer_expr_field(*base, *field, expr.span),
            ExprKind::Call { func, args } => {
                let arg_tys: Vec<_> = args
                    .iter()
                    .map(|arg| Param {
                        ty: self.infer_expr(arg.value),
                        mutable: arg.mutable,
                    })
                    .collect();
                let ret_ty = self.ty_ctx.fresh_var();
                let func_ty = self.ty_ctx.intern_ty(Ty::Func(arg_tys, ret_ty));
                self.unify_expr_ty(*func, func_ty);
                ret_ty
            }
            ExprKind::MethodCall { base, method, args } => {
                let base_ty = self.infer_expr(*base);
                let arg_tys: Vec<_> = args
                    .iter()
                    .map(|arg| Param {
                        ty: self.infer_expr(arg.value),
                        mutable: arg.mutable,
                    })
                    .collect();
                let ret_ty = self.ty_ctx.fresh_var();
                let method_ty = self.ty_ctx.intern_ty(Ty::Func(arg_tys, ret_ty));
                todo!("Method Resolution");
                // let Some(field) = self.hir.ty_info(base_id).fields.get(&field_name.ident) else {
                //     return Err(
                //         ErrorKind::MissingField(base_ty, field_name.ident).span(field_name.span, module)
                //     );
                // };

                // unify_ty_ty(
                //     table,
                //     field_name.span,
                //     module,
                //     field_ty,
                //     &Ty::from(&field.ty),
                // )
                ret_ty
            }
            ExprKind::Lambda {
                params,
                body,
                captures,
            } => {
                let param_tys = params
                    .iter()
                    .map(|param| {
                        let info = self.hir.var_info(*param);
                        Param {
                            ty: info.ty,
                            mutable: info.mutable,
                        }
                    })
                    .collect();
                let body_ty = self.infer_expr(*body);
                self.ty_ctx.intern_ty(Ty::Func(param_tys, body_ty))
            }
            ExprKind::Assign { place, value } => {
                let place_ty = self.infer_expr(*place);
                let value_ty = self.infer_expr(*value);
                self.unify_ty_ty(value_ty, place_ty, self.hir.expr(*value).span, module);
                self.ty_ctx.unit()
            }
            ExprKind::If { cond, th, el } => {
                let bool = self.ty_ctx.bool();
                self.unify_expr_ty(*cond, bool);
                let th_ty = self.infer_block_expr(th);
                match el {
                    Some(el) => {
                        let el_ty = self.infer_block_expr(el);
                        self.unify_ty_ty(el_ty, th_ty, el.span, module);
                    }
                    None => {
                        let unit = self.ty_ctx.unit();
                        self.unify_ty_ty(th_ty, unit, th.span, module)
                    }
                }
                th_ty
            }
            ExprKind::For { id, iter, body } => {
                todo!()
            }
            ExprKind::Loop(body) => {
                self.infer_block_expr(body);
                self.ty_ctx.unit()
            }
            ExprKind::Break => todo!(),
            ExprKind::Continue => todo!(),
            ExprKind::Return(_) => todo!(),
            ExprKind::Block(block) => self.infer_block_expr(block),

            ExprKind::Print(expr) => {
                let _ = self.infer_expr(*expr);
                self.ty_ctx.unit()
            }
        };
        self.unify_ty_ty(ty, expr.ty, expr.span, module);
        ty
    }

    fn infer_expr_infix(&mut self, op: InfixOp, lhs: ExprId, rhs: ExprId) -> TyId {
        match op {
            InfixOp::Add | InfixOp::Sub | InfixOp::Mul | InfixOp::Div => {
                let int_var = self.ty_ctx.fresh_int_var();
                self.unify_expr_ty(lhs, int_var);
                self.unify_expr_ty(rhs, int_var);
                int_var
            }
            InfixOp::AddF | InfixOp::SubF | InfixOp::MulF | InfixOp::DivF => {
                let float = self.ty_ctx.float();
                self.unify_expr_ty(lhs, float);
                self.unify_expr_ty(rhs, float);
                float
            }
            InfixOp::Exp => {
                let float = self.ty_ctx.float();
                self.unify_expr_ty(lhs, float);
                let int_var = self.ty_ctx.fresh_int_var();
                self.unify_expr_ty(rhs, int_var);
                float
            }
            InfixOp::And | InfixOp::Or => {
                let bool = self.ty_ctx.bool();
                self.unify_expr_ty(lhs, bool);
                self.unify_expr_ty(rhs, bool);
                bool
            }
            InfixOp::Eqq | InfixOp::Neq => {
                let lhs_ty = self.infer_expr(lhs);
                self.unify_expr_ty(rhs, lhs_ty);
                self.ty_ctx.bool()
            }
            InfixOp::Gt | InfixOp::Lt | InfixOp::Geq | InfixOp::Leq => {
                let float = self.ty_ctx.float();
                self.unify_expr_ty(lhs, float);
                self.unify_expr_ty(rhs, float);
                self.ty_ctx.bool()
            }
        }
    }

    fn infer_expr_prefix(&mut self, op: PrefixOp, expr: ExprId) -> TyId {
        match op {
            PrefixOp::Not => {
                let bool = self.ty_ctx.bool();
                self.unify_expr_ty(expr, bool);
                bool
            }
            PrefixOp::Neg => {
                let float = self.ty_ctx.float();
                self.unify_expr_ty(expr, float);
                float
            }
        }
    }

    fn infer_expr_field(&mut self, base: ExprId, field_name: SpanIdent, span: Range<u32>) -> TyId {
        let base_ty = self.infer_expr(base);
        let field_ty = self.ty_ctx.fresh_var();

        let base = self.hir.expr(base);

        match self.ty_ctx.get(base_ty) {
            Ty::Named { opaque, module, .. } if *opaque && *module != base.module => {
                self.report(ErrorKind::OpaqueType(base_ty), base.span, base.module);
            }
            Ty::Named { fields, .. } if let Some(field) = fields.get(&field_name.ident) => {
                self.unify_ty_ty(field_ty, field.ty, span, base.module);
            }
            Ty::Named { .. } => {
                self.report(
                    ErrorKind::NoSuchField(base_ty, field_name.ident),
                    field_name.span,
                    base.module,
                );
            }
            Ty::Var(_) => {
                self.report(ErrorKind::UninferredExprType, base.span, base.module);
            }
            _ => {
                self.report(ErrorKind::NoFieldsType(base_ty), base.span, base.module);
            }
        }

        field_ty
    }

    fn infer_exprs(&mut self, exprs: &[ExprId]) -> Vec<TyId> {
        exprs.iter().map(|&e| self.infer_expr(e)).collect()
    }

    fn infer_block_expr(&mut self, block: &BlockExpr) -> TyId {
        let mut stmt_tys: Vec<_> = block
            .stmts
            .iter()
            .map(|stmt| match stmt {
                Stmt::Decl { var, value, .. } => {
                    let var_ty = self.hir.var_info(*var).ty;
                    self.unify_expr_ty(*value, var_ty);
                    self.ty_ctx.unit()
                }
                Stmt::Expr(expr) => self.infer_expr(*expr),
            })
            .collect();
        stmt_tys.pop().unwrap_or_else(|| self.ty_ctx.unit())
    }
}
