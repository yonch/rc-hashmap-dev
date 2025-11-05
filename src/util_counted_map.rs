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

/// Lifetime-bound counted handle carrying a linear token tied to the entry counter.
pub struct CountedHandle<'a> {
    pub(crate) handle: Handle,
    pub(crate) token: Token<'a, UsizeCount>,
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

    pub fn find(&self, key: &K) -> Option<CountedHandle<'_>> {
        let h = self.inner.find(key)?;
        let t = self.inner.handle_value(h)?.refcount.get();
        Some(CountedHandle {
            handle: h,
            token: t,
        })
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

    /// Insert a new key -> value and mint a token for the returned handle.
    pub fn insert(&mut self, key: K, value: V) -> Result<CountedHandle<'_>, InsertError> {
        let h = self.inner.insert(key, Counted::new(value, 0))?;
        let t = self
            .inner
            .handle_value(h)
            .expect("just inserted")
            .refcount
            .get();
        Ok(CountedHandle {
            handle: h,
            token: t,
        })
    }

    /// Mint another token for the same entry; used to clone a counted handle.
    pub fn get(&self, h: &CountedHandle<'_>) -> CountedHandle<'_> {
        let t = self
            .inner
            .handle_value(h.handle)
            .expect("handle must be valid while counted handle is live")
            .refcount
            .get();
        CountedHandle {
            handle: h.handle,
            token: t,
        }
    }

    /// Return a token for an entry; removes and returns (K, V) when count hits zero.
    pub fn put(&mut self, h: CountedHandle<'_>) -> PutResult<K, V> {
        if let Some(entry) = self.inner.handle_value(h.handle) {
            let now_zero = entry.refcount.put(h.token);
            if now_zero {
                let (k, v) = self
                    .inner
                    .remove(h.handle)
                    .expect("entry must exist when count reaches zero");
                return PutResult::Removed {
                    key: k,
                    value: v.value,
                };
            }
            PutResult::Live
        } else {
            // Stale handle; treat as live. Disarm token drop to avoid panic.
            core::mem::forget(h.token);
            PutResult::Live
        }
    }
}

// No unit tests here; exercised via higher-level RcHashMap tests.
