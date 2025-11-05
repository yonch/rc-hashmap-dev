//! CountedHashMap: per-entry reference counting atop HandleHashMap.

use crate::tokens::UsizeCount;
use crate::util_handle_map::{Handle, HandleHashMap, InsertError};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_insert_rejected() {
        let mut m: CountedHashMap<String, i32> = CountedHashMap::new();
        let _ = m.insert("dup".to_string(), 1).unwrap();
        match m.insert("dup".to_string(), 2) {
            Err(InsertError::DuplicateKey) => {}
            other => panic!("unexpected result: {:?}", other),
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

    pub fn find(&self, key: &K) -> Option<Handle> {
        self.inner.find(key)
    }

    pub fn contains_key<Q>(&self, q: &Q) -> bool
    where
        K: core::borrow::Borrow<Q>,
        Q: ?Sized + core::hash::Hash + Eq,
    {
        self.inner.contains_key(q)
    }

    pub fn get_value(&self, h: Handle) -> Option<&V> {
        self.inner.handle_value(h).map(|c| &c.value)
    }
    pub fn get_value_mut(&mut self, h: Handle) -> Option<&mut V> {
        self.inner.handle_value_mut(h).map(|c| &mut c.value)
    }

    /// Insert a new key -> value with refcount initialized to 0.
    pub fn insert(&mut self, key: K, value: V) -> Result<Handle, InsertError> {
        self.inner.insert(key, Counted::new(value, 0))
    }

    /// Increment per-entry refcount; returns new count.
    pub fn inc(&self, h: Handle) -> Option<usize> {
        let c = self.inner.handle_value(h)?;
        // Perform raw increment mirroring Rc semantics.
        Some(c.refcount.inc_raw())
    }

    /// Decrement per-entry refcount; returns true if now zero.
    pub fn dec(&self, h: Handle) -> Option<bool> {
        let c = self.inner.handle_value(h)?;
        Some(c.refcount.dec_raw() == 0)
    }

    /// Physically remove the entry corresponding to the handle; returns (K, V).
    pub fn remove(&mut self, h: Handle) -> Option<(K, V)> {
        self.inner.remove(h).map(|(k, c)| (k, c.value))
    }
}
