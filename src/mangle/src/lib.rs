//! Name mangling.
//!
//! All mangled symbols consist exclusively of a-z, A-Z, 0-9, and _.
//! Unicode characters in identifiers are encoded with punycode, and hyphens in the punycode are replaced with underscores.

use std::fmt::{self, Display, Formatter};

use ident::Ident;
use idna::punycode;
use irs::{
    ModuleId, Package,
    ty::{DefId, Ty, TyContext, TyId},
};

const PREFIX: &str = "_P";

/// Produces a mangled symbol for the given function.
pub fn mangle_func(pkg: &Package, ty_ctx: &TyContext, func: DefId) -> String {
    let func = ty_ctx.def(func);
    format!(
        "{PREFIX}{}",
        PathMangler::new(pkg, func.module, Some(func.ident))
    )
}

/// Produces a mangled symbol for an associated function with the given name
pub fn mangle_assoc_func(pkg: &Package, ty_ctx: &TyContext, ty: TyId, name: Ident) -> String {
    format!(
        "{PREFIX}A{}{}",
        TyMangler::new(pkg, ty_ctx, ty),
        IdentMangler(name)
    )
}

#[derive(Clone, Copy)]
struct PathMangler<'ext> {
    pkg: &'ext Package,
    module: ModuleId,
    end: Option<Ident>,
}

impl<'ext> PathMangler<'ext> {
    fn new(pkg: &'ext Package, module: ModuleId, end: Option<Ident>) -> Self {
        Self { pkg, module, end }
    }

    fn with(self, module: ModuleId) -> Self {
        Self {
            module,
            end: None,
            pkg: self.pkg,
        }
    }
}

impl Display for PathMangler<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let module = self.pkg.get(self.module);
        match module.parent {
            Some(parent) => write!(f, "N{}{}", self.with(parent), IdentMangler(module.name))?,
            None => write!(f, "R{}", IdentMangler(module.name))?,
        }

        if let Some(end) = self.end {
            write!(f, "{}", IdentMangler(end))?;
        }

        Ok(())
    }
}

#[derive(Clone, Copy)]
struct IdentMangler(Ident);

impl Display for IdentMangler {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let str = self.0.str();
        if str.is_ascii() {
            write!(f, "{}{str}", str.len())
        } else {
            let punycode = punycode::encode_str(&str).unwrap().replace("-", "_");
            write!(f, "u{}{punycode}", str.len())
        }
    }
}

/// Produces a mangled symbol for the given type.
pub fn mangle_ty(pkg: &Package, ty_ctx: &TyContext, ty: TyId) -> String {
    format!("{PREFIX}{}", TyMangler::new(pkg, ty_ctx, ty))
}

#[derive(Clone, Copy)]
struct TyMangler<'ext> {
    pkg: &'ext Package,
    ty_ctx: &'ext TyContext,
    ty: TyId,
}

impl<'ext> TyMangler<'ext> {
    fn new(pkg: &'ext Package, ty_ctx: &'ext TyContext, ty: TyId) -> Self {
        Self { pkg, ty_ctx, ty }
    }

    fn with(self, ty: TyId) -> Self {
        Self { ty, ..self }
    }
}

impl Display for TyMangler<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.ty_ctx.get_ty(self.ty) {
            Ty::Int => "i".fmt(f),
            Ty::UInt => "u".fmt(f),
            Ty::Byte => "h".fmt(f),
            Ty::Float => "f".fmt(f),
            Ty::Bool => "b".fmt(f),
            Ty::Tuple(elem_tys) => {
                "T".fmt(f)?;
                for ty in elem_tys {
                    self.with(*ty).fmt(f)?;
                }
                "E".fmt(f)
            }
            Ty::Func(params, ret_ty) => {
                for param in params {
                    if param.mutable {
                        "M".fmt(f)?;
                    }
                    self.with(param.ty).fmt(f)?;
                }
                "E".fmt(f)?;
                self.with(*ret_ty).fmt(f)
            }
            Ty::Named { ident, module, .. } => {
                PathMangler::new(self.pkg, *module, Some(*ident)).fmt(f)
            }
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Tried to mangle type variable"),
        }
    }
}
