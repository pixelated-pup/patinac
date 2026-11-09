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
    /// The local variables of the function.
    ///
    /// The first `arg_count` are the function's arguments,
    /// and the remainder are user-defined variables and temporaries
    pub locals: SlotMap<LocalId, Local>,
    /// The basic blocks comprising the function's body.
    pub blocks: SlotMap<BlockId, Block>,
}

impl Func {
    /// Creates a new empty function with the given parameter count.
    /// The locals representing the parameters must still be set up.
    pub fn new(param_count: usize) -> Self {
        Self {
            param_count,
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

    /// Adds a new empty block to this function and returns it's id.
    pub fn add_block(&mut self) -> BlockId {
        self.blocks.insert(Block {
            stmts: Vec::new(),
            term: None,
        })
    }

    /// Returns a reference to the block with the given id.
    pub fn block(&self, block: BlockId) -> &Block {
        &self.blocks[block]
    }

    /// Returns a mutable reference to the block with the given id.
    pub fn block_mut(&mut self, block: BlockId) -> &mut Block {
        &mut self.blocks[block]
    }

    /// Appends a statement to the block with the given id.
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
    /// The statements contained within the block.
    pub stmts: Vec<Stmt>,
    /// The terminator of the block.
    ///
    /// Should only be `None` during construction,
    /// and should usually be accessed through [`Block::terminator`].
    pub term: Option<Terminator>,
}

impl Block {
    /// Returns the terminator of the block.
    ///
    /// # Panics
    /// Panics if the terminator has not yet been set.
    /// This should only be the case while the block is being built.
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
    /// An immutable argument passed as a value.
    Immut(Value),
    /// A mutable argument passed as a place.
    Mut(Place),
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

/// A simple value that needs minimal further evaluation.
///
/// Values act as operands to [`Exprs`][Expr],
/// and are either some form of read from a [`place`][`Place`],
/// a [`literal`][`LitValue`],
/// or a constant ZST value.
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

/// A place, identifying a memory location.
///
/// A place is a local variables with possible field accesses projecting from it.
/// It may either be written to, read from, or used as a mutable parameter.
#[derive(Clone, Debug)]
pub struct Place {
    /// The base variable this place is derived from.
    pub base: LocalId,
    /// A list of field accesses applied to the base place.
    /// If this is empty, the place is directly a local variable.
    pub accesses: Vec<FieldAccess>,
}

impl Place {
    /// Constructs a new place representing a local variable with no field accesses.
    pub fn local(local: LocalId) -> Self {
        Self {
            base: local,
            accesses: Vec::new(),
        }
    }
}

/// A field access used as part of a [`Place`].
///
/// In the MIR, fields are represented by index in the order written in the source code.
#[derive(Clone, Copy, Debug)]
pub struct FieldAccess {
    /// The source-code index of the field being accessed.
    pub idx: usize,
    /// The type of the field being accessed.
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
    /// Float or integer addition.
    Add,
    /// Float or integer subtraction.
    Sub,
    /// Float or integer multiplication.
    Mul,
    /// Float or integer division.
    ///
    /// Integer division by zero is undefined behaviour,
    /// and checks should have been inserted into the MIR around this operator.
    Div,
    /// Exponentiation.
    Exp,
    /// Equality, on any type.
    Eqq,
    /// Inequality, on any type.
    Neq,
    /// Float or integer greater-than comparison.
    Gt,
    /// Float or integer less-than comparison.
    Lt,
    /// Float or integer greater-than or equal-to comparison.
    Geq,
    /// Float or integer less-than or equal-to comparison.
    Leq,
}

/// A prefix operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixOp {
    /// Boolean negation.
    Not,
    /// Float or integer negation.
    Neg,
}
