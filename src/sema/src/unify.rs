use std::{iter, range::Range};

use ena::unify::InPlaceUnificationTable;
use errors::ErrorHandler;
use ident::SpanIdent;
use irs::{
    ModuleId,
    hir::{Hir, Param, TyId, TyKind, TyVar},
};

use crate::{Constraint, Driver, Table, error::ErrorKind};

// unify: handler, hir
// unify_..._ty: handler, hir

impl Driver<'_, '_> {
    /// Constructs a unification table n
    pub(super) fn unify(&self, constraints: &[Constraint]) -> Table {
        let mut table = Table::new();

        // Set up all the variables we expect to exist
        for _ in 0..self.var_counter {
            table.new_key(None);
        }

        for &constraint in constraints {
            match constraint {
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

        table
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
        lhs: TyId,
        rhs: TyId,
    ) {
        let lhs = self.normalised_ty(table, lhs);
        let rhs = self.normalised_ty(table, rhs);
        match (self.hir.ty_kind(lhs), self.hir.ty_kind(rhs)) {
            (TyKind::Int, TyKind::Int)
            | (TyKind::UInt, TyKind::UInt)
            | (TyKind::Byte, TyKind::Byte)
            | (TyKind::Float, TyKind::Float)
            | (TyKind::Bool, TyKind::Bool) => {}
            (TyKind::Tuple(lhs_elems), TyKind::Tuple(rhs_elems)) => {
                if lhs_elems.len() != rhs_elems.len() {
                    self.report_type_mismatch(lhs, rhs, span, module);
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
                self.unify_var_var(table, span, module, *lhs_var, *rhs_var);
            }
            (&TyKind::Var(var), _) => {
                if occurs_check(&self.hir, rhs, var) {
                    self.handler.report(ErrorKind::Infinite, span, module);
                    return;
                }
                self.unify_var_value(table, span, module, var, rhs);
            }
            (_, &TyKind::Var(var)) => {
                if occurs_check(&self.hir, lhs, var) {
                    self.handler.report(ErrorKind::Infinite, span, module);
                    return;
                }
                self.unify_var_value(table, span, module, var, lhs);
            }
            (TyKind::IntVar(int_var), TyKind::Int | TyKind::UInt | TyKind::Byte) => {
                self.unify_var_value(table, span, module, *int_var, rhs)
            }
            (TyKind::Int | TyKind::UInt | TyKind::Byte, TyKind::IntVar(int_var)) => {
                self.unify_var_value(table, span, module, *int_var, lhs)
            }
            (_, _) => {
                self.report_type_mismatch(lhs, rhs, span, module);
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
        let base_ty = self.normalised_ty(table, base_ty);
        match self.hir.ty_kind(base_ty) {
            TyKind::Named {
                ident,
                opaque,
                fields,
                module: ty_module,
                ..
            } => {
                if *opaque && *ty_module != module {
                    self.handler
                        .report(ErrorKind::OpaqueType(*ident), base_span, module);
                    return;
                }

                let Some(field) = fields
                    .iter()
                    .find(|field| field.ident.ident == field_name.ident)
                else {
                    self.handler.report(
                        ErrorKind::NoSuchField(*ident, field_name.ident),
                        field_name.span,
                        module,
                    );
                    return;
                };

                let ty = field.ty;
                self.unify_ty_ty(table, field_name.span, module, field_ty, ty);
            }
            TyKind::Var(_) => {
                self.handler
                    .report(ErrorKind::UninferredVarType, base_span, module);
                return;
            }
            _ => {
                self.handler.report(
                    ErrorKind::NoFieldsType(self.show_type(base_ty)),
                    base_span,
                    module,
                );
                return;
            }
        }
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
        todo!("Method Resolution")
    }

    fn unify_var_var(
        &self,
        table: &mut Table,
        span: Range<u32>,
        module: ModuleId,
        lhs: TyVar,
        rhs: TyVar,
    ) {
        if let Err((lhs, rhs)) = table.unify_var_var(lhs, rhs) {
            self.report_type_mismatch(lhs, rhs, span, module);
        }
    }

    fn unify_var_value(
        &self,
        table: &mut Table,
        span: Range<u32>,
        module: ModuleId,
        var: TyVar,
        ty: TyId,
    ) {
        if let Err((l, r)) = table.unify_var_value(var, Some(ty)) {
            self.report_type_mismatch(l, r, span, module);
        }
    }

    fn normalised_ty(&self, table: &mut Table, ty: TyId) -> TyId {
        match self.hir.ty_kind(ty) {
            TyKind::Var(v) | TyKind::IntVar(v) => table.probe_value(*v).unwrap_or(ty),
            _ => ty,
        }
    }

    fn report_type_mismatch(&self, lhs: TyId, rhs: TyId, span: Range<u32>, module: ModuleId) {
        let lhs = self.show_type(lhs);
        let rhs = self.show_type(rhs);
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
