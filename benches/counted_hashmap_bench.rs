use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use rc_hashmap::counted_hash_map::CountedHashMap;
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
    c.bench_function("counted::insert_fresh_100k", |b| {
        b.iter_batched(
            CountedHashMap::<String, u64>::new,
            |mut m| {
                let mut hs = Vec::with_capacity(100_000);
                for (i, x) in lcg(1).take(100_000).enumerate() {
                    hs.push(m.insert(key(x), i as u64).unwrap());
                }
                // Defer token return to after timing to avoid skewing insert cost
                black_box(ReturnTokensOnDrop { m, handles: hs })
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_insert_warm_100k(c: &mut Criterion) {
    c.bench_function("counted::insert_warm_100k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                // Grow to 110k then remove all to keep capacity
                let handles: Vec<_> = lcg(2)
                    .take(110_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                for h in handles { let _ = m.put(h); }
                m
            },
            |mut m| {
                let mut hs = Vec::with_capacity(100_000);
                for (i, x) in lcg(3).take(100_000).enumerate() {
                    hs.push(m.insert(key(x), i as u64).unwrap());
                }
                // Defer token return to after timing to avoid skewing insert cost
                black_box(ReturnTokensOnDrop { m, handles: hs })
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_remove_random_10k(c: &mut Criterion) {
    c.bench_function("counted::remove_random_10k_of_110k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let all: Vec<_> = lcg(5)
                    .take(110_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                // Precompute 10k unique indices via LCG
                let n = all.len();
                let mut sel = std::collections::HashSet::with_capacity(10_000);
                let mut s = 0x9e3779b97f4a7c15u64;
                while sel.len() < 10_000 {
                    s = s.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
                    sel.insert((s as usize) % n);
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
}

fn bench_find_hit_10k(c: &mut Criterion) {
    c.bench_function("counted::find_hit_10k_on_100k", |b| {
        let mut m = CountedHashMap::new();
        let keys: Vec<_> = lcg(7).take(100_000).map(key).collect();
        let held: Vec<_> = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| m.insert(k, i as u64).unwrap())
            .collect();
        // Keep tokens alive for the duration of the benchmark; avoid drop at end.
        core::mem::forget(held);
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
            for k in &queries { if let Some(h) = m.find(k) { let _ = m.put(h); } }
        })
    });
}

fn bench_find_miss_10k(c: &mut Criterion) {
    c.bench_function("counted::find_miss_10k_on_100k", |b| {
        let mut m = CountedHashMap::new();
        let held: Vec<_> = lcg(11)
            .take(100_000)
            .enumerate()
            .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
            .collect();
        core::mem::forget(held);
        let mut miss = lcg(0xdead_beef);
        b.iter(|| {
            for _ in 0..10_000 {
                let k = key(miss.next().unwrap());
                black_box(m.find(&k));
            }
        })
    });
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

fn bench_handle_access_increment(c: &mut Criterion) {
    c.bench_function("counted::handle_access_increment_10k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let handles: Vec<_> = lcg(123)
                    .take(100_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                // Precompute 10k random counted handles to touch by minting tokens via get()
                let n = handles.len();
                let mut s = 0x9e3779b97f4a7c15u64;
                let mut targets = Vec::with_capacity(10_000);
                for _ in 0..10_000 {
                    s = s.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
                    let idx = (s as usize) % n;
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
}

fn bench_iter_and_iter_mut(c: &mut Criterion) {
    c.bench_function("counted::iter_all_100k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let held: Vec<_> = lcg(999)
                    .take(100_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
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

    c.bench_function("counted::iter_mut_increment_all_100k", |b| {
        b.iter_batched(
            || {
                let mut m = CountedHashMap::new();
                let held: Vec<_> = lcg(1001)
                    .take(100_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
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
