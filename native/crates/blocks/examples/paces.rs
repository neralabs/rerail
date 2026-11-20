//! Runs decision 0266's blocks through an engine-shaped lifecycle and checks every step against an oracle.
//!
//! The workload has the engine's shape: documents with tapes of a few thousand nodes, and three program parts.
//!
//! - **Chunk (file-local):** `node` from the tape; then `assert_at`, `call_in_assert` (a probe of `node`),
//!   `defines`, `imports` and `has_assert`. Each document's kept rows become a memo entry in the journal.
//! - **P1 (cross-file):**
//!   - S0: `defines`, `imports`, `has_assert`, pushed from every chunk at once;
//!   - S1: `edge(d1, d2) :- imports(d1, a), defines(d2, a)`, a join run on the GPU;
//!   - S2: `reach`, the transitive closure, a fixpoint.
//! - **P2:** `uses_assert(d1) :- reach(d1, d2), has_assert(d2)`.
//! - **Outputs:** `b101`, `call_b101`, `cross_b101`.
//!
//! The runs:
//!
//! 1. **A cold run.** Chunks are evaluated K at a time, and P1, P2 and the outputs are written as full blocks.
//! 2. **A memo-warm cold run.** Every document hits the memo: a hit is a range copied from a journal file
//!    reopened (and checked) from disk.
//! 3. **Maintained runs, one edit each.** Each edit makes a delta of every block, carried forward, cumulative.
//!    The roll-up rule decides when to roll a delta up.
//!
//! Every run is checked against a from-scratch run on the same documents: every relation's digest over its
//! live rows in canonical order must agree. Then the program block's delta and roll-up, the journal's
//! refusals, and the costs are checked.
//!
//! Usage: `paces [docs] [runs] [dir]`.

#[cfg(target_os = "macos")]
use rerail_blocks::block::Block;
use rerail_blocks::block::kind;
use rerail_blocks::chunk::{self, DocIn};
use rerail_blocks::facts::{Facts, FrozenFacts, RelDef, Schema};
use rerail_blocks::journal::{self, Journal};
use rerail_blocks::program::{ModuleIn, Program};
use rerail_blocks::relation::{Spec, View};
use rerail_blocks::{col, packed};
use rerail_blocks::{footprint, retired};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering::*};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const DEF: u16 = 1;
const IMPORT: u16 = 2;
const ASSERT: u16 = 3;
const CALL: u16 = 4;
const CHUNK_DOCS: usize = 16;
const AT_ONCE: usize = 4;
const PACKAGE: u32 = 20;

/// Every Facts, chunk and journal block's reservation: `PACES_RESERVE_MB`, default 2048.
fn reserve() -> usize {
    std::env::var("PACES_RESERVE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(2048usize) << 20
}

// ---- the workload ----

fn rng(mut x: u64) -> impl FnMut() -> u64 {
    move || {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

struct Src {
    key: u32,
    kind: Vec<u16>,
    parent: Vec<u32>,
    name: Vec<u32>,
    digest: u64,
}

/// Document `d` at edit `ver`: 1,500 to 3,900 nodes; 2% definitions and 2% imports of names in its package,
/// 0.5% asserts, 8% calls.
fn make_doc(d: u32, ver: u32) -> Src {
    let mut r = rng(((d as u64) << 20) ^ ver as u64);
    let n = 1500 + (r() % 2400) as usize;
    let (mut kind, mut parent, mut name) = (Vec::with_capacity(n), Vec::with_capacity(n), Vec::with_capacity(n));
    for i in 0..n {
        let x = r() % 1000;
        let k = match x {
            0..20 => DEF,
            20..40 => IMPORT,
            40..45 => ASSERT,
            45..125 => CALL,
            _ => 10 + (x % 90) as u16,
        };
        kind.push(k);
        parent.push(if i == 0 { 0 } else { (r() % i as u64) as u32 });
        name.push(match k {
            DEF => d * 64 + (r() % 64) as u32,
            IMPORT => (d / PACKAGE * PACKAGE + (r() % PACKAGE as u64) as u32) * 64 + (r() % 64) as u32,
            _ => 0,
        });
    }
    let mut h = rerail_blocks::SEED;
    for i in 0..n {
        h = rerail_blocks::mix(h, ((kind[i] as i64) << 40) ^ ((parent[i] as i64) << 8) ^ name[i] as i64);
    }
    Src { key: d + 1, kind, parent, name, digest: h }
}

/// A node's atom: its document's key and its place in it, in 4 bytes (a document has under 4,096 nodes, a run under
/// 2^20 documents).
fn nid(doc: u32, i: u32) -> i64 {
    debug_assert!(i < 1 << 12 && doc < 1 << 20);
    ((doc as i64) << 12) | i as i64
}

// ---- schemas ----

fn spec(widths: &[u32], indexes: &[&[u16]], min_log: u32) -> Spec {
    Spec { widths: widths.to_vec(), order: (0..widths.len() as u16).collect(), indexes: indexes.iter().map(|k| k.to_vec()).collect(), min_log }
}

fn schema(rels: &[(&str, usize, Spec)]) -> Arc<Schema> {
    let nstrata = rels.iter().map(|r| r.1).max().unwrap() + 1;
    Arc::new(Schema { rels: rels.iter().map(|(n, s, sp)| RelDef { name: n.to_string(), stratum: *s, spec: sp.clone() }).collect(), nstrata })
}

// Chunk relations.
const NODE: usize = 0;
const ASSERT_AT: usize = 1;
const CALL_IN: usize = 2;
const DEFINES: usize = 3;
const IMPORTS: usize = 4;
const HAS_ASSERT: usize = 5;
const KEPT: [usize; 5] = [ASSERT_AT, CALL_IN, DEFINES, IMPORTS, HAS_ASSERT];
// P1, P2 and output relations.
const P_DEF: usize = 0;
const P_IMP: usize = 1;
const P_HAS: usize = 2;
const P_EDGE: usize = 3;
const P_REACH: usize = 4;
const USES: usize = 0;
const B101: usize = 0;
const CALL_B101: usize = 1;
const CROSS: usize = 2;

struct Schemas {
    chunk: Arc<Schema>,
    entry: Vec<Spec>,
    p1: Arc<Schema>,
    p2: Arc<Schema>,
    out: Arc<Schema>,
    edge_check: Arc<Schema>,
}

fn schemas() -> Schemas {
    let pair = |ml| spec(&[4, 4], &[&[0], &[1]], ml);
    let doc_node = |ml| spec(&[4, 4], &[&[0]], ml);
    let chunk = schema(&[
        ("node", 0, spec(&[4, 4, 4, 4], &[&[1]], 12)),
        ("assert_at", 1, doc_node(8)),
        ("call_in_assert", 1, doc_node(8)),
        ("defines", 1, pair(8)),
        ("imports", 1, pair(8)),
        ("has_assert", 1, spec(&[4], &[&[0]], 6)),
    ]);
    let entry = KEPT.iter().map(|&r| Spec { min_log: 4, ..chunk.rels[r].spec.clone() }).collect();
    let p1 = schema(&[("defines", 0, pair(10)), ("imports", 0, pair(10)), ("has_assert", 0, spec(&[4], &[&[0]], 8)), ("edge", 1, pair(10)), ("reach", 2, pair(10))]);
    let p2 = schema(&[("uses_assert", 0, spec(&[4], &[&[0]], 8))]);
    let out = schema(&[("b101", 0, doc_node(10)), ("call_b101", 0, doc_node(8)), ("cross_b101", 1, spec(&[4], &[&[0]], 8))]);
    let edge_check = schema(&[("edge", 0, pair(10))]);
    Schemas { chunk, entry, p1, p2, out, edge_check }
}

// ---- measurement ----

#[derive(Default)]
struct Meter {
    rows: BTreeMap<String, (u64, u64, u64)>,
}

// ---- the memo store: journals reopened from disk ----

struct Store {
    journals: Vec<rerail_blocks::Frozen>,
    index: HashMap<(u32, u64), (usize, usize, usize, usize)>,
}

impl Store {
    fn open(paths: &[PathBuf]) -> Store {
        let mut s = Store { journals: Vec::new(), index: HashMap::new() };
        for p in paths {
            let j = journal::open(p).unwrap_or_else(|e| panic!("journal refused: {e}"));
            let n = s.journals.len();
            for e in journal::entries(&j) {
                s.index.insert((e.key, e.digest), (n, e.start as usize, e.end as usize, e.entry as usize));
            }
            s.journals.push(j);
        }
        s
    }
}

// ---- reading helpers ----

fn flat(v: View, ids: &[u32]) -> Vec<i64> {
    let mut out = Vec::with_capacity(ids.len() * v.arity());
    for &id in ids {
        for c in 0..v.arity() {
            out.push(v.get(id, c));
        }
    }
    out
}

// ---- the chunk job ----

fn eval_chunk(sc: &Schemas, srcs: &[&Src], store: Option<&Store>, journal: Option<&Journal>) -> FrozenFacts {
    let mut hits = Vec::new();
    let mut misses = Vec::new();
    for s in srcs {
        match store.and_then(|st| st.index.get(&(s.key, s.digest))) {
            Some(&h) => hits.push(h),
            None => misses.push(*s),
        }
    }
    let ins: Vec<DocIn> = misses.iter().map(|s| DocIn { key: s.key, kind: &s.kind, parent: &s.parent, name: &s.name }).collect();
    let f = chunk::open(sc.chunk.clone(), reserve(), &ins, hits.len());
    if let Some(st) = store {
        for (j, start, end, entry) in hits {
            chunk::add_hit(&f, &st.journals[j], start, end, entry);
        }
    }
    f.begin(0);
    f.create(NODE);
    {
        let w = f.w(NODE);
        for d in chunk::docs(&f.block) {
            let (k, p) = (d.kind.as_slice(), d.parent.as_slice());
            let mut rows = Vec::with_capacity(d.nodes as usize * 4);
            for i in 0..d.nodes {
                rows.extend_from_slice(&[d.key as i64, nid(d.key, i), k[i as usize] as i64, nid(d.key, p[i as usize])]);
            }
            w.extend(&rows);
        }
    }
    f.freeze_rel(NODE);
    f.end(0);
    f.begin(1);
    for r in KEPT {
        f.create(r);
    }
    {
        let node = f.view(NODE);
        let (wa, wc, wd, wi, wh) = (f.w(ASSERT_AT), f.w(CALL_IN), f.w(DEFINES), f.w(IMPORTS), f.w(HAS_ASSERT));
        for d in chunk::docs(&f.block) {
            let (k, p, m) = (d.kind.as_slice(), d.parent.as_slice(), d.name.as_slice());
            let dk = d.key as i64;
            let mut any = false;
            for i in 0..d.nodes as usize {
                match k[i] {
                    ASSERT => {
                        wa.push(&[dk, nid(d.key, i as u32)]);
                        any = true;
                    }
                    DEF => _ = wd.push(&[dk, m[i] as i64]),
                    IMPORT => _ = wi.push(&[dk, m[i] as i64]),
                    CALL => {
                        // A probe of `node` by id: the parent's kind.
                        let par = node.probe(0, &[nid(d.key, p[i])]);
                        if par.first().is_some_and(|&id| node.get(id, 2) == ASSERT as i64) {
                            wc.push(&[dk, nid(d.key, i as u32)]);
                        }
                    }
                    _ => {}
                }
            }
            if any {
                wh.push(&[dk]);
            }
        }
    }
    for r in KEPT {
        f.freeze_rel(r);
    }
    f.end(1);
    if let Some(j) = journal {
        for s in &misses {
            let rels: Vec<(&Spec, Vec<i64>)> = KEPT
                .iter()
                .enumerate()
                .map(|(e, &r)| {
                    let v = f.view(r);
                    (&sc.entry[e], flat(v, &v.probe(0, &[s.key as i64])))
                })
                .collect();
            let (scratch, start, end, off) = chunk::build_entry(s.key, s.digest, &rels);
            j.add(&scratch, start, end, off, s.key, s.digest);
        }
    }
    f.freeze()
}

fn eval_chunks(sc: &Schemas, srcs: &[&Src], store: Option<&Store>, journal: Option<&Journal>) -> Vec<FrozenFacts> {
    let groups: Vec<&[&Src]> = srcs.chunks(CHUNK_DOCS).collect();
    let next = AtomicUsize::new(0);
    let out = Mutex::new((0..groups.len()).map(|_| None).collect::<Vec<Option<FrozenFacts>>>());
    std::thread::scope(|t| {
        for _ in 0..AT_ONCE {
            t.spawn(|| {
                loop {
                    let g = next.fetch_add(1, Relaxed);
                    if g >= groups.len() {
                        break;
                    }
                    let c = eval_chunk(sc, groups[g], store, journal);
                    out.lock().unwrap()[g] = Some(c);
                }
            });
        }
    });
    out.into_inner().unwrap().into_iter().map(|c| c.unwrap()).collect()
}

/// Chunk relation `r`'s rows, from its evaluated documents and from its hits.
fn chunk_rows(c: &FrozenFacts, r: usize) -> Vec<i64> {
    let v = c.view(r);
    let mut rows = flat(v, &v.ordered());
    let e = KEPT.iter().position(|&k| k == r).unwrap();
    for h in chunk::hits(&c.block) {
        let hv = h.rel(e);
        rows.extend(flat(hv, &hv.ordered()));
    }
    rows
}

// ---- the cross parts ----

/// `reach`'s closure into relation `r` of `f`, from `edge` (indexed on its first column): semi-naive.
fn closure(f: &Facts, r: usize, edge: View) {
    let w = f.w(r);
    let mut frontier = Vec::new();
    for id in edge.ordered() {
        let (a, b) = (edge.get(id, 0), edge.get(id, 1));
        if w.insert_unique(&[a, b]) {
            frontier.push((a, b));
        }
    }
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for (a, b) in frontier {
            for id in edge.probe(0, &[b]) {
                let c = edge.get(id, 1);
                if w.insert_unique(&[a, c]) {
                    next.push((a, c));
                }
            }
        }
        frontier = next;
    }
}

/// `edge` on the host, for checking: imports(d1, a), defines(d2, a), d1 != d2.
fn join_cpu(imp: View, def: View) -> Vec<i64> {
    let mut out = Vec::new();
    for id in imp.ordered() {
        let (d1, a) = (imp.get(id, 0), imp.get(id, 1));
        for j in def.probe(1, &[a]) {
            let d2 = def.get(j, 0);
            if d1 != d2 {
                out.extend_from_slice(&[d1, d2]);
            }
        }
    }
    out
}

/// Turns relation `r` of the open delta `f` (carried, so its ids are the old view's) into `new`: kills rows the
/// new set lacks, pushes rows it adds. Both walks are in canonical order, so this is one merge.
fn diff_into(f: &Facts, r: usize, old: View, new: View) -> (usize, usize) {
    let w = f.w(r);
    let (o, n) = (old.ordered(), new.ordered());
    let (mut i, mut j, mut kills, mut adds) = (0, 0, 0, 0);
    while i < o.len() || j < n.len() {
        let ord = match (o.get(i), n.get(j)) {
            (Some(&x), Some(&y)) => old.row(x).cmp(&new.row(y)),
            (Some(_), None) => std::cmp::Ordering::Less,
            _ => std::cmp::Ordering::Greater,
        };
        match ord {
            std::cmp::Ordering::Less => {
                w.kill(o[i]);
                kills += 1;
                i += 1;
            }
            std::cmp::Ordering::Greater => {
                w.push(&new.row(n[j]));
                adds += 1;
                j += 1;
            }
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    (kills, adds)
}

struct Kept {
    p1: FrozenFacts,
    p2: FrozenFacts,
    out: FrozenFacts,
}

struct Ctx<'a> {
    sc: &'a Schemas,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    gpu: Option<&'a Gpu>,
    m: Mutex<Meter>,
    checks: Mutex<Vec<String>>,
}

impl Ctx<'_> {
    fn time<R>(&self, name: &str, f: impl FnOnce() -> R) -> R {
        let (i, t) = (retired(), Instant::now());
        let r = f();
        let mut m = self.m.lock().unwrap();
        let e = m.rows.entry(name.to_string()).or_default();
        e.0 += retired() - i;
        e.1 += t.elapsed().as_micros() as u64;
        e.2 += 1;
        r
    }

    fn check(&self, ok: bool, what: String) {
        assert!(ok, "FAILED: {what}");
        self.checks.lock().unwrap().push(what);
    }
}

/// The join kernels (`rerail_blocks::gpu`) on the process's device, whatever its backend (decision 0440 step C).
#[cfg(any(target_os = "macos", target_os = "linux"))]
struct Gpu {
    shared: &'static rerail_gpu::shared::Shared,
    count: &'static rerail_gpu::device::ComputeFunction,
    write: &'static rerail_gpu::device::ComputeFunction,
}

/// What one join did on the device: how often its write pass ran again at the exact size.
#[derive(Debug, Default, Clone, Copy)]
struct JoinRun {
    reruns: u32,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Gpu {
    /// The kernels on the process's device. A kernel that does not compile is fatal (decision 0382).
    fn new() -> Result<Gpu, String> {
        let shared = rerail_gpu::shared::get().ok_or("no device")?;
        let program = rerail_gpu::kernels::program("join.uglykl", Box::leak(rerail_blocks::gpu::source().into_boxed_str()))
            .unwrap_or_else(|e| panic!("the join kernels do not compile (decision 0382: fatal): {e}"));
        Ok(Gpu { shared, count: program.function("join_count")?, write: program.function("join_write")? })
    }

    /// The block's device buffer: kept by the block, so a dispatch never pays a fresh buffer's mapping. It covers
    /// what the block has used (rounded to 1 MiB), not its whole reservation: a command buffer's cost grows with the
    /// length of the buffers it makes resident (measured: 2 GiB wrapped cost 120 ms a join, 256 MiB 17 ms).
    fn wrap(&self, b: &Block) -> rerail_device::device::Buffer {
        b.device_buffer(&self.shared.device).expect("a block's device buffer")
    }

    /// A fresh no-copy wrap of `len` bytes of the block, not kept: for measuring what keeping saves.
    fn wrap_len(&self, b: &Block, len: usize) -> rerail_device::device::Buffer {
        let len = (len.div_ceil(1 << 20).max(1) << 20).min(b.reserved());
        // SAFETY: the block's pages outlive the wrap, which the caller drops before the block.
        unsafe { rerail_device::device::wrap_host_memory(&self.shared.device, b.base_ptr(), len) }.expect("a wrap of the block")
    }

    fn dispatch(&self, f: &rerail_gpu::device::ComputeFunction, args: &JoinArgsPod, b: [&rerail_device::device::Buffer; 3], counts: &rerail_device::device::Buffer, write: bool) {
        let mut q = rerail_gpu::device::ComputeQueue::new(&self.shared.queue);
        let l = rerail_gpu::kernels::launch(&q, f).value("a", args).buffer("b1", b[0]).buffer("b2", b[1]).buffer("counts", counts);
        let l = if write { l.buffer("b3", b[2]) } else { l };
        l.run(&[u64::from(args.0.n.max(1)).div_ceil(256)], &[256]).unwrap_or_else(|e| panic!("{e}"));
        q.flush().await_().unwrap();
    }

    /// Runs the join: `left` and `right` read from `base` (if they have one) and `top`, and the rows are written
    /// into `out`, an open relation of `out_block`. `short` reserves nothing for them, to take the overflow path:
    /// the write pass is then run again at the exact size.
    fn join(&self, base: Option<&Block>, top: &Block, out_block: &Block, out: &rerail_blocks::relation::Relation, args: &rerail_blocks::gpu::JoinArgs, short: bool) -> JoinRun {
        use std::sync::atomic::Ordering::*;
        let args = &JoinArgsPod::of(args);
        let bt = self.wrap(top);
        let bb = base.map(|b| self.wrap(b));
        let b1 = bb.as_ref().unwrap_or(&bt);
        let counts = rerail_gpu::kernels::alloc::<u32>(args.0.n as usize).unwrap();
        self.dispatch(self.count, args, [b1, &bt, &bt], &counts, false);
        let c: Vec<u32> = rerail_gpu::kernels::read(&counts, args.0.n as usize);
        let total: u64 = c.iter().map(|&x| x as u64).sum();
        let len0 = out.rows.len();
        out.rows.reserve(out_block, len0 as u64 + if short { 0 } else { total });
        let mut run = JoinRun::default();
        loop {
            // The out block is wrapped after its chunks are reserved: its used extent covers them.
            let bo = self.wrap(out_block);
            let bt = if std::ptr::eq(out_block.base_ptr(), top.base_ptr()) { self.wrap(out_block) } else { bt.clone() };
            self.dispatch(self.write, args, [b1, &bt, &bo], &counts, true);
            let needed = out.rows.needed.load(Acquire);
            if needed == 0 {
                break;
            }
            // Overflow: nothing past `reserved` was written. Reset the claims, reserve the exact size, rerun.
            out.rows.needed.store(0, Release);
            out.rows.len.store(len0, Release);
            out.rows.reserve(out_block, len0 as u64 + total);
            run.reruns += 1;
        }
        run
    }
}

/// A join's arguments as the launch passes them: their bytes (`JoinArgs` is a block's structure, never `Copy`).
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[repr(C)]
#[derive(Clone, Copy)]
struct JoinArgsPod(JoinArgsBytes);

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[repr(C)]
#[derive(Clone, Copy)]
struct JoinArgsBytes {
    lbase: u64,
    ltop: u64,
    rbase: u64,
    rtop: u64,
    out: u64,
    n: u32,
    idx: u32,
    lkey: u32,
    lout: u32,
    rkey: u32,
    rout: u32,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
// SAFETY: `#[repr(C)]` u64s and u32s, no padding (as `JoinArgs`, whose `structure!` checks it).
unsafe impl rerail_gpu::Pod for JoinArgsPod {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl JoinArgsPod {
    fn of(a: &rerail_blocks::gpu::JoinArgs) -> JoinArgsPod {
        const _: () = assert!(size_of::<JoinArgsBytes>() == size_of::<rerail_blocks::gpu::JoinArgs>());
        JoinArgsPod(JoinArgsBytes { lbase: a.lbase, ltop: a.ltop, rbase: a.rbase, rtop: a.rtop, out: a.out, n: a.n, idx: a.idx, lkey: a.lkey, lout: a.lout, rkey: a.rkey, rout: a.rout })
    }
}

/// The blocks a view's relations lie in: the base below (none on a first run) and the top.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[derive(Clone, Copy)]
struct Stack<'a> {
    base: Option<&'a Block>,
    top: &'a Block,
}

/// The GPU's edge join over `imp` and `def` (each a view in `stack`), into a relation of `out`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn join_gpu(cx: &Ctx, stack: Stack<'_>, imp: View, def: View, out_f: &Facts, out_r: usize, short: bool) -> Option<JoinRun> {
    use rerail_blocks::gpu::{JoinArgs, NONE};
    let Stack { base, top } = stack;
    let gpu = cx.gpu?;
    let off = |b: Option<&Block>, r: Option<&rerail_blocks::relation::Relation>| match (b, r) {
        (Some(b), Some(r)) => b.offset_of(r) as u64,
        _ => NONE,
    };
    let args = JoinArgs {
        lbase: off(base, imp.base),
        ltop: top.offset_of(imp.top) as u64,
        rbase: off(base, def.base),
        rtop: top.offset_of(def.top) as u64,
        out: out_f.block.offset_of(out_f.rel(out_r).unwrap()) as u64,
        n: imp.total(),
        idx: 1,
        lkey: 1,
        lout: 0,
        rkey: 1,
        rout: 0,
    };
    let before = footprint();
    let run = gpu.join(base, top, &out_f.block, out_f.rel(out_r).unwrap(), &args, short);
    let after = footprint();
    if after > before + (8 << 20) {
        println!("footprint across one GPU join: {} -> {} MB", before >> 20, after >> 20);
    }
    Some(run)
}

/// A cold run: every chunk evaluated (or hit), every block written full.
#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
fn cold(cx: &Ctx, srcs: &[Src], store: Option<&Store>, journal: Option<&Journal>, gpu_edge: bool) -> Kept {
    let sc = cx.sc;
    let refs: Vec<&Src> = srcs.iter().collect();
    let chunks = cx.time("chunks", || eval_chunks(sc, &refs, store, journal));

    let p1 = Facts::full(1, sc.p1.clone(), reserve());
    p1.begin(0);
    for r in [P_DEF, P_IMP, P_HAS] {
        p1.create(r);
    }
    cx.time("p1.s0 exports (threads at once)", || {
        std::thread::scope(|t| {
            for c in &chunks {
                let p1 = &p1;
                t.spawn(move || {
                    for (pr, cr) in [(P_DEF, DEFINES), (P_IMP, IMPORTS), (P_HAS, HAS_ASSERT)] {
                        p1.w(pr).extend(&chunk_rows(c, cr));
                    }
                });
            }
        })
    });
    cx.time("freeze p1.s0", || {
        for r in [P_DEF, P_IMP, P_HAS] {
            p1.freeze_rel(r);
        }
    });
    p1.end(0);

    p1.begin(1);
    p1.create(P_EDGE);
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut on_gpu = false;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if gpu_edge && cx.gpu.is_some() {
        let run = cx.time("p1.s1 edge join (gpu, count + reserve + claim + write)", || {
            join_gpu(cx, Stack { base: None, top: &p1.block }, p1.view(P_IMP), p1.view(P_DEF), &p1, P_EDGE, false)
        });
        on_gpu = run.is_some();
    }
    if !on_gpu {
        let rows = cx.time("p1.s1 edge join (cpu)", || join_cpu(p1.view(P_IMP), p1.view(P_DEF)));
        p1.w(P_EDGE).extend(&rows);
    }
    cx.time("freeze p1.s1", || p1.freeze_rel(P_EDGE));
    if on_gpu {
        // The GPU's rows, claimed in no fixed order, frozen: the same set as the host's join, read the same way.
        let chk = Facts::full(9, sc.edge_check.clone(), reserve());
        chk.begin(0);
        chk.create(0);
        chk.w(0).extend(&join_cpu(p1.view(P_IMP), p1.view(P_DEF)));
        chk.freeze_rel(0);
        chk.end(0);
        cx.check(chk.view(0).digest() == p1.view(P_EDGE).digest(), format!("gpu edge = cpu edge ({} rows)", p1.view(P_EDGE).live_len()));
    }
    p1.end(1);

    p1.begin(2);
    p1.create(P_REACH);
    cx.time("p1.s2 reach (fixpoint, insert_unique)", || closure(&p1, P_REACH, p1.view(P_EDGE)));
    cx.time("freeze p1.s2", || p1.freeze_rel(P_REACH));
    p1.end(2);
    let p1 = p1.freeze();

    let p2 = Facts::full(2, sc.p2.clone(), reserve());
    p2.begin(0);
    p2.create(USES);
    cx.time("p2 uses_assert", || uses_assert(&p2, &p1));
    p2.freeze_rel(USES);
    p2.end(0);
    let p2 = p2.freeze();

    let out = Facts::open(kind::OUTPUTS, 3, sc.out.clone(), reserve(), None);
    out.begin(0);
    out.create(B101);
    out.create(CALL_B101);
    for c in &chunks {
        out.w(B101).extend(&chunk_rows(c, ASSERT_AT));
        out.w(CALL_B101).extend(&chunk_rows(c, CALL_IN));
    }
    out.freeze_rel(B101);
    out.freeze_rel(CALL_B101);
    out.end(0);
    out.begin(1);
    out.create(CROSS);
    let u = p2.view(USES);
    out.w(CROSS).extend(&flat(u, &u.ordered()));
    out.freeze_rel(CROSS);
    out.end(1);
    Kept { p1, p2, out: out.freeze() }
}

fn uses_assert(p2: &Facts, p1: &FrozenFacts) {
    let (reach, has) = (p1.view(P_REACH), p1.view(P_HAS));
    let w = p2.w(USES);
    let mut seen = std::collections::HashSet::new();
    for id in reach.ordered() {
        let (d1, d2) = (reach.get(id, 0), reach.get(id, 1));
        if !has.probe(0, &[d2]).is_empty() && seen.insert(d1) {
            w.push(&[d1]);
        }
    }
}

/// Opens a delta of `kept` (full, or a delta whose base it shares) and carries every relation forward.
fn open_delta(kept: &FrozenFacts, kind_: u32, phase: u32) -> (Facts, Option<FrozenFacts>) {
    let (base, old) = match &kept.base {
        Some(b) => ((**b).clone(), Some(kept.clone())),
        None => (kept.clone(), None),
    };
    let f = Facts::open(kind_, phase, kept.schema.clone(), reserve(), Some(base));
    (f, old)
}

fn make(f: &Facts, old: &Option<FrozenFacts>, r: usize) {
    match old {
        Some(o) => _ = f.carry(o, r),
        None => _ = f.create(r),
    }
}

/// Charges the delta's own bytes (what readers read besides the base), then freezes it.
fn seal(f: Facts) -> FrozenFacts {
    let bytes: usize = (0..f.schema.rels.len()).filter_map(|r| f.rel(r)).map(|r| r.bytes()).sum();
    f.charge(bytes);
    f.freeze()
}

/// A maintained run: `changed` documents were edited. Every block becomes a delta, carried forward.
fn maintained(cx: &Ctx, kept: &Kept, srcs: &[Src], changed: &[usize], journal: &Journal) -> Kept {
    let sc = cx.sc;
    let refs: Vec<&Src> = changed.iter().map(|&i| &srcs[i]).collect();
    let keys: Vec<i64> = refs.iter().map(|s| s.key as i64).collect();
    let chunks = cx.time("chunks (edited only)", || eval_chunks(sc, &refs, None, Some(journal)));

    let (p1, old) = open_delta(&kept.p1, kind::FACTS, 1);
    let was = &kept.p1;
    p1.begin(0);
    cx.time("p1.s0 carry + kill + push", || {
        for (pr, cr) in [(P_DEF, DEFINES), (P_IMP, IMPORTS), (P_HAS, HAS_ASSERT)] {
            make(&p1, &old, pr);
            let w = p1.w(pr);
            for &k in &keys {
                for id in was.view(pr).probe(0, &[k]) {
                    w.kill(id);
                }
            }
            for c in &chunks {
                w.extend(&chunk_rows(c, cr));
            }
        }
    });
    cx.time("freeze p1.s0 (delta: dedup against base)", || {
        for r in [P_DEF, P_IMP, P_HAS] {
            p1.freeze_rel(r);
        }
    });
    p1.end(0);

    p1.begin(1);
    cx.time("p1.s1 edge maintained", || {
        make(&p1, &old, P_EDGE);
        let w = p1.w(P_EDGE);
        let e = was.view(P_EDGE);
        for &k in &keys {
            for id in e.probe(0, &[k]).into_iter().chain(e.probe(1, &[k])) {
                w.kill(id);
            }
        }
        let (imp, def) = (p1.view(P_IMP), p1.view(P_DEF));
        for &k in &keys {
            for id in imp.probe(0, &[k]) {
                for j in def.probe(1, &[imp.get(id, 1)]) {
                    if def.get(j, 0) != k {
                        w.push(&[k, def.get(j, 0)]);
                    }
                }
            }
            for id in def.probe(0, &[k]) {
                for j in imp.probe(1, &[def.get(id, 1)]) {
                    if imp.get(j, 0) != k {
                        w.push(&[imp.get(j, 0), k]);
                    }
                }
            }
        }
    });
    cx.time("freeze p1.s1", || p1.freeze_rel(P_EDGE));
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    if cx.gpu.is_some() {
        // The GPU joins the merged views (base block + this delta block) whole, into a scratch block; with half
        // the room it needs, to take the overflow path. Its set must be the maintained edge.
        let chk = Facts::full(9, sc.edge_check.clone(), reserve());
        chk.begin(0);
        chk.create(0);
        let base = p1.base.as_ref().unwrap();
        let run = cx.time("gpu edge over base + delta (check)", || {
            join_gpu(cx, Stack { base: Some(&base.block), top: &p1.block }, p1.view(P_IMP), p1.view(P_DEF), &chk, 0, true)
        });
        chk.freeze_rel(0);
        chk.end(0);
        let run = run.unwrap();
        cx.check(run.reruns == 1, format!("gpu overflow: one rerun at the exact size ({} reruns)", run.reruns));
        cx.check(chk.view(0).digest() == p1.view(P_EDGE).digest(), format!("gpu join over base + delta = maintained edge ({} rows)", chk.view(0).live_len()));
    }
    p1.end(1);

    p1.begin(2);
    let (kills, adds) = cx.time("p1.s2 reach: recompute + diff", || {
        make(&p1, &old, P_REACH);
        let scratch = Facts::full(9, sc.p1.clone(), reserve());
        scratch.begin(2);
        scratch.create(P_REACH);
        closure(&scratch, P_REACH, p1.view(P_EDGE));
        scratch.freeze_rel(P_REACH);
        scratch.end(2);
        diff_into(&p1, P_REACH, was.view(P_REACH), scratch.view(P_REACH))
    });
    let _ = (kills, adds);
    p1.freeze_rel(P_REACH);
    p1.end(2);
    let p1 = seal(p1);

    let (p2, old2) = open_delta(&kept.p2, kind::FACTS, 2);
    p2.begin(0);
    make(&p2, &old2, USES);
    {
        let scratch = Facts::full(9, sc.p2.clone(), reserve());
        scratch.begin(0);
        scratch.create(USES);
        uses_assert(&scratch, &p1);
        scratch.freeze_rel(USES);
        scratch.end(0);
        diff_into(&p2, USES, kept.p2.view(USES), scratch.view(USES));
    }
    p2.freeze_rel(USES);
    p2.end(0);
    let p2 = seal(p2);

    let (out, old3) = open_delta(&kept.out, kind::OUTPUTS, 3);
    out.begin(0);
    for (r, cr) in [(B101, ASSERT_AT), (CALL_B101, CALL_IN)] {
        make(&out, &old3, r);
        let w = out.w(r);
        for &k in &keys {
            for id in kept.out.view(r).probe(0, &[k]) {
                w.kill(id);
            }
        }
        for c in &chunks {
            w.extend(&chunk_rows(c, cr));
        }
        out.freeze_rel(r);
    }
    out.end(0);
    out.begin(1);
    make(&out, &old3, CROSS);
    diff_into(&out, CROSS, kept.out.view(CROSS), p2.view(USES));
    out.freeze_rel(CROSS);
    out.end(1);
    Kept { p1, p2, out: seal(out) }
}

fn digests(k: &Kept) -> Vec<(String, u64, usize)> {
    let mut v = Vec::new();
    for (tag, f) in [("p1", &k.p1), ("p2", &k.p2), ("out", &k.out)] {
        for (r, d) in f.schema.rels.iter().enumerate() {
            let view = f.view(r);
            v.push((format!("{tag}.{}", d.name), view.digest(), view.live_len()));
        }
    }
    v
}

fn same(cx: &Ctx, what: &str, a: &Kept, b: &Kept) {
    let (da, db) = (digests(a), digests(b));
    for (x, y) in da.iter().zip(&db) {
        cx.check(x.1 == y.1 && x.2 == y.2, format!("{what}: {} ({} rows)", x.0, x.2));
    }
}

/// Rolls a kept delta up when it has cost what a roll-up would (0266 §6), and checks the roll-up reads the same.
fn maybe_rollup(cx: &Ctx, name: &str, f: &mut FrozenFacts, log: &mut Vec<String>) {
    if !f.is_delta() {
        return;
    }
    let (cost, base) = f.cost();
    if cost < base {
        return;
    }
    let rolled = cx.time("rollup", || f.rollup(reserve()));
    for r in 0..f.schema.rels.len() {
        cx.check(rolled.view(r).digest() == f.view(r).digest(), format!("rollup of {name}: {} reads the same", f.schema.rels[r].name));
    }
    log.push(format!("{name}: rolled up at cost {cost} >= base {base} bytes; new full block {} bytes", rolled.block.used()));
    *f = rolled;
}


// ---- packing: every relation written again, exactly sized, in the fewest bits ----

#[derive(Default)]
struct PackTotals {
    rows: usize,
    in_place: usize,
    packed: usize,
    parts: [usize; 7],
}

fn pack_facts(cx: &Ctx, what: &str, f: &FrozenFacts, show: bool, t: &mut PackTotals) {
    for r in 0..f.schema.rels.len() {
        let v = f.view(r);
        let spec = &f.schema.rels[r].spec;
        let (pb, st) = cx.time("pack (from the frozen view)", || packed::pack(&v, spec, kind::FACTS));
        let pk: &packed::Packed = pb.root();
        let mut held = f.rel(r).held_bytes();
        if let Some(b) = &f.base {
            held += b.rel(r).held_bytes();
        }
        cx.check(pk.digest() == v.digest(), format!("{what}.{}: packed reads the same ({} rows)", f.schema.rels[r].name, pk.len()));
        if f.base.is_none() {
            // Frozen in place, a full relation's rows are in canonical order, so its ids are the packed form's
            // positions: a delta's kills name the same rows in either form.
            let same_ids = (0..pk.len()).all(|p| (0..v.arity()).all(|c| v.get(p as u32, c) == pk.get(p, c)));
            cx.check(same_ids && v.total() as usize == pk.len(), format!("{what}.{}: in-place ids = packed positions", f.schema.rels[r].name));
        }
        // Probes: every index, on keys drawn from the rows, the same rows in the same order.
        let mut same = true;
        let ord = v.ordered();
        for (i, ix) in spec.indexes.iter().enumerate() {
            for k in (0..ord.len()).step_by((ord.len() / 64).max(1)) {
                let key: Vec<i64> = ix.iter().map(|&c| v.get(ord[k], c as usize)).collect();
                let a: Vec<Vec<i64>> = v.probe(i, &key).into_iter().map(|id| v.row(id)).collect();
                let b: Vec<Vec<i64>> = pk.probe(ix, &key).map(|p| (0..v.arity()).map(|c| pk.get(p, c)).collect()).collect();
                same &= a == b;
            }
        }
        cx.check(same, format!("{what}.{}: packed probes = in-place probes", f.schema.rels[r].name));
        t.rows += st.rows;
        for (x, y) in t.parts.iter_mut().zip(f.rel(r).held_parts()) {
            *x += y;
        }
        t.in_place += held;
        t.packed += st.bytes;
        if show {
            let encs: Vec<String> = st.cols.iter().map(|(n, b, _)| format!("{n}/{b}")).collect();
            println!(
                "  {:<18} {:>9} rows  in place {:>10} B ({:>5.1} B/row)  packed {:>9} B ({:>5.1} B/row, index {:>8} B)  cols {}",
                format!("{what}.{}", f.schema.rels[r].name),
                st.rows,
                held,
                held as f64 / st.rows.max(1) as f64,
                st.bytes,
                st.bytes as f64 / st.rows.max(1) as f64,
                st.index_bytes,
                encs.join(" ")
            );
        }
    }
}

/// Packs the cold run's blocks: the phases' facts, and a chunk evaluation's file-local relations and documents.
fn pack_paces(cx: &Ctx, kept: &Kept, srcs: &[Src]) {
    println!("packing (in-place frozen -> packed):");
    let mut t = PackTotals::default();
    pack_facts(cx, "p1", &kept.p1, true, &mut t);
    pack_facts(cx, "p2", &kept.p2, true, &mut t);
    pack_facts(cx, "out", &kept.out, true, &mut t);
    let refs: Vec<&Src> = srcs.iter().collect();
    let chunks = eval_chunks(cx.sc, &refs, None, None);
    let mut ct = PackTotals::default();
    let (mut doc_plain, mut doc_packed, mut nodes) = (0usize, 0usize, 0usize);
    for (i, c) in chunks.iter().enumerate() {
        pack_facts(cx, "chunk", c, false, &mut ct);
        // Documents: kind, parent and name per node, plain (2 + 4 + 4 bytes) and as columns; the parent also as
        // its distance back from the node, which is small.
        for d in chunk::docs(&c.block) {
            let n = d.nodes as usize;
            nodes += n;
            doc_plain += n * 10;
            let kind: Vec<i64> = d.kind.as_slice().iter().map(|&x| x as i64).collect();
            let back: Vec<i64> = d.parent.as_slice().iter().enumerate().map(|(j, &p)| j as i64 - p as i64).collect();
            let name: Vec<i64> = d.name.as_slice().iter().map(|&x| x as i64).collect();
            doc_packed += [kind, back, name].iter().map(|v| col::encode(v).bytes() + size_of::<col::IntCol>()).sum::<usize>();
        }
        let _ = i;
    }
    println!(
        "  {:<18} {:>9} rows  in place {:>10} B ({:>5.1} B/row)  packed {:>9} B ({:>5.1} B/row)",
        "chunk relations", ct.rows, ct.in_place, ct.in_place as f64 / ct.rows.max(1) as f64, ct.packed, ct.packed as f64 / ct.rows.max(1) as f64
    );
    println!(
        "  {:<18} {:>9} nodes plain    {:>10} B ({:>5.1} B/node) packed {:>9} B ({:>5.1} B/node)",
        "chunk documents", nodes, doc_plain, doc_plain as f64 / nodes.max(1) as f64, doc_packed, doc_packed as f64 / nodes.max(1) as f64
    );
    for (what, t) in [("phase facts", &t), ("chunk relations", &ct)] {
        let names = ["values", "chunk room", "order", "buckets", "index rows", "dead bits", "descriptors"];
        let parts: Vec<String> = names.iter().zip(t.parts).map(|(n, b)| format!("{n} {:.1}", b as f64 / t.rows.max(1) as f64)).collect();
        println!("  {what}, in place by part (B/row): {}", parts.join(", "));
    }
    println!(
        "  {:<18} {:>9} rows  in place {:>10} B  packed {:>9} B  ({:.1}x smaller)",
        "phase facts", t.rows, t.in_place, t.packed, t.in_place as f64 / t.packed.max(1) as f64
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let ndocs: u32 = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(300);
    let runs: usize = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(8);
    let dir = PathBuf::from(args.get(3).cloned().unwrap_or_else(|| "tmp/paces".into()));
    std::fs::create_dir_all(&dir).unwrap();
    let sc = schemas();

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let gpu = match if std::env::var_os("PACES_NO_GPU").is_some() { Err("PACES_NO_GPU".to_string()) } else { {
        // The process's device (`RERAIL_GPU` chooses its backend).
        Gpu::new()
    } } {
        Ok(g) => Some(g),
        Err(e) => {
            println!("no GPU: {e}");
            None
        }
    };
    let cx = Ctx {
        sc: &sc,
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        gpu: gpu.as_ref(),
        m: Mutex::new(Meter::default()),
        checks: Mutex::new(Vec::new()),
    };

    let fp0 = footprint();
    println!("footprint at start (GPU device and pipelines made): {} MB", fp0 >> 20);
    let mut srcs: Vec<Src> = (0..ndocs).map(|d| make_doc(d, 0)).collect();
    let nodes: usize = srcs.iter().map(|s| s.kind.len()).sum();
    println!("{ndocs} documents, {nodes} nodes; {runs} maintained runs of one edit each");

    // 1. Cold, writing the first journal.
    let jpath = |i: usize| dir.join(format!("journal-{i}.rrb"));
    let j0 = Journal::create(&jpath(0), 4 << 30, ndocs as usize * 2).unwrap();
    let mut kept = cx.time("run: cold", || cold(&cx, &srcs, None, Some(&j0), true));
    let j0len = j0.block.used();
    cx.time("journal freeze", || drop(j0.freeze()));
    println!("footprint after the cold run: {} MB", footprint() >> 20);
    println!("cold: p1 {} bytes, p2 {}, outputs {}, journal {} bytes", kept.p1.block.used(), kept.p2.block.used(), kept.out.block.used(), j0len);
    let mut journals = vec![jpath(0)];
    pack_paces(&cx, &kept, &srcs);

    // 2. Memo-warm cold: every document hits, from the journal reopened from disk.
    let store = cx.time("journal reopen + index", || Store::open(&journals));
    let warm = cx.time("run: memo-warm cold", || cold(&cx, &srcs, Some(&store), None, true));
    same(&cx, "memo-warm = cold", &warm, &kept);
    drop(warm);
    drop(store);

    // 3. Maintained runs, one edit each, checked against a from-scratch run every time.
    let mut r = rng(7);
    let mut rolls = Vec::new();
    let mut ver = vec![0u32; ndocs as usize];
    for run in 1..=runs {
        let d = (r() % ndocs as u64) as usize;
        ver[d] += 1;
        srcs[d] = make_doc(d as u32, ver[d]);
        let jp = jpath(run);
        let j = Journal::create(&jp, reserve(), ndocs as usize).unwrap();
        let next = cx.time("run: maintained (one edit)", || maintained(&cx, &kept, &srcs, &[d], &j));
        drop(j.freeze());
        journals.push(jp);
        let oracle = cx.time("oracle: from scratch", || cold(&cx, &srcs, None, None, false));
        same(&cx, &format!("run {run}: maintained = from scratch"), &next, &oracle);
        drop(oracle);
        kept = next;
        let (c1, b1) = kept.p1.cost();
        println!("run {run}: edited doc {}; p1 delta {} bytes, cost {c1} of base {b1}", d + 1, kept.p1.block.used());
        maybe_rollup(&cx, "p1", &mut kept.p1, &mut rolls);
        maybe_rollup(&cx, "p2", &mut kept.p2, &mut rolls);
        maybe_rollup(&cx, "outputs", &mut kept.out, &mut rolls);
    }
    // A last roll-up of whatever is still a delta, checked.
    for (n, f) in [("p1", &mut kept.p1), ("p2", &mut kept.p2), ("outputs", &mut kept.out)] {
        if f.is_delta() {
            let rolled = f.rollup(reserve());
            for r in 0..f.schema.rels.len() {
                cx.check(rolled.view(r).digest() == f.view(r).digest(), format!("final rollup of {n}: {}", f.schema.rels[r].name));
            }
        }
    }

    // A memo-warm run over every journal written: the edited documents hit their latest entries.
    let store = Store::open(&journals);
    let warm = cx.time("run: memo-warm cold (all journals)", || cold(&cx, &srcs, Some(&store), None, true));
    let oracle = cold(&cx, &srcs, None, None, false);
    same(&cx, "memo-warm after edits = from scratch", &warm, &oracle);
    drop((warm, oracle, store));

    program_paces(&cx);
    journal_paces(&cx, &dir);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    residency(&cx);

    let m = cx.m.lock().unwrap();
    println!("\n{:<58} {:>7} {:>14} {:>11}", "step", "times", "insn", "wall ms");
    for (k, (i, t, n)) in &m.rows {
        println!("{k:<58} {n:>7} {i:>14} {:>11.1}", *t as f64 / 1000.0);
    }
    println!("\nroll-ups:");
    for l in &rolls {
        println!("  {l}");
    }
    let checks = cx.checks.lock().unwrap();
    println!("\n{} checks passed. footprint {} MB (at start {} MB)", checks.len(), footprint() >> 20, fp0 >> 20);
}

fn program_paces(cx: &Ctx) {
    let mk = |id: u32, ver: u32| -> (Vec<[u32; 6]>, String) {
        let mut r = rng(((id as u64) << 8) ^ ver as u64);
        let rules = (0..20 + r() % 40).map(|_| [r() as u32 % 500, 2, r() as u32 % 500, r() as u32 % 500, 0, 0]).collect();
        (rules, format!("module {id} version {ver}"))
    };
    let mods: Vec<(Vec<[u32; 6]>, String)> = (0..40).map(|i| mk(i, 0)).collect();
    let ins: Vec<ModuleIn> = mods.iter().enumerate().map(|(i, (r, t))| ModuleIn { id: i as u32, rules: r, text: t }).collect();
    let full = cx.time("program: build 40 modules", || Program::build(&ins));
    let (er, et) = mk(7, 1);
    let edited = cx.time("program: edit one module (delta)", || full.edit(&[(7, ModuleIn { id: 7, rules: &er, text: &et })]));
    let mut want = mods.clone();
    want[7] = (er.clone(), et.clone());
    let wins: Vec<ModuleIn> = want.iter().enumerate().map(|(i, (r, t))| ModuleIn { id: i as u32, rules: r, text: t }).collect();
    let expect = Program::build(&wins);
    cx.check(edited.digest() == expect.digest(), "program delta reads as the edited program".into());
    cx.check(edited.block.used() < full.block.used() / 10, format!("program delta {} bytes of the full {}", edited.block.used(), full.block.used()));
    let rolled = cx.time("program: rollup (memcpy per module)", || edited.rollup());
    cx.check(rolled.digest() == expect.digest() && rolled.base.is_none(), "program rollup reads the same, and is full".into());
}

fn journal_paces(cx: &Ctx, dir: &Path) {
    let p = dir.join("journal-check.rrb");
    let sc = schemas();
    let j = Journal::create(&p, 64 << 20, 4).unwrap();
    let (s, a, b, o) = chunk::build_entry(1, 2, &[(&sc.entry[0], vec![1, 5, 1, 6])]);
    j.add(&s, a, b, o, 1, 2);
    drop(j.freeze());
    cx.check(journal::open(&p).is_ok(), "a frozen journal opens".into());
    let bytes = std::fs::read(&p).unwrap();
    let short = dir.join("journal-short.rrb");
    std::fs::write(&short, &bytes[..bytes.len() - 100]).unwrap();
    cx.check(journal::open(&short).is_err(), "a journal written back only in part is refused".into());
    let open = dir.join("journal-open.rrb");
    let mut o2 = bytes.clone();
    o2[16..20].copy_from_slice(&rerail_blocks::block::OPEN.to_le_bytes());
    std::fs::write(&open, &o2).unwrap();
    cx.check(journal::open(&open).is_err(), "a journal never frozen is refused".into());
    let crashed = dir.join("journal-crashed.rrb");
    {
        let j = Journal::create(&crashed, 64 << 20, 4).unwrap();
        j.add(&s, a, b, o, 1, 2);
        // The process ends here: never frozen, never truncated.
    }
    cx.check(journal::open(&crashed).is_err(), "a journal its writer never froze is refused".into());
    let reopened = journal::open(&p).unwrap();
    let e = &journal::entries(&reopened)[0];
    cx.check(e.key == 1 && e.digest == 2, "a reopened journal's entry index reads back".into());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn residency(cx: &Ctx) {
    let Some(gpu) = cx.gpu else { return };
    let before = footprint();
    let b = Block::anon(kind::SCRATCH, 4 << 30);
    let buf = gpu.wrap_len(&b, b.reserved());
    let after = footprint();
    cx.check(after.saturating_sub(before) < 64 << 20, format!("a 4 GiB block wrapped for the GPU costs {} KB resident", after.saturating_sub(before) >> 10));
    drop(buf);
    let fb = b.freeze();
    let buf = gpu.wrap(&fb);
    cx.check(buf.length() == (fb.used().div_ceil(1 << 20) << 20) as u64, "a frozen (read-only) block wraps for the GPU, its used extent only".into());
    let again = gpu.wrap(&fb);
    cx.check(again.device_address() == buf.device_address(), "a block's device buffer is kept: the second wrap is the first".into());
}
