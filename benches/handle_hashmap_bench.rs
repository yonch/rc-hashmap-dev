#![cfg(feature = "bench_internal")]

use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use rc_hashmap::HandleHashMap;
use std::time::Duration; // exposed when feature bench_internal is enabled

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
    c.bench_function("handle_hashmap_insert_10k", |b| {
        b.iter_batched(
            || HandleHashMap::<String, u64>::new(),
            |mut m| {
                for (i, x) in lcg(1).take(10_000).enumerate() {
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_find_hit(c: &mut Criterion) {
    c.bench_function("handle_hashmap_find_hit", |b| {
        let mut m = HandleHashMap::new();
        let keys: Vec<_> = lcg(7).take(20_000).map(key).collect();
        for (i, k) in keys.iter().enumerate() {
            let _ = m.insert(k.clone(), i as u64).unwrap();
        }
        let mut it = keys.iter().cycle();
        b.iter(|| {
            let k = it.next().unwrap();
            black_box(m.find(k));
        })
    });
}

fn bench_find_miss(c: &mut Criterion) {
    c.bench_function("handle_hashmap_find_miss", |b| {
        let mut m = HandleHashMap::new();
        for (i, x) in lcg(11).take(10_000).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        let mut miss = lcg(0xdead_beef);
        b.iter(|| {
            let k = key(miss.next().unwrap());
            black_box(m.find(&k));
        })
    });
}

fn bench_remove_by_handle(c: &mut Criterion) {
    c.bench_function("handle_hashmap_remove_by_handle", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                let h = m.insert("k".to_string(), 1u64).unwrap();
                (m, h)
            },
            |(mut m, h)| {
                black_box(m.remove(h));
            },
            BatchSize::SmallInput,
        )
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
    targets = bench_insert, bench_find_hit, bench_find_miss, bench_remove_by_handle
}
criterion_main!(benches);
