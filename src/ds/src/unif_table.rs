//! A disjoint set for union-find algorithms. Based on [`ena`][https://crates.io/crates/ena].

use std::cmp::Ordering;

use crate::{Id, arena::Arena};

/// A disjoint set for union-find algorithms.
///
/// Uses the standard union-find algorithm with path compression:
/// <http://en.wikipedia.org/wiki/Disjoint-set_data_structure>.
pub struct UnifTable<I: Id>(Arena<I, VarValue<I>>);

impl<I: Id> UnifTable<I> {
    /// Constructs a new, empty [`UnifTable`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of IDs created so far.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Creates a fresh ID.
    pub fn new_id(&mut self) -> I {
        self.0.insert_with_id(|id| VarValue {
            parent: id,
            rank: 0,
        })
    }

    /// Reserves space for at least `additional` more IDs.
    pub fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }

    /// Returns the root of the given ID.
    pub fn find(&mut self, id: I) -> I {
        let parent = self.0[id].parent;
        if parent == id {
            id
        } else {
            let root = self.find(parent);
            // Path compression.
            self.0[id].parent = root;
            root
        }
    }

    /// Unions two IDs.
    pub fn unify(&mut self, id_a: I, id_b: I) {
        let root_a = self.find(id_a);
        let root_b = self.find(id_b);

        if root_a == root_b {
            return;
        }

        // The root with the higher rank becomes the parent.
        // If the ranks are equal, an arbitrary root becomes the parent and has it's rank increased by 1.
        match Ord::cmp(&self.0[root_a].rank, &self.0[root_b].rank) {
            Ordering::Greater => self.0[root_b].parent = root_a,
            Ordering::Less => self.0[root_a].parent = root_b,
            Ordering::Equal => {
                self.0[root_a].parent = root_b;
                self.0[root_b].rank += 1;
            }
        }
    }

    /// Returns whether the given IDs have been unioned together.
    pub fn unioned(&mut self, id_a: I, id_b: I) -> bool {
        self.find(id_a) == self.find(id_b)
    }
}

impl<I: Id> Default for UnifTable<I> {
    fn default() -> Self {
        Self(Arena::new())
    }
}

#[derive(Clone, Copy, Debug)]
struct VarValue<I> {
    /// If equal to self, this is a root.
    parent: I,
    /// Max depth (only relevant to root).
    rank: u32,
}
