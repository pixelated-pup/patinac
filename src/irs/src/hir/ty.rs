//! HIR types and type-related items.

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

use ident::{Ident, SpanIdent};

use crate::{ModuleId, hir::VarId};

/// Interns and unifies types.
pub struct TyContext {
    store: OrderSet<Ty, RandomState>,
    unif_table: RefCell<InPlaceUnificationTable<TyId, Vec<VarValue<TyId>>, NoUndo>>,
}

#[expect(
    clippy::as_conversions,
    reason = "All casts are u32 -> usize, and 16-bit platforms are nostd"
)]
impl TyContext {
    /// Either interns the given type and returns a new ID for it,
    /// or returns it's existing ID if it's already been interned.
    pub fn intern_ty(&mut self, ty: Ty) -> TyId {
        let (idx, exists) = self.store.insert_full(ty);
        if exists {
            TyId(idx.try_into().expect("too many type IDs!"))
        } else {
            let key = self.unif_table.get_mut().new_key(());
            assert_eq!(key.0 as usize, idx);
            key
        }
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

    /// Shorthand for `self.intern_ty(TyKind::unit())`.
    pub fn unit(&mut self) -> TyId {
        self.intern_ty(Ty::unit())
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
        fields: OrderMap<Ident, Field>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A field of a `record` or of a `union` variant.
pub struct Field {
    /// The name of the field.
    pub ident: SpanIdent,
    /// The type of the field.
    pub ty: TyId,
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
