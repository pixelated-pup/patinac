use inkwell::{
    types::FunctionType,
    values::{BasicValue, BasicValueEnum, FloatValue, IntValue, PointerValue},
};

use irs::ty::{Ty, TyId};

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum LayoutValue<'mir, 'ctx> {
    Scalar(ScalarKind<'ctx>, ScalarLayout<'ctx>),
    Fields(&'mir [TyId], PointerValue<'ctx>),
    Closure(FunctionType<'ctx>, PointerValue<'ctx>),
    Zst,
}

impl<'mir, 'ctx> LayoutValue<'mir, 'ctx> {
    pub fn int<B: BasicValue<'ctx> + Copy>(size: IntSize, int: B) -> Self {
        assert!(int.as_basic_value_enum().is_int_value());
        Self::Scalar(
            ScalarKind::Int(size),
            ScalarLayout::Direct(int.as_basic_value_enum()),
        )
    }

    pub fn indirect_int<B: BasicValue<'ctx> + Copy>(size: IntSize, int: B) -> Self {
        Self::Scalar(
            ScalarKind::Int(size),
            ScalarLayout::Indirect(int.as_basic_value_enum().into_pointer_value()),
        )
    }

    pub fn float<B: BasicValue<'ctx> + Copy>(float: B) -> Self {
        assert!(float.as_basic_value_enum().is_float_value());
        Self::Scalar(
            ScalarKind::Float,
            ScalarLayout::Direct(float.as_basic_value_enum()),
        )
    }

    pub fn indirect_float<B: BasicValue<'ctx> + Copy>(float: B) -> Self {
        Self::Scalar(
            ScalarKind::Float,
            ScalarLayout::Indirect(float.as_basic_value_enum().into_pointer_value()),
        )
    }

    pub fn func_ptr<B: BasicValue<'ctx> + Copy>(ty: FunctionType<'ctx>, ptr: B) -> Self {
        Self::Scalar(
            ScalarKind::FuncPtr(ty),
            ScalarLayout::Direct(ptr.as_basic_value_enum()),
        )
    }

    pub fn indirect_func_ptr<B: BasicValue<'ctx> + Copy>(ty: FunctionType<'ctx>, ptr: B) -> Self {
        Self::Scalar(
            ScalarKind::FuncPtr(ty),
            ScalarLayout::Indirect(ptr.as_basic_value_enum().into_pointer_value()),
        )
    }

    pub fn as_value(&self) -> BasicValueEnum<'ctx> {
        match self {
            Self::Scalar(_, ScalarLayout::Direct(value)) => *value,
            Self::Scalar(_, ScalarLayout::Indirect(ptr))
            | Self::Closure(_, ptr)
            | Self::Fields(_, ptr) => ptr.as_basic_value_enum(),
            Self::Zst => panic!("not a value: {self:?}"),
        }
    }

    pub fn as_scalar(&self) -> BasicValueEnum<'ctx> {
        let Self::Scalar(_, ScalarLayout::Direct(value)) = self else {
            panic!("not a scalar: {self:?}")
        };
        *value
    }

    pub fn as_int(&self) -> IntValue<'ctx> {
        let Self::Scalar(ScalarKind::Int(_), ScalarLayout::Direct(int)) = self else {
            panic!("not an int: {self:?}")
        };
        int.into_int_value()
    }

    pub fn as_float(&self) -> FloatValue<'ctx> {
        let Self::Scalar(ScalarKind::Float, ScalarLayout::Direct(float)) = self else {
            panic!("not a float: {self:?}")
        };
        float.into_float_value()
    }

    pub fn as_fields(&self) -> (&'mir [TyId], PointerValue<'ctx>) {
        match self {
            &Self::Fields(fields, ptr) => (fields, ptr),
            _ => panic!("not a record: {self:?}"),
        }
    }

    pub fn as_pointer(&self) -> PointerValue<'ctx> {
        match self {
            Self::Scalar(_, ScalarLayout::Indirect(ptr))
            | Self::Closure(_, ptr)
            | Self::Fields(_, ptr) => *ptr,
            _ => panic!("not a pointer: {self:?}"),
        }
    }

    pub fn int_op<F>(lhs: Self, rhs: Self, op: F) -> Self
    where
        F: FnOnce(IntValue<'ctx>, IntValue<'ctx>) -> IntValue<'ctx>,
    {
        let Self::Scalar(ScalarKind::Int(size), ScalarLayout::Direct(lhs)) = lhs else {
            panic!("not an int: {lhs:?}")
        };
        Self::int(size, op(lhs.into_int_value(), rhs.as_int()))
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ScalarLayout<'ctx> {
    Direct(BasicValueEnum<'ctx>),
    Indirect(PointerValue<'ctx>),
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ScalarKind<'ctx> {
    Int(IntSize),
    Float,
    FuncPtr(FunctionType<'ctx>),
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum IntSize {
    Bits8,
    Bits64,
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum StorageClass {
    Zst,
    Indirect,
    Scalar,
}

pub const fn storage_class(ty: &Ty) -> StorageClass {
    match ty {
        Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool => StorageClass::Scalar,
        // FIXME: Account for non-capturing functions.
        Ty::Func(_, _) => StorageClass::Indirect,
        Ty::Fields(elem_tys) => {
            if elem_tys.is_empty() {
                StorageClass::Zst
            } else {
                StorageClass::Indirect
            }
        }
    }
}

pub fn indirect(ty: &Ty) -> bool {
    storage_class(ty) == StorageClass::Indirect
}

pub fn zst(ty: &Ty) -> bool {
    storage_class(ty) == StorageClass::Zst
}

pub fn trivial(ty: &Ty) -> bool {
    match ty {
        Ty::Int | Ty::UInt | Ty::Byte | Ty::Float | Ty::Bool => true,
        Ty::Func(_, _) => false, // FIXME: Account for non-capturing functions.
        Ty::Fields(fields) => all_trivial(fields),
    }
}

pub fn all_trivial(fields: &[Ty]) -> bool {
    fields.iter().all(trivial)
}
