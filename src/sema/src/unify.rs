use std::{iter, range::Range};

use ena::unify::InPlaceUnificationTable;
use errors::ErrorHandler;
use ident::SpanIdent;
use irs::{
    ModuleId,
    hir::{Hir, Param, TyId, TyKind, TyVar},
};

use crate::{Constraint, Driver, error::ErrorKind};

type Table = InPlaceUnificationTable<TyVar>;

// unify: constraints, handler, hir
// unify_..._ty: handler, hir

impl Driver<'_, '_> {
    /// Unifies all types in the unification table.
    pub(super) fn unify(&self) {
        let mut table = Table::new();

        for &constr in &self.constraints {
            match constr {
                Constraint::Eq(ty_a, ty_b, span, module) => {
                    self.unify_ty_ty(&mut table, span, module, ty_a, ty_b);
                }
                Constraint::Field(base_ty, base_span, field_ty, field_name, module) => {
                    self.unify_field_ty(
                        &mut table, base_ty, base_span, field_ty, field_name, module,
                    );
                }
                Constraint::Method(base_ty, base_span, method_ty, method_name, module) => {
                    self.unify_method_ty(
                        &mut table,
                        base_ty,
                        base_span,
                        method_ty,
                        method_name,
                        module,
                    );
                }
            }
        }
    }

    /// Recursively traverse two types until at least one is a type variable,
    /// at which point we unify them in the table,
    /// or until we can no longer traverse them or we know they're mismatched,
    /// at which point we error.
    fn unify_ty_ty(
        &self,
        table: &mut Table,
        span: Range<u32>,
        module: ModuleId,
        unnorm_lhs: TyId,
        unnorm_rhs: TyId,
    ) {
        let lhs = normalize_ty(table, unnorm_lhs);
        let rhs = normalize_ty(table, unnorm_rhs);

        match (self.hir.ty_kind(lhs), self.hir.ty_kind(rhs)) {
            (TyKind::Int, TyKind::Int)
            | (TyKind::UInt, TyKind::UInt)
            | (TyKind::Byte, TyKind::Byte)
            | (TyKind::Float, TyKind::Float)
            | (TyKind::Bool, TyKind::Bool) => {}
            (TyKind::Tuple(lhs_elems), TyKind::Tuple(rhs_elems)) => {
                if lhs_elems.len() != rhs_elems.len() {
                    self.handler.report(
                        ErrorKind::TypeMismatch {
                            expected: lhs,
                            found: rhs,
                        },
                        span,
                        module,
                    );
                    return;
                }
                for (&l, &r) in iter::zip(lhs_elems, rhs_elems) {
                    self.unify_ty_ty(table, span, module, l, r);
                }
            }
            (TyKind::Func(lhs_params, lhs_ret), TyKind::Func(rhs_params, rhs_ret)) => {
                self.unify_ty_ty(table, span, module, *lhs_ret, *rhs_ret);
                for (l, r) in iter::zip(lhs_params, rhs_params) {
                    self.unify_ty_ty(table, span, module, l.ty, r.ty);
                    if l.mutable != r.mutable {
                        self.handler.report(
                            ErrorKind::MutMismatch {
                                should_be_mut: l.mutable,
                            },
                            span,
                            module,
                        );
                    }
                }
            }
            (TyKind::Named { .. }, TyKind::Named { .. }) if lhs == rhs => {}
            (
                TyKind::Var(lhs_var) | TyKind::IntVar(lhs_var),
                TyKind::Var(rhs_var) | TyKind::IntVar(rhs_var),
            ) => {
                unify_var_var(table, &self.handler, span, module, *lhs_var, *rhs_var);
            }
            (&TyKind::Var(var), _) => {
                if occurs_check(&self.hir, rhs, var) {
                    self.handler.report(ErrorKind::Infinite, span, module);
                    return;
                }
                unify_var_value(table, &self.handler, span, module, var, rhs);
            }
            (_, &TyKind::Var(var)) => {
                if occurs_check(&self.hir, lhs, var) {
                    self.handler.report(ErrorKind::Infinite, span, module);
                    return;
                }
                unify_var_value(table, &self.handler, span, module, var, lhs);
            }
            (TyKind::IntVar(int_var), TyKind::Int | TyKind::UInt | TyKind::Byte) => {
                unify_var_value(table, &self.handler, span, module, *int_var, rhs)
            }
            (TyKind::Int | TyKind::UInt | TyKind::Byte, TyKind::IntVar(int_var)) => {
                unify_var_value(table, &self.handler, span, module, *int_var, lhs)
            }
            (_, _) => {
                self.handler.report(
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

    fn unify_field_ty(
        &self,
        table: &mut Table,
        base_ty: TyId,
        base_span: Range<u32>,
        field_ty: TyId,
        field_name: SpanIdent,
        module: ModuleId,
    ) {
        let base_ty = normalize_ty(table, base_ty);

        let base_id = match base_ty {
            TyKind::Named(id) => id,
            TyKind::Var(_) => {
                handler.report(ErrorKind::UninferredVarType, base_span, module);
                return;
            }
            no_fields_ty => {
                handler.report(ErrorKind::NoFieldsType(no_fields_ty), base_span, module);
                return;
            }
        };

        let ty_info = hir.ty_info(base_id);

        if ty_info.opaque && ty_info.module != module {
            handler.report(
                ErrorKind::OpaqueType(hir.ty_ident(base_id).ident),
                base_span,
                module,
            );
            return;
        }

        let Some(field) = ty_info.get_field(field_name.ident) else {
            self.handler.report(
                ErrorKind::NoSuchField(hir.ty_ident(base_id).ident, field_name.ident),
                field_name.span,
                module,
            );
            return;
        };

        self.unify_ty_ty(
            table,
            field_name.span,
            module,
            field_ty,
            &TyKind::from(&field.ty),
        );
    }

    fn unify_method_ty(
        &self,
        table: &mut Table,
        base_ty: TyId,
        base_span: Range<u32>,
        method_ty: TyId,
        method_name: SpanIdent,
        module: ModuleId,
    ) {
        let Ok(base_ty) = Ty::try_from(normalize_ty(table, base_ty)) else {
            self.handler
                .report(ErrorKind::UninferredVarType, base_span, module);
            return;
        };

        todo!("Method Resolution")
        // let Some(field) = hir.ty_info(base_id).fields.get(&field_name.ident) else {
        //     return Err(
        //         ErrorKind::MissingField(base_ty, field_name.ident).span(field_name.span, module)
        //     );
        // };

        // unify_ty_ty(
        //     table,
        //     field_name.span,
        //     module,
        //     field_ty,
        //     &TyKind::from(&field.ty),
        // )
    }
}

fn unify_var_var(
    table: &mut Table,
    handler: &ErrorHandler,
    span: Range<u32>,
    module: ModuleId,
    l: TyVar,
    r: TyVar,
) {
    if let Err((l, r)) = table.unify_var_var(l, r) {
        handler.report(
            ErrorKind::TypeMismatch {
                expected: l,
                found: r,
            },
            span,
            module,
        );
    }
}

fn unify_var_value(
    table: &mut Table,
    handler: &ErrorHandler,
    span: Range<u32>,
    module: ModuleId,
    var: TyVar,
    ty: TyId,
) {
    if let Err((l, r)) = table.unify_var_value(var, Some(ty)) {
        handler.report(
            ErrorKind::TypeMismatch {
                expected: l,
                found: r,
            },
            span,
            module,
        );
    }
}

fn normalize_ty(hir: &mut Hir, table: &mut Table, ty: TyId) -> TyId {
    match hir.ty_kind(ty) {
        TyKind::Int
        | TyKind::UInt
        | TyKind::Byte
        | TyKind::Float
        | TyKind::Bool
        | TyKind::Named { .. } => ty,
        TyKind::Tuple(tys) => {
            let tys = tys
                .iter()
                .copied()
                .map(|ty| normalize_ty(hir, table, ty))
                .collect();
            hir.intern_ty(TyKind::Tuple(tys))
        }
        TyKind::Func(params, result) => {
            let params = params
                .iter()
                .copied()
                .map(|param| Param {
                    ty: normalize_ty(hir, table, param.ty),
                    mutable: param.mutable,
                })
                .collect();
            let result = normalize_ty(hir, table, *result);
            hir.intern_ty(TyKind::Func(params, result))
        }
        TyKind::Var(v) => match table.probe_value(*v) {
            Some(ty) => normalize_ty(hir, table, ty),
            None => hir.intern_ty(TyKind::Var(table.find(*v))),
        },
        TyKind::IntVar(v) => match table.probe_value(*v) {
            Some(ty) => normalize_ty(hir, table, ty),
            None => hir.intern_ty(TyKind::IntVar(table.find(*v))),
        },
    }
}

fn occurs_check(hir: &Hir, ty: TyId, var: TyVar) -> bool {
    match hir.ty_kind(ty) {
        TyKind::Int
        | TyKind::UInt
        | TyKind::Byte
        | TyKind::Float
        | TyKind::Bool
        | TyKind::Named { .. } => false,
        TyKind::Tuple(tys) => tys.iter().any(|ty| occurs_check(hir, *ty, var)),
        TyKind::Func(params, result) => {
            occurs_check(hir, *result, var)
                || params.iter().any(|param| occurs_check(hir, param.ty, var))
        }
        TyKind::Var(this_var) | TyKind::IntVar(this_var) => *this_var == var,
    }
}
