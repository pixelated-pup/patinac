use errors::{Diagnostic, Report};
use ident::Ident;
use irs::hir::ty::{TyContext, TyId};

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ErrorKind {
    TypeMismatch { expected: TyId, found: TyId },
    MutMismatch { should_be_mut: bool },
    ArgCount { expected: usize, found: usize },
    Infinite,
    UninferredExprType,
    UninferredVarType,
    NoSuchField(TyId, Ident),
    NoSuchMethod(TyId, Ident),
    NoFieldsType(TyId),
    OpaqueType(TyId),
}

impl Diagnostic for ErrorKind {
    type Ctx<'a> = &'a TyContext;

    fn report(self, ty_ctx: &TyContext) -> Report {
        match self {
            Self::TypeMismatch { expected, found } => {
                Report::error("mismatched types").with_label(format!(
                    "expected type {}, found type {}",
                    expected.display(ty_ctx),
                    found.display(ty_ctx)
                ))
            }
            Self::MutMismatch { should_be_mut } => {
                let report = Report::error("incorrect argument mutability");
                if should_be_mut {
                    report.with_label("argument should be mutable")
                } else {
                    report.with_label("argument should not be mutable")
                }
            }
            Self::ArgCount { expected, found } => {
                let report = Report::error("wrong number of arguments");
                match expected {
                    1 => report.with_label(format!("expected 1 argument, found {found}")),
                    _ => report.with_label(format!("expected {expected} arguments, found {found}")),
                }
            }
            Self::Infinite => Report::error("infinite type")
                .with_label("the type of this expression recurses infinitely"),
            Self::UninferredExprType => Report::error("uninferred type")
                .with_label("could not infer a concrete type for this expression"),
            Self::UninferredVarType => Report::error("uninferred type")
                .with_label("could not infer a concrete type for this variable"),
            Self::NoSuchField(ty, field) => Report::error("field not found").with_label(format!(
                "type {} does not have a field named {field}",
                ty.display(ty_ctx)
            )),
            Self::NoSuchMethod(ty, method) => {
                Report::error("method not found").with_label(format!(
                    "type {} does not have a method named {method}",
                    ty.display(ty_ctx)
                ))
            }
            Self::NoFieldsType(ty) => Report::error("field access on type with no fields")
                .with_label(format!("type {} does not have fields", ty.display(ty_ctx))),
            Self::OpaqueType(ty) => Report::error("field not accessible")
                .with_label(format!("type `{}` is opaque", ty.display(ty_ctx))),
        }
    }
}
