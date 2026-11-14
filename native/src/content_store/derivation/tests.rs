use super::super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn store(label: &str) -> ContentStore {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rerail-derivation-{label}-{}-{nonce}.sqlite",
        std::process::id()
    ));
    ContentStore::open(&path).unwrap()
}

fn d(n: u8) -> [u8; 32] {
    [n; 32]
}

fn binding(name: &str, n: u8) -> Binding {
    Binding { name: name.to_owned(), digest: d(n) }
}

fn read(name: &str, n: u8) -> DerivationInput {
    DerivationInput { name: Some(name.to_owned()), digest: d(n) }
}

fn record(output: &str, inputs: Vec<DerivationInput>, n: u8) -> DerivationRecord {
    DerivationRecord { output: output.to_owned(), producer: d(9), inputs, digest: d(n), status: 0, cost: 1 }
}

fn outputs(frontier: &[Derivation]) -> Vec<&str> {
    frontier.iter().map(|d| d.output.as_str()).collect()
}

#[test]
fn rebinding_reports_only_names_whose_digest_changed() {
    let store = store("bind");
    assert_eq!(store.bind_names(&[binding("a", 1), binding("b", 2)]).unwrap(), ["a", "b"]);
    assert_eq!(store.bind_names(&[binding("a", 1), binding("b", 3)]).unwrap(), ["b"]);
    assert_eq!(store.binding("b").unwrap(), Some(d(3)));
    assert_eq!(store.binding("c").unwrap(), None);
}

#[test]
fn a_derivation_is_valid_while_every_name_it_read_binds_to_what_it_read() {
    let store = store("valid");
    store.bind_names(&[binding("src/a.py", 1)]).unwrap();
    store
        .record_derivations(&[record("tape:a", vec![read("src/a.py", 1)], 5)])
        .unwrap();
    assert!(store.derivation_valid("tape:a").unwrap());
    store.bind_names(&[binding("src/a.py", 2)]).unwrap();
    assert!(!store.derivation_valid("tape:a").unwrap());
    // Back to the bytes it read: valid again, without re-running.
    store.bind_names(&[binding("src/a.py", 1)]).unwrap();
    assert!(store.derivation_valid("tape:a").unwrap());
    assert!(!store.derivation_valid("never-recorded").unwrap());
}

#[test]
fn an_absent_name_read_is_invalidated_when_it_is_first_bound() {
    let store = store("absent");
    store
        .record_derivations(&[record("resolve:m", vec![DerivationInput { name: Some("pkg/m.py".to_owned()), digest: ABSENT }], 4)])
        .unwrap();
    assert!(store.derivation_valid("resolve:m").unwrap());
    let changed = store.bind_names(&[binding("pkg/m.py", 7)]).unwrap();
    assert_eq!(changed, ["pkg/m.py"]);
    assert!(!store.derivation_valid("resolve:m").unwrap());
    assert_eq!(outputs(&store.derivations_reading(&changed).unwrap()), ["resolve:m"]);
}

#[test]
fn the_frontier_is_exactly_the_readers_of_changed_names() {
    let store = store("frontier");
    store.bind_names(&[binding("a", 1), binding("b", 1)]).unwrap();
    store
        .record_derivations(&[
            record("x", vec![read("a", 1)], 10),
            record("y", vec![read("b", 1)], 11),
            record("z", vec![read("a", 1), read("b", 1)], 12),
        ])
        .unwrap();
    let changed = store.bind_names(&[binding("a", 2), binding("b", 1)]).unwrap();
    assert_eq!(outputs(&store.derivations_reading(&changed).unwrap()), ["x", "z"]);
}

#[test]
fn a_recorded_output_equal_to_the_last_rebinds_nothing() {
    let store = store("cutoff");
    store
        .record_derivations(&[record("x", vec![], 10), record("y", vec![read("x", 10)], 20)])
        .unwrap();
    // Recomputed with the same output: no change, so y is not in any frontier.
    assert!(store.record_derivations(&[record("x", vec![], 10)]).unwrap().is_empty());
    assert!(store.derivation_valid("y").unwrap());
    // A different output dirties its readers.
    let changed = store.record_derivations(&[record("x", vec![], 11)]).unwrap();
    assert_eq!(changed, ["x"]);
    assert!(!store.derivation_valid("y").unwrap());
    assert_eq!(outputs(&store.derivations_reading(&changed).unwrap()), ["y"]);
}

#[test]
fn rerecording_a_derivation_replaces_what_it_read() {
    let store = store("replace");
    store.bind_names(&[binding("a", 1), binding("b", 1)]).unwrap();
    store.record_derivations(&[record("x", vec![read("a", 1)], 10)]).unwrap();
    store.record_derivations(&[record("x", vec![read("b", 1)], 10)]).unwrap();
    let changed = store.bind_names(&[binding("a", 2)]).unwrap();
    assert!(store.derivations_reading(&changed).unwrap().is_empty());
    assert!(store.derivation_valid("x").unwrap());
}
