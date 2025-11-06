use proptest::prelude::*;
use rc_hashmap::RcHashMap;

// Model operations on RcHashMap and assert key liveness matches outstanding refs.
proptest! {
    #[test]
    fn prop_rc_hashmap_liveness(keys in 1usize..=5, ops in proptest::collection::vec((0u8..=4u8, 0usize..100usize), 1..100)) {
        // keys in [0..keys-1]
        let mut m: RcHashMap<String, i32> = RcHashMap::new();
        let mut live: Vec<Vec<rc_hashmap::Ref<String, i32>>> = vec![Vec::new(); keys];

        for (op, raw_k) in ops {
            let k = raw_k % keys;
            let key = format!("k{}", k);
            match op {
                // Insert key with value == k
                0 => {
                    let res = m.insert(key.clone(), k as i32);
                    match res {
                        Ok(r) => live[k].push(r),
                        Err(rc_hashmap::InsertError::DuplicateKey) => {},
                    }
                }
                // Get returns a new Ref if present
                1 => {
                    if let Some(r) = m.get(&key) {
                        // Check value path when present
                        if let Some(v) = r.value() {
                            prop_assert_eq!(*v, k as i32);
                        }
                        live[k].push(r);
                    }
                }
                // Clone one existing Ref for this key
                2 => {
                    if let Some(existing) = live[k].pop() {
                        let cloned = existing.clone();
                        live[k].push(existing);
                        live[k].push(cloned);
                    }
                }
                // Drop one existing Ref for this key
                3 => {
                    if let Some(r) = live[k].pop() { drop(r); }
                }
                // Occasionally drop all refs for a key
                4 => {
                    while let Some(r) = live[k].pop() { drop(r); }
                }
                _ => unreachable!(),
            }

            // Invariants after each step
            let present = m.contains_key(&key);
            prop_assert_eq!(present, !live[k].is_empty());
        }

        // Final invariant: len equals number of keys with live refs
        let expected_len = live.iter().filter(|v| !v.is_empty()).count();
        prop_assert_eq!(m.len(), expected_len);
    }
}
