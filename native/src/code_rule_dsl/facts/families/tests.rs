use super::*;
use crate::code_rule_dsl::facts::observe::roots::{directory_root, git_root};
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

    /// The first push: every path is new, so its families are created; then the rule.
    fn start(&self, root: HashKey) {
        let ingested = self.families.borrow_mut().push(self.graph, WS, root, 8, |_| unreachable!("nothing reads the names yet")).unwrap();
        assert_eq!(ingested.created, 4, "tape, core, prefilter and kinds of a.py");
        self.graph.record_derivations(&[self.rule()]).unwrap();
    }

    fn push(&self, root: HashKey) -> (Ingested, Parses) {
        let mut families = Families::default();
        let ingested = families
            .push(self.graph, WS, root, 8, |frontier| {
                Ok(if frontier.iter().any(|d| d.output == RULE) { vec![self.rule()] } else { Vec::new() })
            })
            .unwrap();
        let parses = families.parses;
        *self.families.borrow_mut() = families;
        (ingested, parses)
    }
}

fn check(graph: &(impl Graph + ?Sized), roots: &[HashKey]) {
    let world = World::new(graph);
    world.start(roots[0]);
    assert_ne!(graph.binding(&kinds_name(WS, "a.py")).unwrap(), Some(ABSENT));

    // A rename keeps every kind's count: the tape, core and prefilter re-run and change, kinds
    // re-runs and cuts off.
    let (ingested, parses) = world.push(roots[1]);
    let work = ingested.push.work;
    assert_eq!((work.waves, work.rerun, work.changed, ingested.created), (3, 4, 3, 0));
    assert_eq!(
        (parses.parsed, parses.decoded, parses.reused),
        (1, 0, 3),
        "one encode; core and prefilter take the tape wave's tape, kinds the core wave's facts"
    );
    assert_eq!(world.rule_runs.get(), 1);

    // A call adds kinds: the histogram changes and the rule runs.
    let (ingested, _) = world.push(roots[2]);
    let work = ingested.push.work;
    assert_eq!((work.waves, work.rerun, work.changed), (4, 5, 5));
    assert_eq!(world.rule_runs.get(), 2);
    let families = world.families.borrow();
    assert!(families.facts.is_empty() && families.tapes.is_empty(), "every wave's readers took its tapes and facts");
    drop(families);
    for output in [
        tape_name(WS, "a.py"),
        core_name(WS, "a.py"),
        prefilter_name(WS, "a.py"),
        kinds_name(WS, "a.py"),
        RULE.to_owned(),
    ] {
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

/// Pushes the directory `dir` as `WS`'s new root through fresh families.
fn push_directory(store: &ContentStore, dir: &std::path::Path) -> (Ingested, Parses) {
    let root = directory_root(store, dir).unwrap();
    let mut families = Families::default();
    let ingested = families.push(store, WS, root, 8, |_| Ok(Vec::new())).unwrap();
    assert!(families.tapes.is_empty() && families.facts.is_empty(), "readers took every tape and facts");
    (ingested, families.parses)
}

fn rows(store: &ContentStore, sql: &str) -> i64 {
    store.query_rows(sql, [], |row| row.get(0)).unwrap()[0]
}

/// 06 S3's check: one blob edit re-encodes exactly one tape, and its unchanged kinds cut off.
#[test]
fn a_directory_edit_reencodes_one_tape_and_its_unchanged_kinds_cut_off() {
    let dir = temp("tree");
    std::fs::create_dir_all(dir.join("pkg")).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    std::fs::write(dir.join("pkg/b.py"), "def f():\n    return 2\n").unwrap();
    std::fs::write(dir.join("c.py"), "import os\n").unwrap();
    std::fs::write(dir.join("notes.unknown"), "not a language\n").unwrap();
    let store = ContentStore::open(&temp("ingest").with_extension("sqlite")).unwrap();

    // Cold: every path is new; each Python file gets its four derivations and its rows.
    let (cold, parses) = push_directory(&store, &dir);
    assert_eq!((cold.created, cold.push.work.rerun, parses.parsed), (12, 0, 3));
    assert_eq!(rows(&store, "SELECT count(*) FROM prefilter_sets"), 3);
    let kinds = store.binding(&kinds_name(WS, "a.py")).unwrap();

    // A rename in a.py: its tape alone is encoded; core and prefilter change, kinds cuts off.
    std::fs::write(dir.join("a.py"), "renamed = 1\n").unwrap();
    let (warm, parses) = push_directory(&store, &dir);
    let work = warm.push.work;
    assert_eq!((warm.push.rebound, warm.created), (2, 0), "a.py's path and the root tree");
    assert_eq!((work.waves, work.rerun, work.changed), (3, 4, 3));
    assert_eq!((parses.parsed, parses.decoded), (1, 0));
    assert_eq!(store.binding(&kinds_name(WS, "a.py")).unwrap(), kinds);
    assert_eq!(rows(&store, "SELECT count(*) FROM prefilter_sets"), 4, "the new tape's rows beside the old");

    // An unchanged snapshot rebinds nothing.
    let (same, parses) = push_directory(&store, &dir);
    assert_eq!((same.push.rebound, same.push.work.rerun, parses.parsed), (0, 0, 0));

    // A copy of b.py at a new path shares its stored tape: decoded, not encoded.
    std::fs::write(dir.join("d.py"), "def f():\n    return 2\n").unwrap();
    let (copy, parses) = push_directory(&store, &dir);
    assert_eq!((copy.created, parses.parsed, parses.decoded), (4, 0, 1));
    assert_eq!(store.binding(&tape_name(WS, "d.py")).unwrap(), store.binding(&tape_name(WS, "pkg/b.py")).unwrap());

    // Removing c.py unbinds its families down to the histogram.
    std::fs::remove_file(dir.join("c.py")).unwrap();
    let (removed, parses) = push_directory(&store, &dir);
    assert_eq!((removed.push.work.rerun, removed.push.work.changed, parses.parsed), (4, 4, 0));
    for name in [tape_name(WS, "c.py"), core_name(WS, "c.py"), prefilter_name(WS, "c.py"), kinds_name(WS, "c.py")] {
        assert_eq!(store.binding(&name).unwrap(), Some(ABSENT), "{name}");
    }
}

/// A clean checkout's snapshot is its `HEAD` tree, and pushing either binds the same names.
#[test]
fn a_clean_checkouts_snapshot_is_its_head_tree() {
    let dir = temp("git");
    std::fs::create_dir_all(dir.join("pkg")).unwrap();
    std::fs::write(dir.join("a.py"), "x = 1\n").unwrap();
    std::fs::write(dir.join("pkg/b.py"), "def f():\n    return 2\n").unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args(["-c", "user.name=rerail", "-c", "user.email=rerail@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(status.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&status.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "a.py", "pkg/b.py"]);
    git(&["commit", "-q", "-m", "fixture"]);
    let store = ContentStore::open(&temp("git-store").with_extension("sqlite")).unwrap();
    let head = git_root(&store, &dir).unwrap();
    assert_eq!(directory_root(&store, &dir).unwrap(), head);
    let mut families = Families::default();
    let ingested = families.push(&store, WS, head, 8, |_| Ok(Vec::new())).unwrap();
    assert_eq!((ingested.created, families.parses.parsed), (8, 2));
}
