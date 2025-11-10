//! HandleHashMap: structural layer with stable handles and debug reentrancy guard.

use crate::reentrancy::DebugReentrancy;
use core::borrow::Borrow;
use core::hash::{BuildHasher, Hash};
use hashbrown::HashTable;
use slotmap::{DefaultKey, SlotMap};
use std::collections::hash_map::RandomState;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Handle(DefaultKey);

impl Handle {
    pub(crate) fn new(k: DefaultKey) -> Self {
        Handle(k)
    }
    pub(crate) fn raw_handle(&self) -> DefaultKey {
        self.0
    }

    pub fn key<'a, K, V, S>(&self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a K>
    where
        K: Eq + Hash,
        S: BuildHasher + Clone + Default,
    {
        map.handle_key(*self)
    }

    pub fn value<'a, K, V, S>(&self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a V>
    where
        K: Eq + Hash,
        S: BuildHasher + Clone + Default,
    {
        map.handle_value(*self)
    }

    pub fn value_mut<'a, K, V, S>(&self, map: &'a mut HandleHashMap<K, V, S>) -> Option<&'a mut V>
    where
        K: Eq + Hash,
        S: BuildHasher + Clone + Default,
    {
        map.handle_value_mut(*self)
    }
}

#[derive(Debug)]
struct Entry<K, V> {
    key: K,
    value: V,
    hash: u64,
}

pub struct HandleHashMap<K, V, S = RandomState> {
    hasher: S,
    index: HashTable<DefaultKey>,
    slots: SlotMap<DefaultKey, Entry<K, V>>, // storage using generational keys
    reentrancy: DebugReentrancy,
}

#[derive(Debug)]
pub enum InsertError {
    DuplicateKey,
}

impl<K, V> HandleHashMap<K, V>
where
    K: Eq + Hash,
{
    pub fn new() -> Self {
        Self::with_hasher(Default::default())
    }
}

impl<K, V> Default for HandleHashMap<K, V>
where
    K: Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Iterator over immutable entries in `HandleHashMap`.
pub struct Iter<'a, K, V, S> {
    it: slotmap::basic::Iter<'a, DefaultKey, Entry<K, V>>,
    pub(crate) _pd: core::marker::PhantomData<&'a (K, V, S)>,
}

impl<'a, K, V, S> Iterator for Iter<'a, K, V, S> {
    type Item = (Handle, &'a K, &'a V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.it
            .next()
            .map(|(k, e)| (Handle::new(k), &e.key, &e.value))
    }
}

/// Iterator over mutable entries in `HandleHashMap`.
pub struct IterMut<'a, K, V, S> {
    it: slotmap::basic::IterMut<'a, DefaultKey, Entry<K, V>>,
    pub(crate) _pd: core::marker::PhantomData<&'a (K, V, S)>,
}

impl<'a, K, V, S> Iterator for IterMut<'a, K, V, S> {
    type Item = (Handle, &'a K, &'a mut V);
    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.it
            .next()
            .map(|(k, e)| (Handle::new(k), &e.key, &mut e.value))
    }
}

impl<K, V, S> HandleHashMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Clone + Default,
{
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            index: HashTable::new(),
            hasher,
            slots: SlotMap::with_key(),
            reentrancy: DebugReentrancy::new(),
        }
    }

    fn make_hash<Q>(&self, q: &Q) -> u64
    where
        Q: ?Sized + Hash,
    {
        self.hasher.hash_one(q)
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    pub fn find<Q>(&self, q: &Q) -> Option<Handle>
    where
        K: Borrow<Q>,
        Q: ?Sized + Hash + Eq,
    {
        let _g = self.reentrancy.enter();
        let hash = self.make_hash(q);
        if let Some(&k) = self.index.find(hash, |&k| {
            self.slots
                .get(k)
                .map(|e| e.key.borrow() == q)
                .unwrap_or(false)
        }) {
            return Some(Handle::new(k));
        }
        None
    }

    pub fn contains_key<Q>(&self, q: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: ?Sized + Hash + Eq,
    {
        let _g = self.reentrancy.enter();
        let hash = self.make_hash(q);
        self.index
            .find(hash, |&k| {
                self.slots
                    .get(k)
                    .map(|e| e.key.borrow() == q)
                    .unwrap_or(false)
            })
            .is_some()
    }

    pub fn insert(&mut self, key: K, value: V) -> Result<Handle, InsertError> {
        let _g = self.reentrancy.enter();
        let hash = self.make_hash(&key);
        let entry = Entry { key, value, hash };
        // Use HashTable::entry to deduplicate or insert.
        match self.index.entry(
            hash,
            |&kk| {
                self.slots
                    .get(kk)
                    .map(|e| e.key == entry.key)
                    .unwrap_or(false)
            },
            |&kk| self.slots.get(kk).map(|e| e.hash).unwrap_or(0),
        ) {
            hashbrown::hash_table::Entry::Occupied(_) => Err(InsertError::DuplicateKey),
            hashbrown::hash_table::Entry::Vacant(v) => {
                let k = self.slots.insert(entry);
                let _ = v.insert(k);
                Ok(Handle::new(k))
            }
        }
    }

    pub fn insert_with<F>(&mut self, key: K, default: F) -> Result<Handle, InsertError>
    where
        F: FnOnce() -> V,
    {
        let _g = self.reentrancy.enter();
        let hash = self.make_hash(&key);
        match self.index.entry(
            hash,
            |&kk| self.slots.get(kk).map(|e| e.key == key).unwrap_or(false),
            |&kk| self.slots.get(kk).map(|e| e.hash).unwrap_or(0),
        ) {
            hashbrown::hash_table::Entry::Occupied(_) => Err(InsertError::DuplicateKey),
            hashbrown::hash_table::Entry::Vacant(v) => {
                let value = default();
                let entry = Entry { key, value, hash };
                let k = self.slots.insert(entry);
                let _ = v.insert(k);
                Ok(Handle::new(k))
            }
        }
    }

    pub fn remove(&mut self, handle: Handle) -> Option<(K, V)> {
        let _g = self.reentrancy.enter();
        let k = handle.raw_handle();

        // Remove slot
        let entry = self.slots.remove(k)?;

        // Unlink from index via occupied entry removal
        self.index
            .find_entry(entry.hash, |&kk| kk == k)
            .unwrap()
            .remove();

        Some((entry.key, entry.value))
    }

    pub(crate) fn handle_key(&self, h: Handle) -> Option<&K> {
        let _g = self.reentrancy.enter();
        self.slots.get(h.raw_handle()).map(|e| &e.key)
    }

    pub(crate) fn handle_value(&self, h: Handle) -> Option<&V> {
        let _g = self.reentrancy.enter();
        self.slots.get(h.raw_handle()).map(|e| &e.value)
    }

    pub(crate) fn handle_value_mut(&mut self, h: Handle) -> Option<&mut V> {
        let _g = self.reentrancy.enter();
        self.slots.get_mut(h.raw_handle()).map(|e| &mut e.value)
    }

    pub fn iter(&self) -> Iter<'_, K, V, S> {
        let it = self.slots.iter();
        Iter {
            it,
            _pd: core::marker::PhantomData,
        }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, K, V, S> {
        let it = self.slots.iter_mut();
        IterMut {
            it,
            _pd: core::marker::PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::hash::Hasher;

    /// Invariant: Duplicate keys are rejected and the map remains unchanged.
    #[test]
    fn duplicate_insert_rejected() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let handle = m.insert("dup".to_string(), 1).unwrap();
        match m.insert("dup".to_string(), 2) {
            Err(InsertError::DuplicateKey) => {}
            other => panic!("unexpected result: {:?}", other),
        }
        assert_eq!(*handle.value(&m).unwrap(), 1);
        assert_eq!(m.len(), 1);
    }

    /// Invariant: `find(k).is_some() == contains_key(k)` for present/absent keys.
    #[test]
    fn find_contains_parity() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let present = ["a", "b", "c"];
        for (i, k) in present.iter().enumerate() {
            m.insert((*k).to_string(), i as i32).unwrap();
        }

        for k in present {
            let s = k.to_string();
            assert!(m.find(&s).is_some());
            assert!(m.contains_key(&s));
        }

        for k in ["x", "y", "z"] {
            let s = k.to_string();
            assert!(m.find(&s).is_none());
            assert!(!m.contains_key(&s));
        }
    }

    /// Invariant: Borrowed lookup works (store `String`, query with `&str`).
    #[test]
    fn borrowed_lookup_with_str() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        m.insert("hello".to_string(), 1).unwrap();
        assert!(m.contains_key("hello"));
        assert!(!m.contains_key("world"));

        // Also validate borrowed find
        assert!(m.find("hello").is_some());
        assert!(m.find("world").is_none());
    }

    /// Invariant: Handle-based access yields references while the entry exists and
    /// becomes `None` after removal. Mutating via `value_mut` updates the stored value.
    #[test]
    fn handle_access_and_mutation() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let h = m.insert("k1".to_string(), 10).unwrap();
        assert_eq!(h.key(&m), Some(&"k1".to_string()));
        assert_eq!(h.value(&m), Some(&10));
        let new_val = h
            .value_mut(&mut m)
            .map(|v| {
                *v += 5;
                *v
            })
            .unwrap();
        assert_eq!(new_val, 15);
        assert_eq!(h.value(&m), Some(&15));

        let (_k, _v) = m.remove(h).unwrap();
        assert!(h.value(&m).is_none());
    }

    /// Invariant: Removing an entry invalidates its handle and does not alias a new
    /// entry inserted afterward, even if the physical slot is reused (generational keys).
    #[test]
    fn stale_handle_does_not_alias_new_entry() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let h1 = m.insert("old".to_string(), 1).unwrap();
        let (_k, _v) = m.remove(h1).unwrap();
        // Next insert likely reuses the freed slot with bumped generation.
        let h2 = m.insert("new".to_string(), 2).unwrap();
        assert_ne!(h1, h2, "handles must differ across generations");
        assert!(h1.value(&m).is_none(), "stale handle must not resolve");
        assert!(m.contains_key("new"));
        assert!(!m.contains_key("old"));
    }

    /// Invariant: Iteration yields each live entry exactly once; `iter_mut` updates
    /// values as seen by subsequent lookups.
    #[test]
    fn iteration_and_mutation() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let keys = ["k1", "k2", "k3"];
        for (i, k) in keys.iter().enumerate() {
            m.insert((*k).to_string(), i as i32).unwrap();
        }

        let seen: BTreeSet<String> = m.iter().map(|(_h, k, _v)| k.clone()).collect();
        let expected: BTreeSet<String> = keys.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(seen, expected);

        for (_h, _k, v) in m.iter_mut() {
            *v += 10;
        }
        for k in keys {
            let h = m.find(&k.to_string()).unwrap();
            assert_eq!(
                h.value(&m),
                Some(&match k {
                    "k1" => 10,
                    "k2" => 11,
                    "k3" => 12,
                    _ => unreachable!(),
                })
            );
        }
    }

    /// Invariant: Lookups work under heavy hash collisions; equality resolves to the
    /// correct entry. This also exercises collision probing via `Eq`.
    #[test]
    fn collision_handling_with_const_hasher() {
        #[derive(Clone, Default)]
        struct ConstBuildHasher;
        struct ConstHasher;
        impl BuildHasher for ConstBuildHasher {
            type Hasher = ConstHasher;
            fn build_hasher(&self) -> Self::Hasher {
                ConstHasher
            }
        }
        impl core::hash::Hasher for ConstHasher {
            fn write(&mut self, _bytes: &[u8]) {}
            fn finish(&self) -> u64 {
                0
            } // force all keys into the same hash bucket
        }

        let mut m: HandleHashMap<String, i32, ConstBuildHasher> =
            HandleHashMap::with_hasher(ConstBuildHasher);
        m.insert("a".to_string(), 1).unwrap();
        m.insert("b".to_string(), 2).unwrap();

        let ha = m.find(&"a".to_string()).expect("find a");
        let hb = m.find(&"b".to_string()).expect("find b");
        assert_ne!(ha, hb);
        assert_eq!(ha.key(&m), Some(&"a".to_string()));
        assert_eq!(hb.key(&m), Some(&"b".to_string()));
    }

    /// Invariant: After `remove`, the key is absent; reinserting the same key adds a
    /// fresh entry with a potentially new handle and the new value is observed.
    #[test]
    fn remove_then_reinsert_same_key_yields_new_value() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let h1 = m.insert("k".to_string(), 1).unwrap();

        // Remove: key must disappear and handle becomes invalid
        let (k_removed, v_removed) = m.remove(h1).expect("present for removal");
        assert_eq!(k_removed, "k");
        assert_eq!(v_removed, 1);
        assert!(!m.contains_key("k"));
        assert!(m.find(&"k".to_string()).is_none());
        assert!(h1.value(&m).is_none());

        // Reinsert same key with a different value
        let h2 = m.insert("k".to_string(), 2).expect("reinsert allowed");
        assert!(m.contains_key("k"));
        let hf = m.find(&"k".to_string()).expect("find reinserted key");
        assert_eq!(hf.value(&m), Some(&2));
        assert_eq!(h2.value(&m), Some(&2));
        assert_ne!(h1, h2, "old handle must not alias new entry");
        assert!(h1.value(&m).is_none(), "stale handle stays invalid");
    }

    /// Invariant (debug-only): Re-entering `HandleHashMap` from within `K: Eq` during a
    /// probe panics due to the reentrancy guard; in release builds, this test is skipped.
    #[cfg(debug_assertions)]
    #[test]
    fn reentrancy_panics_from_eq_during_find() {
        #[derive(Clone, Default)]
        struct ConstBuildHasher;
        struct ConstHasher;
        impl BuildHasher for ConstBuildHasher {
            type Hasher = ConstHasher;
            fn build_hasher(&self) -> Self::Hasher {
                ConstHasher
            }
        }
        impl core::hash::Hasher for ConstHasher {
            fn write(&mut self, _bytes: &[u8]) {}
            fn finish(&self) -> u64 {
                0
            }
        }

        struct ReentryKey {
            id: &'static str,
            map: *const HandleHashMap<ReentryKey, i32, ConstBuildHasher>,
            trigger: bool,
        }
        impl core::fmt::Debug for ReentryKey {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(self.id)
            }
        }
        impl PartialEq for ReentryKey {
            fn eq(&self, other: &Self) -> bool {
                if self.id == other.id {
                    return true;
                }
                if other.trigger {
                    // Attempt to re-enter the same map during probing.
                    unsafe {
                        let m = &*other.map;
                        let _ = m.contains_key(self.id);
                    }
                }
                false
            }
        }
        impl Eq for ReentryKey {}
        impl Hash for ReentryKey {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.id.hash(state);
            }
        }
        impl core::borrow::Borrow<str> for ReentryKey {
            fn borrow(&self) -> &str {
                self.id
            }
        }

        let mut m: HandleHashMap<ReentryKey, i32, ConstBuildHasher> =
            HandleHashMap::with_hasher(ConstBuildHasher);
        let key = ReentryKey {
            id: "a",
            map: core::ptr::null(),
            trigger: false,
        };
        // Set map pointer after creation
        let key = ReentryKey {
            map: &m as *const _,
            ..key
        };
        m.insert(key, 1).unwrap();

        let query = ReentryKey {
            id: "b",
            map: &m as *const _,
            trigger: true,
        };
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = m.find(&query);
        }));
        assert!(res.is_err(), "expected reentrancy to panic in debug builds");
    }

    /// Invariant: `insert_with` only runs the default constructor on
    /// successful insert; on duplicate it does not run and returns an error.
    #[test]
    fn insert_with_is_lazy_and_deduplicates() {
        let mut m: HandleHashMap<String, String> = HandleHashMap::new();
        let calls = Cell::new(0);

        let r = m.insert_with("k".to_string(), || {
            calls.set(calls.get() + 1);
            "v".to_string()
        });
        assert!(r.is_ok());
        assert_eq!(calls.get(), 1);

        // Duplicate: must not run default closure
        let r2 = m.insert_with("k".to_string(), || {
            calls.set(calls.get() + 1);
            "v2".to_string()
        });
        match r2 {
            Err(InsertError::DuplicateKey) => {}
            other => panic!("unexpected result: {:?}", other),
        }
        assert_eq!(calls.get(), 1, "default() must not run on duplicate");

        // Value remains the original one
        let h = m.find(&"k".to_string()).unwrap();
        assert_eq!(h.value(&m), Some(&"v".to_string()));
    }

    /// Invariant: Values inserted via `insert` and `insert_with` are
    /// equivalent for the same key/value; duplicates are rejected by both.
    #[test]
    fn insert_with_value_equivalence() {
        let mut m1: HandleHashMap<&'static str, i32> = HandleHashMap::new();
        let mut m2: HandleHashMap<&'static str, i32> = HandleHashMap::new();

        let h1 = m1.insert("a", 1).unwrap();
        let h2 = m2.insert_with("a", || 1).unwrap();

        assert!(m1.contains_key(&"a"));
        assert!(m2.contains_key(&"a"));
        assert_eq!(h1.value(&m1), h2.value(&m2));

        // insert_with rejects duplicate just like insert
        assert!(m1.insert_with("a", || 2).is_err());
        assert!(m2.insert_with("a", || 3).is_err());
    }

    /// Invariant: Handles referring to the same entry alias: mutating via one handle
    /// is visible through the other obtained via lookup.
    #[test]
    fn handles_alias_same_entry_between_insert_and_find() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let h_insert = m.insert("k".to_string(), 10).unwrap();

        // Obtain another handle via lookup
        let h_find = m.find("k").expect("key present");

        // They should be equal handles for the same slot
        assert_eq!(h_insert, h_find);

        // Mutate through the first handle; observe via the second
        *h_insert.value_mut(&mut m).expect("value_mut present") = 20;
        assert_eq!(h_find.value(&m), Some(&20));

        // Mutate through the second handle; observe via the first
        *h_find.value_mut(&mut m).expect("value_mut present") = 30;
        assert_eq!(h_insert.value(&m), Some(&30));
    }

    /// Invariant: `len()` and `is_empty()` reflect the number of live entries,
    /// unaffected by failed duplicate inserts, and updated after removals.
    #[test]
    fn len_and_is_empty_behaviors() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        assert_eq!(m.len(), 0);
        assert!(m.is_empty());

        let h1 = m.insert("a".to_string(), 1).unwrap();
        assert_eq!(m.len(), 1);
        assert!(!m.is_empty());

        // Duplicate insert must not change len/is_empty
        match m.insert("a".to_string(), 2) {
            Err(InsertError::DuplicateKey) => {}
            other => panic!("unexpected result: {:?}", other),
        }
        assert_eq!(m.len(), 1);
        assert!(!m.is_empty());

        let h2 = m.insert("b".to_string(), 2).unwrap();
        assert_eq!(m.len(), 2);
        assert!(!m.is_empty());

        let _ = m.remove(h1).unwrap();
        assert_eq!(m.len(), 1);
        assert!(!m.is_empty());

        let _ = m.remove(h2).unwrap();
        assert_eq!(m.len(), 0);
        assert!(m.is_empty());
    }
}
