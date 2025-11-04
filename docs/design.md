RcHashMap: Single-threaded, refcounted map layered over an IndexedSlotMap

Summary
- Goal: Build RcHashMap in safe, verifiable layers so we can reason about each piece independently.
- Layers:
  - IndexedSlotMap<K, V, S>: raw index + slot storage with unique-key enforcement; no refcounting.
  - CountedIndexedSlotMap<K, V, S>: wraps IndexedSlotMap and adds per-entry refcounting (increments on get/clone, decrements on put). This layer has no knowledge of higher-level handles.
  - RcHashMap<K, V, S>: public API with `Ref`backed by `Rc<Inner>`; keepalive is implemented via `Rc` strong counts (one per map + one per live entry). `Ref` handles are defined only here and delegate to CountedIndexedSlotMap.
- Constraints: single-threaded, no atomics, no per-entry heap allocations, stable generational keys, O(1) average lookups, unique keys.

Why this split?
- Localize invariants: each layer has a small, precise contract. We can test and audit them separately.
- Keep unsafe to a minimum: raw-pointer identity checks exist only in RcHashMap; pure indexing logic is safe Rust.
- Clear failure boundaries: IndexedSlotMap never calls into user code once the data structure is in a consistent state.

Module 1: IndexedSlotMap
- Purpose: Combine hashbrown::raw::RawTable as an index with slotmap::SlotMap as storage. No refcounts; unique keys are enforced at this layer.
- Entry<K, V>
  - key: K — user key; used for Eq/Hash only.
  - value: V — stored inline.
  - hash: u64 — precomputed with the map’s BuildHasher.
- API (sketch)
  - new(hasher: S) -> Self
  - find(&self, key: &K) -> Option<DefaultKey>
  - insert_unique(&mut self, key: K, value: V) -> Result<DefaultKey, InsertError>
  - key(&self, slot: DefaultKey) -> Option<&K>
  - value(&self, slot: DefaultKey) -> Option<&V>
  - value_mut(&mut self, slot: DefaultKey) -> Option<&mut V>
  - remove(&mut self, slot: DefaultKey) -> Option<(K, V)>
- Behavior
  - Indexing: store only DefaultKey in RawTable, keyed by hash; resolve collisions with K: Eq against entries[slot].key.
  - Insertion: compute hash(key); if key exists (found by probing+Eq), return `Err(InsertError::Duplicate)` and do not modify the map. Otherwise, insert into entries first to obtain slot, then insert slot into index under that hash.
  - Lookup: compute hash(key), probe index, compare by Eq.
  - Removal: remove from index first, then remove the slot from entries and return (K, V).
- Safety and consistency
  - remove() guarantees the data structure is consistent (index and storage no longer reference the slot) before dropping K and V.
  - All public methods leave the data structure in a consistent state before any user code can run, except K: Hash and K: Eq that are invoked during probing. We document this as a footgun for higher layers: if Eq/Hash have side-effects, they must not reenter this map.
  - No refcounting or keepalive here; purely structural.

Module 2: CountedIndexedSlotMap
- Purpose: Add simple reference counting on top of IndexedSlotMap (not for keepalive, and not aware of higher-level handle types).
- Representation
  - Wrap values as Counted<V> = { refcount: Cell<usize>, value: V }.
  - Internally: IndexedSlotMap<K, Counted<V>, S>.
- API (same surface, plus helpers)
  - find(&self, key: &K) -> Option<DefaultKey>
    - If found, increments refcount and returns the slot. Uses interior mutability for the count.
  - insert_unique(&mut self, key: K, value: V) -> Result<DefaultKey, InsertError>
    - Delegates to IndexedSlotMap’s unique insertion. On success, initializes refcount = 1 and returns the slot.
  - value(&self, slot: DefaultKey) -> Option<&V>
  - value_mut(&mut self, slot: DefaultKey) -> Option<&mut V>
  - key(&self, slot: DefaultKey) -> Option<&K>
  - put(&self, slot: DefaultKey) -> PutResult
    - Decrements refcount; if it reaches 0, removes the slot from the underlying IndexedSlotMap and returns `PutResult::Removed { key: K, value: V }`. Otherwise returns `PutResult::Live`.
  - inc(&self, slot: DefaultKey)
    - Increments for cloning/duplication of a slot reference; panics on overflow; panics on invalid slot in debug builds.
- Notes
  - Unique-key policy is enforced in Module 1 and reused here unchanged; refcounting is orthogonal.
  - All increments/decrements are interior-mutable and single-threaded.
  - The underlying IndexedSlotMap remains consistent at all times; drops of K and V happen only after the slot is removed from both index and storage.

Module 3: RcHashMap
- Purpose: Public, ergonomic API with `Ref` handles, Rc-based keepalive, and owner identity checks. Internally holds `Rc<Inner>`.
- Keepalive model (via Rc strong counts)
  - RcHashMap holds `Rc<Inner>`. `Inner` also stores a raw pointer `raw_rc: *const Inner` obtained from `Rc::as_ptr(&rc)`.
  - Each live entry contributes one additional strong count: on successful insertion, call `Rc::increment_strong_count(raw_rc)`; on final removal (when the entry’s `refcount` reaches 0 and the slot is removed), call `Rc::decrement_strong_count(raw_rc)`.
  - Dropping RcHashMap drops its own `Rc` handle; if entries still exist, their per-entry strong counts keep `Inner` alive until those entries are removed.
  - On final removal of the last entry after map drop, the last `decrement_strong_count` frees `Inner`. We call decrement before dropping `K`/`V` so user code in `Drop` runs after `Inner` may already be freed; this is safe because the structure is fully consistent and detached before user code runs.
- Unique keys policy
  - RcHashMap enforces unique keys by delegating to Module 1’s unique insertion. `insert` fails if the key already exists (no modification).
- Ref handle
  - Fields: `{ slot: DefaultKey, owner: NonNull<Inner<…>>, map_id: NonZeroU64, _nosend: PhantomData<*mut ()> }`.
  - Clone: `impl Clone for Ref` increments per-entry count via `inc`; if increment would overflow `usize`, it panics (checked before overflow to avoid UB).
  - Drop: decrements per-entry count via `put`; if it reaches 0, performs physical removal. Removal path calls `Rc::decrement_strong_count(raw_rc)` before dropping `K` and `V`.
  - Hash/Eq: `(map_id, slot)`.
- Accessors
  - `get<Q>(&self, key: &Q) -> Option<Ref>` where `K: Borrow<Q>, Q: Hash + Eq`: delegates to `counted.find(key)` which increments the per-entry refcount upon success.
  - `insert(&mut self, key: K, value: V) -> Result<Ref, InsertError>`: on success, increments the Rc strong count (per-entry) and returns the new `Ref` (unique keys enforced in Module 1).
  - `value<'a>(&'a self, r: &'a Ref) -> Option<&'a V>`; `value_mut<'a>(&'a self, r: &'a Ref) -> Option<&'a mut V>`; `key<'a>(&'a self, r: &'a Ref) -> Option<&'a K>`.
  - Returned references are tied to both the map borrow and the `Ref` lifetime.

Correctness and footguns
- The only user code that can run while the structure is not yet consistent is `K: Hash` and `K: Eq` during probing. These must not reenter the map or cause observable aliasing.
- Final removal order: remove from index → remove from storage and obtain `(K, V)` → decrement per-entry Rc strong count via `Rc::decrement_strong_count(raw_rc)` → then drop `K` and `V`.
- Dropping `K`/`V` may execute user code after `Inner` has been freed (if this was the last entry and the map was already dropped); this is safe because the data structure is no longer referenced.
- Single-threaded only; both `RcHashMap` and `Ref` are `!Send + !Sync`.

Code sketch
```rust
// IndexedSlotMap
struct IndexedSlotMap<K, V, S> { /* entries: SlotMap<DefaultKey, Entry<K,V>>, index: RawTable<DefaultKey>, hasher: S */ }
impl<K: Eq + Hash, V, S: BuildHasher> IndexedSlotMap<K, V, S> {
    fn find<Q: ?Sized + Hash + Eq>(&self, q: &Q) -> Option<DefaultKey> where K: Borrow<Q> { /* probe */ }
    fn insert_unique(&mut self, k: K, v: V) -> Result<DefaultKey, InsertError> { /* add to entries then index, fail on dup */ }
    fn value(&self, s: DefaultKey) -> Option<&V> { /* read */ }
    fn value_mut(&mut self, s: DefaultKey) -> Option<&mut V> { /* write */ }
    fn key(&self, s: DefaultKey) -> Option<&K> { /* read */ }
    fn remove(&mut self, s: DefaultKey) -> Option<(K,V)> { /* index first, then entries */ }
}

// CountedIndexedSlotMap
struct CountedIndexedSlotMap<K, V, S>(IndexedSlotMap<K, Counted<V>, S>);
struct Counted<V> { refcount: Cell<usize>, value: V }
impl<K: Eq + Hash, V, S: BuildHasher> CountedIndexedSlotMap<K, V, S> {
    fn find<Q: ?Sized + Hash + Eq>(&self, q: &Q) -> Option<DefaultKey> where K: Borrow<Q> { /* inc on hit */ }
    fn insert_unique(&mut self, k: K, v: V) -> Result<DefaultKey, InsertError> { /* refcount=1; fail on dup */ }
    fn inc(&self, s: DefaultKey) { /* inc for cloning; checked overflow; panic on overflow */ }
    fn put(&self, s: DefaultKey) -> PutResult { /* dec; remove and return K,V at zero */ }
    fn value(&self, s: DefaultKey) -> Option<&V> { /* read */ }
    fn value_mut(&mut self, s: DefaultKey) -> Option<&mut V> { /* write */ }
    fn key(&self, s: DefaultKey) -> Option<&K> { /* read */ }
}

// RcHashMap (public)
struct RcHashMap<K, V, S> { inner: Rc<Inner<K,V,S>> }
struct Inner<K, V, S> {
    counted: CountedIndexedSlotMap<K, V, S>,
    raw_rc: *const Inner<K,V,S>,
    map_id: NonZeroU64,
    // plus interior mutability guards/markers to keep !Send + !Sync
}
struct Ref<K, V, S> { slot: DefaultKey, owner: NonNull<Inner<K,V,S>>, map_id: NonZeroU64, _nosend: PhantomData<*mut ()> }

impl<K, V, S> RcHashMap<K, V, S> {
    fn insert(&mut self, k: K, v: V) -> Result<Ref<K,V,S>, InsertError> {
        let slot = self.inner.counted.insert_unique(k, v)?;
        // Entry live: bump per-entry strong count
        unsafe { Rc::increment_strong_count(self.inner.raw_rc) }
        Ok(Ref { /* … */ })
    }
    fn get<Q>(&self, q: &Q) -> Option<Ref<K,V,S>> where K: Borrow<Q>, Q: ?Sized + Hash + Eq {
        self.inner.counted.find(q).map(|slot| Ref { /* … */ })
    }
}

impl<K, V, S> Ref<K, V, S> {
    fn drop(&mut self) {
        match unsafe { self.owner.as_ref() }.counted.put(self.slot) {
            PutResult::Live => {}
            PutResult::Removed { key, value } => {
                // Per-entry keepalive: decrement before dropping K,V
                let inner = unsafe { self.owner.as_ref() };
                unsafe { Rc::decrement_strong_count(inner.raw_rc) };
                drop(key); drop(value);
            }
        }
    }
}
```

Testing plan
- IndexedSlotMap
  - Insert/find/remove sequences preserve index ↔ storage consistency; removal drops after consistency.
  - Reentrancy stress via Drop for K/V: ensure removal is index-first.
  - Collision paths: multiple keys with same hash verified by Eq checks.
- CountedIndexedSlotMap
  - `find` increments; `put` decrements; actual removal only at zero.
  - Overflow behavior for refcount increments panics on overflow (checked before wrap). `inc` is infallible and panics on overflow.
  - value/value_mut observe the same slot identity across increments.
- RcHashMap
  - `Ref::clone` increments per-entry count; `Ref::drop` decrements and removes at zero.
  - Rc-based keepalive: map drop with live entries leaves `Inner` alive via per-entry strong counts; final removal of last entry frees `Inner`.
  - Removal path decrements per-entry strong count before dropping `K`/`V`.
  - Owner identity: wrong-map `Ref` rejected by accessors; `Eq`/`Hash` include `map_id` + slot.
  - Unique keys enforced: `insert` fails on duplicate.

Notes and non-goals
- Still single-threaded; no Send/Sync.
- No weak handles (can be added later).
- Unique-keys policy enforced in Module 1: `insert_unique` fails on duplicate; RcHashMap relies on this behavior.

Overflow semantics
- We assume practical refcount overflow is unrealistic; nonetheless, all increments are checked and `Ref::clone`/`get`/iterator cloning will panic on overflow rather than wrap or UB. `try_inc` is removed; use `inc` which panics on overflow.
