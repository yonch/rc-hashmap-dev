#![cfg(test)]

// Property tests for HandleHashMap kept inside the crate so they do not
// require feature gates to access internal modules.

use crate::handle_hash_map::{Handle, HandleHashMap, InsertError};
use proptest::prelude::*;
use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

// Key newtype with Borrow<str> to exercise borrowed lookup.
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Key(String);
impl fmt::Debug for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::borrow::Borrow<str> for Key {
    fn borrow(&self) -> &str {
        &self.0
    }
}

// Pool-indexed operations to improve shrinking: indices shrink to earlier keys,
// pool length shrinks, and op lists shrink in length.
#[derive(Clone, Debug)]
enum OpI {
    Insert(usize, i32),
    InsertWith(usize, i32),
    Remove(usize),
    Find(usize),
    Contains(String),
    Mutate(usize, i32),
    Iterate,
}

fn key_from(pool: &[String], i: usize) -> Key {
    Key(pool[i].clone())
}

fn arb_scenario() -> impl Strategy<Value = (Vec<String>, Vec<OpI>)> {
    proptest::collection::vec("[a-z]{0,5}", 1..=8).prop_flat_map(|pool| {
        let idxs: Vec<usize> = (0..pool.len()).collect();
        let idx = proptest::sample::select(idxs);
        let contains_pool = proptest::sample::select(pool.clone());
        let op = prop_oneof![
            (idx.clone(), any::<i32>()).prop_map(|(i, v)| OpI::Insert(i, v)),
            (idx.clone(), any::<i32>()).prop_map(|(i, v)| OpI::InsertWith(i, v)),
            idx.clone().prop_map(OpI::Remove),
            idx.clone().prop_map(OpI::Find),
            prop_oneof![
                contains_pool.prop_map(|s: String| s),
                "[a-z]{0,5}".prop_map(|s| s)
            ]
            .prop_map(OpI::Contains),
            (idx.clone(), any::<i32>()).prop_map(|(i, d)| OpI::Mutate(i, d)),
            Just(OpI::Iterate),
        ];
        proptest::collection::vec(op, 1..60).prop_map(move |ops| (pool.clone(), ops))
    })
}

// Property: State-machine equivalence against std::collections::HashMap.
// Invariants exercised across random operation sequences:
// - Duplicate keys are rejected; on success a unique stable Handle is returned.
// - `find`/`contains_key` parity and handle stability for live entries.
// - `remove(handle)` returns the owned `(K,V)` matching the model and invalidates the handle.
// - `iter` yields each live entry exactly once; key set equals the model’s key set.
// - Stale handles never resolve; `len`/`is_empty` parity with the model after each op.
proptest! {
    #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]
    #[test]
    fn prop_state_machine((pool, ops) in arb_scenario()) {
        let mut sut: HandleHashMap<Key, i32> = HandleHashMap::new();
        let mut model: HashMap<Key, i32> = HashMap::new();
        let mut live: HashMap<Key, Handle> = HashMap::new();
        let mut stale: Vec<Handle> = Vec::new();

        let default_calls = Rc::new(Cell::new(0));
        for op in ops {
            match op {
                OpI::Insert(i, v) => {
                    let k = key_from(&pool, i);
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
                OpI::InsertWith(i, v) => {
                    let k = key_from(&pool, i);
                    let already = model.contains_key(&k);
                    let counter = default_calls.clone();
                    let before = counter.get();
                    match sut.insert_with(k.clone(), move || { counter.set(counter.get() + 1); v }) {
                        Ok(h) => {
                            prop_assert!(!already, "insert_with must fail on duplicate");
                            prop_assert_eq!(default_calls.get(), before + 1, "default must run exactly once on success");
                            let prev = live.insert(k.clone(), h);
                            prop_assert!(prev.is_none());
                            model.insert(k, v);
                        }
                        Err(InsertError::DuplicateKey) => {
                            prop_assert!(already, "duplicate error only when key exists");
                            prop_assert_eq!(default_calls.get(), before, "default must not run on duplicate");
                        }
                    }
                }
                OpI::Remove(i) => {
                    let k = key_from(&pool, i);
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
                OpI::Find(i) => {
                    let k = key_from(&pool, i);
                    let s = sut.find(&k);
                    let present = model.contains_key(&k);
                    prop_assert_eq!(s.is_some(), present);
                    if let Some(h) = s {
                        // If present, handle must be stable and equal to the tracked one.
                        let &lh = live.get(&k).expect("tracked live handle present");
                        prop_assert_eq!(h, lh);
                    }
                }
                OpI::Contains(s) => {
                    let has = sut.contains_key(s.as_str());
                    let has_model = model.keys().any(|k| k.0 == s);
                    prop_assert_eq!(has, has_model);
                }
                OpI::Mutate(i, d) => {
                    let k = key_from(&pool, i);
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
                OpI::Iterate => {
                    let s_keys: BTreeSet<_> = sut.iter().map(|(_, k, _)| k.clone()).collect();
                    let m_keys: BTreeSet<_> = model.keys().cloned().collect();
                    prop_assert_eq!(s_keys, m_keys);
                }
            }

            // Post-conditions after each op
            // 1) All stale handles must not resolve
            for &h in &stale {
                prop_assert!(h.value(&sut).is_none());
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

// Property: Same state-machine invariants as above, under worst-case
// collision behavior (constant hasher). This stresses equality probing
// and collision resolution in the index.
proptest! {
    #![proptest_config(ProptestConfig { cases: 64, .. ProptestConfig::default() })]
    #[test]
    fn prop_state_machine_with_collisions((pool, ops) in arb_scenario()) {
        let mut sut: HandleHashMap<Key, i32, ConstBuildHasher> = HandleHashMap::with_hasher(ConstBuildHasher);
        let mut model: HashMap<Key, i32> = HashMap::new();
        let mut live: HashMap<Key, Handle> = HashMap::new();
        let mut stale: Vec<Handle> = Vec::new();

        let default_calls = Rc::new(Cell::new(0));
        for op in ops {
            match op {
                OpI::Insert(i, v) => {
                    let k = key_from(&pool, i);
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
                OpI::InsertWith(i, v) => {
                    let k = key_from(&pool, i);
                    let already = model.contains_key(&k);
                    let counter = default_calls.clone();
                    let before = counter.get();
                    match sut.insert_with(k.clone(), move || { counter.set(counter.get() + 1); v }) {
                        Ok(h) => {
                            prop_assert!(!already);
                            prop_assert_eq!(default_calls.get(), before + 1);
                            let prev = live.insert(k.clone(), h);
                            prop_assert!(prev.is_none());
                            model.insert(k, v);
                        }
                        Err(InsertError::DuplicateKey) => {
                            prop_assert!(already);
                            prop_assert_eq!(default_calls.get(), before);
                        }
                    }
                }
                OpI::Remove(i) => {
                    let k = key_from(&pool, i);
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
                OpI::Find(i) => {
                    let k = key_from(&pool, i);
                    let s = sut.find(&k);
                    let present = model.contains_key(&k);
                    prop_assert_eq!(s.is_some(), present);
                    if let Some(h) = s { prop_assert_eq!(Some(&h), live.get(&k)); }
                }
                OpI::Contains(s) => {
                    let has = sut.contains_key(s.as_str());
                    let has_model = model.keys().any(|k| k.0 == s);
                    prop_assert_eq!(has, has_model);
                }
                OpI::Mutate(i, d) => {
                    let k = key_from(&pool, i);
                    if let Some(&h) = live.get(&k) {
                        if let Some(vr) = h.value_mut(&mut sut) {
                            *vr = vr.saturating_add(d);
                            if let Some(mv) = model.get_mut(&k) { *mv = mv.saturating_add(d); }
                        } else { prop_assert!(false, "live handle should resolve"); }
                    }
                }
                OpI::Iterate => {
                    let s_keys: BTreeSet<_> = sut.iter().map(|(_, k, _)| k.clone()).collect();
                    let m_keys: BTreeSet<_> = model.keys().cloned().collect();
                    prop_assert_eq!(s_keys, m_keys);
                }
            }

            for &h in &stale { prop_assert!(h.value(&sut).is_none()); }
            prop_assert_eq!(sut.len(), model.len());
            prop_assert_eq!(sut.is_empty(), model.is_empty());
        }
    }
}
