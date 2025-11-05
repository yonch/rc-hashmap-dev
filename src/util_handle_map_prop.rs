#![cfg(test)]

// Property tests for HandleHashMap kept inside the crate so they do not
// require feature gates to access internal modules.

use crate::util_handle_map::{Handle, HandleHashMap, InsertError};
use proptest::prelude::*;
use std::borrow::Borrow;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};

// Key newtype with Borrow<str> to exercise borrowed lookup.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Key(String);
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl Borrow<str> for Key {
    fn borrow(&self) -> &str {
        &self.0
    }
}

prop_compose! {
    fn arb_key()(s in "[a-z]{0,6}") -> Key { Key(s) }
}

#[derive(Clone, Debug)]
enum Op {
    Insert(Key, i32),
    Remove(Key),
    Find(Key),
    Contains(String),
    Mutate(Key, i32),
    Iterate,
}

prop_compose! {
    fn arb_ops()(ops in proptest::collection::vec(
        prop_oneof![
            (arb_key(), any::<i32>()).prop_map(|(k,v)| Op::Insert(k,v)),
            arb_key().prop_map(Op::Remove),
            arb_key().prop_map(Op::Find),
            "[a-z]{0,6}".prop_map(Op::Contains),
            (arb_key(), any::<i32>()).prop_map(|(k,d)| Op::Mutate(k,d)),
            Just(Op::Iterate),
        ], 1..100)) -> Vec<Op> { ops }
}

// State machine harness over HandleHashMap against std::collections::HashMap model.
proptest! {
    #[test]
    fn prop_state_machine(ops in arb_ops()) {
        let mut sut: HandleHashMap<Key, i32> = HandleHashMap::new();
        let mut model: HashMap<Key, i32> = HashMap::new();
        let mut live: HashMap<Key, Handle> = HashMap::new();
        let mut stale: Vec<Handle> = Vec::new();

        for op in ops {
            match op {
                Op::Insert(k, v) => {
                    let already = model.contains_key(&k);
                    match sut.insert(k.clone(), v) {
                        Ok(h) => {
                            prop_assert!(!already, "insert must fail on duplicate");
                            let prev = live.insert(k.clone(), h);
                            prop_assert!(prev.is_none());
                            model.insert(k, v);
                        }
                        Err(InsertError::DuplicateKey) => {
                            prop_assert!(already, "duplicate error only when key exists");
                        }
                    }
                }
                Op::Remove(k) => {
                    if let Some(&h) = live.get(&k) {
                        // Removing should return the owned key/value pair equal to model
                        let (kk, vv) = sut.remove(h).expect("handle valid for removal");
                        prop_assert!(kk == k);
                        let mv = model.remove(&kk).expect("present in model");
                        prop_assert_eq!(vv, mv);
                        let _ = live.remove(&k);
                        stale.push(h);
                    } else {
                        // Removing a non-existent key: do nothing in model; find should be None
                        prop_assert!(sut.find(&k).is_none());
                    }
                }
                Op::Find(k) => {
                    let s = sut.find(&k);
                    let present = model.contains_key(&k);
                    prop_assert_eq!(s.is_some(), present);
                    if let Some(h) = s {
                        // If present, handle must be stable and equal to the tracked one.
                        let &lh = live.get(&k).expect("tracked live handle present");
                        prop_assert_eq!(h, lh);
                    }
                }
                Op::Contains(s) => {
                    let has = sut.contains_key(s.as_str());
                    let has_model = model.keys().any(|k| k.0 == s);
                    prop_assert_eq!(has, has_model);
                }
                Op::Mutate(k, d) => {
                    if let Some(&h) = live.get(&k) {
                        // Use handle method to mutate in place
                        if let Some(vr) = h.value_mut(&mut sut) {
                            *vr = vr.saturating_add(d);
                            if let Some(mv) = model.get_mut(&k) {
                                *mv = mv.saturating_add(d);
                            }
                        } else {
                            // Stale shouldn't happen for live handle
                            prop_assert!(false, "live handle should resolve");
                        }
                    } else {
                        // No-op when key not present
                    }
                }
                Op::Iterate => {
                    let s_keys: BTreeSet<_> = sut.iter().map(|(_, k, _)| k.clone()).collect();
                    let m_keys: BTreeSet<_> = model.keys().cloned().collect();
                    prop_assert_eq!(s_keys, m_keys);
                }
            }

            // Post-conditions after each op
            // 1) All stale handles must not resolve
            for &h in &stale {
                prop_assert!(h.value_ref(&sut).is_none());
            }
            // 2) Size parity
            prop_assert_eq!(sut.len(), model.len());
            prop_assert_eq!(sut.is_empty(), model.is_empty());
        }
    }
}

// Collision variant using a constant hasher to stress equality resolution.
#[derive(Clone, Default)]
struct ConstBuildHasher;
struct ConstHasher;
impl std::hash::BuildHasher for ConstBuildHasher {
    type Hasher = ConstHasher;
    fn build_hasher(&self) -> Self::Hasher {
        ConstHasher
    }
}
impl Hasher for ConstHasher {
    fn write(&mut self, _bytes: &[u8]) {}
    fn finish(&self) -> u64 {
        0
    }
}

proptest! {
    #[test]
    fn prop_state_machine_with_collisions(ops in arb_ops()) {
        let mut sut: HandleHashMap<Key, i32, ConstBuildHasher> = HandleHashMap::with_hasher(ConstBuildHasher);
        let mut model: HashMap<Key, i32> = HashMap::new();
        let mut live: HashMap<Key, Handle> = HashMap::new();
        let mut stale: Vec<Handle> = Vec::new();

        for op in ops {
            match op {
                Op::Insert(k, v) => {
                    let already = model.contains_key(&k);
                    match sut.insert(k.clone(), v) {
                        Ok(h) => {
                            prop_assert!(!already);
                            let prev = live.insert(k.clone(), h);
                            prop_assert!(prev.is_none());
                            model.insert(k, v);
                        }
                        Err(InsertError::DuplicateKey) => prop_assert!(already),
                    }
                }
                Op::Remove(k) => {
                    if let Some(&h) = live.get(&k) {
                        let (kk, vv) = sut.remove(h).expect("handle valid for removal");
                        prop_assert!(kk == k);
                        let mv = model.remove(&kk).expect("present in model");
                        prop_assert_eq!(vv, mv);
                        let _ = live.remove(&k);
                        stale.push(h);
                    } else {
                        prop_assert!(sut.find(&k).is_none());
                    }
                }
                Op::Find(k) => {
                    let s = sut.find(&k);
                    let present = model.contains_key(&k);
                    prop_assert_eq!(s.is_some(), present);
                    if let Some(h) = s { prop_assert_eq!(Some(&h), live.get(&k)); }
                }
                Op::Contains(s) => {
                    let has = sut.contains_key(s.as_str());
                    let has_model = model.keys().any(|k| k.0 == s);
                    prop_assert_eq!(has, has_model);
                }
                Op::Mutate(k, d) => {
                    if let Some(&h) = live.get(&k) {
                        if let Some(vr) = h.value_mut(&mut sut) {
                            *vr = vr.saturating_add(d);
                            if let Some(mv) = model.get_mut(&k) { *mv = mv.saturating_add(d); }
                        } else { prop_assert!(false, "live handle should resolve"); }
                    }
                }
                Op::Iterate => {
                    let s_keys: BTreeSet<_> = sut.iter().map(|(_, k, _)| k.clone()).collect();
                    let m_keys: BTreeSet<_> = model.keys().cloned().collect();
                    prop_assert_eq!(s_keys, m_keys);
                }
            }

            for &h in &stale { prop_assert!(h.value_ref(&sut).is_none()); }
            prop_assert_eq!(sut.len(), model.len());
            prop_assert_eq!(sut.is_empty(), model.is_empty());
        }
    }
}
