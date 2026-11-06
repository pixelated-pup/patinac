use inkwell::{
    attributes::Attribute,
    types::{AnyType, BasicTypeEnum},
};

use crate::CodegenState;

impl<'mir, 'ty_ctx, 'ctx> CodegenState<'mir, 'ty_ctx, 'ctx> {
    pub(crate) fn sret(&self, ty: BasicTypeEnum<'ctx>) -> Attribute {
        self.ctx
            .create_type_attribute(attr_id("sret"), ty.as_any_type_enum())
    }
}

fn attr_id(name: &str) -> u32 {
    let id = Attribute::get_named_enum_kind_id(name);
    assert_ne!(id, 0);
    id
}
