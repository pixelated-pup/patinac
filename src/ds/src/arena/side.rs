//! Side-arenas using existing IDs.

use std::{
    fmt::{self, Debug, Formatter},
    iter::Enumerate,
    marker::PhantomData,
    ops::{Index, IndexMut},
    slice,
};

use crate::arena::{Id, IdData};

/// A homogenous arena that uses existing IDs.
///
/// Each inserted value is associated with an ID produced by an [`Arena`],
/// which can later be used to access the value again.
///
/// Does not perform any interning.
#[derive(Clone)]
pub struct SideArena<I, V>(Vec<Option<V>>, PhantomData<I>);

impl<I, V> SideArena<I, V> {
    /// Constructs a new, empty [`SideArena`].
    pub fn new() -> Self {
        Self::default()
    }
}

impl<I: Id, V> SideArena<I, V> {
    /// Inserts a value corresponding to the ID.
    pub fn insert(&mut self, id: I, value: V) {
        *self.get_mut(id) = Some(value)
    }

    /// Returns a reference to the value corresponding to the given key, if it exists.
    pub fn get(&self, id: I) -> &Option<V> {
        self.0.get(id.into().index()).unwrap_or(&None)
    }

    /// Returns a mutable reference to the value corresponding to the given key, if it exists.
    pub fn get_mut(&mut self, id: I) -> &mut Option<V> {
        let index = id.into().index();
        if index >= self.0.len() {
            self.0.resize_with(index + 1, Option::default);
        }
        &mut self.0[index]
    }

    /// An iterator of all key-value pairs, in insertion order.
    pub fn iter(&self) -> Iter<'_, I, V> {
        Iter {
            iter: self.0.iter().enumerate(),
            _phantom: PhantomData,
        }
    }

    /// An iterator of all key-value pairs, in insertion order. Allows mutation of the values.
    pub fn iter_mut(&mut self) -> IterMut<'_, I, V> {
        IterMut {
            iter: self.0.iter_mut().enumerate(),
            _phantom: PhantomData,
        }
    }
    /// An iterator of all keys, in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = I> {
        (0..self.0.len())
            .map(|index| IdData::new(index).into())
            .filter(|id| self.get(*id).is_some())
    }

    /// An iterator of all values, in insertion order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter().flatten()
    }

    /// An iterator of all values, in insertion order. Allows mutation of the values.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.0.iter_mut().flatten()
    }
}

impl<I, V: Debug> Debug for SideArena<I, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let iter = self
            .0
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.as_ref().map(|v| (i, v)));
        f.debug_map().entries(iter).finish()
    }
}

impl<I, V> Default for SideArena<I, V> {
    fn default() -> Self {
        Self(Vec::new(), PhantomData)
    }
}

impl<I: Id, V> Index<I> for SideArena<I, V> {
    type Output = V;

    fn index(&self, id: I) -> &Self::Output {
        self.get(id).as_ref().unwrap()
    }
}

impl<I: Id, V> IndexMut<I> for SideArena<I, V> {
    fn index_mut(&mut self, id: I) -> &mut Self::Output {
        self.get_mut(id).as_mut().unwrap()
    }
}

impl<'a, I: Id, V> IntoIterator for &'a SideArena<I, V> {
    type Item = (I, &'a V);
    type IntoIter = Iter<'a, I, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, I: Id, V> IntoIterator for &'a mut SideArena<I, V> {
    type Item = (I, &'a mut V);
    type IntoIter = IterMut<'a, I, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// See [`SideArena::iter`].
pub struct Iter<'a, I, V> {
    iter: Enumerate<slice::Iter<'a, Option<V>>>,
    _phantom: PhantomData<I>,
}

impl<'a, I: Id, V> Iterator for Iter<'a, I, V> {
    type Item = (I, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let (index, value) = loop {
            if let (index, Some(value)) = self.iter.next()? {
                break (index, value);
            }
        };
        let id = IdData::new(index).into();
        Some((id, value))
    }
}

/// See [`SideArena::iter_mut`].
pub struct IterMut<'a, I, V> {
    iter: Enumerate<slice::IterMut<'a, Option<V>>>,
    _phantom: PhantomData<I>,
}

impl<'a, I: Id, V> Iterator for IterMut<'a, I, V> {
    type Item = (I, &'a mut V);

    fn next(&mut self) -> Option<Self::Item> {
        let (index, value) = loop {
            if let (index, Some(value)) = self.iter.next()? {
                break (index, value);
            }
        };
        let id = IdData::new(index).into();
        Some((id, value))
    }
}
