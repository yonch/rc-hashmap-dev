RcHashMap Design Analysis

Scope
- Review of docs/design.md with focus on safety, unspecified behavior, and ergonomics.
- Concrete recommendations, including spec changes and API adjustments.

Key Strengths
- Clear single-threaded constraint and `!Send + !Sync` via `PhantomData<Rc<()>>` (docs/design.md:63).
- Stable keys and O(1) handle ops using SlotMap keys + RawTable index (docs/design.md:47-51, 82-113).
- Keepalive-on-drop model avoids per-entry `Arc` overhead and keeps handles O(1) (docs/design.md:53-60, 37-41).

Safety Risks (and fixes)
- Missing lifetime tie between returned references and the `Ref`
  - Issue: `access`/`access_mut` return references tied only to `&self`/`&mut self`, not to the lifetime of the `Ref` (docs/design.md:114-133). This contradicts the stated invariant that “That Ref cannot be dropped while the returned reference is alive” (docs/design.md:68).
  - Consequence: UB if the caller drops `r` immediately after calling `access(&r)` and then the last `Ref::drop` removes the entry while `&V` is still alive.
  - Fix: Require the same lifetime for both `self` and `r`:
    - `pub fn access<'a>(&'a self, r: &'a Ref<K, V, S>) -> Option<&'a V>`
    - `pub fn access_mut<'a>(&'a mut self, r: &'a Ref<K, V, S>) -> Option<&'a mut V>`
    - Document this invariant explicitly; update tests to assert that dropping `r` while holding `&V` does not compile.

- Interior mutability + aliasing during `Ref::drop`
  - Context: `entries` and `index` live under `UnsafeCell` (docs/design.md:27-30, 75). `access` creates a shared reference to the SlotMap (docs/design.md:121) and returns `&V`; other `Ref::drop`s can concurrently remove different entries via a mutable reference to the same SlotMap.
  - Risk: Creating both `&SlotMap` and `&mut SlotMap` to the same allocation while `&V` exists relies on unsafe aliasing discipline. This is only sound if removals of other entries never move or invalidate the memory of the borrowed value and we avoid reallocation.
  - Current assumption: “SlotMap does not shrink its slot storage on removals; removing one entry never relocates others” (docs/design.md:67). This should be treated as a hard dependency and stated explicitly in Safety & Invariants, including that ‘remove’ does not reallocate.
  - Mitigations:
    - Specify that no operation that could reallocate `entries` (e.g., insertion) may run while any `&V` exists; this already holds because insertion requires `&mut self` and `&mut self` cannot coexist with `&self` used to obtain `&V`.
    - Keep the “only `Ref::drop` may mutate while `&V` exists” invariant, and clarify that `Ref::drop` only removes other entries and that removal does not move memory.
    - Optional: avoid producing `&SlotMap` in `access` and instead derive the value reference from a raw pointer to the entry to minimize aliasing; this reduces the duration of a logical shared borrow of the whole SlotMap.

- Reentrancy during drops
  - Context: `Ref::drop` removes from the index first, then from `entries`, which runs `K`/`V` destructors (docs/design.md:59). `V::drop` is arbitrary user code and may call back into the same map.
  - Risks:
    - Invariant gaps while `Ref::drop` is mid-operation (index removed, entry still present, or vice‑versa if reversed) can surprise reentrant lookups/insertions.
    - Interleaving of `Ref::drop` with insert/remove (via reentrant calls from `V::drop`) could observe a transiently inconsistent `(index, entries)` pair.
  - Recommendations:
    - Specify reentrancy semantics explicitly: while dropping an entry, it is not visible in the index; lookups for that key must tolerate “index hit but stale slot” or “no index entry, slot still present until removal completes”. Implementations must handle both gracefully.
    - Keep the current order (remove index → remove entry) to avoid lookups finding the key during `K`/`V` drop, but document this as part of the observable behavior.
    - Consider a light “dropping flag” inside `Entry` to let lookups ignore an entry that is in-progress of being removed if you ever decide to remove from `entries` first. Not strictly required if you standardize the current order.

- Keepalive-on-drop and zero-ref entries
  - Issue: The map enters keepalive mode when `entries` is non-empty (docs/design.md:55-56), regardless of refcounts. If there exist entries with `refcount == 0` (e.g., future API that inserts without returning a `Ref`, or a key removed from the index while no Refs exist), no `Ref` will ever reclaim `keepalive_raw`, leaking `Inner`.
  - Fix options:
    - Strong invariant: Entries in `entries` always have `refcount > 0`. All creation APIs must return a `Ref`, and any API that would leave `refcount == 0` must remove the slot immediately. Then “entries non-empty” implies “some Ref exists” and keepalive is correct.
    - Or, maintain a global live-ref counter in `Inner` (e.g., `live_refs: Cell<usize>`) updated in `Ref::clone`/`drop`. On `RcHashMap::drop`, enter keepalive only if `live_refs > 0`. Otherwise, drop `Inner` normally even if `entries` contains zero-ref entries (they must be removed eagerly by policy).

- Refcount robustness and panics in `Drop`
  - `Ref::clone` panics on overflow; `Ref::drop` asserts underflow cannot occur (docs/design.md:73). Panicking in `Drop` is undesirable because unwinding across drop may occur during other unwinds.
  - Recommendations: Use saturating or checked arithmetic with explicit debug assert + immediate return in release. Ensure `Drop` never panics; treat underflow as a `debug_assert!`-only condition and otherwise clamp to zero.

- Owner identity correctness across reuse
  - `Ref` equality and hashing include `(owner_ptr, slot)` (docs/design.md:45). After keepalive reclamation, the allocator might reuse the same address for a new map. This is fine because no `Ref` from the old map can exist if `Inner` was freed; if a `Ref` was leaked, `Inner` cannot be freed. Call out this reasoning in the safety notes to preempt concerns.

Unspecified or Underspecified Behaviors (specify explicitly)
- State model of entries vs. index
  - Define three states per key: indexed+live, unindexed-but-live (kept alive solely by Refs), and gone. Document which APIs observe which states:
    - `contains_key` / `get` consult the index only (ignores unindexed-but-live entries).
    - `access`/`access_mut` operate on a `Ref` regardless of index presence, subject to owner/slot validity checks.
    - `remove(key)` removes from the index; if `refcount == 0`, also remove from `entries`. If `refcount > 0`, the slot persists until the last `Ref` drops.

- `access`/`access_mut` owner and slot checks
  - Already described (docs/design.md:65, 69). Tighten to include the lifetime tie to `&Ref` as above; also specify that generation mismatch always returns `None` (even if logically unreachable), avoiding future surprises.

- Reentrancy guarantees
  - Document that during `Ref::drop` the index and entries may be transiently inconsistent and that lookups/insertions must handle both orders safely. State that the implementation removes from the index first, then removes from `entries` (docs/design.md:59) and that `get` tolerates “slot absent” after an index hit by re-checking the slot.

- Keepalive reclamation trigger
  - Specify that the last `Ref` reclaims keepalive only after: (1) removing its own slot, (2) observing the SlotMap is empty, and (3) successfully `take()`-ing `keepalive_raw` (docs/design.md:58, 74). Clarify that reclamation can occur during reentrant drops.

- API shape for insertion and removal
  - Clarify that all creation APIs return a `Ref` and increment `refcount`. If an API ever inserts without returning a `Ref`, it must increment `refcount` and then immediately decrement (and remove) unless a `Ref` escapes. Otherwise, zero-ref entries can exist, which break keepalive assumptions.

- Index removal on `Ref::drop` when index was already removed
  - Spell out that `Ref::drop` attempts index removal “if present” and how that is detected (e.g., probe and conditional erase). Avoid relying on implicit behavior of `RawTable`.

Ergonomics Observations
- No `Deref` for `Ref`
  - Pro: keeps all access centralized via the map and enforces exclusivity.
  - Con: call sites are slightly noisier; you must carry `&RcHashMap` to read `V`. Consider helper adapters (e.g., `map.with(&r, |v| ...)`) to streamline usage.

- Index vs. liveness semantics may surprise
  - After `remove(key)`, `get(key)` returns `None` while existing `Ref`s remain fully usable via `access`. Make this explicit in docs and method names (e.g., prefer `detach` over `remove` if the entry stays alive via handles).

- Refcount overflow panic
  - Cloning a `Ref` can panic on overflow (docs/design.md:37, 73). Consider saturating counts and/or an error-returning `try_clone` for extreme cases, to avoid surprises in production builds.

- Keepalive visibility
  - Dropping the map with live `Ref`s changes how users manage ownership. Consider a small `DropGuard` return or explicit `into_keepalive(self)` for users who want to be deliberate about map shutdown behavior.

Recommended Spec Changes
- Tie lifetimes of `access` and `access_mut` to the `Ref` (see Safety Risks above).
- Codify SlotMap invariants relied upon: removal never relocates other entries; address stability across removals; no shrinking while `&V` exists.
- Define the entry state machine (indexed+live, unindexed+live, gone) and which APIs observe which states.
- Strengthen keepalive precondition: either “no zero-ref entries can exist” (preferred), or track total live refs and only keepalive when `live_refs > 0`.
- Document reentrancy semantics during `Ref::drop` and the chosen removal order.
- Ensure `Drop` paths never panic; use debug assertions only.

Test Additions
- Compile-fail tests for dropping a `Ref` while holding `&V` or `&mut V` from `access`/`access_mut`.
- Reentrancy tests: in `V::drop`, call `get/insert/remove` on the map; assert no panics and invariants hold.
- Index/entries consistency tests for “detached” entries: after `remove(key)`, `get(key)` is `None` but `access` on an existing `Ref` still works.
- Keepalive tests: drop the map with live `Ref`s; last `Ref` reclaims `Inner`. Also test drop with no live `Ref`s (no keepalive) if you choose the “live_refs > 0” condition.
- Refcount stress tests: many clones/drops without overflow or panics in `Drop`.

Open Questions
- Do we want to forbid zero-ref entries by construction (preferred), or support them and adjust keepalive logic?
- Should we provide a read guard type to make access patterns more ergonomic and to centralize lifetime tying to `Ref`?
- Do we want a “detached” terminology/API to make the index-vs-liveness distinction explicit to users?

Conclusion
The design is close to sound and practical for single-threaded use, but there is one critical unsoundness to fix immediately: the lifetimes in `access`/`access_mut` must be tied to the `Ref`. Beyond that, the behavior around keepalive, reentrancy, and the index/entries state machine should be specified more precisely so that the unsafe interior mutability remains justified and robust. With these adjustments, the approach should be implementable safely without atomics or per-entry allocations.

