//! Lowers a [`Hir`] into a [`Mir`].

mod exprs;

use std::cmp::Reverse;

use slotmap::{Key as _, SecondaryMap};

use errors::ErrorHandler;
use ident::Ident;
use irs::{
    ModuleId,
    hir::{self, DefItem, DefKind, ExprId, Hir, VarId},
    mir::{self, Block, BlockId, Func, Item, ItemId, Local, LocalId, Mir, Terminator},
    ty::{Ty, TyContext, TyId},
};

struct LowerInfo<'ext, 'func> {
    hir: &'ext Hir,
    ty_ctx: &'ext TyContext,
    handler: ErrorHandler<'ext>,
    var_map: SecondaryMap<hir::VarId, mir::LocalId>,
    module: ModuleId,
    lambda_counter: u32,
    func: &'func mut Func,
    block: BlockId,
}

/// Lowers the provided [`Hir`] into a [`Mir`].
pub fn lower(hir: &Hir, ty_ctx: &TyContext, handler: ErrorHandler<'_>) -> Mir {
    // for id in self.hir.tys() {
    //     self.lower_ctor(id);
    // }

    // for item in self.hir.defs() {
    //     let item = self.lower_item(item);
    //     self.mir.add_item(item);
    // }

    // if let Some(main) = self.hir.main() {
    //     let main = self.lower_item(main);
    //     self.mir.set_main(main);
    // }

    // self.mir
    todo!()
}

fn lower_func(
    hir: &Hir,
    ty_ctx: &TyContext,
    handler: ErrorHandler<'_>,
    params: Vec<VarId>,
    body: ExprId,
    module: ModuleId,
) -> Func {
    let mut func = Func::new(params.len(), hir.expr(body).ty);

    // Set up function parameters.
    let mut var_map = SecondaryMap::new();
    for param in params {
        let info = hir.var_info(param);
        let local = func.add_local(Local {
            ident: Some(info.ident.ident),
            mutable: info.mutable,
            ty: info.ty,
        });
        var_map.insert(param, local);
    }

    let entry = func.add_block();

    let mut builder = LowerInfo {
        hir,
        ty_ctx,
        handler,
        var_map,
        module,
        lambda_counter: 0,
        func: &mut func,
        block: entry,
    };

    // Lower the function body and build the function return.
    let ret_value = builder.lower_as_value(body);
    builder.set_term(mir::Terminator::Return(ret_value));

    func
}

impl LowerInfo<'_, '_> {
    fn block_mut(&mut self) -> &mut Block {
        self.func.block_mut(self.block)
    }

    fn add_stmt(&mut self, stmt: mir::Stmt) {
        self.block_mut().stmts.push(stmt);
    }

    fn set_term(&mut self, term: Terminator) {
        self.block_mut().term = Some(term);
    }

    fn lower_local(&mut self, var: VarId) -> LocalId {
        match self.var_map.get(var) {
            Some(var) => *var,
            None => {
                let info = self.hir.var_info(var);
                assert!(
                    !info.global,
                    "Variables representing top-level items should not be lowered to MIR locals"
                );
                let local = self.func.add_local(Local {
                    ident: Some(info.ident.ident),
                    ty: info.ty,
                    mutable: info.mutable,
                });
                self.var_map.insert(var, local);
                local
            }
        }
    }

    fn lower_global(&mut self, var: VarId) -> ItemId {
        let info = self.hir.var_info(var);
        assert!(
            info.global,
            "Only variables representing top-level items should be lowered to MIR globals"
        );

        todo!()
    }

    fn lower_ctor(&mut self, ty: TyId) {
        let var = self.lower_local(self.hir.ty_info(ty).ctor);

        let field_tys = self.layout_record_fields(ty);
        let (params, values) = field_tys
            .iter()
            .map(|ty| {
                let var = self.mir.add_var(Local {
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
}
