use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion, Throughput};
use rand_core::{RngCore, SeedableRng};
use rand_pcg::Lcg128Xsl64 as Pcg;
use rc_hashmap::counted_hash_map::CountedHashMap;
use std::collections::HashSet;

fn key(n: u64) -> String {
    format!("k{:016x}", n)
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("counted::insert");
    group.throughput(Throughput::Elements(100_000));
    // fresh_100k
    group.bench_function("fresh_100k", |b| {
        b.iter_batched(
            CountedHashMap::<String, u64>::new,
            |mut m| {
                let mut rng = Pcg::seed_from_u64(1);
                let mut hs = Vec::with_capacity(100_000);
                for i in 0..100_000 {
                    hs.push(m.insert(key(rng.next_u64()), i as u64).unwrap());
                }
                // Defer token return to after timing to avoid skewing insert cost
                black_box(ReturnTokensOnDrop { m, handles: hs })
            },
            BatchSize::SmallInput,
        )
    });
    // warm_100k
    group.bench_function("warm_100k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                // Grow to 110k then remove all to keep capacity
                let mut rng = Pcg::seed_from_u64(2);
                let handles: Vec<_> = (0..110_000)
                    .map(|i| m.insert(key(rng.next_u64()), i as u64).unwrap())
                    .collect();
                for h in handles { let _ = m.put(h); }
                m
            },
            |mut m| {
                let mut rng = Pcg::seed_from_u64(3);
                let mut hs = Vec::with_capacity(100_000);
                for i in 0..100_000 {
                    hs.push(m.insert(key(rng.next_u64()), i as u64).unwrap());
                }
                // Defer token return to after timing to avoid skewing insert cost
                black_box(ReturnTokensOnDrop { m, handles: hs })
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_remove(c: &mut Criterion) {
    let mut group = c.benchmark_group("counted::remove");
    group.throughput(Throughput::Elements(10_000));
    group.bench_function("random_10k_of_110k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let mut rng = Pcg::seed_from_u64(5);
                let all: Vec<_> = (0..110_000)
                    .map(|i| m.insert(key(rng.next_u64()), i as u64).unwrap())
                    .collect();
                // Precompute 10k unique indices via PCG
                let n = all.len();
                let mut sel = HashSet::with_capacity(10_000);
                let mut idx_rng = Pcg::seed_from_u64(0x9e3779b97f4a7c15);
                while sel.len() < 10_000 {
                    sel.insert((idx_rng.next_u64() as usize) % n);
                }
                let mut to_remove = Vec::with_capacity(10_000);
                let mut remain = Vec::with_capacity(n - 10_000);
                for (i, h) in all.into_iter().enumerate() {
                    if sel.contains(&i) { to_remove.push(h); } else { remain.push(h); }
                }
                (m, to_remove, remain)
            },
            |(mut m, to_remove, remain)| {
                for h in to_remove { let _ = m.put(h); }
                // Defer return of remaining tokens to after timing
                black_box(ReturnTokensOnDrop { m, handles: remain })
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

fn bench_query(c: &mut Criterion) {
    let mut group = c.benchmark_group("counted::query");
    group.throughput(Throughput::Elements(10_000));
    // hit
    group.bench_function("hit_10k_on_100k", |b| {
        let mut m = CountedHashMap::new();
        let mut rng_keys = Pcg::seed_from_u64(7);
        let keys: Vec<_> = (0..100_000).map(|_| key(rng_keys.next_u64())).collect();
        let held: Vec<_> = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| m.insert(k, i as u64).unwrap())
            .collect();
        // Keep tokens alive for the duration of the benchmark; avoid drop at end.
        core::mem::forget(held);
        // Precompute 10k random query keys using PCG
        let n = keys.len();
        let mut rng_q = Pcg::seed_from_u64(0x9e3779b97f4a7c15);
        let queries: Vec<String> = (0..10_000)
            .map(|_| keys[(rng_q.next_u64() as usize) % n].clone())
            .collect();
        b.iter(|| {
            for k in &queries { if let Some(h) = m.find(k) { let _ = m.put(h); } }
        })
    });
    // miss
    group.bench_function("miss_10k_on_100k", |b| {
        let mut m = CountedHashMap::new();
        let mut rng_ins = Pcg::seed_from_u64(11);
        let held: Vec<_> = (0..100_000)
            .map(|i| m.insert(key(rng_ins.next_u64()), i as u64).unwrap())
            .collect();
        core::mem::forget(held);
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

// Guard that returns tokens to the map on drop, so cleanup happens
// outside the measured closure time in `iter_batched`.
struct ReturnTokensOnDrop {
    m: CountedHashMap<String, u64>,
    handles: Vec<rc_hashmap::counted_hash_map::CountedHandle<'static>>,
}
impl Drop for ReturnTokensOnDrop {
    fn drop(&mut self) {
        for h in self.handles.drain(..) { let _ = self.m.put(h); }
    }
}


// Guard that holds separate sets of tokens and returns all on drop
struct CountedAccessGuard {
    m: CountedHashMap<String, u64>,
    a: Vec<rc_hashmap::counted_hash_map::CountedHandle<'static>>,
    b: Vec<rc_hashmap::counted_hash_map::CountedHandle<'static>>,
}
impl Drop for CountedAccessGuard {
    fn drop(&mut self) {
        for h in self.a.drain(..) { let _ = self.m.put(h); }
        for h in self.b.drain(..) { let _ = self.m.put(h); }
    }
}

fn bench_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("counted::access");
    group.throughput(Throughput::Elements(100_000));
    // random access increment
    group.bench_function("random_increment_100k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let mut rng = Pcg::seed_from_u64(123);
                let handles: Vec<_> = (0..100_000)
                    .map(|i| m.insert(key(rng.next_u64()), i as u64).unwrap())
                    .collect();
                // Precompute 10k random counted handles to touch by minting tokens via get()
                let n = handles.len();
                let mut rsel = Pcg::seed_from_u64(0x9e3779b97f4a7c15);
                let mut targets = Vec::with_capacity(10_000);
                for _ in 0..100_000 {
                    let idx = (rsel.next_u64() as usize) % n;
                    let h = m.get(&handles[idx]);
                    targets.push(h);
                }
                (m, targets, handles)
            },
            |(mut m, targets, handles)| {
                for h in &targets { if let Some(v) = h.value_mut(&mut m) { *v = v.wrapping_add(1); } }
                // Return all tokens after timing
                black_box(CountedAccessGuard { m, a: targets, b: handles })
            },
            BatchSize::SmallInput,
        )
    });
    // iter
    group.bench_function("iter_all_100k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let mut rng = Pcg::seed_from_u64(999);
                let held: Vec<_> = (0..100_000)
                    .map(|i| m.insert(key(rng.next_u64()), i as u64).unwrap())
                    .collect();
                core::mem::forget(held);
                m
            },
            |m| {
                let mut sum = 0u64;
                for (_h, _k, v) in m.iter() { sum = sum.wrapping_add(*v); }
                black_box(sum)
            },
            BatchSize::SmallInput,
        )
    });
    // iter_mut
    group.bench_function("iter_mut_increment_all_100k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let mut rng = Pcg::seed_from_u64(1001);
                let held: Vec<_> = (0..100_000)
                    .map(|i| m.insert(key(rng.next_u64()), i as u64).unwrap())
                    .collect();
                core::mem::forget(held);
                m
            },
            |mut m| {
                for (_h, _k, v) in m.iter_mut() { *v = v.wrapping_add(1); }
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
    name = benches_counted_insert;
    config = bench_config();
    targets = bench_insert
}
criterion_group! {
    name = benches_counted_ops;
    config = bench_config();
    targets = bench_remove,
              bench_query,
              bench_access
}
criterion_main!(benches_counted_insert, benches_counted_ops);
