RcHashMap: Single-threaded, handle-based map with Rc-like references to entries that allow fast access and cleanup on drop

Summary
- Goal: Build RcHashMap in safe, verifiable layers so we can reason about each piece independently.
- Layers:
  - HandleHashMap<K, V, S>: A HashMap like datastructure that provides handles for quick access to internal entries without hashing. Returns a lightweight `Handle` wrapper internally backed by a `DefaultKey`.
  - CountedHashMap<K, V, S>: wraps HandleHashMap and adds per-entry refcounting (increments on get/clone, decrements on put).
  - RcHashMap<K, V, S>: wraps HandleHashMap and adds a `Ref` handle, which frees the reference when dropped.
- Constraints: single-threaded, no atomics, no per-entry heap allocations, stable generational keys, O(1) average lookups, unique keys.
  - Reentrancy: disallowed in HandleHashMap methods (these may observe transiently inconsistent internal state) and allowed elsewhere. Those methods only call user code via `K: Eq/Hash` during probing. After `remove()` returns `(K,V)` and the structure is consistent again, `Drop` for `K`/`V` may reenter the map. A debug-only guard enforces non-reentrancy inside HandleHashMap; upper layers do not use the guard.

Why this split?
- Localize invariants: each layer has a small, precise contract. We can test and audit them separately.
- Keep unsafe to a minimum: raw-pointer handling is isolated behind `RcCount<T>` used by RcHashMap; pure indexing logic is safe Rust.
- Clear failure boundaries: HandleHashMap never calls into user code once the data structure is in a consistent state.

Module 1: HandleHashMap
- How: Combine hashbrown::HashTable as an index with slotmap::SlotMap as storage. `Handle` is a small wrapper around the slotmap `DefaultKey` and provides efficient access to the entries.
- Entry<K, V>
  - key: K — user key; used for Eq/Hash only.
  - value: V — stored inline.
  - hash: u64 — precomputed with the map’s BuildHasher.
- API (sketch)
  - new(hasher: S) -> Self
  - find<Q>(&self, q: &Q) -> Option<Handle> where K: Borrow<Q>, Q: ?Sized + Hash + Eq
  - contains_key<Q>(&self, q: &Q) -> bool where K: Borrow<Q>, Q: ?Sized + Hash + Eq
  - insert(&mut self, key: K, value: V) -> Result<Handle, InsertError>
  - remove(&mut self, handle: Handle) -> Option<(K, V)>
  - len(&self) -> usize; is_empty(&self) -> bool
  - iter(&self) -> impl Iterator<Item = (Handle, &K, &V)>
  - iter_mut(&mut self) -> impl Iterator<Item = (Handle, &K, &mut V)>
  - `Handle` methods (tie lifetimes via a map borrow):
    - Handle::key<'a, K, V, S>(&'a self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a K>
    - Handle::value<'a, K, V, S>(&'a self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a V>
    - Handle::value_mut<'a, K, V, S>(&'a self, map: &'a mut HandleHashMap<K, V, S>) -> Option<&'a mut V>
- Behavior
  - Indexing: store only `DefaultKey` in RawTable, keyed by hash; resolve collisions with K: Eq against entries[slot].key. The public API returns a `Handle` that wraps this internal key.
  - Insertion (two-phase): compute hash(key), probe index (Eq) to reject duplicates, reserve capacity in index and storage; then commit by inserting into storage to obtain a slot and linking the slot into the index under the stored hash. On failure, roll back so the map remains unchanged.
  - Lookup: compute hash(key), probe index, compare by Eq. Returns a `Handle`.
  - Removal: remove from index first, then remove the handle's slot from entries and return (K, V).
  - Reentrancy guard (debug-only): public entry points begin with a guard `let _g = self.reentrancy.enter();`.
- Safety and consistency
  - remove() guarantees the data structure is consistent (index and storage no longer reference the handle’s slot) before dropping K and V.
  - All public methods leave the data structure in a consistent state before any user code can run, except K: Hash and K: Eq that are invoked during probing. Contract: reentrancy into the same map is disallowed from K: Hash and K: Eq.
  - No refcounting or keepalive here; purely structural.

Module 2: CountedHashMap
- Purpose: Add simple reference counting on top of HandleHashMap, enforced with linear Tokens carried by a lifetime-bound handle.
- Representation
  - Wrap values as Counted<V> = { refcount: UsizeCount, value: V }.
  - Internally: HandleHashMap<K, Counted<V>, S>.
- API (same surface, plus helpers)
  - find<Q>(&self, q: &Q) -> Option<CountedHandle<'static>> where K: Borrow<Q>, Q: ?Sized + Hash + Eq
    - If found, mints a token from the entry’s `UsizeCount` and returns a `CountedHandle<'static>` carrying that token. The handle stores the Module 1 `Handle`.
  - insert(&mut self, key: K, value: V) -> Result<CountedHandle<'static>, InsertError>
    - Delegates to HandleHashMap’s unique insertion. On success, initializes refcount by minting and returning a token in the resulting handle. On failure, no token is minted and the map is unchanged.
  - insert_with<F>(&mut self, key: K, default: F) -> Result<CountedHandle<'static>, InsertError> where F: FnOnce() -> V
    - Lazily constructs the value only when inserting a new key; does not run `default` on duplicates.
  - get(&self, handle: &CountedHandle<'_>) -> CountedHandle<'static>
    - Clones by minting another token from the same entry’s `UsizeCount`.
  - put(&mut self, handle: CountedHandle<'_>) -> PutResult
    - Consumes `handle`, returns its owned token via `UsizeCount::put(token)`; removes and returns `(K, V)` at zero, otherwise reports `Live`.
  - contains_key<Q>(&self, q: &Q) -> bool where K: Borrow<Q>, Q: ?Sized + Hash + Eq
    - Probes using the index without incrementing refcounts.
  - len(&self) -> usize; is_empty(&self) -> bool
  - iter(&self) -> impl Iterator<Item = (Handle, &K, &V)>
  - iter_mut(&mut self) -> impl Iterator<Item = (Handle, &K, &mut V)>
  - Internal helpers for scoped work: `iter_raw()` / `iter_mut_raw()` yield `CountedHandle`s alongside references; callers must return those handles via `put()`.
- Notes
  - Unique-key policy is enforced in Module 1 and reused here unchanged; refcounting is orthogonal.
  - All increments/decrements are interior-mutable and single-threaded.
  - The underlying HandleHashMap remains consistent at all times; drops of K and V happen only after the handle's slot is removed from both index and storage.


Module 3: RcHashMap
- Purpose: Public, ergonomic API with `Ref` handles, Rc-based keepalive, and owner identity checks. Internally holds `Rc<Inner>`.
- Keepalive model (via RcCount + Tokens)
  - `Inner` holds an `RcCount<Inner>` created from the map’s `Rc<Inner>` at construction time. It exposes `get()`/`put()` via the `Count` trait and returns linear Tokens (see tokens.md).
  - Each live entry contributes one additional strong count: on successful insertion, mint a token from `Inner.keepalive` and store it inside the stored value (Module 3 wraps the user value with a keepalive token). On final removal (when the entry’s `refcount` reaches 0 and the slot is removed), move this token out and return it to `keepalive`.
  - Insert failure cleanup: if insertion fails (e.g., duplicate key), return the keepalive token before returning `Err`, so no extra strong counts leak.
  - Dropping RcHashMap drops its own `Rc` handle; if entries still exist, their per-entry keepalive tokens keep `Inner` alive until those entries are removed.
  - Final removal order and rationale: drop `K` and `V` first while `Inner` remains alive (kept by the per-entry keepalive token), then return the token via `keepalive.put(token)`. This preserves safety if user `Drop` for `K`/`V` reenters the map.
  - Length as an invariant aid: all three layers expose `len()`/`is_empty()`. Although Rc-based keepalive does not require checking `len()==0` to trigger deallocation, `len()==0` on `CountedHashMap` precisely indicates “no live entries remain”, which is useful for assertions and tests around last-entry removal.
  - Refcount rationale (conceptual model → efficient implementation):
    - Conceptually, every `Ref` keeps two things alive: (1) its specific entry `(K,V)` and (2) the owning map. If implemented literally with `Rc<(K,V)>` stored per entry and an `Rc<Inner>` inside every `Ref`, this would add heap and pointer overhead to every entry and every `Ref` clone.
    - Instead, we implement the same semantics at lower cost:
      - Entry liveness is tracked by a per-entry counter (`UsizeCount`) in `CountedHashMap`. Cloning/dropping `Ref` only touches this counter.
      - Map keepalive is centralized: each live entry’s stored value holds exactly one token minted from `Inner.keepalive` for the entire duration the entry is live (while its per-entry refcount is ≥ 1). We mint this token once on insertion (transition to live), and return it once at final removal (transition to dead). Cloning additional `Ref`s to the same entry does not touch the map’s strong count.
      - We avoid storing an `Rc` inside each entry; instead, the stored value carries a zero-sized token tied to `Inner.keepalive`.
    - The net effect matches the intuitive model: a `Ref` prevents its entry from being removed, and as long as any entry is live, the map’s `Inner` remains alive. We achieve this without embedding `Rc` in each entry or per-`Ref` heap work.
- Unique keys policy
  - RcHashMap enforces unique keys by delegating to Module 1’s unique insertion. `insert` fails if the key already exists (no modification).
- Ref
  - Fields: `{ ch: CountedHandle<'_>, owner: NonNull<Inner<…>>, _nosend: PhantomData<*mut ()> }`.
  - Clone: `impl Clone for Ref` increments per-entry count by minting another entry token (via Module 2 `get`).
  - Drop: decrements per-entry count via `put`; if it reaches 0, performs physical removal. Removal path drops `K`/user `V` first; then returns the keepalive token stored in the value to `inner.keepalive`.
  - Hash/Eq: `(owner_ptr, handle)`.
  - Accessors
  - `find<Q>(&self, key: &Q) -> Option<Ref>` where `K: Borrow<Q>, Q: Hash + Eq`: delegates to `counted.find(key)` which mints an entry token upon success.
  - `insert(&mut self, key: K, value: V) -> Result<Ref, InsertError>`: on success, mints a keepalive token and wraps the user value; then returns a `Ref` (unique keys enforced in Module 1).
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
    - `iter(&self) -> impl Iterator<Item = Ref<'_, K, V, S>>`
      - Returns a `Ref` per entry. Access key/value via the regular `Ref` accessors which require a shared `&RcHashMap` borrow. This avoids yielding naked references whose lifetimes could outlive the refcount guard.
    - `iter_mut(&mut self) -> impl Iterator<Item = ItemGuardMut<'_, K, V, S>>`
      - Yields an RAII guard that holds the `Ref` plus `&K` and `&mut V` for the entry. Because `iter_mut` holds `&mut self`, the map cannot be shared to call `Ref` accessors; the guard provides direct access instead.
      - On `Drop`, the guard returns its per-entry token to decrement the refcount, mirroring `CountedHashMap`’s guard semantics.

Correctness and footguns
- The only user code that can run while the structure is not yet consistent is `K: Hash` and `K: Eq` during probing. These must not reenter the map or cause observable aliasing.
- Final removal order: remove from index → remove from storage and obtain `(K, V)` → drop `K` and user `V` → then move the keepalive token out of the stored value and return it to `inner.keepalive` to decrement the per-entry Rc strong count. See rationale under Keepalive model.
- Because removal happens only on the last handle for that entry, no other `Ref` to the entry exists. If other entries still have `Ref`s, their per-entry strong counts keep `Inner` alive across this decrement.
- Single-threaded only; both `RcHashMap` and `Ref` are `!Send + !Sync` (enforced with `PhantomData<*mut ()>` on `Ref`).

Code sketch
```rust
// HandleHashMap
struct HandleHashMap<K, V, S> { /* entries: SlotMap<DefaultKey, Entry<K,V>>, index: RawTable<DefaultKey>, hasher: S */ }
struct Handle(DefaultKey);
impl<K: Eq + Hash, V, S: BuildHasher> HandleHashMap<K, V, S> {
    fn find<Q: ?Sized + Hash + Eq>(&self, q: &Q) -> Option<Handle> where K: Borrow<Q> { /* probe, wrap DefaultKey */ }
    fn insert(&mut self, k: K, v: V) -> Result<Handle, InsertError> { /* two-phase commit with rollback on failure; wrap DefaultKey */ }
    fn remove(&mut self, h: Handle) -> Option<(K,V)> { /* index first, then entries */ }
    fn len(&self) -> usize { /* number of stored entries */ }
    fn is_empty(&self) -> bool { self.len() == 0 }
}
impl Handle {
    fn key<'a, K, V, S>(&'a self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a K> { /* read */ }
    fn value<'a, K, V, S>(&'a self, map: &'a HandleHashMap<K, V, S>) -> Option<&'a V> { /* read */ }
    fn value_mut<'a, K, V, S>(&'a self, map: &'a mut HandleHashMap<K, V, S>) -> Option<&'a mut V> { /* write */ }
}

// CountedHashMap
struct CountedHashMap<K, V, S>(HandleHashMap<K, Counted<V>, S>);
struct Counted<V> { refcount: UsizeCount, value: V }

/// Lifetime-bound counted handle carrying a linear token tied to the entry counter.
/// Uses Module 1's `Handle` abstraction instead of raw keys and does not store the counter by reference.
struct CountedHandle<'a> {
    handle: Handle,
    token: Token<'a, UsizeCount>,
}

impl<K: Eq + Hash, V, S: BuildHasher> CountedHashMap<K, V, S> {
    fn find<Q: ?Sized + Hash + Eq>(&self, q: &Q) -> Option<CountedHandle<'static>> where K: Borrow<Q> { /* mint token on hit */ }
    fn insert(&mut self, k: K, v: V) -> Result<CountedHandle<'static>, InsertError> { /* init with token; fail on dup */ }
    fn get(&self, h: &CountedHandle<'_>) -> CountedHandle<'static> { /* mint another token for cloning using h.handle */ }
    fn put(&self, h: CountedHandle<'_>) -> PutResult { /* consume token; via h.handle access entry counter; remove and return K,V at zero */ }
    fn len(&self) -> usize { /* number of stored/live entries (refcount >= 1) */ }
    fn is_empty(&self) -> bool { self.len() == 0 }
}

// RcHashMap (public)
struct RcHashMap<K, V, S> {
    inner: std::rc::Rc<Inner<K,V,S>>,
}
struct RcVal<'m, K, V, S> {
    value: V,
    // Keep `Inner` alive while this entry exists (Module 3)
    owner_token: core::mem::ManuallyDrop<Token<'m, RcCount<Inner<K,V,S>>>>,
}
struct Inner<K, V, S> {
    counted: CountedHashMap<K, RcVal<'static, K, V, S>, S>, // lifetime elided in sketch
    keepalive: RcCount<Inner<K,V,S>>,
    // !Send + !Sync markers
}
struct Ref<'a, K, V, S> {
    ch: CountedHandle<'a>,
    owner: NonNull<Inner<K,V,S>>,
    _nosend: PhantomData<*mut ()>,
}

impl<K, V, S> RcHashMap<K, V, S> {
    fn insert(&mut self, k: K, v: V) -> Result<Ref<'_,K,V,S>, InsertError> {
        // Mint keepalive token and wrap the user value with it.
        let t = self.inner.keepalive.get();
        let mut wrapped = RcVal { value: v, owner_token: core::mem::ManuallyDrop::new(t) };
        // Try to insert wrapped value; on failure, return the keepalive token before bubbling the error.
        match self.inner.counted.insert(k, wrapped) {
            Ok(ch) => Ok(Ref { ch, owner: /* NonNull::from(self.inner.as_ref()) */, _nosend: PhantomData }),
            Err(e) => {
                // Take back and return the keepalive token to avoid leaking a strong count.
                let tok = unsafe { core::mem::ManuallyDrop::take(&mut wrapped.owner_token) };
                let _ = self.inner.keepalive.put(tok);
                Err(e)
            }
        }
    }
    fn find<Q>(&self, q: &Q) -> Option<Ref<'_,K,V,S>> where K: Borrow<Q>, Q: ?Sized + Hash + Eq {
        self.inner.counted.find(q).map(|ch| Ref { ch, owner: /* NonNull::from(self.inner.as_ref()) */, _nosend: PhantomData })
    }
    fn contains_key<Q>(&self, q: &Q) -> bool where K: Borrow<Q>, Q: ?Sized + Hash + Eq {
        self.inner.counted.contains_key(q)
    }
    fn len(&self) -> usize { self.inner.counted.len() }
    fn is_empty(&self) -> bool { self.len() == 0 }
}

impl<'a, K, V, S> Ref<'a, K, V, S> {
    // Lifetimes are tied to both the ref and the map borrow.
    fn key<'a>(&'a self, map: &'a RcHashMap<K,V,S>) -> Result<&'a K, WrongMap> { /* owner check, then read */ }
    fn value<'a>(&'a self, map: &'a RcHashMap<K,V,S>) -> Result<&'a V, WrongMap> { /* owner check, then read */ }
    fn value_mut<'a>(&'a self, map: &'a mut RcHashMap<K,V,S>) -> Result<&'a mut V, WrongMap> { /* owner check, then write */ }
    fn drop(&mut self) {
        let inner = unsafe { self.owner.as_ref() };
        match inner.counted.put(/* self.ch */) {
            PutResult::Live => {}
            PutResult::Removed { key, val } => {
                // Drop user data first while keepalive still holds Inner alive
                drop(key);
                let mut wrapped = val;
                drop(wrapped.value);
                // Final removal: move keepalive token out and return it.
                let token = unsafe { core::mem::ManuallyDrop::take(&mut wrapped.owner_token) };
                let _ = inner.keepalive.put(token);
            }
        }
    }
}
```

Value keepalive via Tokens (Module 3)
- Each stored value (wrapped in `RcVal`) holds a token minted from `Inner.keepalive` for as long as the entry is live. This keeps the `Inner` allocation alive across map drops and until the last entry is removed.
- On final removal, after unlinking and dropping `K` and the user’s `V`, RcHashMap moves the token out (stored in `ManuallyDrop`) and returns it to `keepalive`.
- Exact final-removal order: (1) decrement entry refcount; (2) if non-zero, return; (3) unlink and remove storage slot; (4) drop `K`; (5) drop user `V` while the keepalive token is still held; (6) move the keepalive token out and return it to `keepalive`; (7) return without touching `Inner` again.

Interior mutability and reentrancy
- HandleHashMap employs an internal exclusive-access discipline and a debug-only reentrancy guard at the start of each method to prevent nested entry while its internal state can be transiently inconsistent. These methods only invoke user code via `K: Eq/Hash` during probing.
- Upper layers (CountedHashMap, RcHashMap) do not use the guard. They rely on HandleHashMap to provide consistent operations. After `HandleHashMap::remove` returns `(K,V)`, the structure is consistent again; `Drop` for `K`/`V` may reenter the map safely.

Testing plan
- HandleHashMap
  - Insert/find/remove sequences preserve index ↔ storage consistency; removal drops after consistency.
  - Reentrancy stress via Drop for K/V: ensure removal is index-first.
  - Collision paths: multiple keys with same hash verified by Eq checks.
  - `len`/`is_empty` track number of stored slots; consistent after each operation.
- CountedHashMap
  - `find` increments; `put` decrements; actual removal only at zero.
  - Overflow behavior for refcount increments is unchecked (UB), consistent with `Rc`.
  - value/value_mut observe the same handle identity across increments.
  - `len`/`is_empty` reflect the number of live entries (refcount >= 1); removal at zero decreases `len`.
  - Duplicate insert returns `Err` and does not mint a token; refcounts and `len` remain unchanged.
- RcHashMap
  - `Ref::clone` increments per-entry count; `Ref::drop` decrements and removes at zero.
  - Rc-based keepalive via tokens: map drop with live entries leaves `Inner` alive via per-entry keepalive tokens stored in values; final removal of last entry frees `Inner` when the value’s token is returned.
  - Removal path drops `K`/user `V` before returning the keepalive token to `inner.keepalive`.
  - Duplicate insert returns `Err` and returns the keepalive token before erroring; `Rc<Inner>` strong count remains correct.
  - Owner identity and staleness: wrong-map `Ref` is rejected by accessors via owner-pointer check and returns `Err(WrongMap)`; `Eq`/`Hash` include `(owner_ptr, handle)`. Reference counting prevents stale `Ref`s: an entry cannot be physically removed (and its slot reused) while any `Ref` to it exists; we do not rely on SlotMap generations for `Ref` validity. Invariant: no external constructor for `Ref`; each `Ref` implies an associated per-entry strong count.
  - Unique keys enforced: `insert` fails on duplicate.
  - `len`/`is_empty` proxy to Module 2 and stay consistent across insert/get/put sequences.
  - Reentrancy guard: in debug builds, nested entry during critical sections panics. Add tests that attempt nested `insert/find` from within `Eq` (guarded) and assert the guard triggers; and tests that reenter from `Drop` of `K`/`V` after unlink (unguarded) and assert no panic occurs. In release builds, the guard is compiled out and has zero overhead.

Notes and non-goals
- Still single-threaded; no Send/Sync. Enforced via auto-trait negation using `PhantomData<*mut ()>` on `Ref` (and interior mutability in `Inner`).
- No weak handles (can be added later).
- No explicit `clear()`, `remove()` or `drain()` on RcHashMap; entries are removed only when the last `Ref` is dropped to preserve refcount semantics.
- Unique-keys policy enforced in Module 1: `insert` fails on duplicate; RcHashMap relies on this behavior.
- Not `Clone`: RcHashMap intentionally does not implement `Clone`. Refs are tied to a specific map instance.
- Key immutability: there is no `key_mut`; mutating a key would break the index’s invariants. `value_mut` cannot change `K`.
 - Public visibility: Only `RcHashMap` and its `Ref` are public. `HandleHashMap` and `CountedHashMap` are implementation details and not part of the public API.

Overflow semantics
- We assume practical refcount overflow is unrealistic; overflow of reference counts is considered undefined behavior, consistent with `Rc` semantics. We document the assumption that there will be fewer than `usize::MAX` references to any entry. No runtime overflow checks are performed.

Hasher and rehashing invariants
- We precompute and store a `u64` hash per entry and always use the stored hash for indexing and rehash/growth; we do not call `K: Hash` after insertion. This avoids invoking user code during rehash.
