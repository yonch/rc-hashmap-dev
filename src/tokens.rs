//! Lifetime-tied linear tokens and counting traits.
//!
//! Tokens are zero-sized proofs that a unit was acquired from a
//! particular counter instance. Dropping a token panics; the only valid
//! way to dispose of it is to return it to the originating counter via
//! `Count::put`.

use core::cell::Cell;
use core::marker::PhantomData;
use std::rc::{Rc, Weak};

/// Zero-sized, linear token tied to its originating counter via lifetime.
pub struct Token<'a, C: ?Sized> {
    // Lifetime is tracked separately from the counter type to avoid
    // imposing `'a` bounds on `C` (useful for generic counters).
    _lt: PhantomData<&'a ()>,
    _ctr: PhantomData<*const C>,
}

impl<'a, C: ?Sized> Token<'a, C> {
    #[inline]
    pub(crate) fn new() -> Self {
        Self {
            _lt: PhantomData,
            _ctr: PhantomData,
        }
    }
}

impl<'a, C: ?Sized> Drop for Token<'a, C> {
    fn drop(&mut self) {
        // Intentional fail-fast on misuse: token must be consumed by Count::put.
        panic!("Token dropped without Count::put");
    }
}

/// A source of counted references, enforced by linear Token flow.
pub trait Count {
    /// The token type minted by this counter.
    type Token<'a>: Sized
    where
        Self: 'a;

    /// Acquire one counted reference and return a linear token for it.
    ///
    /// We mint tokens with a 'static lifetime parameter. The token itself is
    /// still branded to this counter via its type parameter, and can be
    /// covariantly shortened when returning it via `put`.
    fn get(&self) -> Self::Token<'static>;

    /// Return (consume) a previously acquired token.
    /// Returns true if the count is now zero.
    fn put<'a>(&'a self, t: Self::Token<'a>) -> bool;
}

/// Single-threaded reference counter for entries.
#[derive(Debug)]
pub struct UsizeCount {
    count: Cell<usize>,
}

impl UsizeCount {
    pub fn new(initial: usize) -> Self {
        Self {
            count: Cell::new(initial),
        }
    }
}

impl Count for UsizeCount {
    type Token<'a>
        = Token<'a, Self>
    where
        Self: 'a;

    #[inline]
    fn get(&self) -> Self::Token<'static> {
        let c = self.count.get();
        let n = c.wrapping_add(1);
        self.count.set(n);
        if n == 0 {
            // Follow Rc semantics: abort on overflow rather than continue unsafely.
            std::process::abort();
        }
        Token::<'static, Self>::new()
    }

    #[inline]
    fn put<'a>(&'a self, t: Self::Token<'a>) -> bool {
        let c = self.count.get();
        assert!(c > 0, "UsizeCount underflow");
        let n = c - 1;
        self.count.set(n);
        core::mem::forget(t);
        n == 0
    }
}

/// Rc-backed manual counter. Uses raw-pointer strong count manipulation.
pub struct RcCount<T> {
    ptr: *const T,
    weak: Weak<T>,
    _nosend: PhantomData<*mut ()>,
}

impl<T> RcCount<T> {
    pub fn new(rc: &Rc<T>) -> Self {
        let weak = Rc::downgrade(rc);
        let raw = Rc::into_raw(rc.clone());
        unsafe { Rc::decrement_strong_count(raw) };
        Self {
            ptr: raw,
            weak,
            _nosend: PhantomData,
        }
    }
}

impl<T: 'static> Count for RcCount<T> {
    type Token<'a>
        = Token<'a, Self>
    where
        Self: 'a;

    #[inline]
    fn get(&self) -> Self::Token<'static> {
        debug_assert!(self.weak.strong_count() > 0);
        unsafe { Rc::increment_strong_count(self.ptr) };
        Token::<'static, Self>::new()
    }

    #[inline]
    fn put<'a>(&'a self, t: Self::Token<'a>) -> bool {
        debug_assert!(self.weak.strong_count() > 0);
        let was_one = self.weak.strong_count() == 1;
        unsafe { Rc::decrement_strong_count(self.ptr) };
        core::mem::forget(t);
        was_one
    }
}
