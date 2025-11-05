#![cfg(test)]

use rc_hashmap::DebugReentrancy;

#[test]
fn enter_and_exit_is_ok() {
    let r = DebugReentrancy::new();
    let _g = r.enter();
    // drop guard at end of scope
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
