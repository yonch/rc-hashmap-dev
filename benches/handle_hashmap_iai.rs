#[cfg(target_os = "linux")]
mod bench {
    use iai::black_box;
    use rc_hashmap::handle_hash_map::HandleHashMap;

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

    // Insert 1k entries.
    pub fn handle_hashmap_insert_1000_ops() {
        let mut m = HandleHashMap::<String, u64>::new();
        for (i, x) in lcg(1).take(OPS).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        black_box(m);
    }

    // Repeated hits on existing keys.
    pub fn handle_hashmap_find_hit_1000_ops() {
        let mut m = HandleHashMap::new();
        let keys: Vec<_> = lcg(7).take(OPS * 2).map(key).collect();
        for (i, k) in keys.iter().enumerate() {
            let _ = m.insert(k.clone(), i as u64).unwrap();
        }
        let mut it = keys.iter().cycle();
        for _ in 0..OPS {
            let k = it.next().unwrap();
            black_box(m.find(k));
        }
    }

    // Repeated misses for keys unlikely to be present.
    pub fn handle_hashmap_find_miss_1000_ops() {
        let mut m = HandleHashMap::new();
        for (i, x) in lcg(11).take(OPS).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        let mut miss = lcg(0xdead_beef);
        for _ in 0..OPS {
            let k = key(miss.next().unwrap());
            black_box(m.find(&k));
        }
    }

    // Remove by handle repeatedly.
    pub fn handle_hashmap_remove_by_handle_1000_ops() {
        let mut m = HandleHashMap::new();
        for _ in 0..OPS {
            let h = m.insert("k".to_string(), 1u64).unwrap();
            black_box(m.remove(h));
        }
    }
}

#[cfg(target_os = "linux")]
use bench::{
    handle_hashmap_find_hit_1000_ops, handle_hashmap_find_miss_1000_ops,
    handle_hashmap_insert_1000_ops, handle_hashmap_remove_by_handle_1000_ops,
};

#[cfg(target_os = "linux")]
iai::main!(
    handle_hashmap_insert_1000_ops,
    handle_hashmap_find_hit_1000_ops,
    handle_hashmap_find_miss_1000_ops,
    handle_hashmap_remove_by_handle_1000_ops
);

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Skipping: iai benches require Linux/valgrind.");
}
