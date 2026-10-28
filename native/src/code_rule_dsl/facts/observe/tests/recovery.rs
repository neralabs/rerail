//! Crash recovery (open-items O1): a push that crashes at any wave, through the store or the
//! overlay, leaves SQLite holding a prefix of its writes, and `recover` then finds exactly the
//! invalid derivations and brings the store to what a push that never crashed leaves.

use super::*;
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::overlay::{Limits, Overlay};
use crate::content_store::{Binding, StoreError};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const V1: u8 = 100;
const V2: u8 = 101;
/// The waves the push from V1 to V2 takes: {page:src, tape:a}, {page:src}, {index}, {report}.
const WAVES: u32 = 4;

/// Each derivation and the names it reads, readers after what they read.
fn reads() -> Vec<(&'static str, Vec<String>)> {
    vec![
        ("tape:a", vec![path_name(WS, "src/a.py")]),
        ("tape:b", vec![path_name(WS, "src/b.py")]),
        ("page:lib", vec![tree_name(WS, "lib")]),
        ("page:src", vec![tree_name(WS, "src"), "tape:a".to_owned(), "tape:b".to_owned()]),
        ("index", vec!["page:src".to_owned(), "page:lib".to_owned()]),
        ("report", vec!["index".to_owned()]),
    ]
}

/// `output` derived from what its names bind to in `graph` now: its digest is a function of
/// theirs, so every change reaches the report.
fn derive(graph: &dyn Graph, output: &str) -> DerivationRecord {
    let (_, names) = reads().into_iter().find(|(o, _)| *o == output).unwrap();
    let inputs = names
        .iter()
        .map(|name| DerivationInput { name: Some(name.clone()), digest: graph.binding(name).unwrap().unwrap_or(ABSENT) })
        .collect::<Vec<_>>();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    output.hash(&mut hasher);
    inputs.iter().for_each(|input| input.digest.hash(&mut hasher));
    let mut digest = [0; 32];
    digest.chunks_mut(8).for_each(|chunk| chunk.copy_from_slice(&hasher.finish().to_le_bytes()));
    DerivationRecord { output: output.to_owned(), producer: [9; 32], inputs, digest, status: 0, cost: 1 }
}

/// A producer over `graph` that crashes (fails) on wave `crash`.
fn recompute(
    graph: &dyn Graph,
    crash: Option<u32>,
) -> impl FnMut(&[Derivation]) -> StoreResult<Vec<DerivationRecord>> + '_ {
    let mut wave = 0;
    move |frontier| {
        wave += 1;
        if Some(wave) == crash {
            return Err(StoreError::Invalid("crashed".to_owned()));
        }
        Ok(frontier.iter().map(|f| derive(graph, &f.output)).collect())
    }
}

fn directory(label: &str) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rerail-recovery-{label}-{}-{nonce}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn open(dir: &Path) -> ContentStore {
    ContentStore::open(&dir.join("store.sqlite")).unwrap()
}

/// A store that observed V1 and holds every derivation, valid.
fn seeded(at: &Path) -> ContentStore {
    let store = open(at);
    let src = tree(10, vec![file("a.py", 1), file("b.py", 2)]);
    let lib = tree(20, vec![file("c.py", 3)]);
    let edited = tree(11, vec![file("a.py", 5), file("b.py", 2)]);
    let v1 = tree(V1, vec![dir("src", &src), dir("lib", &lib)]);
    let v2 = tree(V2, vec![dir("src", &edited), dir("lib", &lib)]);
    store.put_git_trees_bulk(&[src, lib, edited, v1, v2]).unwrap();
    push_root(&store, WS, hash(V1), 8, |_| unreachable!()).unwrap();
    for (output, _) in reads() {
        store.record_derivations(&[derive(&store, output)]).unwrap();
    }
    store
}

/// The derivation graph as SQLite holds it: bindings, and each derivation's inputs.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Dump {
    bindings: BTreeMap<String, Vec<u8>>,
    inputs: BTreeMap<String, Vec<(Option<String>, Vec<u8>)>>,
}

fn dump(dir: &Path) -> Dump {
    let connection = rusqlite::Connection::open(dir.join("store.sqlite")).unwrap();
    let bindings = connection
        .prepare("SELECT a.value,b.digest FROM name_bindings b JOIN atoms a ON a.atom=b.name_atom")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let mut inputs = BTreeMap::<String, Vec<_>>::new();
    let rows = connection
        .prepare(
            "SELECT o.value,n.value,i.digest FROM derivation_inputs i JOIN atoms o ON o.atom=i.output_atom \
             LEFT JOIN atoms n ON n.atom=i.name_atom ORDER BY o.value,i.ordinal",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for (output, name, digest) in rows {
        inputs.entry(output).or_default().push((name, digest));
    }
    Dump { bindings, inputs }
}

/// The derivations whose recorded input digests differ from the bindings, by brute force.
fn invalid(dump: &Dump) -> Vec<String> {
    dump.inputs
        .iter()
        .filter(|(_, inputs)| {
            inputs.iter().any(|(name, digest)| {
                name.as_ref().is_some_and(|name| dump.bindings.get(name).map_or(&ABSENT[..], Vec::as_slice) != digest)
            })
        })
        .map(|(output, _)| output.clone())
        .collect()
}

/// The store after the push from V1 to V2 with no crash.
fn clean() -> Dump {
    let dir = directory("clean");
    let store = seeded(&dir);
    let push = push_root(&store, WS, hash(V2), 8, recompute(&store, None)).unwrap();
    assert_eq!(push.work.waves, WAVES);
    assert_eq!(recover(&store, 8, recompute(&store, None)).unwrap(), None);
    dump(&dir)
}

/// Reopens a crashed store, checks recovery's frontier against brute force, recovers, runs the
/// crashed push again, and returns the store as it is then.
fn recovered(dir: &Path) -> Dump {
    let crashed = dump(dir);
    let store = open(dir);
    assert!(!store.push_clean().unwrap());
    let frontier = store.invalid_derivations().unwrap().into_iter().map(|d| d.output).collect::<Vec<_>>();
    assert_eq!(frontier, invalid(&crashed));
    recover(&store, 8, recompute(&store, None)).unwrap().unwrap();
    assert!(store.push_clean().unwrap());
    assert!(store.invalid_derivations().unwrap().is_empty());
    push_root(&store, WS, hash(V2), 8, recompute(&store, None)).unwrap();
    for (output, _) in reads() {
        assert!(store.derivation_valid(output).unwrap(), "{output}");
    }
    dump(dir)
}

#[test]
fn a_direct_push_that_crashes_at_any_wave_is_recovered_to_the_clean_state() {
    let expected = clean();
    for crash in 1..=WAVES {
        let dir = directory("direct");
        let store = seeded(&dir);
        assert!(push_root(&store, WS, hash(V2), 8, recompute(&store, Some(crash))).is_err());
        drop(store);
        // Observe committed, so every crash leaves a derivation no rebound name will reach again.
        assert!(!invalid(&dump(&dir)).is_empty(), "crash at wave {crash}");
        assert_eq!(recovered(&dir), expected, "crash at wave {crash}");
    }
}

/// The store as a graph that dumps SQLite after each write: the direct push's write prefixes.
struct Prefixes<'a> {
    store: &'a ContentStore,
    dir: &'a Path,
    dumps: RefCell<Vec<Dump>>,
}

impl Graph for Prefixes<'_> {
    fn store(&self) -> &ContentStore {
        self.store
    }
    fn bind_names(&self, bindings: &[Binding]) -> StoreResult<Vec<String>> {
        let changed = self.store.bind_names(bindings);
        self.dumps.borrow_mut().push(dump(self.dir));
        changed
    }
    fn record_derivations(&self, records: &[DerivationRecord]) -> StoreResult<Vec<String>> {
        let changed = self.store.record_derivations(records);
        self.dumps.borrow_mut().push(dump(self.dir));
        changed
    }
    fn binding(&self, name: &str) -> StoreResult<Option<[u8; 32]>> {
        self.store.binding(name)
    }
    fn derivations_reading(&self, names: &[String]) -> StoreResult<Vec<Derivation>> {
        self.store.derivations_reading(names)
    }
    fn derivation_valid(&self, output: &str) -> StoreResult<bool> {
        self.store.derivation_valid(output)
    }
    fn begin_push(&self) -> StoreResult<()> {
        self.store.begin_push()
    }
    fn end_push(&self) -> StoreResult<()> {
        self.store.end_push()
    }
}

#[test]
fn an_overlay_push_that_crashes_leaves_a_write_prefix_and_is_recovered_to_the_clean_state() {
    let expected = clean();
    let prefixes = {
        let dir = directory("prefixes");
        let store = seeded(&dir);
        let graph = Prefixes { store: &store, dir: &dir, dumps: RefCell::new(vec![dump(&dir)]) };
        push_root(&graph, WS, hash(V2), 8, recompute(&graph, None)).unwrap();
        graph.dumps.into_inner()
    };
    // Observe's write and one per wave.
    assert_eq!(prefixes.len(), 2 + WAVES as usize);
    // A freeze per write; layers in memory, spilled, partly written back, and written by the
    // background writer.
    let cases = [
        ("memory", usize::MAX, usize::MAX, false),
        ("spilled", 0, usize::MAX, false),
        ("behind-1", usize::MAX, 1, false),
        ("spilled-behind-2", 0, 2, false),
        ("background", usize::MAX, 0, true),
    ];
    for (case, resident, behind, background) in cases {
        for crash in 1..=WAVES {
            let dir = directory(case);
            let store = Arc::new(seeded(&dir));
            let limits = Limits { top: 0, resident, behind, spill: dir.join("spill"), background };
            let overlay = Overlay::new(Arc::clone(&store), limits).unwrap();
            assert!(push_root(&overlay, WS, hash(V2), 8, recompute(&overlay, Some(crash))).is_err());
            let counters = overlay.crash();
            assert_eq!(counters.frozen, u64::from(crash), "{case} at wave {crash}");
            // SQLite holds exactly the first `written` writes of the same push through the store.
            assert_eq!(dump(&dir), prefixes[counters.written as usize], "{case} at wave {crash}");
            if behind == usize::MAX {
                assert_eq!(counters.written, 0, "{case}");
            }
            assert_eq!(recovered(&dir), expected, "{case} at wave {crash}");
        }
    }
}
