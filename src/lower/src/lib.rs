//! Lowers a [`Hir`] into a [`Mir`].

mod exprs;

use std::mem;

use slotmap::SecondaryMap;

use errors::ErrorHandler;
use irs::{
    hir::{DefKind, ExprId, Hir, VarId},
    mir::{self, Block, BlockId, FieldAccess, Func, Local, LocalId, Mir, Place, Terminator, Value},
    ty::{DefId, DefInfo, Ty, TyContext, TyId},
};

struct LowerInfo<'ext, 'mir, 'func> {
    hir: &'ext Hir,
    ty_ctx: &'ext mut TyContext,
    handler: &'ext ErrorHandler<'ext>,
    mir: &'mir mut Mir,
    var_map: SecondaryMap<VarId, LocalId>,
    drop_stack: Vec<LocalId>,
    lambda_counter: u32,
    def: DefId,
    func: &'func mut Func,
    block: BlockId,
}

/// Lowers the provided [`Hir`] into a [`Mir`].
pub fn lower(hir: &Hir, ty_ctx: &mut TyContext, handler: ErrorHandler<'_>) -> Mir {
    let mut mir = Mir::default();

    // TODO: Lower ctors

    for def in hir.defs() {
        let func = lower_item(hir, ty_ctx, &handler, &mut mir, def);
        mir.add_func(def, func);
    }

    mir
}

fn lower_item(
    hir: &Hir,
    ty_ctx: &mut TyContext,
    handler: &ErrorHandler<'_>,
    mir: &mut Mir,
    def: DefId,
) -> Func {
    match &hir.def(def).kind {
        DefKind::Const(expr) => lower_const(hir, ty_ctx, handler, mir, def, *expr),
        DefKind::Func { params, body } => lower_func(hir, ty_ctx, handler, mir, def, params, *body),
    }
}

fn lower_func(
    hir: &Hir,
    ty_ctx: &mut TyContext,
    handler: &ErrorHandler<'_>,
    mir: &mut Mir,
    def: DefId,
    params: &[VarId],
    body: ExprId,
) -> Func {
    let mut func = Func::new(params.len(), hir.expr(body).ty);
    let mut builder = LowerInfo::new(hir, ty_ctx, handler, mir, def, &mut func);

    // Set up function parameters.
    for &param in params {
        let info = hir.var(param);
        let local = builder.func.add_local(Local {
            ident: Some(info.ident.ident),
            mutable: info.mutable,
            ty: info.ty,
        });
        builder.var_map.insert(param, local);
    }

    // Lower the function body and build the function return.
    let ret_value = builder.lower_as_value(body, false);
    builder.set_term(mir::Terminator::Return(ret_value));

    func
}

fn lower_const(
    hir: &Hir,
    ty_ctx: &mut TyContext,
    handler: &ErrorHandler<'_>,
    mir: &mut Mir,
    def: DefId,
    expr: ExprId,
) -> Func {
    let mut func = Func::new(0, hir.expr(expr).ty);
    let mut builder = LowerInfo::new(hir, ty_ctx, handler, mir, def, &mut func);

    // Lower the function body and build the function return.
    // let ret_value = builder.lower_as_value(body);
    // builder.set_term(mir::Terminator::Return(ret_value));
    todo!("Constants");

    func
}

fn lower_ctor(ty_ctx: &TyContext, ty: TyId) -> Func {
    let Ty::Named { fields, ctor, .. } = ty_ctx.get_ty(ty) else {
        unreachable!("ICE: Attempted to lower constructor for non-named type")
    };

    let mut func = Func::new(fields.len(), ty);
    let entry = func.add_block();

    let temp = func.add_temp(ty);
    for (idx, (ident, field_ty)) in fields.iter().enumerate() {
        // Add the local corresponding to the parameter for this field.
        let local = func.add_local(Local {
            ident: Some(*ident),
            mutable: false,
            ty: *field_ty,
        });
        // Construct the field place.
        let field = Place {
            base: temp,
            accesses: vec![FieldAccess { idx, ty: *field_ty }],
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

    func.block_mut(entry).term = Some(Terminator::Return(Value::Share(Place::local(temp))));

    func
}

impl<'ext, 'mir, 'func> LowerInfo<'ext, 'mir, 'func> {
    fn new(
        hir: &'ext Hir,
        ty_ctx: &'ext mut TyContext,
        handler: &'ext ErrorHandler<'ext>,
        mir: &'mir mut Mir,
        def: DefId,
        func: &'func mut Func,
    ) -> Self {
        let block = func.add_block();
        Self {
            hir,
            ty_ctx,
            handler,
            mir,
            var_map: SecondaryMap::new(),
            drop_stack: Vec::new(),
            lambda_counter: 0,
            def,
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
        self.drop_stack.push(local);
    }

    fn commit_drops(&mut self) {
        for local in mem::take(&mut self.drop_stack).into_iter().rev() {
            self.add_stmt(mir::Stmt::Drop(Place::local(local)));
        }
    }

    fn unit(&self) -> Value {
        Value::Zst(self.ty_ctx.unit())
    }

    fn def(&self) -> &DefInfo {
        self.ty_ctx.def(self.def)
    }

    fn lower_local(&mut self, var: VarId) -> LocalId {
        match self.var_map.get(var) {
            Some(var) => *var,
            None => {
                let info = self.hir.var(var);
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
