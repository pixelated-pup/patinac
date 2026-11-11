//! Name mangling.
//!
//! All mangled symbols consist exclusively of a-z, A-Z, 0-9, and _.
//! Unicode characters in identifiers are encoded with punycode, and hyphens in the punycode are replaced with underscores.

use std::fmt::{self, Display, Formatter, Write as _};

use ident::Ident;
use idna::punycode;
use irs::{
    ModuleId, Package,
    ty::{DefId, Parent, Ty, TyContext, TyId},
};

const PREFIX: &str = "_P";

pub struct SymbolBuilder<'ext> {
    pkg: &'ext Package,
    ty_ctx: &'ext TyContext,
    buf: String,
}

impl<'ext> SymbolBuilder<'ext> {
    pub fn new(pkg: &'ext Package, ty_ctx: &'ext TyContext) -> Self {
        Self {
            pkg,
            ty_ctx,
            buf: String::from(PREFIX),
        }
    }
}

/// Produces a mangled symbol for the given function.
pub fn mangle_func(pkg: &Package, ty_ctx: &TyContext, func: DefId) -> String {
    let func = ty_ctx.def(func);
    match func.parent {
        Some(Parent::Impl(ty)) => {
            format!(
                "{PREFIX}A{}{}",
                TyMangler::new(pkg, ty_ctx, ty),
                IdentMangler(func.ident),
            )
        }
        Some(Parent::Def(def)) => {
            let parent = ty_ctx.def(def);
            PathBuilder::new(pkg, module)
            format!(
                "{PREFIX}{}{}",
                PathMangler::new(pkg, parent.module, Some(parent.ident)),
                IdentMangler(func.ident),
            )
        }
        None => {
            format!(
                "{PREFIX}{}",
                PathMangler::new(pkg, func.module, Some(func.ident)),
            )
        }
    }
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
}

impl<'ext> PathMangler<'ext> {
    fn new(pkg: &'ext Package, module: ModuleId) -> Self {
        Self { pkg, module }
    }

    fn with(self, module: ModuleId) -> Self {
        Self { module, ..self }
    }
}

impl Display for PathMangler<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let module = self.pkg.get(self.module);
        match module.parent {
            Some(parent) => write!(f, "N{}{}", self.with(parent), IdentMangler(module.name)),
            None => write!(f, "R{}", IdentMangler(module.name)),
        }
    }
}

fn mangle_module_path(pkg: &Package, module: ModuleId, buf: &mut String) {
    let module = pkg.get(module);
    match module.parent {
        Some(parent) => {
            buf.push('N');
            mangle_module_path(pkg, parent, buf);
            let _ = write!(buf, "{}", IdentMangler(module.name));
        }
        None => {
            let _ = write!(buf, "R{}", IdentMangler(module.name));
        }
    }
}

struct PathBuilder<'ext> {
    pkg: &'ext Package,
    buf: String,
}

impl<'ext> PathBuilder<'ext> {
    fn new(pkg: &'ext Package, module: ModuleId) -> Self {
        fn mangle_module(pkg: &Package, module: ModuleId, buf: &mut String) {
            let module = pkg.get(module);
            match module.parent {
                Some(parent) => {
                    buf.push('N');
                    mangle_module(pkg, parent, buf);
                    let _ = write!(buf, "{}", IdentMangler(module.name));
                }
                None => {
                    let _ = write!(buf, "R{}", IdentMangler(module.name));
                }
            }
        }

        let mut buf = String::new();
        mangle_module(pkg, module, &mut buf);
        Self { pkg, buf }
    }

    fn add(mut self, ident: Ident) -> Self {
        let _ = write!(self.buf, "{}", IdentMangler(ident));
        self
    }

    fn finish(self) -> String {
        self.buf
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
            Ty::Byte => "b".fmt(f),
            Ty::Float => "f".fmt(f),
            Ty::Bool => "l".fmt(f),
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
