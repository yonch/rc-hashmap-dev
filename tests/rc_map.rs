use rc_hashmap::{InsertError, RcHashMap};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[test]
fn insert_get_clone_drop_removes() {
    let mut m = RcHashMap::new();
    let r = m.insert("k1".to_string(), 42).expect("insert ok");
    assert_eq!(m.len(), 1);
    assert!(m.contains_key(&"k1".to_string()));

    // find returns a new Ref and increments the count
    let g = m.find(&"k1".to_string()).expect("found");
    assert_eq!(*g.value(&m).expect("value borrow"), 42);

    // clone keeps entry alive
    let g2 = g.clone();
    drop(g);
    assert!(m.contains_key(&"k1".to_string()));

    // dropping the last runtime ref that's not `r` should keep entry (since `r` still alive)
    drop(g2);
    assert_eq!(m.len(), 1);
    assert!(m.contains_key(&"k1".to_string()));

    // drop the original returned ref as well; now removal should occur
    drop(r);
    assert_eq!(m.len(), 0);
    assert!(!m.contains_key(&"k1".to_string()));
}

#[test]
fn duplicate_insert_rejected() {
    let mut m = RcHashMap::new();
    let r = m.insert("dup".to_string(), 1).unwrap();
    let e = m.insert("dup".to_string(), 2);
    match e {
        Err(InsertError::DuplicateKey) => {}
        Ok(_) => panic!("expected duplicate insert to error"),
    }
    drop(r);
}

#[test]
fn ref_equality_and_hash() {
    let mut m = RcHashMap::new();
    let r1 = m.insert("a".to_string(), 10).unwrap();
    let r1b = r1.clone();
    assert!(r1 == r1b);

    let mut h1 = DefaultHasher::new();
    r1.hash(&mut h1);
    let mut h2 = DefaultHasher::new();
    r1b.hash(&mut h2);
    assert_eq!(h1.finish(), h2.finish());

    let r2 = m.insert("b".to_string(), 20).unwrap();
    assert!(r1 != r2);
}

#[test]
fn wrong_map_accessors_reject() {
    use rc_hashmap::Ref as _; // silence unused import warnings if traits added later

    let mut m1 = RcHashMap::new();
    let mut m2 = RcHashMap::new();
    let r = m1.insert("a".to_string(), 11).unwrap();

    // Owner-checked accessors
    assert!(r.value(&m1).is_ok());
    assert!(r.key(&m1).is_ok());
    assert!(r.value_mut(&mut m1).is_ok());

    // Wrong map should be rejected
    assert!(r.value(&m2).is_err());
    assert!(r.key(&m2).is_err());
}

#[test]
fn iter_returns_refs() {
    let mut m = RcHashMap::new();
    let _ = m.insert("k1".to_string(), 1).unwrap();
    let _ = m.insert("k2".to_string(), 2).unwrap();
    let _ = m.insert("k3".to_string(), 3).unwrap();

    let count = m.iter().count();
    assert_eq!(count, m.len());

    // Values are reachable via returned Refs
    for r in m.iter() {
        let v = r.value(&m).expect("value borrow");
        assert!(*v == 1 || *v == 2 || *v == 3);
    }
}

#[test]
fn iter_mut_updates_and_allows_cloning_ref() {
    let mut m = RcHashMap::new();
    let r1 = m.insert("k1".to_string(), 1).unwrap();
    let r2 = m.insert("k2".to_string(), 2).unwrap();

    // Mutate values in place and keep clones alive until after iteration
    let mut held = Vec::new();
    for mut it in m.iter_mut() {
        *it.value_mut() += 10;
        // Clone a Ref from the item to keep the entry alive beyond this iteration
        held.push(it.r#ref().clone());
    }
    assert_eq!(m.len(), 2);

    // Verify values updated using existing Refs to keep tokens alive
    assert_eq!(*r1.value(&m).unwrap(), 11);
    assert_eq!(*r2.value(&m).unwrap(), 12);
}
