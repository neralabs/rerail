//! `facts-b hreplay`: M2, the hybrid cross-file model (PHASE-3 § P3a), on the same replay as
//! `greplay`.
//!
//! Resident state: the exported per-content relations, each (export, path, content) block stored
//! once in 32 bits a value when every value's word fits (`Block`), hash-consed across exports,
//! programs and paths, reference-counted by the worktrees holding the (path, content) and freed
//! when none does; per worktree its membership and each plan's result (findings and refusals).
//! Nothing else survives an edit: a plan's stages (the membership-dependent `module_of`, the
//! linker views and the rule logic alike) are evaluated from scratch by `batch` over the
//! worktree's members, and the strings that evaluation computes are dropped with it (the
//! interner is truncated to its mark; retained rows are re-interned).
//!
//! Per edit, plan by plan, the delta rule D1 (REWRITES.md): a plan is a function of the multisets
//! of its exported leaves over the worktree's members (I6, I7; the generator's semantics are
//! order-free, an order dependence is a refusal), so when every block the plan reads is the same
//! block before and after the event at every changed path, its result is unchanged and is not
//! recomputed. Otherwise the plan runs from scratch over the worktree. `--no-skip` recomputes every
//! plan on every event.
//!
//! Exactness (`--check batch|engine`): after every `--check-every`-th event the edited worktree
//! (and at load and every 100 edits the `--sweep` worktrees) is compared with a from-scratch run of
//! the same plans: `batch` re-runs them over the members; `engine` also runs the generator's
//! dataflow over the members (`drive::scratch`'s second dataflow, base records only), so the
//! executor itself is checked against the incremental engine's semantics.
use super::batch::{self, Counts, Outcome};
use super::lower::{self, Source, R};
use super::norm::{Graph, Op};
use super::real::{self, Real};
use super::tags::{W, BASE};
use super::val::{self, Row, Val};
use rerail_facts::sql::value::V;
use crate::derive::World;
use crate::membership::Members;
use crate::replay::{read_events, rss, Event};
use crate::store::{self, Cid, Store, Sym};
use rustc_hash::FxHashMap;
use serde_json::json;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Check {
    None,
    Batch,
    Engine,
}

pub struct Opts {
    pub events: Option<PathBuf>,
    pub limit: Option<usize>,
    pub real: PathBuf,
    pub check: Check,
    pub check_every: usize,
    pub sweep: Option<usize>,
    pub skip: bool,
    /// Load only the first n worktrees (profiles and smoke runs; no events).
    pub worktrees: Option<usize>,
    /// Plan runs in flight at once (forked workers); 1: one at a time.
    pub workers: usize,
    /// X4, design (d): the declared facts (`declared::frontier`) are evaluated first and timed
    /// apart; latencies are the rule logic's recompute.
    pub maintained: bool,
}

// --------------------------------------------------------------------------------------------
// The resident exported relations

enum Words {
    Narrow(Box<[u32]>),
    Wide(Box<[u64]>),
}

/// One (export, path, content)'s rows: `n` rows of `width` values, row-major.
struct Block {
    width: u32,
    n: u32,
    words: Words,
    refs: u32,
}

impl Block {
    fn new(rows: &[Row]) -> Block {
        let width = rows[0].len();
        assert!(rows.iter().all(|r| r.len() == width), "export rows of different widths");
        let bits: Vec<u64> = rows.iter().flat_map(|r| r.iter().map(|v| v.to_bits())).collect();
        let words = if bits.iter().all(|b| *b <= u32::MAX as u64) { Words::Narrow(bits.iter().map(|b| *b as u32).collect()) } else { Words::Wide(bits.into()) };
        Block { width: width as u32, n: rows.len() as u32, words, refs: 0 }
    }
    fn word(&self, i: usize) -> u64 {
        match &self.words {
            Words::Narrow(w) => w[i] as u64,
            Words::Wide(w) => w[i],
        }
    }
    fn rows(&self) -> Vec<Row> {
        let w = self.width as usize;
        let mut buf = Vec::with_capacity(w);
        (0..self.n as usize)
            .map(|r| {
                buf.clear();
                buf.extend((0..w).map(|c| Val::from_bits(self.word(r * w + c))));
                Row::new(&buf)
            })
            .collect()
    }
    fn same(&self, o: &Block) -> bool {
        self.width == o.width && self.n == o.n && (0..(self.width * self.n) as usize).all(|i| self.word(i) == o.word(i))
    }
    fn hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = rustc_hash::FxHasher::default();
        (self.width, self.n).hash(&mut h);
        for i in 0..(self.width * self.n) as usize {
            self.word(i).hash(&mut h);
        }
        h.finish()
    }
    fn bytes(&self) -> usize {
        std::mem::size_of::<Block>()
            + match &self.words {
                Words::Narrow(w) => w.len() * 4,
                Words::Wide(w) => w.len() * 8,
            }
    }
}

#[derive(Default)]
struct Exports {
    /// Global export ids some plan reads.
    read: BTreeSet<u32>,
    /// (export, path, content) → block; absent: no rows.
    at: FxHashMap<(u32, Sym, Cid), u32>,
    blocks: Vec<Option<Block>>,
    free: Vec<u32>,
    by_hash: FxHashMap<u64, Vec<u32>>,
    /// Worktrees holding each (path, content).
    held: FxHashMap<(Sym, Cid), u32>,
    /// Contents without exports (the producer failed, or no retained source): no rows, counted
    /// as a runtime refusal when acquired.
    missing: FxHashMap<(Sym, Cid), String>,
    rows_resident: u64,
}

impl Exports {
    fn intern(&mut self, b: Block) -> u32 {
        let h = b.hash();
        if let Some(ids) = self.by_hash.get(&h) {
            for &i in ids {
                if self.blocks[i as usize].as_ref().unwrap().same(&b) {
                    self.blocks[i as usize].as_mut().unwrap().refs += 1;
                    return i;
                }
            }
        }
        self.rows_resident += b.n as u64;
        let mut b = b;
        b.refs = 1;
        let i = match self.free.pop() {
            Some(i) => {
                self.blocks[i as usize] = Some(b);
                i
            }
            None => {
                self.blocks.push(Some(b));
                (self.blocks.len() - 1) as u32
            }
        };
        self.by_hash.entry(h).or_default().push(i);
        i
    }

    fn unref(&mut self, i: u32) {
        let b = self.blocks[i as usize].as_mut().unwrap();
        b.refs -= 1;
        if b.refs == 0 {
            let b = self.blocks[i as usize].take().unwrap();
            self.rows_resident -= b.n as u64;
            let h = b.hash();
            let v = self.by_hash.get_mut(&h).unwrap();
            v.retain(|x| *x != i);
            if v.is_empty() {
                self.by_hash.remove(&h);
            }
            self.free.push(i);
        }
    }

    fn acquire(&mut self, real: &Real, p: Sym, c: Cid, path: &str, hash: &str) {
        let h = self.held.entry((p, c)).or_default();
        *h += 1;
        if *h > 1 {
            return;
        }
        for k in 0..real.programs() {
            match real.program_rows(k, path, hash) {
                Ok(rows) => {
                    for (e, rs) in rows {
                        if self.read.contains(&e) && !rs.is_empty() {
                            let id = self.intern(Block::new(&rs));
                            self.at.insert((e, p, c), id);
                        }
                    }
                }
                Err(m) => {
                    lower::refuse_external(&m);
                    self.missing.insert((p, c), m);
                }
            }
        }
    }

    fn release(&mut self, p: Sym, c: Cid) {
        let h = self.held.get_mut(&(p, c)).expect("release of a content not held");
        *h -= 1;
        if *h > 0 {
            return;
        }
        self.held.remove(&(p, c));
        self.missing.remove(&(p, c));
        for e in self.read.clone() {
            if let Some(i) = self.at.remove(&(e, p, c)) {
                self.unref(i);
            }
        }
    }

    fn block(&self, e: u32, p: Sym, c: Cid) -> Option<u32> {
        self.at.get(&(e, p, c)).copied()
    }

    fn rows(&self, e: u32, p: Sym, c: Cid) -> Rc<[Row]> {
        match self.at.get(&(e, p, c)) {
            Some(i) => self.blocks[*i as usize].as_ref().unwrap().rows().into(),
            None => Rc::from(Vec::new()),
        }
    }

    /// (blocks, block bytes, narrow blocks, `at` entries).
    fn footprint(&self) -> (usize, usize, usize, usize) {
        let live = self.blocks.iter().flatten();
        let (mut n, mut bytes, mut narrow) = (0, 0, 0);
        for b in live {
            n += 1;
            bytes += b.bytes();
            narrow += matches!(b.words, Words::Narrow(_)) as usize;
        }
        (n, bytes, narrow, self.at.len())
    }
}

// --------------------------------------------------------------------------------------------
// Per-worktree results

/// A plan's result in one worktree: per output its findings, and its refusals (findings withheld
/// while any is live).
#[derive(Clone, PartialEq)]
struct Res {
    outputs: Vec<(String, BTreeMap<Row, R>)>,
    refusals: Vec<Row>,
}

impl Res {
    fn rows(&self) -> usize {
        self.outputs.iter().map(|(_, m)| m.len()).sum::<usize>() + self.refusals.len()
    }
    fn bytes(&self) -> usize {
        self.outputs.iter().flat_map(|(_, m)| m.keys()).chain(self.refusals.iter()).map(|r| std::mem::size_of::<(Row, R)>() + 32 + r.heap_bytes()).sum()
    }
}

fn show(r: &Row) -> String {
    let parts: Vec<String> = r.iter().map(|v| format!("{:?}", val::to_v(*v))).collect();
    format!("({})", parts.join(", "))
}

fn pct(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    s[((s.len() - 1) as f64 * p).round() as usize]
}

fn summary(v: &[f64]) -> serde_json::Value {
    json!({"n": v.len(), "p50_ms": pct(v, 0.5), "p95_ms": pct(v, 0.95), "p99_ms": pct(v, 0.99), "max_ms": pct(v, 1.0)})
}

fn mb(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

// --------------------------------------------------------------------------------------------
// Workers in flight: a plan run (and a check) happens in a forked child, so every relation,
// string and number the evaluation makes dies with it and the parent's resident size is the
// model's state (PHASE-3 § P3c counts workers in flight separately). The child returns its
// result as JSON; text values cross as strings, reals as their bits.

fn put_val(v: Val) -> serde_json::Value {
    match val::to_v(v) {
        V::Null => serde_json::Value::Null,
        V::Int(i) => json!(i),
        V::Real(f) => json!({ "r": f.to_bits() }),
        V::Text(s) => json!(&*s),
    }
}

fn get_val(j: &serde_json::Value) -> Val {
    match j {
        serde_json::Value::Null => Val::NULL,
        serde_json::Value::Number(n) => Val::int(n.as_i64().unwrap()),
        serde_json::Value::String(s) => Val::text(val::sym(s)),
        serde_json::Value::Object(m) => Val::real(m["r"].as_u64().unwrap()),
        o => panic!("not a value: {o}"),
    }
}

fn put_row(r: &Row) -> serde_json::Value {
    serde_json::Value::Array(r.iter().map(|v| put_val(*v)).collect())
}

fn get_row(j: &serde_json::Value) -> Row {
    j.as_array().unwrap().iter().map(get_val).collect()
}

fn res_of(o: &Outcome) -> Res {
    Res { outputs: o.outputs.clone(), refusals: o.refusals.clone() }
}

fn put_res(r: &Res) -> serde_json::Value {
    json!({
        "o": r.outputs.iter().map(|(n, m)| json!([n, m.iter().map(|(row, c)| json!([put_row(row), c])).collect::<Vec<_>>()])).collect::<Vec<_>>(),
        "r": r.refusals.iter().map(put_row).collect::<Vec<_>>(),
    })
}

fn get_res(j: &serde_json::Value) -> Res {
    let outputs = j["o"].as_array().unwrap().iter().map(|o| (o[0].as_str().unwrap().to_owned(), o[1].as_array().unwrap().iter().map(|x| (get_row(&x[0]), x[1].as_i64().unwrap() as R)).collect())).collect();
    let mut refusals: Vec<Row> = j["r"].as_array().unwrap().iter().map(get_row).collect();
    refusals.sort();
    Res { outputs, refusals }
}

/// Runs `f` in a forked child and returns its bytes and the child's peak resident bytes (shared
/// pages it touched included). A child that panics or fails fails the run.
fn in_worker(f: impl FnOnce() -> Vec<u8>) -> (Vec<u8>, u64) {
    collect(spawn(f))
}

/// A worker in flight: its pid and the read end of its pipe.
struct Child(i32, i32);

/// Forks a worker running `f`. The child sees the parent's state as of the fork (copy on
/// write), so a worker is a pure function of that state and several can be in flight.
fn spawn(f: impl FnOnce() -> Vec<u8>) -> Child {
    unsafe {
        let mut fds = [0i32; 2];
        assert_eq!(libc::pipe(fds.as_mut_ptr()), 0, "pipe");
        let pid = libc::fork();
        assert!(pid >= 0, "fork");
        if pid == 0 {
            libc::close(fds[0]);
            let out = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
                Ok(o) => o,
                Err(_) => libc::_exit(3),
            };
            let mut off = 0;
            while off < out.len() {
                let n = libc::write(fds[1], out[off..].as_ptr() as *const libc::c_void, out.len() - off);
                if n <= 0 {
                    libc::_exit(2);
                }
                off += n as usize;
            }
            libc::_exit(0);
        }
        libc::close(fds[1]);
        Child(pid, fds[0])
    }
}

/// Reads a worker's bytes and reaps it: (bytes, peak resident bytes).
fn collect(c: Child) -> (Vec<u8>, u64) {
    unsafe {
        let mut buf = Vec::new();
        let mut chunk = vec![0u8; 1 << 16];
        loop {
            let n = libc::read(c.1, chunk.as_mut_ptr() as *mut libc::c_void, chunk.len());
            if n <= 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n as usize]);
        }
        libc::close(c.1);
        let mut status = 0;
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::wait4(c.0, &mut status, 0, &mut ru);
        assert!(libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0, "worker failed (status {status})");
        (buf, ru.ru_maxrss as u64)
    }
}

/// D1 (REWRITES.md, facts-b table): an event leaves a plan's result unchanged unless, at some
/// changed path, some export the plan reads has a different block before and after it (no
/// content or no rows: no block). Blocks are hash-consed on their rows (`Exports::intern`), so
/// equal blocks are equal row lists.
fn d1_affected(reads: &[u32], changed: &[(Sym, Option<Cid>, Option<Cid>)], block: &dyn Fn(u32, Sym, Cid) -> Option<u32>) -> bool {
    changed.iter().any(|(p, o, n)| {
        reads.iter().any(|e| {
            let (a, b) = (o.and_then(|c| block(*e, *p, c)), n.and_then(|c| block(*e, *p, c)));
            // Mutant 60: a side without rows counts as unchanged (an added or deleted file's rows
            // are missed).
            if crate::mutant::on(60) && (a.is_none() || b.is_none()) {
                return false;
            }
            a != b
        })
    })
}

/// One event's accounting while its plan runs are in flight.
struct EvAcc {
    seq: usize,
    group: String,
    kind: String,
    w: W,
    changes: usize,
    load_ms: f64,
    /// D1's decision plus every plan run's milliseconds.
    ms: f64,
    pending: usize,
    ran: Vec<usize>,
    /// X4: the frontier's milliseconds in this event's runs (from scratch; not in `ms`).
    maint_ms: f64,
}

struct Plan {
    name: String,
    graph: Rc<Graph>,
    reads: Vec<u32>,
    frontier: Vec<usize>,
}

/// `k` worktrees evenly spaced over `ws` (all when `k` is `None` or covers them).
fn spread(ws: &[W], k: Option<usize>) -> Vec<W> {
    match k {
        Some(k) if k < ws.len() => (0..k).map(|i| ws[i * ws.len() / k]).collect(),
        _ => ws.to_vec(),
    }
}

pub fn run(run_dir: &Path, opts: Opts) {
    std::fs::create_dir_all(run_dir).unwrap();
    let t_load = Instant::now();
    let sd = store::store_dir();
    let mdir = store::spike_dir().join("membership");
    let script_base_m = store::read_membership(&mdir.join("base.tsv"));
    let ms = crate::base::load_memberships();
    let mut st = Store { ids_only: true, ..Default::default() };
    for (_, h) in script_base_m.iter() {
        st.load(&sd, h);
    }
    for (_, m) in &ms {
        for (_, h) in m {
            st.load(&sd, h);
        }
    }
    let events: Vec<Event> = opts.events.as_ref().map(|p| read_events(p)).unwrap_or_default();
    let events: Vec<Event> = match opts.limit {
        Some(n) => events.into_iter().take(n).collect(),
        None => events,
    };
    let run_dir = run_dir.to_path_buf();
    let store_s = t_load.elapsed().as_secs_f64();

    timely::execute_directly(move |worker| {
        let world = Rc::new(RefCell::new(World::new(st)));
        val::install(world.clone());
        let rss_base = rss();
        let real = Rc::new(real::load(&opts.real));
        if !real.refused.is_empty() {
            eprintln!("refused at compile time: {}", serde_json::to_string(&real.refused).unwrap());
        }
        let plans: Vec<Plan> = real
            .graphs
            .iter()
            .map(|(n, g)| {
                let mut reads = BTreeSet::new();
                for op in &g.nodes {
                    if let Op::Exported(e) = op {
                        reads.insert(*e);
                    }
                }
                Plan { name: n.clone(), graph: g.clone(), reads: reads.into_iter().collect(), frontier: super::declared::frontier(g) }
            })
            .collect();
        let exports = Rc::new(RefCell::new(Exports { read: plans.iter().flat_map(|p| p.reads.iter().copied()).collect(), ..Default::default() }));
        let ex = exports.clone();
        let src: Source = Rc::new(move |e, p, c| ex.borrow().rows(e, p, c));
        let rss_plans = rss();
        let path_hash = |p: Sym, c: Cid| -> (String, String) {
            let w = world.borrow();
            (w.store.syms.str(p).to_owned(), crate::facts::hex(&w.store.contents[c as usize].hash))
        };
        let acquire = |p: Sym, c: Cid| {
            let (path, hash) = path_hash(p, c);
            exports.borrow_mut().acquire(&real, p, c, &path, &hash);
        };

        let to_map = |m: &store::Membership, wd: &mut World| -> Members { m.iter().map(|(p, hh)| (wd.path(p), wd.store.by_hash[hh])).collect() };
        let script_base_map = to_map(&script_base_m, &mut world.borrow_mut());
        let mut members: BTreeMap<W, Members> = BTreeMap::new();
        for (i, (_, m)) in ms.iter().enumerate().take(opts.worktrees.unwrap_or(usize::MAX)) {
            members.insert(i as W + 1, to_map(m, &mut world.borrow_mut()));
        }
        drop(ms);
        let rss_members = rss();

        // Per plan: from-scratch runs (load and recomputes) and their counts.
        let mut per_plan_ms: Vec<Vec<f64>> = vec![Vec::new(); plans.len()];
        let mut per_plan_counts: Vec<Counts> = vec![Counts::default(); plans.len()];
        let mut skipped: Vec<usize> = vec![0; plans.len()];
        // Per plan and stage: exclusive seconds and rows out, over every run.
        let stage_prof: RefCell<Vec<FxHashMap<String, (f64, u64)>>> = RefCell::new(vec![FxHashMap::default(); plans.len()]);
        // Runtime refusals raised in workers (counted per construct), and the workers' peak RSS.
        let worker_refusals: RefCell<BTreeMap<String, u64>> = RefCell::new(BTreeMap::new());
        let worker_peak: RefCell<(u64, Vec<f64>)> = RefCell::new((0, Vec::new()));
        // X4 (`--maintained`): per plan, the milliseconds and counts of the frontier's facts
        // (evaluated first, apart; a maintained table would hold them) per run, and the last run's.
        let maint_ms: RefCell<Vec<Vec<f64>>> = RefCell::new(vec![Vec::new(); plans.len()]);
        let maint_counts: RefCell<Vec<Counts>> = RefCell::new(vec![Counts::default(); plans.len()]);
        let last_maint = std::cell::Cell::new(0.0f64);
        // A plan run from scratch over `files`, in a worker: the child reports its result, its
        // counts, per-stage profile, the runtime refusals it raised and its compute time (with
        // `--maintained`, the rule logic's only; the frontier's apart).
        let eval_spawn = |k: usize, files: &[(Sym, Cid)]| -> Child {
            spawn(|| {
                let before = lower::refusals().0;
                let t = Instant::now();
                let first: &[usize] = if opts.maintained { &plans[k].frontier } else { &[] };
                let (o, pre) = batch::eval_with(&plans[k].name, &plans[k].graph, &src, files, first);
                let ms = (t.elapsed().as_secs_f64() - pre.secs) * 1e3;
                let inc: BTreeMap<String, u64> = lower::refusals().0.into_iter().filter_map(|(s, n)| (n > before.get(&s).copied().unwrap_or(0)).then(|| (s.clone(), n - before.get(&s).copied().unwrap_or(0)))).collect();
                serde_json::to_vec(&json!({"res": put_res(&res_of(&o)), "counts": o.counts, "stages": o.stages, "refusals": inc, "ms": ms, "maint_ms": pre.secs * 1e3, "maint_counts": pre.counts})).unwrap()
            })
        };
        // Its result in the parent, and the run's milliseconds (the child's compute plus the
        // parent's decoding).
        let eval_finish = |k: usize, c: Child, per_plan_ms: &mut Vec<Vec<f64>>, per_plan_counts: &mut Vec<Counts>| -> (Res, f64) {
            let (bytes, peak) = collect(c);
            let t = Instant::now();
            let j: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let c: Counts = serde_json::from_value(j["counts"].clone()).unwrap();
            per_plan_counts[k].add(&c);
            let mut sp = stage_prof.borrow_mut();
            for (s, v) in j["stages"].as_object().unwrap() {
                let e = sp[k].entry(s.clone()).or_default();
                e.0 += v[0].as_f64().unwrap();
                e.1 += v[1].as_u64().unwrap();
            }
            drop(sp);
            let mut wr = worker_refusals.borrow_mut();
            for (s, n) in j["refusals"].as_object().unwrap() {
                *wr.entry(s.clone()).or_default() += n.as_u64().unwrap();
            }
            drop(wr);
            let r = get_res(&j["res"]);
            let mut wp = worker_peak.borrow_mut();
            wp.0 = wp.0.max(peak);
            wp.1.push(mb(peak));
            let ms = j["ms"].as_f64().unwrap() + t.elapsed().as_secs_f64() * 1e3;
            per_plan_ms[k].push(ms);
            let m = j["maint_ms"].as_f64().unwrap();
            maint_ms.borrow_mut()[k].push(m);
            maint_counts.borrow_mut()[k].add(&serde_json::from_value(j["maint_counts"].clone()).unwrap());
            last_maint.set(m);
            (r, ms)
        };
        let files_of = |m: &Members| -> Vec<(Sym, Cid)> { m.iter().map(|(p, c)| (*p, *c)).collect() };

        // ---- load: every worktree's contents resident, every plan run from scratch (up to
        // `--workers` runs in flight).
        let max_workers = opts.workers.max(1);
        let t_build = Instant::now();
        let mut results: BTreeMap<W, Vec<Res>> = BTreeMap::new();
        let mut fact_load_s = 0.0;
        let mut wt_ms: BTreeMap<W, f64> = BTreeMap::new();
        let mut inflight: VecDeque<(W, usize, Child)> = VecDeque::new();
        let land = |inflight: &mut VecDeque<(W, usize, Child)>, results: &mut BTreeMap<W, Vec<Res>>, per_plan_ms: &mut Vec<Vec<f64>>, per_plan_counts: &mut Vec<Counts>, wt_ms: &mut BTreeMap<W, f64>| {
            let (w, k, c) = inflight.pop_front().unwrap();
            let (r, ms) = eval_finish(k, c, per_plan_ms, per_plan_counts);
            results.get_mut(&w).unwrap()[k] = r;
            *wt_ms.entry(w).or_default() += ms;
        };
        for (w, m) in &members {
            let tf = Instant::now();
            for (p, c) in m {
                acquire(*p, *c);
            }
            fact_load_s += tf.elapsed().as_secs_f64();
            let files = files_of(m);
            results.insert(*w, vec![Res { outputs: Vec::new(), refusals: Vec::new() }; plans.len()]);
            for k in 0..plans.len() {
                inflight.push_back((*w, k, eval_spawn(k, &files)));
                while inflight.len() >= max_workers {
                    land(&mut inflight, &mut results, &mut per_plan_ms, &mut per_plan_counts, &mut wt_ms);
                }
            }
            eprintln!("worktree {w} spawned, rss {:.0} MB", mb(rss().0));
        }
        while !inflight.is_empty() {
            land(&mut inflight, &mut results, &mut per_plan_ms, &mut per_plan_counts, &mut wt_ms);
        }
        let per_worktree_ms: Vec<serde_json::Value> = wt_ms.iter().map(|(w, ms)| json!([w, ms])).collect();
        let load_s = t_build.elapsed().as_secs_f64();
        let rss_loaded = rss();
        let attribution = |members: &BTreeMap<W, Members>, results: &BTreeMap<W, Vec<Res>>| {
            let (blocks, block_bytes, narrow, at) = exports.borrow().footprint();
            let (nsyms, sym_bytes) = val::syms_footprint();
            let member_entries: usize = members.values().map(|m| m.len()).sum();
            let res_rows: usize = results.values().flatten().map(Res::rows).sum();
            let res_bytes: usize = results.values().flatten().map(Res::bytes).sum();
            json!({
                "export_blocks": blocks, "export_block_bytes": block_bytes, "export_blocks_narrow": narrow, "export_rows": exports.borrow().rows_resident,
                "export_index_entries": at, "export_index_bytes_est": at * 32,
                "contents_held": exports.borrow().held.len(),
                "interned": nsyms, "interned_bytes": sym_bytes,
                "member_entries": member_entries, "member_bytes_est": member_entries * 40,
                "result_rows": res_rows, "result_bytes_est": res_bytes,
            })
        };
        let attr_loaded = attribution(&members, &results);
        eprintln!("loaded: {load_s:.1}s, rss {:.1} MB, {}", mb(rss_loaded.0), attr_loaded);

        // ---- checks: the stored results of w against a from-scratch run of the same plans.
        let mut checks = 0usize;
        let mut worktree_checks = 0usize;
        let mut engine_checks = 0usize;
        let mut differences: Vec<serde_json::Value> = Vec::new();
        let mut check_s = 0.0;
        let graphs: Vec<(String, Rc<Graph>)> = plans.iter().map(|p| (p.name.clone(), p.graph.clone())).collect();
        let mut check = |worker: &mut timely::worker::Worker, w: W, members: &BTreeMap<W, Members>, results: &BTreeMap<W, Vec<Res>>, at: usize, differences: &mut Vec<serde_json::Value>, engine_checks: &mut usize| {
            let t = Instant::now();
            let files = files_of(&members[&w]);
            // The whole check runs in a worker: its from-scratch relations never reach the parent.
            let (bytes, _) = in_worker(|| {
                let mut diffs: Vec<serde_json::Value> = Vec::new();
                for (k, p) in plans.iter().enumerate() {
                    let fresh = res_of(&batch::eval(&p.name, &p.graph, &src, &files));
                    let got = &results[&w][k];
                    if *got != fresh {
                        diffs.push(json!({"event": at, "worktree": w, "plan": p.name, "kind": "stored vs batch from scratch", "stored": got.rows(), "scratch": fresh.rows()}));
                    }
                }
                if opts.check == Check::Engine {
                    // The generator's dataflow over the members (base records only).
                    let probe = timely::dataflow::operators::probe::Handle::new();
                    let mut b = lower::build(worker, graphs.clone(), src.clone(), &probe, false);
                    for (p, c) in &files {
                        b.files.update((BASE, (*p, *c)), 1);
                    }
                    b.files.advance_to(1);
                    b.files.flush();
                    worker.step_while(|| probe.less_than(&1));
                    let outs: BTreeMap<String, BTreeMap<Row, R>> = b.outputs.iter().map(|(n, o)| (n.clone(), lower::full(o, BASE).unwrap_or_default())).collect();
                    let refs: BTreeMap<String, Vec<Row>> = b.refusals.iter().map(|(n, o)| (n.clone(), lower::full(o, BASE).unwrap_or_default().into_keys().collect())).collect();
                    for (k, p) in plans.iter().enumerate() {
                        let got = &results[&w][k];
                        let want_refs = refs.get(&p.name).cloned().unwrap_or_default();
                        let got_refs: Vec<String> = got.refusals.iter().map(show).collect();
                        let want_refs_s: Vec<String> = want_refs.iter().map(show).collect();
                        if got_refs != want_refs_s {
                            diffs.push(json!({"event": at, "worktree": w, "plan": p.name, "kind": "refusals vs engine", "batch": got_refs, "engine": want_refs_s}));
                            continue;
                        }
                        if !want_refs.is_empty() {
                            continue;
                        }
                        for (name, m) in &got.outputs {
                            let want = outs.get(name).cloned().unwrap_or_default();
                            if *m != want {
                                let extra: Vec<String> = m.iter().filter(|(r, c)| want.get(*r) != Some(c)).take(5).map(|(r, c)| format!("{} x{c}", show(r))).collect();
                                let missing: Vec<String> = want.iter().filter(|(r, c)| m.get(*r) != Some(c)).take(5).map(|(r, c)| format!("{} x{c}", show(r))).collect();
                                diffs.push(json!({"event": at, "worktree": w, "plan": name, "kind": "batch vs engine", "batch_only": extra, "engine_only": missing, "batch": m.len(), "engine": want.len()}));
                            }
                        }
                    }
                }
                serde_json::to_vec(&diffs).unwrap()
            });
            let diffs: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
            differences.extend(diffs);
            if opts.check == Check::Engine {
                *engine_checks += 1;
            }
            check_s += t.elapsed().as_secs_f64();
        };
        let sweep = |members: &BTreeMap<W, Members>| spread(&members.keys().copied().collect::<Vec<_>>(), opts.sweep);
        if opts.check != Check::None {
            for w in sweep(&members) {
                check(worker, w, &members, &results, 0, &mut differences, &mut engine_checks);
                worktree_checks += 1;
            }
            checks += 1;
            eprintln!("initial check: {} differences", differences.len());
        }

        // ---- replay
        let mut evs: Vec<EvAcc> = Vec::new();
        let mut inflight_ev: VecDeque<(usize, W, usize, Child)> = VecDeque::new();
        let land_ev = |inflight: &mut VecDeque<(usize, W, usize, Child)>, results: &mut BTreeMap<W, Vec<Res>>, per_plan_ms: &mut Vec<Vec<f64>>, per_plan_counts: &mut Vec<Counts>, evs: &mut Vec<EvAcc>| {
            let (i, w, k, c) = inflight.pop_front().unwrap();
            let (r, ms) = eval_finish(k, c, per_plan_ms, per_plan_counts);
            results.get_mut(&w).unwrap()[k] = r;
            evs[i].ms += ms;
            evs[i].maint_ms += last_maint.get();
            evs[i].pending -= 1;
        };
        let mut edits_done = 0usize;
        let mut rss_samples = vec![json!({"at": "loaded", "rss_mb": mb(rss_loaded.0), "footprint_mb": mb(rss_loaded.1)})];
        let mut stopped = false;
        let mut events_done = 0usize;
        let mut recomputes = 0usize;
        for ev in &events {
            if stopped {
                break;
            }
            let w = ev.worktree;
            let tl = Instant::now();
            // (path, old content, new content) of the event in w.
            let mut changed: Vec<(Sym, Option<Cid>, Option<Cid>)> = Vec::new();
            let new_worktree = ev.group == "new_worktree" || ev.group == "commit_init";
            if new_worktree {
                assert!(!members.contains_key(&w), "worktree {w} exists");
                for (p, c) in &script_base_map {
                    acquire(*p, *c);
                }
                members.insert(w, script_base_map.clone());
            }
            {
                let mut wd = world.borrow_mut();
                let m = members.get_mut(&w).unwrap();
                for (path, old, new) in &ev.changes {
                    let p = wd.path(path);
                    let oldc = old.as_ref().map(|x| wd.store.load(&sd, &store::parse_hex(x)));
                    let newc = new.as_ref().map(|x| wd.store.load(&sd, &store::parse_hex(x)));
                    assert_eq!(m.get(&p).copied(), oldc, "event {} {path}: old content mismatch", ev.seq);
                    match newc {
                        Some(c) => m.insert(p, c),
                        None => m.remove(&p),
                    };
                    changed.push((p, oldc, newc));
                }
            }
            // The new contents' exports are made resident before the clock (as greplay does).
            for (p, _, n) in &changed {
                if let Some(c) = n {
                    acquire(*p, *c);
                }
            }
            let load_ms = tl.elapsed().as_secs_f64() * 1e3;
            let t0 = Instant::now();
            let files = files_of(&members[&w]);
            if new_worktree {
                results.insert(w, vec![Res { outputs: Vec::new(), refusals: Vec::new() }; plans.len()]);
            }
            let i = evs.len();
            let mut ran = Vec::new();
            for k in 0..plans.len() {
                // D1: unchanged blocks at every changed path leave the plan's result unchanged.
                let affected = new_worktree || !opts.skip || d1_affected(&plans[k].reads, &changed, &|e, p, c| exports.borrow().block(e, p, c));
                if !affected {
                    skipped[k] += 1;
                    continue;
                }
                ran.push(k);
            }
            let d1_ms = t0.elapsed().as_secs_f64() * 1e3;
            for &k in &ran {
                inflight_ev.push_back((i, w, k, eval_spawn(k, &files)));
                recomputes += 1;
            }
            evs.push(EvAcc { seq: ev.seq, group: ev.group.clone(), kind: ev.kind.clone(), w, changes: ev.changes.len(), load_ms, ms: d1_ms, pending: ran.len(), ran, maint_ms: 0.0 });
            while inflight_ev.len() >= max_workers {
                land_ev(&mut inflight_ev, &mut results, &mut per_plan_ms, &mut per_plan_counts, &mut evs);
            }
            // Old contents no longer held are released after the event (workers in flight hold
            // their own copy of the state they started from).
            for (p, o, _) in &changed {
                if let Some(c) = o {
                    exports.borrow_mut().release(*p, *c);
                }
            }
            if ev.group == "edit" {
                edits_done += 1;
            }
            events_done += 1;
            let sample = ev.group == "edit" && edits_done % 100 == 0;
            if opts.check != Check::None {
                let due = events_done % opts.check_every == 0;
                if due || sample {
                    while !inflight_ev.is_empty() {
                        land_ev(&mut inflight_ev, &mut results, &mut per_plan_ms, &mut per_plan_counts, &mut evs);
                    }
                }
                let before = differences.len();
                if due {
                    check(worker, w, &members, &results, ev.seq, &mut differences, &mut engine_checks);
                    worktree_checks += 1;
                    checks += 1;
                }
                if sample {
                    for ww in sweep(&members) {
                        check(worker, ww, &members, &results, ev.seq, &mut differences, &mut engine_checks);
                        worktree_checks += 1;
                    }
                    eprintln!("edit {edits_done}: sweep, {} differences so far", differences.len());
                }
                if differences.len() > before && differences.len() > 50 {
                    stopped = true;
                }
            }
            if sample {
                while !inflight_ev.is_empty() {
                    land_ev(&mut inflight_ev, &mut results, &mut per_plan_ms, &mut per_plan_counts, &mut evs);
                }
                let r = rss();
                rss_samples.push(json!({"at": format!("edit {edits_done}"), "rss_mb": mb(r.0), "footprint_mb": mb(r.1), "attribution": attribution(&members, &results)}));
                eprintln!("edit {edits_done}: rss {:.0} MB", mb(r.0));
            }
        }
        while !inflight_ev.is_empty() {
            land_ev(&mut inflight_ev, &mut results, &mut per_plan_ms, &mut per_plan_counts, &mut evs);
        }
        let mut lat: BTreeMap<String, Vec<f64>> = BTreeMap::new();
        let mut per_event = Vec::new();
        for e in &evs {
            assert_eq!(e.pending, 0);
            lat.entry(e.group.clone()).or_default().push(e.ms);
            if opts.maintained {
                lat.entry(format!("{}+frontier_from_scratch", e.group)).or_default().push(e.ms + e.maint_ms);
            }
            if e.group == "edit" {
                lat.entry(format!("edit:{}", e.kind)).or_default().push(e.ms);
            }
            per_event.push(json!([e.seq, e.group, e.kind, e.w, e.changes, e.ms, e.load_ms, e.ran, e.maint_ms]));
        }
        let rss_end = rss();
        rss_samples.push(json!({"at": "end", "rss_mb": mb(rss_end.0), "footprint_mb": mb(rss_end.1)}));
        let mut lat_summary = serde_json::Map::new();
        for (k, v) in &lat {
            lat_summary.insert(k.clone(), summary(v));
        }
        let single: Vec<f64> = ["edit", "new_worktree_edit"].iter().flat_map(|k| lat.get(*k).cloned().unwrap_or_default()).collect();
        lat_summary.insert("single_file_all".into(), summary(&single));
        if opts.maintained {
            let s2: Vec<f64> = ["edit+frontier_from_scratch", "new_worktree_edit+frontier_from_scratch"].iter().flat_map(|k| lat.get(*k).cloned().unwrap_or_default()).collect();
            lat_summary.insert("single_file_all+frontier_from_scratch".into(), summary(&s2));
        }
        drop(check);
        let lower::Refusals(mut runtime_refusals) = lower::refusals();
        for (s, n) in worker_refusals.borrow().iter() {
            *runtime_refusals.entry(s.clone()).or_default() += n;
        }
        let (peak_max, peaks) = worker_peak.borrow().clone();
        let live: Vec<serde_json::Value> = results
            .iter()
            .flat_map(|(w, rs)| rs.iter().enumerate().filter(|(_, r)| !r.refusals.is_empty()).map(move |(k, r)| json!({"worktree": w, "plan": k, "refusals": r.refusals.iter().map(show).collect::<Vec<_>>()})))
            .collect();
        let out = json!({
            "model": if opts.maintained { "X4 declared facts (frontier apart)" } else { "M2 hybrid" },
            "skip": opts.skip,
            "plans": plans.iter().enumerate().map(|(k, p)| {
                let mut top: Vec<(String, (f64, u64))> = stage_prof.borrow()[k].clone().into_iter().collect();
                top.sort_by(|a, b| b.1 .0.partial_cmp(&a.1 .0).unwrap());
                json!({"plan": p.name, "nodes": p.graph.nodes.len(), "exports_read": p.reads.len(), "runs": per_plan_ms[k].len(), "skipped": skipped[k], "ms": summary(&per_plan_ms[k]), "counts": per_plan_counts[k],
                       "frontier": p.frontier.iter().map(|x| p.graph.labels[*x].clone()).collect::<Vec<_>>(), "frontier_ms": summary(&maint_ms.borrow()[k]), "frontier_counts": maint_counts.borrow()[k],
                       "top_stages_secs_rows": top.iter().map(|(s, (t, r))| json!([s, t, r])).collect::<Vec<_>>()})
            }).collect::<Vec<_>>(),
            "refused_static": real.refused,
            "events": events.len(),
            "recomputes": recomputes,
            "load": {"store_seconds": store_s, "seconds": load_s, "fact_load_seconds": fact_load_s, "per_worktree_ms": per_worktree_ms,
                     "rss_base_mb": mb(rss_base.0), "rss_plans_mb": mb(rss_plans.0), "rss_members_mb": mb(rss_members.0), "rss_loaded_mb": mb(rss_loaded.0), "footprint_loaded_mb": mb(rss_loaded.1)},
            "attribution_loaded": attr_loaded,
            "attribution_end": attribution(&members, &results),
            "latency": lat_summary,
            "rss": rss_samples,
            "runtime_refusals": runtime_refusals,
            "workers": {"peak_rss_mb_max": mb(peak_max), "peak_rss_mb": summary(&peaks)},
            "runtime_refusals_live": live,
            "exactness": {"check": match opts.check { Check::None => "none", Check::Batch => "batch", Check::Engine => "engine" }, "checks": checks, "worktree_checks": worktree_checks, "engine_checks": engine_checks, "differences": differences.len(), "first_differences": differences.iter().take(20).collect::<Vec<_>>(), "check_seconds": check_s, "check_every": opts.check_every, "sweep": opts.sweep, "stopped_early": stopped},
        });
        std::fs::write(run_dir.join("results.json"), serde_json::to_string_pretty(&out).unwrap()).unwrap();
        std::fs::write(run_dir.join("events.json"), serde_json::to_string(&per_event).unwrap()).unwrap();
        println!("{}", serde_json::to_string_pretty(&json!({"latency": out["latency"]["single_file_all"], "rss": out["rss"], "exactness": out["exactness"], "load": out["load"]["seconds"], "attribution_loaded": out["attribution_loaded"], "plans": out["plans"]})).unwrap());
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(xs: &[(i64, i64)]) -> Vec<Row> {
        xs.iter().map(|(a, b)| Row::from([Val::int(*a), Val::int(*b)])).collect()
    }

    /// D1's side condition holds on the block store: equal rows are one block, different rows
    /// (values, order, count, width, or a word past 32 bits) different blocks, and a freed block's
    /// id is reused only for a new block.
    #[test]
    fn d1_blocks_are_hash_consed_on_their_rows() {
        let mut ex = Exports::default();
        let a = ex.intern(Block::new(&rows(&[(1, 2), (3, 4)])));
        let b = ex.intern(Block::new(&rows(&[(1, 2), (3, 4)])));
        assert_eq!(a, b);
        let c = ex.intern(Block::new(&rows(&[(3, 4), (1, 2)])));
        let d = ex.intern(Block::new(&rows(&[(1, 2)])));
        let e = ex.intern(Block::new(&rows(&[(1, 2), (3, 1 << 40)])));
        let f = ex.intern(Block::new(&[Row::from([Val::int(1)]), Row::from([Val::int(2)])]));
        let ids = [a, c, d, e, f];
        for i in 0..ids.len() {
            for j in 0..i {
                assert_ne!(ids[i], ids[j], "blocks {i} and {j}");
            }
        }
        assert!(matches!(ex.blocks[e as usize].as_ref().unwrap().words, Words::Wide(_)));
        assert_eq!(ex.blocks[e as usize].as_ref().unwrap().rows(), rows(&[(1, 2), (3, 1 << 40)]));
        ex.unref(a);
        assert!(ex.blocks[a as usize].is_some(), "still held once");
        ex.unref(a);
        assert!(ex.blocks[a as usize].is_none());
        let g = ex.intern(Block::new(&rows(&[(9, 9)])));
        assert_eq!(g, a, "freed id reused");
        assert_eq!(ex.intern(Block::new(&rows(&[(1, 2), (3, 4)]))), ex.intern(Block::new(&rows(&[(1, 2), (3, 4)]))));
    }

    /// D1 recomputes exactly when a read export's rows differ at a changed path: an edit that
    /// changes them, an added or a deleted file with rows; and skips an edit whose rows are equal,
    /// a file without rows, or a change only in exports the plan does not read.
    #[test]
    fn d1_affected_iff_a_read_block_differs() {
        // (export, path, content) -> block
        let at: BTreeMap<(u32, Sym, Cid), u32> = [((1, 10, 100), 7), ((1, 10, 101), 7), ((1, 10, 102), 8), ((2, 10, 103), 9), ((1, 11, 200), 5)].into_iter().collect();
        let block = |e: u32, p: Sym, c: Cid| at.get(&(e, p, c)).copied();
        let reads = [1u32];
        // Same rows under a new content (a whitespace edit).
        assert!(!d1_affected(&reads, &[(10, Some(100), Some(101))], &block));
        // Rows changed.
        assert!(d1_affected(&reads, &[(10, Some(100), Some(102))], &block));
        // Only an export the plan does not read changed.
        assert!(!d1_affected(&reads, &[(10, Some(104), Some(103))], &block));
        assert!(d1_affected(&[2], &[(10, Some(104), Some(103))], &block));
        // Added and deleted files with rows; a file without rows either way.
        assert!(d1_affected(&reads, &[(11, None, Some(200))], &block));
        assert!(d1_affected(&reads, &[(11, Some(200), None)], &block));
        assert!(!d1_affected(&reads, &[(12, None, Some(300))], &block));
        // Several changes: any one suffices.
        assert!(d1_affected(&reads, &[(10, Some(100), Some(101)), (11, Some(200), None)], &block));
    }
}
