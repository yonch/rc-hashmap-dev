use crate::tokens::{Count, RcCount, Token};
// Keepalive handled via direct Rc strong-count inc/dec per entry.
use crate::counted_hash_map::{CountedHandle, CountedHashMap, PutResult};
use crate::handle_hash_map::InsertError;
use core::cell::UnsafeCell;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use std::ptr::NonNull;
use core::mem::ManuallyDrop;
use std::rc::Rc;

// Stored value wrapper that holds a keepalive token from `Inner`'s RcCount
// to keep the allocation alive. The token is returned when the last Ref
// for this entry is dropped and the entry is removed.
struct RcVal<K, V, S> {
    value: V,
    keepalive_token: Token<'static, RcCount<Inner<K, V, S>>>,
}

struct Inner<K, V, S> {
    map: UnsafeCell<CountedHashMap<K, RcVal<K, V, S>, S>>, // interior mutability via UnsafeCell
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
                map: UnsafeCell::new(CountedHashMap::new()),
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
    // Internal helpers to access the inner map via UnsafeCell in one place.
    fn map(&self) -> &CountedHashMap<K, RcVal<K, V, S>, S> {
        unsafe { &*self.inner.map.get() }
    }
    fn map_mut(&mut self) -> &mut CountedHashMap<K, RcVal<K, V, S>, S> {
        unsafe { &mut *self.inner.map.get() }
    }
    fn map_and_rccount_mut(
        &mut self,
    ) -> (
        &mut CountedHashMap<K, RcVal<K, V, S>, S>,
        &RcCount<Inner<K, V, S>>,
    ) {
        let m = unsafe { &mut *self.inner.map.get() };
        let rc = &self.inner.keepalive;
        (m, rc)
    }
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            inner: Rc::new_cyclic(|weak| Inner {
                map: UnsafeCell::new(CountedHashMap::with_hasher(hasher)),
                keepalive: RcCount::from_weak(weak),
            }),
        }
    }

    pub fn len(&self) -> usize {
        self.map().len()
    }
    pub fn is_empty(&self) -> bool {
        self.map().is_empty()
    }

    pub fn contains_key<Q>(&self, q: &Q) -> bool
    where
        K: core::borrow::Borrow<Q>,
        Q: ?Sized + core::hash::Hash + Eq,
    {
        self.map().contains_key(q)
    }

    pub fn insert(&mut self, key: K, value: V) -> Result<Ref<K, V, S>, InsertError> {
        let (map, keepalive) = self.map_and_rccount_mut();
        let res = map.insert_with(key, || RcVal {
            value,
            keepalive_token: keepalive.get(),
        });
        match res {
            Ok(ch) => Ok(Ref::new(NonNull::from(self.inner.as_ref()), ch)),
            Err(e) => Err(e),
        }
    }

    pub fn find<Q>(&self, q: &Q) -> Option<Ref<K, V, S>>
    where
        K: core::borrow::Borrow<Q>,
        Q: ?Sized + core::hash::Hash + Eq,
    {
        self.map()
            .find(q)
            .map(|ch| Ref::new(NonNull::from(self.inner.as_ref()), ch))
    }

    pub fn iter(&self) -> Iter<'_, K, V, S> {
        let owner_ptr = NonNull::from(self.inner.as_ref());
        let inner = self.map().iter_raw();
        Iter { owner_ptr, inner }
    }

    pub fn iter_mut(&mut self) -> IterMut<'_, K, V, S> {
        let owner_ptr = NonNull::from(self.inner.as_ref());
        let inner = self.map_mut().iter_mut_raw();
        IterMut { owner_ptr, inner }
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
    handle: ManuallyDrop<CountedHandle<'static>>,
    _nosend: PhantomData<*mut ()>,
}

/// Owner-mismatch error for Ref accessors.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct WrongMap;

impl<K, V, S> Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn new(owner_ptr: NonNull<Inner<K, V, S>>, handle: CountedHandle<'static>) -> Self {
        Self {
            owner_ptr,
            handle: ManuallyDrop::new(handle),
            _nosend: PhantomData,
        }
    }

    #[inline]
    fn check_owner<'a>(&'a self, map: &'a RcHashMap<K, V, S>) -> Result<(), WrongMap> {
        // Safety: owner_ptr is created from Rc::as_ref; compare raw pointers for identity.
        let ptr = NonNull::from(map.inner.as_ref());
        if ptr == self.owner_ptr {
            Ok(())
        } else {
            Err(WrongMap)
        }
    }

    /// Borrow the entry's key, validating owner identity.
    pub fn key<'a>(&'a self, map: &'a RcHashMap<K, V, S>) -> Result<&'a K, WrongMap> {
        self.check_owner(map)?;
        self.handle.key_ref(map.map()).ok_or(WrongMap)
    }

    /// Borrow the entry's value, validating owner identity.
    pub fn value<'a>(&'a self, map: &'a RcHashMap<K, V, S>) -> Result<&'a V, WrongMap> {
        self.check_owner(map)?;
        self.handle.value_ref(map.map())
            .map(|rcv| &rcv.value)
            .ok_or(WrongMap)
    }

    /// Mutably borrow the entry's value, validating owner identity.
    pub fn value_mut<'a>(&'a self, map: &'a mut RcHashMap<K, V, S>) -> Result<&'a mut V, WrongMap> {
        if NonNull::from(map.inner.as_ref()) != self.owner_ptr {
            return Err(WrongMap);
        }
        // SAFETY: owner validated and we have &mut map, so exclusive access for 'a
        self.check_owner(map)?; // ensure owner match
        self.handle.value_mut(map.map_mut())
            .map(|rcv| &mut rcv.value)
            .ok_or(WrongMap)
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
        let handle = unsafe { &*inner.map.get() }.get(&self.handle);
        Ref::new(self.owner_ptr, handle)
    }
}

impl<K, V, S> Drop for Ref<K, V, S>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    fn drop(&mut self) {
        let inner = unsafe { &mut *(self.owner_ptr.as_ptr()) };
        // Move out the handle without running its destructor.
        let ch = unsafe { ManuallyDrop::take(&mut self.handle) };
        let res = unsafe { &mut *inner.map.get() }.put(ch);
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

impl<K, V, S> PartialEq for Ref<K, V, S>
where
    K: Eq + core::hash::Hash,
    S: core::hash::BuildHasher + Clone + Default,
{
    fn eq(&self, other: &Self) -> bool {
        self.owner_ptr == other.owner_ptr && self.handle.handle == other.handle.handle
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
        self.handle.handle.hash(state);
    }
}
/// Placeholder for future mutable iterator item (see design docs).
pub struct ItemMut<'a, K, V, S = std::collections::hash_map::RandomState>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    r: Ref<K, V, S>,
    k: &'a K,
    v: &'a mut V,
}
impl<'a, K, V, S> ItemMut<'a, K, V, S>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    pub fn r#ref(&self) -> &Ref<K, V, S> {
        &self.r
    }
    pub fn key(&self) -> &K {
        self.k
    }
    pub fn value_mut(&mut self) -> &mut V {
        self.v
    }
}

/// Immutable iterator for RcHashMap yielding `Ref`.
pub struct Iter<'a, K, V, S = std::collections::hash_map::RandomState>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    owner_ptr: NonNull<Inner<K, V, S>>,
    inner: crate::counted_hash_map::Iter<'a, K, RcVal<K, V, S>, S>,
}

impl<'a, K, V, S> Iterator for Iter<'a, K, V, S>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    type Item = Ref<K, V, S>;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(ch, _k, _rv)| Ref::new(self.owner_ptr, ch))
    }
}

/// Mutable iterator for RcHashMap yielding ItemMut.
pub struct IterMut<'a, K, V, S = std::collections::hash_map::RandomState>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    owner_ptr: NonNull<Inner<K, V, S>>,
    inner: crate::counted_hash_map::IterMut<'a, K, RcVal<K, V, S>, S>,
}

impl<'a, K, V, S> Iterator for IterMut<'a, K, V, S>
where
    K: Eq + core::hash::Hash + 'static,
    V: 'static,
    S: core::hash::BuildHasher + Clone + Default + 'static,
{
    type Item = ItemMut<'a, K, V, S>;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|(ch, k, rv)| {
            let r = Ref::new(self.owner_ptr, ch);
            ItemMut {
                r,
                k,
                v: &mut rv.value,
            }
        })
    }
}
