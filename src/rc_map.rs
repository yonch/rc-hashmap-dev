use crate::tokens::Token;
use crate::util_counted_map::{CountedHandle, CountedHashMap, PutResult};
use crate::util_handle_map::{Handle, InsertError};
use core::cell::RefCell;
use core::hash::{Hash, Hasher};
use std::rc::Rc;

struct Inner<K, V, S> {
    map: RefCell<CountedHashMap<K, V, S>>, // single-threaded interior mutability
}

pub struct RcHashMap<K, V, S = std::collections::hash_map::RandomState> {
    inner: Rc<Inner<K, V, S>>,
}

impl<K, V> RcHashMap<K, V>
where
    K: Eq + core::hash::Hash,
{
    pub fn new() -> Self {
        Self {
            inner: Rc::new(Inner {
                map: RefCell::new(CountedHashMap::new()),
            }),
        }
    }
}

impl<K, V, S> RcHashMap<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            inner: Rc::new(Inner {
                map: RefCell::new(CountedHashMap::with_hasher(hasher)),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.inner.map.borrow().len()
    }
    pub fn is_empty(&self) -> bool {
        self.inner.map.borrow().is_empty()
    }

    pub fn contains_key<Q>(&self, q: &Q) -> bool
    where
        K: core::borrow::Borrow<Q>,
        Q: ?Sized + core::hash::Hash + Eq,
    {
        self.inner.map.borrow().contains_key(q)
    }

    pub fn insert(&self, key: K, value: V) -> Result<Ref<K, V, S>, InsertError> {
        let mut binding = self.inner.map.borrow_mut();
        let ch = binding.insert(key, value)?;
        // The returned counted handle holds a token representing one ref; transfer
        // responsibility to the returned Ref by forgetting it here.
        let handle = ch.handle;
        core::mem::forget(ch.token);
        Ok(Ref::new(self.inner.clone(), handle))
    }

    pub fn get(&self, key: &K) -> Option<Ref<K, V, S>> {
        let binding = self.inner.map.borrow();
        let ch = binding.find(key)?;
        let handle = ch.handle;
        // Transfer responsibility to the returned Ref.
        core::mem::forget(ch.token);
        Some(Ref::new(self.inner.clone(), handle))
    }
}

/// A reference to an entry inside RcHashMap. Clone increments per-entry count;
/// dropping decrements and removes the entry when it reaches zero.
pub struct Ref<K, V, S = std::collections::hash_map::RandomState>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    owner: Rc<Inner<K, V, S>>, // keep owner alive
    owner_ptr: *const Inner<K, V, S>,
    handle: Handle,
}

impl<K, V, S> Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn new(owner: Rc<Inner<K, V, S>>, handle: Handle) -> Self {
        let owner_ptr = Rc::as_ptr(&owner);
        Self {
            owner,
            owner_ptr,
            handle,
        }
    }

    pub fn handle(&self) -> Handle {
        self.handle
    }

    pub fn value(&self) -> Option<std::cell::Ref<'_, V>> {
        let borrow = self.owner.map.borrow();
        if borrow.inner.handle_value(self.handle).is_some() {
            Some(std::cell::Ref::map(borrow, |m| {
                &m.inner
                    .handle_value(self.handle)
                    .expect("checked is_some above")
                    .value
            }))
        } else {
            None
        }
    }
}

impl<K, V, S> Clone for Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn clone(&self) -> Self {
        // Increment per-entry count via counted handle API.
        let borrow = self.owner.map.borrow();
        // Create a temporary counted handle to use the API; forget its token
        let temp = CountedHandle {
            handle: self.handle,
            token: Token::new(),
        };
        let more = borrow.get(&temp);
        core::mem::forget(temp.token);
        core::mem::forget(more.token);
        Self {
            owner: self.owner.clone(),
            owner_ptr: self.owner_ptr,
            handle: self.handle,
        }
    }
}

impl<K, V, S> Drop for Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn drop(&mut self) {
        // Decrement; if zero, remove the entry immediately.
        let res = {
            let mut b = self.owner.map.borrow_mut();
            let h = CountedHandle {
                handle: self.handle,
                token: Token::new(),
            };
            b.put(h)
        };
        if let PutResult::Removed { key: _k, value: _v } = res {
            // Drop key then value in this scope.
            // NOTE: values were returned already as moved by `put`.
            // Explicit drops to document order.
            // (Bindings already moved by match; nothing further to do.)
        }
    }
}

impl<K, V, S> PartialEq for Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn eq(&self, other: &Self) -> bool {
        self.owner_ptr == other.owner_ptr && self.handle == other.handle
    }
}

impl<K, V, S> Eq for Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
}

impl<K, V, S> Hash for Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.owner_ptr as usize).hash(state);
        self.handle.hash(state);
    }
}
