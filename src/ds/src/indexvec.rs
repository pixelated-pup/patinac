use std::{
    fmt::Debug,
    marker::PhantomData,
    ops::{Index, IndexMut},
};

macro_rules! new_idx_type {
    ($(#[$outer:meta])* $vis:vis struct $name:ident) => {
        $(#[$outer])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash)]
        $vis struct $name(u32);

        impl From<u32> for $name {
            fn from(idx: u32) -> Self {
                Self(idx)
            }
        }

        impl From<$name> for u32 {
            fn from(value: $name) -> u32 {
                value.0
            }
        }

        impl From<usize> for $name {
            fn from(idx: usize) -> Self {
                Self(idx.try_into().expect("index too big"))
            }
        }

        impl From<$name> for usize {
            fn from(value: $name) -> usize {
                value.0.into()
            }
        }

        impl Idx for $name {}
    };
    () => {}
}

pub struct IndexVec<I, T>(Vec<T>, PhantomData<I>);

pub trait Idx: Copy + From<u32> + Into<u32> + From<usize> + Into<usize> {}

impl<I, T: Clone> Clone for IndexVec<I, T> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), PhantomData)
    }
}

impl<I, T> Default for IndexVec<I, T> {
    fn default() -> Self {
        Self(Vec::new(), PhantomData)
    }
}

impl<I: Idx + Debug, T: Debug> Debug for IndexVec<I, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.0.iter().enumerate().map(|(i, v)| (I::from(i), v));
        f.debug_map().entries(entries).finish()
    }
}

impl<I: Idx, T> Index<I> for IndexVec<I, T> {
    type Output = T;

    fn index(&self, index: I) -> &Self::Output {
        self.get(index)
    }
}

impl<I: Idx, T> IndexMut<I> for IndexVec<I, T> {
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        self.get_mut(index)
    }
}

impl<I: Idx, T> IndexVec<I, T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, value: T) -> I {
        let idx = self.0.len();
        self.0.push(value);
        I::from(idx)
    }

    pub fn insert(&mut self, idx: I, value: T) {}

    pub fn get(&self, idx: I) -> &T {
        let idx: usize = idx.into();
        &self.0[idx]
    }

    pub fn get_mut(&mut self, idx: I) -> &mut T {
        let idx: usize = idx.into();
        &mut self.0[idx]
    }
}
