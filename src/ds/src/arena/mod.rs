//! Arenas.

pub mod side;
pub use side::SideArena;

use std::{
    fmt::{self, Debug, Formatter},
    iter::Enumerate,
    marker::PhantomData,
    ops::{Index, IndexMut},
    slice,
};

use crate::{Id, IdData};

/// A homogenous arena.
///
/// Inserting a value into the arena returns a fresh ID for that value,
/// which can later be used to access the value again.
///
/// Does not perform any interning.
#[derive(Clone)]
pub struct Arena<I, V>(Vec<V>, PhantomData<I>);

impl<I, V> Arena<I, V> {
    /// Constructs a new, empty [`Arena`].
    pub fn new() -> Self {
        Self::default()
    }
}

impl<I: Id, V> Arena<I, V> {
    /// Inserts a value, returning a fresh ID corresponding to it.
    pub fn insert(&mut self, value: V) -> I {
        self.0.push(value);
        // SAFETY: The vector was just pushed to, so the length can't be zero.
        unsafe { IdData::new_raw(self.0.len()) }.into()
    }

    /// Returns a reference to the value corresponding to the given key.
    pub fn get(&self, id: I) -> &V {
        &self.0[id.into().index()]
    }

    /// Returns a mutable reference to the value corresponding to the given key.
    pub fn get_mut(&mut self, id: I) -> &mut V {
        &mut self.0[id.into().index()]
    }

    /// The number of values stored.
    pub fn len(&self) -> usize {
        self.0.len()
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
    pub fn keys(&self) -> impl Iterator<Item = I> + use<I, V> {
        (0..self.0.len()).map(|index| IdData::new(index).into())
    }

    /// An iterator of all values, in insertion order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter()
    }

    /// An iterator of all values, in insertion order. Allows mutation of the values.
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut V> {
        self.0.iter_mut()
    }
}

impl<I, V: Debug> Debug for Arena<I, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.0.iter().enumerate()).finish()
    }
}

impl<I, V> Default for Arena<I, V> {
    fn default() -> Self {
        Self(Vec::new(), PhantomData)
    }
}

impl<I: Id, V> Index<I> for Arena<I, V> {
    type Output = V;

    fn index(&self, id: I) -> &Self::Output {
        self.get(id)
    }
}

impl<I: Id, V> IndexMut<I> for Arena<I, V> {
    fn index_mut(&mut self, id: I) -> &mut Self::Output {
        self.get_mut(id)
    }
}

impl<'a, I: Id, V> IntoIterator for &'a Arena<I, V> {
    type Item = (I, &'a V);
    type IntoIter = Iter<'a, I, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, I: Id, V> IntoIterator for &'a mut Arena<I, V> {
    type Item = (I, &'a mut V);
    type IntoIter = IterMut<'a, I, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

pub struct Iter<'a, I, V> {
    iter: Enumerate<slice::Iter<'a, V>>,
    _phantom: PhantomData<I>,
}

impl<'a, I: Id, V> Iterator for Iter<'a, I, V> {
    type Item = (I, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        let (index, value) = self.iter.next()?;
        let id = IdData::new(index).into();
        Some((id, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

pub struct IterMut<'a, I, V> {
    iter: Enumerate<slice::IterMut<'a, V>>,
    _phantom: PhantomData<I>,
}

impl<'a, I: Id, V> Iterator for IterMut<'a, I, V> {
    type Item = (I, &'a mut V);

    fn next(&mut self) -> Option<(I, &'a mut V)> {
        let (index, value) = self.iter.next()?;
        let id = IdData::new(index).into();
        Some((id, value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}
