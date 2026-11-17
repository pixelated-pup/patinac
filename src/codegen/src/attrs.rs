use inkwell::{
    attributes::Attribute,
    types::{AnyType, BasicTypeEnum},
};

use crate::CodegenState;

#[derive(Copy, Clone)]
#[repr(u8)]
pub enum AllocKind {
    Alloc = 1 << 0,
    Realloc = 1 << 1,
    Free = 1 << 2,
    Uninitialized = 1 << 3,
    Zeroed = 1 << 4,
    Aligned = 1 << 5,
}

impl<'ctx> CodegenState<'_, 'ctx> {
    pub(crate) fn sret(&self, ty: BasicTypeEnum<'ctx>) -> Attribute {
        self.ctx
            .create_type_attribute(attr_id("sret"), ty.as_any_type_enum())
    }

    pub(crate) fn nonnull(&self) -> Attribute {
        self.ctx.create_enum_attribute(attr_id("nonnull"), 0)
    }

    pub(crate) fn noundef(&self) -> Attribute {
        self.ctx.create_enum_attribute(attr_id("noundef"), 0)
    }

    pub(crate) fn allockind(&self, kinds: &[AllocKind]) -> Attribute {
        let val = kinds.iter().fold(0, |acc, kind| acc | *kind as u64);
        self.ctx.create_enum_attribute(attr_id("allockind"), val)
    }
}

fn attr_id(name: &str) -> u32 {
    let id = Attribute::get_named_enum_kind_id(name);
    assert_ne!(id, 0);
    id
}
