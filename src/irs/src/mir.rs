//! The mid-level intermediate representation of Patina. Lowered from the HIR, and used for code generation.

use foldhash::fast::RandomState;
use ordermap::OrderSet;
use slotmap::{SlotMap, new_key_type};

use ident::Ident;

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
    tys: OrderSet<Ty, RandomState>,
}

impl Mir {
    /// Returns an iterator over all of the items.
    pub fn items(&self) -> impl Iterator<Item = (ItemId, &Item)> {
        self.items.iter()
    }

    /// Adds an item, returning a unique ID for it.
    pub fn add_item(&mut self, item: Item) -> ItemId {
        self.items.insert(item)
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

impl Mir {
    /// Either interns the given type and returns a new ID for it,
    /// or returns it's existing ID if it's already been interned.
    pub fn intern_ty(&mut self, ty: Ty) -> TyId {
        let (idx, _) = self.tys.insert_full(ty);
        TyId(idx.try_into().expect("too many type IDs!"))
    }

    /// Shorthand for `self.intern_ty(TyKind::Int)`.
    pub fn int(&mut self) -> TyId {
        self.intern_ty(Ty::Int)
    }

    /// Shorthand for `self.intern_ty(TyKind::UInt)`.
    pub fn uint(&mut self) -> TyId {
        self.intern_ty(Ty::UInt)
    }

    /// Shorthand for `self.intern_ty(TyKind::Byte)`.
    pub fn byte(&mut self) -> TyId {
        self.intern_ty(Ty::Byte)
    }

    /// Shorthand for `self.intern_ty(TyKind::Float)`.
    pub fn float(&mut self) -> TyId {
        self.intern_ty(Ty::Float)
    }

    /// Shorthand for `self.intern_ty(TyKind::Bool)`.
    pub fn bool(&mut self) -> TyId {
        self.intern_ty(Ty::Bool)
    }

    /// Returns the type corresponding to the given ID.
    pub fn get_ty(&self, id: TyId) -> &Ty {
        self.tys
            .get_index(id.0 as usize)
            .expect("TyId without corresponding Ty!")
    }
}

new_key_type! {
    /// An ID representing an item.
    pub struct ItemId;
}

/// An item.
///
/// The only items directly tracked by the MIR are functions and constants,
/// known as "executable items" in earlier IRs.
///
/// All information for user-defined types has been lowered into normal types,
/// and methods have been fully desugared into regular functions.
#[derive(Debug)]
pub enum Item {
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
    /// The number of arguments to the function.
    arg_count: u32,
    /// The local variables of the function.
    ///
    /// The first `arg_count` are the function's arguments,
    /// and the remainder are user-defined variables and temporaries
    locals: SlotMap<VarId, VarInfo>,
    /// The return type of the function.
    ret_ty: TyId,
    /// The basic blocks comprising the function's body.
    blocks: SlotMap<BlockId, Block>,
}

impl Func {
    /// Adds a variable with the provided information, returning a unique ID for it.
    pub fn add_var(&mut self, info: VarInfo) -> VarId {
        self.locals.insert(info)
    }

    /// Returns the information for the given variable.
    pub fn var(&self, id: VarId) -> &VarInfo {
        &self.locals[id]
    }
}

new_key_type! {
    /// An ID representing a variable. Not a global ID, but rather per-function.
    pub struct VarId;
}
/// Information for a variable.
#[derive(Debug, Clone, PartialEq)]
pub struct VarInfo {
    /// The name of the variable.
    pub ident: Ident,
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
    stmts: Vec<Stmt>,
    term: Terminator,
}

/// The kinds of terminators, representing different ways to exit from a basic block.
#[derive(Debug)]
pub enum Terminator {
    /// An unconditional jump.
    Goto(BlockId),
    /// A conditional switch on an integer.
    ///
    /// The last element of `targets` is the `else` branch,
    /// so `targets.len() == values.len() + 1` must always hold.
    Switch {
        scrutinee: Value,
        values: Vec<u64>,
        targets: Vec<BlockId>,
    },
    /// Returning from the current function.
    Return(Value),
    /// Indicates an unreachable branch. UB if this is ever executed.
    Unreachable,
}

/// A statement.
#[derive(Debug)]
pub enum Stmt {
    /// Assignment to a place.
    Assign {
        /// The place being assigned to.
        place: Place,
        /// The value being assigned.
        value: Value,
    },
    /// An expression used as a statement, evaluated purely for side-effects.
    // Expr(ExprId),
    /// A function call.
    Call {
        /// The function being called.
        func: Value,
        /// The list of arguments being applied.
        args: Vec<Arg>,
        /// Where the result of the function call will be stored.
        dst: Place,
    },
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
    /// A literal for a heterogenous aggreggate value (tuple or record).
    ///
    /// The two vecs should be the same lengths, forming field type - field value pairs.
    Construct(Vec<TyId>, Vec<Value>),
    /// An infix operation.
    Infix {
        /// The infix operator used.
        op: InfixOp,
        /// The left-hand side of the operation.
        lhs: Value,
        /// The right-hand side of the operation.
        rhs: Value,
    },
    /// A prefix operation.
    Prefix {
        /// The prefix operator used.
        op: PrefixOp,
        /// The value the operator is applied to.
        expr: Value,
    },
    /// A closure.
    Closure {
        /// The lifted function representing the closure body.
        func: VarId,
        /// The captured variables.
        captures: Vec<VarId>,
    },

    /// TEMPORARY, until we have stdlib + FFI.
    Print(Value),
}

#[derive(Debug)]
pub enum Value {
    /// A copy from a place. This is the default way of reading from a place.
    Copy(Place),
    /// A move out of a place.
    /// This does not run any copy code (such as incrementing a refcount),
    /// and is used as an optimisation when the source place is never used again.
    Move(Place),
    /// A scalar literal value. The specific kinds of literals are represented by [`LitValue`].
    Lit(LitValue),
    Item(ItemId),
    Zst,
}

#[derive(Debug)]
pub struct Place {
    pub base: VarId,
    /// A list of field accesses applied to the base place.
    /// If this is empty, the place is a local variable.
    pub accesses: Vec<FieldAccess>,
}

#[derive(Debug)]
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

/// A block of statements.
pub struct BlockExpr(pub Vec<Stmt>);

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
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct TyId(u32);

/// The kinds of types.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Ty {
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
    /// A collection of heterogenous fields. Tuples and records both get lowered to this.
    Fields(Vec<TyId>),
    /// A first-class function value, implemented as a closure.
    Func(Vec<Param>, TyId),
}

impl Mir {
    /// Size of a pointer in bytes. FIXME: Support for non-64 bit architectures?
    const PTR_SIZE: u32 = 8;

    /// Returns the inline size of the provided type, in bytes.
    pub fn size_of(&self, ty: TyId) -> u32 {
        match self.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Float => 8,
            Ty::Byte | Ty::Bool => 1,
            Ty::Fields(field_tys) => {
                let align = self.align_of(ty);

                let base_size = field_tys.iter().fold(0, |sum, &ty| {
                    let align = self.align_of(ty);
                    let padding = (align - (sum % align)) % align;
                    sum + padding + self.size_of(ty)
                });

                let end_padding = (align - (base_size % align)) % align;
                base_size + end_padding
            }
            Ty::Func(_, _) => Self::PTR_SIZE * 5, // Function, environment, drop, copy, equality.
        }
    }

    /// Returns the alignment of this type, in bytes.
    pub fn align_of(&self, ty: TyId) -> u32 {
        match self.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Float | Ty::Byte | Ty::Bool => self.size_of(ty),
            Ty::Fields(field_tys) => field_tys
                .iter()
                .map(|&ty| self.align_of(ty))
                .max()
                .unwrap_or(1),
            Ty::Func(_, _) => Self::PTR_SIZE,
        }
    }
}

/// A parameter of a function type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Param {
    /// The type of the paremeter.
    pub ty: TyId,
    /// Whether the parameter is mutable.
    pub mutable: bool,
}
