//! Debug-only reentrancy guard.
//!
//! Single-threaded structure to detect accidental reentrancy into a data
//! structure. In debug builds, entering twice without dropping the guard
//! panics. In release builds, this compiles to a zero-cost no-op.

use core::cell::Cell;
use core::marker::PhantomData;

/// Per-instance reentrancy tracker. Embed this in structs to guard public
/// entry-points with `let _g = self.reentrancy.enter();`.
pub struct DebugReentrancy {
    #[cfg(debug_assertions)]
    depth: Cell<u32>,
    // Keep !Send + !Sync in line with single-threaded design.
    _nosend: PhantomData<*mut ()>,
}

impl DebugReentrancy {
    /// Create a new reentrancy tracker. Const so it can be a field default.
    pub const fn new() -> Self {
        Self {
            #[cfg(debug_assertions)]
            depth: Cell::new(0),
            _nosend: PhantomData,
        }
    }

    /// Enter a guarded section. In debug builds, panics if already entered.
    #[inline]
    pub fn enter(&self) -> ReentrancyGuard<'_> {
        #[cfg(debug_assertions)]
        {
            let d = self.depth.get();
            assert!(d == 0, "reentrancy detected: nested entry into data structure");
            self.depth.set(d + 1);
            return ReentrancyGuard { owner: self };
        }

        #[cfg(not(debug_assertions))]
        {
            return ReentrancyGuard { _z: PhantomData };
        }
    }
}

impl Default for DebugReentrancy {
    fn default() -> Self { Self::new() }
}

/// RAII guard returned by `DebugReentrancy::enter`.
pub struct ReentrancyGuard<'a> {
    #[cfg(debug_assertions)]
    owner: &'a DebugReentrancy,
    #[cfg(not(debug_assertions))]
    _z: PhantomData<&'a ()>,
}

impl<'a> Drop for ReentrancyGuard<'a> {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        {
            let d = self.owner.depth.get();
            debug_assert!(d > 0);
            self.owner.depth.set(d - 1);
        }
    }
}

