//! HandleHashMap: structural layer with stable handles and debug reentrancy guard.

use crate::reentrancy::DebugReentrancy;
use core::borrow::Borrow;
use core::hash::{BuildHasher, Hash, Hasher};
use hashbrown::raw::RawTable;
use slotmap::{DefaultKey, SlotMap};
use std::collections::hash_map::RandomState;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Handle(DefaultKey);

impl Handle {
    pub(crate) fn new(k: DefaultKey) -> Self {
        Handle(k)
    }
    pub(crate) fn key(&self) -> DefaultKey {
        self.0
    }

    pub fn key_ref<'a, K, V, S>(&self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a K>
    where
        K: Eq + Hash,
        S: BuildHasher + Clone + Default,
    {
        map.handle_key(*self)
    }

    pub fn value_ref<'a, K, V, S>(&self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a V>
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
    index: RawTable<DefaultKey>,
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

impl<K, V, S> HandleHashMap<K, V, S>
where
    K: Eq + Hash,
    S: BuildHasher + Clone + Default,
{
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            index: RawTable::new(),
            hasher,
            slots: SlotMap::with_key(),
            reentrancy: DebugReentrancy::new(),
        }
    }

    fn make_hash<Q>(&self, q: &Q) -> u64
    where
        Q: ?Sized + Hash,
    {
        let mut h = self.hasher.build_hasher();
        q.hash(&mut h);
        h.finish()
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
        if let Some(&k) = self.index.get(hash, |&k| {
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
        if self
            .index
            .get(hash, |&k| {
                self.slots
                    .get(k)
                    .map(|e| e.key.borrow() == q)
                    .unwrap_or(false)
            })
            .is_some()
        {
            return true;
        }
        false
    }

    pub fn insert(&mut self, key: K, value: V) -> Result<Handle, InsertError> {
        let _g = self.reentrancy.enter();
        let hash = self.make_hash(&key);
        if self
            .index
            .get(hash, |&k| {
                self.slots.get(k).map(|e| e.key == key).unwrap_or(false)
            })
            .is_some()
        {
            return Err(InsertError::DuplicateKey);
        }

        let entry = Entry { key, value, hash };
        // Find insertion slot; duplicate returns Ok(bucket)
        match self.index.find_or_find_insert_slot(
            hash,
            |&kk| {
                self.slots
                    .get(kk)
                    .map(|e| e.hash == hash && e.key == entry.key)
                    .unwrap_or(false)
            },
            |&kk| self.slots.get(kk).map(|e| e.hash).unwrap_or(0),
        ) {
            Ok(_) => return Err(InsertError::DuplicateKey),
            Err(slot) => {
                let k = self.slots.insert(entry);
                unsafe {
                    let _bucket = self.index.insert_in_slot(hash, slot, k);
                }
                Ok(Handle::new(k))
            }
        }
    }

    pub fn remove(&mut self, handle: Handle) -> Option<(K, V)> {
        let _g = self.reentrancy.enter();
        let k = handle.key();
        let entry_hash = self.slots.get(k)?.hash;

        // Unlink from index first
        let _removed = self.index.remove_entry(entry_hash, |&kk| kk == k);

        // Now take from slot; structure is consistent for any user code during drops
        self.slots.remove(k).map(|e| (e.key, e.value))
    }

    pub(crate) fn handle_key<'a>(&'a self, h: Handle) -> Option<&'a K> {
        let _g = self.reentrancy.enter();
        self.slots.get(h.key()).map(|e| &e.key)
    }

    pub(crate) fn handle_value<'a>(&'a self, h: Handle) -> Option<&'a V> {
        let _g = self.reentrancy.enter();
        self.slots.get(h.key()).map(|e| &e.value)
    }

    pub(crate) fn handle_value_mut<'a>(&'a mut self, h: Handle) -> Option<&'a mut V> {
        let _g = self.reentrancy.enter();
        self.slots.get_mut(h.key()).map(|e| &mut e.value)
    }

    pub fn iter(&self) -> impl Iterator<Item = (Handle, &K, &V)> {
        self.slots
            .iter()
            .map(|(k, e)| (Handle::new(k), &e.key, &e.value))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle, &K, &mut V)> {
        self.slots
            .iter_mut()
            .map(|(k, e)| (Handle::new(k), &e.key, &mut e.value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::rc::Rc;

    /// Invariant: Duplicate keys are rejected and the map remains unchanged.
    #[test]
    fn duplicate_insert_rejected() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let _ = m.insert("dup".to_string(), 1).unwrap();
        match m.insert("dup".to_string(), 2) {
            Err(InsertError::DuplicateKey) => {}
            other => panic!("unexpected result: {:?}", other),
        }
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
            assert_eq!(m.find(&s).is_some(), m.contains_key(&s));
        }

        for k in ["x", "y", "z"] {
            let s = k.to_string();
            assert_eq!(m.find(&s).is_some(), m.contains_key(&s));
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
        assert_eq!(h.key_ref(&m), Some(&"k1".to_string()));
        assert_eq!(h.value_ref(&m), Some(&10));
        let new_val = h
            .value_mut(&mut m)
            .map(|v| {
                *v += 5;
                *v
            })
            .unwrap();
        assert_eq!(new_val, 15);
        assert_eq!(h.value_ref(&m), Some(&15));

        let (_k, _v) = m.remove(h).unwrap();
        assert!(h.value_ref(&m).is_none());
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
        assert!(h1.value_ref(&m).is_none(), "stale handle must not resolve");
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
                h.value_ref(&m),
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
        assert_eq!(ha.key_ref(&m), Some(&"a".to_string()));
        assert_eq!(hb.key_ref(&m), Some(&"b".to_string()));
    }

    /// Invariant: `remove` unlinks the entry from the index before user `Drop` code for
    /// `K`/`V` runs, so re-entering the map inside `Drop` observes a consistent structure
    /// and does not panic.
    #[test]
    fn reenter_from_value_drop_after_remove_is_allowed() {
        #[derive(Clone)]
        struct ReenterOnDrop {
            map: *const HandleHashMap<String, ReenterOnDrop>,
            removed_key: String,
            // Records what `contains_key` returned during Drop
            observed: Rc<Cell<Option<bool>>>,
        }
        impl Drop for ReenterOnDrop {
            fn drop(&mut self) {
                unsafe {
                    // SAFETY: Test ensures the map outlives this drop.
                    let m = &*self.map;
                    let has_key = m.contains_key(self.removed_key.as_str());
                    self.observed.set(Some(has_key));
                }
            }
        }

        let mut m: HandleHashMap<String, ReenterOnDrop> = HandleHashMap::new();
        let observed = Rc::new(Cell::new(None));
        let h = m
            .insert(
                "rk".to_string(),
                ReenterOnDrop {
                    map: &m as *const _,
                    removed_key: "rk".to_string(),
                    observed: observed.clone(),
                },
            )
            .unwrap();

        let (k, v) = m.remove(h).unwrap();
        drop(k);
        drop(v); // triggers Drop, which re-enters the map

        assert_eq!(
            observed.get(),
            Some(false),
            "index must be unlinked before Drop"
        );
        assert_eq!(m.len(), 0);
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
}
