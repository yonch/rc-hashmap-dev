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
    pub(crate) fn new(k: DefaultKey) -> Self { Handle(k) }
    pub(crate) fn key(&self) -> DefaultKey { self.0 }

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

    pub fn value_mut<'a, K, V, S>(
        &self,
        map: &'a mut HandleHashMap<K, V, S>,
    ) -> Option<&'a mut V>
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

    pub fn len(&self) -> usize { self.slots.len() }
    pub fn is_empty(&self) -> bool { self.slots.is_empty() }

    pub fn find(&self, key: &K) -> Option<Handle> {
        let _g = self.reentrancy.enter();
        let hash = self.make_hash(key);
        if let Some(&k) = self.index.get(hash, |&k| {
            self.slots
                .get(k)
                .map(|e| &e.key == key)
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
            .get(hash, |&k| self.slots.get(k).map(|e| e.key.borrow() == q).unwrap_or(false))
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
            .get(hash, |&k| self.slots.get(k).map(|e| e.key == key).unwrap_or(false))
            .is_some()
        {
            return Err(InsertError::DuplicateKey);
        }

        let entry = Entry { key, value, hash };
        // Find insertion slot; duplicate returns Ok(bucket)
        match self.index.find_or_find_insert_slot(
            hash,
            |&kk| self.slots.get(kk).map(|e| e.hash == hash && e.key == entry.key).unwrap_or(false),
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

    #[test]
    fn duplicate_insert_rejected() {
        let mut m: HandleHashMap<String, i32> = HandleHashMap::new();
        let _ = m.insert("dup".to_string(), 1).unwrap();
        match m.insert("dup".to_string(), 2) {
            Err(InsertError::DuplicateKey) => {}
            other => panic!("unexpected result: {:?}", other),
        }
    }
}
