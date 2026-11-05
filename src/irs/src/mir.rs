//! The mid-level intermediate representation of Patina. Lowered from the HIR, and used for code generation.

use slotmap::{SlotMap, new_key_type};

use ident::Ident;

use crate::ty::TyId;

/// The mid-level intermediate representation of Patina.
///
/// - Expressions have been flattened into something between ANF and SSA
/// - Control flow has been lowered to gotos
/// - Lambdas have been lowered to closures
/// - Records have been erased and are equivalent to tuples
/// - Location information has been stripped
/// - Type information has been trimmed down to what is necessary
/// Eventually:
/// - Generics will have been monomorphised
/// - Pattern matching will have been lowered to switching on integers
#[derive(Debug, Default)]
pub struct Mir {
    main: Option<ItemId>,
    items: SlotMap<ItemId, Item>,
}

impl Mir {
    /// Returns an iterator over all of the items.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.values()
    }

    /// Adds an item, returning a unique ID for it.
    pub fn add_item(&mut self, item: Item) -> ItemId {
        self.items.insert(item)
    }

    /// Returns the information for the given item.
    pub fn item(&self, id: ItemId) -> &Item {
        &self.items[id]
    }

    /// Returns the main function, if it exists.
    pub const fn main(&self) -> Option<ItemId> {
        self.main
    }

    /// Sets the main function.
    pub fn set_main(&mut self, main: ItemId) {
        self.main = Some(main);
    }
}

new_key_type! {
    /// An ID representing an item.
    pub struct ItemId;
}

/// An item.
///
/// The only items directly tracked by the MIR are functions and constants,
/// known as "definition items" in earlier IRs.
/// Methods have also been fully desugared into regular functions.
#[derive(Debug)]
pub struct Item {
    /// The user-defined name of the item.
    pub ident: Ident,
    /// The type of the item.
    pub ty: TyId,
    /// The kind of item.
    pub kind: ItemKind,
}

#[derive(Debug)]
pub enum ItemKind {
    /// A constant item.
    ///
    /// The definition of constant used here is more specific than a source-language constant: These are specifically constants with literal values.
    /// All other constants are lazily-initialised and therefore lowered to functions.
    Const(LitValue),
    /// A function item.
    Func(Func),
}

/// A function item.
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
        body: ItemId,
        /// The environment, containing the captured values.
        env: Value,
    },
}

#[derive(Clone, Debug)]
pub enum Value {
    /// A copy from a place. This is the default way of reading from a place.
    Copy(Place),
    /// A move out of a place.
    /// This does not run any copy code (such as incrementing a refcount),
    /// and is used as an optimisation when the source place is never used again.
    Move(Place),
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
    pub index: u32,
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
