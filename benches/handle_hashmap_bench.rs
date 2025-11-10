use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use rand_core::{RngCore, SeedableRng};
use rand_pcg::Lcg128Xsl64 as Pcg;
use rc_hashmap::handle_hash_map::{Handle, HandleHashMap};
use std::collections::HashSet;

fn key(n: u64) -> String {
    format!("k{:016x}", n)
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle::insert");
    group.throughput(Throughput::Elements(100_000));
    // fresh_100k
    group.bench_function("fresh_100k", |b| {
        b.iter_batched(
            HandleHashMap::<String, u64>::new,
            |mut m| {
                let mut rng = Pcg::seed_from_u64(1);
                for i in 0..100_000 {
                    let x = rng.next_u64();
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
    // warm_100k
    group.bench_function("warm_100k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                // Pre-grow and then clear by removing
                let mut handles: Vec<Handle> = Vec::with_capacity(110_000);
                let mut rng = Pcg::seed_from_u64(2);
                for i in 0..110_000 {
                    let x = rng.next_u64();
                    handles.push(m.insert(key(x), i as u64).unwrap());
                }
                for h in handles {
                    let _ = m.remove(h).unwrap();
                }
                m
            },
            |mut m| {
                let mut rng = Pcg::seed_from_u64(3);
                for i in 0..100_000 {
                    let x = rng.next_u64();
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
                black_box(m)
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_remove(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle::remove");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("random_10k_of_110k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                let mut rng = Pcg::seed_from_u64(5);
                let handles: Vec<Handle> = (0..110_000)
                    .map(|i| {
                        let x = rng.next_u64();
                        m.insert(key(x), i as u64).unwrap()
                    })
                    .collect();
                // Precompute 10k unique indices via PCG
                let n = handles.len();
                let mut sel = HashSet::with_capacity(10_000);
                let mut idx_rng = Pcg::seed_from_u64(0x9e3779b97f4a7c15);
                while sel.len() < 10_000 {
                    sel.insert((idx_rng.next_u64() as usize) % n);
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
    group.finish();
}

fn bench_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle::query");
    group.throughput(Throughput::Elements(10_000));
    // hit
    group.bench_function("hit_10k_on_100k", |b| {
        let mut m = HandleHashMap::new();
        let mut rng_keys = Pcg::seed_from_u64(7);
        let keys: Vec<_> = (0..100_000).map(|_| key(rng_keys.next_u64())).collect();
        for (i, k) in keys.iter().enumerate() {
            let _ = m.insert(k.clone(), i as u64).unwrap();
        }
        // Precompute 10k random query keys using PCG
        let n = keys.len();
        let mut rng_q = Pcg::seed_from_u64(0x9e3779b97f4a7c15);
        let queries: Vec<String> = (0..10_000)
            .map(|_| keys[(rng_q.next_u64() as usize) % n].clone())
            .collect();
        b.iter(|| {
            for k in &queries { black_box(m.find(k)); }
        })
    });
    // miss
    group.bench_function("miss_10k_on_100k", |b| {
        let mut m = HandleHashMap::new();
        let mut rng_ins = Pcg::seed_from_u64(11);
        for i in 0..100_000 {
            let _ = m.insert(key(rng_ins.next_u64()), i as u64).unwrap();
        }
        let mut miss = Pcg::seed_from_u64(0xdead_beefu64);
        b.iter(|| {
            for _ in 0..10_000 {
                let k = key(miss.next_u64());
                black_box(m.find(&k));
            }
        })
    });
    group.finish();
}

fn bench_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("handle::access");
    group.throughput(Throughput::Elements(100_000));
    // random access increment
    group.bench_function("random_increment_100k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                let mut rng = Pcg::seed_from_u64(123);
                let handles: Vec<_> = (0..100_000)
                    .map(|i| m.insert(key(rng.next_u64()), i as u64).unwrap())
                    .collect();
                // Precompute 10k random handles to touch
                let n = handles.len();
                let mut rsel = Pcg::seed_from_u64(0x9e3779b97f4a7c15);
                let targets: Vec<Handle> = (0..100_000)
                    .map(|_| handles[(rsel.next_u64() as usize) % n])
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
    // iter
    group.bench_function("iter_all_100k", |b| {
        let mut m = HandleHashMap::new();
        let mut rng = Pcg::seed_from_u64(999);
        for i in 0..100_000 {
            let _ = m.insert(key(rng.next_u64()), i as u64).unwrap();
        }
        b.iter(|| {
            let mut sum = 0u64;
            for (_h, _k, v) in m.iter() {
                sum = sum.wrapping_add(*v);
            }
            black_box(sum)
        })
    });
    // iter_mut
    group.bench_function("iter_mut_increment_all_100k", |b| {
        b.iter_batched(
            || {
                let mut m = HandleHashMap::new();
                let mut rng = Pcg::seed_from_u64(1001);
                for i in 0..100_000 {
                    let _ = m.insert(key(rng.next_u64()), i as u64).unwrap();
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
    group.finish();
}

fn bench_config() -> Criterion {
    Criterion::default()
}

criterion_group! {
    name = benches_handle_insert;
    config = bench_config();
    targets = bench_insert
}
criterion_group! {
    name = benches_handle_ops;
    config = bench_config();
    targets = bench_remove,
              bench_query,
              bench_access
}
criterion_main!(benches_handle_insert, benches_handle_ops);
