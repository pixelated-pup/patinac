use crate::{Id, IdData};

pub struct UnifTable<I>(Vec<VarValue<I>>);

impl<I> Default for UnifTable<I> {
    fn default() -> Self {
        Self(Vec::new())
    }
}
impl<I> UnifTable<I> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<I> UnifTable<I> {
    /// Returns the number of keys created so far.
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl<I: Id> UnifTable<I> {
    /// Obtains the current value for a particular key.
    /// Not for end-users; they can use `probe_value`.
    fn value(&self, id: I) -> &VarValue<I> {
        &self.0[id.into().index()]
    }

    /// Creates a fresh key.
    pub fn new_key(&mut self) -> I {
        let key = IdData::new(self.0.len()).into();
        self.0.push(VarValue::new_var(key));
        key
    }

    /// Reserve memory for `num_new_keys` to be created. Does not
    /// actually create the new keys; you must then invoke `new_key`.
    pub fn reserve(&mut self, num_new_keys: usize) {
        self.0.reserve(num_new_keys);
    }

    /// Clears all unifications that have been performed, resetting to
    /// the initial state.
    pub fn reset_unifications(&mut self) {
        for (index, slot) in self.0.iter_mut().enumerate() {
            let key = IdData::new(index).into();
            *slot = VarValue::new_var(key);
        }
    }

    /// Find the root node for `vid`. This uses the standard
    /// union-find algorithm with path compression:
    /// <http://en.wikipedia.org/wiki/Disjoint-set_data_structure>.
    ///
    /// NB. This is a building-block operation and you would probably
    /// prefer to call `probe` below.
    ///
    /// This is an always-inlined version of this function for the hot
    /// callsites. `uninlined_get_root_key` is the never-inlined version.
    #[inline(always)]
    fn inlined_get_root_key(&mut self, vid: I) -> I {
        let v = self.value(vid);
        if v.parent == vid {
            return vid;
        }

        let redirect = v.parent;
        let root_key: I = self.uninlined_get_root_key(redirect);
        if root_key != redirect {
            // Path compression
            self.update_value(vid, |value| value.parent = root_key);
        }

        root_key
    }

    // This is a never-inlined version of this function for cold callsites.
    // 'inlined_get_root_key` is the always-inlined version.
    #[inline(never)]
    fn uninlined_get_root_key(&mut self, vid: I) -> I {
        self.inlined_get_root_key(vid)
    }

    fn update_value<OP>(&mut self, key: I, op: OP)
    where
        OP: FnOnce(&mut VarValue<I>),
    {
        self.values.update(key.index() as usize, op);
        debug!("Updated variable {:?} to {:?}", key, self.value(key));
    }

    /// Either redirects `node_a` to `node_b` or vice versa, depending
    /// on the relative rank. The value associated with the new root
    /// will be `new_value`.
    ///
    /// NB: This is the "union" operation of "union-find". It is
    /// really more of a building block. If the values associated with
    /// your key are non-trivial, you would probably prefer to call
    /// `unify_var_var` below.
    fn unify_roots(&mut self, key_a: I, key_b: I, new_value: S::Value) {
        debug!("unify(key_a={:?}, key_b={:?})", key_a, key_b);

        let rank_a = self.value(key_a).rank;
        let rank_b = self.value(key_b).rank;
        if let Some((new_root, redirected)) = I::order_roots(
            key_a,
            &self.value(key_a).value,
            key_b,
            &self.value(key_b).value,
        ) {
            // compute the new rank for the new root that they chose;
            // this may not be the optimal choice.
            let new_rank = if new_root == key_a {
                debug_assert!(redirected == key_b);
                if rank_a > rank_b { rank_a } else { rank_b + 1 }
            } else {
                debug_assert!(new_root == key_b);
                debug_assert!(redirected == key_a);
                if rank_b > rank_a { rank_b } else { rank_a + 1 }
            };
            self.redirect_root(new_rank, redirected, new_root, new_value);
        } else if rank_a > rank_b {
            // a has greater rank, so a should become b's parent,
            // i.e., b should redirect to a.
            self.redirect_root(rank_a, key_b, key_a, new_value);
        } else if rank_a < rank_b {
            // b has greater rank, so a should redirect to b.
            self.redirect_root(rank_b, key_a, key_b, new_value);
        } else {
            // If equal, redirect one to the other and increment the
            // other's rank.
            self.redirect_root(rank_a + 1, key_a, key_b, new_value);
        }
    }

    /// Internal method to redirect `old_root_key` (which is currently
    /// a root) to a child of `new_root_key` (which will remain a
    /// root). The rank and value of `new_root_key` will be updated to
    /// `new_rank` and `new_value` respectively.
    fn redirect_root(
        &mut self,
        new_rank: u32,
        old_root_key: I,
        new_root_key: I,
        new_value: S::Value,
    ) {
        self.update_value(old_root_key, |old_root_value| {
            old_root_value.redirect(new_root_key);
        });
        self.update_value(new_root_key, |new_root_value| {
            new_root_value.root(new_rank, new_value);
        });
    }
}

/// ////////////////////////////////////////////////////////////////////////
/// Public API

impl<S, K, V> UnificationTable<S>
where
    S: UnificationStoreBase<Key = K, Value = V>,
    K: UnifyKey<Value = V>,
    V: UnifyValue,
{
    /// Obtains current value for key without any pointer chasing; may return `None` if key has been union'd.
    #[inline]
    pub fn try_probe_value<'a, K1>(&'a self, id: K1) -> Option<&'a V>
    where
        K1: Into<K>,
        K: 'a,
    {
        let id = id.into();
        let v = self.value(id);
        if v.parent == id {
            return Some(&v.value);
        }
        None
    }
}

impl<S, K, V> UnificationTable<S>
where
    S: UnificationStoreMut<Key = K, Value = V>,
    K: UnifyKey<Value = V>,
    V: UnifyValue,
{
    /// Unions two keys without the possibility of failure; only
    /// applicable when unify values use `NoError` as their error
    /// type.
    pub fn union<K1, K2>(&mut self, a_id: K1, b_id: K2)
    where
        K1: Into<K>,
        K2: Into<K>,
        V: UnifyValue<Error = NoError>,
    {
        self.unify_var_var(a_id, b_id).unwrap();
    }

    /// Unions a key and a value without the possibility of failure;
    /// only applicable when unify values use `NoError` as their error
    /// type.
    pub fn union_value<K1>(&mut self, id: K1, value: V)
    where
        K1: Into<K>,
        V: UnifyValue<Error = NoError>,
    {
        self.unify_var_value(id, value).unwrap();
    }

    /// Given two keys, indicates whether they have been unioned together.
    pub fn unioned<K1, K2>(&mut self, a_id: K1, b_id: K2) -> bool
    where
        K1: Into<K>,
        K2: Into<K>,
    {
        self.find(a_id) == self.find(b_id)
    }

    /// Given a key, returns the (current) root key.
    pub fn find<K1>(&mut self, id: K1) -> K
    where
        K1: Into<K>,
    {
        let id = id.into();
        self.uninlined_get_root_key(id)
    }

    /// Unions together two variables, merging their values. If
    /// merging the values fails, the error is propagated and this
    /// method has no effect.
    pub fn unify_var_var<K1, K2>(&mut self, a_id: K1, b_id: K2) -> Result<(), V::Error>
    where
        K1: Into<K>,
        K2: Into<K>,
    {
        let a_id = a_id.into();
        let b_id = b_id.into();

        let root_a = self.uninlined_get_root_key(a_id);
        let root_b = self.uninlined_get_root_key(b_id);

        if root_a == root_b {
            return Ok(());
        }

        let combined = V::unify_values(&self.value(root_a).value, &self.value(root_b).value)?;

        Ok(self.unify_roots(root_a, root_b, combined))
    }

    /// Sets the value of the key `a_id` to `b`, attempting to merge
    /// with the previous value.
    pub fn unify_var_value<K1>(&mut self, a_id: K1, b: V) -> Result<(), V::Error>
    where
        K1: Into<K>,
    {
        let a_id = a_id.into();
        let root_a = self.uninlined_get_root_key(a_id);
        let value = V::unify_values(&self.value(root_a).value, &b)?;
        self.update_value(root_a, |node| node.value = value);
        Ok(())
    }

    /// Returns the current value for the given key. If the key has
    /// been union'd, this will give the value from the current root.
    pub fn probe_value<K1>(&mut self, id: K1) -> V
    where
        K1: Into<K>,
    {
        self.inlined_probe_value(id)
    }

    // An always-inlined version of `probe_value`, for hot callsites.
    #[inline(always)]
    pub fn inlined_probe_value<K1>(&mut self, id: K1) -> V
    where
        K1: Into<K>,
    {
        let id = id.into();
        let id = self.inlined_get_root_key(id);
        self.value(id).value.clone()
    }

    // An always-inlined version of `probe_value`, for hot callsites.
    // And returns the root key of the given key along with the value.
    #[inline(always)]
    pub fn inlined_probe_key_value<K1>(&mut self, id: K1) -> (K, V)
    where
        K1: Into<K>,
    {
        let id = id.into();
        let id = self.inlined_get_root_key(id);
        (id, self.value(id).value.clone())
    }
}

#[derive(Clone, Copy, Debug)]
struct VarValue<I> {
    /// If equal to self, this is a root.
    parent: I,
    /// Max depth (only relevant to root).
    rank: u32,
}

impl<I: Id> VarValue<I> {
    fn new(parent: I, rank: u32) -> Self {
        VarValue {
            parent, // this is a root.
            rank,
        }
    }

    fn new_var(key: I) -> Self {
        VarValue::new(key, 0)
    }

    fn redirect(&mut self, to: I) {
        self.parent = to;
    }

    fn root(&mut self, rank: u32) {
        self.rank = rank;
    }
}
