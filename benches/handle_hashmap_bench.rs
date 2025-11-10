use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use rc_hashmap::handle_hash_map::{Handle, HandleHashMap};
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

fn bench_insert_fresh_100k(c: &mut Criterion) {
    c.bench_function("handle::insert_fresh_100k", |b| {
        b.iter_batched(
            HandleHashMap::<String, u64>::new,
            |mut m| {
                for (i, x) in lcg(1).take(100_000).enumerate() {
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_insert_warm_100k(c: &mut Criterion) {
    c.bench_function("handle::insert_warm_100k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                // Pre-grow and then clear by removing
                let mut handles: Vec<Handle> = Vec::with_capacity(110_000);
                for (i, x) in lcg(2).take(110_000).enumerate() {
                    handles.push(m.insert(key(x), i as u64).unwrap());
                }
                for h in handles {
                    let _ = m.remove(h).unwrap();
                }
                m
            },
            |mut m| {
                for (i, x) in lcg(3).take(100_000).enumerate() {
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_remove_random_10k(c: &mut Criterion) {
    c.bench_function("handle::remove_random_10k_of_110k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                let handles: Vec<Handle> = lcg(5)
                    .take(110_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                // Precompute 10k unique indices via LCG
                let n = handles.len();
                let mut sel = std::collections::HashSet::with_capacity(10_000);
                let mut s = 0x9e3779b97f4a7c15u64;
                while sel.len() < 10_000 {
                    s = s.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
                    sel.insert((s as usize) % n);
                }
                let to_remove: Vec<Handle> = sel.into_iter().map(|i| handles[i]).collect();
                (m, to_remove)
            },
            |(mut m, to_remove)| {
                for h in to_remove { let _ = m.remove(h); }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_find_hit_10k(c: &mut Criterion) {
    c.bench_function("handle::find_hit_10k_on_100k", |b| {
        let mut m = HandleHashMap::new();
        let keys: Vec<_> = lcg(7).take(100_000).map(key).collect();
        for (i, k) in keys.iter().enumerate() {
            let _ = m.insert(k.clone(), i as u64).unwrap();
        }
        // Precompute 10k random query keys using LCG
        let n = keys.len();
        let mut s = 0x9e3779b97f4a7c15u64;
        let queries: Vec<String> = (0..10_000)
            .map(|_| {
                s = s.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
                keys[(s as usize) % n].clone()
            })
            .collect();
        b.iter(|| {
            for k in &queries { black_box(m.find(k)); }
        })
    });
}

fn bench_find_miss_10k(c: &mut Criterion) {
    c.bench_function("handle::find_miss_10k_on_100k", |b| {
        let mut m = HandleHashMap::new();
        for (i, x) in lcg(11).take(100_000).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        let mut miss = lcg(0xdead_beef);
        b.iter(|| {
            for _ in 0..10_000 {
                let k = key(miss.next().unwrap());
                black_box(m.find(&k));
            }
        })
    });
}

fn bench_handle_access_increment(c: &mut Criterion) {
    c.bench_function("handle::handle_access_increment_10k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                let handles: Vec<_> = lcg(123)
                    .take(100_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                // Precompute 10k random handles to touch
                let n = handles.len();
                let mut s = 0x9e3779b97f4a7c15u64;
                let targets: Vec<Handle> = (0..10_000)
                    .map(|_| {
                        s = s.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
                        handles[(s as usize) % n]
                    })
                    .collect();
                (m, targets)
            },
            |(mut m, targets)| {
                for h in targets {
                    if let Some(v) = h.value_mut(&mut m) { *v = v.wrapping_add(1); }
                }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_iter_and_iter_mut(c: &mut Criterion) {
    c.bench_function("handle::iter_all_100k", |b| {
        let mut m = HandleHashMap::new();
        for (i, x) in lcg(999).take(100_000).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        b.iter(|| {
            let mut sum = 0u64;
            for (_h, _k, v) in m.iter() {
                sum = sum.wrapping_add(*v);
            }
            black_box(sum)
        })
    });

    c.bench_function("handle::iter_mut_increment_all_100k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                for (i, x) in lcg(1001).take(100_000).enumerate() {
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
                m
            },
            |mut m| {
                for (_h, _k, v) in m.iter_mut() {
                    *v = v.wrapping_add(1);
                }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_config() -> Criterion {
    Criterion::default()
        .sample_size(12)
        .measurement_time(Duration::from_secs(5))
        .warm_up_time(Duration::from_secs(1))
}

criterion_group! {
    name = benches_insert;
    config = bench_config();
    targets = bench_insert_fresh_100k, bench_insert_warm_100k
}
criterion_group! {
    name = benches_ops;
    config = bench_config();
    targets = bench_remove_random_10k,
              bench_find_hit_10k,
              bench_find_miss_10k,
              bench_handle_access_increment,
              bench_iter_and_iter_mut
}
criterion_main!(benches_insert, benches_ops);
