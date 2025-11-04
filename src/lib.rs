//! rc-hashmap: A hashmap with reference-counted key-value entries.
//!
//! This crate is currently a placeholder. The data structure and API
//! will be implemented in future versions.
//!
//! Helper type `ManualRc<T>` encapsulates the raw-pointer based use of
//! `std::rc::Rc` increment/decrement APIs, and is intended to be used by
//! the final Rc-backed map layer.

mod manual_rc;

pub use manual_rc::ManualRc;
