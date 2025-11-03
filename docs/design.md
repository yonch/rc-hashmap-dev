RcHashMap: Single-threaded, refcounted map with SlotMap-backed storage

Overview
- Goal: A HashMap-like structure storing reference-counted key→value pairs. Clients receive Rc-like handles (Ref) that provide identity and lifetime management; all value access happens only through RcHashMap methods to preserve exclusivity and safety.
- Design: Store Entry<K,V> inline inside a SlotMap for stable, generational keys; maintain a separate raw index keyed by a precomputed u64 hash that maps to SlotMap slots. A Ref identifies entries by the SlotMap key. All value access uses RcHashMap::{value, value_mut}, which return references whose lifetimes are tied to both the map borrow and the provided Ref. This prevents a Ref from being dropped while its value reference is alive and ensures inserts and other &mut operations cannot run concurrently.
- Lifecycle model: Each key has two stable, user-visible states — indexed+live (present in index and entries while at least one Ref exists) and gone (removed from index and entries once the last Ref is dropped). There is no public removal API; removal happens only when the final Ref is dropped.
- Keepalive-on-drop: The map owns a `Box<Inner>`. If the map is dropped while Refs exist, `Inner` is put into a keepalive state via `Box::into_raw`. The last `Ref` drop removes its entry (if any) and, if no entries remain, reconstructs the box and drops it to free `Inner`.
- Constraints: Single-threaded; no atomics; no per-entry heap allocations; no global registries. Ref hash/eq/drop are O(1); ref creation/cloning is O(1) and fallible to uphold no-panic and no-overflow guarantees.

Prior Art
- The internment crate’s ArcIntern (see /workspaces/internment/src/arc.rs) demonstrates counted headers with pointer-like handles. We adapt that pattern to a per-instance, single-threaded structure with SlotMap-backed storage and no per-entry allocation.

Non-Goals
- Thread-safe operation or cross-thread sharing.
- Weak handles (can be added later).
- Long-lived references to values without holding a read guard.
- Value iteration that yields `&V` directly (would require additional read guards). We instead provide a live-only iterator that yields `Ref`s.

Data Model
- Entry<K, V>
  - key: K — logical key; Eq + Hash.
  - value: V — stored inline.
  - refcount: Cell<usize> — number of live Ref handles for this entry.
  - hash: u64 — precomputed hash of `key` using the map’s BuildHasher.
- Containers inside RcHashMap<K, V, S = RandomState>
  - inner: Box<Inner<K, V, S>> — owning handle of map state. Refs capture a raw pointer to this Inner at creation time. RcHashMap is not Clone by design.
- Inner<K, V, S>
  - entries: UnsafeCell<slotmap::SlotMap<DefaultKey, Entry<K, V>>> — interior-mutably accessed by `Ref::drop` for removal.
  - index: UnsafeCell<hashbrown::raw::RawTable<DefaultKey>> — interior-mutably accessed by `Ref::drop` for index removal.
  - hasher: S — BuildHasher used for all key hashing.
  - keepalive_raw: Cell<Option<NonNull<Inner<K, V, S>>>> — populated when RcHashMap is dropped while Refs exist; reclaimed exactly once by the last Ref after it removes its slot and observes the map is empty.

Handles
- Ref<K, V, S>
  - slot: DefaultKey — identifies the Entry in the SlotMap.
  - owner: NonNull<Inner<K, V, S>> — raw pointer to map state for O(1) handle ops without borrowing the map or bumping an Rc.
  - marker: PhantomData<Rc<()>> — forces `!Send + !Sync` and single-threaded semantics.
  - No Clone: Ref does not implement `Clone`. Use `try_clone() -> Option<Ref>` which increments the entry refcount and returns `None` if cloning would overflow the refcount. Underflow is impossible by invariant (there is exactly one `Ref` per refcount unit).
  - Drop: decrements; when zero, performs removal in a reentrancy-safe order and may reclaim keepalive exactly once (details in Drop and Keepalive).
  - Does not implement Deref. All value access goes through RcHashMap::{value, value_mut}.
  - Does not implement Copy. `try_clone()` increments the entry’s refcount when it succeeds.
  - Owner pointer acquisition: computed once at Ref creation via `NonNull::from(self.inner.as_ref())`.
  - Key accessor: keys are accessed with a map borrow to tie the lifetime to the map, preventing concurrent inserts that might reallocate SlotMap storage. Provide `RcHashMap::key<'a>(&'a self, r: &'a Ref<K, V, S>) -> Option<&'a K>` (or equivalently `Ref::key(&self, map: &RcHashMap<…>) -> Option<&K>`).

Hashing and Equality
- Entry<K,V>: Hash/Eq delegate to `key`.
- Ref: Hash and Eq use `(owner_ptr, slot)` to avoid cross-map collisions, where `owner_ptr` is the `NonNull<Inner>` address.

Index Operations (RawTable)
- Insert: compute u64 hash with the map’s hasher; insert the slot into `index` using that hash. Store the same hash in Entry.
- Get: compute u64 hash; probe `index` for that hash and linearly check candidate slots’ `entries[slot].key == query_key`. Returning a `Ref` is fallible: `None` indicates not found or refcount would overflow if another `Ref` were created.
- Get-or-insert: `get_or_insert[_with]` returns `Option<Ref>`; on hit it attempts to increment the refcount, returning `None` only if doing so would overflow, and on miss it inserts a new entry with `refcount == 1` and returns `Some(Ref)`.
- Removal: There is no public removal API. When an entry’s refcount reaches 0 in `Ref::drop`, remove exactly the recorded `(hash, slot)` pair from the raw index (using the precomputed hash in Entry), then remove the SlotMap entry in a way that allows deferring user destructor runs until after potential keepalive reclamation (see Drop and Keepalive).

Drop and Keepalive
- Dropping RcHashMap:
  - If `entries` is empty, drop `inner` normally.
  - Otherwise (by invariant, entries are present only while some Ref exists), move `inner` into a raw pointer with `Box::into_raw` and store it in `keepalive_raw`. The map value is now logically dropped; only `Refs` keep `Inner` alive.
- Dropping Ref:
  - On last drop of an entry (`refcount` becomes 0), perform removal and potential keepalive reclamation before running user destructors.
  - Order:
    1) Remove from the index first (preventing reentrant lookups from finding the key during `K`/`V` drops).
    2) Remove from the SlotMap, taking ownership of the `Entry { key, value, … }` by value without dropping yet.
    3) If `keepalive_raw.is_some()` and after this removal the SlotMap is empty, reclaim `keepalive_raw` exactly once using `take()` + `Box::from_raw` to free `Inner`.
    4) Finally, drop the owned `Entry` (which runs `K`/`V` destructors).
  - The address of `Inner` is stable for the entire lifetime of the map and during keepalive: `Inner` is boxed once and never moved, memswapped, or relocated after any `Ref` has been created.

Safety and Invariants
- Single-threaded: `RcHashMap<K, V, S>` and `Ref<K, V, S>` are not Send nor Sync (PhantomData<Rc<()>>).
 - Access discipline: `Ref` never dereferences to `V`. All `&V`/`&mut V` are obtained exclusively via `RcHashMap::{value, value_mut}`, tying lifetimes to both the map borrow and the provided `&Ref`. The returned reference cannot outlive the `Ref` used to access it.
 - Owner validation: All APIs that accept a `Ref` validate map identity by pointer equality. On mismatch, they return `None` (never panic).
- Liveness: An entry is live while `refcount > 0`. When `refcount` reaches 0 in `Ref::drop`, the entry is removed from the index (always present) and from the SlotMap immediately.
- No zero-ref entries: By construction, all entries in `entries` have `refcount > 0`. Creation APIs return a `Ref`; removal removes the SlotMap entry eagerly if no Refs exist. Therefore, if `entries` is non-empty, at least one live `Ref` exists.
 - Stable storage: SlotMap removals never relocate other entries, and `entries` is not shrunk while any `&V` exists. References produced by `value`/`value_mut` remain valid while their borrowed `Ref` is alive, even if other entries are removed.
 - Access while dropping other Refs: `value`/`value_mut` borrow a `&Ref` to the accessed slot; that `Ref` cannot be dropped while the returned reference is alive. Dropping other Refs may proceed; their removals cannot invalidate the borrowed reference due to SlotMap’s non-relocating removal.
 - Handle liveness: Refs are never stale. The keepalive mechanism guarantees that if a `Ref` exists, its `owner` pointer is valid and points to the same `Inner` it was created from. APIs return `None` only for wrong-owner mismatches.
- Generational keys: ABA on slots is prevented by SlotMap’s generational keys; stale Refs never alias new entries.
- Refcount bounds and no-panics policy: The code never panics. Refcount underflow is impossible by invariant. Any operation that would overflow a refcount fails and returns `None` instead of panicking.
- Keepalive: `keepalive_raw` is set only during `RcHashMap::drop`. Reclamation is guarded by a single successful `take()`; the last `Ref` drops `Inner` only after removing its own slot and observing the SlotMap is empty.
- Interior mutability and reentrancy: `entries` and `index` are behind `UnsafeCell` to allow `Ref::drop` to remove the last entry without `&mut RcHashMap`. During `Ref::drop`, removal occurs in the order index → entries (capturing by value) → maybe reclaim keepalive → drop Entry. Reentrant lookups may observe the key as present (if they run before index removal) or absent (if they run after); they never observe an index entry pointing to a missing slot under this order.

Managing `entries` References
- While any `&V` obtained via `value`/`value_mut` or any `&K` obtained via `key()` exists, no operation may reallocate or shrink `entries`.
- Only `Ref::drop` may mutate `entries` while an `&V` exists, and it may only remove other entries. Such removals never relocate unrelated entries.
- APIs such as `shrink_to_fit` are not provided; any future APIs must uphold the above invariants.

Trait Bounds
- K: Eq + Hash + Borrow<Q> for lookups; no 'static bound required.
- V: no bounds.
- S: BuildHasher + Default (RandomState default).

Type Sketch
```rust
use core::{cell::{Cell, UnsafeCell}, hash::Hash, borrow::Borrow, ptr::NonNull, marker::PhantomData};
use std::rc::Rc;
use hashbrown::raw::RawTable;
use slotmap::{SlotMap, DefaultKey};

struct Entry<K, V> {
    key: K,
    value: V,
    refcount: Cell<usize>,
    hash: u64,
}

struct Inner<K, V, S> {
    entries: UnsafeCell<SlotMap<DefaultKey, Entry<K, V>>>,
    index: UnsafeCell<RawTable<DefaultKey>>,
    hasher: S,
    keepalive_raw: Cell<Option<NonNull<Inner<K, V, S>>>>,
}

pub struct RcHashMap<K, V, S = RandomState> {
    inner: Box<Inner<K, V, S>>,
    _nosend: PhantomData<Rc<()>>,
}

pub struct Ref<K, V, S = RandomState> {
    slot: DefaultKey,
    owner: NonNull<Inner<K, V, S>>,
    _nosend: PhantomData<Rc<()>>,
}

impl<K, V, S> RcHashMap<K, V, S> {
    pub fn key<'a>(&'a self, r: &'a Ref<K, V, S>) -> Option<&'a K> {
        // Fail if the handle belongs to a different map
        let owner_ptr = NonNull::from(self.inner.as_ref());
        if !core::ptr::eq(owner_ptr.as_ptr(), r.owner.as_ptr()) { return None }
        let inner: &Inner<K, V, S> = &self.inner;
        // SAFETY: read-only access through UnsafeCell; borrowing &self prevents &mut operations like insert
        let entries: &SlotMap<DefaultKey, Entry<K, V>> = unsafe { &*inner.entries.get() };
        let e = entries.get(r.slot)?;
        Some(&e.key)
    }
    pub fn value<'a>(&'a self, r: &'a Ref<K, V, S>) -> Option<&'a V> {
        // Fail if the handle belongs to a different map
        let owner_ptr = NonNull::from(self.inner.as_ref());
        if !core::ptr::eq(owner_ptr.as_ptr(), r.owner.as_ptr()) { return None }
        let inner: &Inner<K, V, S> = &self.inner;
        // SAFETY: read-only access through UnsafeCell
        let entries: &SlotMap<DefaultKey, Entry<K, V>> = unsafe { &*inner.entries.get() };
        let e = entries.get(r.slot)?;
        Some(&e.value)
    }

    pub fn value_mut<'a>(&'a mut self, r: &'a Ref<K, V, S>) -> Option<&'a mut V> {
        let owner_ptr = NonNull::from(self.inner.as_ref());
        if !core::ptr::eq(owner_ptr.as_ptr(), r.owner.as_ptr()) { return None }
        let inner_mut: &mut Inner<K, V, S> = &mut self.inner;
        // SAFETY: exclusive borrow of RcHashMap gives us exclusive access to entries
        let entries: &mut SlotMap<DefaultKey, Entry<K, V>> = unsafe { &mut *inner_mut.entries.get() };
        let e = entries.get_mut(r.slot)?;
        Some(&mut e.value)
    }
}
```

Iteration
- Provide a live-only iterator over current entries that yields fallible `Ref`s, e.g. `iter_refs(&self) -> impl Iterator<Item = Option<Ref<K,V,S>>>`.
- Each item attempts to create a `Ref` for that slot; it yields `None` only if creating a new `Ref` would overflow the refcount for that entry.
- Iteration yields `Ref`s and, if callers need keys/values, they can use `RcHashMap::key(&ref)` and `RcHashMap::value(&ref)` per item.

Example Usage
```rust
let mut map: RcHashMap<String, Vec<u8>> = RcHashMap::new();

let a1 = map.get_or_insert_with("alpha".to_string(), || vec![1,2,3]).unwrap();
let a2 = map.get("alpha").unwrap();

// Hash Ref cheaply by (map id, slot)
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
let mut h1 = DefaultHasher::new(); a1.hash(&mut h1);
let mut h2 = DefaultHasher::new(); a2.hash(&mut h2);
assert_eq!(h1.finish(), h2.finish());

// Read via RcHashMap::value (fallible owner check)
let len = map.value(&a1).unwrap().len();

// Mutate via exclusive access to the map
{
    let vmut: &mut Vec<u8> = map.value_mut(&a2).unwrap();
    vmut.push(4);
}

drop(a1); drop(a2); // last drop removes index + SlotMap entry
assert!(!map.contains_key("alpha"));
```

Testing Plan
- Lifecycle: insert → try_clone Ref → drop clones → ensure last drop removes from index and SlotMap immediately.
- Hash/Eq correctness: Ref equality and hashing include map identity and slot.
- Access safety: `value`/`value_mut` tie returned reference lifetimes to both the map and the `Ref`; verify that references remain valid while their `Ref` is alive, despite removals of other entries.
- Identity safety: APIs that take a Ref verify map identity and return None on mismatch.
- Liveness invariant: while any `Ref` to an entry exists, `value`/`value_mut` return references to that entry.
- Reentrancy stress: make `V::drop` perform lookups/inserts and drop other `Ref`s; assert that the removing key is absent after index removal, keepalive is reclaimed exactly once, and no use-after-free occurs.
- Aliasing safety: hold an `&V` from `value`, drop many other `Ref`s; assert continued validity of the borrowed `&V` and absence of relocations.
- Pointer reuse: drop a map to reclaim keepalive, create a new map, and ensure wrong-owner handles are rejected. Refs are never stale by invariant.
- Refcount limits: fuzz clone/drop sequences near the refcount limit; assert no panics and that operations that would overflow instead return `None`.

Rationale Recap
- SlotMap-backed entries avoid per-entry heap allocations and provide stable, generational keys.
- RawTable-based index keyed by precomputed u64 avoids duplicating K and keeps lookups O(1) average while tolerating collisions via K: Eq checks.
- Decoupling handles from borrowing the map removes the &self lifetime coupling while keeping all value access centralized in RcHashMap via value/value_mut, with returned lifetimes tied to both map and Ref for safety.
- Immediate reclamation on last-drop is safe because SlotMap does not relocate other entries on removal, and value/value_mut tie reference lifetimes to a live `Ref`. Reentrancy is handled by removing from the index first, then capturing the entry by value, possibly reclaiming keepalive, and only then running user destructors.

Convenience Types
- Provide a type alias for the default hasher case: `type DefaultRef<K, V> = Ref<K, V, RandomState>`.
