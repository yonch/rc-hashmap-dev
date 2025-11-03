RcHashMap: Single-threaded, refcounted map with SlotMap-backed storage

Overview
- Goal: A HashMap-like structure storing reference-counted key→value pairs. Clients receive Rc-like handles (Ref) that provide identity and lifetime management; all value access happens only through RcHashMap methods to preserve exclusivity and safety.
- Design: Store Entry<K,V> inline inside a SlotMap for stable, generational keys; maintain a separate raw index keyed by a precomputed u64 hash that maps to SlotMap slots. A Ref identifies entries by the SlotMap key. All value access uses RcHashMap::{access, access_mut} so that inserts and other &mut operations cannot run while a user holds a reference to V.
- Keepalive-on-drop: The map owns a `Box<Inner>`. If the map is dropped while Refs exist, `Inner` is put into a keepalive state via `Box::into_raw`. The last `Ref` drop removes its entry (if any) and, if no entries remain, reconstructs the box and drops it to free `Inner`.
- Constraints: Single-threaded; no atomics; no per-entry heap allocations; no global registries. Ref clone/hash/eq/drop are O(1).

Prior Art
- The internment crate’s ArcIntern (see /workspaces/internment/src/arc.rs) demonstrates counted headers with pointer-like handles. We adapt that pattern to a per-instance, single-threaded structure with SlotMap-backed storage and no per-entry allocation.

Non-Goals
- Thread-safe operation or cross-thread sharing.
- Weak handles (can be added later).
- Long-lived references to values without holding a read guard.
- Iteration APIs. Follow-up work will define live-only iteration over current entries.

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
  - Clone: increments Entry refcount via interior Cell using checked_add; panics on overflow.
  - Drop: decrements; when zero, removes the entry immediately from the raw index (if still present) and the SlotMap. If `keepalive_raw.is_some()` and the SlotMap is now empty, reclaim `keepalive_raw` exactly once via `take()` + `Box::from_raw` to free `Inner`.
  - Does not implement Deref. All value access goes through RcHashMap::{access, access_mut}.
  - Does not implement Copy. Cloning a Ref increments the entry’s refcount.
  - Owner pointer acquisition: computed once at Ref creation via `NonNull::from(self.inner.as_ref())`.

Hashing and Equality
- Entry<K,V>: Hash/Eq delegate to `key`.
- Ref: Hash and Eq use `(owner_ptr, slot)` to avoid cross-map collisions, where `owner_ptr` is the `NonNull<Inner>` address.

Index Operations (RawTable)
- Insert: compute u64 hash with the map’s hasher; insert the slot into `index` using that hash. Store the same hash in Entry.
- Get: compute u64 hash; probe `index` for that hash and linearly check candidate slots’ `entries[slot].key == query_key`.
- Remove: compute u64 hash; probe for candidates and remove the matching slot if keys are equal. This only removes the index entry. The SlotMap entry is removed automatically by `Ref::drop` when the last Ref goes away. If no Refs exist at removal time, the SlotMap entry may be removed eagerly.
>> Can use the u64 hash stored in Entry to avoid recomputing it during removal in `Ref::drop`. There is no way to remove only the index entry -- it is only removed during `Ref::drop` when the last Ref goes away.

Drop and Keepalive
- Dropping RcHashMap:
  - If `entries` is empty, drop `inner` normally.
  - Otherwise (live refs exist or entries are present), move `inner` into a raw pointer with `Box::into_raw` and store it in `keepalive_raw`. The map value is now logically dropped; only `Refs` keep `Inner` alive.
- Dropping Ref:
  - On last drop of an entry (`refcount` becomes 0), remove it immediately from the index (if still present) and from the SlotMap. If `keepalive_raw.is_some()` and the SlotMap becomes empty, take and drop the keepalive pointer exactly once with `Box::from_raw`.
  - Order: remove from index first (preventing reentrant lookups from finding the key during `K`/`V` drops), then remove from the SlotMap (which runs `K`/`V` destructors).
  - The address of `Inner` is stable for the life of the map and during keepalive; no moves after Refs are created.

Safety and Invariants
- Single-threaded: `RcHashMap<K, V, S>` and `Ref<K, V, S>` are not Send nor Sync (PhantomData<Rc<()>>).
- Access discipline: `Ref` never dereferences to `V`. All `&V`/`&mut V` are obtained exclusively via `RcHashMap::{access, access_mut}`, tying lifetimes to a map borrow.
- Owner validation: All APIs that accept a `Ref` validate map identity. On mismatch, they return `None` (never panic).
- Liveness: An entry is live while `refcount > 0`. When `refcount` reaches 0 in `Ref::drop`, the entry is removed from the index (if present) and from the SlotMap immediately.
- Stable storage: SlotMap does not shrink its slot storage on removals; removing one entry never relocates others. References produced by `access`/`access_mut` remain valid while their borrowed `Ref` is alive, even if other entries are removed.
- Access while dropping other Refs: `access`/`access_mut` borrow a `&Ref` to the accessed slot. That `Ref` cannot be dropped while the returned reference is alive, so its entry cannot be removed during that time. Dropping other Refs may proceed; their removals cannot invalidate the borrowed reference due to SlotMap’s non-shrinking storage.
- Stale handle behavior: `access`/`access_mut` return `None` if the SlotMap no longer contains the referenced key (generation mismatch) or if the handle belongs to a different map.
>> Note that generation mismatch cannot occur in the current design since the slot is removed only when the last Ref goes away. However, the code returns None on generation mismatch to handle all cases safely (although we know this case doesn't occur)
- Generational keys: ABA on slots is prevented by SlotMap’s generational keys; stale Refs never alias new entries.
>> The design currently does not allow stale Refs, but this is an added layer of safety.
- Refcount bounds: `Ref::clone` uses checked_add and panics on overflow; `Ref::drop` uses checked subtraction and debug-asserts underflow cannot occur.
- Keepalive: `keepalive_raw` is set only during `RcHashMap::drop`. Reclamation is guarded by a single successful `take()`; the last `Ref` drops `Inner` only after removing its own slot and observing the SlotMap is empty.
- Interior mutability: `entries` and `index` are behind `UnsafeCell` solely to support `Ref::drop` removing the last entry without an `&mut RcHashMap`. Mutations in `Ref::drop` are limited to removing the dropping entry and its index record.

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
    pub fn access<'a>(&'a self, r: &Ref<K, V, S>) -> Option<&'a V> {
        // Fail if the handle belongs to a different map
        let owner_ptr = NonNull::from(self.inner.as_ref());
        if !core::ptr::eq(owner_ptr.as_ptr(), r.owner.as_ptr()) { return None }
        let inner: &Inner<K, V, S> = &self.inner;
        // SAFETY: read-only access through UnsafeCell
        let entries: &SlotMap<DefaultKey, Entry<K, V>> = unsafe { &*inner.entries.get() };
        let e = entries.get(r.slot)?;
        Some(&e.value)
    }

    pub fn access_mut<'a>(&'a mut self, r: &Ref<K, V, S>) -> Option<&'a mut V> {
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

Example Usage
```rust
let mut map: RcHashMap<String, Vec<u8>> = RcHashMap::new();

let a1 = map.get_or_insert_with("alpha".to_string(), || vec![1,2,3]);
let a2 = map.get("alpha").unwrap();

// Hash Ref cheaply by (map id, slot)
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
let mut h1 = DefaultHasher::new(); a1.hash(&mut h1);
let mut h2 = DefaultHasher::new(); a2.hash(&mut h2);
assert_eq!(h1.finish(), h2.finish());

// Read via RcHashMap::access (fallible owner check)
let len = map.access(&a1).unwrap().len();

// Mutate via exclusive access to the map
{
    let vmut: &mut Vec<u8> = map.access_mut(&a2).unwrap();
    vmut.push(4);
}

drop(a1); drop(a2); // last drop removes index + SlotMap entry
assert!(!map.contains_key("alpha"));
```

Testing Plan
- Lifecycle: insert → clone Ref → drop clones → ensure last drop removes from index and SlotMap immediately.
- Hash/Eq correctness: Ref equality and hashing include map identity and slot.
- Access safety: `access`/`access_mut` enforce exclusivity via &self/&mut self; verify that references remain valid while their `Ref` is alive, despite removals of other entries.
- Identity safety: APIs that take a Ref verify map identity and return None on mismatch.
- Liveness invariant: while any `Ref` to an entry exists, `access`/`access_mut` return references to that entry.
- Stress: repeat insert/get/drop sequences; ensure len reaches 0 after last drop and no panics.

Rationale Recap
- SlotMap-backed entries avoid per-entry heap allocations and provide stable, generational keys.
- RawTable-based index keyed by precomputed u64 avoids duplicating K and keeps lookups O(1) average while tolerating collisions via K: Eq checks.
- Decoupling handles from borrowing the map removes the &self lifetime coupling while keeping all value access centralized in RcHashMap via access/access_mut.
- Immediate reclamation on last-drop is safe because SlotMap does not relocate other entries on removal, and access/access_mut tie reference lifetimes to a live `Ref`.
