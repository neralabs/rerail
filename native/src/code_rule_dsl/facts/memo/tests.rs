use super::*;
use crate::content_store::ContentStore;
use crate::code_rule_dsl::facts::observe::push_root;
use crate::content_store::{Derivation, GitTree, GitTreeEntry, HashKey};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

fn store() -> ContentStore {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let path = std::env::temp_dir().join(format!("rerail-facts-derivation-{}-{nonce}.sqlite", std::process::id()));
    ContentStore::open(&path).unwrap()
}

fn blob(n: u8) -> [u8; 32] {
    let mut digest = [0; 32];
    digest[..20].fill(n);
    digest
}

fn root(store: &ContentStore, n: u8, a: u8) -> HashKey {
    let entry = GitTreeEntry { name: "a.py".to_owned(), mode: 0o100644, target: HashKey::git_sha1([a; 20]) };
    let tree = GitTree { id: HashKey::git_sha1([n; 20]), prefix: String::new(), entries: vec![entry] };
    store.put_git_trees_bulk(&[tree.clone()]).unwrap();
    tree.id
}

const WS: &str = "/repo";

/// A toy workspace: a.py's blobs, the facts each content reports, and one rule reading a.py's
/// facts. Counts clause runs, memo hits and rule runs.
struct World {
    store: ContentStore,
    set: FactsPlanSet,
    sources: HashMap<[u8; 32], (SourceId, &'static [u8])>,
    runs: RefCell<Vec<&'static [u8]>>,
    hits: RefCell<usize>,
    rule_runs: RefCell<usize>,
}

impl World {
    fn new() -> Self {
        let store = store();
        let mut sources = HashMap::new();
        for (n, bytes) in [(1, &b"x = 1\n"[..]), (2, b"x = 1  \n"), (3, b"x = 2\n")] {
            let content = HashKey::sha256(bytes).sha256_bytes().unwrap();
            let source = store.put_facts_source(bytes, content, &[]).unwrap();
            sources.insert(blob(n), (source, bytes));
        }
        let set = FactsPlanSet::new(vec![[1; 32]]);
        World { store, set, sources, runs: RefCell::default(), hits: RefCell::default(), rule_runs: RefCell::default() }
    }

    fn file(&self) -> FileFacts<'_> {
        FileFacts { scope: WS, path: "a.py", set: &self.set, fact_schema: 1 }
    }

    /// The clause: a finding naming the assigned value, whitespace ignored.
    fn facts(&self) -> DerivationRecord {
        let digest = self.store.binding(&path_name(WS, "a.py")).unwrap().unwrap();
        let (source, bytes) = self.sources[&digest];
        let evaluated = evaluate(&self.store, self.file(), source, || {
            self.runs.borrow_mut().push(bytes);
            let value = String::from_utf8_lossy(bytes).trim().to_owned();
            Ok((vec![FactsResult { plan: [1; 32], path_dependent: false, entry: value.into_bytes() }], vec![]))
        })
        .unwrap();
        *self.hits.borrow_mut() += usize::from(evaluated.hit);
        evaluated.record
    }

    fn rule(&self) -> DerivationRecord {
        *self.rule_runs.borrow_mut() += 1;
        let facts = self.file().output();
        let digest = self.store.binding(&facts).unwrap().unwrap();
        DerivationRecord {
            output: "rule:r".to_owned(),
            producer: [8; 32],
            inputs: vec![DerivationInput { name: Some(facts), digest }],
            digest,
            status: 0,
            cost: 1,
        }
    }

    fn recompute(&self, frontier: &[Derivation]) -> StoreResult<Vec<DerivationRecord>> {
        Ok(frontier.iter().map(|d| if d.output == "rule:r" { self.rule() } else { self.facts() }).collect())
    }

    fn push(&self, n: u8, a: u8) -> super::super::observe::Push {
        let root = root(&self.store, n, a);
        push_root(&self.store, WS, root, 8, |frontier| self.recompute(frontier)).unwrap()
    }
}

#[test]
fn a_blob_edit_with_the_same_facts_stops_at_the_facts_and_a_seen_content_is_a_lookup() {
    let world = World::new();
    world.push(100, 1);
    let facts = world.facts();
    world.store.record_derivations(&[facts]).unwrap();
    let rule = world.rule();
    world.store.record_derivations(&[rule]).unwrap();
    assert_eq!(*world.rule_runs.borrow(), 1);

    // Whitespace only: the clause re-runs on the new content, reports the same, and cuts off.
    let push = world.push(101, 2);
    assert_eq!((push.work.waves, push.work.rerun, push.work.changed), (1, 1, 0));
    assert_eq!(*world.rule_runs.borrow(), 1);

    // A real change reaches the rule.
    let push = world.push(102, 3);
    assert_eq!((push.work.waves, push.work.rerun, push.work.changed), (2, 2, 2));
    assert_eq!(*world.rule_runs.borrow(), 2);

    // Back to the first content: the memo answers without running the clause.
    let push = world.push(103, 1);
    assert_eq!((push.work.waves, push.work.rerun), (2, 2));
    assert_eq!(world.runs.borrow().as_slice(), [&b"x = 1\n"[..], b"x = 1  \n", b"x = 2\n"]);
    assert_eq!(*world.hits.borrow(), 1);
    assert!(world.store.derivation_valid(&world.file().output()).unwrap());
    assert!(world.store.derivation_valid("rule:r").unwrap());
}
