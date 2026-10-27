use super::*;
use crate::code_rule_dsl::facts::propagate::{Work, propagate};
use crate::content_store::DerivationInput;
use std::time::{SystemTime, UNIX_EPOCH};

fn store(label: &str) -> (Arc<ContentStore>, PathBuf) {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let base = std::env::temp_dir().join(format!("rerail-overlay-{label}-{}-{nonce}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    (Arc::new(ContentStore::open(&base.join("store.sqlite")).unwrap()), base.join("spill"))
}

fn limits(spill: PathBuf, top: usize, resident: usize, behind: usize, background: bool) -> Limits {
    Limits { top, resident, behind, spill, background }
}

fn d(n: u8) -> [u8; 32] {
    [n; 32]
}

fn binding(name: &str, n: u8) -> Binding {
    Binding { name: name.to_owned(), digest: d(n) }
}

fn record(output: &str, reads: &[(&str, u8)], n: u8) -> DerivationRecord {
    DerivationRecord {
        output: output.to_owned(),
        producer: d(9),
        inputs: reads.iter().map(|(name, n)| DerivationInput { name: Some((*name).to_owned()), digest: d(*n) }).collect(),
        digest: d(n),
        status: 0,
        cost: 1,
    }
}

fn spilled_files(spill: &std::path::Path) -> usize {
    std::fs::read_dir(spill).unwrap().count()
}

#[test]
fn writes_are_read_back_before_they_reach_sqlite() {
    let (store, spill) = store("unwritten");
    let overlay = Overlay::new(Arc::clone(&store), limits(spill, usize::MAX, usize::MAX, usize::MAX, false)).unwrap();
    assert_eq!(overlay.bind_names(&[binding("a", 1)]).unwrap(), ["a"]);
    assert_eq!(overlay.record_derivations(&[record("x", &[("a", 1)], 5)]).unwrap(), ["x"]);
    assert_eq!(overlay.binding("x").unwrap(), Some(d(5)));
    assert!(overlay.derivation_valid("x").unwrap());
    assert_eq!(store.binding("a").unwrap(), None);
    assert!(!store.derivation_valid("x").unwrap());
    // Rebinding to the same digest is no change, in the overlay as in the store.
    assert!(overlay.bind_names(&[binding("a", 1)]).unwrap().is_empty());
    overlay.flush().unwrap();
    assert_eq!(store.binding("a").unwrap(), Some(d(1)));
    assert!(store.derivation_valid("x").unwrap());
    assert_eq!(overlay.counters(), Counters { frozen: 1, written: 1, ..Counters::default() });
}

#[test]
fn a_full_top_freezes_spills_and_merges_into_sqlite() {
    let (store, spill) = store("spill");
    // Every write freezes a layer and every frozen layer spills.
    let overlay = Overlay::new(Arc::clone(&store), limits(spill.clone(), 0, 0, usize::MAX, false)).unwrap();
    overlay.bind_names(&[binding("a", 1), binding("b", 2)]).unwrap();
    overlay.record_derivations(&[record("x", &[("a", 1)], 5), record("y", &[("a", 1), ("b", 2)], 6)]).unwrap();
    overlay.bind_names(&[binding("a", 3)]).unwrap();
    assert_eq!((overlay.behind(), spilled_files(&spill)), (3, 3));
    assert_eq!(overlay.resident_bytes(), 0);
    // Reads see the newest layer for each fact, from the spilled files.
    assert_eq!(overlay.binding("a").unwrap(), Some(d(3)));
    assert_eq!(overlay.binding("b").unwrap(), Some(d(2)));
    let frontier = overlay.derivations_reading(&["a".to_owned()]).unwrap();
    assert_eq!(frontier.iter().map(|f| f.output.as_str()).collect::<Vec<_>>(), ["x", "y"]);
    assert!(!overlay.derivation_valid("x").unwrap());
    assert!(overlay.counters().loaded > 0);
    assert_eq!(store.binding("a").unwrap(), None);
    overlay.flush().unwrap();
    assert_eq!(spilled_files(&spill), 0);
    assert_eq!(store.binding("a").unwrap(), Some(d(3)));
    assert_eq!(store.binding("y").unwrap(), Some(d(6)));
    assert!(!store.derivation_valid("x").unwrap());
    let counters = overlay.counters();
    assert_eq!((counters.frozen, counters.spilled, counters.written), (3, 3, 3));
}

#[test]
fn a_writer_that_falls_behind_is_caught_up_by_the_push_path() {
    let (store, spill) = store("behind");
    let overlay = Overlay::new(Arc::clone(&store), limits(spill, 0, usize::MAX, 2, false)).unwrap();
    for n in 1..=5 {
        overlay.bind_names(&[binding(&format!("n{n}"), n)]).unwrap();
        assert!(overlay.behind() <= 2);
    }
    assert_eq!(overlay.counters().pressed, 3);
    // The oldest layers are in SQLite; the newest still only in memory.
    assert_eq!(store.binding("n1").unwrap(), Some(d(1)));
    assert_eq!(store.binding("n5").unwrap(), None);
    assert_eq!(overlay.binding("n5").unwrap(), Some(d(5)));
}

#[test]
fn a_rerecorded_derivation_stops_reading_what_it_dropped_across_layers() {
    let (store, spill) = store("reread");
    let overlay = Overlay::new(Arc::clone(&store), limits(spill, usize::MAX, usize::MAX, usize::MAX, false)).unwrap();
    overlay.bind_names(&[binding("a", 1), binding("b", 1)]).unwrap();
    overlay.record_derivations(&[record("x", &[("a", 1)], 5)]).unwrap();
    overlay.flush().unwrap();
    // SQLite says x reads a; the top says x now reads b.
    overlay.record_derivations(&[record("x", &[("b", 1)], 5)]).unwrap();
    assert!(overlay.derivations_reading(&["a".to_owned()]).unwrap().is_empty());
    assert_eq!(overlay.derivations_reading(&["b".to_owned()]).unwrap().len(), 1);
}

#[test]
fn propagation_over_the_overlay_matches_propagation_over_the_store() {
    // a → t → f → {r, s}: an edit that changes t but not f stops at f.
    let reads: &[(&str, &str)] = &[("t", "a"), ("f", "t"), ("r", "f"), ("s", "f")];
    let run = |graph: &dyn Graph| -> (Vec<Vec<String>>, Work) {
        graph.bind_names(&[binding("a", 1)]).unwrap();
        for (output, input) in reads {
            let digest = graph.binding(input).unwrap().unwrap();
            graph
                .record_derivations(&[DerivationRecord {
                    output: (*output).to_owned(),
                    producer: d(9),
                    inputs: vec![DerivationInput { name: Some((*input).to_owned()), digest }],
                    digest: d(if *output == "t" { 10 } else { 20 }),
                    status: 0,
                    cost: 1,
                }])
                .unwrap();
        }
        let changed = graph.bind_names(&[binding("a", 2)]).unwrap();
        let mut waves = Vec::new();
        let work = propagate(graph, changed, 8, |frontier| {
            waves.push(frontier.iter().map(|f| f.output.clone()).collect());
            Ok(frontier
                .iter()
                .map(|f| {
                    let input = reads.iter().find(|(o, _)| *o == f.output).unwrap().1;
                    let digest = graph.binding(input).unwrap().unwrap();
                    DerivationRecord {
                        output: f.output.clone(),
                        producer: f.producer,
                        inputs: vec![DerivationInput { name: Some(input.to_owned()), digest }],
                        digest: d(if f.output == "t" { 11 } else { 20 }),
                        status: 0,
                        cost: 1,
                    }
                })
                .collect())
        })
        .unwrap();
        (waves, work)
    };
    let (direct, _) = store("direct");
    let expected = run(&*direct);
    assert_eq!(expected.0, [vec!["t"], vec!["f"]]);
    let (store, spill) = store("background");
    let overlay = Overlay::new(Arc::clone(&store), limits(spill, 64, 128, 4, true)).unwrap();
    assert_eq!(run(&overlay), expected);
    overlay.flush().unwrap();
    for output in ["t", "f", "r", "s"] {
        assert!(store.derivation_valid(output).unwrap(), "{output}");
    }
    assert_eq!(store.binding("t").unwrap(), Some(d(11)));
}
