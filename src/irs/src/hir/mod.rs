//! The high-level intermediate representation of Patina. Produced after name resolution, and used for typechecking.

pub mod ty;

use std::range::Range;

use slotmap::{SlotMap, new_key_type};

use ident::SpanIdent;

use crate::ModuleId;
use ty::TyId;

/// An AST-like structure, with additional metadata in the form of type information and resolved variable identifiers.
#[derive(Debug, Default)]
pub struct Hir {
    main: Option<DefItem>,
    defs: Vec<DefItem>,

    exprs: SlotMap<ExprId, Expr>,
    vars: SlotMap<VarId, VarInfo>,
}

impl Hir {
    /// Returns all of the "executable items". These are the items that contain expressions.
    pub fn defs(&self) -> &[DefItem] {
        &self.defs
    }

    /// Adds an "executable item". These are the items that contain expressions.
    pub fn add_def(&mut self, exec: DefItem) {
        self.defs.push(exec);
    }

    /// Returns the main function, if it exists.
    pub const fn main(&self) -> Option<&DefItem> {
        self.main.as_ref()
    }

    /// Sets the main function.
    pub fn set_main(&mut self, main: DefItem) {
        self.main = Some(main);
    }
}

// Expr-related functions
impl Hir {
    /// Adds a spanned expression, returning a unique ID for it.
    pub fn add_expr(
        &mut self,
        kind: ExprKind,
        ty: TyId,
        span: impl Into<Range<u32>>,
        module: ModuleId,
    ) -> ExprId {
        self.exprs.insert(Expr {
            kind,
            ty,
            span: span.into(),
            module,
        })
    }

    /// Returns the expression associated with the given ID.
    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id]
    }

    /// Returns an iterator over all the expressions.
    pub fn exprs(&self) -> impl Iterator<Item = &Expr> {
        self.exprs.values()
    }
}

// Var-related functions
impl Hir {
    /// Adds a variable with the provided information, returning a unique ID for it.
    pub fn add_var(&mut self, info: VarInfo) -> VarId {
        self.vars.insert(info)
    }

    /// Returns the information for the given variable.
    pub fn var_info(&self, id: VarId) -> VarInfo {
        self.vars[id]
    }

    /// Returns an iterator over all the variables.
    pub fn vars(&self) -> impl Iterator<Item = &VarInfo> {
        self.vars.values()
    }
}

/// An "executable item". These are the items that contain expressions, namely constants and functions.
#[derive(Debug, PartialEq, Eq)]
pub struct DefItem {
    /// The variable the item is associated with.
    pub var: VarId,
    /// The kind of the item (constant or function).
    pub kind: DefKind,
    /// The module containing the item's definition.
    pub module: ModuleId,
}

/// The information of an [`ExecItem`] specific to whether it's a constant or function.
#[derive(Debug, PartialEq, Eq)]
pub enum DefKind {
    /// A constant item.
    Const(ExprId),
    /// A function item.
    Func {
        /// The value parameters.
        params: Vec<VarId>,
        /// The body.
        body: ExprId,
    },
}

new_key_type! {
    /// An ID representing a variable. Information about the variable can be acquired through [`Hir::var_info`].
    pub struct VarId;
}
/// Information for a variable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarInfo {
    /// The name of the variable, with span.
    pub ident: SpanIdent,
    /// Whether the variable was declared mutable.
    pub mutable: bool,
    /// The type of the variable. May be `Var` prior to inference.
    pub ty: TyId,
    /// Whether the variable is global (i.e. represents an [`ExecItem`]).
    pub global: bool,
    /// The module containing the variable's declaration.
    pub module: ModuleId,
}

new_key_type! {
    /// An ID corresponding to an [`Expr`].
    pub struct ExprId;
}

/// An expression, with type and span information.
#[derive(Debug, PartialEq)]
pub struct Expr {
    /// The underlying kind of expression.
    pub kind: ExprKind,
    /// The type of the expression.
    pub ty: TyId,
    /// The source span of the expression.
    pub span: Range<u32>,
    /// The module containing the expression.
    pub module: ModuleId,
}

/// The kinds of expressions.
#[derive(Debug, PartialEq)]
pub enum ExprKind {
    /// A reference to a named value.
    Var(VarId),
    /// A scalar literal value. The specific kinds of literals are represented by [`LitExpr`].
    Lit(LitExpr),
    /// A tuple literal.
    Tuple(Vec<ExprId>),
    /// An infix operation.
    Infix {
        /// The infix operator used.
        op: InfixOp,
        /// The left-hand side of the operation.
        lhs: ExprId,
        /// The right-hand side of the operation.
        rhs: ExprId,
    },
    /// A prefix operation.
    Prefix {
        /// The prefix operator used.
        op: PrefixOp,
        /// The base expression the operator is applied to.
        expr: ExprId,
    },
    /// Record field access.
    Field {
        /// The base expression from which the field is being accessed.
        base: ExprId,
        /// The name of the field being accessed.
        field: SpanIdent,
    },
    /// A function call.
    Call {
        /// The function being called.
        func: ExprId,
        /// The list of arguments being applied.
        args: Vec<Arg>,
    },
    /// A method call.
    MethodCall {
        /// The base expression that the method is being called on.
        base: ExprId,
        /// The name of the method being called.
        method: SpanIdent,
        /// The list of arguments being applied.
        args: Vec<Arg>,
    },
    /// A capturing lambda.
    Lambda {
        /// The parameters of the function.
        params: Vec<VarId>,
        /// List of external capture - local rebinding pairs.
        captures: Vec<(VarId, VarId)>,
        /// The body of the function.
        body: ExprId,
    },
    /// A variable assignment.
    Assign {
        /// The place being assigned to, on the left of the `=`. Semantically, this must be a "place expression".
        place: ExprId,
        /// The value being assigned, on the right of the `=`.
        value: ExprId,
    },
    /// An if-then, with an optional else branch.
    If {
        /// The condition of the if.
        cond: ExprId,
        /// The "then" block.
        th: BlockExpr,
        /// The "else" block, if there is one.
        el: Option<BlockExpr>,
    },
    /// A loop over the elements of an iterator.
    For {
        /// The variable to bind each element to.
        id: VarId,
        /// The iterator to be iterated over.
        iter: ExprId,
        /// The body to execute for each element.
        body: BlockExpr,
    },
    /// An infinite loop.
    Loop(BlockExpr),
    /// Break, which terminates the enclosing [`loop`][Self::Loop] or [`for loop`][Self::For] early.
    Break,
    /// Continue, which skips to the next iteration of the enclosing [`loop`][Self::Loop] or [`for loop`][Self::For].
    Continue,
    /// Return, which returns early from the enclosing function.
    Return(ExprId),
    /// A block, which executes each contained statement sequentially.
    ///
    /// Blocks evaluate to the value of the last statement, or unit if the last statement is not an expression.
    Block(BlockExpr),

    /// TEMPORARY, until we have stdlib + FFI.
    Print(ExprId),
}

/// The kinds of [literal expressions][Expr::Lit].
#[derive(Debug, PartialEq)]
pub enum LitExpr {
    /// An integer, of any of the [three][crate::Ty::Int] [integer][crate::Ty::UInt] [types][crate::Ty::Byte]. Sign is not part of the literal.
    Int(u64),
    /// A float. Can include exponent.
    Float(f64),
    /// A string. Common escape sequences and raw strings are supported.
    String(String),
    /// A boolean.
    Bool(bool),
}

/// An argument in a [ function call][Expr::Call], consisting of an expression that may have a mutability annotation.
#[derive(Debug, PartialEq, Eq)]
pub struct Arg {
    /// The value of the function argument.
    pub value: ExprId,
    /// Whether this argument is mutable. If it is, the value must be a place expression.
    pub mutable: bool,
    /// The total span of the argument.
    /// If `mutable` is false, this should be identical to the span of `val`.
    /// Otherwise, it should include the span of the `mut` keyword.
    pub span: Range<u32>,
}

/// A block of statements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockExpr {
    /// The statements within the block.
    pub stmts: Vec<Stmt>,
    /// The total span of the block, from opening to closing brace.
    pub span: Range<u32>,
}

/// A statement. Always contained within a [`BlockExpr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stmt {
    /// A variable declaration.
    Decl {
        /// The the variable being bound.
        var: VarId,
        /// The initial value for the variable.
        value: ExprId,
        /// The span of the declaration, starting from the `let` and ending after the [`value`][`Stmt::Decl::value`].
        span: Range<u32>,
    },
    /// An expression used as a statement, evaluated purely for side-effects.
    Expr(ExprId),
}

/// An infix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfixOp {
    /// `+`.
    Add,
    /// TEMPORARY, until we have traits. `+.`.
    AddF,
    /// `-`.
    Sub,
    /// TEMPORARY, until we have traits. `-.`.
    SubF,
    /// `*`.
    Mul,
    /// TEMPORARY, until we have traits. `*.`.
    MulF,
    /// `/`.
    Div,
    /// TEMPORARY, until we have traits. `/.`.
    DivF,
    /// `^`.
    Exp,
    /// `&&`.
    And,
    /// `||`.
    Or,
    /// `==`.
    Eqq,
    /// `!=`.
    Neq,
    /// `>`.
    Gt,
    /// `<`.
    Lt,
    /// `>=`.
    Geq,
    /// `<=`.
    Leq,
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixOp {
    /// `!`. Logical negation.
    Not,
    /// `-`. Numeric negation.
    Neg,
}
