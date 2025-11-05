//! Debug-only reentrancy guard.
//!
//! Single-threaded structure to detect accidental reentrancy into a data
//! structure. In debug builds, entering twice without dropping the guard
//! panics. In release builds, this compiles to a zero-cost no-op.

use core::cell::Cell;
use core::marker::PhantomData;

/// Per-instance reentrancy tracker. Embed this in structs to guard public
/// entry-points with `let _g = self.reentrancy.enter();`.
#[derive(Debug)]
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
            assert!(
                d == 0,
                "reentrancy detected: nested entry into data structure"
            );
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
    fn default() -> Self {
        Self::new()
    }
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

#[cfg(test)]
mod tests {
    use super::DebugReentrancy;

    #[test]
    fn enter_and_exit_is_ok() {
        let r = DebugReentrancy::new();
        let _g = r.enter();
    }

    #[cfg(debug_assertions)]
    #[test]
    fn reentrancy_panics_in_debug() {
        let r = DebugReentrancy::new();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g1 = r.enter();
            // Re-entering should panic in debug builds
            let _g2 = r.enter();
            let _ = _g2; // silence unused
        }));
        assert!(res.is_err(), "expected reentrancy to panic in debug builds");
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn reentrancy_noop_in_release() {
        let r = DebugReentrancy::new();
        let _g1 = r.enter();
        let _g2 = r.enter();
        let (_g1, _g2) = (_g1, _g2);
    }
}
