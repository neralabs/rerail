//! W's reference graph's joins, counted (`RERAIL_W_JOINS=FILE`): each loop and lookup of W's Rust
//! (`calls::summarize`, the scope table, `closure::Graph::link_references`) as the join it is, so its work
//! can be set beside the rules' (`native.tsv`). Off, a count is one thread-local test.
//!
//! A site counts its calls, its steps (the rows or entries it touches: a walk's nodes, a search's probes,
//! a chain's entries scanned) and what it gives out. Counts are kept a thread at a time (`begin`), merged
//! into the run's (`flush`), and written as a table at the end (`write`).

use std::cell::{Cell, RefCell};
use std::sync::Arc;

/// A counted site of W's Rust.
#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub enum Site {
    ScopeVisit,
    ScopeBind,
    ScopeReference,
    ScopeSortReferences,
    SummaryWalk,
    SummaryDefinition,
    SummaryParentRead,
    ReferenceSearch,
    ResolveWalk,
    ResolveProbe,
    ResolveChain,
    Callee,
    Follow,
    FollowReference,
    Intern,
    Members,
    Stars,
    LinkTargets,
    LinkName,
    LinkMember,
    LinkMro,
    LinkLookup,
    LinkAliases,
    LinkPrefix,
    LinkUsers,
    RowsEdges,
    RowsTargets,
    ResolveEdges,
    Contribution,
}

/// Each site's name and the join it is.
pub const SITES: [(&str, &str); 29] = [
    ("scope.visit", "tree walk (a DFS, every node once)"),
    ("scope.bind", "hash insert: a scope's map by name, the binding pushed on the name's chain"),
    ("scope.reference", "append: a read's reference (range, scope)"),
    ("scope.sort_references", "sort of the references by start"),
    ("summary.walk", "tree walk (a DFS with a context stack: owner, receiver, class)"),
    ("summary.definition", "a definition made at its node (context pushed)"),
    ("summary.parent_read", "a parent's read and a field test (head of attribute, call target)"),
    ("summary.reference_search", "binary search of the sorted references by range"),
    ("resolve.walk", "scope chain walk: the read's scope, then parents"),
    ("resolve.probe", "hash probe: a scope's map by name"),
    ("resolve.chain", "scan of a name's bindings from the latest back"),
    ("summary.callee", "a call's callee: its function expression followed"),
    ("summary.follow", "a binding's value followed (an alias chain)"),
    ("summary.follow_reference", "a reference's target followed"),
    ("summary.intern", "hash probe and insert: a name's id"),
    ("summary.members", "loop over the classes' members"),
    ("summary.stars", "loop over the star imports"),
    ("link.targets", "loop over each file's call targets, hash probe of the links"),
    ("link.name", "per distinct target name: its link (string-keyed hash lookups)"),
    ("link.member", "hash probes: by_name, aliases, opaque"),
    ("link.mro", "a class's C3 order, by hash probes of its bases"),
    ("link.lookup", "per class of an order: a member's hash probe"),
    ("link.aliases", "hash probe: aliases of a declared, imported or prefix name"),
    ("link.prefix", "loop over a name's dotted prefixes, hash probes"),
    ("link.users", "loop over each file's targets: a link's users pushed"),
    ("rows.edges", "loop over each file's edges, a hash set test of its references"),
    ("rows.targets", "the edges out: each edge's targets"),
    ("resolve_edges", "loop over each file's edges, a link's definitions"),
    ("link.contribution", "a file's summary indexed (defs by name, aliases, modules)"),
];

thread_local! {
    static ON: Cell<bool> = const { Cell::new(false) };
    static TALLY: RefCell<[[u64; 3]; SITES.len()]> = const { RefCell::new([[0; 3]; SITES.len()]) };
}

/// The run's counts, merged from its threads.
#[derive(Default)]
struct Run(parking_lot::Mutex<Option<[[u64; 3]; SITES.len()]>>);

/// Whether the run counts W's joins.
pub fn on() -> bool {
    crate::code_rule_dsl::logic::profile::setting(|| crate::code_rule_dsl::logic::profile::var_os("RERAIL_W_JOINS").is_some())
}

/// Starts counting on this thread, where the run counts.
pub fn begin() {
    if on() {
        ON.with(|o| o.set(true));
    }
}

/// A call of `site`: `steps` rows or entries touched, `out` given out.
#[inline]
pub fn count(site: Site, steps: u64, out: u64) {
    if ON.with(Cell::get) {
        TALLY.with(|t| {
            let e = &mut t.borrow_mut()[site as usize];
            e[0] += 1;
            e[1] += steps;
            e[2] += out;
        });
    }
}

/// More of a call's work at `site`, counted as steps (and `out`) of a call already counted.
#[inline]
pub fn steps(site: Site, steps: u64, out: u64) {
    if ON.with(Cell::get) {
        TALLY.with(|t| {
            let e = &mut t.borrow_mut()[site as usize];
            e[1] += steps;
            e[2] += out;
        });
    }
}

/// Merges this thread's counts into the run's, and stops counting on it.
pub fn flush() {
    if !ON.with(Cell::get) {
        return;
    }
    ON.with(|o| o.set(false));
    let mine = TALLY.with(|t| std::mem::replace(&mut *t.borrow_mut(), [[0; 3]; SITES.len()]));
    let run: Arc<Run> = rerail_context::local::<Run>();
    let mut held = run.0.lock();
    let all = held.get_or_insert([[0; 3]; SITES.len()]);
    for (a, m) in all.iter_mut().zip(mine) {
        for k in 0..3 {
            a[k] += m[k];
        }
    }
}

/// `work` counted on each thread it runs on (`begin`, `flush`): for a loop W runs on several threads.
/// A thread already counting (the caller's own, where a parallel loop runs an item on it) goes on counting.
pub fn counted<R>(work: impl FnOnce() -> R) -> R {
    if ON.with(Cell::get) {
        return work();
    }
    begin();
    let r = work();
    flush();
    r
}

/// Writes the run's counts to `RERAIL_W_JOINS`'s file: a row a site, `site`, `join`, `calls`, `steps`, `out`.
pub fn write() {
    let Some(path) = crate::code_rule_dsl::logic::profile::var_os("RERAIL_W_JOINS") else { return };
    flush();
    let run: Arc<Run> = rerail_context::local::<Run>();
    let held = run.0.lock();
    let all = held.unwrap_or([[0; 3]; SITES.len()]);
    let mut out = String::from("site\tjoin\tcalls\tsteps\tout\n");
    for ((name, join), [calls, steps, given]) in SITES.iter().zip(all) {
        out.push_str(&format!("{name}\t{join}\t{calls}\t{steps}\t{given}\n"));
    }
    if let Err(e) = std::fs::write(&path, out) {
        eprintln!("RERAIL_W_JOINS: {}: {e}", std::path::Path::new(&path).display());
    }
}
