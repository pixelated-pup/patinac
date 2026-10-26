//! Performs typechecking on the [`Hir`], producing a mapping of expressions to their types, and filling in the Hir's mapping of variables to types.

mod error;
mod infer;

use std::{iter, range::Range};

use errors::{ErrorHandler, HandledError, Result};
use irs::{
    ModuleId,
    hir::{
        DefKind, ExprId, Hir,
        ty::{Ty, TyContext, TyId},
    },
};

use crate::error::ErrorKind;

struct TypeChecker<'hir, 'ty_ctx, 'handler> {
    hir: &'hir Hir,
    ty_ctx: &'ty_ctx mut TyContext,
    handler: ErrorHandler<'handler>,
}

/// Runs typechecking on the provided [`Hir`], reporting errors through the provided [`ErrorHandler`].
///
/// # Errors
/// Returns an error if any types don't match or can't be inferred.
pub fn type_hir(hir: &Hir, ty_ctx: &mut TyContext, handler: ErrorHandler<'_>) -> Result<()> {
    TypeChecker {
        hir,
        ty_ctx,
        handler,
    }
    .type_hir()
}

impl TypeChecker<'_, '_, '_> {
    fn type_hir(&mut self) -> Result<()> {
        for def in self.hir.defs() {
            match &def.kind {
                DefKind::Const(val) => {
                    let binding_ty = self.hir.var_info(def.var).ty;
                    self.unify_expr_ty(*val, binding_ty);
                }
                DefKind::Func { body, .. } => {
                    let Ty::Func(_, ret_ty) = self.ty_ctx.get(self.hir.var_info(def.var).ty) else {
                        unreachable!("function was given non-function type during nameres")
                    };
                    self.unify_expr_ty(*body, *ret_ty);
                }
            }
        }
        if let Some(main) = self.hir.main() {
            let DefKind::Func { body, .. } = &main.kind else {
                unreachable!("ICE")
            };
            let unit = self.ty_ctx.unit();
            self.unify_expr_ty(*body, unit);
        }

        for expr in self.hir.exprs() {
            if !check_inferred(self.ty_ctx, expr.ty) {
                self.report(ErrorKind::UninferredExprType, expr.span, expr.module);
            }
        }
        for var in self.hir.vars() {
            if !check_inferred(self.ty_ctx, var.ty) {
                self.report(ErrorKind::UninferredVarType, var.ident.span, var.module);
            }
        }

        self.handler.checked(())
    }

    fn unify_ty_ty(&self, lhs: TyId, rhs: TyId, span: Range<u32>, module: ModuleId) {
        fn occurs_check(ty_ctx: &TyContext, ty: TyId, var: TyId) -> bool {
            match ty_ctx.get(ty) {
                Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool | Ty::Named { .. } => false,
                Ty::Tuple(tys) => tys.iter().any(|ty| occurs_check(ty_ctx, *ty, var)),
                Ty::Func(params, ret_ty) => {
                    occurs_check(ty_ctx, *ret_ty, var)
                        || params
                            .iter()
                            .any(|param| occurs_check(ty_ctx, param.ty, var))
                }
                Ty::Var(this_var) | Ty::IntVar(this_var) => *this_var == var,
            }
        }

        match (self.ty_ctx.get(lhs), self.ty_ctx.get(rhs)) {
            (Ty::Int, Ty::Int)
            | (Ty::UInt, Ty::UInt)
            | (Ty::Byte, Ty::Byte)
            | (Ty::Float, Ty::Float)
            | (Ty::Bool, Ty::Bool) => {}
            (Ty::Tuple(lhs_elems), Ty::Tuple(rhs_elems)) if lhs_elems.len() == rhs_elems.len() => {
                for (l, r) in iter::zip(lhs_elems, rhs_elems) {
                    self.unify_ty_ty(*l, *r, span, module);
                }
            }
            (Ty::Func(lhs_params, lhs_ret_ty), Ty::Func(rhs_params, rhs_ret_ty)) => {
                self.unify_ty_ty(*lhs_ret_ty, *rhs_ret_ty, span, module);
                if lhs_params.len() != rhs_params.len() {
                    self.report(
                        ErrorKind::ArgCount {
                            expected: lhs_params.len(),
                            found: rhs_params.len(),
                        },
                        span,
                        module,
                    );
                    return;
                }
                for (l, r) in iter::zip(lhs_params, rhs_params) {
                    self.unify_ty_ty(l.ty, r.ty, span, module);
                    if l.mutable != r.mutable {
                        self.report(
                            ErrorKind::MutMismatch {
                                should_be_mut: l.mutable,
                            },
                            span,
                            module,
                        );
                    }
                }
            }
            (Ty::Named { .. }, Ty::Named { .. }) if lhs == rhs => {}
            (Ty::IntVar(lhs_var), Ty::IntVar(rhs_var)) | (Ty::Var(lhs_var), Ty::Var(rhs_var)) => {
                self.ty_ctx.unify(*lhs_var, *rhs_var);
            }
            (Ty::Var(var), _) => {
                if occurs_check(self.ty_ctx, rhs, *var) {
                    self.report(ErrorKind::Infinite, span, module);
                    return;
                }
                self.ty_ctx.unify(*var, rhs);
            }
            (_, Ty::Var(var)) => {
                if occurs_check(self.ty_ctx, lhs, *var) {
                    self.report(ErrorKind::Infinite, span, module);
                    return;
                }
                self.ty_ctx.unify(*var, lhs);
            }
            (Ty::IntVar(int_var), Ty::Int | Ty::UInt | Ty::Byte) => {
                self.ty_ctx.unify(*int_var, rhs);
            }
            (Ty::Int | Ty::UInt | Ty::Byte, Ty::IntVar(int_var)) => {
                self.ty_ctx.unify(*int_var, lhs);
            }
            _ => {
                self.report(
                    ErrorKind::TypeMismatch {
                        expected: lhs,
                        found: rhs,
                    },
                    span,
                    module,
                );
            }
        }
    }

    fn unify_expr_ty(&mut self, expr: ExprId, ty: TyId) {
        let expr_ty = self.infer_expr(expr);
        let expr = self.hir.expr(expr);
        self.unify_ty_ty(expr_ty, ty, expr.span, expr.module);
    }

    fn report(&self, error: ErrorKind, span: Range<u32>, module: ModuleId) -> HandledError {
        self.handler.report_ctx(error, self.ty_ctx, span, module)
    }
}

fn check_inferred(ty_ctx: &TyContext, ty: TyId) -> bool {
    match ty_ctx.get(ty) {
        Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool | Ty::Named { .. } => true,
        Ty::Tuple(elem_tys) => elem_tys.iter().all(|ty| check_inferred(ty_ctx, *ty)),
        Ty::Func(params, ret_ty) => {
            check_inferred(ty_ctx, *ret_ty)
                && params.iter().all(|param| check_inferred(ty_ctx, param.ty))
        }
        Ty::Var(_) | Ty::IntVar(_) => false,
    }
}
