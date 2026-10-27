use super::*;
use crate::code_rule_dsl::facts::observe::{Push, push_root};
use crate::code_rule_dsl::facts::overlay::{Limits, Overlay};
use crate::content_store::{ContentStore, GitTree, GitTreeEntry};
use std::cell::{Cell, RefCell};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(what: &str) -> std::path::PathBuf {
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    std::env::temp_dir().join(format!("rerail-families-{what}-{}-{nonce}", std::process::id()))
}

const WS: &str = "/repo";
const RULE: &str = "rule:kinds-of-a";

/// a.py's contents, each stored under a Git blob id, and a root tree holding each.
fn contents(store: &ContentStore) -> Vec<HashKey> {
    let mut roots = Vec::new();
    for (n, source) in [(1u8, "x = 1\n"), (2, "renamed = 1\n"), (3, "renamed = f(1)\n")] {
        let blob = HashKey::git_sha1([n; 20]);
        store.put_source(source.as_bytes(), &[blob]).unwrap();
        let entry = GitTreeEntry { name: "a.py".to_owned(), mode: 0o100644, target: blob };
        let tree = GitTree { id: HashKey::git_sha1([100 + n; 20]), prefix: String::new(), entries: vec![entry] };
        store.put_git_trees_bulk(&[tree.clone()]).unwrap();
        roots.push(tree.id);
    }
    roots
}

/// The families plus one rule that reads a.py's kind histogram, over `graph`.
struct World<'g, G: Graph + ?Sized> {
    graph: &'g G,
    rule_runs: Cell<usize>,
    families: RefCell<Families>,
}

impl<'g, G: Graph + ?Sized> World<'g, G> {
    fn new(graph: &'g G) -> Self {
        Self { graph, rule_runs: Cell::new(0), families: RefCell::default() }
    }

    fn rule(&self) -> DerivationRecord {
        self.rule_runs.set(self.rule_runs.get() + 1);
        let name = kinds_name(WS, "a.py");
        let digest = self.graph.binding(&name).unwrap().unwrap();
        DerivationRecord {
            output: RULE.to_owned(),
            producer: [9; 32],
            inputs: vec![DerivationInput { name: Some(name), digest }],
            digest,
            status: 0,
            cost: 1,
        }
    }

    /// The first push: observe, then derive core, kinds and the rule from scratch.
    fn start(&self, root: HashKey) {
        push_root(self.graph, WS, root, 8, |_| unreachable!("nothing reads the names yet")).unwrap();
        let mut families = self.families.borrow_mut();
        let (core, _) = families.core(self.graph, WS, "a.py").unwrap();
        self.graph.record_derivations(&[core]).unwrap();
        let kinds = families.kinds(self.graph, WS, "a.py").unwrap();
        self.graph.record_derivations(&[kinds]).unwrap();
        drop(families);
        self.graph.record_derivations(&[self.rule()]).unwrap();
    }

    fn push(&self, root: HashKey) -> (Push, Parses) {
        *self.families.borrow_mut() = Families::default();
        let push = push_root(self.graph, WS, root, 8, |frontier| {
            let mut records = self.families.borrow_mut().recompute(self.graph, frontier)?;
            if frontier.iter().any(|d| d.output == RULE) {
                records.push(self.rule());
            }
            Ok(records)
        })
        .unwrap();
        (push, self.families.borrow().parses)
    }
}

fn check(graph: &(impl Graph + ?Sized), roots: &[HashKey]) {
    let world = World::new(graph);
    world.start(roots[0]);
    assert_ne!(graph.binding(&kinds_name(WS, "a.py")).unwrap(), Some(ABSENT));

    // A rename keeps every kind's count: core re-runs and changes, kinds re-runs and cuts off.
    let (push, parses) = world.push(roots[1]);
    assert_eq!((push.work.waves, push.work.rerun, push.work.changed), (2, 2, 1));
    assert_eq!((parses.parsed, parses.reused), (1, 1), "the kinds wave reads the core wave's facts");
    assert_eq!(world.rule_runs.get(), 1);

    // A call adds kinds: the histogram changes and the rule runs.
    let (push, _) = world.push(roots[2]);
    assert_eq!((push.work.waves, push.work.rerun, push.work.changed), (3, 3, 3));
    assert_eq!(world.rule_runs.get(), 2);
    for output in [core_name(WS, "a.py"), kinds_name(WS, "a.py"), RULE.to_owned()] {
        assert!(graph.derivation_valid(&output).unwrap(), "{output}");
    }
}

#[test]
fn a_rename_stops_at_the_kind_histogram_and_a_new_kind_reaches_its_reader() {
    let store = ContentStore::open(&temp("store").with_extension("sqlite")).unwrap();
    let roots = contents(&store);
    check(&store, &roots);
}

#[test]
fn the_families_propagate_the_same_over_the_overlay() {
    let store = Arc::new(ContentStore::open(&temp("overlay").with_extension("sqlite")).unwrap());
    let roots = contents(&store);
    let limits = Limits { top: 1, resident: 1 << 20, behind: 4, spill: temp("spill"), background: true };
    let overlay = Overlay::new(Arc::clone(&store), limits).unwrap();
    check(&overlay, &roots);
    overlay.flush().unwrap();
    assert!(store.derivation_valid(RULE).unwrap());
}
