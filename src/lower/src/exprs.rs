use std::{iter, range::Range};

use errors::Report;
use ident::Ident;
use irs::{
    hir::{
        self, ExprId, ExprKind, LitExpr, ty::{Ty as HirTy, TyId as HirTyId},
    }, mir::{self, BlockId, FieldAccess, Item, Place, Value},
};

use crate::LowerInfo;

impl LowerInfo<'_, '_, '_> {
    pub fn lower_expr(&mut self, expr: hir::ExprId) -> Value {
        let expr = self.hir.expr(expr);
        match &expr.kind {
            ExprKind::Var(var) => mir::Expr::Var(self.lower_var(*var)),
            ExprKind::Lit(lit) => Value::Lit(self.lower_lit(lit, expr.span, expr.ty)),
            ExprKind::Tuple(elems) => {
                let (field_tys, values) = elems
                    .iter()
                    .map(|&elem| (self.lower_expr_ty(elem), self.lower_expr(elem)))
                    .unzip();
                mir::Expr::Construct(field_tys, values)
            }
            &ExprKind::Infix { op, lhs, rhs } => {
                let tmp = self.
                mir::Expr::Infix {
                    op: convert_infix_op(op),
                    lhs: self.lower_expr(lhs),
                    rhs: self.lower_expr(rhs),
                }
            },
            &ExprKind::Prefix { op, expr } => mir::Expr::Prefix {
                op: convert_prefix_op(op),
                expr: self.lower_expr(expr),
            },
            &ExprKind::Field { base, field } => {
                let HirTy::Named(ty) = self.expr_ty(base) else {
                    unreachable!("field access of non-record type")
                };
                let base = self.lower_expr(base);
                let field = self.field_index(*ty, field.ident);
                mir::Expr::Field { base, field }
            }
            ExprKind::Call { func, args } => {
                let func = self.lower_expr(*func);
                let args = args
                    .iter()
                    .map(|arg| mir::Arg {
                        ty: self.lower_expr_ty(arg.value),
                        value: self.lower_expr(arg.value),
                        mutable: arg.mutable,
                    })
                    .collect();
                let ret_ty = self.lower_expr_ty(expr);
                mir::Expr::Call { func, args, ret_ty }
            }
            ExprKind::MethodCall { base, method, args } => {
                todo!("Method Lowering")
                // let method = self.hir.methods[&method.ident];
                // let HirTy::Func(args, ret_ty) = self.hir.var_ty(method) else {
                //     unreachable!("")
                // }
                // let base = mir::Arg {
                //     ty: self.lower_expr_ty(*base),
                //     value: self.lower_expr(*base),
                //     mutable: self.hir.var_ty(method).,
                // };
                // let args = iter::once(base)
                //     .chain(args.iter().map(|arg| mir::Arg {
                //         ty: self.lower_expr_ty(arg.value),
                //         value: self.lower_expr(arg.value),
                //         mutable: arg.mutable,
                //     }))
                //     .collect();
                // let ret_ty = self.lower_expr_ty(expr);
                // mir::Expr::Call { func, args, ret_ty }
            }
            ExprKind::Lambda {
                params,
                captures,
                body,
            } => self.lower_lambda(expr, params, captures, *body),
            &ExprKind::Assign { place, value } => {
                let stmt = mir::Stmt::Assign { 
                    place: self.lower_place(place), 
                    value: self.lower_expr(value) 
                };
                todo!();
                mir::Value::Zst
            },
            ExprKind::If { cond, th, el } => mir::Expr::If {
                ty: self.lower_expr_ty(expr),
                cond: self.lower_expr(*cond),
                th: self.lower_block_expr(th),
                el: el.as_ref().map(|el| self.lower_block_expr(el)),
            },
            ExprKind::For { .. } => todo!("For Loops"),
            ExprKind::Loop(body) => mir::Expr::Loop(self.lower_block_expr(body)),
            ExprKind::Break => todo!("Unconditional Control Flow"),
            ExprKind::Continue => todo!("Unconditional Control Flow"),
            ExprKind::Return(_) => todo!("Unconditional Control Flow"),
            ExprKind::Block(block) => mir::Expr::Block(self.lower_block_expr(block)),

            &ExprKind::Print(expr) => {
                mir::Expr::Print(self.lower_expr_ty(expr), self.lower_expr(expr))
            }
        }
    }

    fn lower_place(&mut self, expr: ExprId) -> Place {
        let expr = self.hir.expr(expr);
        match &expr.kind {
            ExprKind::Var(id) => {
                let var: mir::VarId = todo!();
                Place { base: var, accesses: Vec::new() }
            },
            ExprKind::Field { base, field } => {
                let HirTy::Named { fields, .. } = self.ty_ctx.get(self.hir.expr(*base).ty) else {
                    unreachable!("field access of non-record type")
                };
                let mut place = self.lower_place(*base);
                let index = fields.get_index_of(&field.ident).expect("ICE: Missing Field").try_into().expect("ICE: Too Many Fields");
                place.accesses.push(FieldAccess {
                    index,
                    ty: self.lower_ty(expr.ty)
                });
                place
            }
            _ => unreachable!("ICE: not a place"),
        }
    }

    fn lower_lit(&self, lit: &LitExpr, span: Range<u32>, ty: HirTyId) -> mir::LitValue {
        match lit {
            &LitExpr::Int(value) => match self.ty_ctx.get(ty) {
                HirTy::Int => {
                    let i = i64::try_from(value).unwrap_or_else(|_| {
                        self.handler.report(
                            Report::warning(format!(
                                "int literal {value} overflowed and was clamped to {}",
                                i64::MAX
                            )),
                            span,
                            self.module,
                        );
                        i64::MAX
                    });
                    mir::LitValue::Int(i)
                }
                HirTy::UInt => mir::LitValue::UInt(value),
                HirTy::Byte => {
                    let b = u8::try_from(value).unwrap_or_else(|_| {
                        self.handler.report(
                            Report::warning(format!(
                                "byte literal {value} overflowed and was clamped to {}",
                                u8::MAX
                            )),
                            span,
                            self.module,
                        );
                        u8::MAX
                    });
                    mir::LitValue::Byte(b)
                }
                _ => unreachable!("not an int type"),
            },
            &LitExpr::Float(value) => mir::LitValue::Float(value),
            hir::LitExpr::String(_) => todo!("Strings"),
            &LitExpr::Bool(value) => mir::LitValue::Bool(value),
        }
    }

    fn lower_lambda(
        &mut self,
        expr: hir::ExprId,
        params: &[hir::VarId],
        captures: &[(hir::VarId, hir::VarId)],
        body: hir::ExprId,
    ) -> mir::Expr {
        // Create a distinct name for the lifted function.
        let func_name = format!("_lambda{}", self.lambda_counter);
        self.lambda_counter += 1;

        let (captures, rebindings): (Vec<_>, Vec<_>) = captures
            .iter()
            .map(|(capture, rebinding)| (self.lower_var(*capture), self.lower_var(*rebinding)))
            .collect();

        // Create a type for the environment.
        let capture_tys = captures
            .iter()
            .map(|var| self.mir.var(*var).ty.clone())
            .collect();
        let env_ty = mir::Ty::Fields(capture_tys);

        // Create the variable representing the lifted function.
        let mir::Ty::Func(mut param_tys, ret_ty) = self.lower_expr_ty(expr) else {
            unreachable!("lambda expression with non-function type")
        };
        param_tys.push(mir::Param {
            ty: env_ty.clone(),
            mutable: false,
        });
        let func_var = self.mir.add_var(mir::VarInfo {
            ident: Ident::new(&func_name),
            ty: mir::Ty::Func(param_tys, ret_ty),
            mutable: false,
        });

        // Add the environment to the parameter list.
        let env = self.mir.add_var(mir::VarInfo {
            ident: Ident::new("env"),
            ty: env_ty,
            mutable: false,
        });
        let param_vars = params
            .iter()
            .map(|var| self.lower_var(*var))
            .chain(iter::once(env))
            .collect();

        // Create the body, formed by extracting each capture from the environment before executing the original body.
        let env = self.mir.add_expr(mir::Expr::Var(env));
        let mut stmts: Vec<_> = rebindings
            .iter()
            .enumerate()
            .map(|(index, var)| {
                let field = u32::try_from(index).expect("too many captures");
                let value = self.mir.add_expr(mir::Expr::Field { base: env, field });
                mir::Stmt::Decl { var: *var, value }
            })
            .collect();
        stmts.push(mir::Stmt::Expr(self.lower_expr(body)));
        let body = self.mir.add_expr(mir::Expr::Block(mir::BlockExpr(stmts)));

        self.mir.add_item(Item {
            var: func_var,
            kind: ItemKind::Func {
                params: param_vars,
                body,
            },
        });

        mir::Expr::Closure {
            func: func_var,
            captures,
        }
    }

    fn lower_block_expr(&mut self, block: &hir::BlockExpr) -> mir::BlockExpr {
        let stmts = block
            .stmts
            .iter()
            .map(|stmt| match stmt {
                hir::Stmt::Decl { var, value, .. } => mir::Stmt::Decl {
                    var: self.lower_var(*var),
                    value: self.lower_expr(*value),
                },
                hir::Stmt::Expr(expr) => mir::Stmt::Expr(self.lower_expr(*expr)),
            })
            .collect();
        mir::BlockExpr(stmts)
    }
}

macro_rules! convert_op {
    ($op:ident, $enum_name:ident, $($variant:ident),*) => {
        match $op {
            $(hir::$enum_name::$variant => mir::$enum_name::$variant),*
        }
    };
}

const fn convert_prefix_op(op: hir::PrefixOp) -> mir::PrefixOp {
    convert_op!(op, PrefixOp, Not, Neg)
}

const fn convert_infix_op(op: hir::InfixOp) -> mir::InfixOp {
    convert_op!(
        op, InfixOp, Add, AddF, Sub, SubF, Mul, MulF, Div, DivF, Exp, And, Or, Eqq, Neq, Gt, Lt,
        Geq, Leq
    )
}
