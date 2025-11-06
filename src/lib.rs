//! rc-hashmap: A single-threaded, handle-based map with Rc-like
//! references to entries that allow fast access and cleanup on drop.
//!
//! This crate provides three internal layers built to keep invariants
//! simple and auditable:
//! - HandleHashMap: structural map with stable handles and a debug-only
//!   reentrancy guard to keep internals consistent during operations.
//! - CountedHashMap: adds a per-entry reference count.
//! - RcHashMap: public API that exposes `Ref` handles with Rc-like
//!   clone/drop semantics.
//!
//! The internal `RcCount<T>` helper (in `tokens`) encapsulates the
//! raw-pointer based use of `std::rc::Rc` increment/decrement APIs.

mod rc_hash_map;
mod reentrancy;
mod tokens;
mod counted_hash_map;
mod handle_hash_map;
mod handle_hash_map_proptest;

// Public surface
pub use rc_hash_map::{RcHashMap, Ref};
pub use handle_hash_map::InsertError;

// Optional: expose the internal HandleHashMap to criterion benches when requested.
// This keeps the public API surface clean by default.
#[cfg(feature = "bench_internal")]
pub use handle_hash_map::HandleHashMap;
