RcHashMap: Single-threaded, handle-based map with Rc-like references to entries that allow fast access and cleanup on drop

Summary
- Goal: Build RcHashMap in safe, verifiable layers so we can reason about each piece independently.
- Layers:
  - HandleHashMap<K, V, S>: A HashMap like datastructure that provides handles for quick access to internal entries without hashing.
  - CountedHashMap<K, V, S>: wraps HandleHashMap and adds per-entry refcounting (increments on get/clone, decrements on put).
  - RcHashMap<K, V, S>: wraps HandleHashMap and adds a `Ref` handle, which frees the reference when dropped.
- Constraints: single-threaded, no atomics, no per-entry heap allocations, stable generational keys, O(1) average lookups, unique keys.

Why this split?
- Localize invariants: each layer has a small, precise contract. We can test and audit them separately.
- Keep unsafe to a minimum: raw-pointer identity checks exist only in RcHashMap; pure indexing logic is safe Rust.
- Clear failure boundaries: HandleHashMap never calls into user code once the data structure is in a consistent state.

Module 1: HandleHashMap
- How: Combine hashbrown::raw::RawTable as an index with slotmap::SlotMap as storage. Handles are slotmap "keys" and provide efficient access to the entries.
- Entry<K, V>
  - key: K — user key; used for Eq/Hash only.
  - value: V — stored inline.
  - hash: u64 — precomputed with the map’s BuildHasher.
- API (sketch)
  - new(hasher: S) -> Self
  - find(&self, key: &K) -> Option<DefaultKey>
  - contains_key<Q>(&self, q: &Q) -> bool where K: Borrow<Q>, Q: ?Sized + Hash + Eq
  - insert(&mut self, key: K, value: V) -> Result<DefaultKey, InsertError>
  - key(&self, slot: DefaultKey) -> Option<&K>
  - value(&self, slot: DefaultKey) -> Option<&V>
  - value_mut(&mut self, slot: DefaultKey) -> Option<&mut V>
  - remove(&mut self, slot: DefaultKey) -> Option<(K, V)>
  - len(&self) -> usize; is_empty(&self) -> bool
  - iter(&self) -> impl Iterator<Item = (DefaultKey, &K, &V)>
  - iter_mut(&mut self) -> impl Iterator<Item = (DefaultKey, &K, &mut V)>
- Behavior
  - Indexing: store only DefaultKey in RawTable, keyed by hash; resolve collisions with K: Eq against entries[slot].key.
  - Insertion (two-phase): compute hash(key), probe index (Eq) to reject duplicates, reserve capacity in index and storage; then commit by inserting into storage to obtain a slot and linking the slot into the index under the stored hash. On failure, roll back so the map remains unchanged.
  - Lookup: compute hash(key), probe index, compare by Eq.
  - Removal: remove from index first, then remove the slot from entries and return (K, V).
- Safety and consistency
  - remove() guarantees the data structure is consistent (index and storage no longer reference the slot) before dropping K and V.
  - All public methods leave the data structure in a consistent state before any user code can run, except K: Hash and K: Eq that are invoked during probing. We document this as a footgun for higher layers: if Eq/Hash have side-effects, they must not reenter this map.
  - No refcounting or keepalive here; purely structural.

Module 2: CountedHashMap
- Purpose: Add simple reference counting on top of HandleHashMap (not for keepalive, and not aware of higher-level handle types).
- Representation
  - Wrap values as Counted<V> = { refcount: Cell<usize>, value: V }.
  - Internally: HandleHashMap<K, Counted<V>, S>.
- API (same surface, plus helpers)
  - find(&self, key: &K) -> Option<DefaultKey>
    - If found, increments refcount and returns the slot. Uses interior mutability for the count.
  - insert(&mut self, key: K, value: V) -> Result<DefaultKey, InsertError>
    - Delegates to HandleHashMap’s unique insertion. On success, initializes refcount = 1 and returns the slot.
  - value(&self, slot: DefaultKey) -> Option<&V>
  - value_mut(&mut self, slot: DefaultKey) -> Option<&mut V>
  - key(&self, slot: DefaultKey) -> Option<&K>
  - contains_key<Q>(&self, q: &Q) -> bool where K: Borrow<Q>, Q: ?Sized + Hash + Eq
    - Probes using the index without incrementing refcounts.
  - put(&self, slot: DefaultKey) -> PutResult
    - Decrements refcount; if it reaches 0, removes the slot from the underlying HandleHashMap and returns `PutResult::Removed { key: K, value: V }`. Otherwise returns `PutResult::Live`.
  - inc(&self, slot: DefaultKey)
    - Increments for cloning/duplication of a slot reference; overflow is unchecked (UB), consistent with `Rc`; panics on invalid slot in debug builds.
  - len(&self) -> usize; is_empty(&self) -> bool
  - iter(&self) -> impl Iterator<Item = ItemGuard<'_, K, V>>
    - Before yielding each item, increments the entry's refcount. Yields an RAII guard that:
      - exposes `slot() -> DefaultKey`, `key() -> &K`, `value() -> &V` (and `Deref<Target = V>`),
      - on Drop, calls `put(slot)` to release the increment.
    - This ensures `put` is balanced correctly and never runs while `&K`/`&V` are still borrowed.
  - iter_mut(&mut self) -> impl Iterator<Item = ItemGuardMut<'_, K, V>>
    - Before yielding each item, increments the entry's refcount. Yields an RAII guard that:
      - exposes `slot() -> DefaultKey`, `key() -> &K`, `value_mut() -> &mut V` (and `DerefMut<Target = V>`),
      - on Drop, calls `put(slot)` to release the increment.
    - Using a guard keeps semantics parallel to Module 3 (where `Ref` acts as the guard) and ensures refcount is balanced even on early loop exit.  
  - Notes
  - Unique-key policy is enforced in Module 1 and reused here unchanged; refcounting is orthogonal.
  - All increments/decrements are interior-mutable and single-threaded.
  - The underlying HandleHashMap remains consistent at all times; drops of K and V happen only after the slot is removed from both index and storage.

Module 3: RcHashMap
- Purpose: Public, ergonomic API with `Ref` handles, Rc-based keepalive, and owner identity checks. Internally holds `Rc<Inner>`.
- Keepalive model (via Rc strong counts)
  - RcHashMap holds `Rc<Inner>`. `Inner` also stores a raw pointer `raw_rc: *const Inner` that is obtained via `Rc::into_raw` on a temporary clone of the `Rc<Inner>`. Immediately after storing this pointer, we call `Rc::decrement_strong_count(raw_rc)` to release the clone, so the baseline strong count remains one (owned by the map). This satisfies the safety preconditions of `increment_strong_count/decrement_strong_count` (pointer obtained via `into_raw`) without holding an extra count.
  - Each live entry contributes one additional strong count: on successful insertion, call `Rc::increment_strong_count(raw_rc)`; on final removal (when the entry’s `refcount` reaches 0 and the slot is removed), call `Rc::decrement_strong_count(raw_rc)`.
  - Dropping RcHashMap drops its own `Rc` handle; if entries still exist, their per-entry strong counts keep `Inner` alive until those entries are removed.
  - Final removal order and rationale: drop `K` and `V` first while `Inner` remains alive (kept by the per-entry strong count), then decrement the per-entry strong count. This preserves safety if user `Drop` for `K`/`V` reenters the map.
  - Length as an invariant aid: all three layers expose `len()`/`is_empty()`. Although Rc-based keepalive does not require checking `len()==0` to trigger deallocation, `len()==0` on `CountedHashMap` precisely indicates “no live entries remain”, which is useful for assertions and tests around last-entry removal.
  - Refcount rationale (conceptual model → efficient implementation):
    - Conceptually, every `Ref` keeps two things alive: (1) its specific entry `(K,V)` and (2) the owning map. If implemented literally with `Rc<(K,V)>` stored per entry and an `Rc<Inner>` inside every `Ref`, this would add heap and pointer overhead to every entry and every `Ref` clone.
    - Instead, we implement the same semantics at lower cost:
      - Entry liveness is tracked by a per-entry `usize` refcount in `CountedHashMap`. Cloning/dropping `Ref` only touches this counter.
      - Map keepalive is centralized: each live entry contributes exactly one extra `Rc<Inner>` strong count for the entire duration the entry is live (i.e., while its per-entry refcount is ≥ 1). We increment this strong count once on insertion (transition to live), and we decrement it once at final removal (transition to dead). Cloning additional `Ref`s to the same entry does not touch the map’s strong count.
      - We avoid storing an `Rc` inside each entry. `Inner` holds a self-pointer (`raw_rc`) obtained via `Rc::into_raw` on a temporary clone. We use `Rc::increment_strong_count(raw_rc)` / `Rc::decrement_strong_count(raw_rc)` to manage the per-entry keepalive for the map without extra allocations or pointer fields per entry.
    - The net effect matches the intuitive model: a `Ref` prevents its entry from being removed, and as long as any entry is live, the map’s `Inner` remains alive. We achieve this without embedding `Rc` in each entry or per-`Ref` heap work.
- Unique keys policy
  - RcHashMap enforces unique keys by delegating to Module 1’s unique insertion. `insert` fails if the key already exists (no modification).
- Ref
  - Fields: `{ slot: DefaultKey, owner: NonNull<Inner<…>>, _nosend: PhantomData<*mut ()> }`.
  - Clone: `impl Clone for Ref` increments per-entry count via `inc`; overflow is unchecked (UB) to match `Rc`.
  - Drop: decrements per-entry count via `put`; if it reaches 0, performs physical removal. Removal path drops `K`/`V` first and then calls `Rc::decrement_strong_count(raw_rc)`.
  - Hash/Eq: `(owner_ptr, slot)`.
  - Accessors
  - `get<Q>(&self, key: &Q) -> Option<Ref>` where `K: Borrow<Q>, Q: Hash + Eq`: delegates to `counted.find(key)` which increments the per-entry refcount upon success.
  - `insert(&mut self, key: K, value: V) -> Result<Ref, InsertError>`: on success, increments the Rc strong count (per-entry) and returns the new `Ref` (unique keys enforced in Module 1).
  - `len(&self) -> usize; is_empty(&self) -> bool` (delegates to Module 2).
  - Access is Ref-centric: methods live on `Ref` and require a map borrow for owner checking.
    - `impl Ref { fn key<'a>(&'a self, map: &'a RcHashMap<..>) -> Result<&'a K, WrongMap>; fn value<'a>(&'a self, map: &'a RcHashMap<..>) -> Result<&'a V, WrongMap>; fn value_mut<'a>(&'a self, map: &'a mut RcHashMap<..>) -> Result<&'a mut V, WrongMap> }`
    - Accessors validate that `self.owner` matches this map’s `Inner` pointer; on mismatch, they return `Err(WrongMap)`.
  - Returned references are tied to both the map borrow and the `Ref` lifetime. All `value_mut` methods require `&mut self` on the map to guarantee uniqueness during mutation.
  - Additional queries: `contains_key(&Q) -> bool` is provided; there is no `peek()` that returns `&V` without a `Ref`, to avoid dangling borrows if the last `Ref` is dropped while holding `&V`.
  - Errors: `WrongMap` is a zero-sized error type indicating an owner mismatch. All `Ref` accessors return `Result<_, WrongMap>`.
  - Accessor lifetime rationale (why `Ref` + `&map`/`&mut map`)
    - The `Ref` borrow ties the returned reference’s lifetime to the handle, ensuring the entry cannot be removed while the reference is live. Without this, a last `Ref` could be dropped while `&V` persists, invalidating the reference.
    - The map borrow enforces aliasing and structural safety:
      - `&RcHashMap` held for the duration of the borrow prevents obtaining `&mut RcHashMap` concurrently, ruling out mutation (e.g., insert/rehash) while `&K`/`&V` is live.
      - `&mut RcHashMap` for `value_mut` guarantees exclusive access to the entire map during the mutable reference, ruling out other reads/writes and preserving “only one `&mut`” semantics.
    - Together, these borrows (to `Ref` and to the map) encode: “the entry stays alive” and “no conflicting aliasing or structural mutation occurs” for the lifetime of the returned reference.
  - Iteration:
    - `iter(&self) -> impl Iterator<Item = (Ref<K,V,S>, &K, &V)>` (creates a `Ref` per entry).
    - `iter_mut(&mut self) -> impl Iterator<Item = (Ref<K,V,S>, &K, &mut V)>`.

Correctness and footguns
- The only user code that can run while the structure is not yet consistent is `K: Hash` and `K: Eq` during probing. These must not reenter the map or cause observable aliasing.
- Final removal order: remove from index → remove from storage and obtain `(K, V)` → drop `K` and `V` → then decrement the per-entry Rc strong count via `Rc::decrement_strong_count(raw_rc)`. See rationale under Keepalive model.
- Because removal happens only on the last handle for that entry, no other `Ref` to the entry exists. If other entries still have `Ref`s, their per-entry strong counts keep `Inner` alive across this decrement.
- Single-threaded only; both `RcHashMap` and `Ref` are `!Send + !Sync` (enforced with `PhantomData<*mut ()>` on `Ref`).

Code sketch
```rust
// HandleHashMap
struct HandleHashMap<K, V, S> { /* entries: SlotMap<DefaultKey, Entry<K,V>>, index: RawTable<DefaultKey>, hasher: S */ }
impl<K: Eq + Hash, V, S: BuildHasher> HandleHashMap<K, V, S> {
    fn find<Q: ?Sized + Hash + Eq>(&self, q: &Q) -> Option<DefaultKey> where K: Borrow<Q> { /* probe */ }
    fn insert(&mut self, k: K, v: V) -> Result<DefaultKey, InsertError> { /* two-phase commit with rollback on failure */ }
    fn value(&self, s: DefaultKey) -> Option<&V> { /* read */ }
    fn value_mut(&mut self, s: DefaultKey) -> Option<&mut V> { /* write */ }
    fn key(&self, s: DefaultKey) -> Option<&K> { /* read */ }
    fn remove(&mut self, s: DefaultKey) -> Option<(K,V)> { /* index first, then entries */ }
    fn len(&self) -> usize { /* number of stored slots */ }
    fn is_empty(&self) -> bool { self.len() == 0 }
}

// CountedHashMap
struct CountedHashMap<K, V, S>(HandleHashMap<K, Counted<V>, S>);
struct Counted<V> { refcount: Cell<usize>, value: V }
impl<K: Eq + Hash, V, S: BuildHasher> CountedHashMap<K, V, S> {
    fn find<Q: ?Sized + Hash + Eq>(&self, q: &Q) -> Option<DefaultKey> where K: Borrow<Q> { /* inc on hit */ }
    fn insert(&mut self, k: K, v: V) -> Result<DefaultKey, InsertError> { /* refcount=1; fail on dup */ }
    fn inc(&self, s: DefaultKey) { /* inc for cloning; unchecked overflow (UB) */ }
    fn put(&self, s: DefaultKey) -> PutResult { /* dec; remove and return K,V at zero */ }
    fn value(&self, s: DefaultKey) -> Option<&V> { /* read */ }
    fn value_mut(&mut self, s: DefaultKey) -> Option<&mut V> { /* write */ }
    fn key(&self, s: DefaultKey) -> Option<&K> { /* read */ }
    fn len(&self) -> usize { /* number of stored/live slots (refcount >= 1) */ }
    fn is_empty(&self) -> bool { self.len() == 0 }
}

// RcHashMap (public)
struct RcHashMap<K, V, S> { inner: Rc<Inner<K,V,S>> }
struct Inner<K, V, S> {
    counted: CountedHashMap<K, V, S>,
    raw_rc: *const Inner<K,V,S>,
    // plus interior mutability guards/markers to keep !Send + !Sync
}
struct Ref<K, V, S> { slot: DefaultKey, owner: NonNull<Inner<K,V,S>>, _nosend: PhantomData<*mut ()> }

impl<K, V, S> RcHashMap<K, V, S> {
    fn insert(&mut self, k: K, v: V) -> Result<Ref<K,V,S>, InsertError> {
        let slot = self.inner.counted.insert(k, v)?;
        // Entry live: bump per-entry strong count
        unsafe { Rc::increment_strong_count(self.inner.raw_rc) }
        Ok(Ref { /* … */ })
    }
    fn get<Q>(&self, q: &Q) -> Option<Ref<K,V,S>> where K: Borrow<Q>, Q: ?Sized + Hash + Eq {
        self.inner.counted.find(q).map(|slot| Ref { /* … */ })
    }
    fn contains_key<Q>(&self, q: &Q) -> bool where K: Borrow<Q>, Q: ?Sized + Hash + Eq {
        self.inner.counted.contains_key(q)
    }
    fn len(&self) -> usize { self.inner.counted.len() }
    fn is_empty(&self) -> bool { self.len() == 0 }
}

impl<K, V, S> Ref<K, V, S> {
    // Lifetimes are tied to both the ref and the map borrow.
    fn key<'a>(&'a self, map: &'a RcHashMap<K,V,S>) -> Result<&'a K, WrongMap> { /* owner check, then read */ }
    fn value<'a>(&'a self, map: &'a RcHashMap<K,V,S>) -> Result<&'a V, WrongMap> { /* owner check, then read */ }
    fn value_mut<'a>(&'a self, map: &'a mut RcHashMap<K,V,S>) -> Result<&'a mut V, WrongMap> { /* owner check, then write */ }
    fn drop(&mut self) {
        match unsafe { self.owner.as_ref() }.counted.put(self.slot) {
            PutResult::Live => {}
            PutResult::Removed { key, value } => {
                let inner = unsafe { self.owner.as_ref() };
                drop(key); drop(value);
                unsafe { Rc::decrement_strong_count(inner.raw_rc) };
            }
        }
    }
}
```

Testing plan
- HandleHashMap
  - Insert/find/remove sequences preserve index ↔ storage consistency; removal drops after consistency.
  - Reentrancy stress via Drop for K/V: ensure removal is index-first.
  - Collision paths: multiple keys with same hash verified by Eq checks.
  - `len`/`is_empty` track number of stored slots; consistent after each operation.
- CountedHashMap
  - `find` increments; `put` decrements; actual removal only at zero.
  - Overflow behavior for refcount increments is unchecked (UB), consistent with `Rc`.
  - value/value_mut observe the same slot identity across increments.
  - `len`/`is_empty` reflect the number of live entries (refcount >= 1); removal at zero decreases `len`.
- RcHashMap
  - `Ref::clone` increments per-entry count; `Ref::drop` decrements and removes at zero.
  - Rc-based keepalive: map drop with live entries leaves `Inner` alive via per-entry strong counts; final removal of last entry frees `Inner`.
  - Removal path drops `K`/`V` before decrementing the per-entry strong count.
- Owner identity and staleness: wrong-map `Ref` is rejected by accessors via owner-pointer check and returns `Err(WrongMap)`; `Eq`/`Hash` include `(owner_ptr, slot)`. Reference counting prevents stale `Ref`s: an entry cannot be physically removed (and its slot reused) while any `Ref` to it exists; we do not rely on SlotMap generations for `Ref` validity. Invariant: no external constructor for `Ref`; each `Ref` implies an associated per-entry strong count.
  - Unique keys enforced: `insert` fails on duplicate.
  - `len`/`is_empty` proxy to Module 2 and stay consistent across insert/get/put sequences.

Notes and non-goals
- Still single-threaded; no Send/Sync. Enforced via auto-trait negation using `PhantomData<*mut ()>` on `Ref` (and interior mutability in `Inner`).
- No weak handles (can be added later).
- No explicit `clear()`, `remove()` or `drain()` on RcHashMap; entries are removed only when the last `Ref` is dropped to preserve refcount semantics.
- Unique-keys policy enforced in Module 1: `insert` fails on duplicate; RcHashMap relies on this behavior.
- Not `Clone`: RcHashMap intentionally does not implement `Clone`. Refs are tied to a specific map instance.
- Key immutability: there is no `key_mut`; mutating a key would break the index’s invariants. `value_mut` cannot change `K`.

Overflow semantics
- We assume practical refcount overflow is unrealistic; overflow of reference counts is considered undefined behavior, consistent with `Rc` semantics. We document the assumption that there will be fewer than `usize::MAX` references to any entry. No runtime overflow checks are performed.

Hasher and rehashing invariants
- We precompute and store a `u64` hash per entry and always use the stored hash for indexing and rehash/growth; we do not call `K: Hash` after insertion. This avoids invoking user code during rehash.
