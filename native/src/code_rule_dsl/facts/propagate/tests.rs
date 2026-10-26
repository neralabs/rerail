use super::*;
use crate::content_store::{Binding, DerivationInput};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn store() -> ContentStore {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rerail-propagate-{}-{nonce}.sqlite", std::process::id()));
    ContentStore::open(&path).unwrap()
}

fn d(n: u8) -> [u8; 32] {
    [n; 32]
}

/// A toy producer graph: each output reads the names in `reads` and produces `outputs[name]`.
struct Graph {
    reads: HashMap<&'static str, Vec<&'static str>>,
    outputs: HashMap<&'static str, u8>,
    runs: Vec<Vec<String>>,
}

impl Graph {
    fn run(&mut self, store: &ContentStore, frontier: &[Derivation]) -> StoreResult<Vec<DerivationRecord>> {
        self.runs.push(frontier.iter().map(|d| d.output.clone()).collect());
        frontier.iter().map(|d| self.record(store, &d.output)).collect()
    }

    fn record(&self, store: &ContentStore, output: &str) -> StoreResult<DerivationRecord> {
        let inputs = self.reads[output]
            .iter()
            .map(|name| {
                Ok(DerivationInput {
                    name: Some((*name).to_owned()),
                    digest: store.binding(name)?.unwrap_or(crate::content_store::ABSENT),
                })
            })
            .collect::<StoreResult<_>>()?;
        Ok(DerivationRecord {
            output: output.to_owned(),
            producer: d(99),
            inputs,
            digest: d(self.outputs[output]),
            status: 0,
            cost: 1,
        })
    }
}

/// a.py → tape:a → facts:a → {rule:x, rule:y}; b.py → tape:b → facts:b.
fn graph() -> Graph {
    Graph {
        reads: HashMap::from([
            ("tape:a", vec!["a.py"]),
            ("facts:a", vec!["tape:a"]),
            ("rule:x", vec!["facts:a"]),
            ("rule:y", vec!["facts:a", "facts:b"]),
            ("tape:b", vec!["b.py"]),
            ("facts:b", vec!["tape:b"]),
        ]),
        outputs: HashMap::from([
            ("tape:a", 10),
            ("facts:a", 20),
            ("rule:x", 30),
            ("rule:y", 31),
            ("tape:b", 11),
            ("facts:b", 21),
        ]),
        runs: Vec::new(),
    }
}

fn seed(store: &ContentStore, graph: &Graph) {
    store
        .bind_names(&[
            Binding { name: "a.py".to_owned(), digest: d(1) },
            Binding { name: "b.py".to_owned(), digest: d(2) },
        ])
        .unwrap();
    for wave in [&["tape:a", "tape:b"][..], &["facts:a", "facts:b"], &["rule:x", "rule:y"]] {
        let records = wave.iter().map(|o| graph.record(store, o).unwrap()).collect::<Vec<_>>();
        store.record_derivations(&records).unwrap();
    }
}

#[test]
fn an_edit_whose_facts_are_unchanged_stops_after_the_facts() {
    let store = store();
    let mut graph = graph();
    seed(&store, &graph);
    // The edit changes a.py's bytes and its tape, but not the facts the rules read.
    graph.outputs.insert("tape:a", 12);
    let changed = store.bind_names(&[Binding { name: "a.py".to_owned(), digest: d(3) }]).unwrap();
    let work = propagate(&store, changed, 16, |f| graph.run(&store, f)).unwrap();
    assert_eq!(graph.runs, [vec!["tape:a"], vec!["facts:a"]]);
    assert_eq!(work, Work { waves: 2, rerun: 2, changed: 1 });
    for output in ["tape:a", "facts:a", "rule:x", "rule:y", "facts:b"] {
        assert!(store.derivation_valid(output).unwrap(), "{output}");
    }
}

#[test]
fn a_changed_fact_reruns_exactly_its_readers_in_one_wave() {
    let store = store();
    let mut graph = graph();
    seed(&store, &graph);
    graph.outputs.insert("tape:a", 12);
    graph.outputs.insert("facts:a", 22);
    let changed = store.bind_names(&[Binding { name: "a.py".to_owned(), digest: d(3) }]).unwrap();
    let work = propagate(&store, changed, 16, |f| graph.run(&store, f)).unwrap();
    assert_eq!(graph.runs, [vec!["tape:a"], vec!["facts:a"], vec!["rule:x", "rule:y"]]);
    assert_eq!(work, Work { waves: 3, rerun: 4, changed: 2 });
}

#[test]
fn an_unchanged_rebinding_runs_nothing() {
    let store = store();
    let mut graph = graph();
    seed(&store, &graph);
    let changed = store.bind_names(&[Binding { name: "a.py".to_owned(), digest: d(1) }]).unwrap();
    let work = propagate(&store, changed, 16, |f| graph.run(&store, f)).unwrap();
    assert!(graph.runs.is_empty());
    assert_eq!(work, Work::default());
}

#[test]
fn digests_that_never_settle_are_an_error() {
    let store = store();
    store.record_derivations(&[DerivationRecord {
        output: "loop".to_owned(),
        producer: d(99),
        inputs: vec![DerivationInput { name: Some("loop".to_owned()), digest: d(0) }],
        digest: d(1),
        status: 0,
        cost: 1,
    }]).unwrap();
    let mut n = 1u8;
    let error = propagate(&store, vec!["loop".to_owned()], 4, |frontier| {
        n += 1;
        Ok(frontier
            .iter()
            .map(|f| DerivationRecord {
                output: f.output.clone(),
                producer: f.producer,
                inputs: vec![DerivationInput { name: Some("loop".to_owned()), digest: d(n - 1) }],
                digest: d(n),
                status: 0,
                cost: 1,
            })
            .collect())
    })
    .unwrap_err();
    assert!(error.to_string().contains("did not settle"), "{error}");
}
