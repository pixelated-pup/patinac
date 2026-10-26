//! Lowers a [`Hir`] into a [`Mir`].
//! This involves the following:
//! - Unifying records and tuples, and rearranging their fields for best packing.
//! - Lowering field access to a numeric field number.
//! - Seperating integer literals into specific types, and emitting warnings if the literal overflows the type.

mod exprs;

use std::cmp::Reverse;

use slotmap::{Key as _, SecondaryMap};

use errors::ErrorHandler;
use ident::Ident;
use irs::{
    ModuleId,
    hir::{self, DefItem, DefKind, Hir, ty::TyId},
    mir::{self, Item, ItemKind, Mir, VarInfo},
};

struct LowerInfo<'hir, 'err> {
    hir: &'hir Hir,
    expr_tys: &'hir SecondaryMap<hir::ExprId, hir::Ty>,
    handler: ErrorHandler<'err>,
    mir: Mir,
    var_map: SecondaryMap<hir::VarId, mir::VarId>,
    module: ModuleId,
    lambda_counter: u32,
}

/// Resolves and lowers the provided [`Package`] into a single [`Hir`].
///
/// # Errors
/// Returns an error if there are any unbound variables, undefined types, or multiple items with the same name.
pub fn lower(
    handler: ErrorHandler,
    hir: &Hir,
    expr_tys: &SecondaryMap<hir::ExprId, hir::Ty>,
) -> Mir {
    LowerInfo {
        hir,
        expr_tys,
        handler,
        mir: Mir::default(),
        var_map: SecondaryMap::new(),
        module: ModuleId::null(),
        lambda_counter: 0,
    }
    .lower()
}

impl<'hir> LowerInfo<'hir, '_> {
    fn lower(mut self) -> Mir {
        // Build constructors, populating the field map along the way.
        for id in self.hir.tys() {
            self.lower_ctor(id);
        }

        for item in self.hir.execs() {
            let item = self.lower_item(item);
            self.mir.add_item(item);
        }

        if let Some(main) = self.hir.main() {
            let main = self.lower_item(main);
            self.mir.set_main(main);
        }

        self.mir
    }

    fn lower_var(&mut self, var: hir::VarId) -> mir::VarId {
        match self.var_map.get(var) {
            Some(var) => *var,
            None => {
                let var_info = self.hir.var_info(var);
                let ty = self.lower_ty(self.hir.var_ty(var));
                let new_var = self.mir.add_var(VarInfo {
                    ident: var_info.ident.ident,
                    ty,
                    mutable: var_info.mutable,
                });
                self.var_map.insert(var, new_var);
                new_var
            }
        }
    }

    fn lower_ctor(&mut self, ty: TyId) {
        let var = self.lower_var(self.hir.ty_info(ty).ctor);

        let field_tys = self.layout_record_fields(ty);
        let (params, values) = field_tys
            .iter()
            .map(|ty| {
                let var = self.mir.add_var(VarInfo {
                    ident: Ident::new("f"),
                    ty: ty.clone(),
                    mutable: false,
                });
                let expr = self.mir.add_expr(mir::Expr::Var(var));
                (var, expr)
            })
            .unzip();
        let body = self.mir.add_expr(mir::Expr::Construct(field_tys, values));

        self.mir.add_item(Item {
            var,
            kind: ItemKind::Func { params, body },
        });
    }

    fn lower_item(&mut self, item: &DefItem) -> Item {
        self.module = item.module;
        let kind = match &item.kind {
            DefKind::Const(val) => ItemKind::Const(self.lower_expr(*val)),
            DefKind::Func { params, body } => {
                let params = params.iter().map(|var| self.lower_var(*var)).collect();
                let body = self.lower_expr(*body);
                ItemKind::Func { params, body }
            }
        };
        Item {
            var: self.lower_var(item.var),
            kind,
        }
    }

    fn expr_ty(&self, expr: hir::ExprId) -> &'hir hir::Ty {
        &self.expr_tys[expr]
    }

    fn lower_ty(&self, ty: &'hir hir::Ty) -> mir::Ty {
        match ty {
            hir::Ty::Int => mir::Ty::Int,
            hir::Ty::UInt => mir::Ty::UInt,
            hir::Ty::Byte => mir::Ty::Byte,
            hir::Ty::Float => mir::Ty::Float,
            hir::Ty::Bool => mir::Ty::Bool,
            hir::Ty::Tuple(elem_tys) => mir::Ty::Fields(self.layout_fields(elem_tys)),
            hir::Ty::Func(params, ret_ty) => {
                let params = params
                    .iter()
                    .map(|param| mir::Param {
                        ty: self.lower_ty(&param.ty),
                        mutable: param.mutable,
                    })
                    .collect();
                let ret_ty = Box::new(self.lower_ty(ret_ty));
                mir::Ty::Func(params, ret_ty)
            }
            hir::Ty::Named(id) => mir::Ty::Fields(self.layout_record_fields(*id)),
        }
    }

    fn layout_fields(&self, field_tys: impl IntoIterator<Item = &'hir hir::Ty>) -> Vec<mir::Ty> {
        field_tys.into_iter().map(|ty| self.lower_ty(ty)).collect()
    }

    fn layout_record_fields(&self, ty: TyId) -> Vec<mir::Ty> {
        self.hir
            .ty_info(ty)
            .fields
            .iter()
            .map(|field| self.lower_ty(&field.ty))
            .collect()
    }

    fn lower_expr_ty(&self, expr: hir::ExprId) -> mir::Ty {
        self.lower_ty(self.expr_ty(expr))
    }

    fn field_index(&self, ty: TyId, ident: Ident) -> u32 {
        self.hir
            .ty_info(ty)
            .fields
            .iter()
            .position(|field| field.ident.ident == ident)
            .expect("type does not have that field")
            .try_into()
            .expect("too many fields")
    }
}
