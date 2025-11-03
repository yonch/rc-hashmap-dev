RcHashMap: Single-threaded, refcounted map layered over an IndexedSlotMap

Summary
- Goal: Build RcHashMap in safe, verifiable layers so we can reason about each piece independently.
- Layers:
  - IndexedSlotMap<K, V, S>: raw index + slot storage with no refcounting.
  - CountedIndexedSlotMap<K, V, S>: wraps IndexedSlotMap and adds per-entry refcounting.
  - RcHashMap<K, V, S>: public API with keepalive and Ref handles that delegate to CountedIndexedSlotMap.
- Constraints: single-threaded, no atomics, no per-entry heap allocations, stable generational keys, O(1) average lookups.

Why this split?
- Localize invariants: each layer has a small, precise contract. We can test and audit them separately.
- Minimize unsafe scope: pointer/keepalive trickery only exists in RcHashMap; pure indexing logic is safe Rust.
- Clear failure boundaries: IndexedSlotMap never calls into user code once the data structure is in a consistent state.

Module 1: IndexedSlotMap
- Purpose: Combine hashbrown::raw::RawTable as an index with slotmap::SlotMap as storage. No refcounts; duplicates allowed unless the caller enforces uniqueness.
- Entry<K, V>
  - key: K — user key; used for Eq/Hash only.
  - value: V — stored inline.
  - hash: u64 — precomputed with the map’s BuildHasher.
- API (sketch)
  - new(hasher: S) -> Self
  - find(&self, key: &K) -> Option<DefaultKey>
  - insert(&mut self, key: K, value: V) -> DefaultKey
  - key(&self, slot: DefaultKey) -> Option<&K>
  - value(&self, slot: DefaultKey) -> Option<&V>
  - value_mut(&mut self, slot: DefaultKey) -> Option<&mut V>
  - remove(&mut self, slot: DefaultKey) -> Option<(K, V)>
- Behavior
  - Indexing: store only DefaultKey in RawTable, keyed by hash; resolve collisions with K: Eq against entries[slot].key.
  - Insertion: compute hash(key), insert into entries first to obtain slot, then insert slot into index under that hash.
  - Lookup: compute hash(key), probe index, compare by Eq.
  - Removal: remove from index first, then remove the slot from entries and return (K, V).
- Safety and consistency
  - remove() guarantees the data structure is consistent (index and storage no longer reference the slot) before dropping K and V.
  - All public methods leave the data structure in a consistent state before any user code can run, except K: Hash and K: Eq that are invoked during probing. We document this as a footgun for higher layers: if Eq/Hash have side-effects, they must not reenter this map.
  - No refcounting or keepalive here; purely structural.

Module 2: CountedIndexedSlotMap
- Purpose: Add simple reference counting on top of IndexedSlotMap.
- Representation
  - Wrap values as Counted<V> = { refcount: Cell<usize>, value: V }.
  - Internally: IndexedSlotMap<K, Counted<V>, S>.
- API (same surface, plus helpers)
  - find(&mut self, key: &K) -> Option<DefaultKey>
    - If found, increments refcount and returns the slot.
  - insert(&mut self, key: K, value: V) -> DefaultKey
    - Inserts with refcount = 1 and returns the slot.
  - value(&self, slot: DefaultKey) -> Option<&V>
  - value_mut(&mut self, slot: DefaultKey) -> Option<&mut V>
  - key(&self, slot: DefaultKey) -> Option<&K>
  - remove(&mut self, slot: DefaultKey) -> bool
    - Decrements refcount; if it reaches 0, delegates to IndexedSlotMap::remove(slot) and returns false (slot no longer valid). Returns true if the entry remains live.
  - try_inc(&mut self, slot: DefaultKey) -> bool
    - Best-effort increment for handle cloning; returns false on overflow or invalid slot.
- Notes
  - “Every get() increments” means find() (keyed lookup) acquires a live count. Direct slot-based clones use try_inc().
  - The underlying IndexedSlotMap remains consistent at all times; drops of K and V happen only after the slot is removed from both index and storage.

Module 3: RcHashMap
- Purpose: Public, ergonomic API with Ref handles, keepalive-on-drop, and owner identity checks. Internally holds Inner { counted: CountedIndexedSlotMap<…>, … }.
- Keepalive model
  - If RcHashMap is dropped while any Ref exists, move Inner into a keepalive raw pointer (Box::into_raw). The last Ref::drop triggers decrements/removals and, after observing the map is empty, reconstructs the box and drops it to free memory.
- Ref handle
  - Fields: { slot: DefaultKey, owner: NonNull<Inner<…>>, !Send + !Sync }.
  - try_clone() -> Option<Ref>: uses try_inc(slot) via interior mutability; returns None on overflow.
  - Drop: calls counted.remove(slot); if the entry reaches 0, the underlying slot is physically removed.
  - Hash/Eq: (owner_ptr, slot).
- Accessors
  - value<'a>(&'a self, r: &'a Ref) -> Option<&'a V>
  - value_mut<'a>(&'a mut self, r: &'a Ref) -> Option<&'a mut V>
  - key<'a>(&'a self, r: &'a Ref) -> Option<&'a K>
  - These tie the returned reference lifetime to both the map borrow and the Ref, preventing concurrent mutations and ensuring the Ref cannot be dropped while a borrow is alive.
- Lookups
  - get(&mut self, key: &K) -> Option<Ref>: delegates to counted.find(key) which increments the refcount upon success.
  - insert(&mut self, key: K, value: V) -> Ref: delegates to counted.insert (starts at 1) and wraps the returned slot.

Correctness and footguns
- The only user code that can run while the structure is not yet consistent is K: Hash and K: Eq during probing. These must not reenter the map or cause observable aliasing; we document this caveat prominently for RcHashMap users.
- Removal order always ensures: remove from index → remove from storage → only then drop K and V.
- Single-threaded only; Ref makes this !Send + !Sync.

Code sketch
```rust
// IndexedSlotMap
struct IndexedSlotMap<K, V, S> { /* entries: SlotMap<DefaultKey, Entry<K,V>>, index: RawTable<DefaultKey>, hasher: S */ }
impl<K: Eq + Hash, V, S: BuildHasher> IndexedSlotMap<K, V, S> {
    fn find(&self, k: &K) -> Option<DefaultKey> { /* probe */ }
    fn insert(&mut self, k: K, v: V) -> DefaultKey { /* add to entries then index */ }
    fn value(&self, s: DefaultKey) -> Option<&V> { /* read */ }
    fn value_mut(&mut self, s: DefaultKey) -> Option<&mut V> { /* write */ }
    fn key(&self, s: DefaultKey) -> Option<&K> { /* read */ }
    fn remove(&mut self, s: DefaultKey) -> Option<(K,V)> { /* index first, then entries */ }
}

// CountedIndexedSlotMap
struct CountedIndexedSlotMap<K, V, S>(IndexedSlotMap<K, Counted<V>, S>);
struct Counted<V> { refcount: Cell<usize>, value: V }
impl<K: Eq + Hash, V, S: BuildHasher> CountedIndexedSlotMap<K, V, S> {
    fn find(&mut self, k: &K) -> Option<DefaultKey> { /* inc on hit */ }
    fn insert(&mut self, k: K, v: V) -> DefaultKey { /* refcount=1 */ }
    fn try_inc(&mut self, s: DefaultKey) -> bool { /* inc for cloning */ }
    fn remove(&mut self, s: DefaultKey) -> bool { /* dec; remove at zero */ }
    fn value(&self, s: DefaultKey) -> Option<&V> { /* read */ }
    fn value_mut(&mut self, s: DefaultKey) -> Option<&mut V> { /* write */ }
    fn key(&self, s: DefaultKey) -> Option<&K> { /* read */ }
}

// RcHashMap (public)
struct RcHashMap<K, V, S> { /* inner: Box<Inner> with keepalive */ }
struct Ref<K, V, S> { slot: DefaultKey, owner: NonNull<Inner<K,V,S>>, _nosend: PhantomData<*const ()> }
```

Testing plan
- IndexedSlotMap
  - Insert/find/remove sequences preserve index ↔ storage consistency; removal drops after consistency.
  - Reentrancy stress via Drop for K/V: ensure removal is index-first.
  - Collision paths: multiple keys with same hash verified by Eq checks.
- CountedIndexedSlotMap
  - find increments; remove decrements; actual removal only at zero.
  - Overflow behavior for refcount (find/try_inc returns None/false). No panics.
  - value/value_mut observe the same slot identity across increments.
- RcHashMap
  - Ref::try_clone uses try_inc; drop uses remove; last-drop triggers physical removal.
  - Keepalive correctness: drop map with live Refs; last Ref reclaims keepalive exactly once.
  - Owner identity: wrong-map Ref rejected by accessors; Hash/Eq include owner ptr.
  - Iterator yields Refs (or fallible options) without exposing &V directly.

Notes and non-goals
- Still single-threaded; no Send/Sync.
- No weak handles (can be added later).
- Deduplication policy is left to RcHashMap’s API; the lower layers allow duplicate keys by design.

