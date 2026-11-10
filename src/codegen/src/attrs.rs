use inkwell::{
    attributes::Attribute,
    types::{AnyType, BasicTypeEnum},
};

use crate::CodegenState;

macro_rules! attr {
    ($name:ident) => {
        pub(crate) fn $name(&self) -> Attribute {
            self.ctx
                .create_enum_attribute(attr_id(stringify!($name)), 0)
        }
    };
    ($name:ident with ty) => {
        pub(crate) fn $name(&self, ty: BasicTypeEnum<'ctx>) -> Attribute {
            self.ctx
                .create_type_attribute(attr_id(stringify!($name)), ty.as_any_type_enum())
        }
    };
}

fn attr_id(name: &str) -> u32 {
    let id = Attribute::get_named_enum_kind_id(name);
    assert_ne!(id, 0);
    id
}

impl<'ctx> CodegenState<'_, 'ctx> {
    attr!(sret with ty);
    attr!(nonnull);
    attr!(noundef);
}
