use std::{
    fmt::{self, Debug, Formatter},
    hash::Hash,
    iter::Enumerate,
    marker::PhantomData,
    ops::Index,
};

use foldhash::fast::RandomState as FoldRandomState;
use ordermap::{OrderSet, set};

use crate::{Id, IdData};

/// An interner.
///
/// Inserting a value into the interner returns an ID for that value,
/// which can later be used to access the value again.
/// If the value already exists, the ID for the previous instance of it will be reused.
#[derive(Clone)]
pub struct Interner<I: Id, V: Eq + Hash>(OrderSet<V, FoldRandomState>, PhantomData<I>);

impl<I: Id, V: Eq + Hash> Interner<I, V> {
    /// Constructs a new, empty [`Interner`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserves space for at least `additional` more values.
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }

    /// Inserts a value, returning an ID for it.
    pub fn insert(&mut self, value: V) -> I {
        let (idx, exists) = self.0.insert_full(value);
        IdData::new(idx).into()
    }

    /// Returns a reference to the value corresponding to the given key.
    pub fn get(&self, id: I) -> &V {
        &self.0[id.into().index()]
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

    /// An iterator of all keys, in insertion order.
    pub fn keys(&self) -> impl Iterator<Item = I> + use<I, V> {
        (0..self.0.len()).map(|index| IdData::new(index).into())
    }

    /// An iterator of all values, in insertion order.
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.0.iter()
    }
}

impl<I: Id, V: Debug + Eq + Hash> Debug for Interner<I, V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.0.iter().enumerate()).finish()
    }
}

impl<I: Id, V: Eq + Hash> Default for Interner<I, V> {
    fn default() -> Self {
        Self(OrderSet::default(), PhantomData)
    }
}

impl<I: Id, V: Eq + Hash> Index<I> for Interner<I, V> {
    type Output = V;

    fn index(&self, id: I) -> &Self::Output {
        self.get(id)
    }
}

impl<'a, I: Id, V: Eq + Hash> IntoIterator for &'a Interner<I, V> {
    type Item = (I, &'a V);
    type IntoIter = Iter<'a, I, V>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// See [`Interner::iter`].
pub struct Iter<'a, I, V> {
    iter: Enumerate<set::Iter<'a, V>>,
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
