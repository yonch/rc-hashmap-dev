#[cfg(target_os = "linux")]
mod bench {
    use iai::black_box;
    use rc_hashmap::RcHashMap;

    const OPS: usize = 1_000;

    fn lcg(mut s: u64) -> impl Iterator<Item = u64> {
        std::iter::from_fn(move || {
            s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
            Some(s)
        })
    }

    fn key(n: u64) -> String {
        format!("k{:016x}", n)
    }

    // Insert 1k entries holding refs to avoid immediate removals.
    pub fn rc_hashmap_insert_1000_ops() {
        let mut m = RcHashMap::<String, u64>::new();
        let mut refs = Vec::with_capacity(OPS);
        for (i, x) in lcg(1).take(OPS).enumerate() {
            refs.push(m.insert(key(x), i as u64).unwrap());
        }
        black_box((m.len(), refs.len()));
    }

    // Repeated hits on existing keys.
    pub fn rc_hashmap_get_hit_1000_ops() {
        let mut m = RcHashMap::new();
        let keys: Vec<_> = lcg(7).take(OPS * 2).map(key).collect();
        // Keep the inserted refs alive so entries remain in the map.
        let _held: Vec<_> = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| m.insert(k, i as u64).unwrap())
            .collect();

        // Perform 1k successful lookups.
        let mut it = keys.iter().cycle();
        for _ in 0..OPS {
            let k = it.next().unwrap();
            let r = m.find(k).unwrap();
            black_box(&r);
        }
    }

    // Repeated misses for keys unlikely to be present.
    pub fn rc_hashmap_get_miss_1000_ops() {
        let mut m = RcHashMap::new();
        for (i, x) in lcg(11).take(OPS).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        let mut miss = lcg(0xdead_beef);
        for _ in 0..OPS {
            let k = key(miss.next().unwrap());
            black_box(m.find(&k));
        }
    }

    // Clone and drop a Ref repeatedly.
    pub fn rc_hashmap_clone_drop_ref_1000_ops() {
        let mut m = RcHashMap::new();
        let r = m.insert("key".to_string(), 1u64).unwrap();
        for _ in 0..OPS {
            let x = r.clone();
            black_box(&x);
            drop(x);
        }
    }

    // Insert 1k entries, keep Refs, then cycle and increment each value via handle.
    pub fn rc_hashmap_ref_increment_1000_ops() {
        let mut m: RcHashMap<String, u64> = RcHashMap::new();
        let mut refs = Vec::with_capacity(OPS);
        for (i, x) in lcg(123).take(OPS).enumerate() {
            refs.push(m.insert(key(x), i as u64).unwrap());
        }
        // Perform 1k increments distributed over the refs.
        let mut idx = 0usize;
        for _ in 0..OPS {
            let r = &refs[idx];
            let v = r.value_mut(&mut m).unwrap();
            *v = v.wrapping_add(1);
            idx += 1;
            if idx == refs.len() {
                idx = 0;
            }
        }
        black_box(m.len());
    }

    // Insert 1k entries and iterate mutably, incrementing each value.
    pub fn rc_hashmap_iter_mut_increment_1000_ops() {
        let mut m: RcHashMap<String, u64> = RcHashMap::new();
        for (i, x) in lcg(999).take(OPS).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        for mut item in m.iter_mut() {
            let v = item.value_mut();
            *v = v.wrapping_add(1);
        }
        black_box(m.len());
    }
}

#[cfg(target_os = "linux")]
use bench::{
    rc_hashmap_clone_drop_ref_1000_ops, rc_hashmap_get_hit_1000_ops, rc_hashmap_get_miss_1000_ops,
    rc_hashmap_insert_1000_ops, rc_hashmap_iter_mut_increment_1000_ops,
    rc_hashmap_ref_increment_1000_ops,
};

#[cfg(target_os = "linux")]
iai::main!(
    rc_hashmap_insert_1000_ops,
    rc_hashmap_get_hit_1000_ops,
    rc_hashmap_get_miss_1000_ops,
    rc_hashmap_clone_drop_ref_1000_ops,
    rc_hashmap_ref_increment_1000_ops,
    rc_hashmap_iter_mut_increment_1000_ops
);

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Skipping: iai benches require Linux/valgrind.");
}
