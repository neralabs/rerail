//! Work counts (docs/facts/LOOP.md § Counts): deterministic, additive over files and plans, never
//! read by the evaluation (observation only).
//!
//! **Algebraic** counts are properties of the logical plan on the content, the same for every
//! engine that evaluates the plan exactly; `rerail_facts::reference` reports the same numbers
//! (`stage-check` and `pattern-check` assert it):
//!
//! - `leaf_rows`: rows of the leaf relations a relational clause reads (its input and syntax tables);
//! - `stage_rows`: rows emitted by its stages and final query (each stage a logical operator);
//! - `fix_rounds`, `fix_rows`: rounds of its top-level fixpoints (recursive CTEs outside expression
//!   subqueries; round 0 is the initial select) and rows admitted over them (set fixpoints: new
//!   rows; bag fixpoints: every derived row), per round in `StageCounts::fix`;
//! - `match_nodes`, `match_roots`, `input_rows`: per evaluation of a pattern input, the nodes its
//!   selection ranges over (the content's `node` rows: matcher node visits), the nodes it keeps,
//!   and the rows it emits.
//!
//! **Physical** counts are this engine's work for the plan, deterministic for a given plan and
//! content: `index_builds`/`index_rows` (hash indexes and rows inserted), `range_builds`/`range_rows`
//! (R14's sorted indexes), `probes` (hash lookups), `rows_scanned` (candidate rows a join level
//! visits), `rows_joined` (bindings out of a select's join pipeline), `rows_out` (rows of every
//! select's output table), `subquery_runs` (expression subqueries evaluated, cache misses),
//! `bounds_unproven` (fixpoints whose per-content bound check failed, W1: they ran under the
//! algebraic budget, docs/facts/PHASE-3.md § P3b; not work, a deterministic fact of the plan on
//! the content, kept with the physical counts).
//! The matcher's physical work (`pred_tests`, nodes tested by R12's predicates; `memo_hits`,
//! nodes R13's memo answered) is shared by a rule's inputs, so it is counted per input, not per
//! clause (`InputCounts`).
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::ops::AddAssign;

/// The engine's physical counters, indexed by `Phys`.
#[derive(Clone, Copy)]
pub enum Phys {
    IndexBuilds,
    IndexRows,
    RangeBuilds,
    RangeRows,
    Probes,
    RowsScanned,
    RowsJoined,
    RowsOut,
    SubqueryRuns,
    BoundsUnproven,
}
pub const NPHYS: usize = 10;

/// Per-execution counter cells (`sql::engine::Exec`), flushed into this thread's totals when the
/// execution ends.
#[derive(Default)]
pub struct Cells([Cell<u64>; NPHYS]);

impl Cells {
    #[inline]
    pub fn add(&self, k: Phys, n: u64) {
        let c = &self.0[k as usize];
        c.set(c.get() + n);
    }
    pub fn flush(&self) {
        TOTAL.with(|t| {
            for (a, b) in t.0.iter().zip(&self.0) {
                a.set(a.get() + b.take());
            }
        });
    }
}

thread_local! {
    /// This thread's physical totals since it started (read as differences).
    static TOTAL: Cells = Cells::default();
    /// Admitted rows per round of the top-level fixpoints since the last `take_fix` (summed by round).
    static FIX: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}

/// This thread's physical totals.
pub fn phys_now() -> [u64; NPHYS] {
    TOTAL.with(|t| std::array::from_fn(|i| t.0[i].get()))
}

/// Records a top-level fixpoint's admitted rows per round (round 0: the initial select).
pub fn fix(rounds: &[u64]) {
    FIX.with(|f| {
        let mut f = f.borrow_mut();
        if f.len() < rounds.len() {
            f.resize(rounds.len(), 0);
        }
        for (a, b) in f.iter_mut().zip(rounds) {
            *a += b;
        }
    });
}

/// The fixpoint rounds recorded since the last call.
pub fn take_fix() -> Vec<u64> {
    FIX.with(|f| std::mem::take(&mut *f.borrow_mut()))
}

/// One stage's counts in one run (`rel::run`).
#[derive(Default, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StageCounts {
    pub rows: u64,
    /// Admitted rows per round of the stage's top-level fixpoints, summed by round.
    pub fix: Vec<u64>,
    pub phys: [u64; NPHYS],
}

/// One clause's counts (one file's, or summed); see the module doc.
#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    pub leaf_rows: u64,
    pub stage_rows: u64,
    pub fix_rounds: u64,
    pub fix_rows: u64,
    pub match_nodes: u64,
    pub match_roots: u64,
    pub input_rows: u64,
    pub index_builds: u64,
    pub index_rows: u64,
    pub range_builds: u64,
    pub range_rows: u64,
    pub probes: u64,
    pub rows_scanned: u64,
    pub rows_joined: u64,
    pub rows_out: u64,
    pub subquery_runs: u64,
    #[serde(default)]
    pub bounds_unproven: u64,
}

/// The algebraic counts' names, in `Counts` order (the rest are physical).
pub const ALGEBRAIC: [&str; 7] = ["leaf_rows", "stage_rows", "fix_rounds", "fix_rows", "match_nodes", "match_roots", "input_rows"];
pub const PHYSICAL: [&str; NPHYS] = ["index_builds", "index_rows", "range_builds", "range_rows", "probes", "rows_scanned", "rows_joined", "rows_out", "subquery_runs", "bounds_unproven"];

impl Counts {
    pub fn add_phys(&mut self, p: &[u64; NPHYS]) {
        self.index_builds += p[Phys::IndexBuilds as usize];
        self.index_rows += p[Phys::IndexRows as usize];
        self.range_builds += p[Phys::RangeBuilds as usize];
        self.range_rows += p[Phys::RangeRows as usize];
        self.probes += p[Phys::Probes as usize];
        self.rows_scanned += p[Phys::RowsScanned as usize];
        self.rows_joined += p[Phys::RowsJoined as usize];
        self.rows_out += p[Phys::RowsOut as usize];
        self.subquery_runs += p[Phys::SubqueryRuns as usize];
        self.bounds_unproven += p[Phys::BoundsUnproven as usize];
    }

    pub fn add_stage(&mut self, s: &StageCounts) {
        self.stage_rows += s.rows;
        self.fix_rounds += s.fix.len() as u64;
        self.fix_rows += s.fix.iter().sum::<u64>();
        self.add_phys(&s.phys);
    }

    pub fn add_input(&mut self, i: &InputCounts) {
        self.match_nodes += i.nodes;
        self.match_roots += i.roots;
        self.input_rows += i.rows;
    }

    pub fn is_zero(&self) -> bool {
        *self == Counts::default()
    }
}

impl AddAssign<&Counts> for Counts {
    fn add_assign(&mut self, o: &Counts) {
        self.leaf_rows += o.leaf_rows;
        self.stage_rows += o.stage_rows;
        self.fix_rounds += o.fix_rounds;
        self.fix_rows += o.fix_rows;
        self.match_nodes += o.match_nodes;
        self.match_roots += o.match_roots;
        self.input_rows += o.input_rows;
        self.index_builds += o.index_builds;
        self.index_rows += o.index_rows;
        self.range_builds += o.range_builds;
        self.range_rows += o.range_rows;
        self.probes += o.probes;
        self.rows_scanned += o.rows_scanned;
        self.rows_joined += o.rows_joined;
        self.rows_out += o.rows_out;
        self.subquery_runs += o.subquery_runs;
        self.bounds_unproven += o.bounds_unproven;
    }
}

/// One evaluation of a pattern input: algebraic (`nodes`, `roots`, `rows`) and the matcher's
/// physical work (`pred_tests`, `memo_hits`; zero on the leaf executor, which keeps no count).
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputCounts {
    pub nodes: u64,
    pub roots: u64,
    pub rows: u64,
    pub pred_tests: u64,
    pub memo_hits: u64,
}

impl AddAssign<&InputCounts> for InputCounts {
    fn add_assign(&mut self, o: &InputCounts) {
        self.nodes += o.nodes;
        self.roots += o.roots;
        self.rows += o.rows;
        self.pred_tests += o.pred_tests;
        self.memo_hits += o.memo_hits;
    }
}
