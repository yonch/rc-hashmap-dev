#[cfg(target_os = "linux")]
mod bench {
    use iai::black_box;
    use rc_hashmap::{RcHashMap, Ref};
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

    // Global one-time setups to avoid measuring initialization in the benches below.
    // These are initialized by a dedicated setup bench placed first in iai::main!.
    thread_local! {
        static GET_HIT_MAP: RefCell<Option<RcHashMap<String, u64>>> = RefCell::new(None);
        static GET_HIT_KEYS: RefCell<Option<Vec<String>>> = RefCell::new(None);
        static GET_HIT_HELD: RefCell<Option<Vec<Ref<String, u64>>>> = RefCell::new(None);

        static GET_MISS_MAP: RefCell<Option<RcHashMap<String, u64>>> = RefCell::new(None);
        static GET_MISS_HELD: RefCell<Option<Vec<Ref<String, u64>>>> = RefCell::new(None);

        static CLONE_DROP_MAP: RefCell<Option<RcHashMap<String, u64>>> = RefCell::new(None);
        static CLONE_DROP_REF: RefCell<Option<Ref<String, u64>>> = RefCell::new(None);

        static REF_INCR_MAP: RefCell<Option<RcHashMap<String, u64>>> = RefCell::new(None);
        static REF_INCR_REFS: RefCell<Option<Vec<Ref<String, u64>>>> = RefCell::new(None);

        static ITER_MUT_MAP: RefCell<Option<RcHashMap<String, u64>>> = RefCell::new(None);
        static ITER_MUT_REFS: RefCell<Option<Vec<Ref<String, u64>>>> = RefCell::new(None);

        // For measuring drop cost of the last Ref to each entry
        static DROP_MAP: RefCell<Option<RcHashMap<String, u64>>> = RefCell::new(None);
        static DROP_REFS: RefCell<Option<Vec<Ref<String, u64>>>> = RefCell::new(None);
    }

    fn ensure_get_hit_setup() {
        GET_HIT_MAP.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(RcHashMap::new());
            }
        });
        GET_HIT_KEYS.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(lcg(7).take(OPS * 2).map(key).collect());
            }
        });
        GET_HIT_HELD.with(|held| {
            if held.borrow().is_none() {
                GET_HIT_MAP.with(|m_cell| {
                    GET_HIT_KEYS.with(|k_cell| {
                        let mut m_b = m_cell.borrow_mut();
                        let m = m_b.as_mut().unwrap();
                        let keys_b = k_cell.borrow();
                        let keys = keys_b.as_ref().unwrap();
                        let refs: Vec<_> = keys
                            .iter()
                            .cloned()
                            .enumerate()
                            .map(|(i, k)| m.insert(k, i as u64).unwrap())
                            .collect();
                        *held.borrow_mut() = Some(refs);
                    })
                })
            }
        });
    }

    fn ensure_get_miss_setup() {
        GET_MISS_MAP.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(RcHashMap::new());
            }
        });
        GET_MISS_HELD.with(|held| {
            if held.borrow().is_none() {
                GET_MISS_MAP.with(|m_cell| {
                    let mut m_b = m_cell.borrow_mut();
                    let m = m_b.as_mut().unwrap();
                    let refs: Vec<_> = lcg(11)
                        .take(OPS)
                        .enumerate()
                        .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                        .collect();
                    *held.borrow_mut() = Some(refs);
                })
            }
        });
    }

    fn ensure_clone_drop_setup() {
        CLONE_DROP_MAP.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(RcHashMap::new());
            }
        });
        CLONE_DROP_REF.with(|r_cell| {
            if r_cell.borrow().is_none() {
                CLONE_DROP_MAP.with(|m_cell| {
                    let mut m_b = m_cell.borrow_mut();
                    let m = m_b.as_mut().unwrap();
                    let r = m.insert("key".to_string(), 1u64).unwrap();
                    *r_cell.borrow_mut() = Some(r);
                })
            }
        });
    }

    fn ensure_ref_incr_setup() {
        REF_INCR_MAP.with(|c| {
            if c.borrow().is_none() {
                *c.borrow_mut() = Some(RcHashMap::new());
            }
        });
        REF_INCR_REFS.with(|refs_cell| {
            if refs_cell.borrow().is_none() {
                REF_INCR_MAP.with(|m_cell| {
                    let mut m_b = m_cell.borrow_mut();
                    let m = m_b.as_mut().unwrap();
                    let refs: Vec<_> = lcg(123)
                        .take(OPS)
                        .enumerate()
                        .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                        .collect();
                    *refs_cell.borrow_mut() = Some(refs);
                })
            }
        });
    }

    fn ensure_iter_mut_setup() {
        ITER_MUT_MAP.with(|m_cell| {
            if m_cell.borrow().is_none() {
                *m_cell.borrow_mut() = Some(RcHashMap::new());
            }
        });
        ITER_MUT_REFS.with(|refs_cell| {
            if refs_cell.borrow().is_none() {
                ITER_MUT_MAP.with(|m_cell| {
                    let mut m_b = m_cell.borrow_mut();
                    let m = m_b.as_mut().unwrap();
                    let refs: Vec<_> = lcg(999)
                        .take(OPS)
                        .enumerate()
                        .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                        .collect();
                    *refs_cell.borrow_mut() = Some(refs);
                })
            }
        });
    }

    fn ensure_drop_setup() {
        DROP_MAP.with(|m_cell| {
            if m_cell.borrow().is_none() {
                *m_cell.borrow_mut() = Some(RcHashMap::new());
            }
        });
        DROP_REFS.with(|refs_cell| {
            if refs_cell.borrow().is_none() {
                DROP_MAP.with(|m_cell| {
                    let mut m_b = m_cell.borrow_mut();
                    let m = m_b.as_mut().unwrap();
                    let refs: Vec<_> = lcg(4242)
                        .take(OPS)
                        .enumerate()
                        .map(|(i, x)| m.insert(key(x), i as u64).unwrap())
                        .collect();
                    *refs_cell.borrow_mut() = Some(refs);
                })
            }
        });
    }

    // A dedicated setup bench to initialize global state before measured benches run.
    pub fn __rc_hashmap_iai_setup() {
        ensure_get_hit_setup();
        ensure_get_miss_setup();
        ensure_clone_drop_setup();
        ensure_ref_incr_setup();
        ensure_iter_mut_setup();
        ensure_drop_setup();
        black_box(())
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

    // Repeated hits on existing keys; setup pre-initialized by __rc_hashmap_iai_setup.
    pub fn rc_hashmap_get_hit_1000_ops() {
        GET_HIT_MAP.with(|m_cell| {
            GET_HIT_KEYS.with(|k_cell| {
                let m_b = m_cell.borrow();
                let m = m_b.as_ref().expect("setup not initialized");
                let keys_b = k_cell.borrow();
                let keys = keys_b.as_ref().expect("setup not initialized");
                // Perform 1k successful lookups.
                let mut it = keys.iter().cycle();
                for _ in 0..OPS {
                    let k = it.next().unwrap();
                    let r = m.find(k).unwrap();
                    black_box(&r);
                }
            })
        })
    }

    // Repeated misses for keys unlikely to be present; setup pre-initialized.
    pub fn rc_hashmap_get_miss_1000_ops() {
        GET_MISS_MAP.with(|m_cell| {
            let m_b = m_cell.borrow();
            let m = m_b.as_ref().expect("setup not initialized");
            let mut miss = lcg(0xdead_beef);
            for _ in 0..OPS {
                let k = key(miss.next().unwrap());
                black_box(m.find(&k));
            }
        })
    }

    // Clone and drop a Ref repeatedly; setup pre-initialized.
    pub fn rc_hashmap_clone_drop_ref_1000_ops() {
        CLONE_DROP_REF.with(|r_cell| {
            let r_b = r_cell.borrow();
            let r = r_b.as_ref().expect("setup not initialized");
            for _ in 0..OPS {
                let x = r.clone();
                black_box(&x);
                drop(x);
            }
        })
    }

    // Cycle and increment values via handle; setup map and refs pre-initialized.
    pub fn rc_hashmap_ref_increment_1000_ops() {
        REF_INCR_MAP.with(|m_cell| {
            REF_INCR_REFS.with(|refs_cell| {
                let mut m_b = m_cell.borrow_mut();
                let m = m_b.as_mut().expect("setup not initialized");
                let refs_b = refs_cell.borrow();
                let refs = refs_b.as_ref().expect("setup not initialized");
                let mut idx = 0usize;
                for _ in 0..OPS {
                    let r = &refs[idx];
                    let v = r.value_mut(m).unwrap();
                    *v = v.wrapping_add(1);
                    idx += 1;
                    if idx == refs.len() {
                        idx = 0;
                    }
                }
                black_box(m.len());
            })
        })
    }

    // Iterate mutably, incrementing each value; setup map pre-initialized.
    pub fn rc_hashmap_iter_mut_increment_1000_ops() {
        ITER_MUT_MAP.with(|m_cell| {
            let mut m_b = m_cell.borrow_mut();
            let m = m_b.as_mut().expect("setup not initialized");
            for mut item in m.iter_mut() {
                let v = item.value_mut();
                *v = v.wrapping_add(1);
            }
            black_box(m.len());
        })
    }

    // Drop the last Ref to 1k entries, triggering removals.
    pub fn rc_hashmap_drop_last_ref_1000_ops() {
        DROP_MAP.with(|m_cell| {
            DROP_REFS.with(|refs_cell| {
                // Keep the map alive while dropping refs
                let m_b = m_cell.borrow();
                let m = m_b.as_ref().expect("setup not initialized");
                let mut refs_b = refs_cell.borrow_mut();
                let refs = refs_b.as_mut().expect("setup not initialized");
                assert!(refs.len() == OPS);
                for r in refs.drain(..) {
                    black_box(&r);
                    drop(r);
                }
                black_box(m.len());
            })
        })
    }
}

#[cfg(target_os = "linux")]
use bench::{
    __rc_hashmap_iai_setup, rc_hashmap_clone_drop_ref_1000_ops, rc_hashmap_drop_last_ref_1000_ops,
    rc_hashmap_get_hit_1000_ops, rc_hashmap_get_miss_1000_ops, rc_hashmap_insert_1000_ops,
    rc_hashmap_iter_mut_increment_1000_ops, rc_hashmap_ref_increment_1000_ops,
};

// Custom harness to run a global setup before iai's runner, so setup is
// accounted for equally in calibration and benchmark runs (and subtracted).
#[cfg(target_os = "linux")]
mod __iai_custom_harness {
    use super::*;

    mod wrappers {
        use super::*;
        pub fn rc_hashmap_insert_1000_ops() {
            let _ = iai::black_box(bench::rc_hashmap_insert_1000_ops());
        }
        pub fn rc_hashmap_get_hit_1000_ops() {
            let _ = iai::black_box(bench::rc_hashmap_get_hit_1000_ops());
        }
        pub fn rc_hashmap_get_miss_1000_ops() {
            let _ = iai::black_box(bench::rc_hashmap_get_miss_1000_ops());
        }
        pub fn rc_hashmap_clone_drop_ref_1000_ops() {
            let _ = iai::black_box(bench::rc_hashmap_clone_drop_ref_1000_ops());
        }
        pub fn rc_hashmap_ref_increment_1000_ops() {
            let _ = iai::black_box(bench::rc_hashmap_ref_increment_1000_ops());
        }
        pub fn rc_hashmap_iter_mut_increment_1000_ops() {
            let _ = iai::black_box(bench::rc_hashmap_iter_mut_increment_1000_ops());
        }
        pub fn rc_hashmap_drop_last_ref_1000_ops() {
            let _ = iai::black_box(bench::rc_hashmap_drop_last_ref_1000_ops());
        }
    }

    pub fn main() {
        // Prepare shared state before dispatching to runner so it's deducted by calibration.
        __rc_hashmap_iai_setup();
        let benches: &[&(&'static str, fn())] = &[
            &(
                "rc_hashmap_insert_1000_ops",
                wrappers::rc_hashmap_insert_1000_ops,
            ),
            &(
                "rc_hashmap_get_hit_1000_ops",
                wrappers::rc_hashmap_get_hit_1000_ops,
            ),
            &(
                "rc_hashmap_get_miss_1000_ops",
                wrappers::rc_hashmap_get_miss_1000_ops,
            ),
            &(
                "rc_hashmap_clone_drop_ref_1000_ops",
                wrappers::rc_hashmap_clone_drop_ref_1000_ops,
            ),
            &(
                "rc_hashmap_ref_increment_1000_ops",
                wrappers::rc_hashmap_ref_increment_1000_ops,
            ),
            &(
                "rc_hashmap_iter_mut_increment_1000_ops",
                wrappers::rc_hashmap_iter_mut_increment_1000_ops,
            ),
            &(
                "rc_hashmap_drop_last_ref_1000_ops",
                wrappers::rc_hashmap_drop_last_ref_1000_ops,
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
