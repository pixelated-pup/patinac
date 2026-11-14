//! Name mangling.
//!
//! All mangled symbols consist exclusively of a-z, A-Z, 0-9, and _.
//! Unicode characters in identifiers are encoded with punycode, and hyphens in the punycode are replaced with underscores.

use std::fmt::Write as _;

use ident::Ident;
use idna::punycode;
use irs::{
    ModuleId, Package,
    ty::{DefId, Parent, Ty, TyContext, TyId},
};

const PREFIX: &str = "_P";

/// Produces a mangled symbol for the given function.
pub fn mangle_func(pkg: &Package, ty_ctx: &TyContext, func: DefId) -> String {
    let mut buf = String::from(PREFIX);
    let func = ty_ctx.def(func);

    match func.parent {
        Some(Parent::Impl(ty)) => mangle_assoc_item(pkg, ty_ctx, ty, func.ident, &mut buf),
        Some(Parent::Def(def)) => {
            let parent = ty_ctx.def(def);
            buf.push('N');
            mangle_module_item(pkg, parent.module, parent.ident, &mut buf);
            mangle_ident(func.ident, &mut buf);
        }
        None => mangle_module_item(pkg, func.module, func.ident, &mut buf),
    }

    buf
}

/// Produces a mangled symbol for an associated item with the given name
pub fn assoc_item_symbol(pkg: &Package, ty_ctx: &TyContext, ty: TyId, ident: Ident) -> String {
    let mut buf = String::from(PREFIX);
    mangle_assoc_item(pkg, ty_ctx, ty, ident, &mut buf);
    buf
}

fn mangle_assoc_item(pkg: &Package, ty_ctx: &TyContext, ty: TyId, ident: Ident, buf: &mut String) {
    buf.push('N');
    mangle_assoc_path(pkg, ty_ctx, ty, buf);
    mangle_ident(ident, buf);
}

fn mangle_assoc_path(pkg: &Package, ty_ctx: &TyContext, ty: TyId, buf: &mut String) {
    buf.push('A');
    mangle_ty(pkg, ty_ctx, ty, buf);
}

/// Produces a mangled symbol for an item with the given name and module
pub fn module_item_symbol(pkg: &Package, module: ModuleId, ident: Ident) -> String {
    let mut buf = String::from(PREFIX);
    mangle_module_item(pkg, module, ident, &mut buf);
    buf
}

fn mangle_module_item(pkg: &Package, module: ModuleId, ident: Ident, buf: &mut String) {
    buf.push('N');
    mangle_module_path(pkg, module, buf);
    mangle_ident(ident, buf);
}

fn mangle_module_path(pkg: &Package, module: ModuleId, buf: &mut String) {
    let module = pkg.get(module);
    match module.parent {
        Some(parent) => {
            buf.push('N');
            mangle_module_path(pkg, parent, buf);
            mangle_ident(module.name, buf);
        }
        None => {
            buf.push('R');
            mangle_ident(module.name, buf);
        }
    }
}

/// Produces a mangled symbol for the given type.
// pub fn mangle_ty(pkg: &Package, ty_ctx: &TyContext, ty: TyId) -> String {
//     format!("{PREFIX}{}", TyMangler::new(pkg, ty_ctx, ty))
// }

fn mangle_ty(pkg: &Package, ty_ctx: &TyContext, ty: TyId, buf: &mut String) {
    match ty_ctx.get_ty(ty) {
        Ty::Int => buf.push('i'),
        Ty::UInt => buf.push('u'),
        Ty::Byte => buf.push('b'),
        Ty::Float => buf.push('f'),
        Ty::Bool => buf.push('l'),
        Ty::Tuple(elem_tys) => {
            buf.push('T');
            for &elem_ty in elem_tys {
                mangle_ty(pkg, ty_ctx, elem_ty, buf);
            }
            buf.push('E');
        }
        Ty::Func(params, ret_ty) => {
            buf.push('F');
            for param in params {
                if param.mutable {
                    buf.push('M');
                }
                mangle_ty(pkg, ty_ctx, param.ty, buf);
            }
            buf.push('E');
            mangle_ty(pkg, ty_ctx, *ret_ty, buf);
        }
        Ty::Named { ident, module, .. } => mangle_module_item(pkg, *module, *ident, buf),
        Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Tried to mangle type variable"),
    }
}

fn mangle_ident(ident: Ident, buf: &mut String) {
    fn debug_assert_valid(str: &str) {
        debug_assert!(
            str.bytes()
                .all(|b| matches!(b, b'a'..b'z' | b'A'..b'Z' | b'0'..b'9' | b'_'))
        );
    }

    let str = ident.str();

    if str.is_ascii() {
        debug_assert_valid(&str);
        let _ = write!(buf, "{}{str}", str.len());
    } else {
        let punycode = punycode::encode_str(&str).unwrap().replace("-", "_");
        debug_assert_valid(&punycode);
        let _ = write!(buf, "u{}{punycode}", str.len());
    }
}
