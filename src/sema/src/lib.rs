//! Lowers all [`Asts`][Ast] into a single [`Hir`], resolving stringly variable and type names into numeric identifiers along the way.

mod error;
mod exprs;
mod scope;
mod substitute;
mod unify;

use std::range::Range;

use ena::unify::{InPlaceUnificationTable, UnifyKey};
use ident::SpanIdent;
use itertools::Itertools;
use slotmap::SecondaryMap;

use errors::{ErrorHandler, HandledError, Result, TryCollectEager as _};
use irs::{
    ModuleId, Package,
    ast::{self, Ast, Binding, Import, Pat, PatKind, TyItem, TyItemKind},
    hir::{self, Field, Hir, Param, TyId, TyVar, VarId, VarInfo},
};

use crate::{error::ErrorKind, scope::ScopeInfo};

type Table = InPlaceUnificationTable<TyVar>;

struct Driver<'pkg, 'err> {
    package: &'pkg Package,
    asts: &'pkg SecondaryMap<ModuleId, Ast>,
    handler: ErrorHandler<'err>,
    hir: Hir,
    scopes: ScopeInfo<'pkg>,
    var_counter: u32,
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
        let ty = self.resolve_opt_ty(binding.ty.as_ref())?;
        Ok(self.resolve_pat(&binding.pat, binding.mutable, ty))
    }

    fn resolve_ty(&mut self, ty: &ast::Ty) -> Result<TyId> {
        match &ty.kind {
            ast::TyKind::Tuple(tys) => {
                let tys = self.resolve_tys(tys)?;
                Ok(self.hir.intern_ty(hir::TyKind::Tuple(tys)))
            }
            ast::TyKind::Func(params, ret_ty) => {
                let params = params
                    .iter()
                    .map(|param| {
                        Ok(Param {
                            ty: self.resolve_ty(&param.ty)?,
                            mutable: param.mutable,
                            // span: param.span,
                        })
                    })
                    .try_collect_eager();
                let ret_ty = self.resolve_ty(ret_ty)?;
                Ok(self.hir.intern_ty(hir::TyKind::Func(params?, ret_ty)))
            }
            ast::TyKind::Named(path, args) => {
                if !args.is_empty() {
                    todo!("Generics")
                }

                let prim_kind = match &*path.last().str() {
                    "Int" => Some(hir::TyKind::Int),
                    "UInt" => Some(hir::TyKind::UInt),
                    "Byte" => Some(hir::TyKind::Byte),
                    "Bool" => Some(hir::TyKind::Bool),
                    "Float" => Some(hir::TyKind::Float),
                    _ => None,
                };

                match prim_kind {
                    Some(prim_kind) => Ok(self.hir.intern_ty(prim_kind)),
                    None => self
                        .scopes
                        .resolve_ty(path.as_slice())
                        .map_err(|e| self.err(e, ty.span)),
                }
            }
        }
    }

    fn resolve_tys(&mut self, tys: &[ast::Ty]) -> Result<Vec<TyId>> {
        tys.iter().map(|ty| self.resolve_ty(ty)).try_collect_eager()
    }

    fn resolve_opt_ty(&mut self, ty: Option<&ast::Ty>) -> Result<TyId> {
        match ty {
            Some(ty) => self.resolve_ty(ty),
            None => Ok(self.fresh_ty_var()),
        }
    }

    fn resolve_pat(&mut self, pat: &Pat, mutable: bool, ty: TyId) -> VarId {
        match pat.kind {
            PatKind::Ident(ident) => {
                let id = self.hir.add_var(VarInfo {
                    ident: ident.span(pat.span),
                    mutable,
                    ty,
                    global: false,
                    module: self.scopes.module(),
                });
                self.scopes.add_var(ident, id);
                id
            }
            _ => todo!("Pattern Matching"),
        }
    }

    fn fresh_ty_var(&mut self) -> TyId {
        let index = self.var_counter;
        self.var_counter += 1;
        self.hir
            .intern_ty(hir::TyKind::Var(TyVar::from_index(index)))
    }

    fn fresh_int_ty_var(&mut self) -> TyId {
        let index = self.var_counter;
        self.var_counter += 1;
        self.hir
            .intern_ty(hir::TyKind::IntVar(TyVar::from_index(index)))
    }

    fn show_type(&self, ty: TyId) -> String {
        let mut buf = String::new();
        self.show_type_inner(ty, &mut buf);
        buf
    }

    fn show_type_inner(&self, ty: TyId, buf: &mut String) {
        match self.hir.ty_kind(ty) {
            hir::TyKind::Int => buf.push_str("Int"),
            hir::TyKind::UInt => buf.push_str("UInt"),
            hir::TyKind::Byte => buf.push_str("Byte"),
            hir::TyKind::Float => buf.push_str("Float"),
            hir::TyKind::Bool => buf.push_str("Bool"),
            hir::TyKind::Tuple(tys) => {
                buf.push('(');
                let mut tys = tys.iter();
                if let Some(ty) = tys.next() {
                    self.show_type_inner(*ty, buf);
                }
                for ty in tys {
                    buf.push_str(", ");
                    self.show_type_inner(*ty, buf);
                }
                buf.push(')');
            }
            hir::TyKind::Func(params, result) => {
                buf.push_str("Fn(");
                let mut params = params.iter();
                if let Some(param) = params.next() {
                    if param.mutable {
                        buf.push_str("mut ");
                    }
                    self.show_type_inner(param.ty, buf);
                }
                for param in params {
                    buf.push_str(", ");
                    if param.mutable {
                        buf.push_str("mut ");
                    }
                    self.show_type_inner(param.ty, buf);
                }
                buf.push_str(") -> ");
                self.show_type_inner(*result, buf);
            }
            hir::TyKind::Named { ident, .. } => buf.push_str(&ident.str()),
            hir::TyKind::Var(_) => buf.push_str("{var}"),
            hir::TyKind::IntVar(_) => buf.push_str("{integer}"),
        }
    }

    fn err(&self, error: ErrorKind, span: Range<u32>) -> HandledError {
        self.handler.report(error, span, self.scopes.module())
    }
}
