//! Lowers all [`Asts`][Ast] into a single [`Hir`], resolving stringly variable and type names into numeric identifiers along the way.

mod error;
mod exprs;
mod scope;

use std::range::Range;

use itertools::Itertools;
use slotmap::SecondaryMap;

use errors::{ErrorHandler, HandledError, Result, TryCollectEager as _};
use irs::{
    ModuleId, Package,
    ast::{self, Ast, Binding, Import, Pat, PatKind, TyItem, TyItemKind, TyKind},
    hir::{self, Field, Hir, Param, TyId, VarId, VarInfo},
};

use crate::{
    error::{ErrorKind, ItemKind},
    scope::{ScopeInfo, Visibility},
};

struct ResolveInfo<'pkg, 'err> {
    package: &'pkg Package,
    asts: &'pkg SecondaryMap<ModuleId, Ast>,
    handler: ErrorHandler<'err>,
    hir: Hir,
    scopes: ScopeInfo<'pkg>,
}

/// Resolves and lowers the provided [`Package`] into a single [`Hir`].
///
/// # Errors
/// Returns an error if there are any unbound variables, undefined types, or multiple items with the same name.
pub fn resolve(
    package: &Package,
    asts: &SecondaryMap<ModuleId, Ast>,
    handler: ErrorHandler,
) -> Result<Hir> {
    let mut info = ResolveInfo {
        package,
        asts,
        handler,
        hir: Hir::default(),
        scopes: ScopeInfo::new(package),
    };
    info.resolve_module(package.root(), true);
    info.handler.checked(info.hir)
}

impl ResolveInfo<'_, '_> {
    fn populate_scope(&mut self, module: ModuleID, is_root: bool) {
        for &child in &self.package.get(module).children {
            self.resolve_module(child, false);
        }

        self.scopes.set_module(module);

        let ast = &self.asts[module];

        let mut ty_ids = Vec::new();
        for ty in &ast.ty_items {
            let id = self.hir.fresh_var();
            let vis = if ty.public {
                Visibility::Public
            } else {
                Visibility::Private
            };
            if self.scopes.add_ty(vis, ty.ident.ident, id).is_some() {
                self.err(
                    ErrorKind::DuplicateItem(ItemKind::Type, ty.ident.ident),
                    ty.ident.span,
                );
            }
            ty_ids.push((ty, id));
        }

        let mut def_ids = Vec::new();
        for impl_ in &ast.impls {
            for def in &impl_.defs {
                let Ok(ty) = self.resolve_ty(&impl_.ty) else {
                    continue;
                };
                let id = self.hir.add_var(VarInfo {
                    ident: def.ident,
                    mutable: false,
                    global: true,
                    module: self.scopes.module(),
                });
                let vis = if def.public {
                    Visibility::Public
                } else {
                    Visibility::Private
                };
                if self
                    .scopes
                    .add_assoc_def(ty, vis, def.ident.ident, id)
                    .is_some()
                {
                    self.err(
                        ErrorKind::DuplicateItem(ItemKind::Value, def.ident.ident),
                        def.ident.span,
                    );
                }
                def_ids.push((def, id));
            }
        }
        for def in &ast.defs {
            let id = self.hir.add_var(VarInfo {
                ident: def.ident,
                mutable: false,
                global: true,
                module: self.scopes.module(),
            });
            let vis = if def.public {
                Visibility::Public
            } else {
                Visibility::Private
            };
            if self.scopes.add_def(vis, def.ident.ident, id).is_some() {
                self.err(
                    ErrorKind::DuplicateItem(ItemKind::Value, def.ident.ident),
                    def.ident.span,
                );
            }
            def_ids.push((def, id));
        }

        for Import(path, span) in &ast.imports {
            if let Err(e) = self.scopes.import(path.as_slice()) {
                self.err(e, *span);
            }
        }
    }

    fn resolve_module(&mut self, module: ModuleId, is_root: bool) {
        for &child in &self.package.get(module).children {
            self.resolve_module(child, false);
        }

        self.scopes.set_module(module);

        let ast = &self.asts[module];

        let mut ty_ids = Vec::new();
        for ty in &ast.ty_items {
            let id = self.hir.fresh_var();
            let vis = if ty.public {
                Visibility::Public
            } else {
                Visibility::Private
            };
            if self.scopes.add_ty(vis, ty.ident.ident, id).is_some() {
                self.err(
                    ErrorKind::DuplicateItem(ItemKind::Type, ty.ident.ident),
                    ty.ident.span,
                );
            }
            ty_ids.push((ty, id));
        }
        for (ty, id) in ty_ids {
            self.resolve_ty_item(ty, id);
        }

        let mut def_ids = Vec::new();
        for impl_ in &ast.impls {
            for def in &impl_.defs {
                let Ok(ty) = self.resolve_ty(&impl_.ty) else {
                    continue;
                };
                let id = self.hir.add_var(VarInfo {
                    ident: def.ident,
                    mutable: false,
                    global: true,
                    module: self.scopes.module(),
                });
                let vis = if def.public {
                    Visibility::Public
                } else {
                    Visibility::Private
                };
                if self
                    .scopes
                    .add_assoc_def(ty, vis, def.ident.ident, id)
                    .is_some()
                {
                    self.err(
                        ErrorKind::DuplicateItem(ItemKind::Value, def.ident.ident),
                        def.ident.span,
                    );
                }
                def_ids.push((def, id));
            }
        }
        for def in &ast.defs {
            let id = self.hir.add_var(VarInfo {
                ident: def.ident,
                mutable: false,
                global: true,
                module: self.scopes.module(),
            });
            let vis = if def.public {
                Visibility::Public
            } else {
                Visibility::Private
            };
            if self.scopes.add_def(vis, def.ident.ident, id).is_some() {
                self.err(
                    ErrorKind::DuplicateItem(ItemKind::Value, def.ident.ident),
                    def.ident.span,
                );
            }
            def_ids.push((def, id));
        }

        for Import(path, span) in &ast.imports {
            if let Err(e) = self.scopes.import(path.as_slice()) {
                self.err(e, *span);
            }
        }

        for (ast_def, id) in def_ids {
            if let Ok(def) = self.resolve_def_item(ast_def, id) {
                if is_root && self.is_main(ast_def).is_ok_and(|x| x) {
                    self.hir.set_main(def);
                } else {
                    self.hir.add_def(def);
                }
            }
        }
    }

    fn is_main(&self, def: &ast::DefItem) -> Result<bool> {
        if let ast::DefKind::Func { params, ret_ty, .. } = &def.kind
            && def.ident.ident == "main"
        {
            if params.is_empty() && ret_ty.kind == ast::TyKind::unit() {
                Ok(true)
            } else {
                Err(self.err(ErrorKind::InvalidMain, def.ident.span))
            }
        } else {
            Ok(false)
        }
    }

    fn resolve_ty_item(&mut self, item: &TyItem, id: TyId) {
        if !item.generics.is_empty() {
            todo!("Generics")
        }

        // let id = self
        //     .scopes
        //     .resolve_ty(PathSlice::single(item.ident.ident))
        //     .expect("all items should have already been inserted into the scope");

        match &item.kind {
            TyItemKind::Record(old_fields) => {
                let mut fields = Vec::new();
                for old_field in old_fields {
                    let Ok(ty) = self.resolve_ty(&old_field.ty) else {
                        continue;
                    };
                    let field = Field {
                        ident: old_field.ident,
                        ty,
                    };
                    fields.push(field);
                }
                for dup in fields.iter().duplicates_by(|field| field.ident.ident) {
                    self.err(ErrorKind::DupFields(dup.ident.ident), dup.ident.span);
                }

                let constructor_ty = hir::Ty::Func(
                    fields
                        .iter()
                        .map(|field| Param {
                            ty: field.ty.clone(),
                            mutable: false,
                            span: field.ident.span,
                        })
                        .collect(),
                    Box::new(hir::Ty::Named(id)),
                );
                let ctor = self.hir.add_var(VarInfo {
                    ident: item.ident,
                    mutable: false,
                    global: true,
                    module: self.scopes.module(),
                });
                let ctor_vis = if item.opaque || !item.public {
                    Visibility::Private
                } else {
                    Visibility::Public
                };
                self.hir.add_var_ty(ctor, constructor_ty);
                self.scopes.add_def(ctor_vis, item.ident.ident, ctor);

                self.hir.fulfill_ty(
                    id,
                    TyInfo {
                        opaque: item.opaque,
                        fields,
                        ctor,
                        module: self.scopes.module(),
                    },
                );
            }
            TyItemKind::Union(_) => {
                todo!("Unions");
            }
        }
    }

    // fn resolve_impl(&mut self, impl_: &ImplItem) {
    //     let Ok(ty) = self.resolve_ty(&impl_.ty) else {
    //         return;
    //     };

    //     for def in &impl_.defs {
    //         if let Ok(def) = self.resolve_def_item(def) {
    //             self.hir.add_def(def);
    //         }
    //     }
    // }

    fn resolve_def_item(&mut self, def: &ast::DefItem, id: VarId) -> Result<hir::DefItem> {
        if !def.generics.is_empty() {
            todo!("Generics")
        }

        match &def.kind {
            ast::DefKind::Const { ty, val } => {
                let val = self.resolve_expr(val);
                let ty = self.resolve_ty(ty)?;
                self.hir.add_var_ty(id, ty);

                Ok(hir::DefItem {
                    module: self.scopes.module(),
                    var: id,
                    kind: hir::DefKind::Const(val?),
                })
            }
            ast::DefKind::Func {
                self_param,
                params,
                ret_ty,
                body,
            } => {
                self.scopes.push_scope();

                let self_param = self_param.map(|(mutable, span)| {
                    todo!("Methods")
                    // let Some(ty) = parent_ty else {
                    //     return Err(self.err(ErrorKind::SelfOutsideImpl, span));
                    // };
                    // let id = self.resolve_pat(
                    //     &ast::PatKind::Ident(Ident::new("self")).span((span.end - 4)..span.end),
                    //     mutable,
                    //     Some(hir::Ty::Named(ty)),
                    // );
                    // Ok((
                    //     id,
                    //     Param {
                    //         ty: hir::Ty::Named(ty),
                    //         mutable,
                    //         span,
                    //     },
                    // ))
                });

                let params = self_param
                    .into_iter()
                    .chain(params.iter().map(|p| {
                        let ty = self.resolve_ty(&p.ty)?;
                        let id = self.resolve_pat(&p.pat, p.mutable, ty);
                        Ok((
                            id,
                            Param {
                                ty,
                                mutable: p.mutable,
                            },
                        ))
                    }))
                    .try_collect_eager();

                let body = self.resolve_expr(body);

                self.scopes.pop_scope();

                let ret_ty = self.resolve_ty(ret_ty)?;
                let (params, param_tys) = params?;

                self.hir
                    .add_var_ty(id, hir::Ty::Func(param_tys, Box::new(ret_ty)));

                Ok(hir::DefItem {
                    module: self.scopes.module(),
                    var: id,
                    kind: hir::DefKind::Func {
                        params,
                        body: body?,
                    },
                })
            }
        }
    }

    fn resolve_binding(&mut self, binding: &Binding) -> Result<VarId> {
        let ty = match binding.ty.as_ref() {
            None => self.hir.fresh_var(),
            Some(ty) => self.resolve_ty(ty)?,
        };
        Ok(self.resolve_pat(&binding.pat, binding.mutable, ty))
    }

    fn resolve_ty(&mut self, ty: &ast::Ty) -> Result<hir::TyId> {
        match &ty.kind {
            TyKind::Tuple(tys) => {
                let ty = hir::Ty::Tuple(self.resolve_tys(tys)?);
                Ok(self.hir.intern_ty(ty))
            }
            TyKind::Func(params, ret_ty) => {
                let params = params
                    .iter()
                    .map(|param| {
                        Ok(Param {
                            ty: self.resolve_ty(&param.ty)?,
                            mutable: param.mutable,
                        })
                    })
                    .try_collect_eager();
                let ret_ty = self.resolve_ty(ret_ty)?;
                Ok(self.hir.intern_ty(hir::Ty::Func(params?, ret_ty)))
            }
            TyKind::Named(path, args) => {
                if !args.is_empty() {
                    todo!("Generics")
                }

                let prim_ty = match &*path.last().str() {
                    "Int" => Some(hir::Ty::Int),
                    "UInt" => Some(hir::Ty::UInt),
                    "Byte" => Some(hir::Ty::Byte),
                    "Bool" => Some(hir::Ty::Bool),
                    "Float" => Some(hir::Ty::Float),
                    _ => None,
                };

                match prim_ty {
                    Some(ty) => Ok(self.hir.intern_ty(ty)),
                    None => self
                        .scopes
                        .resolve_ty(path.as_slice())
                        .map_err(|e| self.err(e, ty.span)),
                }
            }
        }
    }

    fn resolve_tys(&mut self, tys: &[ast::Ty]) -> Result<Vec<hir::TyId>> {
        tys.iter().map(|ty| self.resolve_ty(ty)).try_collect_eager()
    }

    fn resolve_pat(&mut self, pat: &Pat, mutable: bool, ty: hir::TyId) -> VarId {
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

    fn err(&self, error: ErrorKind, span: Range<u32>) -> HandledError {
        self.handler.report(error, span, self.scopes.module())
    }
}
