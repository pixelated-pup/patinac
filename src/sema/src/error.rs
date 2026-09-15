use std::{
    fmt::{self, Display, Formatter},
    range::Range,
};

use errors::{Diagnostic, Report};
use ident::Ident;
use irs::hir::TyId;

#[derive(Debug, PartialEq, Eq)]
pub enum ErrorKind {
    UnknownName(ItemKind, Ident),
    PrivateItem(ItemKind, Ident),
    DuplicateItem(ItemKind, Ident),
    Mutation(Ident),
    NotPlaceExpr,
    OverlappingArgs(Range<u32>, Range<u32>),
    DupFields(Ident),
    InvalidMain,
    SelfOutsideImpl,
    TypeMismatch { expected: TyId, found: TyId },
    MutMismatch { should_be_mut: bool },
    Infinite,
    UninferredExprType,
    UninferredVarType,
    NoSuchField(Ident, Ident),
    NoSuchMethod(TyId, Ident),
    NoFieldsType(TyId),
    OpaqueType(Ident),
}

impl Diagnostic for ErrorKind {
    fn report(self) -> Report {
        match self {
            Self::UnknownName(kind, ident) => Report::error(format!("unresolved {kind}"))
                .with_label(format!("cannot find {kind} `{ident}`")),
            Self::PrivateItem(kind, ident) => Report::error(format!("private {kind}"))
                .with_label(format!("`{ident}` is not visible")),
            Self::DuplicateItem(kind, ident) => Report::error(format!("duplicate {kind}"))
                .with_label(format!(
                    "there is already a {kind} named `{ident}` in this scope"
                )),
            Self::Mutation(ident) => Report::error("attempted mutation of immutable variable")
                .with_label(format!("variable `{ident}` is not mutable")),
            Self::NotPlaceExpr => Report::error("attempted mutation of immutable value")
                .with_note("only mutable variables, fields, and indices can be mutated"),
            Self::OverlappingArgs(range, range1) => Report::error("overlapping mutable arguments")
                .with_label("this mutable argument is not unique"),
            Self::DupFields(ident) => Report::error("duplicate field"),
            Self::InvalidMain => Report::error("invalid main function")
                .with_note("the main function must take no parameters and have a unit return type"),
            Self::SelfOutsideImpl => Report::error("self parameter on free function")
                .with_note("self parameters can only appear on functions within impl blocks"),
            Self::TypeMismatch { expected, found } => Report::error("mismatched types")
                .with_label(format!("expected type {expected:?}, found type {found:?}")),
            Self::MutMismatch { should_be_mut } => {
                let report = Report::error("incorrect argument mutability");
                if should_be_mut {
                    report.with_label("argument should be mutable")
                } else {
                    report.with_label("argument should not be mutable")
                }
            }
            Self::Infinite => Report::error("infinite type")
                .with_label("the type of this expression recurses infinitely"),
            Self::UninferredExprType => Report::error("uninferred type")
                .with_label("could not infer a concrete type for this expression"),
            Self::UninferredVarType => Report::error("uninferred type")
                .with_label("could not infer a concrete type for this variable"),
            Self::NoSuchField(ty, field) => Report::error("field not found")
                .with_label(format!("type {ty} does not have a field named {field}")),
            Self::NoSuchMethod(ty, method) => Report::error("method not found")
                .with_label(format!("type {ty:?} does not have a method named {method}")),
            Self::NoFieldsType(ty) => Report::error("field access on type with no fields")
                .with_label(format!("type {ty:?} does not have fields")),
            Self::OpaqueType(ident) => Report::error("field not accessible")
                .with_label(format!("type `{ident}` is opaque")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Module,
    Type,
    Value,
    Unknown,
}

impl Display for ItemKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Module => "module",
            Self::Type => "type",
            Self::Value => "value",
            Self::Unknown => "item",
        }
        .fmt(f)
    }
}
