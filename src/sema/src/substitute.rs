use std::mem;

use foldhash::HashMap;
use itertools::Itertools as _;
use slotmap::SecondaryMap;

use errors::{Result, TryCollectEager as _};
use irs::hir::{ExprId, Hir, Param, TyId, TyKind};

use crate::{Driver, Table, error::ErrorKind};

impl Driver<'_, '_> {
    pub(super) fn sub_all(&mut self, table: &mut Table) -> Result<HashMap<TyId, TyId>> {
        // Don't even try if we have previous errors
        self.handler.checked(())?;

        for ty in self.hir.tys() {
            match ty {}
        }

        for expr in self.hir.exprs() {
            match self.hir.ty_kind(expr.ty) {
                TyKind::Var(var) => {
                    let ty = table
                        .probe_value(*var)
                        .ok_or_else(|| {
                            self.handler.report(
                                ErrorKind::UninferredExprType,
                                expr.span,
                                expr.module,
                            )
                        })
                        .map_or(Err(()), |ty| sub_ty(table, &ty))?;
                }
                TyKind::IntVar(var) => todo!(),
                _ => {}
            }
        }

        let expr_map = mem::take(&mut self.substitution)
            .iter()
            .map(|(expr, (ty, module))| match sub_ty(&mut self.table, ty) {
                Ok(ty) => Ok((expr, ty)),
                Err(()) => Err(self.handler.report(
                    ErrorKind::UninferredExprType,
                    hir.expr_span(expr),
                    *module,
                )),
            })
            .try_collect_eager();

        let () = mem::take(&mut self.ctx)
            .iter()
            .map(|(var, ty)| match sub_ty(&mut self.table, ty) {
                Ok(ty) => {
                    hir.add_var_ty(var, ty);
                    Ok(())
                }
                Err(()) => {
                    let var_info = hir.var_info(var);
                    Err(self.handler.report(
                        ErrorKind::UninferredVarType,
                        var_info.ident.span,
                        var_info.module,
                    ))
                }
            })
            .try_collect_eager()?;

        expr_map
    }

    fn sub_ty(&mut self, table: &mut Table, ty: TyId) -> Result<TyId, ()> {
        match self.hir.ty_kind(ty) {
            TyKind::Int
            | TyKind::UInt
            | TyKind::Byte
            | TyKind::Float
            | TyKind::Bool
            | TyKind::Named { .. } => Ok(ty),
            TyKind::Tuple(tys) => {
                let tys = tys
                    .iter()
                    .map(|ty| self.sub_ty(table, *ty))
                    .try_collect_eager()?;
                Ok(self.hir.intern_ty(TyKind::Tuple(tys)))
            }
            TyKind::Func(params, ret) => {
                let params = params
                    .iter()
                    .map(|param| {
                        Ok(Param {
                            ty: self.sub_ty(table, param.ty)?,
                            mutable: param.mutable,
                        })
                    })
                    .try_collect_eager();
                let ret = self.sub_ty(table, *ret)?;
                Ok(self.hir.intern_ty(TyKind::Func(params?, ret)))
            }
            TyKind::Var(var) => todo!(),
            TyKind::IntVar(var) => todo!(),
        }
    }
}
