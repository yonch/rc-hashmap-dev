use crate::tokens::RcCount;
// Keepalive handled via direct Rc strong-count inc/dec per entry.
use crate::util_counted_map::{CountedHandle, CountedHashMap, PutResult};
use crate::util_handle_map::InsertError;
use core::cell::RefCell;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

// Stored value wrapper that holds a raw pointer to `Inner` to keep
// the allocation alive by manually adjusting Rc strong counts.
struct RcVal<K, V, S> {
    value: V,
    owner_raw: *const Inner<K, V, S>,
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
    K: Eq + core::hash::Hash,
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
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
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

    pub fn insert(&self, key: K, value: V) -> Result<Ref<K, V, S>, InsertError> {
        // Increment Inner strong count and wrap the user value with the raw pointer.
        let raw = Rc::as_ptr(&self.inner);
        unsafe { Rc::increment_strong_count(raw) };
        let wrapped = RcVal {
            value,
            owner_raw: raw,
        };

        let out = {
            let mut map = self.inner.map.borrow_mut();
            let x = match map.insert(key, wrapped) {
                Ok(ch) => {
                    let ch_static: CountedHandle<'static> = unsafe { core::mem::transmute(ch) };
                    Ok(Ref::new(NonNull::from(self.inner.as_ref()), ch_static))
                }
                Err(e) => {
                    // On failure, undo the strong-count increment.
                    unsafe { Rc::decrement_strong_count(raw) };
                    Err(e)
                }
            };
            x
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
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
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
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
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
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn drop(&mut self) {
        let inner = unsafe { self.owner_ptr.as_ref() };
        if let Some(ch) = self.handle.take() {
            let res = inner.map.borrow_mut().put(ch);
            match res {
                PutResult::Live => {}
                PutResult::Removed { key, mut value } => {
                    // Drop user data first while keepalive still holds Inner alive via strong count
                    drop(key);
                    drop(value.value);
                    // Decrement strong count that was incremented on insert.
                    unsafe { Rc::decrement_strong_count(value.owner_raw) };
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
