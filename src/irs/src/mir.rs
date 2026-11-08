//! The mid-level intermediate representation of Patina. Lowered from the HIR, and used for code generation.

use slotmap::{SecondaryMap, SlotMap, new_key_type};

use ident::Ident;

use crate::ty::{DefId, TyId};

/// The mid-level intermediate representation of Patina, a CFG with no nested expressions.
///
/// - Lambdas have been lowered to closures
/// - Location information has been stripped
/// - Type information has been trimmed down to what is necessary
/// Eventually:
/// - Methods have been desugared into regular functions
/// - Constants will have been desugared into functions
/// - Generics will have been monomorphised
/// - Pattern matching will have been lowered to switching on integers
#[derive(Debug, Default)]
pub struct Mir {
    funcs: SecondaryMap<DefId, Func>,
}

impl Mir {
    /// Adds a function corresponding to the given definition.
    pub fn add_func(&mut self, def: DefId, func: Func) {
        self.funcs.insert(def, func);
    }

    /// Returns the information for the given function.
    pub fn func(&self, def: DefId) -> &Func {
        &self.funcs[def]
    }
}

/// A function item.
///
/// This is the only kind of item in the MIR, as constants are lowered to functions.
#[derive(Debug)]
pub struct Func {
    /// The number of parameters that the function has.
    pub param_count: usize,
    /// The return type of the function.
    pub ret_ty: TyId,
    /// The local variables of the function.
    ///
    /// The first `arg_count` are the function's arguments,
    /// and the remainder are user-defined variables and temporaries
    pub locals: SlotMap<LocalId, Local>,
    /// The basic blocks comprising the function's body.
    pub blocks: SlotMap<BlockId, Block>,
}

impl Func {
    pub fn new(param_count: usize, ret_ty: TyId) -> Self {
        Self {
            param_count,
            ret_ty,
            locals: SlotMap::default(),
            blocks: SlotMap::default(),
        }
    }

    /// Adds a local variable with the provided information, returning a unique ID for it.
    pub fn add_local(&mut self, var: Local) -> LocalId {
        self.locals.insert(var)
    }

    /// Returns the information for the given variable.
    pub fn local(&self, id: LocalId) -> &Local {
        &self.locals[id]
    }

    /// Adds a temporary variable of the provided type (immutable, no name).
    pub fn add_temp(&mut self, ty: TyId) -> LocalId {
        self.add_local(Local {
            ident: None,
            mutable: false,
            ty,
        })
    }

    pub fn add_block(&mut self) -> BlockId {
        self.blocks.insert(Block {
            stmts: Vec::new(),
            term: None,
        })
    }

    pub fn block(&self, block: BlockId) -> &Block {
        &self.blocks[block]
    }

    pub fn block_mut(&mut self, block: BlockId) -> &mut Block {
        &mut self.blocks[block]
    }

    pub fn add_stmt(&mut self, block: BlockId, stmt: Stmt) {
        self.block_mut(block).stmts.push(stmt);
    }
}

new_key_type! {
    /// An ID representing a local variable. Not a global ID, but rather per-function.
    pub struct LocalId;
}
/// Information for a local variable.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    /// The name of the variable, if there is one (`None` for e.g. temporaries).
    pub ident: Option<Ident>,
    /// Whether the variable was declared mutable.
    pub mutable: bool,
    /// The type of the variable.
    pub ty: TyId,
}

new_key_type! {
    /// An ID representing a basic block. Not a global ID, but rather per-function.
    pub struct BlockId;
}

/// A basic block of control flow.
#[derive(Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    /// The terminator of the block.
    ///
    /// Should only be `None` during construction,
    /// and should usually be accessed through [`Block::terminator`].
    pub term: Option<Terminator>,
}

impl Block {
    pub fn terminator(&self) -> &Terminator {
        self.term
            .as_ref()
            .expect("Accessed terminator of incomplete block")
    }
}

/// The kinds of terminators, representing different ways to exit from a basic block.
#[derive(Debug)]
pub enum Terminator {
    /// An unconditional jump.
    Goto(BlockId),
    /// A conditional switch on an integer value.
    Switch {
        /// The value being switched on.
        scrutinee: Value,
        /// Pairs of values and target blocks.
        targets: Vec<(u64, BlockId)>,
        /// The fallback block.
        otherwise: BlockId,
    },
    /// Returning from the current function.
    Return(Value),
    /// Indicates an unreachable branch. UB if this is ever executed.
    Unreachable,
}

/// A statement.
#[derive(Debug)]
pub enum Stmt {
    /// Evaluate an expression and store it in a place.
    Assign {
        /// The place being assigned to.
        place: Place,
        /// The expression being assigned.
        expr: Expr,
    },
    /// An expression used as a statement, evaluated purely for side-effects.
    // Expr(ExprId),
    /// A function call.
    Call {
        /// The function being called.
        callee: Value,
        /// The list of arguments being applied.
        args: Vec<Arg>,
        /// Where the result of the function call will be stored.
        dst: Place,
    },
    /// Drop the contents of a place.
    Drop(Place),
    /// TEMPORARY, until we have stdlib + FFI.
    Print(Value),
}

/// An argument in a [function call][Stmt::Call]. Either a [`Value`] or a [`Place`].
#[derive(Debug)]
pub enum Arg {
    Value(Value),
    Place(Place),
}

/// An expression.
#[derive(Debug)]
pub enum Expr {
    /// A value used unchanged as an expression.
    Val(Value),
    /// An infix operation.
    Infix(InfixOp, Value, Value),
    /// A prefix operation.
    Prefix(PrefixOp, Value),
    /// A closure.
    Closure {
        /// The lifted function representing the closure body.
        body: DefId,
        /// The environment, containing the captured values.
        env: Value,
    },
}

#[derive(Clone, Debug)]
pub enum Value {
    /// A copy from a place. The resulting value is independent of the source place.
    Copy(Place),
    /// A move out of a place. This does not run any copy code (such as incrementing a refcount).
    /// The resulting value may be tied to the source place, and so neither may be mutated while both are live.
    Share(Place),
    /// A scalar literal value. The specific kinds of literals are represented by [`LitValue`].
    Lit(LitValue),
    /// A ZST value of the given type.
    Zst(TyId),
}

#[derive(Clone, Debug)]
pub struct Place {
    /// The base variable this place is derived from.
    pub base: LocalId,
    /// A list of field accesses applied to the base place.
    /// If this is empty, the place is directly a local variable.
    pub accesses: Vec<FieldAccess>,
}

impl Place {
    pub fn local(local: LocalId) -> Self {
        Self {
            base: local,
            accesses: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FieldAccess {
    pub idx: usize,
    pub ty: TyId,
}

/// The kinds of [literal values][Value::Lit].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LitValue {
    /// A signed 64-bit integer.
    Int(i64),
    /// An unsigned 64-bit interger.
    UInt(u64),
    /// An unsigned 8-bit integer.
    Byte(u8),
    /// A float. Can include exponent.
    Float(f64),
    /// A boolean.
    Bool(bool),
}

/// An infix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfixOp {
    Add,
    Sub,
    Mul,
    Div,
    Exp,
    Eqq,
    Neq,
    Gt,
    Lt,
    Geq,
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
