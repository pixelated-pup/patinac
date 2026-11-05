//! Lowers a [`Hir`] into a [`Mir`].

mod exprs;

use std::mem;

use foldhash::fast::RandomState;
use ordermap::OrderSet;
use slotmap::SecondaryMap;

use errors::ErrorHandler;
use ident::Ident;
use irs::{
    ModuleId,
    hir::{DefItem, DefKind, ExprId, Hir, VarId},
    mir::{self, Block, BlockId, FieldAccess, Func, Local, LocalId, Mir, Place, Terminator, Value},
    ty::{Ty, TyContext, TyId},
};

struct LowerInfo<'ext, 'func> {
    hir: &'ext Hir,
    ty_ctx: &'ext TyContext,
    handler: &'ext ErrorHandler<'ext>,
    module: ModuleId,
    var_map: SecondaryMap<VarId, LocalId>,
    drop_stack: OrderSet<LocalId, RandomState>,
    lambda_counter: u32,
    func: &'func mut Func,
    block: BlockId,
}

/// Lowers the provided [`Hir`] into a [`Mir`].
pub fn lower(hir: &Hir, ty_ctx: &TyContext, handler: ErrorHandler<'_>) -> Mir {
    let mut mir = Mir::default();

    //for ty in ty_ctx.

    for item in hir.defs() {
        let item = lower_item(hir, ty_ctx, &handler, item);
        mir.add_func(item);
    }

    // if let Some(main) = self.hir.main() {
    //     let main = self.lower_item(main);
    //     self.mir.set_main(main);
    // }

    // self.mir
    todo!()
}

fn lower_item(hir: &Hir, ty_ctx: &TyContext, handler: &ErrorHandler<'_>, item: &DefItem) -> Func {
    let info = hir.var_info(item.var);
    match &item.kind {
        DefKind::Const(expr) => {
            lower_const(hir, ty_ctx, handler, info.ident.ident, *expr, item.module)
        }
        DefKind::Func { params, body } => lower_func(
            hir,
            ty_ctx,
            handler,
            info.ident.ident,
            params,
            *body,
            item.module,
        ),
    }
}

fn lower_func(
    hir: &Hir,
    ty_ctx: &TyContext,
    handler: &ErrorHandler<'_>,
    ident: Ident,
    params: &[VarId],
    body: ExprId,
    module: ModuleId,
) -> Func {
    let mut func = Func::new(ident, params.len(), hir.expr(body).ty);
    let mut builder = LowerInfo::new(hir, ty_ctx, handler, module, &mut func);

    // Set up function parameters.
    for &param in params {
        let info = hir.var_info(param);
        let local = builder.func.add_local(Local {
            ident: Some(info.ident.ident),
            mutable: info.mutable,
            ty: info.ty,
        });
        builder.var_map.insert(param, local);
    }

    // Lower the function body and build the function return.
    let ret_value = builder.lower_as_value(body);
    builder.set_term(mir::Terminator::Return(ret_value));

    func
}

fn lower_const(
    hir: &Hir,
    ty_ctx: &TyContext,
    handler: &ErrorHandler<'_>,
    ident: Ident,
    expr: ExprId,
    module: ModuleId,
) -> Func {
    let mut func = Func::new(ident, 0, hir.expr(expr).ty);
    let mut builder = LowerInfo::new(hir, ty_ctx, handler, module, &mut func);

    // Lower the function body and build the function return.
    // let ret_value = builder.lower_as_value(body);
    // builder.set_term(mir::Terminator::Return(ret_value));
    todo!();

    func
}

fn lower_ctor(ty_ctx: &TyContext, ty: TyId) -> Func {
    let Ty::Named {
        ident,
        fields,
        ctor,
        ..
    } = ty_ctx.get(ty)
    else {
        unreachable!("ICE: Attempted to lower constructor for non-named type")
    };

    let mut func = Func::new(*ident, fields.len(), ty);
    let entry = func.add_block();

    let temp = func.add_temp(ty);
    for (index, (ident, field_ty)) in fields.iter().enumerate() {
        // Add the local corresponding to the parameter for this field.
        let local = func.add_local(Local {
            ident: Some(*ident),
            mutable: false,
            ty: *field_ty,
        });
        // Construct the field place.
        let index = u32::try_from(index).expect("too many fields");
        let field = Place {
            base: temp,
            accesses: vec![FieldAccess {
                index,
                ty: *field_ty,
            }],
        };
        // Assign the parameter to the field place.
        func.add_stmt(
            entry,
            mir::Stmt::Assign {
                place: field,
                expr: mir::Expr::Val(Value::Copy(Place::local(local))),
            },
        );
    }

    func.block_mut(entry).term = Some(Terminator::Return(Value::Move(Place::local(temp))));

    func
}

impl<'ext, 'func> LowerInfo<'ext, 'func> {
    fn new(
        hir: &'ext Hir,
        ty_ctx: &'ext TyContext,
        handler: &'ext ErrorHandler<'ext>,
        module: ModuleId,
        func: &'func mut Func,
    ) -> Self {
        let block = func.add_block();
        Self {
            hir,
            ty_ctx,
            handler,
            module,
            var_map: SecondaryMap::new(),
            drop_stack: OrderSet::default(),
            lambda_counter: 0,
            func,
            block,
        }
    }

    fn block_mut(&mut self) -> &mut Block {
        self.func.block_mut(self.block)
    }

    fn add_stmt(&mut self, stmt: mir::Stmt) {
        self.block_mut().stmts.push(stmt);
    }

    fn set_term(&mut self, term: Terminator) {
        self.block_mut().term = Some(term);
    }

    fn stage_drop(&mut self, local: LocalId) {
        self.drop_stack.insert(local);
    }

    fn stage_drop_value(&mut self, value: &Value) {
        match value {
            Value::Copy(place) | Value::Move(place) => self.stage_drop(place.base),
            Value::Lit(_) | Value::Zst(_) => {}
        }
    }

    fn unstage_drop(&mut self, local: LocalId) {
        self.drop_stack.remove(&local);
    }

    fn commit_drops(&mut self) {
        for local in mem::take(&mut self.drop_stack).into_iter().rev() {
            self.add_stmt(mir::Stmt::Drop(Place::local(local)));
        }
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
}
