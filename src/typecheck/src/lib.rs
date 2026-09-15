//! Performs typechecking on the [`Hir`], producing a mapping of expressions to their types, and filling in the Hir's mapping of variables to types.

mod substitute;
mod unify;

use std::range::Range;

use ena::unify::{InPlaceUnificationTable, UnificationTable};
use slotmap::SecondaryMap;

use errors::{ErrorHandler, Result};
use ident::SpanIdent;
use irs::{
    ModuleId,
    hir::{DefKind, ExprId, Hir, Ty, VarId},
};

use crate::types::{PartialTy, TyVar};

type Table = InPlaceUnificationTable<TyVar>;

struct TypeChecker<'handler> {
    substitution: SecondaryMap<ExprId, (PartialTy, ModuleId)>,
    ctx: SecondaryMap<VarId, PartialTy>,
}

/// Runs typechecking on the provided [`Hir`], reporting errors through the provided [`ErrorHandler`].
///
/// # Errors
/// Returns an error if any types don't match or can't be inferred.
pub fn type_hir(hir: &mut Hir, handler: ErrorHandler<'_>) -> Result<SecondaryMap<ExprId, Ty>> {
    let mut checker = TypeChecker {
        table: UnificationTable::new(),
        constraints: Vec::new(),
        substitution: SecondaryMap::new(),
        ctx: SecondaryMap::new(),
        handler,
    };

    for exec in hir.defs() {
        match &exec.kind {
            DefKind::Const(val) => {
                let initialiser_ty = checker.infer_expr(hir, exec.module, *val);
                let binding_ty = checker.var_ty(hir, exec.var).clone();
                checker.constrain_eq(initialiser_ty, binding_ty, hir.expr_span(*val), exec.module);
            }
            DefKind::Func { body, .. } => {
                let body_ty = checker.infer_expr(hir, exec.module, *body);
                let PartialTy::Fn(_, ret_ty) = checker.var_ty(hir, exec.var) else {
                    unreachable!("function was given non-function type during nameres")
                };
                let ret_ty = *ret_ty.clone();
                checker.constrain_eq(body_ty, ret_ty, hir.expr_span(*body), exec.module);
            }
        }
    }
    if let Some(main) = hir.main() {
        let DefKind::Func { body, .. } = &main.kind else {
            unreachable!("ICE")
        };
        let body_ty = checker.infer_expr(hir, main.module, *body);
        checker.constrain_eq(
            body_ty,
            PartialTy::unit(),
            hir.expr_span(*body),
            main.module,
        );
    }

    checker.unify(hir);

    checker.sub_all(hir)
}
