//! Data structures used by the rest of the compiler.

use std::{
    fmt::{self, Display, Formatter},
    num::NonZeroU32,
};

pub mod arena;
//pub mod interner;
pub mod unif_table;

/// The internal data of an ID, used by a number of data structures.
///
/// Has a niche.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct IdData(NonZeroU32);

impl IdData {
    /// Constructs a new [`IdData`] representing the given index.
    pub fn new(index: usize) -> Self {
        // SAFETY: The value just had 1 added to it, so it can't be zero.
        unsafe { Self::new_raw(index.strict_add(1)) }
    }

    /// The index represented by the returned `IdData` is 1 less than the provided value.
    ///
    /// SAFETY: `value` must not be zero.
    unsafe fn new_raw(value: usize) -> Self {
        let value = u32::try_from(value).expect("key overflow");
        unsafe { Self(NonZeroU32::new_unchecked(value)) }
    }

    /// Returns the index represented.
    pub fn index(self) -> usize {
        usize::try_from(self.0.get() - 1).expect("16-bit platforms are not supported")
    }
}

impl Display for IdData {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        self.0.get().fmt(f)
    }
}

/// Marker trait for IDs. Should not be implemented directly, use [`new_id_type`] instead.
pub trait Id: From<IdData> + Into<IdData> + Copy + Eq {}

/// Creates a new ID type.
#[macro_export]
macro_rules! new_id_type {
    { $(#[$outer:meta])* $vis:vis struct $name:ident; } => {
        $(#[$outer])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        $vis struct $name($crate::IdData);

        impl ::std::convert::From<$crate::IdData> for $name {
            fn from(value: $crate::IdData) -> Self {
                $name(value)
            }
        }

        impl ::std::convert::From<$name> for $crate::IdData {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl $crate::Id for $name {}
    };
}
