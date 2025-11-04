use std::marker::PhantomData;
use std::rc::{Rc, Weak};

/// ManualRc encapsulates raw-pointer based strong-count manipulation for Rc.
///
/// It holds a raw pointer obtained from `Rc::into_raw` on a clone of a
/// provided `Rc<T>` (and immediately drops that clone to avoid holding
/// an extra strong count), plus a `Weak<T>` to the same allocation to
/// reason about liveness in debug checks.
///
/// Safety: `get`/`put` must only be called while the allocation is alive.
/// The intended usage is within a data structure that also holds an owning
/// `Rc<T>` for the same allocation and uses `get` when creating a new live
/// entry and `put` when the last such entry is removed.
pub struct ManualRc<T> {
    ptr: *const T,
    weak: Weak<T>,
    // !Send + !Sync like Rc
    _nosend: PhantomData<*mut ()>,
}

impl<T> ManualRc<T> {
    /// Create a new ManualRc keeper from an existing `&Rc<T>`.
    ///
    /// This stores a raw pointer suitable for `Rc::increment_strong_count`
    /// and `Rc::decrement_strong_count` and a `Weak<T>` to the same allocation.
    pub fn new(rc: &Rc<T>) -> Self {
        let weak = Rc::downgrade(rc);
        let raw = Rc::into_raw(rc.clone());
        // Drop the temporary clone; we only keep the raw pointer.
        unsafe { Rc::decrement_strong_count(raw) };
        Self { ptr: raw, weak, _nosend: PhantomData }
    }

    /// Increment the strong count using the stored raw pointer.
    pub fn get(&self) {
        debug_assert!(self.weak.strong_count() > 0);
        unsafe { Rc::increment_strong_count(self.ptr) }
    }

    /// Decrement the strong count using the stored raw pointer.
    pub fn put(&self) {
        unsafe { Rc::decrement_strong_count(self.ptr) }
    }

    /// Expose the raw pointer for identity checks.
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }
}

