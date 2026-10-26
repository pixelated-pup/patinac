use std::iter;

use errors::Report;
use ident::Ident;
use irs::{
    hir,
    mir::{self, Item, ItemKind},
};

use crate::LowerInfo;

impl LowerInfo<'_, '_> {
    pub fn lower_expr(&mut self, expr: hir::ExprId) -> mir::ExprId {
        let new_expr = match self.hir.expr(expr) {
            hir::Expr::Var(var) => mir::Expr::Var(self.lower_var(*var)),
            hir::Expr::Lit(lit) => mir::Expr::Lit(self.lower_lit(expr, lit)),
            hir::Expr::Tuple(elems) => {
                let (field_tys, values) = elems
                    .iter()
                    .map(|&elem| (self.lower_expr_ty(elem), self.lower_expr(elem)))
                    .unzip();
                mir::Expr::Construct(field_tys, values)
            }
            &hir::Expr::Infix { op, lhs, rhs } => mir::Expr::Infix {
                op: convert_infix_op(op),
                lhs: self.lower_expr(lhs),
                rhs: self.lower_expr(rhs),
            },
            &hir::Expr::Prefix { op, expr } => mir::Expr::Prefix {
                op: convert_prefix_op(op),
                expr: self.lower_expr(expr),
            },
            &hir::Expr::Field { base, field } => {
                let hir::Ty::Named(ty) = self.expr_ty(base) else {
                    unreachable!("field access of non-record type")
                };
                let base = self.lower_expr(base);
                let field = self.field_index(*ty, field.ident);
                mir::Expr::Field { base, field }
            }
            hir::Expr::Call { func, args } => {
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
            hir::Expr::MethodCall { base, method, args } => {
                todo!("Method Lowering")
                // let method = self.hir.methods[&method.ident];
                // let hir::Ty::Func(args, ret_ty) = self.hir.var_ty(method) else {
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
            hir::Expr::Lambda {
                params,
                captures,
                body,
            } => self.lower_lambda(expr, params, captures, *body),
            &hir::Expr::Assign { place, value } => mir::Expr::Assign {
                place: self.lower_expr(place),
                value: self.lower_expr(value),
            },
            hir::Expr::If { cond, th, el } => mir::Expr::If {
                ty: self.lower_expr_ty(expr),
                cond: self.lower_expr(*cond),
                th: self.lower_block_expr(th),
                el: el.as_ref().map(|el| self.lower_block_expr(el)),
            },
            hir::Expr::For { .. } => todo!("For Loops"),
            hir::Expr::Loop(body) => mir::Expr::Loop(self.lower_block_expr(body)),
            hir::Expr::Break => todo!("Unconditional Control Flow"),
            hir::Expr::Continue => todo!("Unconditional Control Flow"),
            hir::Expr::Return(_) => todo!("Unconditional Control Flow"),
            hir::Expr::Block(block) => mir::Expr::Block(self.lower_block_expr(block)),

            &hir::Expr::Print(expr) => {
                mir::Expr::Print(self.lower_expr_ty(expr), self.lower_expr(expr))
            }
        };
        self.mir.add_expr(new_expr)
    }

    fn lower_lit(&self, expr: hir::ExprId, lit: &hir::LitExpr) -> mir::LitValue {
        match lit {
            &hir::LitExpr::Int(value) => match self.expr_ty(expr) {
                hir::Ty::Int => {
                    let i = i64::try_from(value).unwrap_or_else(|_| {
                        self.handler.report(
                            Report::warning(format!(
                                "int literal {value} overflowed and was clamped to {}",
                                i64::MAX
                            )),
                            self.hir.expr_span(expr),
                            self.module,
                        );
                        i64::MAX
                    });
                    mir::LitValue::Int(i)
                }
                hir::Ty::UInt => mir::LitValue::UInt(value),
                hir::Ty::Byte => {
                    let b = u8::try_from(value).unwrap_or_else(|_| {
                        self.handler.report(
                            Report::warning(format!(
                                "byte literal {value} overflowed and was clamped to {}",
                                u8::MAX
                            )),
                            self.hir.expr_span(expr),
                            self.module,
                        );
                        u8::MAX
                    });
                    mir::LitValue::Byte(b)
                }
                _ => unreachable!("not an int type"),
            },
            &hir::LitExpr::Float(value) => mir::LitValue::Float(value),
            hir::LitExpr::String(_) => todo!("Strings"),
            &hir::LitExpr::Bool(value) => mir::LitValue::Bool(value),
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
