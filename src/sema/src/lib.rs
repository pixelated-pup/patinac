//! Lowers all [`Asts`][Ast] into a single [`Hir`], resolving stringly variable and type names into numeric identifiers along the way.

mod error;
mod exprs;
mod scope;
mod unify;

use std::range::Range;

use ident::SpanIdent;
use itertools::Itertools;
use slotmap::SecondaryMap;

use errors::{ErrorHandler, HandledError, Result, TryCollectEager as _};
use irs::{
    ModuleId, Package,
    ast::{self, Ast, Binding, Import, Pat, PatKind, TyItem, TyItemKind, TyKind},
    hir::{self, Field, Hir, Param, TyId, VarId, VarInfo},
};

use crate::{error::ErrorKind, scope::ScopeInfo};

struct Driver<'pkg, 'err> {
    package: &'pkg Package,
    asts: &'pkg SecondaryMap<ModuleId, Ast>,
    handler: ErrorHandler<'err>,
    hir: Hir,
    scopes: ScopeInfo<'pkg>,
    constraints: Vec<Constraint>,
}

#[derive(Clone, Copy, Debug)]
enum Constraint {
    Eq(TyId, TyId, Range<u32>, ModuleId),
    Field(TyId, Range<u32>, TyId, SpanIdent, ModuleId),
    Method(TyId, Range<u32>, TyId, SpanIdent, ModuleId),
}

/// Resolves, typechecks, and lowers the provided [`Package`] into a single [`Hir`].
///
/// # Errors
/// Returns an error if there are any unbound variables, undefined types, or multiple items with the same name.
pub fn analyse(
    package: &Package,
    asts: &SecondaryMap<ModuleId, Ast>,
    handler: ErrorHandler,
) -> Result<Hir> {
    todo!()
    // let mut info = ResolveInfo {
    //     package,
    //     asts,
    //     handler,
    //     hir: Hir::default(),
    //     scopes: ScopeInfo::new(package),
    // };
    // info.resolve_module(package.root(), true);
    // info.handler.checked(info.hir)
}

impl Driver<'_, '_> {
    fn resolve_binding(&mut self, binding: &Binding) -> Result<VarId> {
        let ty = binding
            .ty
            .as_ref()
            .map(|ty| self.resolve_ty(ty))
            .transpose()?;
        Ok(self.resolve_pat(&binding.pat, binding.mutable, ty))
    }

    fn resolve_ty(&mut self, ty: &ast::Ty) -> Result<hir::Ty> {
        match &ty.kind {
            TyKind::Tuple(tys) => Ok(hir::Ty::Tuple(self.resolve_tys(tys)?)),
            TyKind::Func(params, ret_ty) => {
                let params = params
                    .iter()
                    .map(|param| {
                        Ok(Param {
                            ty: self.resolve_ty(&param.ty)?,
                            mutable: param.mutable,
                            span: param.span,
                        })
                    })
                    .try_collect_eager();
                let ret_ty = Box::new(self.resolve_ty(ret_ty)?);
                Ok(hir::Ty::Func(params?, ret_ty))
            }
            TyKind::Named(path, args) => {
                if !args.is_empty() {
                    todo!("Generics")
                }

                match &*path.last().str() {
                    "Int" => return Ok(hir::Ty::Int),
                    "UInt" => return Ok(hir::Ty::UInt),
                    "Byte" => return Ok(hir::Ty::Byte),
                    "Bool" => return Ok(hir::Ty::Bool),
                    "Float" => return Ok(hir::Ty::Float),
                    _ => (),
                }

                match self.scopes.resolve_ty(path.as_slice()) {
                    Ok(id) => Ok(hir::Ty::Named(id)),
                    Err(e) => Err(self.err(e, ty.span)),
                }
            }
        }
    }

    fn resolve_tys(&mut self, tys: &[ast::Ty]) -> Result<Vec<hir::Ty>> {
        tys.iter().map(|ty| self.resolve_ty(ty)).try_collect_eager()
    }

    fn resolve_pat(&mut self, pat: &Pat, mutable: bool, ty: Option<hir::Ty>) -> VarId {
        match pat.kind {
            PatKind::Ident(ident) => {
                let id = self.hir.add_var(VarInfo {
                    ident: ident.span(pat.span),
                    mutable,
                    global: false,
                    module: self.scopes.module(),
                });
                if let Some(ty) = ty {
                    self.hir.add_var_ty(id, ty);
                }
                self.scopes.add_var(ident, id);
                id
            }
            _ => todo!("Pattern Matching"),
        }
    }

    fn constrain_eq(&mut self, ty_a: TyId, ty_b: TyId, span: Range<u32>) {
        self.constraints
            .push(Constraint::Eq(ty_a, ty_b, span, self.scopes.module()));
    }

    fn constrain_field(
        &mut self,
        base_ty: TyId,
        base_span: Range<u32>,
        field_ty: TyId,
        field_name: SpanIdent,
    ) {
        self.constraints.push(Constraint::Field(
            base_ty,
            base_span,
            field_ty,
            field_name,
            self.scopes.module(),
        ));
    }

    fn constrain_method(
        &mut self,
        base_ty: TyId,
        base_span: Range<u32>,
        method_ty: TyId,
        method_name: SpanIdent,
    ) {
        self.constraints.push(Constraint::Method(
            base_ty,
            base_span,
            method_ty,
            method_name,
            self.scopes.module(),
        ));
    }

    fn err(&self, error: ErrorKind, span: Range<u32>) -> HandledError {
        self.handler.report(error, span, self.scopes.module())
    }
}
