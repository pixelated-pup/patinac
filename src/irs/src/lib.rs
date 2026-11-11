//! All of the IRs used throughout the compiler.

pub mod ast;
pub mod hir;
pub mod mir;
pub mod ty;

use std::fmt::Write as _;

use slotmap::{SlotMap, new_key_type};

use ident::Ident;

/// A whole package, comprised of one or more modules. Generic over the contents of each module.
#[derive(Debug, Default)]
pub struct Package {
    modules: SlotMap<ModuleId, Module>,
    root_id: ModuleId,
}

impl Package {
    /// Creates a new package with the given root module.
    pub fn new(root: Module) -> Self {
        let mut modules = SlotMap::default();
        let root_id = modules.insert(root);
        Self { modules, root_id }
    }

    /// Returns the id for the root module.
    pub const fn root(&self) -> ModuleId {
        self.root_id
    }

    /// Returns a reference to the module associated with the given ID.
    pub fn get(&self, id: ModuleId) -> &Module {
        &self.modules[id]
    }

    /// Returns a mutable reference to the module associated with the given ID.
    pub fn get_mut(&mut self, id: ModuleId) -> &mut Module {
        &mut self.modules[id]
    }

    /// Inserts the provided module into the package, returning an ID for it.
    pub fn insert(&mut self, module: Module) -> ModuleId {
        self.modules.insert(module)
    }

    /// Returns the name of this package.
    pub fn name(&self) -> Ident {
        self.modules[self.root_id].name
    }

    /// Returns the textual path to the given module.
    pub fn path_to(&self, module: ModuleId) -> String {
        fn inner(this: &Package, module: ModuleId, buf: &mut String) {
            let module = &this.modules[module];
            if let Some(parent) = module.parent {
                inner(this, parent, buf);
                buf.push_str("::");
            }
            let _ = write!(buf, "{}", module.name);
        }

        let mut buf = String::new();
        inner(self, module, &mut buf);
        buf
    }
}

new_key_type! {
    /// An ID for a module.
    pub struct ModuleId;
}

/// A module.
#[derive(Debug)]
pub struct Module {
    /// The parent module of this module, unless this is the root module.
    pub parent: Option<ModuleId>,
    /// The name of this module, determined by it's filename.
    pub name: Ident,
    /// Any child modules of this module.
    pub children: Vec<ModuleId>,
}
