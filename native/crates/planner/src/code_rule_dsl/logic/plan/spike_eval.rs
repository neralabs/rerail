//! SPIKE (throw-away): a stratum's rules evaluated over the chunk's cached tree (`spike_strata::Tree`) and the
//! relations the chunk holds, in dense arrays and hash indexes: no view read, no numbering, no candidate search, no
//! engine join step. Any stratum whose every rule this can compile (`compile`) is run here (`RERAIL_SPIKE_EVAL=1`):
//!
//! - the tree's relations (`kind`, `node` with its document and span unread, `child`, `field`, `text`, `within`,
//!   `nearest` and the `rr_nearest_`/`rr_inside_`/`rr_has_` sets) are read off the tree's arrays by row: a node
//!   variable bound by one carries its row, so the next tree atom on it reads an array, not an index;
//! - every other relation is held by the chunk (`HRel`: flat rows, a dedup table, hash indexes by the columns a step
//!   binds), made here or read once from the engine where a stratum this declined made it;
//! - a recursive stratum is evaluated semi-naively, each rule once per recursive atom read as the last wave's rows.
//!
//! Its relations are held by the chunk for the strata after it and retained as the engine's, which the strata it
//! declines and the run's outputs read. `RERAIL_SPIKE_CHECK=1` as well: each stratum is run both ways and the rows
//! compared (`SPIKE CHECK`).

use std::hash::Hasher;
use std::sync::Arc;

use hashbrown::HashTable;
use rustc_hash::{FxHashMap as HashMap, FxHasher};

use super::spike_strata::{Chunk, NIL, NONE, Tree};
use crate::code_rule_dsl::facts::rows::RangeRead;
use crate::code_rule_dsl::logic::program_image::{AggRef, CmpOp, Lit, RuleRef, TermRef, Version};
use crate::code_rule_dsl::logic::strings::functions::Kind;

pub fn enabled() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_EVAL").is_some())
}

/// Whether the evaluator logs what it declines (`SPIKE EVAL`).
fn verbose() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_WHY").is_some())
}

fn hash_vals(vals: impl Iterator<Item = i64>) -> u64 {
    let mut h = FxHasher::default();
    for v in vals {
        h.write_i64(v);
    }
    h.finish()
}

/// A relation's values: its own, flat (`k` a row: made here, or read off a source the engine holds otherwise), or
/// the engine's relation of the same rows, read where they lie (`Rows::range_reader`, a column at a time, one load a
/// value), kept alive by its source: then the chunk holds only its indexes. One part of live rows reads by id
/// (`Lent`); rows over many parts (`Runs`) find their part's readers, and past dead rows (`live`) their row first.
enum Vals {
    Own(Vec<i64>),
    Lent { cols: Vec<RangeRead<'static>>, _keep: Arc<dyn crate::code_rule_dsl::facts::vtab::Source> },
    Runs {
        /// Each part's first row (by the relation's ids), and its `k` readers, a part after another.
        starts: Vec<u32>,
        cols: Vec<RangeRead<'static>>,
        /// Each live row's id, where the relation has dead rows.
        live: Option<Vec<u32>>,
        _keep: Arc<dyn crate::code_rule_dsl::facts::vtab::Source>,
    },
}

// SAFETY: a lent relation's readers only read its rows, in a block its source (`Send + Sync`) keeps alive and that
// nothing writes while it is held; an own relation's rows are a `Vec`.
unsafe impl Send for Vals {}
// SAFETY: as `Send`: shared reads of rows nothing writes.
unsafe impl Sync for Vals {}

impl Vals {
    #[inline(always)]
    fn get(&self, k: usize, id: u32, c: usize) -> i64 {
        match self {
            Vals::Own(v) => v[id as usize * k + c],
            Vals::Lent { cols, .. } => cols[c].get(id),
            Vals::Runs { starts, cols, live, .. } => {
                let at = live.as_ref().map_or(id, |l| l[id as usize]);
                let part = starts.partition_point(|&s| s <= at) - 1;
                cols[part * k + c].get(at)
            }
        }
    }

    /// The engine's relation `src` of `n` live rows read where they lie. `None` where it holds none.
    fn lent(src: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, k: usize, n: usize) -> Option<Vals> {
        let (rows, held, dead, _) = super::super::native::component::held_rows(&**src)?;
        if n == 0 || k == 0 {
            return None;
        }
        // SAFETY: a reader reads the relation's rows where they lie, in its block, which `_keep` (the source that
        // holds the relation) keeps alive and unchanged while these values are held.
        let keep = |r: RangeRead<'_>| unsafe { std::mem::transmute::<RangeRead<'_>, RangeRead<'static>>(r) };
        if held == n && dead.is_empty()
            && let Some(cols) = (0..k).map(|c| rows.range_reader(0, n as u32, c).map(keep)).collect::<Option<Vec<_>>>()
        {
            return Some(Vals::Lent { cols, _keep: Arc::clone(src) });
        }
        let mut starts = Vec::new();
        let mut cols = Vec::new();
        let mut whole = true;
        rows.each_run(0, held as u32, |_, i, j, id| {
            starts.push(id);
            for c in 0..k {
                match rows.range_reader(id, id + (j - i), c) {
                    Some(r) => cols.push(keep(r)),
                    None => whole = false,
                }
            }
        });
        if !whole {
            return None;
        }
        let live = (!dead.is_empty()).then(|| (0..held as u32).filter(|&id| dead.get(id as usize / 64).is_none_or(|w| w >> (id % 64) & 1 == 0)).collect::<Vec<u32>>());
        if live.as_ref().map_or(held, Vec::len) != n {
            return None;
        }
        Some(Vals::Runs { starts, cols, live, _keep: Arc::clone(src) })
    }

    /// The bytes it holds itself: an own relation's rows, a lent one's map past its dead rows.
    fn bytes(&self) -> usize {
        match self {
            Vals::Own(v) => v.capacity() * 8,
            Vals::Lent { .. } => 0,
            Vals::Runs { starts, live, .. } => starts.capacity() * 4 + live.as_ref().map_or(0, |l| l.capacity() * 4),
        }
    }
}

/// An index of a relation by some of its columns: each key's newest row, and each row's previous row of its key
/// (newest first, so a wave's rows lead).
pub struct Idx {
    cols: Vec<usize>,
    heads: HashTable<u32>,
    next: Vec<u32>,
    keys: usize,
}

impl Idx {
    fn new(cols: Vec<usize>) -> Idx {
        Idx { cols, heads: HashTable::new(), next: Vec::new(), keys: 0 }
    }

    /// Indexes rows `next.len()..n` of `v` (`k` values a row).
    fn extend(&mut self, v: &Vals, k: usize, n: usize) {
        let cols = &self.cols;
        for id in self.next.len() as u32..n as u32 {
            let h = hash_vals(cols.iter().map(|&c| v.get(k, id, c)));
            let eq = |&o: &u32| cols.iter().all(|&c| v.get(k, o, c) == v.get(k, id, c));
            match self.heads.find_mut(h, eq) {
                Some(head) => {
                    self.next.push(*head);
                    *head = id;
                }
                None => {
                    self.next.push(NONE);
                    self.keys += 1;
                    self.heads.insert_unique(h, id, |&o| hash_vals(cols.iter().map(|&c| v.get(k, o, c))));
                }
            }
        }
    }

    /// The newest row whose columns are `key`.
    #[inline]
    fn find(&self, v: &Vals, k: usize, key: &[i64]) -> u32 {
        let h = hash_vals(key.iter().copied());
        self.heads.find(h, |&o| self.cols.iter().zip(key).all(|(&c, &x)| v.get(k, o, c) == x)).copied().unwrap_or(NONE)
    }
}

/// An index of a relation the chunk holds for the strata after it, whole once made (nothing is added to it): its rows
/// in the order of their key's hash (then the key), so each key's rows lie together, each with its hash's high 32
/// bits (`fp`), and where each of `2^bits` buckets of hashes starts. About 9 bytes a row, read in order, with no
/// table: a probe finds its key's bucket, then its rows by a binary search of their hashes' bits, reading a row's
/// columns only to confirm its key.
pub struct Sorted {
    cols: Vec<usize>,
    order: Vec<u32>,
    fp: Vec<u32>,
    dir: Vec<u32>,
    bits: u32,
    keys: usize,
    /// The rows a probe by a key of its own rows finds, on average: `Σ c² / n` over its keys' row counts `c`.
    fan: f64,
}

impl Sorted {
    #[inline]
    fn bucket(h: u64, bits: u32) -> usize {
        let m = (h ^ h >> 29).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        m.checked_shr(64 - bits).unwrap_or(0) as usize
    }

    /// The index of `n` rows of `v` on `cols`: buckets about four rows each, counted, then filled, then each sorted.
    fn new(cols: Vec<usize>, v: &Vals, k: usize, n: usize) -> Sorted {
        let hash = |id: u32| hash_vals(cols.iter().map(|&c| v.get(k, id, c)));
        let bits = (n / 4).max(1).next_power_of_two().trailing_zeros();
        let mut dir = vec![0u32; (1 << bits) + 1];
        let mut fp = vec![0u32; n];
        for id in 0..n as u32 {
            let h = hash(id);
            fp[id as usize] = (h >> 32) as u32;
            dir[Self::bucket(h, bits) + 1] += 1;
        }
        for b in 1..dir.len() {
            dir[b] += dir[b - 1];
        }
        let mut order = vec![0u32; n];
        let mut fill = dir.clone();
        for id in 0..n as u32 {
            let b = Self::bucket(hash(id), bits);
            order[fill[b] as usize] = id;
            fill[b] += 1;
        }
        drop(fill);
        let vals = |id: u32| cols.iter().map(move |&c| v.get(k, id, c));
        let (mut keys, mut squares) = (0, 0f64);
        for b in 0..1 << bits {
            let rows = &mut order[dir[b] as usize..dir[b + 1] as usize];
            if rows.len() > 1 {
                rows.sort_unstable_by(|&x, &y| fp[x as usize].cmp(&fp[y as usize]).then_with(|| vals(x).cmp(vals(y))));
            }
            // Each key's run of rows.
            let mut run = 0f64;
            for (j, &id) in rows.iter().enumerate() {
                if j > 0 && (fp[rows[j - 1] as usize] != fp[id as usize] || vals(rows[j - 1]).ne(vals(id))) {
                    keys += 1;
                    squares += run * run;
                    run = 0.0;
                }
                run += 1.0;
            }
            if run > 0.0 {
                keys += 1;
                squares += run * run;
            }
        }
        let fan = squares / n.max(1) as f64;
        // Each row's bits in the index's order, read beside it.
        let fp: Vec<u32> = order.iter().map(|&id| fp[id as usize]).collect();
        Sorted { cols, order, fp, dir, bits, keys, fan }
    }

    /// The rows whose columns are `key`.
    #[inline]
    fn find(&self, v: &Vals, k: usize, key: &[i64]) -> &[u32] {
        let h = hash_vals(key.iter().copied());
        let f = (h >> 32) as u32;
        let b = Self::bucket(h, self.bits);
        let (lo, hi) = (self.dir[b] as usize, self.dir[b + 1] as usize);
        let fps = &self.fp[lo..hi];
        let mut at = lo + fps.partition_point(|&x| x < f);
        let to = lo + fps.partition_point(|&x| x <= f);
        if at == to {
            return &[];
        }
        let same = |id: u32| self.cols.iter().zip(key).all(|(&c, &x)| v.get(k, id, c) == x);
        // The rows of these bits are sorted by their columns: where the first and last are this key, all are.
        if same(self.order[at]) && same(self.order[to - 1]) {
            return &self.order[at..to];
        }
        // Past keys of the same bits (rare): sorted by their columns, so a scan reaches this one's run.
        while at < hi && self.fp[at] == f && !same(self.order[at]) {
            at += 1;
        }
        let mut end = at;
        while end < hi && self.fp[end] == f && same(self.order[end]) {
            end += 1;
        }
        &self.order[at..end]
    }

    fn bytes(&self) -> usize {
        (self.order.capacity() + self.fp.capacity() + self.dir.capacity()) * 4
    }
}

/// A relation the chunk holds: `n` rows of `k` values (`Vals`); for one made here, its dedup table; its indexes, by
/// the columns they key.
pub struct HRel {
    pub k: usize,
    pub n: usize,
    vals: Vals,
    set: HashTable<u32>,
    /// A relation made here: its indexes, brought up to date at each wave's end.
    idx: Vec<Idx>,
    /// A relation the chunk holds for the strata after it: its indexes, each made whole at its first probe.
    shared: std::sync::Mutex<Vec<Arc<Sorted>>>,
    /// Its keys' counts by columns, estimated off a sample for a plan's costs (`keys`): no index made to price one.
    est: std::sync::Mutex<Vec<(Vec<usize>, usize, f64)>>,
}

impl HRel {
    fn new(k: usize) -> HRel {
        HRel { k, n: 0, vals: Vals::Own(Vec::new()), set: HashTable::new(), idx: Vec::new(), shared: std::sync::Mutex::default(), est: std::sync::Mutex::default() }
    }

    /// `n` rows as given (distinct already), flat, with no dedup table.
    pub fn of_rows(k: usize, v: Vec<i64>, n: usize) -> HRel {
        HRel { k, n, vals: Vals::Own(v), set: HashTable::new(), idx: Vec::new(), shared: std::sync::Mutex::default(), est: std::sync::Mutex::default() }
    }

    /// The engine's relation `src` (`k` columns, `n` rows) read where it lies (`Vals::lent`); `None` where it cannot be.
    pub fn of_engine(src: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>, k: usize, n: usize) -> Option<HRel> {
        let vals = Vals::lent(src, k, n)?;
        Some(HRel { k, n, vals, set: HashTable::new(), idx: Vec::new(), shared: std::sync::Mutex::default(), est: std::sync::Mutex::default() })
    }

    /// Its rows, flat, while they are its own (a relation made here, before the engine holds it).
    pub fn own(&self) -> &[i64] {
        match &self.vals {
            Vals::Own(v) => v,
            Vals::Lent { .. } | Vals::Runs { .. } => panic!("a relation the engine lends has no rows of its own"),
        }
    }

    /// Its rows let go for the engine's relation `src` of the same rows in the same order, read where they lie: its
    /// indexes stay. Whether they were (a relation the engine holds, in its parts, past its dead rows).
    pub fn lend_to(&mut self, src: &Arc<dyn crate::code_rule_dsl::facts::vtab::Source>) -> bool {
        match Vals::lent(src, self.k, self.n) {
            Some(vals) => {
                self.vals = vals;
                true
            }
            None => false,
        }
    }

    /// The index on `cols` of a relation the chunk holds, made whole at its first use.
    fn shared_index(&self, cols: &[usize]) -> Arc<Sorted> {
        let mut shared = self.shared.lock().unwrap();
        if let Some(i) = shared.iter().find(|i| i.cols == cols) {
            return Arc::clone(i);
        }
        let i = Arc::new(Sorted::new(cols.to_vec(), &self.vals, self.k, self.n));
        shared.push(Arc::clone(&i));
        i
    }

    #[inline(always)]
    fn get(&self, id: u32, c: usize) -> i64 {
        self.vals.get(self.k, id, c)
    }

    /// Its keys' count by `cols`, and the rows a probe by a key drawn from its own rows finds on average (`Σ c² / n`
    /// over each key's row count `c`: under skew, far more than `n / keys`): its index's where one is made, else
    /// estimated off at most `SAMPLE` rows spread over it, once a relation and columns. The keys by Haas and Stokes'
    /// GEE (the sample's values seen twice or more, and those seen once scaled by `sqrt(n / s)`); the probe's rows by
    /// the unbiased estimate of a self-join's size from a sample, `Σ c(c - 1) · n(n - 1) / (s(s - 1)) + n`, over `n`.
    fn keys(&self, cols: &[usize]) -> (usize, f64) {
        const SAMPLE: usize = 4096;
        if let Some(i) = self.shared.lock().unwrap().iter().find(|i| i.cols == cols) {
            return (i.keys, i.fan);
        }
        let mut est = self.est.lock().unwrap();
        if let Some(&(_, k, f)) = est.iter().find(|(c, _, _)| c == cols) {
            return (k, f);
        }
        let n = self.n;
        let (s, step) = if n <= SAMPLE { (n, 1) } else { (SAMPLE, n / SAMPLE) };
        let mut counts: HashMap<u64, u32> = HashMap::default();
        for j in 0..s {
            let id = (j * step) as u32;
            *counts.entry(hash_vals(cols.iter().map(|&c| self.get(id, c)))).or_default() += 1;
        }
        let (k, f) = if s == n {
            (counts.len(), counts.values().map(|&c| f64::from(c) * f64::from(c)).sum::<f64>() / n.max(1) as f64)
        } else {
            let once = counts.values().filter(|&&c| c == 1).count() as f64;
            let more = counts.len() as f64 - once;
            let keys = ((n as f64 / s as f64).sqrt() * once + more).round().clamp(1.0, n as f64) as usize;
            let (nf, sf) = (n as f64, s as f64);
            let pairs: f64 = counts.values().map(|&c| f64::from(c) * (f64::from(c) - 1.0)).sum();
            (keys, (pairs * nf * (nf - 1.0) / (sf * (sf - 1.0)) + nf) / nf)
        };
        est.push((cols.to_vec(), k, f));
        (k, f)
    }

    /// Adds `row` if new.
    fn insert(&mut self, row: &[i64]) -> bool {
        let k = self.k;
        if k == 0 {
            let new = self.n == 0;
            self.n = 1;
            return new;
        }
        let Vals::Own(v) = &mut self.vals else { panic!("rows added to a relation the engine lends") };
        let h = hash_vals(row.iter().copied());
        if self.set.find(h, |&o| &v[o as usize * k..o as usize * k + k] == row).is_some() {
            return false;
        }
        let id = self.n as u32;
        v.extend_from_slice(row);
        self.n += 1;
        let v = &*v;
        self.set.insert_unique(h, id, |&o| hash_vals(v[o as usize * k..o as usize * k + k].iter().copied()));
        true
    }

    /// The index on `cols`, made or brought up to date.
    fn index(&mut self, cols: &[usize]) -> usize {
        let at = match self.idx.iter().position(|i| i.cols == cols) {
            Some(at) => at,
            None => {
                self.idx.push(Idx::new(cols.to_vec()));
                self.idx.len() - 1
            }
        };
        let (k, n) = (self.k, self.n);
        self.idx[at].extend(&self.vals, k, n);
        at
    }

    /// The rows' dedup table and wave indexes dropped, once nothing is added (a stratum's end).
    fn seal(&mut self) {
        self.set = HashTable::new();
        self.idx = Vec::new();
        if let Vals::Own(v) = &mut self.vals {
            v.shrink_to_fit();
        }
    }

    /// The bytes it holds: its own rows' (none where the engine lends them), and its indexes'.
    pub fn bytes_split(&self) -> (usize, usize) {
        let own = self.vals.bytes();
        (own, self.bytes() - own)
    }

    /// The bytes it holds.
    pub fn bytes(&self) -> usize {
        let idx = |i: &Idx| i.next.capacity() * 4 + i.heads.capacity() * 5;
        let own = self.vals.bytes();
        own + self.set.capacity() * 5 + self.idx.iter().map(idx).sum::<usize>() + self.shared.lock().unwrap().iter().map(|i| i.bytes()).sum::<usize>()
    }
}

/// A term of a compiled literal.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Arg {
    Var(usize),
    Const(i64),
    Wild,
}

/// A relation of the tree, read off its arrays.
#[derive(Clone, Debug)]
enum TreeRel {
    /// `kind(n, k)`, and `node(_, n, k, _, _)`.
    Kind,
    /// `node(d, n, k, start, end)`.
    Node,
    /// `child(p, c, i)`.
    Child,
    /// `field(p, f, c)`.
    Field,
    /// `text(n, s)`.
    Text,
    /// `within(n, a)`: `n` strictly below `a`.
    Within,
    /// `nearest(n, a, set)`: `a` is `n`'s nearest strict ancestor of a kind in the set.
    Nearest(Vec<i64>),
    /// `rr_inside_<set>(n, a)`: `a` is a strict ancestor of `n` of a kind in the set.
    Inside(Vec<i64>),
    /// `rr_has_<set>(n, a)`: `n` of a kind in the set, `a` a strict ancestor of it.
    Has(Vec<i64>),
}

/// A relation made by one rule that projects one tree atom (`kind(n, k) :- node(_, n, k, _, _)`): read off the tree
/// as that atom, its columns at `cols` of the atom's.
#[derive(Clone, Debug)]
pub struct Alias {
    tr: TreeRel,
    cols: Vec<usize>,
    arity: usize,
}

#[derive(Clone, Debug)]
enum Src {
    Tree(TreeRel),
    /// A relation by its place in the program's (`Prog::rels`).
    Rel(usize),
}

#[derive(Clone, Debug)]
struct AtomP {
    src: Src,
    neg: bool,
    args: Vec<Arg>,
}

#[derive(Clone, Copy, Debug)]
enum AggOp {
    Count,
    Min,
    Max,
    Sum,
}

/// A computed term: a term, or a function of terms (`Fun`).
#[derive(Clone, Debug)]
enum Expr {
    A(Arg),
    Call(Fun, Vec<Expr>),
}

/// A text test's matcher of a constant pattern (or set of them), made at its first use and kept: no row pays its
/// lookup in the run's shared matchers.
#[derive(Clone, Default)]
struct Held(std::sync::OnceLock<Option<Arc<crate::code_rule_dsl::logic::strings::Matcher>>>);

impl std::fmt::Debug for Held {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("matcher")
    }
}

/// A function a rule calls.
#[derive(Clone, Debug)]
enum Fun {
    /// `@cat(a, b)` (`@dot`): `a.b` over the run's symbols.
    Cat,
    /// `@doc(n)`: node `n`'s document.
    Doc,
    /// A node read (`syntax::NODE_READS`): its text at any length, value or string, by the run's long-text nodes.
    NodeRead(&'static str),
    /// A text test (`strings::TESTS`) of a string against a pattern or a set of them: 1 or 0; a constant pattern's
    /// matcher held.
    Test(&'static str, Held),
    /// `@in(x, {…})`, `@equals(x, {…})` once lowered: membership of the set's constants.
    In(Vec<i64>),
    /// A set's patterns (a test's second argument).
    Set(Vec<i64>),
    /// A function of `strings::functions` (its name, arguments' kinds and result's), its first two arguments swapped
    /// where the catalogue says.
    Eval(&'static str, &'static [Kind], Kind, bool),
}

#[derive(Clone, Debug)]
enum LitP {
    Atom(AtomP),
    Cmp { l: Expr, op: CmpOp, r: Expr },
    /// `var = op over : { atom }`: `over` the column of the atom aggregated.
    Agg { var: usize, op: AggOp, over: Option<usize>, atom: AtomP },
}

#[derive(Debug)]
struct RuleP {
    head: usize,
    args: Vec<Expr>,
    lits: Vec<LitP>,
    nslots: usize,
    /// The literals that read a relation of the stratum (positive atoms).
    rec: Vec<usize>,
    text: String,
}

/// A stratum compiled: its relations (the stratum's first), its rules.
pub struct Prog {
    pub rels: Vec<String>,
    pub nscc: usize,
    pub widths: Vec<usize>,
    rules: Vec<RuleP>,
}

/// What compiling reads of the run: constants' symbols, kind sets, relation widths, which names are the tree's.
pub trait Names {
    fn intern(&self, text: &str) -> i64;
    /// The kind atoms of kind set `set` in the chunk's dialects.
    fn kindset(&self, set: &str) -> Option<Vec<i64>>;
    /// Whether `rel` is a relation of the chunk's syntax batch (a view or made relation over its tree).
    fn tree_relation(&self, rel: &str) -> bool;
    /// The tree atom `rel` projects, if an earlier stratum of the chunk found it one (`Prog::alias`).
    fn alias(&self, rel: &str) -> Option<Alias>;
}

fn term(v: &Version, t: TermRef<'_>, names: &dyn Names) -> Result<Arg, String> {
    Ok(match t {
        TermRef::Var(s) => Arg::Var(s.0 as usize),
        TermRef::Wild => Arg::Wild,
        TermRef::Int(i) => Arg::Const(i),
        TermRef::Branded(b) => Arg::Const(b.value()),
        TermRef::Str(s) => Arg::Const(names.intern(v.text(s))),
        TermRef::Top => return Err("a top term".into()),
        TermRef::Call(f, _) => return Err(format!("a call @{}", v.text(f))),
    })
}

/// A rule's term, computed or not.
fn expr(v: &Version, t: TermRef<'_>, names: &dyn Names) -> Result<Expr, String> {
    use crate::code_rule_dsl::logic::strings::{self, functions};
    let TermRef::Call(f, args) = t else { return Ok(Expr::A(term(v, t, names)?)) };
    let f = v.text(f);
    let args: Vec<TermRef<'_>> = args.collect();
    let n = args.len();
    let consts = |args: &[TermRef<'_>]| -> Result<Vec<i64>, String> {
        args.iter().map(|&a| match term(v, a, names)? {
            Arg::Const(c) => Ok(c),
            _ => Err(format!("a set of @{f} not of constants")),
        }).collect()
    };
    let sub = |args: &[TermRef<'_>]| -> Result<Vec<Expr>, String> { args.iter().map(|&a| expr(v, a, names)).collect() };
    let fun = match f {
        "cat" | "dot" if n == 2 => Fun::Cat,
        "doc" if n == 1 => Fun::Doc,
        _ if crate::code_rule_dsl::facts::vtab::NODE_READS.contains(&f) && n == 1 => Fun::NodeRead(crate::code_rule_dsl::facts::vtab::NODE_READS.iter().find(|r| **r == f).copied().expect("a node read")),
        strings::SET => Fun::Set(consts(&args)?),
        "in" | "equals" if n == 2 && matches!(args[1], TermRef::Call(s, _) if v.text(s) == strings::SET) => {
            let TermRef::Call(_, members) = args[1] else { unreachable!() };
            let members: Vec<TermRef<'_>> = members.collect();
            return Ok(Expr::Call(Fun::In(consts(&members)?), vec![expr(v, args[0], names)?]));
        }
        _ if strings::is_test(f) && n == 2 => Fun::Test(strings::TESTS.iter().find(|t| **t == f).copied().expect("a test"), Held::default()),
        "concat" if n == 2 => {
            let (name, kinds, out) = functions::signature("str_concat").ok_or("no str_concat")?;
            Fun::Eval(name, kinds, out, false)
        }
        _ => match functions::native_call(f).filter(|(_, k, _)| k.len() == n) {
            Some((name, kinds, out)) => Fun::Eval(name, kinds, out, false),
            None => {
                let (name, swap) = functions::catalogue(f, n).ok_or_else(|| format!("a call @{f}"))?;
                let (name, kinds, out) = functions::signature(name).filter(|(_, k, _)| k.len() == n).ok_or_else(|| format!("a call @{f}"))?;
                Fun::Eval(name, kinds, out, swap)
            }
        },
    };
    Ok(Expr::Call(fun, sub(&args)?))
}

/// The tree relation `rel` of these terms, if it is one.
fn tree_atom(rel: &str, args: &[Arg], names: &dyn Names) -> Result<Option<(TreeRel, Vec<Arg>)>, String> {
    let set = |prefix: &str| -> Result<Option<Vec<i64>>, String> {
        let Some(name) = rel.strip_prefix(prefix) else { return Ok(None) };
        names.kindset(name).map(Some).ok_or_else(|| format!("no kind set {name}"))
    };
    if !names.tree_relation(rel) {
        return Ok(None);
    }
    Ok(Some(match rel {
        "kind" if args.len() == 2 => (TreeRel::Kind, args.to_vec()),
        "node" if args.len() == 5 => {
            if [args[0], args[3], args[4]].iter().all(|a| *a == Arg::Wild) {
                (TreeRel::Kind, vec![args[1], args[2]])
            } else {
                (TreeRel::Node, args.to_vec())
            }
        }
        "child" if args.len() == 3 => (TreeRel::Child, args.to_vec()),
        "field" if args.len() == 3 => (TreeRel::Field, args.to_vec()),
        "text" if args.len() == 2 => (TreeRel::Text, args.to_vec()),
        "within" if args.len() == 2 => (TreeRel::Within, args.to_vec()),
        _ => {
            if let Some(s) = set("rr_nearest_")? {
                (TreeRel::Nearest(s), args.to_vec())
            } else if let Some(s) = set("rr_inside_")? {
                (TreeRel::Inside(s), args.to_vec())
            } else if let Some(s) = set("rr_has_")? {
                (TreeRel::Has(s), args.to_vec())
            } else {
                return Err(format!("a tree relation not read off the tree: {rel}"));
            }
        }
    }))
}

/// Compiles stratum `scc`'s `rules`, each with its version; `Err`: why not.
pub fn compile(scc: &[&str], rules: &[(&Version, RuleRef<'_>)], widths: &dyn Fn(&str) -> usize, names: &dyn Names) -> Result<Prog, String> {
    let mut rels: Vec<String> = scc.iter().map(|r| (*r).to_owned()).collect();
    let at = |rel: &str, rels: &mut Vec<String>| -> usize {
        match rels.iter().position(|r| r == rel) {
            Some(i) => i,
            None => {
                rels.push(rel.to_owned());
                rels.len() - 1
            }
        }
    };
    let mut out = Vec::new();
    for &(v, rule) in rules {
        let head_atom = rule.head();
        let head_rel = v.rel_name(head_atom.rel());
        let head = scc.iter().position(|r| *r == head_rel).ok_or_else(|| format!("a head outside the stratum: {head_rel}"))?;
        let args: Vec<Expr> = head_atom.args().map(|t| expr(v, t, names)).collect::<Result<_, _>>()?;
        if args.iter().any(|a| matches!(a, Expr::A(Arg::Wild))) {
            return Err("a wildcard in a head".into());
        }
        let mut lits = Vec::new();
        let mut rec = Vec::new();
        for lit in rule.body() {
            let atom_of = |a: crate::code_rule_dsl::logic::program_image::AtomRef<'_>, rels: &mut Vec<String>| -> Result<AtomP, String> {
                if a.gated() {
                    return Err("a gated atom".into());
                }
                let rel = v.rel_name(a.rel());
                let args: Vec<Arg> = a.args().map(|t| term(v, t, names)).collect::<Result<_, _>>()?;
                Ok(match tree_atom(rel, &args, names)? {
                    Some((t, args)) => AtomP { src: Src::Tree(t), neg: a.neg(), args },
                    None => match names.alias(rel).filter(|_| !scc.contains(&rel)) {
                        Some(al) => {
                            let mut tree_args = vec![Arg::Wild; al.arity];
                            for (c, &x) in al.cols.iter().zip(&args) {
                                tree_args[*c] = x;
                            }
                            AtomP { src: Src::Tree(al.tr), neg: a.neg(), args: tree_args }
                        }
                        None => AtomP { src: Src::Rel(at(rel, rels)), neg: a.neg(), args },
                    },
                })
            };
            match lit.view() {
                Lit::Atom(a) => {
                    let p = atom_of(a, &mut rels)?;
                    if let Src::Rel(r) = p.src
                        && r < scc.len()
                    {
                        if p.neg {
                            return Err("a negation within the stratum".into());
                        }
                        rec.push(lits.len());
                    }
                    lits.push(LitP::Atom(p));
                }
                Lit::Cmp { l, op, r } => lits.push(LitP::Cmp { l: expr(v, l, names)?, op, r: expr(v, r, names)? }),
                Lit::Agg { var, op, over, atom } => {
                    let op = match op {
                        AggRef::Count => AggOp::Count,
                        AggRef::Min => AggOp::Min,
                        AggRef::Max => AggOp::Max,
                        AggRef::Sum => AggOp::Sum,
                        _ => return Err("an aggregate other than count, min, max or sum".into()),
                    };
                    let p = atom_of(atom, &mut rels)?;
                    if p.neg || matches!(p.src, Src::Tree(_)) || matches!(p.src, Src::Rel(r) if r < scc.len()) {
                        return Err("an aggregate over the tree, a negation or the stratum".into());
                    }
                    let over = match over {
                        None => None,
                        Some(s) => Some(p.args.iter().position(|a| *a == Arg::Var(s.0 as usize)).ok_or("an aggregate's value not in its atom")?),
                    };
                    lits.push(LitP::Agg { var: var.0 as usize, op, over, atom: p });
                }
                Lit::Nearest { n, a, kindset } => {
                    let set = names.kindset(v.text(kindset)).ok_or_else(|| format!("no kind set {}", v.text(kindset)))?;
                    lits.push(LitP::Atom(AtomP { src: Src::Tree(TreeRel::Nearest(set)), neg: false, args: vec![Arg::Var(n.0 as usize), Arg::Var(a.0 as usize)] }));
                }
                Lit::Ast { .. } => return Err("an ast literal".into()),
            }
        }
        if rule.var_count() > 64 {
            return Err("a rule of more than 64 variables".into());
        }
        out.push(RuleP { head, args, lits, nslots: rule.var_count(), rec, text: format!("{head_rel}/{}", out.len()) });
    }
    let widths: Vec<usize> = rels.iter().map(|r| widths(r)).collect();
    if let Some((r, _)) = rels.iter().zip(&widths).enumerate().find(|&(i, (_, &w))| w > 16 || (i < scc.len() && w == 0)) {
        return Err(format!("a relation of no columns or more than 16: {}", rels[r]));
    }
    Ok(Prog { rels, nscc: scc.len(), widths, rules: out })
}

impl Prog {
    /// The tree atom the stratum's one relation projects, if it is made so (`Alias`).
    pub fn alias(&self) -> Option<Alias> {
        let [rule] = &self.rules[..] else { return None };
        let [LitP::Atom(AtomP { src: Src::Tree(tr), neg: false, args })] = &rule.lits[..] else { return None };
        if self.nscc != 1 {
            return None;
        }
        let mut seen = 0u64;
        for a in args {
            match a {
                Arg::Wild => {}
                Arg::Var(s) if seen >> s & 1 == 0 => seen |= 1 << s,
                _ => return None,
            }
        }
        let mut heads = 0u64;
        let cols = rule
            .args
            .iter()
            .map(|e| match e {
                Expr::A(Arg::Var(s)) if heads >> s & 1 == 0 => {
                    heads |= 1 << s;
                    args.iter().position(|a| *a == Arg::Var(*s))
                }
                _ => None,
            })
            .collect::<Option<Vec<usize>>>()?;
        Some(Alias { tr: tr.clone(), cols, arity: args.len() })
    }
}

/// Which rows of a relation a step reads.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Part {
    /// Every row up to the wave's end.
    All,
    /// The last wave's rows.
    Delta,
}

/// What a step does with a column of a tuple it reads.
#[derive(Clone, Copy, Debug)]
enum Act {
    Skip,
    Bind(usize),
    Test(usize),
    Is(i64),
}

/// A step of a rule's plan: its literal, the slots bound before it, its columns' acts, and for a held relation, the
/// index it probes (`None`: a scan) and the part it reads.
struct Step {
    lit: usize,
    bound: u64,
    acts: Vec<Act>,
    /// A held relation's columns a row it finds is read at, with their acts: none skipped, none its index matched.
    reads: Vec<(usize, Act)>,
    key: Vec<usize>,
    ix: Option<IxRef>,
    part: Part,
}

/// A step's index: an own relation's, by place, or a held one's.
enum IxRef {
    Own(usize),
    Shared(Arc<Sorted>),
}

fn is_bound(bound: u64, a: Arg) -> bool {
    match a {
        Arg::Var(s) => bound >> s & 1 == 1,
        Arg::Const(_) => true,
        Arg::Wild => false,
    }
}

fn acts_of(args: &[Arg], bound: u64) -> Vec<Act> {
    let mut seen = bound;
    args.iter()
        .map(|&a| match a {
            Arg::Wild => Act::Skip,
            Arg::Const(c) => Act::Is(c),
            Arg::Var(s) if seen >> s & 1 == 1 => Act::Test(s),
            Arg::Var(s) => {
                seen |= 1 << s;
                Act::Bind(s)
            }
        })
        .collect()
}

fn binds(args: &[Arg]) -> u64 {
    args.iter().fold(0, |m, a| if let Arg::Var(s) = a { m | 1 << s } else { m })
}

/// The variables of a computed term.
fn vars(e: &Expr) -> u64 {
    match e {
        Expr::A(a) => binds(&[*a]),
        Expr::Call(_, args) => args.iter().fold(0, |m, a| m | vars(a)),
    }
}

/// Whether a computed term's value is known with `bound` slots bound.
fn known(bound: u64, e: &Expr) -> bool {
    match e {
        Expr::A(a) => is_bound(bound, *a),
        Expr::Call(_, args) => args.iter().all(|a| known(bound, a)),
    }
}

/// A comparison's side that is an unbound variable, the other side known: `(slot, the other side)`.
fn binding<'e>(bound: u64, l: &'e Expr, r: &'e Expr) -> Option<(usize, &'e Expr)> {
    match (l, r) {
        (Expr::A(Arg::Var(s)), other) if bound >> s & 1 == 0 && known(bound, other) => Some((*s, other)),
        (other, Expr::A(Arg::Var(s))) if bound >> s & 1 == 0 && known(bound, other) => Some((*s, other)),
        _ => None,
    }
}

/// `whole = @cat(a, b)` with `whole` known and `a`, `b` variables, not both bound: `(whole, a, b)`, which a split of
/// `whole`'s text binds (`Eval::go`), a bound one's text tested.
fn split<'e>(bound: u64, l: &'e Expr, r: &'e Expr) -> Option<(&'e Expr, usize, usize)> {
    let parts = |e: &Expr| match e {
        Expr::Call(Fun::Cat, args) => match args[..] {
            [Expr::A(Arg::Var(a)), Expr::A(Arg::Var(b))] if a != b && (bound >> a & 1 == 0 || bound >> b & 1 == 0) => Some((a, b)),
            _ => None,
        },
        _ => None,
    };
    match (parts(l), parts(r)) {
        (None, Some((a, b))) if known(bound, l) => Some((l, a, b)),
        (Some((a, b)), None) if known(bound, r) => Some((r, a, b)),
        _ => None,
    }
}

/// What a call reads of the run: its symbols' texts, new symbols, `@cat`, node reads.
pub trait Calls {
    fn name(&self, sym: i64) -> Option<String>;
    fn intern(&self, text: &str) -> i64;
    /// The symbol of `text`: the one held, else a new one (`intern`).
    fn atom(&self, text: &str) -> i64;
    /// Whether symbol `sym`'s text matches `m`, read where it lies; `None` for a symbol with no text.
    fn is_match(&self, sym: i64, m: &crate::code_rule_dsl::logic::strings::Matcher) -> Option<bool>;
    fn cat(&self, a: i64, b: i64) -> i64;
    fn node_read(&self, name: &str, n: i64) -> Option<String>;
}

/// The evaluation of a stratum over a chunk: its relations (the stratum's owned, the others held by the chunk).
struct Eval<'a> {
    prog: &'a Prog,
    calls: &'a dyn Calls,
    c: &'a Chunk<'a>,
    own: Vec<HRel>,
    read: Vec<Option<Arc<HRel>>>,
    /// Each own relation's last wave: rows `lo..hi`.
    lo: Vec<usize>,
    hi: Vec<usize>,
    /// The values of calls of text functions, `@cat` and node reads by the call (its place) and its arguments' values,
    /// for the stratum: a row whose arguments an earlier row had reads no text and interns nothing. At most `MEMO`.
    memo: std::cell::RefCell<HashMap<(usize, [i64; 3]), Option<i64>>>,
}

/// The most call values a stratum keeps (`Eval::memo`): 32 bytes each.
const MEMO: usize = 1 << 18;

/// The slots' values and, for a node bound off the tree, its row.
struct Env {
    v: Vec<i64>,
    r: Vec<u32>,
}

impl<'a> Eval<'a> {
    fn rel(&self, r: usize) -> &HRel {
        if r < self.prog.nscc { &self.own[r] } else { self.read[r].as_deref().expect("read before planning") }
    }

    fn size(&self, r: usize, part: Part) -> usize {
        if r < self.prog.nscc {
            match part {
                Part::All => self.hi[r],
                Part::Delta => self.hi[r] - self.lo[r],
            }
        } else {
            self.rel(r).n
        }
    }

    /// What literal `lit` costs with `bound` slots bound, per row that reaches it: the rows it gives each (its
    /// fanout) and the work it does for each (in hash probes); `None` where it cannot run yet.
    fn cost(&self, lit: &LitP, bound: u64, part: Part, locals: u64) -> Option<(f64, f64)> {
        fn call_cost(e: &Expr) -> f64 {
            match e {
                Expr::A(_) => 0.0,
                Expr::Call(f, args) => {
                    let own = match f {
                        Fun::NodeRead(_) => 30.0,
                        Fun::Eval(..) | Fun::Test(..) => 10.0,
                        Fun::Cat => 5.0,
                        Fun::Doc | Fun::In(_) | Fun::Set(_) => 0.1,
                    };
                    own + args.iter().map(call_cost).sum::<f64>()
                }
            }
        }
        let t = self.c.t;
        let n = t.len().max(1) as f64;
        match lit {
            LitP::Cmp { l, op, r } => {
                if known(bound, l) && known(bound, r) {
                    Some((if *op == CmpOp::Eq { 0.05 } else { 0.5 }, 0.05 + call_cost(l) + call_cost(r)))
                } else if *op == CmpOp::Eq
                    && let Some((_, from)) = binding(bound, l, r)
                {
                    Some((1.0, 0.05 + call_cost(from)))
                } else if *op == CmpOp::Eq
                    && let Some((_, a, b)) = split(bound, l, r)
                {
                    // A name's splits at its dots: a few, or with one part bound, at most one.
                    Some((if (bound >> a | bound >> b) & 1 == 1 { 0.5 } else { 2.0 }, 10.0))
                } else {
                    None
                }
            }
            LitP::Agg { atom, .. } => {
                let shared = binds(&atom.args) & !locals;
                if shared & !bound != 0 {
                    return None;
                }
                let rows = self.cost(&LitP::Atom(atom.clone()), bound, Part::All, 0).map_or(1.0, |(f, _)| f);
                Some((0.9, 1.0 + rows))
            }
            LitP::Atom(a) => {
                let b: Vec<bool> = a.args.iter().map(|&x| is_bound(bound, x)).collect();
                if a.neg {
                    if !a.args.iter().all(|&x| x == Arg::Wild || is_bound(bound, x)) {
                        return None;
                    }
                    return Some((0.5, if matches!(a.src, Src::Tree(_)) { 0.3 } else { 1.0 }));
                }
                // A test of a bound value narrows by this much.
                let test = |k: usize, sel: f64| if b[k] { sel } else { 1.0 };
                match &a.src {
                    Src::Tree(tr) => {
                        let kinds = by_kind(t);
                        let of_kind = |k: usize| match a.args[k] {
                            Arg::Const(c) => kinds.of(c).len() as f64 / n,
                            _ if b[k] => 1.0 / kinds.at.len().max(1) as f64,
                            _ => 1.0,
                        };
                        let fan = match tr {
                            TreeRel::Kind if b[0] => of_kind(1),
                            TreeRel::Kind => n * of_kind(1),
                            TreeRel::Node if b[1] => of_kind(2) * test(0, 0.1) * test(3, 0.01) * test(4, 0.01),
                            TreeRel::Node => n * of_kind(2) * test(0, 0.1) * test(3, 0.01) * test(4, 0.01),
                            TreeRel::Child if b[1] => 0.95 * test(0, 0.05) * test(2, 0.3),
                            TreeRel::Child if b[0] => 2.5 * test(2, 0.3),
                            TreeRel::Child => n * test(2, 0.3),
                            TreeRel::Field if b[2] => 0.5 * test(1, 0.1) * test(0, 0.05),
                            TreeRel::Field if b[0] => 1.5 * test(1, 0.3),
                            TreeRel::Field => n * 0.5 * test(1, 0.1),
                            TreeRel::Text if b[0] => 0.9 * test(1, 0.01),
                            TreeRel::Text => return None,
                            TreeRel::Within if b[0] && b[1] => 0.1,
                            TreeRel::Within if b[0] => 10.0,
                            TreeRel::Within if b[1] => 30.0,
                            TreeRel::Within => return None,
                            TreeRel::Nearest(_) if b[0] => 0.9 * test(1, 0.05),
                            TreeRel::Nearest(_) => {
                                if b[1] {
                                    20.0
                                } else {
                                    n
                                }
                            }
                            TreeRel::Inside(_) | TreeRel::Has(_) if b[0] => 3.0 * test(1, 0.1),
                            TreeRel::Inside(_) | TreeRel::Has(_) if b[1] => 30.0,
                            TreeRel::Inside(_) | TreeRel::Has(_) => 3.0 * n,
                        };
                        Some((fan, 0.2 + 0.1 * fan))
                    }
                    Src::Rel(r) => {
                        let size = self.size(*r, part) as f64;
                        let key: Vec<usize> = (0..b.len()).filter(|&c| b[c]).collect();
                        if key.is_empty() {
                            return Some((size, 0.5 + size));
                        }
                        // The rows a key finds, on average: the relation's rows over its keys' count.
                        let (keys, skew) = if *r < self.prog.nscc {
                            (self.own[*r].idx.iter().find(|i| i.cols == key).map_or(0, |i| i.keys), 0.0)
                        } else {
                            self.rel(*r).keys(&key)
                        };
                        let all = self.size(*r, Part::All) as f64;
                        // A probe finds its key's rows: as many as a key has on average, or under skew, as many as a
                        // key drawn from the rows has.
                        let mut fan = if keys == 0 { (size / 2.0).max(0.5) } else { (all / keys as f64).max(skew) };
                        if key.len() == b.len() {
                            fan = fan.min(0.5);
                        }
                        if part == Part::Delta && all > 0.0 {
                            fan *= size / all;
                        }
                        Some((fan, 1.0 + 0.5 * fan))
                    }
                }
            }
        }
    }

    /// The order of rule `rule`'s literals by `cost`, `delta` first: greedy, or where the greedy order's work is more
    /// than a search's (and the rule has at most 14 literals), the least over every order (by subsets).
    fn order(&self, rule: &RuleP, delta: Option<usize>, locals: u64) -> Result<Vec<usize>, String> {
        let lits = &rule.lits;
        let n = lits.len();
        let part = |i: usize| if Some(i) == delta { Part::Delta } else { Part::All };
        let adds = |i: usize| -> u64 {
            match &lits[i] {
                LitP::Atom(a) if !a.neg => binds(&a.args),
                LitP::Atom(_) => 0,
                LitP::Cmp { l, r, .. } => vars(l) | vars(r),
                LitP::Agg { var, .. } => 1 << var,
            }
        };
        // Greedily first: the literal that costs least next, with its price as the search prices it.
        let mut left: Vec<usize> = (0..n).collect();
        let mut bound = 0u64;
        let mut greedy = Vec::new();
        let (mut work, mut card) = (0.0f64, 1.0f64);
        while !left.is_empty() {
            let pick = match delta.filter(|d| left.contains(d)) {
                Some(d) => self.cost(&lits[d], bound, part(d), locals).map(|p| (p, d)),
                None => {
                    let mut best: Option<((f64, f64), usize)> = None;
                    for &i in &left {
                        if let Some((fan, unit)) = self.cost(&lits[i], bound, part(i), locals)
                            && best.is_none_or(|((f, u), _)| fan + unit < f + u)
                        {
                            best = Some(((fan, unit), i));
                        }
                    }
                    best
                }
            };
            let Some(((fan, unit), pick)) = pick else { return Err(format!("no order binds {}", rule.text)) };
            work += card * unit + card * fan;
            card = (card * fan).max(1e-6);
            left.retain(|&i| i != pick);
            bound |= adds(pick);
            greedy.push(pick);
        }
        // A search prices each literal after each subset: taken only where the greedy order's work is more than that
        // (planning no more than running), and for at most 14 literals.
        if n > 14 || work <= (n << n) as f64 {
            return Ok(greedy);
        }
        let full = (1usize << n) - 1;
        // A literal's cost turns only on which of its own variables are bound: priced once for each such set.
        let mine: Vec<u64> = lits
            .iter()
            .map(|l| match l {
                LitP::Atom(a) => binds(&a.args),
                LitP::Cmp { l, r, .. } => vars(l) | vars(r),
                LitP::Agg { var, atom, .. } => binds(&atom.args) | 1 << var,
            })
            .collect();
        let mut priced: Vec<Vec<(u64, Option<(f64, f64)>)>> = vec![Vec::new(); n];
        // Per subset: the least cost of running it first, the rows after it, and the literal run last.
        let mut best: Vec<Option<(f64, f64, usize)>> = vec![None; full + 1];
        let mut bound_of = vec![0u64; full + 1];
        best[0] = Some((0.0, 1.0, usize::MAX));
        for mask in 0..full {
            let Some((cost, card, _)) = best[mask] else { continue };
            let bound = bound_of[mask];
            for (i, lit) in lits.iter().enumerate() {
                if mask >> i & 1 == 1 || (mask == 0 && delta.is_some_and(|d| d != i)) {
                    continue;
                }
                let on = bound & mine[i];
                let price = match priced[i].iter().find(|&&(b, _)| b == on) {
                    Some(&(_, p)) => p,
                    None => {
                        let p = self.cost(lit, on, part(i), locals);
                        priced[i].push((on, p));
                        p
                    }
                };
                let Some((fan, unit)) = price else { continue };
                let next = mask | 1 << i;
                let out = card * fan;
                let c = cost + card * unit + out;
                if best[next].is_none_or(|(b, _, _)| c < b) {
                    best[next] = Some((c, out.max(1e-6), i));
                    bound_of[next] = bound | adds(i);
                }
            }
        }
        if best[full].is_none() {
            return Err(format!("no order binds {}", rule.text));
        }
        let mut out = Vec::new();
        let mut mask = full;
        while mask != 0 {
            let (_, _, i) = best[mask].expect("reached");
            out.push(i);
            mask &= !(1 << i);
        }
        out.reverse();
        Ok(out)
    }

    /// Rule `rule`'s plan, its literal `delta` (if any) reading the last wave's rows and led first.
    fn plan(&mut self, rule: &RuleP, delta: Option<usize>) -> Result<Vec<Step>, String> {
        // A variable that occurs only within an aggregate is its own.
        let mut outside = rule.args.iter().fold(0, |m, a| m | vars(a));
        let mut inside = 0u64;
        for l in &rule.lits {
            match l {
                LitP::Atom(a) => outside |= binds(&a.args),
                LitP::Cmp { l, r, .. } => outside |= vars(l) | vars(r),
                LitP::Agg { var, atom, .. } => {
                    outside |= 1 << var;
                    inside |= binds(&atom.args);
                }
            }
        }
        let locals = inside & !outside;
        let order = {
            let _span = super::super::profile::span(|| "spike plan: order".to_string());
            self.order(rule, delta, locals)?
        };
        let mut bound = 0u64;
        let mut steps = Vec::new();
        for pick in order {
            let part = if Some(pick) == delta { Part::Delta } else { Part::All };
            let lit = &rule.lits[pick];
            let mut step = Step { lit: pick, bound, acts: Vec::new(), reads: Vec::new(), key: Vec::new(), ix: None, part };
            match lit {
                LitP::Atom(a) => {
                    step.acts = acts_of(&a.args, bound);
                    if let Src::Rel(r) = a.src {
                        step.key = a.args.iter().enumerate().filter(|&(_, &x)| is_bound(bound, x)).map(|(c, _)| c).collect();
                        if !step.key.is_empty() {
                            step.ix = Some(self.index(r, &step.key));
                        }
                        let matched = |c: usize| step.ix.is_some() && step.key.contains(&c);
                        step.reads = step.acts.iter().enumerate().filter(|&(c, act)| !matches!(act, Act::Skip) && !matched(c)).map(|(c, &act)| (c, act)).collect();
                    }
                    if !a.neg {
                        bound |= binds(&a.args);
                    }
                }
                LitP::Cmp { l, r, .. } => {
                    bound |= vars(l) | vars(r);
                }
                LitP::Agg { var, atom, .. } => {
                    let Src::Rel(r) = atom.src else { unreachable!("compiled over a relation") };
                    // Its key: the columns of constants and variables bound outside it.
                    step.key = atom.args.iter().enumerate().filter(|&(_, &x)| is_bound(bound, x)).map(|(c, _)| c).collect();
                    let local_vars: Vec<usize> = atom.args.iter().filter_map(|a| if let Arg::Var(s) = a { (bound >> s & 1 == 0).then_some(*s) } else { None }).collect();
                    let mut sorted = local_vars.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    if sorted.len() != local_vars.len() {
                        return Err("a variable repeated within an aggregate".into());
                    }
                    if !step.key.is_empty() {
                        step.ix = Some(self.index(r, &step.key));
                    }
                    bound |= 1 << var;
                }
            }
            steps.push(step);
        }
        let head_vars = rule.args.iter().fold(0, |m, a| m | vars(a));
        if head_vars & !bound != 0 {
            return Err(format!("a head variable unbound in {}", rule.text));
        }
        Ok(steps)
    }

    fn index(&mut self, r: usize, key: &[usize]) -> IxRef {
        if r < self.prog.nscc {
            IxRef::Own(self.own[r].index(key))
        } else {
            IxRef::Shared(self.rel(r).shared_index(key))
        }
    }

    #[inline]
    fn row_of(&self, env: &Env, a: Arg) -> u32 {
        match a {
            Arg::Var(s) if env.r[s] != NONE => env.r[s],
            Arg::Var(s) => self.c.row_of(env.v[s]),
            Arg::Const(c) => self.c.row_of(c),
            Arg::Wild => NONE,
        }
    }

    /// A computed term's value; `None` where a function gives none.
    fn value(&self, env: &Env, e: &Expr) -> Option<i64> {
        let (f, args) = match e {
            Expr::A(a) => return Some(Self::val(env, *a)),
            Expr::Call(f, args) => (f, args),
        };
        let arg = |i: usize| self.value(env, &args[i]);
        // A text function's, `@cat`'s or a node read's value, by its arguments' values: once a stratum.
        let memo = matches!(f, Fun::Cat | Fun::NodeRead(_) | Fun::Eval(..)) && args.len() <= 3;
        let key = if memo {
            let mut vals = [i64::MIN; 3];
            for (v, i) in vals.iter_mut().zip(0..args.len()) {
                *v = arg(i)?;
            }
            let key = (std::ptr::from_ref(e) as usize, vals);
            if let Some(&x) = self.memo.borrow().get(&key) {
                return x;
            }
            Some(key)
        } else {
            None
        };
        let x = self.call(env, f, args);
        if let Some(key) = key {
            let mut m = self.memo.borrow_mut();
            if m.len() < MEMO {
                m.insert(key, x);
            }
        }
        x
    }

    /// Call `f` of `args`, evaluated.
    fn call(&self, env: &Env, f: &Fun, args: &[Expr]) -> Option<i64> {
        use crate::code_rule_dsl::logic::strings;
        let calls = self.calls;
        let arg = |i: usize| self.value(env, &args[i]);
        Some(match f {
            Fun::Cat => calls.cat(arg(0)?, arg(1)?),
            Fun::Doc => arg(0)? >> 32,
            Fun::NodeRead(name) => calls.intern(&calls.node_read(name, arg(0)?)?),
            Fun::In(set) => i64::from(set.contains(&arg(0)?)),
            Fun::Set(_) => return None,
            Fun::Test(op, held) if matches!(&args[1], Expr::Call(Fun::Set(_), _) | Expr::A(Arg::Const(_))) => {
                let x = arg(0)?;
                let m = held.0.get_or_init(|| {
                    // The patterns as `strings::test` reads them: the set's joined, split at its separator.
                    let joined = match &args[1] {
                        Expr::Call(Fun::Set(ps), _) => ps.iter().map(|&p| calls.name(p)).collect::<Option<Vec<_>>>()?.join(&strings::SET_SEP.to_string()),
                        p => calls.name(self.value(env, p)?)?,
                    };
                    let patterns: Vec<String> = joined.split(strings::SET_SEP).map(str::to_owned).collect();
                    strings::Matcher::new(op, &patterns).ok().map(Arc::new)
                });
                match m {
                    Some(m) => i64::from(calls.is_match(x, m)?),
                    None => 0,
                }
            }
            Fun::Test(op, _) => {
                let s = calls.name(arg(0)?)?;
                let pattern = match &args[1] {
                    Expr::Call(Fun::Set(ps), _) => ps.iter().map(|&p| calls.name(p)).collect::<Option<Vec<_>>>()?.join(&strings::SET_SEP.to_string()),
                    p => calls.name(self.value(env, p)?)?,
                };
                i64::from(strings::test(op, &s, &pattern))
            }
            Fun::Eval(name, kinds, out, swap) => {
                let mut values: Vec<i64> = (0..args.len()).map(arg).collect::<Option<_>>()?;
                if *swap {
                    values.swap(0, 1);
                }
                let mut texts = Vec::new();
                let mut numbers = Vec::new();
                for (k, &x) in kinds.iter().zip(&values) {
                    match k {
                        Kind::Text => texts.push(calls.name(x)?),
                        Kind::Number => numbers.push(x),
                    }
                }
                let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
                match (strings::functions::eval(name, &texts, &numbers), out) {
                    (rusqlite::types::Value::Integer(n), _) => n,
                    (rusqlite::types::Value::Text(t), _) => calls.intern(&t),
                    _ => return None,
                }
            }
        })
    }

    #[inline]
    fn val(env: &Env, a: Arg) -> i64 {
        match a {
            Arg::Var(s) => env.v[s],
            Arg::Const(c) => c,
            Arg::Wild => unreachable!("a wildcard's value"),
        }
    }

    /// Applies a step's acts to a tuple's columns (value, tree row), then goes on to the next step.
    #[inline]
    fn accept(&self, rule: &RuleP, steps: &[Step], i: usize, cols: &[(i64, u32)], env: &mut Env, out: &mut Vec<i64>) {
        for (&(x, r), act) in cols.iter().zip(&steps[i].acts) {
            match *act {
                Act::Skip => {}
                Act::Is(c) => {
                    if x != c {
                        return;
                    }
                }
                Act::Test(s) => {
                    if env.v[s] != x {
                        return;
                    }
                    if r != NONE {
                        env.r[s] = r;
                    }
                }
                Act::Bind(s) => {
                    env.v[s] = x;
                    env.r[s] = r;
                }
            }
        }
        self.go(rule, steps, i + 1, env, out);
    }

    #[inline]
    fn text(&self, r: u32) -> i64 {
        self.c.text(r)
    }

    fn go(&self, rule: &RuleP, steps: &[Step], i: usize, env: &mut Env, out: &mut Vec<i64>) {
        let Some(step) = steps.get(i) else {
            let at = out.len();
            for a in &rule.args {
                match self.value(env, a) {
                    Some(x) => out.push(x),
                    None => {
                        out.truncate(at);
                        return;
                    }
                }
            }
            return;
        };
        let t: &Tree = self.c.t;
        let id = |r: u32| (t.id(r), r);
        match &rule.lits[step.lit] {
            LitP::Cmp { l, op, r } => {
                if let Some((s, from)) = binding(step.bound, l, r) {
                    let Some(x) = self.value(env, from) else { return };
                    env.v[s] = x;
                    env.r[s] = if let Expr::A(Arg::Var(f)) = from { env.r[*f] } else { NONE };
                    self.go(rule, steps, i + 1, env, out);
                    return;
                }
                // `q = @cat(a, b)`, `q` known: each way `@cat` makes `q`: `"" . q`, and `q`'s text split at each dot past its
                // first character (`@cat("", y)` is `y`).
                if *op == CmpOp::Eq
                    && let Some((whole, a, b)) = split(step.bound, l, r)
                {
                    let Some(q) = self.value(env, whole) else { return };
                    let Some(text) = self.calls.name(q) else { return };
                    // A bound part's text, which its split's must be.
                    let fixed = |s: usize| if step.bound >> s & 1 == 1 { self.calls.name(env.v[s]).map(Some) } else { Some(None) };
                    let (Some(fa), Some(fb)) = (fixed(a), fixed(b)) else { return };
                    let splits = std::iter::once(("", text.as_str())).chain(text.match_indices('.').filter(|&(at, _)| at > 0).map(|(at, _)| (&text[..at], &text[at + 1..])));
                    for (n, (pa, pb)) in splits.enumerate() {
                        if fa.as_deref().is_some_and(|x| x != pa) || fb.as_deref().is_some_and(|x| x != pb) {
                            continue;
                        }
                        if fa.is_none() {
                            env.r[a] = NONE;
                            env.v[a] = self.calls.atom(pa);
                        }
                        if fb.is_none() {
                            env.r[b] = NONE;
                            env.v[b] = if n == 0 { q } else { self.calls.atom(pb) };
                        }
                        self.go(rule, steps, i + 1, env, out);
                    }
                    return;
                }
                {
                    {
                        let (Some(a), Some(b)) = (self.value(env, l), self.value(env, r)) else { return };
                        let pass = match op {
                            CmpOp::Eq => a == b,
                            CmpOp::Ne => a != b,
                            CmpOp::Lt => a < b,
                            CmpOp::Le => a <= b,
                            CmpOp::Gt => a > b,
                            CmpOp::Ge => a >= b,
                        };
                        if pass {
                            self.go(rule, steps, i + 1, env, out);
                        }
                    }
                }
            }
            LitP::Agg { var, op, over, atom } => {
                let Src::Rel(r) = atom.src else { unreachable!() };
                let h = self.rel(r);
                let mut key = [0i64; 16];
                for (x, &c) in key.iter_mut().zip(&step.key) {
                    *x = Self::val(env, atom.args[c]);
                }
                let key = &key[..step.key.len()];
                let (mut count, mut acc): (i64, Option<i64>) = (0, None);
                let mut take = |id: u32| {
                    count += 1;
                    if let Some(c) = over {
                        let x = h.get(id, *c);
                        acc = Some(match (op, acc) {
                            (_, None) => x,
                            (AggOp::Min, Some(a)) => a.min(x),
                            (AggOp::Max, Some(a)) => a.max(x),
                            (AggOp::Sum, Some(a)) => a + x,
                            (AggOp::Count, Some(a)) => a,
                        });
                    }
                };
                match &step.ix {
                    Some(IxRef::Own(at)) => {
                        let ix = &h.idx[*at];
                        let mut at = ix.find(&h.vals, h.k, key);
                        while at != NONE {
                            take(at);
                            at = ix.next[at as usize];
                        }
                    }
                    Some(IxRef::Shared(ix)) => ix.find(&h.vals, h.k, key).iter().for_each(|&id| take(id)),
                    None => (0..h.n as u32).for_each(&mut take),
                }
                let value = match op {
                    AggOp::Count => Some(count),
                    AggOp::Sum => Some(acc.unwrap_or(0)),
                    AggOp::Min | AggOp::Max => acc,
                };
                let Some(value) = value else { return };
                if step.bound >> var & 1 == 1 {
                    if env.v[*var] != value {
                        return;
                    }
                } else {
                    env.v[*var] = value;
                    env.r[*var] = NONE;
                }
                self.go(rule, steps, i + 1, env, out);
            }
            LitP::Atom(a) => {
                let b = |k: usize| is_bound(step.bound, a.args[k]);
                if a.neg {
                    if !self.exists(a, step, env) {
                        self.go(rule, steps, i + 1, env, out);
                    }
                    return;
                }
                match &a.src {
                    Src::Rel(r) => {
                        let h = self.rel(*r);
                        let (lo, hi) = if *r < self.prog.nscc {
                            match step.part {
                                Part::All => (0, self.hi[*r]),
                                Part::Delta => (self.lo[*r], self.hi[*r]),
                            }
                        } else {
                            (0, h.n)
                        };
                        let k = h.k;
                        // A row's columns read only where an act needs them (`Step::reads`).
                        let visit = |id: u32, env: &mut Env, out: &mut Vec<i64>| {
                            for &(c, act) in &step.reads {
                                let x = h.get(id, c);
                                match act {
                                    Act::Skip => {}
                                    Act::Is(v) => {
                                        if x != v {
                                            return;
                                        }
                                    }
                                    Act::Test(s) => {
                                        if env.v[s] != x {
                                            return;
                                        }
                                    }
                                    Act::Bind(s) => {
                                        env.v[s] = x;
                                        env.r[s] = NONE;
                                    }
                                }
                            }
                            self.go(rule, steps, i + 1, env, out);
                        };
                        match &step.ix {
                            Some(IxRef::Shared(ix)) => {
                                let mut key = [0i64; 16];
                                for (x, &c) in key.iter_mut().zip(&step.key) {
                                    *x = Self::val(env, a.args[c]);
                                }
                                for &id in ix.find(&h.vals, k, &key[..step.key.len()]) {
                                    visit(id, env, out);
                                }
                            }
                            Some(IxRef::Own(at)) => {
                                let mut key = [0i64; 16];
                                for (x, &c) in key.iter_mut().zip(&step.key) {
                                    *x = Self::val(env, a.args[c]);
                                }
                                let ix = &h.idx[*at];
                                let mut at = ix.find(&h.vals, k, &key[..step.key.len()]);
                                while at != NONE {
                                    let id = at as usize;
                                    if id < lo {
                                        break;
                                    }
                                    if id < hi {
                                        visit(at, env, out);
                                    }
                                    at = ix.next[id];
                                }
                            }
                            None if k == 0 => {
                                if hi > lo {
                                    self.go(rule, steps, i + 1, env, out);
                                }
                            }
                            None => (lo..hi).for_each(|id| visit(id as u32, env, out)),
                        }
                    }
                    Src::Tree(tr) => match tr {
                        TreeRel::Kind => {
                            if b(0) {
                                let r = self.row_of(env, a.args[0]);
                                if r != NONE {
                                    self.accept(rule, steps, i, &[id(r), (t.kind(r), NONE)], env, out);
                                }
                            } else if b(1) {
                                let k = Self::val(env, a.args[1]);
                                for &r in by_kind(t).of(k) {
                                    self.accept(rule, steps, i, &[id(r), (k, NONE)], env, out);
                                }
                            } else {
                                for r in 0..t.len() as u32 {
                                    self.accept(rule, steps, i, &[id(r), (t.kind(r), NONE)], env, out);
                                }
                            }
                        }
                        TreeRel::Node => {
                            let tuple = |r: u32| {
                                let (ru, sp) = (r as usize, self.c.spans());
                                [(t.file[t.doc[ru] as usize], NONE), id(r), (t.kind(r), NONE), (i64::from(sp.0[ru]), NONE), (i64::from(sp.1[ru]), NONE)]
                            };
                            if b(1) {
                                let r = self.row_of(env, a.args[1]);
                                if r != NONE {
                                    self.accept(rule, steps, i, &tuple(r), env, out);
                                }
                            } else if b(2) {
                                let k = Self::val(env, a.args[2]);
                                for &r in by_kind(t).of(k) {
                                    self.accept(rule, steps, i, &tuple(r), env, out);
                                }
                            } else {
                                for r in 0..t.len() as u32 {
                                    self.accept(rule, steps, i, &tuple(r), env, out);
                                }
                            }
                        }
                        TreeRel::Child => {
                            let tuple = |c: u32| [id(t.sh().parent[c as usize]), id(c), (i64::from(t.sh().idx[c as usize]), NONE)];
                            if b(1) {
                                let c = self.row_of(env, a.args[1]);
                                if c != NONE && t.sh().parent[c as usize] != NONE {
                                    self.accept(rule, steps, i, &tuple(c), env, out);
                                }
                            } else if b(0) {
                                let p = self.row_of(env, a.args[0]);
                                if p != NONE {
                                    for &c in t.kids(p) {
                                        self.accept(rule, steps, i, &tuple(c), env, out);
                                    }
                                }
                            } else {
                                for c in 0..t.len() as u32 {
                                    if t.sh().parent[c as usize] != NONE {
                                        self.accept(rule, steps, i, &tuple(c), env, out);
                                    }
                                }
                            }
                        }
                        TreeRel::Field => {
                            let tuple = |c: u32| [id(t.sh().parent[c as usize]), (t.sh().field[c as usize], NONE), id(c)];
                            let fielded = |c: u32| t.sh().field[c as usize] != NIL && t.sh().parent[c as usize] != NONE;
                            if b(2) {
                                let c = self.row_of(env, a.args[2]);
                                if c != NONE && fielded(c) {
                                    self.accept(rule, steps, i, &tuple(c), env, out);
                                }
                            } else if b(0) {
                                let p = self.row_of(env, a.args[0]);
                                if p != NONE {
                                    for &c in t.kids(p) {
                                        if fielded(c) {
                                            self.accept(rule, steps, i, &tuple(c), env, out);
                                        }
                                    }
                                }
                            } else {
                                for c in 0..t.len() as u32 {
                                    if fielded(c) {
                                        self.accept(rule, steps, i, &tuple(c), env, out);
                                    }
                                }
                            }
                        }
                        TreeRel::Text => {
                            let r = self.row_of(env, a.args[0]);
                            if r != NONE {
                                let s = self.text(r);
                                if s != NIL {
                                    self.accept(rule, steps, i, &[id(r), (s, NONE)], env, out);
                                }
                            }
                        }
                        TreeRel::Within => {
                            let (bn, ba) = (b(0), b(1));
                            if bn {
                                let n = self.row_of(env, a.args[0]);
                                if n == NONE {
                                    return;
                                }
                                if ba {
                                    let an = self.row_of(env, a.args[1]);
                                    if an != NONE && t.within(n, an) {
                                        self.accept(rule, steps, i, &[id(n), id(an)], env, out);
                                    }
                                    return;
                                }
                                let mut up = t.sh().parent[n as usize];
                                while up != NONE {
                                    self.accept(rule, steps, i, &[id(n), id(up)], env, out);
                                    up = t.sh().parent[up as usize];
                                }
                            } else {
                                let an = self.row_of(env, a.args[1]);
                                if an != NONE {
                                    for n in an + 1..t.sh().end[an as usize] {
                                        self.accept(rule, steps, i, &[id(n), id(an)], env, out);
                                    }
                                }
                            }
                        }
                        TreeRel::Nearest(set) => {
                            let near = nearest(t, set);
                            if b(0) {
                                let n = self.row_of(env, a.args[0]);
                                if n != NONE && near[n as usize] != NONE {
                                    self.accept(rule, steps, i, &[id(n), id(near[n as usize])], env, out);
                                }
                            } else if b(1) {
                                let an = self.row_of(env, a.args[1]);
                                if an != NONE {
                                    for n in an + 1..t.sh().end[an as usize] {
                                        if near[n as usize] == an {
                                            self.accept(rule, steps, i, &[id(n), id(an)], env, out);
                                        }
                                    }
                                }
                            } else {
                                for n in 0..t.len() as u32 {
                                    if near[n as usize] != NONE {
                                        self.accept(rule, steps, i, &[id(n), id(near[n as usize])], env, out);
                                    }
                                }
                            }
                        }
                        TreeRel::Inside(set) | TreeRel::Has(set) => {
                            // Inside: `a` in the set; has: `n` in the set.
                            let has = matches!(tr, TreeRel::Has(_));
                            let ok = |n: u32, a: u32| set.contains(&t.kind(if has { n } else { a }));
                            let ups = |n: u32, env: &mut Env, out: &mut Vec<i64>| {
                                let mut up = t.sh().parent[n as usize];
                                while up != NONE {
                                    if ok(n, up) {
                                        self.accept(rule, steps, i, &[id(n), id(up)], env, out);
                                    }
                                    up = t.sh().parent[up as usize];
                                }
                            };
                            if b(0) {
                                let n = self.row_of(env, a.args[0]);
                                if n != NONE {
                                    ups(n, env, out);
                                }
                            } else if b(1) {
                                let an = self.row_of(env, a.args[1]);
                                if an != NONE {
                                    for n in an + 1..t.sh().end[an as usize] {
                                        if ok(n, an) {
                                            self.accept(rule, steps, i, &[id(n), id(an)], env, out);
                                        }
                                    }
                                }
                            } else {
                                for n in 0..t.len() as u32 {
                                    ups(n, env, out);
                                }
                            }
                        }
                    },
                }
            }
        }
    }

    /// Whether a negated atom's bound columns match a row.
    fn exists(&self, a: &AtomP, step: &Step, env: &mut Env) -> bool {
        let t = self.c.t;
        let val = |k: usize| Self::val(env, a.args[k]);
        let wild = |k: usize| a.args[k] == Arg::Wild;
        match &a.src {
            Src::Rel(r) => {
                let h = self.rel(*r);
                match &step.ix {
                    Some(ix) => {
                        let mut key = [0i64; 16];
                        for (x, &c) in key.iter_mut().zip(&step.key) {
                            *x = val(c);
                        }
                        let key = &key[..step.key.len()];
                        match ix {
                            IxRef::Own(at) => h.idx[*at].find(&h.vals, h.k, key) != NONE,
                            IxRef::Shared(ix) => !ix.find(&h.vals, h.k, key).is_empty(),
                        }
                    }
                    None => h.n > 0,
                }
            }
            Src::Tree(tr) => {
                let row = |k: usize| self.row_of(env, a.args[k]);
                match tr {
                    TreeRel::Kind => {
                        let r = row(0);
                        r != NONE && (wild(1) || t.kind(r) == val(1))
                    }
                    TreeRel::Node => {
                        let r = row(1);
                        if r == NONE {
                            return false;
                        }
                        let (ru, sp) = (r as usize, self.c.spans());
                        let cols = [t.file[t.doc[ru] as usize], 0, t.kind(r), i64::from(sp.0[ru]), i64::from(sp.1[ru])];
                        [0, 2, 3, 4].iter().all(|&k| wild(k) || cols[k] == val(k))
                    }
                    TreeRel::Child => {
                        if wild(1) {
                            let p = row(0);
                            return p != NONE && t.kids(p).iter().any(|&c| wild(2) || i64::from(t.sh().idx[c as usize]) == val(2));
                        }
                        let c = row(1);
                        c != NONE && t.sh().parent[c as usize] != NONE && (wild(0) || t.id(t.sh().parent[c as usize]) == val(0)) && (wild(2) || i64::from(t.sh().idx[c as usize]) == val(2))
                    }
                    TreeRel::Field => {
                        if wild(2) {
                            let p = row(0);
                            return p != NONE && t.kids(p).iter().any(|&c| t.sh().field[c as usize] != NIL && (wild(1) || t.sh().field[c as usize] == val(1)));
                        }
                        let c = row(2);
                        c != NONE && t.sh().parent[c as usize] != NONE && t.sh().field[c as usize] != NIL && (wild(0) || t.id(t.sh().parent[c as usize]) == val(0)) && (wild(1) || t.sh().field[c as usize] == val(1))
                    }
                    TreeRel::Text => {
                        let r = row(0);
                        r != NONE && {
                            let s = self.text(r);
                            s != NIL && (wild(1) || s == val(1))
                        }
                    }
                    TreeRel::Within => {
                        let (n, an) = (row(0), row(1));
                        match (wild(0), wild(1)) {
                            (false, false) => n != NONE && an != NONE && t.within(n, an),
                            (false, true) => n != NONE && t.sh().parent[n as usize] != NONE,
                            (true, false) => an != NONE && t.sh().end[an as usize] > an + 1,
                            (true, true) => true,
                        }
                    }
                    TreeRel::Nearest(set) => {
                        let near = nearest(t, set);
                        let n = row(0);
                        n != NONE && near[n as usize] != NONE && (wild(1) || t.id(near[n as usize]) == val(1))
                    }
                    TreeRel::Inside(set) | TreeRel::Has(set) => {
                        let has = matches!(tr, TreeRel::Has(_));
                        let n = row(0);
                        if n == NONE {
                            return false;
                        }
                        let target = (!wild(1)).then(|| val(1));
                        let mut up = t.sh().parent[n as usize];
                        while up != NONE {
                            if set.contains(&t.kind(if has { n } else { up })) && target.is_none_or(|x| t.id(up) == x) {
                                return true;
                            }
                            up = t.sh().parent[up as usize];
                        }
                        false
                    }
                }
            }
        }
    }

    fn run_rule(&self, rule: &RuleP, steps: &[Step], out: &mut Vec<i64>) {
        let mut env = Env { v: vec![0; rule.nslots], r: vec![NONE; rule.nslots] };
        let t0 = verbose().then(std::time::Instant::now);
        let before = out.len();
        self.go(rule, steps, 0, &mut env, out);
        if let Some(t0) = t0
            && t0.elapsed().as_secs_f64() > 0.2
        {
            let order: Vec<String> = steps.iter().map(|s| format!("{:?} bound {:#x} key {:?} {:?}", rule.lits[s.lit], s.bound, s.key, s.part)).collect();
            eprintln!("SPIKE SLOW {} {:.3} s, {} values: {}", rule.text, t0.elapsed().as_secs_f64(), out.len() - before, order.join(" ; "));
        }
    }
}

/// The rows of each kind, in one array (a counting sort of the rows by kind, each kind's in row order), and each
/// kind's place in it.
pub struct KindRows {
    pub(super) order: Vec<u32>,
    at: HashMap<i64, (u32, u32)>,
}

impl KindRows {
    /// The rows of kind `k`.
    #[inline]
    fn of(&self, k: i64) -> &[u32] {
        self.at.get(&k).map_or(&[], |&(lo, hi)| &self.order[lo as usize..hi as usize])
    }
}

/// The rows of each kind, made once for the chunk (`Tree::by_kind`).
fn by_kind(t: &Tree) -> &KindRows {
    t.by_kind.get_or_init(|| {
        let mut at: HashMap<i64, (u32, u32)> = HashMap::default();
        for &k in &t.kinds {
            at.entry(i64::from(k)).or_default().1 += 1;
        }
        let mut next = 0;
        for (lo, hi) in at.values_mut() {
            *lo = next;
            next += *hi;
            *hi = *lo;
        }
        let mut order = vec![0u32; t.len()];
        for (r, &k) in t.kinds.iter().enumerate() {
            let fill = &mut at.get_mut(&i64::from(k)).expect("counted").1;
            order[*fill as usize] = r as u32;
            *fill += 1;
        }
        KindRows { order, at }
    })
}

/// Each row's nearest strict ancestor of a kind in `set`, made once for the chunk (`Tree::near_by`).
fn nearest(t: &Tree, set: &[i64]) -> Arc<Vec<u32>> {
    if let Some(n) = t.near_by.lock().unwrap().get(set) {
        return Arc::clone(n);
    }
    let n = t.len();
    let mut near = vec![NONE; n];
    // Preorder rows: a parent's row is before its children's.
    for r in 0..n {
        let p = t.sh().parent[r];
        if p != NONE {
            near[r] = if set.contains(&t.kind(p)) { p } else { near[p as usize] };
        }
    }
    let near = Arc::new(near);
    t.near_by.lock().unwrap().insert(set.to_vec(), Arc::clone(&near));
    near
}

/// The relations a stratum reads that it does not make, by place in `prog.rels`.
pub fn reads(prog: &Prog) -> impl Iterator<Item = (usize, &str)> {
    prog.rels.iter().enumerate().skip(prog.nscc).map(|(i, r)| (i, r.as_str()))
}

/// Whether `prog` reads a tree relation (which a session with no syntax batch has none of).
pub fn reads_tree(prog: &Prog) -> bool {
    prog.rules.iter().flat_map(|r| &r.lits).any(|l| match l {
        LitP::Atom(a) | LitP::Agg { atom: a, .. } => matches!(a.src, Src::Tree(_)),
        LitP::Cmp { .. } => false,
    })
}

/// Evaluates `prog` over chunk `c`, the relations it reads given (`read`, by place; `None` for the stratum's own):
/// its relations' rows, or why not (a rule no order binds).
pub fn run(prog: &Prog, c: &Chunk<'_>, read: Vec<Option<Arc<HRel>>>, calls: &dyn Calls) -> Result<Vec<HRel>, String> {
    let _span = super::super::profile::span(|| format!("spike eval: {}", prog.rels[..prog.nscc].join(",")));
    let shaped = |tr: &TreeRel| !matches!(tr, TreeRel::Kind | TreeRel::Node | TreeRel::Text);
    let reads_shape = prog.rules.iter().flat_map(|r| &r.lits).any(|l| match l {
        LitP::Atom(a) | LitP::Agg { atom: a, .. } => matches!(&a.src, Src::Tree(tr) if shaped(tr)),
        LitP::Cmp { .. } => false,
    });
    if reads_shape {
        c.shape();
    }
    let own: Vec<HRel> = (0..prog.nscc).map(|r| HRel::new(prog.widths[r])).collect();
    let mut ev = Eval { prog, calls, c, own, read, lo: vec![0; prog.nscc], hi: vec![0; prog.nscc], memo: Default::default() };
    // Wave 0: every rule over its relations as they are (the stratum's empty).
    let mut plans: Vec<Vec<Step>> = Vec::new();
    for rule in &prog.rules {
        plans.push(ev.plan(rule, None)?);
    }
    let t0 = std::time::Instant::now();
    // Each rule's time and runs, for a slow stratum's report (`RERAIL_SPIKE_WHY`).
    let mut spent = vec![(0f64, 0u32); prog.rules.len()];
    let mut pending: Vec<Vec<i64>> = (0..prog.nscc).map(|_| Vec::new()).collect();
    for (ri, (rule, steps)) in prog.rules.iter().zip(&plans).enumerate() {
        if rule.rec.is_empty() {
            let t = std::time::Instant::now();
            ev.run_rule(rule, steps, &mut pending[rule.head]);
            spent[ri].0 += t.elapsed().as_secs_f64();
            spent[ri].1 += 1;
        }
    }
    let recursive: Vec<&RuleP> = prog.rules.iter().filter(|r| !r.rec.is_empty()).collect();
    let mut delta_plans: HashMap<(usize, usize), Vec<Step>> = HashMap::default();
    loop {
        // The wave's new rows become the delta.
        let mut grew = false;
        for (r, rows) in pending.iter_mut().enumerate() {
            let h = &mut ev.own[r];
            let k = h.k;
            ev.lo[r] = h.n;
            for row in rows.chunks_exact(k) {
                h.insert(row);
            }
            rows.clear();
            ev.hi[r] = h.n;
            grew |= ev.hi[r] > ev.lo[r];
            let cols: Vec<Vec<usize>> = h.idx.iter().map(|i| i.cols.clone()).collect();
            for cols in cols {
                h.index(&cols);
            }
        }
        if !grew || recursive.is_empty() {
            break;
        }
        for (ri, rule) in prog.rules.iter().enumerate() {
            for &d in &rule.rec {
                let Src::Rel(dr) = (match &rule.lits[d] {
                    LitP::Atom(a) => a.src.clone(),
                    _ => unreachable!(),
                }) else {
                    unreachable!()
                };
                if ev.hi[dr] == ev.lo[dr] {
                    continue;
                }
                let steps = match delta_plans.entry((ri, d)) {
                    std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                    std::collections::hash_map::Entry::Vacant(e) => e.insert(ev.plan(rule, Some(d))?),
                };
                let t = std::time::Instant::now();
                ev.run_rule(rule, steps, &mut pending[rule.head]);
                spent[ri].0 += t.elapsed().as_secs_f64();
                spent[ri].1 += 1;
            }
        }
    }
    if verbose() && t0.elapsed().as_secs_f64() > 1.0 {
        let plan = |steps: &[Step]| {
            steps
                .iter()
                .map(|s| {
                    let how = match &s.ix {
                        None => "scan",
                        Some(IxRef::Own(_)) => "own",
                        Some(IxRef::Shared(_)) => "held",
                    };
                    format!("{}{how}{:?}{}", s.lit, s.key, if s.part == Part::Delta { "Δ" } else { "" })
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        let sizes: Vec<String> = prog.rels.iter().enumerate().map(|(r, n)| format!("{n} {}", ev.rel(r).n)).collect();
        eprintln!("SPIKE SLOW {} {:.2} s; sizes {}", prog.rels[..prog.nscc].join(","), t0.elapsed().as_secs_f64(), sizes.join(", "));
        for (ri, rule) in prog.rules.iter().enumerate() {
            let deltas: Vec<String> = delta_plans.iter().filter(|((r, _), _)| *r == ri).map(|((_, d), st)| format!("Δ{d}: {}", plan(st))).collect();
            eprintln!("SPIKE SLOW   {:.2} s {} runs: {} | wave 0: {} | {}", spent[ri].0, spent[ri].1, rule.text, plan(&plans[ri]), deltas.join(" | "));
        }
    }
    let mut own = ev.own;
    for h in &mut own {
        h.seal();
    }
    Ok(own)
}

/// Logs why a stratum is declined, once per stratum (`RERAIL_SPIKE_WHY`).
pub fn declined(scc: &[&str], why: &str) {
    if verbose() {
        eprintln!("SPIKE EVAL declined {}: {why}", scc.join(","));
    }
}
