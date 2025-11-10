// RcHashMap property tests (consolidated).
//
// Property 1: basic liveness matches outstanding Refs per key.
//  - Model: per-key multiset of external Refs (Vec of Refs per key).
//  - Invariant: contains_key(key) == !live[k].is_empty();
//               len() == count(keys with !live[k].is_empty()).
//  - Operations: insert, find, clone, drop-one, drop-all.
//  - Accessor check: for successful find, validate value == k.
//
// Property 2: DAG liveness with values holding Refs.
//  - Model: adjacency list (i -> children j) and external Ref roots per i.
//  - Invariant: alive nodes == transitive closure reachable from nodes
//    with external Refs, after pruning edges whose endpoints were removed.
//  - Operations: insert/find/clone/drop/drop-all, add-edge (i->j),
//    remove-edge.
//  - Safety: edges always go from i to some j > i to avoid cycles.
//  - At each step: assert len() and contains_key() match model state.
use proptest::prelude::*;
use rc_hashmap::RcHashMap;

// Property 1: liveness equals outstanding Refs per key.
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
                // Insert key with value == k; duplicate inserts do not mint a token.
                0 => {
                    let res = m.insert(key.clone(), k as i32);
                    match res {
                        Ok(r) => live[k].push(r),
                        Err(rc_hashmap::InsertError::DuplicateKey) => {},
                    }
                }
                // Find returns a new Ref if present; also sanity-check the value accessor.
                1 => {
                    if let Some(r) = m.find(&key) {
                        // Check value path when present
                        let v = r.value(&m).unwrap();
                        prop_assert_eq!(*v, k as i32);
                        live[k].push(r);
                    }
                }
                // Clone one existing Ref for this key.
                2 => {
                    if let Some(existing) = live[k].pop() {
                        let cloned = existing.clone();
                        live[k].push(existing);
                        live[k].push(cloned);
                    }
                }
                // Drop one existing Ref for this key.
                3 => {
                    if let Some(r) = live[k].pop() { drop(r); }
                }
                // Drop all Refs for this key (removal at zero).
                4 => {
                    while let Some(r) = live[k].pop() { drop(r); }
                }
                _ => unreachable!(),
            }

            // Invariant after each step: presence matches whether there is ≥1 outstanding Ref.
            let present = m.contains_key(&key);
            prop_assert_eq!(present, !live[k].is_empty());
        }

        // Final invariant: len equals number of keys with live refs.
        let expected_len = live.iter().filter(|v| !v.is_empty()).count();
        prop_assert_eq!(m.len(), expected_len);
    }
}

// ---- Property 2: DAG liveness proptest ----
#[derive(Default)]
struct VNode {
    children: Vec<rc_hashmap::Ref<String, VNode>>, // DAG edges: i -> j
}

fn key(i: usize) -> String {
    format!("k{}", i)
}

proptest! {
    #[test]
    fn prop_dag_liveness(
        n in 1usize..=6,
        ops in proptest::collection::vec((0u8..=6u8, 0usize..64usize, 0usize..64usize), 1..128)
    ) {
        // Map under test and per-node external Refs.
        let mut m: RcHashMap<String, VNode> = RcHashMap::new();
        let mut live: Vec<Vec<rc_hashmap::Ref<String, VNode>>> = vec![Vec::new(); n];
        // Adjacency: edges i -> j are stored in values and keep j alive while i is alive.
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];

        fn closure(n: usize, roots: &[bool], adj: &[Vec<usize>]) -> Vec<bool> {
            let mut alive = roots.to_vec();
            let mut changed = true;
            while changed {
                changed = false;
                for i in 0..n {
                    if alive[i] {
                        for &j in &adj[i] {
                            if !alive[j] { alive[j] = true; changed = true; }
                        }
                    }
                }
            }
            alive
        }

        for (op, a, b) in ops.into_iter() {
            let i = a % n;
            // Choose j > i to avoid cycles; if no such j exists, skip edge ops.
            let j_opt = if i + 1 < n { Some(i + 1 + (b % (n - i - 1))) } else { None };
            let ki = key(i);
            match op {
                0 => {
                    match m.insert(ki.clone(), VNode { children: vec![] }) {
                        Ok(r) => live[i].push(r),
                        Err(rc_hashmap::InsertError::DuplicateKey) => {}
                    }
                }
                1 => { if let Some(r) = m.find(&ki) { live[i].push(r); } }
                2 => { if let Some(r) = live[i].pop() { let c = r.clone(); live[i].push(r); live[i].push(c); } }
                3 => { if let Some(r) = live[i].pop() { drop(r); } }
                4 => { while let Some(r) = live[i].pop() { drop(r); } }
                5 => {
                    if let Some(j) = j_opt {
                        let kj = key(j);
                        if let (Some(ri), Some(rj)) = (m.find(&ki), m.find(&kj)) {
                            if !adj[i].contains(&j) {
                                ri.value_mut(&mut m).unwrap().children.push(rj.clone());
                                adj[i].push(j);
                            }
                        }
                    }
                }
                6 => {
                    if let Some(ri) = m.find(&ki) {
                        if let Some(_child) = ri.value_mut(&mut m).unwrap().children.pop() {}
                        adj[i].pop();
                    }
                }
                _ => unreachable!()
            }

            // Prune adjacency to reflect removals: if i is removed, its outgoing
            // edges no longer exist (its value was dropped); if a child was removed,
            // drop edges to it to keep the model consistent.
            let present: Vec<bool> = (0..n).map(|t| m.contains_key(&key(t))).collect();
            for i in 0..n {
                if !present[i] { adj[i].clear(); } else { adj[i].retain(|&child| present[child]); }
            }

            // Model alive nodes as the transitive closure from nodes with any external Ref.
            let roots: Vec<bool> = (0..n).map(|t| !live[t].is_empty()).collect();
            let alive = closure(n, &roots, &adj);

            let expected_len = alive.iter().filter(|&&b| b).count();
            prop_assert_eq!(m.len(), expected_len);
            for (t, alive_t) in alive.iter().enumerate() {
                prop_assert_eq!(m.contains_key(&key(t)), *alive_t);
            }
        }

        // After dropping all external Refs, only nodes reachable from none remain (i.e., none).
        for v in &mut live { while let Some(r) = v.pop() { drop(r); } }
        let roots: Vec<bool> = (0..n).map(|_t| false).collect();
        let alive = closure(n, &roots, &adj);
        let expected_len = alive.iter().filter(|&&b| b).count();
        prop_assert_eq!(m.len(), expected_len);
    }
}
