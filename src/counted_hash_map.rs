//! CountedHashMap: per-entry reference counting atop HandleHashMap using tokens.

use crate::handle_hash_map::{Handle, HandleHashMap, InsertError};
use crate::tokens::{Count, Token, UsizeCount};

#[derive(Debug)]
pub struct Counted<V> {
    pub refcount: UsizeCount,
    pub value: V,
}

impl<V> Counted<V> {
    pub fn new(value: V, initial: usize) -> Self {
        Self {
            refcount: UsizeCount::new(initial),
            value,
        }
    }
}

pub struct CountedHashMap<K, V, S = std::collections::hash_map::RandomState> {
    pub(crate) inner: HandleHashMap<K, Counted<V>, S>,
}

/// Counted handle carrying a linear token branded to its entry counter instance.
pub struct CountedHandle<'a> {
    pub(crate) handle: Handle,
    pub(crate) token: Token<'a, UsizeCount>, // owned and consumed by put()
}

impl<'a> CountedHandle<'a> {
    pub fn key_ref<'m, K, V, S>(&self, map: &'m CountedHashMap<K, V, S>) -> Option<&'m K>
    where
        K: Eq + core::hash::Hash,
        S: core::hash::BuildHasher + Clone + Default,
    {
        map.inner.handle_key(self.handle)
    }

    pub fn value_ref<'m, K, V, S>(&self, map: &'m CountedHashMap<K, V, S>) -> Option<&'m V>
    where
        K: Eq + core::hash::Hash,
        S: core::hash::BuildHasher + Clone + Default,
    {
        map.inner.handle_value(self.handle).map(|c| &c.value)
    }

    pub fn value_mut<'m, K, V, S>(&self, map: &'m mut CountedHashMap<K, V, S>) -> Option<&'m mut V>
    where
        K: Eq + core::hash::Hash,
        S: core::hash::BuildHasher + Clone + Default,
    {
        map.inner
            .handle_value_mut(self.handle)
            .map(|c| &mut c.value)
    }
}

/// Result of returning a token; indicates whether the entry was removed.
pub enum PutResult<K, V> {
    Live,
    Removed { key: K, value: V },
}

impl<K, V> CountedHashMap<K, V>
where
    K: Eq + core::hash::Hash,
{
    pub fn new() -> Self {
        Self {
            inner: HandleHashMap::new(),
        }
    }
}

impl<K, V, S> CountedHashMap<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            inner: HandleHashMap::with_hasher(hasher),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn find<Q>(&self, q: &Q) -> Option<CountedHandle<'static>>
    where
        K: core::borrow::Borrow<Q>,
        Q: ?Sized + core::hash::Hash + Eq,
    {
        let handle = self.inner.find(q)?;
        let entry = self.inner.handle_value(handle)?;
        let counter = &entry.refcount;
        let token = counter.get();
        Some(CountedHandle { handle, token })
    }

    pub fn contains_key<Q>(&self, q: &Q) -> bool
    where
        K: core::borrow::Borrow<Q>,
        Q: ?Sized + core::hash::Hash + Eq,
    {
        self.inner.contains_key(q)
    }

    /// Insert a new key -> value and mint a token for the returned handle.
    pub fn insert(&mut self, key: K, value: V) -> Result<CountedHandle<'static>, InsertError> {
        let counted = Counted::new(value, 0);
        match self.inner.insert(key, counted) {
            Ok(handle) => {
                let entry = self
                    .inner
                    .handle_value(handle)
                    .expect("entry must exist immediately after successful insert");
                let counter = &entry.refcount;
                let token = counter.get();
                Ok(CountedHandle { handle, token })
            }
            Err(e) => Err(e),
        }
    }

    /// Mint another token for the same entry; used to clone a counted handle.
    pub fn get(&self, h: &CountedHandle<'_>) -> CountedHandle<'static> {
        // Validate the handle still refers to a live entry while the existing token is held.
        let entry = self
            .inner
            .handle_value(h.handle)
            .expect("handle must be valid while counted handle is live");
        let token = entry.refcount.get();
        CountedHandle {
            handle: h.handle,
            token,
        }
    }

    /// Insert using a lazy value constructor; only calls `default()` when inserting.
    pub fn insert_with<F>(
        &mut self,
        key: K,
        default: F,
    ) -> Result<CountedHandle<'static>, InsertError>
    where
        F: FnOnce() -> V,
    {
        match self.inner.insert_with(key, || Counted::new(default(), 0)) {
            Ok(handle) => {
                let entry = self
                    .inner
                    .handle_value(handle)
                    .expect("entry must exist immediately after successful insert");
                let token = entry.refcount.get();
                Ok(CountedHandle { handle, token })
            }
            Err(e) => Err(e),
        }
    }

    /// Return a token for an entry; removes and returns (K, V) when count hits zero.
    pub fn put(&mut self, h: CountedHandle<'_>) -> PutResult<K, V> {
        let CountedHandle { handle, token, .. } = h;
        let entry = self
            .inner
            .handle_value(handle)
            .expect("CountedHandle must refer to a live entry when returned to put()");
        let now_zero = entry.refcount.put(token);
        if now_zero {
            let (k, v) = self
                .inner
                .remove(handle)
                .expect("entry must exist when count reaches zero");
            PutResult::Removed {
                key: k,
                value: v.value,
            }
        } else {
            PutResult::Live
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle, &K, &V)> {
        self.inner.iter().map(|(h, k, c)| (h, k, &c.value))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &K, &mut V)> {
        self.inner.iter_mut().map(|(h, k, c)| (h, k, &mut c.value))
    }

    pub(crate) fn iter_raw(&self) -> impl Iterator<Item = (CountedHandle<'static>, &K, &V)> {
        self.inner.iter().map(|(h, k, c)| {
            let ch = CountedHandle {
                handle: h,
                token: c.refcount.get(),
            };
            (ch, k, &c.value)
        })
    }

    pub(crate) fn iter_mut_raw(
        &mut self,
    ) -> impl Iterator<Item = (CountedHandle<'static>, &K, &mut V)> {
        self.inner.iter_mut().map(|(h, k, c)| {
            let ch = CountedHandle {
                handle: h,
                token: c.refcount.get(),
            };
            (ch, k, &mut c.value)
        })
    }
}

// Simple iterators yield the same item shapes as HandleHashMap.
// For internal use, iter_raw and iter_mut_raw mint CountedHandles; callers must put() them.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::cell::Cell;
    use std::collections::BTreeSet;

    // Property-based invariant: for each key, entry liveness in the map
    // matches whether there exists at least one outstanding `CountedHandle`
    // for that key.
    //
    // Invariants exercised:
    // - `contains_key(key)` is true iff there is ≥1 outstanding handle for `key`.
    // - `insert` mints exactly one handle on success; duplicates do not mint.
    // - `find`/`get` mint one new handle when the key is present.
    // - `put` decrements the refcount and removes the entry exactly when the
    //   last handle is returned.
    proptest! {
        #[test]
        fn prop_counted_hashmap_liveness(keys in 1usize..=5, ops in proptest::collection::vec((0u8..=4u8, 0usize..100usize), 1..100)) {
            let mut m: CountedHashMap<String, i32> = CountedHashMap::new();
            let mut live: Vec<Vec<CountedHandle<'static>>> = std::iter::repeat_with(Vec::new).take(keys).collect();

            for (op, raw_k) in ops.into_iter() {
                let k = raw_k % keys;
                let key = format!("k{}", k);
                match op {
                    // Insert new entry with value == k
                    0 => {
                        let res = m.insert(key.clone(), k as i32);
                        match res {
                            Ok(h) => live[k].push(h),
                            Err(InsertError::DuplicateKey) => {}
                        }
                    }
                    // Find returns a new handle if present
                    1 => {
                        if let Some(h) = m.find(&key) {
                            live[k].push(h);
                        }
                    }
                    // Clone using get()
                    2 => {
                        if let Some(h) = live[k].pop() {
                            let h2 = m.get(&h);
                            live[k].push(h);
                            live[k].push(h2);
                        }
                    }
                    // Put one handle back
                    3 => {
                        if let Some(h) = live[k].pop() {
                            match m.put(h) {
                                PutResult::Live => {}
                                PutResult::Removed { key: _, value: _ } => {
                                    // After removal there should be no more live handles for this key
                                    // (since this was the last token).
                                    prop_assert!(live[k].is_empty());
                                }
                            }
                        }
                    }
                    // Return all handles for this key
                    4 => {
                        while let Some(h) = live[k].pop() { let _ = m.put(h); }
                    }
                    _ => unreachable!(),
                }

                let present = m.contains_key(&key);
                prop_assert_eq!(present, !live[k].is_empty());
            }

            // Drain remaining handles and verify emptiness condition is consistent
            for k in 0..keys {
                while let Some(h) = live[k].pop() { let _ = m.put(h); }
                let key = format!("k{}", k);
                prop_assert_eq!(m.contains_key(&key), false);
            }
        }
    }

    /// insert_with is lazy (default closure runs only on insertion) and
    /// returns a handle with a freshly minted token. On duplicate, the
    /// default closure must not run and no token is minted. Returning the
    /// last handle removes the entry and yields the stored `(K, V)`.
    #[test]
    fn insert_with_is_lazy_and_mints_token() {
        use crate::handle_hash_map::InsertError;

        let mut m: CountedHashMap<String, i32> = CountedHashMap::new();
        let calls = Cell::new(0);

        let ch = m
            .insert_with("k".to_string(), || {
                calls.set(calls.get() + 1);
                7
            })
            .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(ch.value_ref(&m), Some(&7));

        // Duplicate must not call default and must not mint a token
        {
            let dup = m.insert_with("k".to_string(), || {
                calls.set(calls.get() + 1);
                99
            });
            match dup {
                Err(InsertError::DuplicateKey) => {}
                _ => panic!("unexpected result"),
            }
        }
        assert_eq!(calls.get(), 1);

        // Since ch is the only outstanding token, returning it removes the entry
        match m.put(ch) {
            PutResult::Removed { key, value } => {
                assert_eq!(key, "k".to_string());
                assert_eq!(value, 7);
            }
            _ => panic!("expected removal"),
        }
        assert!(!m.contains_key(&"k".to_string()));
    }

    /// Handle-based accessors expose references to the stored value and
    /// allow in-place mutation through `value_mut`. Mutations persist and
    /// are reflected in subsequent reads.
    #[test]
    fn insert_with_then_mutate_value() {
        let mut m: CountedHashMap<String, i32> = CountedHashMap::new();
        let ch = m.insert_with("k".to_string(), || 10).unwrap();
        if let Some(v) = ch.value_mut(&mut m) {
            *v += 5;
        }
        assert_eq!(ch.value_ref(&m), Some(&15));
        let _ = m.put(ch);
    }

    /// `get` clones a counted handle by minting a new token for the same
    /// entry. Returning one of two handles leaves the entry live; returning
    /// the last one removes the entry and returns `(K, V)`.
    #[test]
    fn get_mints_new_token_and_put_removes_at_zero() {
        let mut m: CountedHashMap<&'static str, i32> = CountedHashMap::new();
        let h1 = m.insert("a", 1).unwrap();
        let h2 = m.get(&h1);

        // Returning one of them leaves the entry live
        match m.put(h1) {
            PutResult::Live => {}
            _ => panic!("expected Live when one handle remains"),
        }
        assert!(m.contains_key(&"a"));

        // Returning the last one removes the entry
        match m.put(h2) {
            PutResult::Removed { key, value } => {
                assert_eq!(key, "a");
                assert_eq!(value, 1);
            }
            _ => panic!("expected Removed at zero"),
        }
        assert!(!m.contains_key(&"a"));
    }

    /// `key_ref`/`value_ref` return references tied to the map borrow and
    /// reflect the current storage; `value_mut` updates persist.
    #[test]
    fn key_ref_value_ref_and_mutation_persist() {
        let mut m: CountedHashMap<String, i32> = CountedHashMap::new();
        let h = m.insert("k1".to_string(), 10).unwrap();
        assert_eq!(h.key_ref(&m), Some(&"k1".to_string()));
        assert_eq!(h.value_ref(&m), Some(&10));
        if let Some(v) = h.value_mut(&mut m) {
            *v += 7;
        }
        assert_eq!(h.value_ref(&m), Some(&17));
        let _ = m.put(h);
    }

    /// Iterators yield each live entry exactly once; `iter_mut` updates are
    /// visible in subsequent reads.
    #[test]
    fn iter_yields_all_entries_once_and_iter_mut_updates_values() {
        let mut m: CountedHashMap<String, i32> = CountedHashMap::new();
        let keys = ["k1", "k2", "k3", "k4"];
        let mut handles = Vec::new();
        for (i, k) in keys.iter().enumerate() {
            handles.push(m.insert((*k).to_string(), i as i32).unwrap());
        }

        // iter yields each live entry exactly once
        let seen: BTreeSet<String> = m.iter().map(|(_h, k, _v)| k.clone()).collect();
        let expected: BTreeSet<String> = keys.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(seen, expected);

        // iter_mut updates are visible in subsequent reads
        for (_h, _k, v) in m.iter_mut() {
            *v += 100;
        }
        for (i, _k) in keys.iter().enumerate() {
            let hv = handles[i].value_ref(&m).copied();
            assert_eq!(hv, Some((i as i32) + 100));
        }

        // Return the outstanding insert handles to clean up
        for h in handles {
            let _ = m.put(h);
        }
    }

    /// `iter_raw` mints a `CountedHandle` per entry for scoped work. These
    /// raw handles keep entries live until explicitly returned to `put`.
    /// Dropping the original handles while the raw handles are outstanding
    /// must not remove the entries; returning the raw handles eventually
    /// removes all entries when the count reaches zero.
    #[test]
    fn iter_raw_requires_put_and_keeps_entries_live() {
        let mut m: CountedHashMap<String, i32> = CountedHashMap::new();
        // Insert 3 entries and keep their handles
        let h1 = m.insert("a".to_string(), 1).unwrap();
        let h2 = m.insert("b".to_string(), 2).unwrap();
        let h3 = m.insert("c".to_string(), 3).unwrap();

        // Mint one extra handle per entry via iter_raw
        let mut raw: Vec<CountedHandle<'static>> = m.iter_raw().map(|(ch, _k, _v)| ch).collect();

        // Drop the original handles; entries must remain live due to raw handles
        match m.put(h1) {
            PutResult::Live => {}
            _ => panic!("expected Live"),
        }
        match m.put(h2) {
            PutResult::Live => {}
            _ => panic!("expected Live"),
        }
        match m.put(h3) {
            PutResult::Live => {}
            _ => panic!("expected Live"),
        }
        assert!(m.contains_key(&"a".to_string()));
        assert!(m.contains_key(&"b".to_string()));
        assert!(m.contains_key(&"c".to_string()));

        // Now return all raw handles; each should remove the corresponding entry
        let mut removed: BTreeSet<String> = BTreeSet::new();
        while let Some(ch) = raw.pop() {
            match m.put(ch) {
                PutResult::Removed { key, value } => {
                    removed.insert(key.clone());
                    match key.as_str() {
                        "a" => assert_eq!(value, 1),
                        "b" => assert_eq!(value, 2),
                        "c" => assert_eq!(value, 3),
                        _ => unreachable!(),
                    }
                }
                PutResult::Live => {}
            }
        }
        assert_eq!(
            removed,
            ["a", "b", "c"].into_iter().map(|s| s.to_string()).collect()
        );
        assert!(!m.contains_key(&"a".to_string()));
        assert!(!m.contains_key(&"b".to_string()));
        assert!(!m.contains_key(&"c".to_string()));
    }

    /// `iter_mut_raw` behaves like `iter_raw` but yields `&mut V`. Mutations
    /// applied through these mutable references are persisted. As with
    /// `iter_raw`, the minted handles must be returned via `put` to allow
    /// final removal.
    #[test]
    fn iter_mut_raw_requires_put_and_keeps_entries_live() {
        let mut m: CountedHashMap<&'static str, i32> = CountedHashMap::new();
        let h1 = m.insert("x", 10).unwrap();
        let h2 = m.insert("y", 20).unwrap();

        // Mint one extra handle per entry via iter_mut_raw and also mutate
        let mut raw: Vec<CountedHandle<'static>> = m
            .iter_mut_raw()
            .map(|(ch, _k, v)| {
                *v += 1;
                ch
            })
            .collect();

        // Return the original handles; entries remain live due to raw handles
        assert!(matches!(m.put(h1), PutResult::Live));
        assert!(matches!(m.put(h2), PutResult::Live));
        assert!(m.contains_key(&"x"));
        assert!(m.contains_key(&"y"));

        // Verify mutations persisted
        let xr = m.find(&"x").unwrap();
        let yr = m.find(&"y").unwrap();
        assert_eq!(xr.value_ref(&m), Some(&11));
        assert_eq!(yr.value_ref(&m), Some(&21));
        // Return the temporary verification handles
        let _ = m.put(xr);
        let _ = m.put(yr);

        // Return raw handles and ensure removals happen
        let mut removed = 0;
        while let Some(ch) = raw.pop() {
            match m.put(ch) {
                PutResult::Removed { key, value } => {
                    removed += 1;
                    match key {
                        "x" => assert_eq!(value, 11),
                        "y" => assert_eq!(value, 21),
                        _ => unreachable!(),
                    }
                }
                PutResult::Live => {}
            }
        }
        assert_eq!(removed, 2);
        assert!(!m.contains_key(&"x"));
        assert!(!m.contains_key(&"y"));
    }

    /// Negative behavior: dropping a `CountedHandle` without calling `put`
    /// must panic due to the underlying `Token`'s `Drop` implementation.
    /// Likewise, collecting raw handles from `iter_raw` and dropping them
    /// without returning to `put` should panic. This verifies fail-fast
    /// behavior that guards token balance.
    #[test]
    fn dropping_counted_handle_without_put_panics() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        let res = catch_unwind(AssertUnwindSafe(|| {
            let mut m: CountedHashMap<&'static str, i32> = CountedHashMap::new();
            let h = m.insert("boom", 1).unwrap();
            drop(h); // dropping a CountedHandle drops its token and should panic
        }));
        assert!(
            res.is_err(),
            "expected panic when CountedHandle is dropped without put"
        );

        // Also verify dropping raw handles from iter_raw without put panics
        let res2 = catch_unwind(AssertUnwindSafe(|| {
            let m: CountedHashMap<&'static str, i32> = {
                let mut mm = CountedHashMap::new();
                let _ = mm.insert("a", 1).unwrap();
                let _ = mm.insert("b", 2).unwrap();
                mm
            };
            let v: Vec<_> = m.iter_raw().collect();
            drop(v); // each CountedHandle inside should panic on drop
        }));
        assert!(
            res2.is_err(),
            "expected panic when raw handles are dropped without put"
        );
    }
}
