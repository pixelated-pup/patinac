//! The high-level intermediate representation of Patina. Produced after name resolution, and used for typechecking.

use std::range::Range;

use ena::unify::{EqUnifyValue, UnifyKey};
use indexmap::IndexSet;
use slotmap::{SecondaryMap, SlotMap, new_key_type};

use ident::{Ident, SpanIdent};

use super::ModuleId;

/// An AST-like structure, with additional metadata in the form of type information and resolved variable identifiers.
#[derive(Debug, Default)]
pub struct Hir {
    main: Option<DefItem>,
    defs: Vec<DefItem>,
    tys: IndexSet<TyKind, foldhash::fast::RandomState>,
    exprs: SlotMap<ExprId, (Expr, Range<u32>, TyId)>,
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

// Type-related functions
impl Hir {
    /// Returns an iterator over all user-defined types.
    // pub fn tys(&self) -> impl Iterator<Item = TyId> {
    //     self.tys.keys()
    // }

    pub fn intern_ty(&mut self, kind: TyKind) -> TyId {
        match self.tys.get_index_of(&kind) {
            Some(index) => TyId(index),
            None => {
                let index = self.tys.len();
                self.tys.insert(kind);
                TyId(index)
            }
        }
    }

    /// Shorthand for `self.intern_ty(TyKind::Int)`
    pub fn int(&mut self) -> TyId {
        self.intern_ty(TyKind::Int)
    }

    /// Shorthand for `self.intern_ty(TyKind::UInt)`
    pub fn uint(&mut self) -> TyId {
        self.intern_ty(TyKind::UInt)
    }

    /// Shorthand for `self.intern_ty(TyKind::Byte)`
    pub fn byte(&mut self) -> TyId {
        self.intern_ty(TyKind::Byte)
    }

    /// Shorthand for `self.intern_ty(TyKind::Float)`
    pub fn float(&mut self) -> TyId {
        self.intern_ty(TyKind::Float)
    }

    /// Shorthand for `self.intern_ty(TyKind::Bool)`
    pub fn bool(&mut self) -> TyId {
        self.intern_ty(TyKind::Bool)
    }

    /// Shorthand for `self.intern_ty(TyKind::unit())`
    pub fn unit(&mut self) -> TyId {
        self.intern_ty(TyKind::unit())
    }

    pub fn ty_kind(&self, id: TyId) -> &TyKind {
        self.tys.get_index(id.0).expect("Invalid ID")
    }

    // /// Reserves a user-defined type, storing only it's name and returning a unique ID for it.
    // ///
    // /// This allows getting an ID for a type before having the full information for it.
    // /// The reservation should be fullfilled with [`fulfill_ty`][Self::fulfill_ty] as soon as possible.
    // pub fn reserve_ty(&mut self, ident: SpanIdent) -> TyId {
    //     self.tys.insert(ident)
    // }

    // /// Fulfills a previously-reserved user-defined type, providing the rest of the information for the type.
    // ///
    // /// See [`reserve_ty`][Self::reserve_ty] for more information.
    // pub fn fulfill_ty(&mut self, id: TyId, info: TyInfo) {
    //     self.ty_info.insert(id, info);
    // }

    // /// Adds a method to an existing type.
    // ///
    // /// # Panics
    // /// Panics if the type has not been [`fulfilled`][Self::fulfill_ty].
    // pub fn add_method(&mut self, ty: TyId, method: VarId) {
    //     let method_name = self.vars[method].ident.ident;
    //     self.methods[ty].insert(method_name, method);
    // }

    // /// Returns the name of the given type.
    // pub fn ty_ident(&self, id: TyId) -> SpanIdent {
    //     self.tys[id]
    // }

    // /// Returns the information for the given type.
    // ///
    // /// # Panics
    // /// Panics if the type was reserved with [`reserve_ty`][Self::reserve_ty], but not yet fulfilled with [`fulfill_ty`][Self::fulfill_ty].
    // pub fn ty_info(&self, id: TyId) -> &TyInfo {
    //     &self.ty_info[id]
    // }
}

// Expr-related functions
impl Hir {
    /// Adds a spanned expression, returning a unique ID for it.
    pub fn add_expr(&mut self, expr: Expr, span: impl Into<Range<u32>>, ty: TyId) -> ExprId {
        self.exprs.insert((expr, span.into(), ty))
    }

    /// Returns the actual expression associated with the given ID.
    pub fn expr(&self, id: ExprId) -> &Expr {
        &self.exprs[id].0
    }

    /// Returns the span of the expression associated with the given ID.
    pub fn expr_span(&self, id: ExprId) -> Range<u32> {
        self.exprs[id].1
    }

    /// Returns the type of the expression associated with the given ID.
    pub fn expr_ty(&self, id: ExprId) -> TyId {
        self.exprs[id].2
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
/// An expression.
#[derive(Debug, PartialEq)]
pub enum Expr {
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

/// An ID representing an interned type. Information about the type can be acquired through [`Hir::ty_kind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TyId(usize);

/// The kinds of types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyKind {
    /// 64-bit signed integer.
    Int,
    /// 64-bit unsigned integer.
    UInt,
    /// 8-bit unsigned integer.
    Byte,
    /// Double-precision floating point number (binary64).
    Float,
    /// Truth value (`true`/`false`).
    Bool,
    /// A heterogenous tuple (compile-time length).
    Tuple(Vec<TyId>),
    /// A first-class function value, implemented as a closure.
    Func(Vec<Param>, TyId),
    /// A user-defined type, such as a `record` or `union`.
    Named {
        /// Whether the type is opaque (meaning it's fields/variants are private).
        opaque: bool,
        /// The fields of the type.
        fields: Vec<Field>,
        /// The ID of the type's constructor function.
        ctor: VarId,
        /// The module containing the type's definition.
        module: ModuleId,
    },
    /// A type variable, used during type inference.
    Var(TyVar),
    /// A type variable, standing in for an integer. Used during type inference.
    IntVar(TyVar),
}

impl TyKind {
    /// Constructs an empty [`Ty::Tuple`] for representing the Unit type.
    pub const fn unit() -> Self {
        Self::Tuple(vec![])
    }
}

/// A parameter of a function type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Param {
    /// The type of the paremeter.
    pub ty: TyId,
    /// Whether the parameter is mutable.
    pub mutable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A field of a `record` or of a `union` variant.
pub struct Field {
    /// The name of the field.
    pub ident: SpanIdent,
    /// The type of the field.
    pub ty: TyId,
}

#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct TyVar(u32);

impl UnifyKey for TyVar {
    type Value = Option<TyId>;

    fn index(&self) -> u32 {
        self.0
    }

    fn from_index(u: u32) -> Self {
        Self(u)
    }

    fn tag() -> &'static str {
        "TypeVar"
    }
}

impl EqUnifyValue for TyId {}
