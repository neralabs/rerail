//! The dead-code families on `propagate`, over a stand-in for Y1 and the symbol table (each
//! edit's `Refs` and symbol-table rows given by the test), on the store and on the overlay, in
//! both modes: what runs, what cuts off, and exactness against Y2 from scratch after every edit.
use super::*;
use crate::code_rule_dsl::facts::lmap::LMap;
use crate::code_rule_dsl::facts::overlay::{Limits, Overlay};
use crate::code_rule_dsl::facts::y2::dc::Rule;
use crate::code_rule_dsl::facts::y2::facts::{sym, Facts, Row};
use crate::code_rule_dsl::facts::y3::sym::SymState;
use crate::code_rule_dsl::facts::ygraph::world::{Delta, Step as Y1Step};
use crate::content_store::ContentStore;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp(label: &str) -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("rerail-deadcode-{label}-{}-{nonce}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn s(x: &str) -> Val {
    Val::S(sym(x))
}

fn i(n: i64) -> Val {
    Val::Int(n)
}

/// A function `m.{name}` in `m.py` at line `start` (`dc__dead_traits`' 20 columns).
fn def(name: &str, start: i64) -> Row {
    let id = format!("m.{name}");
    Box::new([s("m.py"), i(start), i(start + 5), s(name), Val::Null, Val::Null, Val::Null, Val::Null, s("m"), s("function"), s(&id), s(&id), Val::Null, s(name), Val::Null, i(0), i(0), i(0), i(0), i(0)])
}

/// A reference from the definition at line `from` to the one at line `to`.
fn call(from: i64, to: i64) -> Row {
    Box::new([s("m.py"), i(from), i(0), s("m.py"), i(to)])
}

fn change(tab: Tab, row: Row, added: bool) -> Change {
    Change { tab, row, added }
}

/// Stands in for Y1 and the symbol table: each content of `m.py` has its rows given here.
struct Script {
    top: Top,
    /// (new content) → (`Refs` changes, symbol-table changes)
    steps: FxHashMap<String, (Vec<Change>, Vec<Change>)>,
}

impl Inputs for Script {
    fn top(&self, _: u32) -> &Top {
        &self.top
    }
    fn top_mut(&mut self, _: u32) -> &mut Top {
        &mut self.top
    }
    fn xref(&mut self, _: u32, edits: &[&PathEdit]) -> wire::Xref {
        let mut refs = Vec::new();
        for (_, _, n) in edits {
            refs.extend(self.steps[n.as_deref().unwrap()].0.iter().cloned());
        }
        self.top.refs_digest.changes(&refs, &mut Vec::new());
        wire::Xref { refs, y1: Y1Step::default(), refs_ms: 0.0 }
    }
    fn symbols(&mut self, _: u32, edits: &[&PathEdit]) -> (Vec<Change>, sym::Work) {
        let mut out = Vec::new();
        for (p, _, n) in edits {
            self.top.members.insert(sym(p), sym(n.as_deref().unwrap()));
            out.extend(self.steps[n.as_deref().unwrap()].1.iter().cloned());
        }
        self.top.sym_digest.changes(&out, &mut Vec::new());
        (out, sym::Work::default())
    }
    fn symbol_rows(&self, _: u32) -> Option<Vec<(Tab, Box<[Val]>)>> {
        None
    }
}

/// `m.f` (exported) calls `m.g`; `m.h` is dead. Then: the call moves to `h` (v2), a new dead
/// function `k` (v3), an edit Y1 and the table see as nothing (v4).
fn script() -> Script {
    let rows = vec![
        change(Tab::DeadTraits, def("f", 1), true),
        change(Tab::DeadTraits, def("g", 10), true),
        change(Tab::DeadTraits, def("h", 20), true),
        change(Tab::Roots, Box::new([s("m.py"), s("m"), s("f"), s("m.f")]), true),
    ];
    let refs = vec![change(Tab::Refs, call(1, 10), true)];
    let mut facts = Facts::default();
    let mut all = refs.clone();
    all.extend(rows.iter().cloned());
    facts.apply(&all);
    let rule = Rule::scratch(&facts);
    let mut members = LMap::default();
    members.insert(sym("m.py"), sym("v1"));
    let mut top = Top { y1: Delta::default(), sym: SymState::default(), facts, rule: Some(rule), rev: LMap::default(), members, refs_digest: SetDigest::default(), sym_digest: SetDigest::default() };
    top.refs_digest.changes(&refs, &mut Vec::new());
    top.sym_digest.changes(&rows, &mut Vec::new());
    let mut steps = FxHashMap::default();
    steps.insert("v2".to_owned(), (vec![change(Tab::Refs, call(1, 10), false), change(Tab::Refs, call(1, 20), true)], vec![]));
    steps.insert("v3".to_owned(), (vec![], vec![change(Tab::DeadTraits, def("k", 30), true)]));
    steps.insert("v4".to_owned(), (vec![], vec![]));
    Script { top, steps }
}

fn dead(dc: &DeadCode<Script>) -> Vec<String> {
    dc.finding_rows(1).iter().map(|r| string(r[3].s())).collect()
}

/// Every edit through `propagate`; returns the bound digests of the five outputs at the end.
fn run(graph: &(impl Graph + ?Sized), mode: Mode, label: &str) -> Vec<Option<[u8; 32]>> {
    let sc = script();
    let (base, _) = Base::build(&sc.top.facts, &temp(label).join("base"));
    let mut dc = DeadCode::new(sc, base, mode);
    dc.create(graph, 1).unwrap();
    assert_eq!(dead(&dc), ["m.h"]);
    assert_eq!(dc.check(1), Vec::<String>::new());
    assert_eq!(dc.bindings_differ(graph, 1).unwrap(), Vec::<String>::new());
    let edit = |n: &str, o: &str| vec![("m.py".to_owned(), Some(o.to_owned()), Some(n.to_owned()))];

    // The call moves: the graph and the reached set change, so the findings run twice (with the
    // graph, then after the reached set).
    let w = dc.push(graph, 1, &edit("v2", "v1"), 16).unwrap();
    assert_eq!(dead(&dc), ["m.g"]);
    assert_eq!((w.waves, w.rerun, w.changed), (4, 6, 4), "{w:?}");
    assert_eq!(dc.check(1), Vec::<String>::new());
    assert_eq!(dc.bindings_differ(graph, 1).unwrap(), Vec::<String>::new());

    // A new function no state reaches: no edge changes, so the graph cuts off and the reached
    // set never runs; the findings do (they read the symbol table).
    dc.step = Step::default();
    let w = dc.push(graph, 1, &edit("v3", "v2"), 16).unwrap();
    assert_eq!(dead(&dc), ["m.g", "m.k"]);
    assert_eq!((w.waves, w.rerun, w.changed), (2, 4, 2), "{w:?}");
    assert_eq!(dc.step.runs, [1, 1, 1, 0, 1]);
    assert_eq!(dc.check(1), Vec::<String>::new());

    // Nothing changes for Y1 or the table: both cut off in the first wave.
    let w = dc.push(graph, 1, &edit("v4", "v3"), 16).unwrap();
    assert_eq!((w.waves, w.rerun, w.changed), (1, 2, 0), "{w:?}");
    assert_eq!(dc.check(1), Vec::<String>::new());
    assert_eq!(dc.bindings_differ(graph, 1).unwrap(), Vec::<String>::new());
    FAMILIES.into_iter().map(|f| graph.binding(&f.name(&scope_of(1))).unwrap()).collect()
}

#[test]
fn dead_code_families_cut_off_and_equal_y2_on_the_store_and_the_overlay_in_both_modes() {
    let mut outputs = Vec::new();
    for mode in [Mode::Maintained, Mode::Recompute { threads: 1 }, Mode::Recompute { threads: 3 }] {
        let dir = temp("store");
        let store = ContentStore::open(&dir.join("store.sqlite")).unwrap();
        outputs.push(run(&store, mode, "store"));
        let store = Arc::new(ContentStore::open(&dir.join("overlay.sqlite")).unwrap());
        let limits = Limits { top: 1 << 20, resident: 64 << 20, behind: 8, spill: dir.join("spill"), background: true };
        let overlay = Overlay::new(Arc::clone(&store), limits).unwrap();
        outputs.push(run(&overlay, mode, "overlay"));
        overlay.flush().unwrap();
    }
    // One value, whatever the graph or the mode.
    assert!(outputs.windows(2).all(|w| w[0] == w[1]), "{outputs:?}");
}
