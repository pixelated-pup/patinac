use inkwell::{
    types::FunctionType,
    values::{BasicValue, BasicValueEnum, FloatValue, IntValue, PointerValue},
};

use irs::ty::{Ty, TyId};

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum LayoutValue<'mir, 'ctx> {
    Scalar(BasicValueEnum<'ctx>),
    Fields(&'mir [TyId], PointerValue<'ctx>),
    Closure(FunctionType<'ctx>, PointerValue<'ctx>),
    Zst,
}

impl<'mir, 'ctx> LayoutValue<'mir, 'ctx> {
    pub fn int<B: BasicValue<'ctx> + Copy>(int: B) -> Self {
        assert!(int.as_basic_value_enum().is_int_value());
        Self::Scalar(int.as_basic_value_enum())
    }

    pub fn float<B: BasicValue<'ctx> + Copy>(float: B) -> Self {
        assert!(float.as_basic_value_enum().is_float_value());
        Self::Scalar(float.as_basic_value_enum())
    }

    pub fn as_value(&self) -> BasicValueEnum<'ctx> {
        match self {
            Self::Scalar(value) => *value,
            Self::Closure(_, ptr) | Self::Fields(_, ptr) => ptr.as_basic_value_enum(),
            Self::Zst => panic!("not a value: {self:?}"),
        }
    }

    pub fn as_scalar(&self) -> BasicValueEnum<'ctx> {
        match self {
            Self::Scalar(value) => *value,
            _ => panic!("not a scalar: {self:?}"),
        }
    }

    pub fn as_int(&self) -> IntValue<'ctx> {
        match self {
            Self::Scalar(value) => value.into_int_value(),
            _ => panic!("not an int: {self:?}"),
        }
    }

    pub fn as_float(&self) -> FloatValue<'ctx> {
        match self {
            Self::Scalar(value) => value.into_float_value(),
            _ => panic!("not a float: {self:?}"),
        }
    }

    pub fn int_op<F>(lhs: Self, rhs: Self, op: F) -> Self
    where
        F: FnOnce(IntValue<'ctx>, IntValue<'ctx>) -> IntValue<'ctx>,
    {
        Self::int(op(lhs.as_int(), rhs.as_int()))
    }
}

#[derive(PartialEq, Eq, Debug, Clone, Copy)]
pub enum ScalarKind {
    Int,
    Float,
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
