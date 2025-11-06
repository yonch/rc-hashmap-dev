//! CountedHashMap: per-entry reference counting atop HandleHashMap using tokens.

use crate::tokens::{Count, Token, UsizeCount};
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

    pub fn find<Q>(&self, q: &Q) -> Option<CountedHandle<'_>>
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
    pub fn insert(&mut self, key: K, value: V) -> Result<CountedHandle<'_>, InsertError> {
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
    pub fn get(&self, h: &CountedHandle<'_>) -> CountedHandle<'_> {
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

    pub(crate) fn iter_raw(&self) -> impl Iterator<Item = (CountedHandle<'_>, &K, &V)> {
        self.inner.iter().map(|(h, k, c)| {
            let ch = CountedHandle {
                handle: h,
                token: c.refcount.get(),
            };
            (ch, k, &c.value)
        })
    }

    pub(crate) fn iter_mut_raw(&mut self) -> impl Iterator<Item = (CountedHandle<'_>, &K, &mut V)> {
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
