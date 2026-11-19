use crate::ty::{Ty, TyContext, TyId};

/// Size of a pointer in bytes.
pub const PTR_SIZE: u32 = 8;

#[derive(Debug)]
pub struct Layout {
    /// Inline size, in bytes.
    pub size: u32,
    /// Alignment, in bytes.
    pub align: u32,
    pub uninhabited: bool,
    pub storage_class: StorageClass,
    pub field_offsets: Option<Vec<u32>>,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum StorageClass {
    Zst,
    Indirect,
    Scalar,
}

impl TyContext {
    pub fn layout_of(&mut self, ty: TyId) -> &Layout {
        // self.layouts
        //     .get_mut(ty)
        //     .get_or_insert_with(|| Self::calc_layout_of(ty))
        todo!()
    }

    fn calc_layout_of(&self, ty: TyId) -> Layout {
        match self.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Float => Layout {
                size: 8,
                align: 8,
                uninhabited: false,
                storage_class: StorageClass::Scalar,
                field_offsets: None,
            },
            Ty::Byte | Ty::Bool => Layout {
                size: 1,
                align: 1,
                uninhabited: false,
                storage_class: StorageClass::Scalar,
                field_offsets: None,
            },
            Ty::Tuple(field_tys) => {
                let align = field_tys
                    .iter()
                    .map(|ty| self.align_of(*ty))
                    .max()
                    .unwrap_or(1);

                let mut size = 0;
                let mut offsets = vec![0; field_tys.len()];

                let mut sorted_field_tys = field_tys.clone();
                sorted_field_tys.sort_by_key(|ty| self.layout_of(*ty).align);

                for ty in sorted_field_tys {
                    //     let align = self.align_of(ty);
                    //     let padding = (align - (sum % align)) % align;
                    //     sum + padding + self.size_of(ty)
                }
                // let base_size = field_tys.iter().fold(0, |sum, &ty| {
                //     let align = self.align_of(ty);
                //     let padding = (align - (sum % align)) % align;
                //     sum + padding + self.size_of(ty)
                // });

                // let end_padding = (align - (base_size % align)) % align;
                // base_size + end_padding

                Layout {
                    size,
                    align,
                    uninhabited: (),
                    storage_class: (),
                    field_offsets: Some(offsets),
                }
            }
            Ty::Func(_, _) => Layout {
                size: PTR_SIZE * 4,
                align: PTR_SIZE,
                uninhabited: false,
                storage_class: StorageClass::Indirect,
                field_offsets: None,
            },
            Ty::Named {
                ident,
                opaque,
                fields,
                ctor,
                module,
            } => todo!(),
            Ty::Var(_) | Ty::IntVar(_) => panic!("Attempted to compute layout of type variable"),
        }
    }

    /// Returns the inline size of the provided type, in bytes.
    ///
    /// # Panics
    /// Panics if the provided type is a [`Ty::Var`] or [`Ty::IntVar`].
    pub fn size_of(&self, ty: TyId) -> u32 {
        match self.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Float => 8,
            Ty::Byte | Ty::Bool => 1,
            Ty::Tuple(elem_tys) => {
                todo!()
                // let align = self.align_of(ty);

                // let base_size = field_tys.iter().fold(0, |sum, &ty| {
                //     let align = self.align_of(ty);
                //     let padding = (align - (sum % align)) % align;
                //     sum + padding + self.size_of(ty)
                // });

                // let end_padding = (align - (base_size % align)) % align;
                // base_size + end_padding
            }
            Ty::Func(_, _) => PTR_SIZE * 4, // Function, environment, drop, copy.
            Ty::Named {
                ident,
                opaque,
                fields,
                ctor,
                module,
            } => todo!(),
            Ty::Var(_) | Ty::IntVar(_) => panic!("Attempted to compute size of type variable"),
        }
    }

    /// Returns the alignment of the provided type, in bytes.
    ///
    /// # Panics
    /// Panics if the provided type is a [`Ty::Var`] or [`Ty::IntVar`].
    pub fn align_of(&self, ty: TyId) -> u32 {
        match self.get_ty(ty) {
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
            Ty::Var(_) | Ty::IntVar(_) => panic!("Attempted to compute alignment of type variable"),
        }
    }

    pub fn storage_class(&self, ty: TyId) -> StorageClass {
        match self.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool => StorageClass::Scalar,
            Ty::Tuple(elem_tys) => {
                if elem_tys.is_empty() || elem_tys.iter().all(|ty| self.is_zst(*ty)) {
                    StorageClass::Zst
                } else {
                    StorageClass::Indirect
                }
            }
            Ty::Func(_, _) => StorageClass::Indirect,
            Ty::Named { fields, .. } => {
                if fields.is_empty() || fields.iter().all(|(_, ty)| self.is_zst(*ty)) {
                    StorageClass::Zst
                } else {
                    StorageClass::Indirect
                }
            }
            Ty::Var(_) | Ty::IntVar(_) => {
                panic!("Attempted to compute storage class of type variable")
            }
        }
    }

    pub fn is_indirect(&self, ty: TyId) -> bool {
        self.storage_class(ty) == StorageClass::Indirect
    }

    pub fn is_zst(&self, ty: TyId) -> bool {
        self.storage_class(ty) == StorageClass::Zst
    }

    pub fn is_trivial(&self, ty: TyId) -> bool {
        match self.get_ty(ty) {
            Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool => true,
            Ty::Tuple(elem_tys) => elem_tys.iter().all(|ty| self.is_trivial(*ty)),
            Ty::Func(_, _) => false,
            Ty::Named { fields, .. } => fields.values().all(|ty| self.is_trivial(*ty)),
            Ty::Var(_) | Ty::IntVar(_) => panic!("ICE: Type variables in codegen"),
        }
    }
}
