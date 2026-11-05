//! Types and type-related items.
//!
//! This is a semantic representation of a type used by the HIR and MIR,
//! as opposed to the syntactic types used by the AST.

use std::{
    cell::RefCell,
    fmt::{self, Display, Formatter},
};

use ena::{
    undo_log::NoUndo,
    unify::{InPlaceUnificationTable, UnifyKey, VarValue},
};
use foldhash::fast::RandomState;
use itertools::Itertools as _;
use ordermap::{OrderMap, OrderSet};

use ident::Ident;

use crate::{ModuleId, hir::VarId};

/// Size of a pointer in bytes. FIXME: Support for non-64 bit architectures?
pub const PTR_SIZE: u32 = 8;

/// Interns and unifies types.
pub struct TyContext {
    store: OrderSet<Ty, RandomState>,
    unif_table: RefCell<InPlaceUnificationTable<TyId, Vec<VarValue<TyId>>, NoUndo>>,
    int: TyId,
    uint: TyId,
    byte: TyId,
    float: TyId,
    bool: TyId,
    unit: TyId,
}

#[expect(
    clippy::as_conversions,
    reason = "All casts are u32 -> usize, and 16-bit platforms are nostd"
)]
impl TyContext {
    pub fn new() -> Self {
        let mut store = OrderSet::default();
        let mut unif_table: RefCell<InPlaceUnificationTable<TyId, Vec<VarValue<TyId>>, NoUndo>> =
            todo!(); //RefCell::default();

        let mut init = |ty| {
            let (idx, _) = store.insert_full(ty);
            let key: TyId = unif_table.get_mut().new_key(());
            assert_eq!(key.0 as usize, idx);
            key
        };

        let int = init(Ty::Int);
        let uint = init(Ty::UInt);
        let byte = init(Ty::Byte);
        let float = init(Ty::Float);
        let bool = init(Ty::Bool);
        let unit = init(Ty::Tuple(Vec::new()));

        Self {
            store,
            unif_table,
            int,
            uint,
            byte,
            float,
            bool,
            unit,
        }
    }

    /// Either interns the given type and returns a new ID for it,
    /// or returns it's existing ID if it's already been interned.
    pub fn intern(&mut self, ty: Ty) -> TyId {
        let (idx, exists) = self.store.insert_full(ty);
        if exists {
            TyId(idx.try_into().expect("too many type IDs!"))
        } else {
            let key = self.unif_table.get_mut().new_key(());
            assert_eq!(key.0 as usize, idx);
            key
        }
    }

    /// Faster than `self.intern_ty(TyKind::Int)`.
    pub fn int(&self) -> TyId {
        self.int
    }

    /// Faster than `self.intern_ty(TyKind::UInt)`.
    pub fn uint(&self) -> TyId {
        self.uint
    }

    /// Faster than `self.intern_ty(TyKind::Byte)`.
    pub fn byte(&self) -> TyId {
        self.byte
    }

    /// Faster than `self.intern_ty(TyKind::Float)`.
    pub fn float(&self) -> TyId {
        self.float
    }

    /// Faster than `self.intern_ty(TyKind::Bool)`.
    pub fn bool(&self) -> TyId {
        self.bool
    }

    /// Faster than `self.intern_ty(TyKind::unit())`.
    pub fn unit(&self) -> TyId {
        self.unit
    }

    /// Creates a new, distinct type variable and returns it's ID.
    pub fn fresh_var(&mut self) -> TyId {
        let key = self.unif_table.get_mut().new_key(());
        let (idx, exists) = self.store.insert_full(Ty::Var(key));
        assert!(!exists);
        assert_eq!(key.0 as usize, idx);
        key
    }

    /// Creates a new, distinct int type variable and returns it's ID.
    pub fn fresh_int_var(&mut self) -> TyId {
        let key = self.unif_table.get_mut().new_key(());
        let (idx, exists) = self.store.insert_full(Ty::IntVar(key));
        assert!(!exists);
        assert_eq!(key.0 as usize, idx);
        key
    }

    /// Returns the type corresponding to the given ID.
    pub fn get(&self, id: TyId) -> &Ty {
        let id = self.unif_table.borrow_mut().find(id);
        self.store
            .get_index(id.0 as usize)
            .expect("TyId without corresponding Ty!")
    }

    /// Unifies the types represented by the provided ids.
    /// Does not perform any validation, ensure the types are semantically equivalent before calling this method.
    pub fn unify(&self, a: TyId, b: TyId) {
        self.unif_table.borrow_mut().union(a, b);
    }

    /// Returns the inline size of the provided type, in bytes.
    ///
    /// # Panics
    /// Panics if the provided type is a [`Ty::Var`] or [`Ty::IntVar`].
    pub fn size_of(&self, ty: TyId) -> u32 {
        match self.get(ty) {
            Ty::Int | Ty::UInt | Ty::Float => 8,
            Ty::Byte | Ty::Bool => 1,
            Ty::Tuple(elem_tys) => todo!(),
            // Ty::Fields(field_tys) => {
            //     let align = self.align_of(ty);

            //     let base_size = field_tys.iter().fold(0, |sum, &ty| {
            //         let align = self.align_of(ty);
            //         let padding = (align - (sum % align)) % align;
            //         sum + padding + self.size_of(ty)
            //     });

            //     let end_padding = (align - (base_size % align)) % align;
            //     base_size + end_padding
            // }
            Ty::Func(_, _) => PTR_SIZE * 5, // Function, environment, drop, copy, equality.
            Ty::Named {
                ident,
                opaque,
                fields,
                ctor,
                module,
            } => todo!(),
            Ty::Var(_) | Ty::IntVar(_) => panic!("Attempted to get size of type variable"),
        }
    }

    /// Returns the alignment of the provided type, in bytes.
    ///
    /// # Panics
    /// Panics if the provided type is a [`Ty::Var`] or [`Ty::IntVar`].
    pub fn align_of(&self, ty: TyId) -> u32 {
        match self.get(ty) {
            Ty::Int | Ty::UInt | Ty::Float | Ty::Byte | Ty::Bool => self.size_of(ty),
            Ty::Tuple(elem_tys) => elem_tys
                .iter()
                .map(|ty| self.align_of(*ty))
                .max()
                .unwrap_or(1),
            Ty::Func(_, _) => PTR_SIZE,
            Ty::Named { fields, .. } => fields
                .iter()
                .map(|(_, ty)| self.align_of(*ty))
                .max()
                .unwrap_or(1),
            Ty::Var(_) | Ty::IntVar(_) => panic!("Attempted to get alignment of type variable"),
        }
    }

    /// Returns whether the provided type is a ZST (zero-sized type).
    /// More efficient than calculating the size and checking if it's 0.
    ///
    /// # Panics
    /// Panics if the provided type is a [`Ty::Var`] or [`Ty::IntVar`].
    pub fn is_zst(&self, ty: TyId) -> bool {
        match self.get(ty) {
            Ty::Int | Ty::UInt | Ty::Float | Ty::Byte | Ty::Bool | Ty::Func(_, _) => false,
            Ty::Tuple(elem_tys) => {
                elem_tys.is_empty() || elem_tys.iter().all(|ty| self.is_zst(*ty))
            }
            Ty::Named { fields, .. } => {
                fields.is_empty() || fields.iter().all(|(_, ty)| self.is_zst(*ty))
            }
            Ty::Var(_) | Ty::IntVar(_) => panic!("Attempted to get size of type variable"),
        }
    }
}

/// An ID representing an interned type. Information about the type can be acquired through [`Hir::ty_kind`].
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct TyId(u32);

impl TyId {
    /// Wraps this `TyId` alongside a `TyContext` so that it can be displayed.
    pub const fn display(self, ctx: &TyContext) -> TyDisplay<'_> {
        TyDisplay(self, ctx)
    }
}

impl UnifyKey for TyId {
    type Value = ();

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
    /// A heterogenous tuple (compile-time length).
    Tuple(Vec<TyId>),
    /// A first-class function value, implemented as a closure.
    Func(Vec<Param>, TyId),
    /// A user-defined type, such as a `record` or `union`.
    Named {
        /// The name of the type.
        ident: Ident,
        /// Whether the type is opaque (meaning it's fields/variants are private).
        opaque: bool,
        /// The fields of the type.
        fields: OrderMap<Ident, TyId>,
        /// The ID of the type's constructor function.
        ctor: VarId,
        /// The module containing the type's definition.
        module: ModuleId,
    },
    /// A type variable, used during type inference.
    /// Stores it's own ID, so that it's distinct from every other `Var`.
    Var(TyId),
    /// A type variable, standing in for an integer. Used during type inference.
    /// Stores it's own ID, so that it's distinct from every other `IntVar`.
    IntVar(TyId),
}

impl Ty {
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

impl Param {
    /// Wraps this `Param` alongside a `TyContext` so that it can be displayed.
    pub const fn display(self, ctx: &TyContext) -> ParamDisplay<'_> {
        ParamDisplay(self, ctx)
    }
}

/// A helper to display a `TyId` using information from the `TyContext`.
#[derive(Clone, Copy)]
pub struct TyDisplay<'ctx>(TyId, &'ctx TyContext);

impl Display for TyDisplay<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let &TyDisplay(id, ctx) = self;
        match ctx.get(id) {
            Ty::Int => "Int".fmt(f),
            Ty::UInt => "UInt".fmt(f),
            Ty::Byte => "Byte".fmt(f),
            Ty::Float => "Float".fmt(f),
            Ty::Bool => "Bool".fmt(f),
            Ty::Tuple(elem_tys) => {
                let tys = elem_tys.iter().map(|id| id.display(ctx)).join(", ");
                write!(f, "({tys})")
            }
            Ty::Func(params, ret_ty) => {
                let params = params.iter().map(|param| param.display(ctx)).join(", ");
                write!(f, "Fn({params}) -> {}", ret_ty.display(ctx))
            }
            Ty::Named { ident, .. } => ident.fmt(f),
            Ty::Var(id) => write!(f, "?{}", id.0),
            Ty::IntVar(_) => write!(f, "{{int}}"),
        }
    }
}

/// A helper to display a `Param` using information from the `Hir`.
#[derive(Clone, Copy)]
pub struct ParamDisplay<'ctx>(Param, &'ctx TyContext);

impl Display for ParamDisplay<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let &ParamDisplay(param, ctx) = self;
        if param.mutable {
            "mut ".fmt(f)?;
        }
        param.ty.display(ctx).fmt(f)
    }
}
