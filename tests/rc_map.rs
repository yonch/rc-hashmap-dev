use rc_hashmap::{InsertError, RcHashMap};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[test]
fn insert_get_clone_drop_removes() {
    let mut m = RcHashMap::new();
    let r = m.insert("k1".to_string(), 42).expect("insert ok");
    assert_eq!(m.len(), 1);
    assert!(m.contains_key(&"k1".to_string()));

    // get returns a new Ref and increments the count
    let g = m.get(&"k1".to_string()).expect("found");
    assert_eq!(*g.value().expect("value borrow"), 42);

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
