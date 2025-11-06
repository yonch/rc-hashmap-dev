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

    /// Returns true if the current count is zero.
    #[inline]
    pub fn is_zero(&self) -> bool {
        self.count.get() == 0
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

    pub fn from_weak(weak: &Weak<T>) -> Self {
        let raw = weak.as_ptr();
        Self {
            ptr: raw,
            weak: weak.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn token_drop_panics() {
        let c = UsizeCount::new(0);
        let t = c.get();
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(t)));
        assert!(res.is_err());
    }

    #[test]
    fn usizecount_balance_and_zero() {
        let c = UsizeCount::new(0);
        let t1 = c.get();
        let t2 = c.get();
        assert!(!c.is_zero());
        assert!(!c.put(t1));
        assert!(c.put(t2));
        assert!(c.is_zero());
    }

    #[test]
    fn rccount_increments_and_put_flag() {
        let rc = Rc::new(123);
        let weak = Rc::downgrade(&rc);
        let c = RcCount::new(&rc);
        let before = weak.strong_count();
        let t = c.get();
        assert_eq!(weak.strong_count(), before + 1);
        let was_one = c.put(t);
        assert!(!was_one);
        assert_eq!(weak.strong_count(), before);
    }

    proptest! {
        #[test]
        fn prop_usizecount_get_put_balance(ops in proptest::collection::vec(0u8..=1, 0..200)) {
            let c = UsizeCount::new(0);
            let mut toks: Vec<Token<'static, UsizeCount>> = Vec::new();
            for op in ops.iter().copied() {
                match op {
                    0 => {
                        toks.push(c.get());
                        assert!(!c.is_zero());
                    }
                    _ => {
                        if let Some(t) = toks.pop() {
                            let now_zero = c.put(t);
                            assert_eq!(now_zero, toks.is_empty());
                            assert_eq!(c.is_zero(), toks.is_empty());
                        }
                    }
                }
            }
            assert_eq!(c.is_zero(), toks.is_empty());
            while let Some(t) = toks.pop() {
                let now_zero = c.put(t);
                assert_eq!(now_zero, toks.is_empty());
            }
            assert!(c.is_zero());
        }

        #[test]
        fn prop_two_usizecounts_independent(ops in proptest::collection::vec((0u8..=1, 0u8..=1), 0..200)) {
            let a = UsizeCount::new(0);
            let b = UsizeCount::new(0);
            let mut ta: Vec<Token<'static, UsizeCount>> = Vec::new();
            let mut tb: Vec<Token<'static, UsizeCount>> = Vec::new();
            for (which, op) in ops.into_iter() {
                match (which, op) {
                    (0, 0) => { ta.push(a.get()); }
                    (0, 1) => {
                        if let Some(t) = ta.pop() {
                            let now_zero = a.put(t);
                            assert_eq!(now_zero, ta.is_empty());
                        }
                    }
                    (1, 0) => { tb.push(b.get()); }
                    (1, 1) => {
                        if let Some(t) = tb.pop() {
                            let now_zero = b.put(t);
                            assert_eq!(now_zero, tb.is_empty());
                        }
                    }
                    _ => unreachable!(),
                }
                assert_eq!(a.is_zero(), ta.is_empty());
                assert_eq!(b.is_zero(), tb.is_empty());
            }

            while let Some(t) = ta.pop() { let _ = a.put(t); }
            while let Some(t) = tb.pop() { let _ = b.put(t); }
            assert!(a.is_zero());
            assert!(b.is_zero());
        }

        #[test]
        fn prop_rccount_roundtrip(n in 0usize..100) {
            let rc = Rc::new(());
            let weak = Rc::downgrade(&rc);
            let c = RcCount::new(&rc);
            let before = weak.strong_count();
            let mut toks: Vec<Token<'static, RcCount<_>>> = Vec::new();
            for _ in 0..n { toks.push(c.get()); }
            assert_eq!(weak.strong_count(), before + n);
            while let Some(t) = toks.pop() {
                let was_one = c.put(t);
                assert!(!was_one);
                assert_eq!(weak.strong_count(), before + toks.len());
            }
            assert_eq!(weak.strong_count(), before);
        }
    }
}
