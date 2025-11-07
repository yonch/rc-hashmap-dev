#[cfg(target_os = "linux")]
mod bench {
    use iai::black_box;
    use rc_hashmap::handle_hash_map::{Handle, HandleHashMap};
    use std::cell::RefCell;
    use std::thread_local;

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

    // Thread-local single-run setup to avoid measuring initialization.
    thread_local! {
        static HIT_MAP: RefCell<Option<HandleHashMap<String, u64>>> = RefCell::new(None);
        static HIT_KEYS: RefCell<Option<Vec<String>>> = RefCell::new(None);

        static MISS_MAP: RefCell<Option<HandleHashMap<String, u64>>> = RefCell::new(None);

        static REMOVE_MAP: RefCell<Option<HandleHashMap<String, u64>>> = RefCell::new(None);
        static REMOVE_HANDLES: RefCell<Option<Vec<Handle>>> = RefCell::new(None);

        // For increment via handle and iter_mut increment
        static INCR_MAP: RefCell<Option<HandleHashMap<String, u64>>> = RefCell::new(None);
        static INCR_HANDLES: RefCell<Option<Vec<Handle>>> = RefCell::new(None);

        static ITER_MUT_MAP: RefCell<Option<HandleHashMap<String, u64>>> = RefCell::new(None);
    }

    fn ensure_hit_setup() {
        HIT_MAP.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(HandleHashMap::new());
            }
        });
        HIT_KEYS.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(lcg(7).take(OPS * 2).map(key).collect());
            }
        });
        // Populate map and keep keys vector for cycling
        HIT_MAP.with(|m_cell| {
            HIT_KEYS.with(|k_cell| {
                let mut m_b = m_cell.borrow_mut();
                let m = m_b.as_mut().unwrap();
                if m.len() == 0 {
                    for (i, k) in k_cell
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .iter()
                        .cloned()
                        .enumerate()
                    {
                        let _ = m.insert(k, i as u64).unwrap();
                    }
                }
            })
        });
    }

    fn ensure_miss_setup() {
        MISS_MAP.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(HandleHashMap::new());
            }
            let mut m_b = c.borrow_mut();
            let m = m_b.as_mut().unwrap();
            if m.len() == 0 {
                for (i, x) in lcg(11).take(OPS).enumerate() {
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
            }
        });
    }

    fn ensure_remove_setup() {
        REMOVE_MAP.with(|m_cell| {
            if m_cell.borrow().is_none() {
                *m_cell.borrow_mut() = Some(HandleHashMap::new());
            }
        });
        REMOVE_HANDLES.with(|h_cell| {
            if h_cell.borrow().is_none() {
                REMOVE_MAP.with(|m_cell| {
                    let mut m_b = m_cell.borrow_mut();
                    let m = m_b.as_mut().unwrap();
                    // Pre-insert OPS items and keep their handles for removal bench
                    let hs: Vec<_> = (0..OPS)
                        .map(|i| m.insert(format!("k{:04}", i), i as u64).unwrap())
                        .collect();
                    *h_cell.borrow_mut() = Some(hs);
                })
            }
        });
    }

    fn ensure_incr_setup() {
        INCR_MAP.with(|m_cell| {
            if m_cell.borrow().is_none() {
                *m_cell.borrow_mut() = Some(HandleHashMap::new());
            }
        });
        INCR_HANDLES.with(|h_cell| {
            if h_cell.borrow().is_none() {
                INCR_MAP.with(|m_cell| {
                    let mut m_b = m_cell.borrow_mut();
                    let m = m_b.as_mut().unwrap();
                    let hs: Vec<_> = lcg(123)
                        .take(OPS)
                        .enumerate()
                        .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                        .collect();
                    *h_cell.borrow_mut() = Some(hs);
                })
            }
        });
    }

    fn ensure_iter_mut_setup() {
        ITER_MUT_MAP.with(|m_cell| {
            if m_cell.borrow().is_none() {
                *m_cell.borrow_mut() = Some(HandleHashMap::new());
            }
            let mut m_b = m_cell.borrow_mut();
            let m = m_b.as_mut().unwrap();
            if m.len() == 0 {
                for (i, x) in lcg(999).take(OPS).enumerate() {
                    let _ = m.insert(key(x), i as u64).unwrap();
                }
            }
        });
    }

    pub fn __handle_hashmap_iai_setup() {
        ensure_hit_setup();
        ensure_miss_setup();
        ensure_remove_setup();
        ensure_incr_setup();
        ensure_iter_mut_setup();
        black_box(())
    }

    // Insert 1k entries.
    pub fn handle_hashmap_insert_1000_ops() {
        let mut m = HandleHashMap::<String, u64>::new();
        for (i, x) in lcg(1).take(OPS).enumerate() {
            let _ = m.insert(key(x), i as u64).unwrap();
        }
        black_box(m);
    }

    // Repeated hits on existing keys; setup pre-initialized.
    pub fn handle_hashmap_find_hit_1000_ops() {
        HIT_MAP.with(|m_cell| {
            HIT_KEYS.with(|k_cell| {
                let m_b = m_cell.borrow();
                let m = m_b.as_ref().expect("setup not initialized");
                let keys_b = k_cell.borrow();
                let keys = keys_b.as_ref().expect("setup not initialized");
                let mut it = keys.iter().cycle();
                for _ in 0..OPS {
                    let k = it.next().unwrap();
                    black_box(m.find(k));
                }
            })
        })
    }

    // Repeated misses for keys unlikely to be present; setup pre-initialized.
    pub fn handle_hashmap_find_miss_1000_ops() {
        MISS_MAP.with(|m_cell| {
            let m_b = m_cell.borrow();
            let m = m_b.as_ref().expect("setup not initialized");
            let mut miss = lcg(0xdead_beef);
            for _ in 0..OPS {
                let k = key(miss.next().unwrap());
                black_box(m.find(&k));
            }
        })
    }

    // Remove by handle repeatedly; handles and map pre-initialized.
    pub fn handle_hashmap_remove_by_handle_1000_ops() {
        REMOVE_MAP.with(|m_cell| {
            REMOVE_HANDLES.with(|h_cell| {
                let mut m_b = m_cell.borrow_mut();
                let m = m_b.as_mut().expect("setup not initialized");
                let mut hs_b = h_cell.borrow_mut();
                let hs = hs_b.as_mut().expect("setup not initialized");
                for h in hs.drain(..) {
                    black_box(m.remove(h));
                }
            })
        })
    }

    // Cycle and increment values via stored handles.
    pub fn handle_hashmap_handle_increment_1000_ops() {
        INCR_MAP.with(|m_cell| {
            INCR_HANDLES.with(|h_cell| {
                let mut m_b = m_cell.borrow_mut();
                let m = m_b.as_mut().expect("setup not initialized");
                let hs_b = h_cell.borrow();
                let hs = hs_b.as_ref().expect("setup not initialized");
                let mut idx = 0usize;
                for _ in 0..OPS {
                    let h = hs[idx];
                    if let Some(v) = h.value_mut(m) {
                        *v = v.wrapping_add(1);
                    }
                    idx += 1;
                    if idx == hs.len() {
                        idx = 0;
                    }
                }
                black_box(m.len());
            })
        })
    }

    // Iterate mutably and increment each value.
    pub fn handle_hashmap_iter_mut_increment_1000_ops() {
        ITER_MUT_MAP.with(|m_cell| {
            let mut m_b = m_cell.borrow_mut();
            let m = m_b.as_mut().expect("setup not initialized");
            for (_h, _k, v) in m.iter_mut() {
                *v = v.wrapping_add(1);
            }
            black_box(m.len());
        })
    }
}

#[cfg(target_os = "linux")]
use bench::{
    __handle_hashmap_iai_setup, handle_hashmap_find_hit_1000_ops,
    handle_hashmap_find_miss_1000_ops, handle_hashmap_handle_increment_1000_ops,
    handle_hashmap_insert_1000_ops, handle_hashmap_iter_mut_increment_1000_ops,
    handle_hashmap_remove_by_handle_1000_ops,
};

// Custom harness: run setup before invoking iai::runner so calibration subtracts it.
#[cfg(target_os = "linux")]
mod __iai_custom_harness {
    use super::*;

    mod wrappers {
        use super::*;
        pub fn handle_hashmap_insert_1000_ops() {
            let _ = iai::black_box(bench::handle_hashmap_insert_1000_ops());
        }
        pub fn handle_hashmap_find_hit_1000_ops() {
            let _ = iai::black_box(bench::handle_hashmap_find_hit_1000_ops());
        }
        pub fn handle_hashmap_find_miss_1000_ops() {
            let _ = iai::black_box(bench::handle_hashmap_find_miss_1000_ops());
        }
        pub fn handle_hashmap_remove_by_handle_1000_ops() {
            let _ = iai::black_box(bench::handle_hashmap_remove_by_handle_1000_ops());
        }
        pub fn handle_hashmap_handle_increment_1000_ops() {
            let _ = iai::black_box(bench::handle_hashmap_handle_increment_1000_ops());
        }
        pub fn handle_hashmap_iter_mut_increment_1000_ops() {
            let _ = iai::black_box(bench::handle_hashmap_iter_mut_increment_1000_ops());
        }
    }

    pub fn main() {
        __handle_hashmap_iai_setup();
        let benches: &[&(&'static str, fn())] = &[
            &(
                "handle_hashmap_insert_1000_ops",
                wrappers::handle_hashmap_insert_1000_ops,
            ),
            &(
                "handle_hashmap_find_hit_1000_ops",
                wrappers::handle_hashmap_find_hit_1000_ops,
            ),
            &(
                "handle_hashmap_find_miss_1000_ops",
                wrappers::handle_hashmap_find_miss_1000_ops,
            ),
            &(
                "handle_hashmap_remove_by_handle_1000_ops",
                wrappers::handle_hashmap_remove_by_handle_1000_ops,
            ),
            &(
                "handle_hashmap_handle_increment_1000_ops",
                wrappers::handle_hashmap_handle_increment_1000_ops,
            ),
            &(
                "handle_hashmap_iter_mut_increment_1000_ops",
                wrappers::handle_hashmap_iter_mut_increment_1000_ops,
            ),
        ];
        iai::runner(benches);
    }
}

#[cfg(target_os = "linux")]
fn main() {
    __iai_custom_harness::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Skipping: iai benches require Linux/valgrind.");
}
