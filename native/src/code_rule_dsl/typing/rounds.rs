//! Native lattice evaluation for the spike's W4 (`type_of`) and W4b (`cv`) semantics (../souffle/workload.dl).
//!
//! The facts become one value-flow graph per lattice, which is evaluated by:
//! - four strategies: naive rounds, semi-naive frontier waves, a priority worklist, and SCC condensation;
//! - three kinds of parallelism: sequential, per document in parallel, and wave-parallel with atomics;
//! - a cross-file wavefront over the module dependency graph, with early cutoff on interface digests.
//!
//! Every result is checked against the goldens.
//!
//! usage: dl-lattice <facts dir> <golden dir>
//!
//! In Rerail (05 Phase 10 row 2), `main` is a function over the two directories; under test, every
//! golden check must find no difference.
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use std::time::Instant;

const NONE: u32 = u32::MAX;

#[derive(Default)]
struct Sym {
    map: HashMap<String, u32>,
    strs: Vec<String>,
}
impl Sym {
    fn id(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.map.get(s) {
            return i;
        }
        let i = self.strs.len() as u32;
        self.strs.push(s.to_owned());
        self.map.insert(s.to_owned(), i);
        i
    }
    fn get(&self, s: &str) -> u32 {
        self.map.get(s).copied().unwrap_or(NONE)
    }
    fn s(&self, i: u32) -> &str {
        &self.strs[i as usize]
    }
    fn cat(&mut self, m: u32, s: u32) -> u32 {
        let joined = format!("{}.{}", self.s(m), self.s(s));
        self.id(&joined)
    }
}

/// Compressed rows keyed by a dense id, preserving input order within a key.
fn csr<T: Copy + Default>(n: usize, rows: &[(u32, T)]) -> (Vec<u32>, Vec<T>) {
    let mut off = vec![0u32; n + 1];
    for &(k, _) in rows {
        off[k as usize + 1] += 1;
    }
    for i in 0..n {
        off[i + 1] += off[i];
    }
    let mut pos = off.clone();
    let mut val = vec![T::default(); rows.len()];
    for &(k, v) in rows {
        val[pos[k as usize] as usize] = v;
        pos[k as usize] += 1;
    }
    (off, val)
}

fn read(dir: &str, rel: &str) -> String {
    fs::read_to_string(format!("{dir}/{rel}.facts")).unwrap()
}
fn num(s: &str) -> u32 {
    s.parse().unwrap()
}

struct Facts {
    sy: Sym,
    n: usize,
    module: Vec<u32>,
    path: Vec<String>,
    kind: Vec<u32>,
    doc: Vec<u32>,
    par: Vec<u32>,
    ch_off: Vec<u32>,
    ch: Vec<u32>,
    fl_off: Vec<u32>,
    fl: Vec<(u32, u32)>,
    text: Vec<u32>,
    intval: HashMap<u32, i64>,
    op: Vec<u32>,
}
impl Facts {
    fn load(dir: &str) -> Facts {
        let mut sy = Sym::default();
        let (mut module, mut path) = (Vec::new(), Vec::new());
        for l in read(dir, "doc").lines() {
            let c: Vec<&str> = l.split('\t').collect();
            let d = num(c[0]) as usize;
            if module.len() <= d {
                module.resize(d + 1, NONE);
                path.resize(d + 1, String::new());
            }
            module[d] = sy.id(c[2]);
            path[d] = c[1].to_owned();
        }
        let node = read(dir, "node");
        let mut rows = Vec::new();
        let mut maxn = 0;
        for l in node.lines() {
            let c: Vec<&str> = l.split('\t').collect();
            let n = num(c[1]);
            maxn = maxn.max(n);
            rows.push((n, num(c[0]), sy.id(c[2])));
        }
        // Node ids are global; a subset keeps its original ids, so arrays are sized to the largest id.
        let n = maxn as usize + 1;
        let (mut kind, mut doc) = (vec![NONE; n], vec![NONE; n]);
        for (v, d, k) in rows {
            kind[v as usize] = k;
            doc[v as usize] = d;
        }
        let mut par = vec![NONE; n];
        for l in read(dir, "tape").lines() {
            let c: Vec<&str> = l.split('\t').collect();
            if c[1] != "-1" {
                par[num(c[0]) as usize] = num(c[1]);
            }
        }
        let child: Vec<(u32, u32)> = read(dir, "child")
            .lines()
            .map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                (num(c[0]), num(c[1]))
            })
            .collect();
        let (ch_off, ch) = csr(n, &child);
        let field: Vec<(u32, (u32, u32))> = read(dir, "field")
            .lines()
            .map(|l| {
                let c: Vec<&str> = l.split('\t').collect();
                (num(c[0]), (sy.id(c[1]), num(c[2])))
            })
            .collect();
        let (fl_off, fl) = csr(n, &field);
        let mut text = vec![NONE; n];
        for l in read(dir, "text").lines() {
            let (a, b) = l.split_once('\t').unwrap();
            text[num(a) as usize] = sy.id(b);
        }
        let mut intval = HashMap::new();
        for l in read(dir, "intval").lines() {
            let (a, b) = l.split_once('\t').unwrap();
            intval.insert(num(a), b.parse::<i64>().unwrap());
        }
        let mut op = vec![NONE; n];
        for l in read(dir, "op").lines() {
            let (a, b) = l.split_once('\t').unwrap();
            op[num(a) as usize] = sy.id(b);
        }
        Facts { sy, n, module, path, kind, doc, par, ch_off, ch, fl_off, fl, text, intval, op }
    }
    fn kids(&self, v: u32) -> &[u32] {
        &self.ch[self.ch_off[v as usize] as usize..self.ch_off[v as usize + 1] as usize]
    }
    fn field(&self, v: u32, f: u32) -> impl Iterator<Item = u32> + '_ {
        self.fl[self.fl_off[v as usize] as usize..self.fl_off[v as usize + 1] as usize]
            .iter()
            .filter(move |x| x.0 == f)
            .map(|x| x.1)
    }
}

/// A value-flow graph: copy edges (pulled by `eval`), plus binary-operator nodes whose two operands feed a transfer.
struct Graph {
    n: usize,
    in_off: Vec<u32>,
    ins: Vec<u32>,
    out_off: Vec<u32>,
    outs: Vec<u32>,
    bin: Vec<u32>,
    bins: Vec<(u8, u32, u32)>,
}
impl Graph {
    fn new(n: usize, edges: &[(u32, u32)], bins: &[(u32, u8, u32, u32)]) -> Graph {
        let inr: Vec<(u32, u32)> = edges.iter().map(|&(s, d)| (d, s)).collect();
        let (in_off, ins) = csr(n, &inr);
        let mut outr: Vec<(u32, u32)> = edges.to_vec();
        let mut bin = vec![NONE; n];
        let mut bv = Vec::new();
        for &(v, op, l, r) in bins {
            bin[v as usize] = bv.len() as u32;
            bv.push((op, l, r));
            outr.push((l, v));
            outr.push((r, v));
        }
        outr.sort_unstable();
        outr.dedup();
        let (out_off, outs) = csr(n, &outr);
        Graph { n, in_off, ins, out_off, outs, bin, bins: bv }
    }
    fn ins(&self, v: usize) -> &[u32] {
        &self.ins[self.in_off[v] as usize..self.in_off[v + 1] as usize]
    }
    fn outs(&self, v: usize) -> &[u32] {
        &self.outs[self.out_off[v] as usize..self.out_off[v + 1] as usize]
    }
}

trait Lat: Clone + PartialEq + Send + Sync {
    fn bot() -> Self;
    fn join(&mut self, o: &Self) -> bool;
    fn bin(_op: u8, _a: &Self, _b: &Self) -> Self {
        Self::bot()
    }
}

/// The W4b flat lattice: Bot < C(v) < Top.
#[derive(Clone, PartialEq, Debug)]
enum Cv {
    Bot,
    C(i64),
    Top,
}
const BIG: i64 = 1 << 40;
impl Lat for Cv {
    fn bot() -> Self {
        Cv::Bot
    }
    fn join(&mut self, o: &Self) -> bool {
        match (&*self, o) {
            (_, Cv::Bot) | (Cv::Top, _) => false,
            (Cv::Bot, x) => {
                *self = x.clone();
                true
            }
            (Cv::C(a), Cv::C(b)) if a == b => false,
            _ => {
                *self = Cv::Top;
                true
            }
        }
    }
    // workload.dl derives TOP from either TOP operand before it needs the other one, so TOP is checked first.
    fn bin(op: u8, a: &Self, b: &Self) -> Self {
        match (a, b) {
            (Cv::Top, _) | (_, Cv::Top) => Cv::Top,
            (Cv::Bot, _) | (_, Cv::Bot) => Cv::Bot,
            (Cv::C(a), Cv::C(b)) => match op {
                0 => Cv::C(a.wrapping_add(*b)),
                1 => Cv::C(a.wrapping_sub(*b)),
                2 if *a < BIG && *a > -BIG && *b < BIG && *b > -BIG => Cv::C(a.wrapping_mul(*b)),
                _ => Cv::Top,
            },
        }
    }
}

/// W4's exact set union of type symbols.
#[derive(Clone, PartialEq, Debug, Default)]
struct Ty(Vec<u32>);
fn union(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] < b[j] {
            out.push(a[i]);
            i += 1;
        } else if a[i] > b[j] {
            out.push(b[j]);
            j += 1;
        } else {
            out.push(a[i]);
            i += 1;
            j += 1;
        }
    }
    out.extend_from_slice(&a[i..]);
    out.extend_from_slice(&b[j..]);
    out
}
impl Lat for Ty {
    fn bot() -> Self {
        Ty(Vec::new())
    }
    fn join(&mut self, o: &Self) -> bool {
        if o.0.is_empty() || o.0.iter().all(|t| self.0.binary_search(t).is_ok()) {
            return false;
        }
        self.0 = union(&self.0, &o.0);
        true
    }
}

/// The capped union: more than K members widen to TOP.
const K: usize = 4;
#[derive(Clone, PartialEq, Debug, Default)]
struct TyK {
    top: bool,
    s: Vec<u32>,
}
impl Lat for TyK {
    fn bot() -> Self {
        TyK::default()
    }
    fn join(&mut self, o: &Self) -> bool {
        if self.top || (!o.top && o.s.iter().all(|t| self.s.binary_search(t).is_ok())) {
            return false;
        }
        if o.top {
            *self = TyK { top: true, s: Vec::new() };
            return true;
        }
        self.s = union(&self.s, &o.s);
        if self.s.len() > K {
            *self = TyK { top: true, s: Vec::new() };
        }
        true
    }
}

fn eval<L: Lat>(g: &Graph, seed: &[L], vals: &[L], v: usize) -> L {
    let mut x = seed[v].clone();
    for &s in g.ins(v) {
        x.join(&vals[s as usize]);
    }
    let b = g.bin[v];
    if b != NONE {
        let (op, l, r) = g.bins[b as usize];
        x.join(&L::bin(op, &vals[l as usize], &vals[r as usize]));
    }
    x
}

#[derive(Default, Clone, Debug)]
struct St {
    ms: f64,
    iters: usize,
    visits: u64,
    max_changes: u32,
    max_frontier: usize,
}

fn finish(ch: &[u32], st: &mut St, t: Instant) {
    st.ms = t.elapsed().as_secs_f64() * 1e3;
    st.max_changes = ch.iter().copied().max().unwrap_or(0);
}

fn naive<L: Lat>(g: &Graph, seed: &[L]) -> (Vec<L>, St) {
    let t = Instant::now();
    let mut st = St::default();
    let mut vals = vec![L::bot(); g.n];
    let mut ch = vec![0u32; g.n];
    loop {
        st.iters += 1;
        let new: Vec<L> = (0..g.n).map(|v| eval(g, seed, &vals, v)).collect();
        st.visits += g.n as u64;
        let mut any = false;
        for v in 0..g.n {
            if new[v] != vals[v] {
                ch[v] += 1;
                any = true;
            }
        }
        vals = new;
        if !any {
            break;
        }
    }
    finish(&ch, &mut st, t);
    (vals, st)
}

/// Semi-naive frontier waves: a wave re-evaluates the successors of the nodes that changed in the previous wave.
fn waves<L: Lat>(g: &Graph, seed: &[L]) -> (Vec<L>, St) {
    let t = Instant::now();
    let mut st = St::default();
    let mut vals = vec![L::bot(); g.n];
    let mut ch = vec![0u32; g.n];
    let mut frontier = Vec::new();
    for v in 0..g.n {
        if seed[v] != L::bot() {
            vals[v] = seed[v].clone();
            ch[v] = 1;
            if !g.outs(v).is_empty() {
                frontier.push(v as u32);
            }
        }
    }
    st.max_frontier = frontier.len();
    let mut stamp = vec![0u32; g.n];
    let mut wave = 0u32;
    while !frontier.is_empty() {
        wave += 1;
        let mut cand = Vec::new();
        for &u in &frontier {
            for &v in g.outs(u as usize) {
                if stamp[v as usize] != wave {
                    stamp[v as usize] = wave;
                    cand.push(v);
                }
            }
        }
        st.visits += cand.len() as u64;
        let upd: Vec<(u32, L)> = cand
            .iter()
            .filter_map(|&v| {
                let x = eval(g, seed, &vals, v as usize);
                (x != vals[v as usize]).then_some((v, x))
            })
            .collect();
        frontier.clear();
        for (v, x) in upd {
            vals[v as usize] = x;
            ch[v as usize] += 1;
            frontier.push(v);
        }
        st.max_frontier = st.max_frontier.max(frontier.len());
    }
    st.iters = wave as usize;
    finish(&ch, &mut st, t);
    (vals, st)
}

/// Reverse postorder of the flow graph, by an iterative depth-first search.
fn rpo(g: &Graph) -> Vec<u32> {
    let mut seen = vec![false; g.n];
    let mut post = Vec::with_capacity(g.n);
    for root in 0..g.n {
        if seen[root] {
            continue;
        }
        seen[root] = true;
        let mut stack = vec![(root as u32, 0usize)];
        while let Some(top) = stack.last_mut() {
            let (v, i) = *top;
            let outs = g.outs(v as usize);
            if i < outs.len() {
                top.1 += 1;
                let w = outs[i];
                if !seen[w as usize] {
                    seen[w as usize] = true;
                    stack.push((w, 0));
                }
            } else {
                post.push(v);
                stack.pop();
            }
        }
    }
    let mut rank = vec![0u32; g.n];
    for (i, &v) in post.iter().rev().enumerate() {
        rank[v as usize] = i as u32;
    }
    rank
}

/// A worklist in reverse postorder: a node is re-evaluated when a predecessor changes, earliest rank first.
fn worklist<L: Lat>(g: &Graph, seed: &[L], rank: &[u32]) -> (Vec<L>, St) {
    let t = Instant::now();
    let mut st = St::default();
    let mut vals = vec![L::bot(); g.n];
    let mut ch = vec![0u32; g.n];
    let mut inq = vec![false; g.n];
    let mut heap = BinaryHeap::new();
    for v in 0..g.n {
        if seed[v] != L::bot() {
            vals[v] = seed[v].clone();
            ch[v] = 1;
            for &w in g.outs(v) {
                if !inq[w as usize] {
                    inq[w as usize] = true;
                    heap.push(Reverse((rank[w as usize], w)));
                }
            }
        }
    }
    while let Some(Reverse((_, v))) = heap.pop() {
        let v = v as usize;
        inq[v] = false;
        st.visits += 1;
        let x = eval(g, seed, &vals, v);
        if x != vals[v] {
            vals[v] = x;
            ch[v] += 1;
            for &w in g.outs(v) {
                if !inq[w as usize] {
                    inq[w as usize] = true;
                    heap.push(Reverse((rank[w as usize], w)));
                }
            }
        }
        st.max_frontier = st.max_frontier.max(heap.len());
    }
    finish(&ch, &mut st, t);
    (vals, st)
}

/// Tarjan's strongly connected components, iteratively; components come out sinks first.
fn tarjan(g: &Graph) -> Vec<Vec<u32>> {
    let n = g.n;
    let (mut idx, mut low, mut on) = (vec![NONE; n], vec![0u32; n], vec![false; n]);
    let (mut stack, mut comps, mut counter) = (Vec::new(), Vec::new(), 0u32);
    for root in 0..n {
        if idx[root] != NONE {
            continue;
        }
        let mut cs: Vec<(u32, usize)> = vec![(root as u32, 0)];
        idx[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root as u32);
        on[root] = true;
        while let Some(&(v, i)) = cs.last() {
            let outs = g.outs(v as usize);
            if i < outs.len() {
                cs.last_mut().unwrap().1 += 1;
                let w = outs[i] as usize;
                if idx[w] == NONE {
                    idx[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w as u32);
                    on[w] = true;
                    cs.push((w as u32, 0));
                } else if on[w] {
                    low[v as usize] = low[v as usize].min(idx[w]);
                }
            } else {
                cs.pop();
                if let Some(&(p, _)) = cs.last() {
                    low[p as usize] = low[p as usize].min(low[v as usize]);
                }
                if low[v as usize] == idx[v as usize] {
                    let mut comp = Vec::new();
                    loop {
                        let w = stack.pop().unwrap();
                        on[w as usize] = false;
                        comp.push(w);
                        if w == v {
                            break;
                        }
                    }
                    comps.push(comp);
                }
            }
        }
    }
    comps
}

/// SCC condensation: components in topological order, one evaluation per acyclic node, a local fixpoint per cycle.
fn scc<L: Lat>(g: &Graph, seed: &[L], comps: &[Vec<u32>]) -> (Vec<L>, St) {
    let t = Instant::now();
    let mut st = St::default();
    let mut vals = vec![L::bot(); g.n];
    let mut ch = vec![0u32; g.n];
    for comp in comps.iter().rev() {
        let cyclic = comp.len() > 1 || g.outs(comp[0] as usize).contains(&comp[0]);
        let mut rounds = 0;
        loop {
            rounds += 1;
            let mut changed = false;
            for &v in comp {
                let v = v as usize;
                st.visits += 1;
                let x = eval(g, seed, &vals, v);
                if x != vals[v] {
                    vals[v] = x;
                    ch[v] += 1;
                    changed = true;
                }
            }
            if !cyclic || !changed {
                break;
            }
        }
        st.iters = st.iters.max(rounds);
    }
    finish(&ch, &mut st, t);
    (vals, st)
}

// ---- Atomic lattices for wave-parallel evaluation ------------------------------------------------------------

trait AL: Sync {
    const BOT: u64;
    fn join(a: u64, b: u64) -> u64;
    fn bin(_op: u8, _a: u64, _b: u64) -> u64 {
        Self::BOT
    }
}
/// Cv in one word: 0 = Bot, 1 = Top, `v << 2 | 2` for a constant that fits in 62 bits, and `i << 2 | 3` for an
/// interned larger one. Interning keeps equal constants equal as words, so `join` never has to decode.
struct ACv;
static BIGS: std::sync::Mutex<(Vec<i64>, Option<HashMap<i64, u64>>)> = std::sync::Mutex::new((Vec::new(), None));
fn cv_enc(c: &Cv) -> u64 {
    match c {
        Cv::Bot => 0,
        Cv::Top => 1,
        Cv::C(v) if *v < (1 << 61) && *v >= -(1 << 61) => ((*v as u64) << 2) | 2,
        Cv::C(v) => {
            let mut g = BIGS.lock().unwrap();
            let (list, map) = &mut *g;
            let map = map.get_or_insert_with(HashMap::new);
            let i = *map.entry(*v).or_insert_with(|| {
                list.push(*v);
                (list.len() - 1) as u64
            });
            (i << 2) | 3
        }
    }
}
fn cv_dec(x: u64) -> Cv {
    match (x, x & 3) {
        (0, _) => Cv::Bot,
        (1, _) => Cv::Top,
        (_, 2) => Cv::C((x as i64) >> 2),
        _ => Cv::C(BIGS.lock().unwrap().0[(x >> 2) as usize]),
    }
}
impl AL for ACv {
    const BOT: u64 = 0;
    fn join(a: u64, b: u64) -> u64 {
        if b == 0 || a == b {
            a
        } else if a == 0 {
            b
        } else {
            1
        }
    }
    fn bin(op: u8, a: u64, b: u64) -> u64 {
        cv_enc(&Cv::bin(op, &cv_dec(a), &cv_dec(b)))
    }
}
/// The capped type set in one word: four 16-bit dense type ids, ascending, padded with 0xFFFF; TOP has 0xFFFE first.
struct ATy;
const TY_BOT: u64 = u64::MAX;
const TY_TOP: u64 = 0xFFFF_FFFF_FFFF_FFFE;
fn ty_slots(x: u64) -> impl Iterator<Item = u16> {
    (0..4).map(move |i| (x >> (16 * i)) as u16).filter(|&s| s != 0xFFFF)
}
fn ty_pack(ids: &[u16]) -> u64 {
    let mut x = TY_BOT;
    for (i, &t) in ids.iter().enumerate() {
        x &= !(0xFFFFu64 << (16 * i));
        x |= (t as u64) << (16 * i);
    }
    x
}
impl AL for ATy {
    const BOT: u64 = TY_BOT;
    fn join(a: u64, b: u64) -> u64 {
        if a == TY_TOP || b == TY_TOP {
            return TY_TOP;
        }
        if b == TY_BOT || a == b {
            return a;
        }
        let mut s: Vec<u16> = ty_slots(a).chain(ty_slots(b)).collect();
        s.sort_unstable();
        s.dedup();
        if s.len() > K { TY_TOP } else { ty_pack(&s) }
    }
}

fn eval_a<A: AL>(g: &Graph, seed: &[u64], vals: &[AtomicU64], v: usize) -> u64 {
    let mut x = seed[v];
    for &s in g.ins(v) {
        x = A::join(x, vals[s as usize].load(Relaxed));
    }
    let b = g.bin[v];
    if b != NONE {
        let (op, l, r) = g.bins[b as usize];
        x = A::join(x, A::bin(op, vals[l as usize].load(Relaxed), vals[r as usize].load(Relaxed)));
    }
    x
}
fn cas_join<A: AL>(cell: &AtomicU64, x: u64) -> bool {
    let mut cur = cell.load(Relaxed);
    loop {
        let nw = A::join(cur, x);
        if nw == cur {
            return false;
        }
        match cell.compare_exchange_weak(cur, nw, Relaxed, Relaxed) {
            Ok(_) => return true,
            Err(c) => cur = c,
        }
    }
}
/// Frontier waves in parallel: successors are deduplicated per wave with an atomic stamp, values join by CAS.
fn waves_par<A: AL>(g: &Graph, seed: &[u64]) -> (Vec<u64>, St) {
    let t = Instant::now();
    let mut st = St::default();
    let vals: Vec<AtomicU64> = seed.iter().map(|&s| AtomicU64::new(s)).collect();
    let stamp: Vec<AtomicU32> = (0..g.n).map(|_| AtomicU32::new(0)).collect();
    let mut frontier: Vec<u32> = (0..g.n as u32).into_par_iter().filter(|&v| seed[v as usize] != A::BOT && !g.outs(v as usize).is_empty()).collect();
    st.max_frontier = frontier.len();
    let mut wave = 0u32;
    while !frontier.is_empty() {
        wave += 1;
        let cand: Vec<u32> = frontier
            .par_iter()
            .with_min_len(256)
            .flat_map_iter(|&u| g.outs(u as usize).iter().copied())
            .filter(|&v| stamp[v as usize].swap(wave, Relaxed) != wave)
            .collect();
        st.visits += cand.len() as u64;
        frontier = cand
            .par_iter()
            .with_min_len(256)
            .copied()
            .filter(|&v| {
                let x = eval_a::<A>(g, seed, &vals, v as usize);
                cas_join::<A>(&vals[v as usize], x)
            })
            .collect();
        st.max_frontier = st.max_frontier.max(frontier.len());
    }
    st.iters = wave as usize;
    st.ms = t.elapsed().as_secs_f64() * 1e3;
    (vals.into_iter().map(|a| a.into_inner()).collect(), st)
}

// ---- Graph construction from the facts ----------------------------------------------------------------------

struct Built {
    gn: usize,
    gdoc: Vec<u32>,
    ty_edges: Vec<(u32, u32)>,
    ty_seed: Vec<Ty>,
    cv_edges: Vec<(u32, u32)>,
    cv_bins: Vec<(u32, u8, u32, u32)>,
    cv_seed: Vec<Cv>,
    exported: Vec<Vec<u32>>,
    classes: Vec<Vec<u32>>,
    class_deps: Vec<(u32, u32)>,
    anc_ms: f64,
    build_ms: f64,
}

fn build(f: &mut Facts) -> Built {
    let t = Instant::now();
    let n = f.n;
    let k = |f: &Facts, s: &str| f.sy.get(s);
    let (k_module, k_fn, k_class, k_lambda) = (k(f, "module"), k(f, "function_definition"), k(f, "class_definition"), k(f, "lambda"));
    let (k_ident, k_attr, k_dotted, k_alias) = (k(f, "identifier"), k(f, "attribute"), k(f, "dotted_name"), k(f, "aliased_import"));
    let (k_import, k_from, k_assign, k_aug) = (k(f, "import_statement"), k(f, "import_from_statement"), k(f, "assignment"), k(f, "augmented_assignment"));
    let (k_call, k_ret, k_paren, k_binop, k_int) = (k(f, "call"), k(f, "return_statement"), k(f, "parenthesized_expression"), k(f, "binary_operator"), k(f, "integer"));
    let fld = |f: &mut Facts, s: &str| f.sy.id(s);
    let (f_name, f_alias, f_modname, f_object, f_attribute) = (fld(f, "name"), fld(f, "alias"), fld(f, "module_name"), fld(f, "object"), fld(f, "attribute"));
    let (f_left, f_right, f_function, f_params) = (fld(f, "left"), fld(f, "right"), fld(f, "function"), fld(f, "parameters"));
    let lits = [
        ("integer", "int"), ("float", "float"), ("string", "str"), ("concatenated_string", "str"), ("true", "bool"),
        ("false", "bool"), ("none", "None"), ("list", "list"), ("list_comprehension", "list"), ("dictionary", "dict"),
        ("dictionary_comprehension", "dict"), ("set", "set"), ("set_comprehension", "set"), ("tuple", "tuple"),
    ];
    let lit: HashMap<u32, u32> = lits.iter().map(|(a, b)| (f.sy.id(a), f.sy.id(b))).collect();
    let scope = |kd: u32| kd == k_module || kd == k_fn || kd == k_class || kd == k_lambda;

    // anc_P for P = scope kinds: one forward pass in preorder, since a parent's id precedes its children's.
    let ta = Instant::now();
    let mut anc = vec![NONE; n];
    for v in 0..n {
        let p = f.par[v];
        if p != NONE {
            anc[v] = if scope(f.kind[p as usize]) { p } else { anc[p as usize] };
        }
    }
    let anc_ms = ta.elapsed().as_secs_f64() * 1e3;

    let ndoc = f.module.len();
    let mut modroot = vec![NONE; ndoc];
    for v in 0..n {
        if f.kind[v] == k_module {
            modroot[f.doc[v] as usize] = v as u32;
        }
    }
    // W3 binds and resolves.
    let mut binds: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    for i in 0..n as u32 {
        let (kd, d) = (f.kind[i as usize], f.doc[i as usize]);
        if kd == k_import {
            for nn in f.field(i, f_name).collect::<Vec<_>>() {
                if f.kind[nn as usize] == k_dotted {
                    if let Some(&c) = f.kids(nn).first() {
                        let t = f.text[c as usize];
                        if t != NONE {
                            binds.entry((d, t)).or_default().push(t);
                        }
                    }
                } else if f.kind[nn as usize] == k_alias {
                    for m in f.field(nn, f_name).map(|x| f.text[x as usize]).collect::<Vec<_>>() {
                        for a in f.field(nn, f_alias).map(|x| f.text[x as usize]).collect::<Vec<_>>() {
                            if m != NONE && a != NONE {
                                binds.entry((d, a)).or_default().push(m);
                            }
                        }
                    }
                }
            }
        } else if kd == k_from {
            for m in f.field(i, f_modname).map(|x| f.text[x as usize]).collect::<Vec<_>>() {
                if m == NONE {
                    continue;
                }
                for nn in f.field(i, f_name).collect::<Vec<_>>() {
                    if f.kind[nn as usize] == k_dotted {
                        let s = f.text[nn as usize];
                        if s != NONE {
                            let q = f.sy.cat(m, s);
                            binds.entry((d, s)).or_default().push(q);
                        }
                    } else if f.kind[nn as usize] == k_alias {
                        for s in f.field(nn, f_name).map(|x| f.text[x as usize]).collect::<Vec<_>>() {
                            for a in f.field(nn, f_alias).map(|x| f.text[x as usize]).collect::<Vec<_>>() {
                                if s != NONE && a != NONE {
                                    let q = f.sy.cat(m, s);
                                    binds.entry((d, a)).or_default().push(q);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let mut resolves: HashMap<u32, Vec<u32>> = HashMap::new();
    for v in (0..n as u32).rev() {
        let kd = f.kind[v as usize];
        if kd == k_ident && f.text[v as usize] != NONE {
            if let Some(qs) = binds.get(&(f.doc[v as usize], f.text[v as usize])) {
                let mut qs = qs.clone();
                qs.sort_unstable();
                qs.dedup();
                resolves.insert(v, qs);
            }
        } else if kd == k_attr {
            let mut out = Vec::new();
            for o in f.field(v, f_object).collect::<Vec<_>>() {
                if let Some(ms) = resolves.get(&o).cloned() {
                    for a in f.field(v, f_attribute).collect::<Vec<_>>() {
                        let s = f.text[a as usize];
                        if s != NONE {
                            for &m in &ms {
                                out.push(f.sy.cat(m, s));
                            }
                        }
                    }
                }
            }
            out.sort_unstable();
            out.dedup();
            if !out.is_empty() {
                resolves.insert(v, out);
            }
        }
    }
    // W4 scopes and bindings.
    let mut assigns: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut augdef: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut def_class: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut class_q: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut def_fn: HashMap<(u32, u32), Vec<u32>> = HashMap::new();
    let mut fn_q: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut bound: HashSet<(u32, u32)> = HashSet::new();
    let mut classes = vec![Vec::new(); ndoc];
    let mut exported_fns = vec![Vec::new(); ndoc];
    for v in 0..n as u32 {
        let kd = f.kind[v as usize];
        let s = anc[v as usize];
        if kd == k_assign || kd == k_aug {
            for l in f.field(v, f_left).collect::<Vec<_>>() {
                let x = f.text[l as usize];
                if f.kind[l as usize] != k_ident || x == NONE || s == NONE {
                    continue;
                }
                if kd == k_assign {
                    for r in f.field(v, f_right).collect::<Vec<_>>() {
                        assigns.entry((s, x)).or_default().push(r);
                        bound.insert((s, x));
                    }
                } else {
                    augdef.entry((s, x)).or_default().push(v);
                }
            }
        } else if (kd == k_class || kd == k_fn) && s != NONE && f.kind[s as usize] == k_module {
            for nm in f.field(v, f_name).collect::<Vec<_>>() {
                let x = f.text[nm as usize];
                if x == NONE {
                    continue;
                }
                let d = f.doc[v as usize];
                let q = f.sy.cat(f.module[d as usize], x);
                bound.insert((s, x));
                if kd == k_class {
                    def_class.entry((s, x)).or_default().push(q);
                    class_q.entry(q).or_default().push(d);
                    classes[d as usize].push(q);
                } else {
                    def_fn.entry((s, x)).or_default().push(v);
                    fn_q.entry(q).or_default().push(v);
                    exported_fns[d as usize].push(v);
                }
            }
        }
        if kd == k_fn {
            for ps in f.field(v, f_params).collect::<Vec<_>>() {
                for &p in f.kids(ps) {
                    if f.kind[p as usize] == k_ident && f.text[p as usize] != NONE {
                        bound.insert((v, f.text[p as usize]));
                    }
                    if let Some(&i) = f.kids(p).first() {
                        if f.kind[i as usize] == k_ident && f.text[i as usize] != NONE {
                            bound.insert((v, f.text[i as usize]));
                        }
                    }
                }
            }
        }
    }
    for v in class_q.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    let mut look = vec![NONE; n];
    for v in 0..n {
        let x = f.text[v];
        if f.kind[v] != k_ident || x == NONE || anc[v] == NONE {
            continue;
        }
        let s = anc[v];
        if bound.contains(&(s, x)) {
            look[v] = s;
        } else {
            let m = modroot[f.doc[v] as usize];
            if m != NONE && bound.contains(&(m, x)) {
                look[v] = m;
            }
        }
    }
    // Return slots: one graph node per function_definition, after the AST nodes.
    let mut retslot = vec![NONE; n];
    let mut gdoc = f.doc.clone();
    let mut gn = n as u32;
    for v in 0..n {
        if f.kind[v] == k_fn {
            retslot[v] = gn;
            gn += 1;
            gdoc.push(f.doc[v]);
        }
    }
    let gn = gn as usize;
    let exported: Vec<Vec<u32>> = exported_fns.iter().map(|fs| fs.iter().map(|&x| retslot[x as usize]).collect()).collect();

    let mut ty_seed = vec![Ty::default(); gn];
    let mut ty_edges = Vec::new();
    let mut cv_edges = Vec::new();
    let mut cv_bins = Vec::new();
    let mut cv_seed = vec![Cv::Bot; n];
    let mut class_deps = Vec::new();
    let opcode = |f: &Facts, o: u32| match f.sy.s(o) {
        "+" | "+=" => 0u8,
        "-" | "-=" => 1,
        "*" | "*=" => 2,
        _ => 3,
    };
    for v in 0..n as u32 {
        let vi = v as usize;
        let kd = f.kind[vi];
        if let Some(&t) = lit.get(&kd) {
            ty_seed[vi].0.push(t);
        }
        if look[vi] != NONE {
            let key = (look[vi], f.text[vi]);
            let mut has_def = false;
            for &r in assigns.get(&key).into_iter().flatten() {
                ty_edges.push((r, v));
                cv_edges.push((r, v));
                has_def = true;
            }
            for &a in augdef.get(&key).into_iter().flatten() {
                cv_edges.push((a, v));
                has_def = true;
            }
            if !has_def {
                cv_seed[vi] = Cv::Top;
            }
        } else if kd == k_ident {
            cv_seed[vi] = Cv::Top;
        }
        if kd == k_paren {
            if let Some(&c) = f.kids(v).first() {
                ty_edges.push((c, v));
                cv_edges.push((c, v));
            }
        }
        if kd == k_call {
            for fnode in f.field(v, f_function).collect::<Vec<_>>() {
                let fi = fnode as usize;
                let mut qs: Vec<u32> = Vec::new();
                let mut funcs: Vec<u32> = Vec::new();
                if look[fi] != NONE {
                    let key = (look[fi], f.text[fi]);
                    qs.extend(def_class.get(&key).into_iter().flatten());
                    funcs.extend(def_fn.get(&key).into_iter().flatten());
                }
                for &q in resolves.get(&fnode).into_iter().flatten() {
                    if let Some(ds) = class_q.get(&q) {
                        qs.push(q);
                        for &d in ds {
                            if d != f.doc[vi] {
                                class_deps.push((f.doc[vi], d));
                            }
                        }
                    }
                    funcs.extend(fn_q.get(&q).into_iter().flatten());
                }
                ty_seed[vi].0.extend(qs);
                for func in funcs {
                    ty_edges.push((retslot[func as usize], v));
                }
            }
        }
        if kd == k_ret {
            let fs = anc[vi];
            if fs != NONE && f.kind[fs as usize] == k_fn {
                if let Some(&e) = f.kids(v).first() {
                    ty_edges.push((e, retslot[fs as usize]));
                }
            }
        }
        // W4b seeds and binary transfers.
        let is_const_kind = kd == k_int || kd == k_ident || kd == k_binop || kd == k_paren;
        if kd == k_int {
            cv_seed[vi] = match f.intval.get(&v) {
                Some(&x) => Cv::C(x),
                None => Cv::Top,
            };
        } else if lit.contains_key(&kd) || !is_const_kind {
            cv_seed[vi] = Cv::Top;
        }
        if kd == k_binop || kd == k_aug {
            let o = f.op[vi];
            let (l, r) = (f.field(v, f_left).next(), f.field(v, f_right).next());
            if o != NONE {
                let code = opcode(f, o);
                if kd == k_binop && code == 3 {
                    cv_seed[vi] = Cv::Top;
                }
                let aug_ok = kd == k_binop || matches!(f.sy.s(o), "+=" | "-=" | "*=");
                if let (Some(l), Some(r), true) = (l, r, aug_ok) {
                    cv_bins.push((v, code, l, r));
                }
            }
        }
    }
    for s in ty_seed.iter_mut() {
        s.0.sort_unstable();
        s.0.dedup();
    }
    ty_edges.sort_unstable();
    ty_edges.dedup();
    cv_edges.sort_unstable();
    cv_edges.dedup();
    class_deps.sort_unstable();
    class_deps.dedup();
    Built {
        gn, gdoc, ty_edges, ty_seed, cv_edges, cv_bins, cv_seed, exported, classes, class_deps, anc_ms,
        build_ms: t.elapsed().as_secs_f64() * 1e3,
    }
}

fn golden(dir: &str, name: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_to_string(format!("{dir}/{name}.csv"))
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    v.sort();
    v
}
fn ty_rows(f: &Facts, vals: &[Ty]) -> Vec<String> {
    let mut out = Vec::new();
    for v in 0..f.n {
        for &t in &vals[v].0 {
            out.push(format!("{v}\t{}", f.sy.s(t)));
        }
    }
    out.sort();
    out
}
fn cv_rows(f: &Facts, vals: &[Cv]) -> Vec<String> {
    let (ki, kb) = (f.sy.get("identifier"), f.sy.get("binary_operator"));
    let mut out = Vec::new();
    for v in 0..f.n {
        if let Cv::C(x) = vals[v] {
            if f.kind[v] == ki || f.kind[v] == kb {
                out.push(format!("{v}\t{x}"));
            }
        }
    }
    out.sort();
    out
}
fn diff(a: &[String], b: &[String]) -> usize {
    let (sa, sb): (HashSet<&String>, HashSet<&String>) = (a.iter().collect(), b.iter().collect());
    sa.symmetric_difference(&sb).count()
}

fn line(name: &str, st: &St, check: &str) {
    #[cfg(test)]
    assert!(check.split("diff ").skip(1).all(|n| n.starts_with("0")), "{name}: {check}");
    println!(
        "| {name} | {:.1} | {} | {} | {} | {} | {check} |",
        st.ms, st.iters, st.visits, st.max_changes, st.max_frontier
    );
}

/// A document's own subgraph: local ids, edges inside the document, and the cross-file edges that enter it.
struct DocG {
    l2g: Vec<u32>,
    g: Graph,
    comps: Vec<Vec<u32>>,
    cv_g: Graph,
    cross_in: Vec<(u32, u32)>,
}

pub(crate) fn main(dir: &str, gold: &str) {
    let t0 = Instant::now();
    let mut f = Facts::load(dir);
    let load_ms = t0.elapsed().as_secs_f64() * 1e3;
    let b = build(&mut f);
    let n = f.n;
    let ndoc = f.module.len();
    println!(
        "nodes {n}, graph nodes {} (with return slots), type edges {}, cv edges {} + {} binary transfers",
        b.gn, b.ty_edges.len(), b.cv_edges.len(), b.cv_bins.len()
    );
    println!("load {load_ms:.0} ms; anc_P column {:.2} ms; graph build {:.0} ms", b.anc_ms, b.build_ms);
    let (g_ty, g_cv) = (gold_rows(gold, "type_of"), gold_rows(gold, "cv_const"));

    let tg = Graph::new(b.gn, &b.ty_edges, &[]);
    let mut cv_seed = b.cv_seed.clone();
    cv_seed.resize(b.gn, Cv::Bot);
    let cg = Graph::new(b.gn, &b.cv_edges, &b.cv_bins);
    let ty_k_seed: Vec<TyK> = b.ty_seed.iter().map(|s| TyK { top: s.0.len() > K, s: if s.0.len() > K { vec![] } else { s.0.clone() } }).collect();

    // SCC structure of both graphs.
    let ts = Instant::now();
    let ty_comps = tarjan(&tg);
    let tarjan_ty_ms = ts.elapsed().as_secs_f64() * 1e3;
    let ts = Instant::now();
    let cv_comps = tarjan(&cg);
    let tarjan_cv_ms = ts.elapsed().as_secs_f64() * 1e3;
    for (name, comps, g, ms) in [("type_of", &ty_comps, &tg, tarjan_ty_ms), ("cv", &cv_comps, &cg, tarjan_cv_ms)] {
        let cyclic: Vec<usize> = comps
            .iter()
            .filter(|c| c.len() > 1 || g.outs(c[0] as usize).contains(&c[0]))
            .map(|c| c.len())
            .collect();
        let in_cycles: usize = cyclic.iter().sum();
        let bucket = |lo: usize, hi: usize| cyclic.iter().filter(|&&s| s >= lo && s <= hi).count();
        println!(
            "SCC {name}: {} components in {ms:.0} ms, {} cyclic holding {in_cycles} nodes; sizes 1 (self-loop) {}, 2 {}, 3-10 {}, 11-100 {}, >100 {}; largest {}",
            comps.len(), cyclic.len(), bucket(1, 1), bucket(2, 2), bucket(3, 10), bucket(11, 100), bucket(101, usize::MAX),
            cyclic.iter().max().unwrap_or(&0)
        );
    }
    let ts = Instant::now();
    let ty_rank = rpo(&tg);
    let cv_rank = rpo(&cg);
    println!("reverse postorder for both graphs: {:.0} ms", ts.elapsed().as_secs_f64() * 1e3);

    println!("\n| strategy (whole corpus, 1 thread) | ms | iterations / waves | node visits | max changes per node | max frontier | goldens |");
    println!("|---|---|---|---|---|---|---|");
    let ck_ty = |v: &[Ty]| format!("type_of diff {}", diff(&ty_rows(&f, v), &g_ty));
    let ck_cv = |v: &[Cv]| format!("cv diff {}", diff(&cv_rows(&f, v), &g_cv));
    let (v, st) = naive(&tg, &b.ty_seed);
    line("naive, exact types", &st, &ck_ty(&v));
    let (v, st) = waves(&tg, &b.ty_seed);
    line("semi-naive waves, exact types", &st, &ck_ty(&v));
    let (v, st) = worklist(&tg, &b.ty_seed, &ty_rank);
    line("RPO worklist, exact types", &st, &ck_ty(&v));
    let (exact, st) = scc(&tg, &b.ty_seed, &ty_comps);
    line("SCC condensation, exact types", &st, &ck_ty(&exact));
    let (v, st) = naive(&cg, &cv_seed);
    line("naive, cv", &st, &ck_cv(&v));
    let (v, st) = waves(&cg, &cv_seed);
    line("semi-naive waves, cv", &st, &ck_cv(&v));
    let (v, st) = worklist(&cg, &cv_seed, &cv_rank);
    line("RPO worklist, cv", &st, &ck_cv(&v));
    let (cv_scc, st) = scc(&cg, &cv_seed, &cv_comps);
    line("SCC condensation, cv", &st, &ck_cv(&cv_scc));
    let capped_check = |v: &[TyK]| {
        let top = v[..n].iter().filter(|x| x.top).count();
        let wide = exact[..n].iter().filter(|x| x.0.len() > K).count();
        let lost = (0..n).filter(|&i| v[i].top && exact[i].0.len() <= K).count();
        let agree = (0..n).filter(|&i| !exact[i].0.is_empty() && !v[i].top && v[i].s == exact[i].0).count();
        let typed = (0..n).filter(|&i| !exact[i].0.is_empty()).count();
        format!("TOP {top} (exact >{K}: {wide}; widened by propagation {lost}); identical {agree} of {typed} typed nodes")
    };
    let (v, st) = naive(&tg, &ty_k_seed);
    line("naive, capped K=4", &st, &capped_check(&v));
    let (v, st) = waves(&tg, &ty_k_seed);
    line("semi-naive waves, capped K=4", &st, &capped_check(&v));
    let (v, st) = worklist(&tg, &ty_k_seed, &ty_rank);
    line("RPO worklist, capped K=4", &st, &capped_check(&v));
    let (capped, st) = scc(&tg, &ty_k_seed, &ty_comps);
    line("SCC condensation, capped K=4", &st, &capped_check(&capped));
    let max_exact = exact.iter().map(|x| x.0.len()).max().unwrap_or(0);
    println!("largest exact type set: {max_exact} members (height of the exact lattice on this corpus)");

    // ---- Wave-parallel with atomics, whole corpus and the largest document ----------------------------------
    let mut ty_ids: HashMap<u32, u16> = HashMap::new();
    let mut ty_back: Vec<u32> = Vec::new();
    for s in &b.ty_seed {
        for &t in &s.0 {
            ty_ids.entry(t).or_insert_with(|| {
                ty_back.push(t);
                (ty_back.len() - 1) as u16
            });
        }
    }
    assert!(ty_back.len() < 0xFFFE);
    let aty_seed: Vec<u64> = b.ty_seed.iter().map(|s| {
        if s.0.len() > K { TY_TOP } else {
            let mut ids: Vec<u16> = s.0.iter().map(|t| ty_ids[t]).collect();
            ids.sort_unstable();
            ty_pack(&ids)
        }
    }).collect();
    let acv_seed: Vec<u64> = cv_seed.iter().map(cv_enc).collect();
    let dec_ty = |x: u64| -> TyK {
        if x == TY_TOP { TyK { top: true, s: vec![] } } else {
            let mut s: Vec<u32> = ty_slots(x).map(|i| ty_back[i as usize]).collect();
            s.sort_unstable();
            TyK { top: false, s }
        }
    };
    let threads = rayon::current_num_threads();
    println!("\n| wave-parallel ({threads} threads, atomics) | ms | waves | node visits | — | max frontier | agrees with sequential |");
    println!("|---|---|---|---|---|---|---|");
    let (v, st) = waves_par::<ACv>(&cg, &acv_seed);
    let ok = (0..b.gn).all(|i| cv_dec(v[i]) == cv_scc[i]);
    line("corpus, cv", &st, &format!("{ok}"));
    let (v, st) = waves_par::<ATy>(&tg, &aty_seed);
    let ok = (0..b.gn).all(|i| dec_ty(v[i]) == capped[i]);
    line("corpus, capped types", &st, &format!("{ok}"));

    // ---- Per-document subgraphs ---------------------------------------------------------------------------------
    let tb = Instant::now();
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); ndoc];
    for v in 0..b.gn {
        members[b.gdoc[v] as usize].push(v as u32);
    }
    let mut g2l = vec![NONE; b.gn];
    for m in &members {
        for (i, &v) in m.iter().enumerate() {
            g2l[v as usize] = i as u32;
        }
    }
    let mut ty_local: Vec<Vec<(u32, u32)>> = vec![Vec::new(); ndoc];
    let mut cross: Vec<Vec<(u32, u32)>> = vec![Vec::new(); ndoc];
    for &(s, d) in &b.ty_edges {
        let (ds, dd) = (b.gdoc[s as usize], b.gdoc[d as usize]);
        if ds == dd {
            ty_local[dd as usize].push((g2l[s as usize], g2l[d as usize]));
        } else {
            cross[dd as usize].push((s, g2l[d as usize]));
        }
    }
    let mut cv_local: Vec<Vec<(u32, u32)>> = vec![Vec::new(); ndoc];
    for &(s, d) in &b.cv_edges {
        assert_eq!(b.gdoc[s as usize], b.gdoc[d as usize], "cv flow never crosses files");
        cv_local[b.gdoc[d as usize] as usize].push((g2l[s as usize], g2l[d as usize]));
    }
    let mut cv_bins_local: Vec<Vec<(u32, u8, u32, u32)>> = vec![Vec::new(); ndoc];
    for &(v, op, l, r) in &b.cv_bins {
        cv_bins_local[b.gdoc[v as usize] as usize].push((g2l[v as usize], op, g2l[l as usize], g2l[r as usize]));
    }
    let docs: Vec<DocG> = (0..ndoc)
        .into_par_iter()
        .map(|d| {
            let g = Graph::new(members[d].len(), &ty_local[d], &[]);
            let comps = tarjan(&g);
            DocG { l2g: members[d].clone(), comps, cv_g: Graph::new(members[d].len(), &cv_local[d], &cv_bins_local[d]), g, cross_in: cross[d].clone() }
        })
        .collect();
    println!("\nper-document subgraphs built in {:.0} ms (parallel); cross-file type edges {}", tb.elapsed().as_secs_f64() * 1e3, cross.iter().map(Vec::len).sum::<usize>());
    let lseed_ty = |d: &DocG| -> Vec<Ty> { d.l2g.iter().map(|&v| b.ty_seed[v as usize].clone()).collect() };
    let lseed_cv = |d: &DocG| -> Vec<Cv> { d.l2g.iter().map(|&v| cv_seed[v as usize].clone()).collect() };

    // Per-document cv in parallel (cv is entirely doc-local, so this is the whole answer).
    let tp = Instant::now();
    let per: Vec<(Vec<Cv>, St)> = docs.par_iter().map(|d| scc(&d.cv_g, &lseed_cv(d), &tarjan(&d.cv_g))).collect();
    let par_ms = tp.elapsed().as_secs_f64() * 1e3;
    let mut cv_all = vec![Cv::Bot; b.gn];
    for (d, (vals, _)) in docs.iter().zip(&per) {
        for (i, x) in vals.iter().enumerate() {
            cv_all[d.l2g[i] as usize] = x.clone();
        }
    }
    println!("per-document rayon, cv (SCC per document, Tarjan included): {par_ms:.1} ms, {}", ck_cv(&cv_all));
    let tp = Instant::now();
    let _: Vec<(Vec<Ty>, St)> = docs.par_iter().map(|d| scc(&d.g, &lseed_ty(d), &d.comps)).collect();
    println!("per-document rayon, exact types, doc-local part only (R0): {:.1} ms", tp.elapsed().as_secs_f64() * 1e3);
    let tp = Instant::now();
    let _: Vec<(Vec<Ty>, St)> = (0..ndoc).map(|i| scc(&docs[i].g, &lseed_ty(&docs[i]), &docs[i].comps)).collect();
    println!("same, one thread: {:.1} ms", tp.elapsed().as_secs_f64() * 1e3);

    // Waves per document (semi-naive, doc-local), for the GPU estimate.
    let mut wv: Vec<(usize, usize, usize, u32)> = docs
        .par_iter()
        .map(|d| {
            let (_, a) = waves(&d.cv_g, &lseed_cv(d));
            let (_, t) = waves(&d.g, &lseed_ty(d).iter().map(|s| TyK { top: s.0.len() > K, s: if s.0.len() > K { vec![] } else { s.0.clone() } }).collect::<Vec<_>>());
            (a.iters, t.iters, d.l2g.len(), a.max_changes.max(t.max_changes))
        })
        .collect();
    let pct = |v: &mut Vec<usize>, p: f64| {
        v.sort_unstable();
        v[((v.len() as f64 - 1.0) * p) as usize]
    };
    let mut wcv: Vec<usize> = wv.iter().map(|x| x.0).collect();
    let mut wty: Vec<usize> = wv.iter().map(|x| x.1).collect();
    let (c50, c99) = (pct(&mut wcv, 0.5), pct(&mut wcv, 0.99));
    let (t50, t99) = (pct(&mut wty, 0.5), pct(&mut wty, 0.99));
    println!(
        "waves per document: cv p50 {c50} p99 {c99} max {}; capped types p50 {t50} p99 {t99} max {}; max changes of any node {}",
        wcv.last().unwrap(), wty.last().unwrap(), wv.iter().map(|x| x.3).max().unwrap()
    );
    wv.sort_by_key(|x| Reverse(x.2));
    let big = (0..ndoc).max_by_key(|&d| docs[d].l2g.len()).unwrap();
    let dbig = &docs[big];
    println!("largest document: {} ({} graph nodes)", f.path[big], dbig.l2g.len());
    println!("| largest document | ms | waves | node visits | max changes | max frontier | agrees |");
    println!("|---|---|---|---|---|---|---|");
    let (vs, st) = scc(&dbig.cv_g, &lseed_cv(dbig), &tarjan(&dbig.cv_g));
    line("SCC, cv, 1 thread", &st, "—");
    let (vw, st) = waves(&dbig.cv_g, &lseed_cv(dbig));
    line("semi-naive waves, cv, 1 thread", &st, &format!("{}", vw == vs));
    let a_seed: Vec<u64> = lseed_cv(dbig).iter().map(cv_enc).collect();
    let (va, st) = waves_par::<ACv>(&dbig.cv_g, &a_seed);
    line(&format!("wave-parallel, cv, {threads} threads"), &st, &format!("{}", va.iter().map(|&x| cv_dec(x)).collect::<Vec<_>>() == vs));
    let (_, st) = scc(&dbig.g, &lseed_ty(dbig), &dbig.comps);
    line("SCC, exact types (doc-local), 1 thread", &st, "—");
    let la: Vec<u64> = dbig.l2g.iter().map(|&v| aty_seed[v as usize]).collect();
    let (_, st) = waves_par::<ATy>(&dbig.g, &la);
    line(&format!("wave-parallel, capped types, {threads} threads"), &st, "—");

    // ---- Cross-file wavefront -------------------------------------------------------------------------------------
    // D depends on M when a type flows from M's return slot into D, or D constructs a class defined in M.
    let mut deps: Vec<HashSet<u32>> = vec![HashSet::new(); ndoc];
    for (d, dg) in docs.iter().enumerate() {
        for &(s, _) in &dg.cross_in {
            deps[d].insert(b.gdoc[s as usize]);
        }
    }
    for &(d, m) in &b.class_deps {
        deps[d as usize].insert(m);
    }
    let dep_edges: Vec<(u32, u32)> = deps.iter().enumerate().flat_map(|(d, ms)| ms.iter().map(move |&m| (d as u32, m))).collect();
    let dg = Graph::new(ndoc, &dep_edges, &[]);
    let dcomps = tarjan(&dg); // dependencies first
    let mut comp_of = vec![0usize; ndoc];
    for (i, c) in dcomps.iter().enumerate() {
        for &d in c {
            comp_of[d as usize] = i;
        }
    }
    let mut layer = vec![0usize; dcomps.len()];
    for (i, c) in dcomps.iter().enumerate() {
        for &d in c {
            for &m in &deps[d as usize] {
                let j = comp_of[m as usize];
                if j != i {
                    layer[i] = layer[i].max(layer[j] + 1);
                }
            }
        }
    }
    let nlayers = layer.iter().max().map_or(0, |x| x + 1);
    let cyclic_docs: usize = dcomps.iter().filter(|c| c.len() > 1).map(Vec::len).sum();
    let with_deps = deps.iter().filter(|s| !s.is_empty()).count();
    let mut rdeps: Vec<Vec<u32>> = vec![Vec::new(); ndoc];
    for &(d, m) in &dep_edges {
        rdeps[m as usize].push(d);
    }
    println!(
        "\ndependency graph: {} edges, {with_deps} of {ndoc} modules depend on another; {} components, {} cyclic holding {cyclic_docs} modules; {nlayers} layers; widest layer {}",
        dep_edges.len(), dcomps.len(), dcomps.iter().filter(|c| c.len() > 1).count(),
        (0..nlayers).map(|l| (0..dcomps.len()).filter(|&i| layer[i] == l).map(|i| dcomps[i].len()).sum::<usize>()).max().unwrap_or(0)
    );

    // Layered evaluation: each document once after its dependencies; a cyclic component iterates until its
    // interface digests stop changing. `ret` holds the current value of every return slot.
    // `over` holds return slots updated inside a cyclic component that is still iterating.
    let get = |s: u32, ret: &[Ty], over: &HashMap<u32, Ty>| -> Ty { over.get(&s).cloned().unwrap_or_else(|| ret[s as usize].clone()) };
    let eval_doc = |d: usize, ret: &[Ty], over: &HashMap<u32, Ty>, extra: &HashMap<u32, Ty>| -> Vec<Ty> {
        let dgr = &docs[d];
        let mut seed = lseed_ty(dgr);
        for &(s, l) in &dgr.cross_in {
            seed[l as usize].join(&get(s, ret, over));
        }
        for (i, &v) in dgr.l2g.iter().enumerate() {
            if let Some(x) = extra.get(&v) {
                seed[i].join(x);
            }
        }
        scc(&dgr.g, &seed, &dgr.comps).0
    };
    let digest = |d: usize, ret: &[Ty], over: &HashMap<u32, Ty>| -> u64 {
        let mut h = DefaultHasher::new();
        for &r in &b.exported[d] {
            (r, get(r, ret, over).0).hash(&mut h);
        }
        b.classes[d].hash(&mut h);
        h.finish()
    };
    let run_layers = |extra: &HashMap<u32, Ty>| -> (Vec<Ty>, usize, f64) {
        let t = Instant::now();
        let mut all = vec![Ty::default(); b.gn];
        let mut evals = 0;
        for l in 0..nlayers {
            let comps: Vec<usize> = (0..dcomps.len()).filter(|&i| layer[i] == l).collect();
            // Components of one layer are independent; run them in parallel.
            let out: Vec<(usize, Vec<(usize, Vec<Ty>)>)> = comps
                .par_iter()
                .map(|&ci| {
                    let mut over: HashMap<u32, Ty> = HashMap::new();
                    let mut res: Vec<(usize, Vec<Ty>)> = Vec::new();
                    let single = dcomps[ci].len() == 1;
                    let mut rounds = 0;
                    loop {
                        rounds += 1;
                        let before: Vec<u64> = dcomps[ci].iter().map(|&d| digest(d as usize, &all, &over)).collect();
                        res.clear();
                        for &d in &dcomps[ci] {
                            let vals = eval_doc(d as usize, &all, &over, extra);
                            for (i, x) in vals.iter().enumerate() {
                                let gv = docs[d as usize].l2g[i];
                                if gv as usize >= n {
                                    over.insert(gv, x.clone());
                                }
                            }
                            res.push((d as usize, vals));
                        }
                        let after: Vec<u64> = dcomps[ci].iter().map(|&d| digest(d as usize, &all, &over)).collect();
                        if single || before == after {
                            break;
                        }
                    }
                    (rounds * dcomps[ci].len(), res)
                })
                .collect();
            for (count, res) in out {
                evals += count;
                for (d, vals) in res {
                    for (i, x) in vals.into_iter().enumerate() {
                        all[docs[d].l2g[i] as usize] = x;
                    }
                }
            }
        }
        (all, evals, t.elapsed().as_secs_f64() * 1e3)
    };
    let (all, evals, ms) = run_layers(&HashMap::new());
    println!("layered wavefront: {nlayers} layers, {evals} document evaluations, {ms:.0} ms, {}", ck_ty(&all));
    let base_ret = all.clone();

    // Edits to the most depended-on module, re-derived with early cutoff on interface digests.
    let mstar = (0..ndoc).max_by_key(|&d| rdeps[d].len()).unwrap();
    let mut seen = HashSet::from([mstar as u32]);
    let mut q = vec![mstar as u32];
    while let Some(x) = q.pop() {
        for &d in &rdeps[x as usize] {
            if seen.insert(d) {
                q.push(d);
            }
        }
    }
    println!(
        "most depended-on module: {} ({} direct dependents, {} transitive including itself: the re-derivation cost without cutoff)",
        f.path[mstar], rdeps[mstar].len(), seen.len()
    );
    let edit_sym = f.sy.id("EDITED");
    let rederive = |extra: &HashMap<u32, Ty>| -> (usize, usize, f64, Vec<Ty>) {
        let t = Instant::now();
        let mut ret = base_ret.clone();
        let mut todo: std::collections::BTreeSet<(usize, u32)> = std::collections::BTreeSet::new();
        todo.insert((comp_of[mstar], mstar as u32));
        let (mut derived, mut changed) = (0, 0);
        while let Some((_, d)) = todo.pop_first() {
            let d = d as usize;
            derived += 1;
            let none = HashMap::new();
            let before = digest(d, &ret, &none);
            let vals = eval_doc(d, &ret, &none, extra);
            for (i, x) in vals.into_iter().enumerate() {
                ret[docs[d].l2g[i] as usize] = x;
            }
            if digest(d, &ret, &none) != before {
                changed += 1;
                for &r in &rdeps[d] {
                    todo.insert((comp_of[r as usize], r));
                }
            }
        }
        (derived, changed, t.elapsed().as_secs_f64() * 1e3, ret)
    };
    // Edit A: a body change whose value reaches no return slot (a node with no flow successors).
    let body = docs[mstar].l2g.iter().copied().find(|&v| (v as usize) < n && tg.outs(v as usize).is_empty() && b.ty_seed[v as usize].0.len() == 1).unwrap();
    let (dn, cn, ms, _) = rederive(&HashMap::from([(body, Ty(vec![edit_sym]))]));
    println!("edit A (body, interface unchanged): {dn} module re-derived, {cn} digest changed, {ms:.1} ms");
    // Edit B: the exported function of M* that flows into the most other files gains a return type.
    let mut uses: HashMap<u32, usize> = HashMap::new();
    for dgr in &docs {
        for &(s, _) in &dgr.cross_in {
            if b.gdoc[s as usize] as usize == mstar {
                *uses.entry(s).or_default() += 1;
            }
        }
    }
    if let Some((&slot, &cnt)) = uses.iter().max_by_key(|x| *x.1) {
        let extra = HashMap::from([(slot, Ty(vec![edit_sym]))]);
        let (dn, cn, ms, ret) = rederive(&extra);
        let (full, _, fms) = run_layers(&extra);
        let same = (0..b.gn).all(|v| ret[v] == full[v]);
        println!(
            "edit B (return type of a function used at {cnt} cross-file call sites): {dn} modules re-derived, {cn} digests changed, {ms:.1} ms; full layered recompute {fms:.0} ms; return slots agree with it: {same}"
        );
    } else {
        println!("edit B: the most depended-on module has no function used across files");
    }
}

fn gold_rows(dir: &str, name: &str) -> Vec<String> {
    golden(dir, name)
}

#[cfg(test)]
mod tests;
