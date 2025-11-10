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

fn bench_insert_fresh_100k(c: &mut Criterion) {
    c.bench_function("rc::insert_fresh_100k", |b| {
        b.iter_batched(
            RcHashMap::<String, u64>::new,
            |mut m| {
                let mut refs = Vec::with_capacity(100_000);
                for (i, x) in lcg(1).take(100_000).enumerate() {
                    refs.push(m.insert(key(x), i as u64).unwrap());
                }
                black_box((m, refs))
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_insert_warm_100k(c: &mut Criterion) {
    c.bench_function("rc::insert_warm_100k", |b| {
        b.iter_batched(
            || {
                let mut m = RcHashMap::new();
                let refs: Vec<_> = lcg(2)
                    .take(110_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                drop(refs);
                m
            },
            |mut m| {
                let mut refs = Vec::with_capacity(100_000);
                for (i, x) in lcg(3).take(100_000).enumerate() {
                    refs.push(m.insert(key(x), i as u64).unwrap());
                }
                black_box((m, refs))
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_remove_random_10k(c: &mut Criterion) {
    c.bench_function("rc::remove_random_10k_of_110k", |b| {
        b.iter_batched(
            || {
                let mut m = RcHashMap::new();
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
                let mut to_drop = Vec::with_capacity(10_000);
                let mut remain = Vec::with_capacity(n - 10_000);
                for (i, r) in all.into_iter().enumerate() {
                    if sel.contains(&i) { to_drop.push(r); } else { remain.push(r); }
                }
                (m, to_drop, remain)
            },
            |(m, to_drop, remain)| {
                for r in to_drop { drop(r); }
                // Defer drop of remaining refs to after timing
                black_box((m, remain))
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_get_hit_10k(c: &mut Criterion) {
    c.bench_function("rc::get_hit_10k_on_100k", |b| {
        let mut m = RcHashMap::new();
        let keys: Vec<_> = lcg(7).take(100_000).map(key).collect();
        let _held: Vec<_> = keys
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, k)| m.insert(k, i as u64).unwrap())
            .collect();
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
            for k in &queries { let r = m.find(k).unwrap(); black_box(r); }
        })
    });
}

fn bench_get_miss_10k(c: &mut Criterion) {
    c.bench_function("rc::get_miss_10k_on_100k", |b| {
        let mut m = RcHashMap::new();
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
    c.bench_function("rc::handle_access_increment_10k", |b| {
        b.iter_batched(
            || {
                let mut m: RcHashMap<String, u64> = RcHashMap::new();
                let refs: Vec<_> = lcg(123)
                    .take(100_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                // Precompute 10k random refs to touch
                let n = refs.len();
                let mut s = 0x9e3779b97f4a7c15u64;
                let mut targets = Vec::with_capacity(10_000);
                // Select indices (allow duplicates; cheaper)
                for _ in 0..10_000 {
                    s = s.wrapping_mul(2862933555777941757).wrapping_add(3037000493);
                    let idx = (s as usize) % n;
                    targets.push(refs[idx].clone());
                }
                // Keep original refs alive as well to avoid removal; drop after timing
                let remain = refs;
                (m, targets, remain)
            },
            |(mut m, targets, remain)| {
                for r in &targets { let v = r.value_mut(&mut m).unwrap(); *v = v.wrapping_add(1); }
                // Return both map and refs so ref-drop occurs after timing
                black_box((m, targets, remain))
            },
            BatchSize::SmallInput,
        )
    });
}

fn bench_iter_and_iter_mut(c: &mut Criterion) {
    c.bench_function("rc::iter_all_100k", |b| {
        let mut m = RcHashMap::new();
        let _held: Vec<_> = lcg(999)
            .take(100_000)
            .enumerate()
            .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
            .collect();
        b.iter(|| {
            let mut cnt = 0usize;
            for _r in m.iter() {
                cnt += 1;
            }
            black_box(cnt)
        })
    });

    c.bench_function("rc::iter_mut_increment_all_100k", |b| {
        b.iter_batched(
            || {
                let mut m: RcHashMap<String, u64> = RcHashMap::new();
                let _held: Vec<_> = lcg(1001)
                    .take(100_000)
                    .enumerate()
                    .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                    .collect();
                m
            },
            |mut m| {
                for mut item in m.iter_mut() {
                    let v = item.value_mut();
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
              bench_get_hit_10k,
              bench_get_miss_10k,
              bench_handle_access_increment,
              bench_iter_and_iter_mut
}
criterion_main!(benches_insert, benches_ops);
