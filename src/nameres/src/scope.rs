use foldhash::{HashMap, HashMapExt as _};
use ident::Ident;
use irs::{
    ModuleId, Package,
    ast::PathSlice,
    hir::{Ty, TyId, VarId},
};
use slotmap::SecondaryMap;

use crate::error::{ErrorKind, ItemKind};

#[derive(Debug)]
pub struct ScopeInfo<'pkg> {
    package: &'pkg Package,
    module: ModuleId,
    locals: Vec<HashMap<Ident, VarId>>,
    modules: SecondaryMap<ModuleId, ModuleScope>,
    assocs: HashMap<(Ty, Ident), (Visibility, VarId)>,
}

impl<'pkg> ScopeInfo<'pkg> {
    pub fn new(package: &'pkg Package) -> Self {
        let mut modules = SecondaryMap::new();
        modules.insert(package.root(), Self::new_module(package, package.root()));
        Self {
            package,
            module: package.root(),
            locals: Vec::new(),
            modules,
            assocs: HashMap::new(),
        }
    }

    pub fn push_scope(&mut self) {
        self.locals.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        self.locals.pop();
    }

    pub const fn module(&self) -> ModuleId {
        self.module
    }

    pub fn set_module(&mut self, module: ModuleId) {
        self.module = module;
        self.modules
            .entry(module)
            .expect("key still valid")
            .or_insert_with(|| Self::new_module(self.package, module));
    }

    fn new_module(package: &Package, module: ModuleId) -> ModuleScope {
        let mut scope = ModuleScope::default();
        scope.modules.extend(
            package
                .get(module)
                .children
                .iter()
                .copied()
                .map(|module| (package.get(module).name, (Visibility::Public, module))),
        );
        scope
    }

    pub fn add_ty(&mut self, vis: Visibility, ident: Ident, id: TyId) -> Option<TyId> {
        self.mod_scope_mut()
            .tys
            .insert(ident, (vis, id))
            .map(|(_, id)| id)
    }

    pub fn add_def(&mut self, vis: Visibility, ident: Ident, id: VarId) -> Option<VarId> {
        self.mod_scope_mut()
            .defs
            .insert(ident, (vis, id))
            .map(|(_, id)| id)
    }

    pub fn add_assoc_def(
        &mut self,
        ty: Ty,
        vis: Visibility,
        ident: Ident,
        id: VarId,
    ) -> Option<VarId> {
        self.assocs.insert((ty, ident), (vis, id)).map(|(_, id)| id)
    }

    pub fn add_var(&mut self, ident: Ident, id: VarId) -> Option<VarId> {
        self.locals
            .last_mut()
            .expect("Tried to add local variable with no local scope on stack")
            .insert(ident, id)
    }

    pub fn resolve_ty(&self, path: PathSlice<'_>) -> Result<TyId, ErrorKind> {
        let (head, tail) = path.split();
        let base = match head {
            Some(head) => self.resolve_module(head)?,
            None => self.module,
        };
        let (vis, id) = self.modules[base]
            .get_ty(tail)
            .ok_or_else(|| ErrorKind::UnknownName(ItemKind::Type, tail))?;
        if base != self.module && vis == Visibility::Private {
            Err(ErrorKind::PrivateItem(ItemKind::Type, tail))
        } else {
            Ok(id)
        }
    }

    pub fn resolve_var(&self, path: PathSlice<'_>) -> Result<VarId, ErrorKind> {
        match path.split() {
            (Some(head), tail) => {
                let def = match self.resolve_module(head) {
                    Ok(module) => self.modules[module].get_def(tail),
                    Err(_) => match self.resolve_ty(head) {
                        Ok(ty) => self.assocs.get(&(Ty::Named(ty), tail)).copied(),
                        Err(_) => {
                            return Err(ErrorKind::UnknownName(ItemKind::Module, head.last()));
                        }
                    },
                };

                let (vis, id) = def.ok_or_else(|| ErrorKind::UnknownName(ItemKind::Value, tail))?;

                if vis == Visibility::Private {
                    Err(ErrorKind::PrivateItem(ItemKind::Value, tail))
                } else {
                    Ok(id)
                }
            }
            (None, tail) => self
                .locals
                .iter()
                .rev()
                .find_map(|scope| scope.get(&tail).copied())
                .or_else(|| self.mod_scope().get_def(tail).map(|(_, id)| id))
                .ok_or_else(|| ErrorKind::UnknownName(ItemKind::Value, tail)),
        }
    }

    fn resolve_module(&self, path: PathSlice<'_>) -> Result<ModuleId, ErrorKind> {
        match path.split() {
            (Some(head), tail) => {
                let base = self.resolve_module(head)?;
                let (vis, id) = self.modules[base]
                    .get_mod(tail)
                    .ok_or(ErrorKind::UnknownName(ItemKind::Module, tail))?;

                if vis == Visibility::Private {
                    Err(ErrorKind::PrivateItem(ItemKind::Module, tail))
                } else {
                    Ok(id)
                }
            }
            (None, tail) => self
                .mod_scope()
                .get_mod(tail)
                .ok_or(ErrorKind::UnknownName(ItemKind::Module, tail))
                .map(|(_, id)| id),
        }
    }

    pub fn import(&mut self, path: PathSlice<'_>) -> Result<(), ErrorKind> {
        let (head, tail) = path.split();
        let base = match head {
            Some(head) => self.resolve_module(head)?,
            None => self.module,
        };
        let module = self.modules[base].get_mod(tail);
        let ty = self.modules[base].get_ty(tail);
        let def = self.modules[base].get_def(tail);

        if module.is_none() && ty.is_none() && def.is_none() {
            return Err(ErrorKind::UnknownName(ItemKind::Unknown, tail));
        }

        if let Some((vis, module)) = module {
            if base != self.module && vis == Visibility::Private {
                return Err(ErrorKind::PrivateItem(ItemKind::Module, tail));
            }

            if self
                .mod_scope_mut()
                .modules
                .insert(tail, (Visibility::Private, module))
                .is_some()
            {
                return Err(ErrorKind::DuplicateItem(ItemKind::Module, tail));
            }
        }

        if let Some((vis, ty)) = ty {
            if base != self.module && vis == Visibility::Private {
                return Err(ErrorKind::PrivateItem(ItemKind::Type, tail));
            }

            if self
                .mod_scope_mut()
                .tys
                .insert(tail, (Visibility::Private, ty))
                .is_some()
            {
                return Err(ErrorKind::DuplicateItem(ItemKind::Type, tail));
            }
        }

        if let Some((vis, def)) = def {
            if base != self.module && vis == Visibility::Private {
                return Err(ErrorKind::PrivateItem(ItemKind::Value, tail));
            }

            if self
                .mod_scope_mut()
                .defs
                .insert(tail, (Visibility::Private, def))
                .is_some()
            {
                return Err(ErrorKind::DuplicateItem(ItemKind::Value, tail));
            }
        }

        Ok(())
    }

    fn mod_scope(&self) -> &ModuleScope {
        &self.modules[self.module]
    }

    fn mod_scope_mut(&mut self) -> &mut ModuleScope {
        &mut self.modules[self.module]
    }
}

#[derive(Debug, Default)]
struct ModuleScope {
    defs: HashMap<Ident, (Visibility, VarId)>,
    tys: HashMap<Ident, (Visibility, TyId)>,
    modules: HashMap<Ident, (Visibility, ModuleId)>,
}

impl ModuleScope {
    pub fn get_mod(&self, ident: Ident) -> Option<(Visibility, ModuleId)> {
        self.modules.get(&ident).copied()
    }

    pub fn get_ty(&self, ident: Ident) -> Option<(Visibility, TyId)> {
        self.tys.get(&ident).copied()
    }

    pub fn get_def(&self, ident: Ident) -> Option<(Visibility, VarId)> {
        self.defs.get(&ident).copied()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Public,
}
