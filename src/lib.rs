//! rc-hashmap: A single-threaded, handle-based map with Rc-like
//! references to entries that allow fast access and cleanup on drop.
//!
//! Internal Design:
//!
//! Summary
//! - Goal: build RcHashMap in safe, verifiable layers so each piece can
//!   be reasoned about independently.
//! - Layers:
//!   - HandleHashMap<K, V, S>: structural map that returns stable
//!     handles for O(1) average access without re-hashing; includes a
//!     debug-only reentrancy guard to keep internals consistent while
//!     mutating.
//!   - CountedHashMap<K, V, S>: wraps HandleHashMap and adds per-entry
//!     reference counting (increments on get/clone, decrements on put).
//!   - RcHashMap<K, V, S>: public API that exposes `Ref` handles; drops
//!     free entries when the last `Ref` is dropped.
//!
//! Constraints
//! - Single-threaded: `!Send`/`!Sync` by design (no atomics).
//! - No per-entry heap allocations beyond the map’s own storage.
//! - Stable, generational keys behind small `Handle` wrappers.
//! - O(1) average lookups with unique keys; duplicate inserts fail.
//! - Reentrancy: disallowed during critical sections of HandleHashMap
//!   (only `K: Eq/Hash` may run); allowed elsewhere.
//!
//! Why this split?
//! - Localize invariants: each layer has a small, precise contract.
//! - Minimize unsafe: raw-pointer handling is isolated in `tokens::RcCount`;
//!   structural indexing uses safe Rust.
//! - Clear failure boundaries: HandleHashMap never calls into user code
//!   once the structure is consistent.
//!
//! Reentrancy policy and interior mutability
//! - HandleHashMap employs an internal exclusive-access discipline and a
//!   debug-only reentrancy guard at the start of each method to prevent
//!   nested entry while its internal state can be transiently
//!   inconsistent. These methods only invoke user code via `K: Eq/Hash`
//!   during probing.
//! - Upper layers (CountedHashMap, RcHashMap) rely on HandleHashMap’s
//!   guarantees and do not need their own guard. After
//!   `HandleHashMap::remove` returns `(K, V)`, the structure is again
//!   consistent; `Drop` for `K`/`V` may reenter safely.
//!
//! Overflow semantics
//! - Reference-count overflow is considered undefined behavior, matching
//!   `Rc`. The crate assumes there are fewer than `usize::MAX` references
//!   to any entry; no runtime checks are performed.
//!
//! Hasher and rehashing invariants
//! - Each entry stores a precomputed `u64` hash and indexing always uses
//!   the stored hash; `K: Hash` is never invoked after insertion. This
//!   avoids rehash-time calls into user code.
//!
//! Notes and non-goals
//! - Still single-threaded; enforced with marker types on `Ref`/`Inner`.
//! - No weak handles (could be added later).
//! - No explicit `clear()`/`remove()`/`drain()` on RcHashMap; removal
//!   occurs when the last `Ref` is dropped to preserve refcount
//!   semantics.
//! - RcHashMap does not implement `Clone`.
//! - Keys are immutable post-insert; there is no `key_mut`.
//! - Public API surface is `RcHashMap` and its `Ref`; lower layers are
//!   implementation details.
//!
//! Implementation note
//! - The internal `RcCount<T>` helper (in `tokens`) encapsulates the
//!   raw-pointer based use of `std::rc::Rc` increment/decrement APIs.

mod counted_hash_map;
pub mod handle_hash_map;
mod handle_hash_map_proptest;
mod rc_hash_map;
mod reentrancy;
pub mod tokens;

// Public surface
pub use handle_hash_map::InsertError;
pub use rc_hash_map::{RcHashMap, Ref};
