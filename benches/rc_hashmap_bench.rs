use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use rc_hashmap::RcHashMap;
use std::time::Duration;

fn lcg(mut s: u64) -> impl Iterator<Item = u64> {
    std::iter::from_fn(move || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        Some(s)
    })
}

fn key(n: u64) -> String {
    format!("k{:016x}", n)
}

fn bench_insert(c: &mut Criterion) {
    c.bench_function("rc_hashmap_insert_10k", |b| {
        b.iter_batched(
            || RcHashMap::<String, u64>::new(),
            |m| {
                // Hold refs to avoid immediate removals during insert loop.
                let mut refs = Vec::with_capacity(10_000);
                for (i, x) in lcg(1).take(10_000).enumerate() {
                    refs.push(m.insert(key(x), i as u64).unwrap());
                }
                black_box((m, refs))
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_get_hit(c: &mut Criterion) {
    c.bench_function("rc_hashmap_get_hit", |b| {
        let m = RcHashMap::new();
        let keys: Vec<_> = lcg(7).take(20_000).map(key).collect();
        // Keep the inserted refs alive to ensure entries remain in the map.
        let _held: Vec<_> = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| m.insert(k, i as u64).unwrap())
            .collect();
        let mut it = keys.iter().cycle();
        b.iter(|| {
            let k = it.next().unwrap();
            let r = m.get(k).unwrap();
            black_box(r);
        })
    });
}

fn bench_get_miss(c: &mut Criterion) {
    c.bench_function("rc_hashmap_get_miss", |b| {
        let m = RcHashMap::new();
        for (i, x) in lcg(11).take(10_000).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        let mut miss = lcg(0xdead_beef);
        b.iter(|| {
            // generate keys unlikely in map
            let k = key(miss.next().unwrap());
            black_box(m.get(&k));
        })
    });
}

fn bench_clone_drop_refs(c: &mut Criterion) {
    c.bench_function("rc_hashmap_clone_drop_ref", |b| {
        let m = RcHashMap::new();
        let r = m.insert("key".to_string(), 1u64).unwrap();
        b.iter(|| {
            let x = r.clone();
            black_box(&x);
            drop(x);
        })
    });
}

fn bench_config() -> Criterion {
    Criterion::default()
        .sample_size(50)
        .measurement_time(Duration::from_secs(8))
        .warm_up_time(Duration::from_secs(2))
}

criterion_group! {
    name = benches;
    config = bench_config();
    targets = bench_insert, bench_get_hit, bench_get_miss, bench_clone_drop_refs
}
criterion_main!(benches);
