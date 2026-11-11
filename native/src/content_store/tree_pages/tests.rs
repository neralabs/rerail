use super::super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn store(label: &str) -> ContentStore {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rerail-tree-pages-{label}-{}-{nonce}.sqlite", std::process::id()));
    ContentStore::open(&path).unwrap()
}

fn key(tree: u8, plan: u8) -> TreePageKey<'static> {
    TreePageKey { tree: [tree; 32], prefix: "src", context: [0; 32], plan: [plan; 32] }
}

fn page(bytes: &[u8], file_count: usize) -> TreePage {
    TreePage { file_count, bytes: bytes.to_vec() }
}

#[test]
fn tree_pages_round_trip_and_an_identity_holds_one_set_of_bytes() {
    let store = store("round-trip");
    let bytes = br#"{"files":["src/a.py"],"findings":[],"failures":[]}"#;
    let pages = vec![page(bytes, 1), page(b"second", 64)];
    let digest = store.put_tree_pages(&key(31, 1), &pages).unwrap();
    assert_eq!(digest, tree_pages_digest(&pages));
    assert_eq!(store.tree_pages(&key(31, 1)).unwrap(), Some((digest, pages.clone())));
    // Storing the same pages again is a no-op; other bytes under the identity are refused.
    assert_eq!(store.put_tree_pages(&key(31, 1), &pages).unwrap(), digest);
    assert!(matches!(store.put_tree_pages(&key(31, 1), &[page(b"other", 1), page(b"second", 64)]), Err(StoreError::Corrupt(_))));
    // Another tree or plan is another identity.
    assert_eq!(store.tree_pages(&key(32, 1)).unwrap(), None);
    assert_eq!(store.tree_pages(&key(31, 2)).unwrap(), None);
    assert!(store.put_tree_pages(&key(31, 3), &[]).is_err());
    assert!(store.put_tree_pages(&key(31, 3), &[page(b"x", 65)]).is_err());
}

#[test]
fn collection_keeps_pages_a_name_binds() {
    let store = store("collect");
    let kept = store.put_tree_pages(&key(1, 1), &[page(b"kept", 1)]).unwrap();
    store.put_tree_pages(&key(2, 1), &[page(b"dropped", 1)]).unwrap();
    store.bind_names(&[Binding { name: "/repo#page:src".to_owned(), digest: kept }]).unwrap();
    store.collect(i64::MAX, 256).unwrap();
    assert!(store.has_tree_pages(&key(1, 1)).unwrap());
    assert!(!store.has_tree_pages(&key(2, 1)).unwrap());
}
