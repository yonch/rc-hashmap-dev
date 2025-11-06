//! CountedHashMap: per-entry reference counting atop HandleHashMap using tokens.

use crate::tokens::{Count, Token, UsizeCount};
use crate::util_handle_map::{Handle, HandleHashMap, InsertError};
use core::ops::{Deref, DerefMut};

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
        let h = self.inner.find(q)?;
        let entry = self.inner.handle_value(h)?;
        let counter = &entry.refcount;
        let t = counter.get();
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
        let t = entry.refcount.get();
        CountedHandle {
            handle: h.handle,
            token: t,
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

    pub fn iter(&self) -> impl Iterator<Item = ItemGuard<'_, K, V>> {
        self.inner.iter().map(|(h, k, c)| ItemGuard {
            handle: h,
            key: k,
            value: &c.value,
            counter: &c.refcount,
            token: Some(c.refcount.get()),
        })
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = ItemGuardMut<'_, K, V>> {
        self.inner.iter_mut().map(|(h, k, c)| {
            let Counted { refcount, value } = c;
            ItemGuardMut {
                handle: h,
                key: k,
                value,
                counter: refcount,
                token: Some(refcount.get()),
            }
        })
    }
}

/// Read-only item guard yielded by `iter()`.
pub struct ItemGuard<'a, K, V> {
    handle: Handle,
    key: &'a K,
    value: &'a V,
    counter: &'a UsizeCount,
    token: Option<Token<'a, UsizeCount>>, // consumed on Drop
}

impl<'a, K, V> ItemGuard<'a, K, V> {
    pub fn key(&self) -> &'a K {
        self.key
    }
    pub fn value(&self) -> &'a V {
        self.value
    }
}

impl<'a, K, V> Deref for ItemGuard<'a, K, V> {
    type Target = V;
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, K, V> Drop for ItemGuard<'a, K, V> {
    fn drop(&mut self) {
        if let Some(t) = self.token.take() {
            let _ = self.counter.put(t);
        }
    }
}

/// Mutable item guard yielded by `iter_mut()`.
pub struct ItemGuardMut<'a, K, V> {
    handle: Handle,
    key: &'a K,
    value: &'a mut V,
    counter: &'a UsizeCount,
    token: Option<Token<'a, UsizeCount>>, // consumed on Drop
}

impl<'a, K, V> ItemGuardMut<'a, K, V> {
    pub fn handle(&self) -> CountedHandle<'a> {
        let t = self.counter.get();
        CountedHandle {
            handle: self.handle,
            token: t,
        }
    }
    pub fn key(&self) -> &'a K {
        self.key
    }
    pub fn value_mut(&mut self) -> &mut V {
        self.value
    }
}

impl<'a, K, V> Deref for ItemGuardMut<'a, K, V> {
    type Target = V;
    fn deref(&self) -> &Self::Target {
        self.value
    }
}

impl<'a, K, V> DerefMut for ItemGuardMut<'a, K, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.value
    }
}

impl<'a, K, V> Drop for ItemGuardMut<'a, K, V> {
    fn drop(&mut self) {
        if let Some(t) = self.token.take() {
            let _ = self.counter.put(t);
        }
    }
}
