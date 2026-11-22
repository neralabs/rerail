//! The planner's vocabulary, below its search (`plan.rs`, rerail-planner's): the planner's errors, a rule's steps
//! and their modes, a join's order and signature, a plan's key, the constants the body reads, the cells and digests
//! of stored rows, the SQL expressions for constants, and the planner's counts. `plan.rs` re-exports it (through
//! rerail-plan-base's `plan`) under its old names.

use super::*;

/// A planning or evaluation failure.
#[derive(Debug)]
pub enum Error {
    Front(super::super::front::Error),
    Sql(rusqlite::Error),
    Cache(crate::content_store::StoreError),
    /// The facts have no planner statistics, or statistics of other data.
    Statistics(String),
    /// A rule the planner cannot order or lower.
    Plan(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Front(e) => write!(f, "{e}"),
            Error::Sql(e) => write!(f, "{e}"),
            Error::Cache(e) => write!(f, "{e}"),
            Error::Statistics(m) | Error::Plan(m) => f.write_str(m),
        }
    }
}

impl From<crate::content_store::StoreError> for Error {
    fn from(error: crate::content_store::StoreError) -> Self {
        Self::Cache(error)
    }
}

impl From<rusqlite::Error> for Error {
    fn from(e: rusqlite::Error) -> Self {
        Error::Sql(e)
    }
}

impl From<super::super::front::Error> for Error {
    fn from(e: super::super::front::Error) -> Self {
        Error::Front(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// A constant of a rule or of the statistics (`stat_val.val`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Constant {
    Int(i64),
    Str(String),
}

/// How a join literal is read: a range inside one document, or a `nearest` climb or descent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Range,
    Climb,
    Descend,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Range => "range",
            Mode::Climb => "climb",
            Mode::Descend => "descend",
        }
    }
}

/// A planned body literal: its position in the body it was planned for (T compared literals by identity), and its
/// mode. A step names its literal and holds none (decision 0173): it is read beside its body, by `Order`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub at: usize,
    pub mode: Option<Mode>,
}

/// A plan read beside the rule it orders (`Body`, by handles): each step with its literal. Every reader of a plan's
/// literals reads them here (`view`), so none indexes a body itself.
#[derive(Clone, Copy)]
pub struct Order<'a, 'v> {
    pub b: &'a Body<'v>,
    pub steps: &'a [Step],
}

impl<'a, 'v> Order<'a, 'v> {
    pub fn new(b: &'a Body<'v>, steps: &'a [Step]) -> Order<'a, 'v> {
        Order { b, steps }
    }
    /// Step `s`'s literal.
    pub fn view(&self, s: &Step) -> Lit<'v> {
        self.b.view(s.at)
    }
    /// The literals in step order.
    pub fn lits(&self) -> impl Iterator<Item = Lit<'v>> + '_ {
        self.steps.iter().map(|s| self.view(s))
    }
    pub fn iter(&self) -> std::slice::Iter<'a, Step> {
        self.steps.iter()
    }
    pub fn len(&self) -> usize {
        self.steps.len()
    }
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
    /// The plan of its first `n` steps.
    pub fn take(&self, n: usize) -> Order<'a, 'v> {
        Order { b: self.b, steps: &self.steps[..n.min(self.steps.len())] }
    }
}

/// An atom's binding pattern: its relation, and which columns are bound, constants by value (T's `atom_sig`).
pub type Sig = (String, Vec<(usize, Option<Constant>)>);

/// A kept plan's key: a digest of the rule, the delta and the component solved (`derived::digest`). Not the rule's
/// address: each chunk's program is a new value (the demand split makes one per chunk), so no address is kept between
/// chunks. Not the rule's text either: formatting every rule was a tenth of a session run's planning.
pub type PlanKey = u128;

/// A step as the store keeps it: its position in the body, and its mode.
pub type StoredStep = (u16, Option<Mode>);

pub fn integer(i: i64) -> Expr {
    Expr::Value(Value_::Integer(i))
}

pub fn string(s: &str) -> Expr {
    Expr::Value(Value_::Text(s.to_owned()))
}

/// A relation's column types as the store keeps its rows: `s` where it declares a symbol, whatever the engine holds
/// the column as (over atoms, an integer). The store maps a forked process's symbols by them (decision 0170); rows of
/// another width than declared keep the engine's types.
pub fn stored_types(f: Fields<'_>, rel: &str, engine: Vec<u8>) -> Vec<u8> {
    let declared = f.types(rel);
    if declared.len() != engine.len() {
        return engine;
    }
    declared.iter().map(|t| if *t == "sym" { b's' } else { b'i' }).collect()
}

/// How a digest reads a relation's cells (decision 0212): an atom column (`b's'`, as frames type it) by its atoms'
/// content keys (`Atoms::content_keys`: their strings' hashes), any other as it is. So a kept frame's digest, and the
/// read digests its readers record, are of its rows' content: the same in every process, whichever ids it gave the
/// strings. By id, a process whose atoms forked from the store's (decision 0170) digested the same rows otherwise, and
/// its frames, keyed by its own numbering, could not be written at all: every next run derived them again.
pub struct Cells<'a> {
    atoms: Option<&'a crate::content_store::atoms::Atoms>,
    atom: Vec<usize>,
    width: usize,
}

impl<'a> Cells<'a> {
    /// The cells of rows of column `types` (`b's'` an atom), whose atoms are `atoms`'.
    pub fn new(atoms: &'a crate::content_store::atoms::Atoms, types: &[u8]) -> Self {
        let atom: Vec<usize> = types.iter().enumerate().filter(|(_, t)| **t == b's').map(|(c, _)| c).collect();
        Cells { atoms: (!atom.is_empty()).then_some(atoms), atom, width: types.len() }
    }

    /// The cells of rows of `width` columns that hold no atom.
    pub fn plain(width: usize) -> Self {
        Cells { atoms: None, atom: Vec::new(), width }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    /// `rows` (whole rows) with each atom cell replaced by its content key.
    pub fn key(&self, rows: &mut [i64]) {
        let Some(atoms) = self.atoms else { return };
        let w = self.width.max(1);
        let mut cells: Vec<i64> = Vec::with_capacity(rows.len() / w * self.atom.len());
        for row in rows.chunks_exact(w) {
            cells.extend(self.atom.iter().map(|&c| row[c]));
        }
        atoms.content_keys(&mut cells);
        let mut it = cells.into_iter();
        for row in rows.chunks_exact_mut(w) {
            for &c in &self.atom {
                row[c] = it.next().expect("a key per atom cell");
            }
        }
    }

    /// The sum of `rows`' row digests (`row_digest`), keyed (`key`) a chunk at a time: no copy of all of them. Many
    /// rows are summed in parts at once (a wrapping sum: the same in any order).
    pub fn sum(&self, rows: &[i64]) -> u64 {
        let w = self.width;
        if w == 0 {
            return 0;
        }
        const PART: usize = 1 << 15;
        if rows.len() / w >= 2 * PART {
            use rayon::prelude::*;
            return super::super::profile::on_cores(|| rows.par_chunks(PART * w).map(|part| self.sum_one(part)).reduce(|| 0, u64::wrapping_add));
        }
        self.sum_one(rows)
    }

    /// `rows_digest` of `n` rows read a batch at a time where they lie (`batches(lo, hi, f)`: the rows of ids
    /// `lo..hi` of `ids`, `f` per batch), with no copy of them all: many ids are summed in parts at once.
    pub fn digest_batches(&self, ids: usize, n: usize, batches: &(dyn Fn(usize, usize, &mut dyn FnMut(&[i64])) + Sync)) -> u64 {
        const PART: usize = 1 << 15;
        let part = |lo: usize| {
            let mut s = 0u64;
            batches(lo, (lo + PART).min(ids), &mut |rows| s = s.wrapping_add(self.sum_one(rows)));
            s
        };
        let sum = if self.width == 0 {
            0
        } else if ids >= 2 * PART {
            use rayon::prelude::*;
            super::super::profile::on_cores(|| (0..ids).into_par_iter().step_by(PART).map(part).reduce(|| 0, u64::wrapping_add))
        } else {
            part(0)
        };
        sum ^ (n as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
    }

    /// `sum` on this thread.
    fn sum_one(&self, rows: &[i64]) -> u64 {
        let w = self.width;
        if self.atoms.is_none() {
            return rows.chunks_exact(w).fold(0u64, |s, r| s.wrapping_add(row_digest(r)));
        }
        let mut sum = 0u64;
        let mut buf: Vec<i64> = Vec::new();
        for chunk in rows.chunks(w * 4096) {
            buf.clear();
            buf.extend_from_slice(&chunk[..chunk.len() / w * w]);
            self.key(&mut buf);
            sum = buf.chunks_exact(w).fold(sum, |s, r| s.wrapping_add(row_digest(r)));
        }
        sum
    }

    /// One row's digest (`row_digest`) of its content.
    pub fn row(&self, row: &[i64]) -> u64 {
        if self.atoms.is_none() {
            return row_digest(row);
        }
        let mut r = row.to_vec();
        self.key(&mut r);
        row_digest(&r)
    }

    /// Each of `rows`' digests (`row`), keyed under one read of the atoms.
    pub fn rows(&self, rows: &[i64]) -> Vec<u64> {
        let w = self.width.max(1);
        if self.atoms.is_none() {
            return rows.chunks_exact(w).map(row_digest).collect();
        }
        let mut r = rows.to_vec();
        self.key(&mut r);
        r.chunks_exact(w).map(row_digest).collect()
    }
}

/// An order-independent digest of rows (a set) of their content (`Cells`), as `corpus::retain::digest` takes a
/// source's.
pub fn rows_digest(cells: &Cells, rows: &[i64], n: usize) -> u64 {
    cells.sum(rows) ^ (n as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
}

/// A value of a decoded row before it is held (`decoded_outputs`).
pub enum Shown {
    Int(i64),
    Text(String),
}

impl Shown {
    pub fn cell(&self) -> output::Cell<'_> {
        match self {
            Shown::Int(v) => output::Cell::Int(*v),
            Shown::Text(s) => output::Cell::Text(s),
        }
    }
}

/// The run's counts of this module's work (decision 0442: a run's own, `run::local`).
#[derive(Default)]
pub struct PlanCounts {
    /// Relations whose statistics their layouts answered (`Model::resolve`, decision 0361 step 4): none was made.
    pub shaped: std::sync::atomic::AtomicU64,
    /// The kept-plan outcomes of `plan_rule_reused` in this process, by `CacheOutcome`: `RERAIL_PLAN_CACHE_COUNTS`
    /// prints them at the end of each corpus run (`KeepPlans`).
    pub plan_cache: [std::sync::atomic::AtomicU64; 12],
    /// Why a kept plan was not taken as it is (decision 0122), under `RERAIL_PLAN_CACHE_COUNTS`: no price kept, never
    /// ran, ran over a search's work, or its inputs grew past what that allows.
    pub budget_miss: [std::sync::atomic::AtomicU64; 4],
    /// Decision 0123's statistics, under `RERAIL_PLAN_CACHE_COUNTS`: batches made, relations they made, relations made
    /// late (one line at a time, `Model::late`).
    pub statistics: [std::sync::atomic::AtomicU64; 3],
    /// Orders made, by how (`RERAIL_PLAN_CACHE_COUNTS`): searched, and taken greedily under a search's cost (decision 0123
    /// §7).
    pub searches: [std::sync::atomic::AtomicU64; 2],
    /// Decision 0123 §9's probes in this process: left unplanned when their stratum opened, and planned later (when their
    /// delta came, or when the device took their stratum). Never reset, so a test reads its own run's by difference.
    pub lazy: [std::sync::atomic::AtomicU64; 2],
    /// Publications of new plans and of new orders (decision 0301), and the entries they carried: for the profile.
    pub published: [std::sync::atomic::AtomicU64; 2],
    /// Answers a paused component's requests and adds the answers to its produced relations (decision 0065 §2a).
    /// Rows for a produced relation outside it are kept for the stratum that holds that relation.
    /// Time spent in answer hooks, and their calls (`RERAIL_STRATA_COUNTS` prints them): a measurement.
    pub answer_wait: std::sync::atomic::AtomicU64,
    pub answer_calls: std::sync::atomic::AtomicU64,
    /// The strata the process held as views of a bound input's columns (`Run::input_view`, Q37), not derived.
    pub input_views: std::sync::atomic::AtomicU64,
    /// Tests: the programs the run normalized (`Placed::normalized`).
    #[cfg(any(test, feature = "test-hooks"))]
    pub normalized: std::sync::atomic::AtomicU64,
    /// Strata the evaluator's maintain mode maintained (decision 0364 stage 3), over the process.
    pub mode_strata: std::sync::atomic::AtomicU64,
    /// How often strata suspended on requests, and for how long in all (`RERAIL_STRATA_COUNTS`, the profile).
    pub ask_suspensions: std::sync::atomic::AtomicU64,
    pub ask_suspended: std::sync::atomic::AtomicU64,
}

/// The run's `PlanCounts`.
#[inline]
pub fn run_counts() -> std::sync::Arc<PlanCounts> {
    crate::code_rule_dsl::logic::run::local::<PlanCounts>()
}

/// One row's part of `rows_digest`, a sum of them: a relation's digest is patched by its rows gone and added.
pub fn row_digest(row: &[i64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for v in row {
        h = (h ^ *v as u64).wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    h.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

/// Plans kept across a corpus run's chunks, in the run's context (decision 0442: its jobs plan with them): per rule
/// (head, body, delta, the component solved), its order and its work per node when planned. `None` outside a corpus
/// run: every plan is searched. When profiling, the plan as EXPLAIN showed it when searched: a re-pricing reports it
/// with its own work and rows. The planner's search binds them (`search::SharedPlans`, `in_view`, `KeepPlans`) and
/// reads its own (`with_plans`); the joins read their chunk's memo (`KeptView::chunk_memo`).
#[derive(Default)]
pub struct KeptPlans(pub Option<Box<dyn KeptView>>);

/// What the planner's base reads of the run's kept plans; the search's (`SharedPlans`) are the ones there are.
pub trait KeptView: std::any::Any + Send + Sync {
    /// The joins' memo of the chunk being planned (Q261): its view's, where the run is in one.
    fn chunk_memo(&self) -> Option<Arc<joins::ChunkMemo>>;
    /// The kept plans as their own type, for the search that bound them.
    fn as_any(&self) -> &dyn std::any::Any;
}

/// `f` of the run's kept plans (`KeptPlans`), if a corpus run keeps them.
pub fn kept_plans<R>(f: impl FnOnce(Option<&dyn KeptView>) -> R) -> R {
    rerail_prof::code_rule_dsl::logic::profile::with_bound(|p: &KeptPlans| f(p.0.as_deref()))
}
