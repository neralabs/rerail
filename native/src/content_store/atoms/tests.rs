use super::super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn path(label: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-atoms-{label}-{}-{nonce}.sqlite", std::process::id()))
}

#[test]
fn atoms_are_dense_stable_and_survive_reopening() {
    let path = path("dense");
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.atom("").unwrap(), Some(0));
    assert_eq!(store.intern_atoms(&["a", "b", "a"]).unwrap(), [1, 2, 1]);
    assert_eq!(store.intern_atoms(&["b", "c"]).unwrap(), [2, 3]);
    assert_eq!(store.atom("d").unwrap(), None);
    drop(store);
    let store = ContentStore::open(&path).unwrap();
    assert_eq!(store.atom("c").unwrap(), Some(3));
    assert_eq!(store.atom_value(2).unwrap().as_deref(), Some("b"));
    assert_eq!(store.atom_value(9).unwrap(), None);
    assert_eq!(store.intern_atoms(&["d"]).unwrap(), [4]);
}

#[test]
fn two_handles_on_one_store_allocate_the_same_ids() {
    let path = path("shared");
    let one = ContentStore::open(&path).unwrap();
    let two = ContentStore::open(&path).unwrap();
    assert_eq!(one.intern_atoms(&["x"]).unwrap(), [1]);
    // `two` has not seen `x`: it catches up with the table before appending.
    assert_eq!(two.intern_atoms(&["y", "x"]).unwrap(), [2, 1]);
    assert_eq!(one.atom("y").unwrap(), Some(2));
    assert_eq!(one.intern_atoms(&["z"]).unwrap(), [3]);
    assert_eq!(two.atom_value(3).unwrap().as_deref(), Some("z"));
}
