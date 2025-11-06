Reference Tokens: A Safer Counting Interface

Goal
- Provide a lifetime-driven API that forces balanced reference counting without relying on caller discipline.
- Use zero-sized, linear tokens whose destruction panics unless they are returned to their originating counter.
- Make the ownership/liveness of map internals easier to reason about: entries keep their backing owner alive; user-facing refs are guaranteed tracked and released.

Abstraction
- Count: an object that mints and reclaims a unit “reference” by returning a Token and later accepting it back. Count is the sole place where increments and decrements occur.
- Token: a zero-sized, non-cloneable proof that one unit was acquired. It is lifetime-bound to the Count value that minted it, and its Drop panics to catch unbalanced flows. The only valid disposal is passing it to Count::put.

Why the type system helps
- Origin binding: Token<'a, C> uses two markers to separate lifetime from the counter type: `PhantomData<&'a ()>` tracks the lifetime, and `PhantomData<*const C>` brands the token to the counter type without requiring `C: 'a`. This avoids imposing lifetime bounds on `C` while preserving the brand so a token can only be returned to the same counter value.
- Linearity and balance: Token does not implement Copy or Clone, so it cannot be duplicated. Each `get` yields exactly one Token that must be consumed by exactly one `put`. Dropping a Token instead of returning it panics, catching unbalanced flows.
- Zero cost: Tokens are ZSTs; they add no runtime footprint and no allocation. The only costs are the underlying counter operations.

Unwinding and Drop panics
- Panicking in Token::drop during another unwind aborts. Tokens are internal implementation details, so this fail-fast behavior is acceptable and desired for our use case.

API Sketch (as implemented)
```rust
use core::marker::PhantomData;

/// Zero-sized, linear token tied to its originating count via lifetime.
pub struct Token<'a, C: ?Sized> {
    // Lifetime tracked separately from counter type.
    _lt: PhantomData<&'a ()>,
    // Brand to the counter type without imposing `C: 'a`.
    _ctr: PhantomData<*const C>,
}

impl<'a, C: ?Sized> Token<'a, C> {
    #[inline]
    fn new() -> Self { Self { _lt: PhantomData, _ctr: PhantomData } }
}

impl<'a, C: ?Sized> Drop for Token<'a, C> {
    fn drop(&mut self) {
        // Intentional: misuse should be loud. See "Unwinding" note below.
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

 

Moving a Token out in Drop
- To avoid branches and keep zero-sized storage, store tokens in `core::mem::ManuallyDrop<Token<...>>` and extract them in `Drop` with `ManuallyDrop::take`. This avoids constructing throwaway tokens and keeps the path branch-free.

Owned-token pattern (no ManuallyDrop)
- When a function owns the token and can consume it by value (i.e., not in a `Drop` impl), prefer moving the token directly into `Count::put` without `ManuallyDrop`.
- Example: `CountedHandle` owns `Token<'_, UsizeCount>`. `CountedHashMap::put(self, handle)` consumes `handle`, moves out `handle.token` by value, and calls `entry.refcount.put(token)`. Because the token is owned and consumed in this path, there is no need for `ManuallyDrop`.
- This pattern relies on structuring APIs so that token consumption happens in an owned context (e.g., methods that take `self` or consume a handle) rather than inside `Drop`, where only `&mut self` is available.

Generic holder pattern
```rust
use core::mem::ManuallyDrop;

struct Holder<'a, C: Count> {
    counter: &'a C,
    token: ManuallyDrop<C::Token<'a>>,
}

impl<'a, C: Count> Holder<'a, C> {
    fn new(counter: &'a C) -> Self {
        Self { counter, token: ManuallyDrop::new(counter.get()) }
    }
}

impl<'a, C: Count> Drop for Holder<'a, C> {
    fn drop(&mut self) {
        // SAFETY: We never drop `token` automatically; we always move it out once.
        let t = unsafe { ManuallyDrop::take(&mut self.token) };
        let _ = self.counter.put(t);
    }
}
```

Using the pieces together

Keep the owner alive while any entry exists
```rust
/// The per-map Rc owner whose allocation must outlive all entries.
struct Owner { /* ... */ }

/// Within the map state
struct MapInner {
    owner_rc: std::rc::Rc<Owner>,
    owner_count: RcCount<Owner>,
}

struct Entry<'m, K, V> {
    // Keep-alive for the owner allocation; dropped when Entry is dropped.
    _owner_keepalive: core::mem::ManuallyDrop<Token<'m, RcCount<Owner>>>,
    // Count of outstanding user-visible Refs to this entry.
    refcount: UsizeCount,
    key: K,
    value: V,
    // borrow to tie the token lifetime to the map’s RcCount value
    _map_counter: &'m RcCount<Owner>,
}

impl<'m, K, V> Entry<'m, K, V> {
    fn new(map: &'m MapInner, map_counter: &'m RcCount<Owner>, key: K, value: V) -> Self {
        let owner_token = map_counter.get(); // bumps Rc strong count
        Self {
            _owner_keepalive: core::mem::ManuallyDrop::new(owner_token),
            refcount: UsizeCount::new(0),
            key, value,
            _map_counter: map_counter,
        }
    }
}

impl<'m, K, V> Drop for Entry<'m, K, V> {
    fn drop(&mut self) {
        // We must return the owner token; move it out and put it.
        let token = unsafe { core::mem::ManuallyDrop::take(&mut self._owner_keepalive) };
        let _now_zero = self._map_counter.put(token);
    }
}
```

Ensure every user Ref is counted and released
```rust
/// A handle to an entry’s value, cloning increments the entry’s UsizeCount.
struct Ref<'m, V> {
    value_ptr: *const V,               // or a safer handle in the real map
    counter: &'m UsizeCount,           // entry-local counter
    token: core::mem::ManuallyDrop<Token<'m, UsizeCount>>, // linear token for this counted ref
}

impl<'m, V> Ref<'m, V> {
    fn new(counter: &'m UsizeCount, value_ptr: *const V) -> Self {
        let token = counter.get();
        Self { value_ptr, counter, token: core::mem::ManuallyDrop::new(token) }
    }

    fn get(&self) -> &V { unsafe { &*self.value_ptr } }
}

impl<'m, V> Clone for Ref<'m, V> {
    fn clone(&self) -> Self {
        Self { value_ptr: self.value_ptr, counter: self.counter, token: core::mem::ManuallyDrop::new(self.counter.get()) }
    }
}

impl<'m, V> Drop for Ref<'m, V> {
    fn drop(&mut self) {
        // Move the token out and return it.
        let token = unsafe { core::mem::ManuallyDrop::take(&mut self.token) };
        let now_zero = self.counter.put(token);
        if now_zero {
            // No more Refs to this entry. In practice, coordinate with the map.
        }
    }
}
```

Implementation variants

UsizeCount and RcCount
- UsizeCount: single-threaded counter using Cell<usize> to track outstanding user-facing references to an entry. Increment uses wrapping_add and aborts on wrap to 0 (matching Rc); decrement asserts nonzero before subtracting. An `is_zero()` helper is provided for checking whether the current count is zero.
- RcCount<T>: encapsulates raw Rc strong-count inc/dec behind the Count interface. Unsafety is internal; callers only manipulate Tokens. You can construct it from an `Rc<T>` via `RcCount::new(&rc)` or from a `Weak<T>` via `RcCount::from_weak(&weak)`.

```rust
use core::cell::Cell;

/// RcCount: ties lifetime-safe tokens to raw Rc strong-count ops.
pub struct RcCount<T> {
    ptr: *const T,
    weak: std::rc::Weak<T>,
    // !Send + !Sync like Rc
    _nosend: core::marker::PhantomData<*mut ()>,
}

impl<T> RcCount<T> {
    /// Create from an existing Rc; stores a raw pointer compatible with
    /// Rc::increment_strong_count/decrement_strong_count and a Weak for debug checks.
    pub fn new(rc: &std::rc::Rc<T>) -> Self {
        let weak = std::rc::Rc::downgrade(rc);
        let raw = std::rc::Rc::into_raw(rc.clone());
        // Drop the temporary clone; keep only the raw pointer for inc/dec.
        unsafe { std::rc::Rc::decrement_strong_count(raw) };
        Self { ptr: raw, weak, _nosend: core::marker::PhantomData }
    }

    /// Create from an existing Weak. Useful when using `Rc::new_cyclic`.
    pub fn from_weak(weak: &std::rc::Weak<T>) -> Self {
        let raw = weak.as_ptr();
        Self { ptr: raw, weak: weak.clone(), _nosend: core::marker::PhantomData }
    }
}

impl<T: 'static> Count for RcCount<T> {
    type Token<'a> = Token<'a, Self> where Self: 'a;

    #[inline]
    fn get(&self) -> Self::Token<'static> {
        // Debug-only liveness check: there must be at least one strong count
        debug_assert!(self.weak.strong_count() > 0);
        unsafe { std::rc::Rc::increment_strong_count(self.ptr) };
        Token::<'static, Self>::new()
    }

    #[inline]
    fn put<'a>(&'a self, t: Self::Token<'a>) -> bool {
        // Debug-only liveness check mirrors `get`.
        debug_assert!(self.weak.strong_count() > 0);
        // Observe the count before decrement; after decrement the allocation may be freed.
        let was_one = self.weak.strong_count() == 1;
        unsafe { std::rc::Rc::decrement_strong_count(self.ptr) };
        core::mem::forget(t); // disarm the panic-on-Drop for the token
        was_one
    }
}

/// Single-threaded reference counter for entries.
pub struct UsizeCount {
    count: Cell<usize>,
}

impl UsizeCount {
    pub fn new(initial: usize) -> Self { Self { count: Cell::new(initial) } }
    /// Returns true if the current count is zero.
    pub fn is_zero(&self) -> bool { self.count.get() == 0 }
}

impl Count for UsizeCount {
    type Token<'a> = Token<'a, Self> where Self: 'a;

    #[inline]
    fn get(&self) -> Self::Token<'static> {
        let c = self.count.get();
        // Match Rc semantics: wrapping add, store, then abort on wraparound.
        let n = c.wrapping_add(1);
        self.count.set(n);
        if n == 0 {
            // Abort the process rather than silently overflowing and risking logic errors.
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
        core::mem::forget(t); // disarm panic-on-Drop
        n == 0
    }
}
```

Notes on practical struct layout
- Observing zero: For `UsizeCount::put` we return a bool indicating whether the count reached zero. The map uses this to decide whether to unlink and drop the entry immediately. `RcCount::put` returns true iff the strong count was 1 before the decrement (single-threaded assumption and typically false if the map also holds a strong `Rc`).
- No shared mutation or threading: This design is single-threaded. `UsizeCount` is not `Sync`, and `RcCount` inherits `Rc`’s `!Send + !Sync` semantics.
- Overflow behavior (same as Rc): `UsizeCount::get` performs `wrapping_add(1)`, stores it, then aborts the process if the result is 0. This mirrors `Rc`’s strong-count increment semantics and avoids continuing after overflow.
 - Debug-only behavior: `RcCount::{get,put}` include debug assertions on liveness via `Weak::strong_count()`. These checks are compiled out in release builds and have zero cost.

 

Alternatives considered
- Plain `usize` counts without tokens: relies on discipline and is easy to misuse (double put, missing put on early return). Tokens close this gap by construction.
- Storing a runtime back-pointer in the token: unnecessary; lifetime binding to `&self` is sufficient to prevent cross-counter misuse.
