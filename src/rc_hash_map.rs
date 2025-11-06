use crate::tokens::{Count, RcCount, Token};
// Keepalive handled via direct Rc strong-count inc/dec per entry.
use crate::counted_hash_map::{CountedHandle, CountedHashMap, PutResult};
use crate::handle_hash_map::InsertError;
use core::cell::RefCell;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

// Stored value wrapper that holds a keepalive token from `Inner`'s RcCount
// to keep the allocation alive. The token is returned when the last Ref
// for this entry is dropped and the entry is removed.
struct RcVal<K, V, S> {
    value: V,
    keepalive_token: Token<'static, RcCount<Inner<K, V, S>>>,
}

struct Inner<K, V, S> {
    map: RefCell<CountedHashMap<K, RcVal<K, V, S>, S>>, // single-threaded interior mutability
    keepalive: RcCount<Inner<K, V, S>>,
}

pub struct RcHashMap<K, V, S = std::collections::hash_map::RandomState> {
    inner: Rc<Inner<K, V, S>>,
}

impl<K, V> RcHashMap<K, V>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
{
    pub fn new() -> Self {
        Self {
            inner: Rc::new_cyclic(|weak| Inner {
                map: RefCell::new(CountedHashMap::new()),
                keepalive: RcCount::from_weak(weak),
            }),
        }
    }
}

impl<K, V, S> RcHashMap<K, V, S>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            inner: Rc::new_cyclic(|weak| Inner {
                map: RefCell::new(CountedHashMap::with_hasher(hasher)),
                keepalive: RcCount::from_weak(weak),
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

    pub fn insert(&mut self, key: K, value: V) -> Result<Ref<K, V, S>, InsertError> {
        let out = {
            let mut map = self.inner.map.borrow_mut();
            if map.contains_key(&key) {
                return Err(InsertError::DuplicateKey);
            }
            // Acquire keepalive token upfront; safe because we've checked duplicates.
            let token = self.inner.keepalive.get();
            let wrapped = RcVal {
                value,
                keepalive_token: token,
            };
            let ch = match map.insert(key, wrapped) {
                Ok(ch) => ch,
                Err(e) => {
                    // Should not happen after contains_key check; no way to recover the token here.
                    return Err(e);
                }
            };
            let ch_static: CountedHandle<'static> = unsafe { core::mem::transmute(ch) };
            Ok(Ref::new(NonNull::from(self.inner.as_ref()), ch_static))
        };
        out
    }

    pub fn get(&self, key: &K) -> Option<Ref<K, V, S>> {
        let out = {
            let map = self.inner.map.borrow();
            let x = match map.find(key) {
                Some(ch) => {
                    let ch_static: CountedHandle<'static> = unsafe { core::mem::transmute(ch) };
                    Some(Ref::new(NonNull::from(self.inner.as_ref()), ch_static))
                }
                None => None,
            };
            x
        };
        out
    }
}

/// A reference to an entry inside RcHashMap. Clone increments per-entry count;
/// dropping decrements and removes the entry when it reaches zero.
pub struct Ref<K, V, S = std::collections::hash_map::RandomState>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    owner_ptr: NonNull<Inner<K, V, S>>,
    handle: Option<CountedHandle<'static>>,
    _nosend: PhantomData<*mut ()>,
}

impl<K, V, S> Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn new(owner_ptr: NonNull<Inner<K, V, S>>, handle: CountedHandle<'static>) -> Self {
        Self {
            owner_ptr,
            handle: Some(handle),
            _nosend: PhantomData,
        }
    }

    pub fn value(&self) -> Option<std::cell::Ref<'_, V>> {
        let inner = unsafe { self.owner_ptr.as_ref() };
        let borrow = inner.map.borrow();
        let ch = self.handle.as_ref()?;
        if ch.value_ref(&borrow).is_some() {
            Some(std::cell::Ref::map(borrow, |m| {
                &ch.value_ref(m)
                    .expect("entry must exist while Ref is live")
                    .value
            }))
        } else {
            None
        }
    }
}

impl<K, V, S> Clone for Ref<K, V, S>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    fn clone(&self) -> Self {
        // Increment per-entry count via counted handle API.
        let inner = unsafe { self.owner_ptr.as_ref() };
        let map = inner.map.borrow();
        let ch2 = map.get(self.handle.as_ref().expect("live ref must have handle"));
        let ch2_static: CountedHandle<'static> = unsafe { core::mem::transmute(ch2) };
        Ref::new(self.owner_ptr, ch2_static)
    }
}

impl<K, V, S> Drop for Ref<K, V, S>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    fn drop(&mut self) {
        let inner = unsafe { self.owner_ptr.as_ref() };
        if let Some(ch) = self.handle.take() {
            let res = inner.map.borrow_mut().put(ch);
            match res {
                PutResult::Live => {}
                PutResult::Removed { key, value } => {
                    // Drop user data first while keepalive still holds Inner alive via strong count
                    let RcVal {
                        value: user_value,
                        keepalive_token,
                    } = value;
                    drop(key);
                    drop(user_value);
                    // Return the keepalive token to decrement the strong count.
                    inner.keepalive.put(keepalive_token);
                }
            }
        }
    }
}

impl<K, V, S> PartialEq for Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn eq(&self, other: &Self) -> bool {
        self.owner_ptr == other.owner_ptr
            && self.handle.as_ref().map(|h| h.handle) == other.handle.as_ref().map(|h| h.handle)
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
        (self.owner_ptr.as_ptr() as usize).hash(state);
        if let Some(h) = &self.handle {
            h.handle.hash(state);
        }
    }
}
