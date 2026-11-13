//! Decision 0173's version simulation. Replays a `RERAIL_VERSION_TRACE` (each place a run made one program from
//! another, with both programs' texts) through each form a program could take, and measures each operation:
//! making the new version, reading it (every node, and every head relation's rules by index), and its digest.
//!
//! Forms:
//! - `tree`:  today's `front::Program`: clone, then retain and push (what every traced site does).
//! - `flat`:  one flat image per program (nodes in columns): copy the image, tombstone, append, rebuild its index.
//! - `pool`:  a run's rule pool, a program a list of rule ids: copy the list, retain, append, rebuild its index.
//! - `v:*`:   the proposal: views (ranges of a base), overlays (a base, a removed set, added ids), derivations (a new
//!            base), under a layout of the roots (source, head, oracle), a fold policy, and a removed-set kind.
//!
//! Usage: versions <trace dir> [more trace dirs…]
use rerail_rr::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Lists, Program, Rule, Term, Value};
use rerail_logic::code_rule_dsl::logic::program_text;
use std::alloc::{GlobalAlloc, Layout, System};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{BuildHasher, Hash, Hasher};
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

// ---- measurement ----

struct Counting;
static BYTES: AtomicU64 = AtomicU64::new(0);
static ALLOCS: AtomicU64 = AtomicU64::new(0);
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        BYTES.fetch_add(l.size() as u64, Relaxed);
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // A realloc may copy: counted as the new size, as an allocation would be.
        BYTES.fetch_add(n as u64, Relaxed);
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
}
#[global_allocator]
static A: Counting = Counting;

fn retired() -> u64 {
    let mut info: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V4, (&raw mut info).cast()) };
    if ok == 0 { info.ri_instructions } else { 0 }
}

#[derive(Default, Clone, Copy)]
struct Cost {
    insn: u64,
    bytes: u64,
    allocs: u64,
    n: u64,
}
impl std::ops::AddAssign for Cost {
    fn add_assign(&mut self, o: Cost) {
        self.insn += o.insn;
        self.bytes += o.bytes;
        self.allocs += o.allocs;
        self.n += o.n;
    }
}

static mut OVERHEAD: u64 = 0;
fn measure<R>(f: impl FnOnce() -> R) -> (R, Cost) {
    let (b, a) = (BYTES.load(Relaxed), ALLOCS.load(Relaxed));
    let i = retired();
    let r = black_box(f());
    let j = retired();
    let over = unsafe { OVERHEAD };
    (r, Cost { insn: (j - i).saturating_sub(over), bytes: BYTES.load(Relaxed) - b, allocs: ALLOCS.load(Relaxed) - a, n: 1 })
}

fn fx(bytes: &[u8]) -> u64 {
    foldhash::quality::FixedState::with_seed(0x0173).hash_one(bytes)
}

// ---- encoding: a program's items (rules, and rows of its other parts) as nodes ----

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
struct Node {
    // u32, not u8: no padding, so a node's bytes are all initialized and hash the same every time.
    kind: u32,
    size: u32,
    val: u32,
}

const RULE: u8 = 1;
const ATOM: u8 = 2;
const CMP: u8 = 3;
const AGG: u8 = 4;
const NEAREST: u8 = 5;
const AST: u8 = 6;
const VAR: u8 = 7;
const STR: u8 = 8;
const INT: u8 = 9;
const WILD: u8 = 10;
const TOP: u8 = 11;
const CALL: u8 = 12;
const DECL: u8 = 13;
const FACT: u8 = 14;
const INPUT: u8 = 15;
const OUTPUT: u8 = 16;
const PAIR: u8 = 17;

#[derive(Default)]
struct Atoms(HashMap<String, u32>);
impl Atoms {
    fn id(&mut self, s: &str) -> u32 {
        let n = self.0.len() as u32;
        *self.0.entry(s.to_owned()).or_insert(n)
    }
}

struct Enc<'a> {
    out: Vec<Node>,
    atoms: &'a mut Atoms,
    slots: HashMap<String, u32>,
}
impl Enc<'_> {
    fn open(&mut self, kind: u8, val: u32) -> usize {
        self.out.push(Node { kind: kind as u32, size: 0, val });
        self.out.len() - 1
    }
    fn close(&mut self, at: usize) {
        self.out[at].size = (self.out.len() - at) as u32;
    }
    fn leaf(&mut self, kind: u8, val: u32) {
        self.out.push(Node { kind: kind as u32, size: 1, val });
    }
    fn slot(&mut self, v: &str) -> u32 {
        let n = self.slots.len() as u32;
        *self.slots.entry(v.to_owned()).or_insert(n)
    }
    fn term(&mut self, t: &Term) {
        match t {
            Term::Var(v) => {
                let s = self.slot(v);
                self.leaf(VAR, s)
            }
            Term::Wild => self.leaf(WILD, 0),
            Term::Top => self.leaf(TOP, 0),
            Term::Int(i) => self.leaf(INT, *i as u32),
            Term::Str(s) => {
                let a = self.atoms.id(s);
                self.leaf(STR, a)
            }
            Term::Call(f, args) => {
                let a = self.atoms.id(f);
                let at = self.open(CALL, a);
                for x in args.iter() {
                    self.term(x);
                }
                self.close(at);
            }
        }
    }
    fn atom(&mut self, a: &Atom) {
        let flags = (a.neg as u8) << 6 | (a.gated as u8) << 7;
        let r = self.atoms.id(&a.rel);
        let at = self.open(ATOM | flags, r);
        for x in a.args.iter() {
            self.term(x);
        }
        self.close(at);
    }
    fn var(&mut self, v: &str) {
        let s = self.slot(v);
        self.leaf(VAR, s);
    }
    fn literal(&mut self, l: &Literal) {
        match l {
            Literal::Atom(a) => self.atom(a),
            Literal::Cmp { l, op, r } => {
                let o = self.atoms.id(op);
                let at = self.open(CMP, o);
                self.term(l);
                self.term(r);
                self.close(at);
            }
            Literal::Agg { var, op, over, atom } => {
                let code = match op {
                    AggOp::Count => 0,
                    AggOp::Min => 1,
                    AggOp::Max => 2,
                    AggOp::Sum => 3,
                    AggOp::Product => 4,
                    AggOp::Join { .. } => 5,
                };
                let s = self.slot(var);
                let at = self.open(AGG | code << 5, s);
                match over {
                    Some(v) => self.var(v),
                    None => self.leaf(WILD, 0),
                }
                if let AggOp::Join { by, sep } = op {
                    for b in by {
                        self.var(b);
                    }
                    let a = self.atoms.id(sep);
                    self.leaf(STR, a);
                }
                self.atom(atom);
                self.close(at);
            }
            Literal::Nearest { n, a, kindset } => {
                let k = self.atoms.id(kindset);
                let at = self.open(NEAREST, k);
                self.var(n);
                self.var(a);
                self.close(at);
            }
            Literal::Ast { document, node, matcher, captures } => {
                let m = self.atoms.id(matcher);
                let at = self.open(AST, m);
                self.var(document);
                self.var(node);
                for (n, v) in captures {
                    let a = self.atoms.id(n);
                    self.leaf(STR, a);
                    self.var(v);
                }
                self.close(at);
            }
        }
    }
}

/// One item of a program: a rule, or a row of its other parts. `head` is its relation's atom.
#[derive(Clone)]
struct Item {
    nodes: Vec<Node>,
    hash: u64,
    head: u32,
    rule: bool,
}

fn node_bytes(n: &[Node]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(n.as_ptr().cast(), std::mem::size_of_val(n)) }
}

fn item(nodes: Vec<Node>, head: u32, rule: bool) -> Item {
    let hash = fx(node_bytes(&nodes));
    Item { nodes, hash, head, rule }
}

fn encode(p: &Program, atoms: &mut Atoms) -> Vec<Item> {
    let mut items = Vec::new();
    for r in &p.rules {
        let head = atoms.id(&r.head.rel);
        let mut e = Enc { out: Vec::new(), atoms, slots: HashMap::new() };
        let at = e.open(RULE, 0);
        e.atom(&r.head);
        for l in r.body.iter() {
            e.literal(l);
        }
        e.close(at);
        let out = e.out;
        items.push(item(out, head, true));
    }
    for (rel, cols) in &p.decls {
        let head = atoms.id(rel);
        let mut e = Enc { out: Vec::new(), atoms, slots: HashMap::new() };
        let at = e.open(DECL, head);
        for (n, t) in cols.iter() {
            let p = e.open(PAIR, 0);
            let (a, b) = (e.atoms.id(n), e.atoms.id(t));
            e.leaf(STR, a);
            e.leaf(STR, b);
            e.close(p);
        }
        e.close(at);
        let out = e.out;
        items.push(item(out, head, false));
    }
    for (rel, rows) in &p.facts {
        let head = atoms.id(rel);
        for row in rows {
            let mut e = Enc { out: Vec::new(), atoms, slots: HashMap::new() };
            let at = e.open(FACT, head);
            for v in row.iter() {
                match v {
                    Value::Int(i) => e.leaf(INT, *i as u32),
                    Value::Str(s) => {
                        let a = e.atoms.id(s);
                        e.leaf(STR, a)
                    }
                    #[allow(unreachable_patterns)]
                    _ => e.leaf(TOP, 0),
                }
            }
            e.close(at);
            let out = e.out;
            items.push(item(out, head, false));
        }
    }
    for (kind, list) in [(INPUT, &p.inputs), (OUTPUT, &p.outputs)] {
        for rel in list {
            let head = atoms.id(rel);
            items.push(item(vec![Node { kind: kind as u32, size: 1, val: head }], head, false));
        }
    }
    items
}

// ---- the trace ----

struct Op {
    site: String,
    from: String,
    to: String,
}

struct Prog {
    tree: Program,
    items: Vec<Item>,
}

/// What an op changes, as every form is given it: which of `from`'s items are kept, and `to`'s items that are new.
struct Delta {
    keep: Vec<bool>,
    added: Vec<usize>,
}

fn delta(f: &Prog, t: &Prog) -> Delta {
    let mut want: HashMap<u64, usize> = HashMap::new();
    for i in &t.items {
        *want.entry(i.hash).or_default() += 1;
    }
    let keep: Vec<bool> = f
        .items
        .iter()
        .map(|i| match want.get_mut(&i.hash) {
            Some(n) if *n > 0 => {
                *n -= 1;
                true
            }
            _ => false,
        })
        .collect();
    let mut have: HashMap<u64, usize> = HashMap::new();
    for (i, k) in f.items.iter().zip(&keep) {
        if *k {
            *have.entry(i.hash).or_default() += 1;
        }
    }
    let added = t
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| match have.get_mut(&i.hash) {
            Some(n) if *n > 0 => {
                *n -= 1;
                false
            }
            _ => true,
        })
        .map(|(k, _)| k)
        .collect();
    Delta { keep, added }
}

// ---- reads, the same for every form: every node, then each head's items by index ----

fn visit(nodes: &[Node]) -> u64 {
    let mut s = 0u64;
    for n in nodes {
        s = s.wrapping_add(n.kind as u64 ^ n.val as u64);
    }
    s
}

// tree
fn tree_visit_term(t: &Term) -> u64 {
    match t {
        Term::Call(_, a) => 1 + a.iter().map(tree_visit_term).sum::<u64>(),
        Term::Var(v) | Term::Str(v) => v.len() as u64,
        _ => 1,
    }
}
fn tree_visit_atom(a: &Atom) -> u64 {
    a.rel.len() as u64 + a.args.iter().map(tree_visit_term).sum::<u64>()
}
fn tree_visit_rule(r: &Rule) -> u64 {
    tree_visit_atom(&r.head)
        + r.body
            .iter()
            .map(|l| match l {
                Literal::Atom(a) => tree_visit_atom(a),
                Literal::Cmp { l, r, .. } => tree_visit_term(l) + tree_visit_term(r),
                Literal::Agg { atom, .. } => 1 + tree_visit_atom(atom),
                _ => 1,
            })
            .sum::<u64>()
}
fn tree_read(p: &Program, heads: &[String]) -> u64 {
    let mut s: u64 = p.rules.iter().map(tree_visit_rule).sum();
    let mut by_head: HashMap<&str, Vec<&Rule>> = HashMap::new();
    for r in &p.rules {
        by_head.entry(r.head.rel.as_str()).or_default().push(r);
    }
    for h in heads {
        for r in by_head.get(h.as_str()).into_iter().flatten() {
            s += tree_visit_rule(r);
        }
        s += p.decls.get(h.as_str()).map_or(0, |c| c.len() as u64);
        s += p.facts.get(h.as_str()).map_or(0, |f| f.len() as u64);
    }
    s
}
fn tree_digest(p: &Program) -> u64 {
    // As `memo::program_digest` and `retain::key` do: the `Debug` text of the parts.
    let text = format!("{:?}{:?}{:?}{:?}{:?}", p.rules, p.decls, p.facts, p.inputs, p.outputs);
    fx(text.as_bytes())
}

// CSR by head over positions 0..n: (head, position) pairs sorted, the heads found by binary search. Three arrays, as
// an image's index regions are; no map.
struct Csr {
    heads: Vec<u32>,
    starts: Vec<u32>,
    pos: Vec<u32>,
}
fn csr(pairs: impl Iterator<Item = (u32, u32)>) -> Csr {
    let mut v: Vec<u64> = pairs.map(|(h, p)| (h as u64) << 32 | p as u64).collect();
    v.sort_unstable();
    let mut heads = Vec::new();
    let mut starts = Vec::new();
    let mut pos = Vec::with_capacity(v.len());
    for x in v {
        let h = (x >> 32) as u32;
        if heads.last() != Some(&h) {
            heads.push(h);
            starts.push(pos.len() as u32);
        }
        pos.push(x as u32);
    }
    starts.push(pos.len() as u32);
    Csr { heads, starts, pos }
}
impl Csr {
    fn get(&self, h: u32) -> &[u32] {
        match self.heads.binary_search(&h) {
            Ok(k) => &self.pos[self.starts[k] as usize..self.starts[k + 1] as usize],
            Err(_) => &[],
        }
    }
}

// flat: one image a program
struct Flat {
    nodes: Vec<Node>,
    start: Vec<u32>,
    len: Vec<u32>,
    head: Vec<u32>,
    hash: Vec<u64>,
    dead: Vec<u32>,
    by_head: Csr,
}
fn flat_of(items: &[Item]) -> Flat {
    let mut f = Flat { nodes: Vec::new(), start: Vec::new(), len: Vec::new(), head: Vec::new(), hash: Vec::new(), dead: Vec::new(), by_head: csr(std::iter::empty()) };
    for i in items {
        f.start.push(f.nodes.len() as u32);
        f.len.push(i.nodes.len() as u32);
        f.nodes.extend_from_slice(&i.nodes);
        f.head.push(i.head);
        f.hash.push(i.hash);
        f.dead.push(0);
    }
    f.by_head = csr(f.head.iter().enumerate().map(|(p, &h)| (h, p as u32)));
    f
}
fn flat_make(f: &Flat, d: &Delta, t: &Prog) -> Flat {
    let mut n = Flat { nodes: f.nodes.clone(), start: f.start.clone(), len: f.len.clone(), head: f.head.clone(), hash: f.hash.clone(), dead: f.dead.clone(), by_head: csr(std::iter::empty()) };
    for (k, keep) in d.keep.iter().enumerate() {
        if !keep {
            n.dead[k] = 1;
        }
    }
    for &a in &d.added {
        let i = &t.items[a];
        n.start.push(n.nodes.len() as u32);
        n.len.push(i.nodes.len() as u32);
        n.nodes.extend_from_slice(&i.nodes);
        n.head.push(i.head);
        n.hash.push(i.hash);
        n.dead.push(0);
    }
    n.by_head = csr(n.head.iter().enumerate().filter(|(p, _)| n.dead[*p] == 0).map(|(p, &h)| (h, p as u32)));
    n
}
fn flat_read(f: &Flat, heads: &[u32]) -> u64 {
    let mut s = 0;
    for k in 0..f.start.len() {
        if f.dead[k] == 0 {
            s += visit(&f.nodes[f.start[k] as usize..(f.start[k] + f.len[k]) as usize]);
        }
    }
    for &h in heads {
        for &p in f.by_head.get(h) {
            s += visit(&f.nodes[f.start[p as usize] as usize..(f.start[p as usize] + f.len[p as usize]) as usize]);
        }
    }
    s
}
fn flat_digest(f: &Flat) -> u64 {
    let mut h = rustc_hash::FxHasher::default();
    for k in 0..f.start.len() {
        if f.dead[k] == 0 {
            node_bytes(&f.nodes[f.start[k] as usize..(f.start[k] + f.len[k]) as usize]).hash(&mut h);
        }
    }
    h.finish()
}

// pool: a run's items once each
#[derive(Clone, Default)]
struct Pool {
    nodes: Vec<Node>,
    start: Vec<u32>,
    len: Vec<u32>,
    head: Vec<u32>,
    hash: Vec<u64>,
    index: hashbrown::HashMap<u64, u32>,
}
impl Pool {
    fn intern(&mut self, i: &Item) -> u32 {
        // The bytes' hash is computed here, as an append would.
        let h = fx(node_bytes(&i.nodes));
        if let Some(&id) = self.index.get(&h) {
            return id;
        }
        let id = self.start.len() as u32;
        self.start.push(self.nodes.len() as u32);
        self.len.push(i.nodes.len() as u32);
        self.nodes.extend_from_slice(&i.nodes);
        self.head.push(i.head);
        self.hash.push(h);
        self.index.insert(h, id);
        id
    }
    fn nodes(&self, id: u32) -> &[Node] {
        &self.nodes[self.start[id as usize] as usize..(self.start[id as usize] + self.len[id as usize]) as usize]
    }
}

struct Listed {
    ids: Vec<u32>,
    by_head: Csr,
}
fn listed_make(pool: &mut Pool, f: &Listed, d: &Delta, t: &Prog) -> Listed {
    let mut ids = f.ids.clone();
    let mut k = 0;
    ids.retain(|_| {
        k += 1;
        d.keep[k - 1]
    });
    for &a in &d.added {
        ids.push(pool.intern(&t.items[a]));
    }
    let by_head = csr(ids.iter().enumerate().map(|(p, &id)| (pool.head[id as usize], p as u32)));
    Listed { ids, by_head }
}
fn listed_read(pool: &Pool, l: &Listed, heads: &[u32]) -> u64 {
    let mut s = 0;
    for &id in &l.ids {
        s += visit(pool.nodes(id));
    }
    for &h in heads {
        for &p in l.by_head.get(h) {
            s += visit(pool.nodes(l.ids[p as usize]));
        }
    }
    s
}
fn listed_digest(pool: &Pool, l: &Listed) -> u64 {
    l.ids.iter().fold(0u64, |a, &id| a.wrapping_add(pool.hash[id as usize]))
}

// ---- the proposal: views, overlays, derivations ----

#[derive(Clone, Copy, PartialEq, Debug)]
enum Removed {
    Sorted,
    Hash,
    Bitmap,
}

enum RemovedSet {
    Sorted(Vec<u32>),
    Hash(HashSet<u32>),
    Bitmap(Vec<u64>),
}
impl RemovedSet {
    fn has(&self, id: u32) -> bool {
        match self {
            RemovedSet::Sorted(v) => v.binary_search(&id).is_ok(),
            RemovedSet::Hash(h) => h.contains(&id),
            RemovedSet::Bitmap(b) => b.get(id as usize / 64).is_some_and(|w| w >> (id % 64) & 1 == 1),
        }
    }
    fn len(&self) -> usize {
        match self {
            RemovedSet::Sorted(v) => v.len(),
            RemovedSet::Hash(h) => h.len(),
            RemovedSet::Bitmap(b) => b.iter().map(|w| w.count_ones() as usize).sum(),
        }
    }
}

enum Version {
    Base { ids: Vec<u32>, by_head: Csr, digest: u64 },
    /// Ranges of a base's positions.
    View { base: std::rc::Rc<Version>, ranges: Vec<(u32, u32)>, size: usize, digest: u64 },
    Overlay { parent: std::rc::Rc<Version>, removed: RemovedSet, added: Vec<u32>, by_head: HashMap<u32, Vec<u32>>, size: usize, depth: usize, digest: u64 },
}
impl Version {
    fn size(&self) -> usize {
        match self {
            Version::Base { ids, .. } => ids.len(),
            Version::View { size, .. } | Version::Overlay { size, .. } => *size,
        }
    }
    fn depth(&self) -> usize {
        match self {
            Version::Base { .. } => 0,
            Version::View { .. } => 1,
            Version::Overlay { depth, .. } => *depth,
        }
    }
    fn digest(&self) -> u64 {
        match self {
            Version::Base { digest, .. } | Version::View { digest, .. } | Version::Overlay { digest, .. } => *digest,
        }
    }
    fn each(&self, f: &mut dyn FnMut(u32)) {
        match self {
            Version::Base { ids, .. } => ids.iter().for_each(|&i| f(i)),
            Version::View { base, ranges, .. } => {
                let Version::Base { ids, .. } = &**base else { unreachable!() };
                for &(s, e) in ranges {
                    ids[s as usize..e as usize].iter().for_each(|&i| f(i));
                }
            }
            Version::Overlay { parent, removed, added, .. } => {
                parent.each(&mut |i| {
                    if !removed.has(i) {
                        f(i)
                    }
                });
                added.iter().for_each(|&i| f(i));
            }
        }
    }
    fn lookup(&self, h: u32, f: &mut dyn FnMut(u32)) {
        match self {
            Version::Base { ids, by_head, .. } => by_head.get(h).iter().for_each(|&p| f(ids[p as usize])),
            Version::View { base, ranges, .. } => {
                let Version::Base { ids, by_head, .. } = &**base else { unreachable!() };
                for &p in by_head.get(h) {
                    let k = ranges.partition_point(|&(s, _)| s <= p);
                    if k > 0 && p < ranges[k - 1].1 {
                        f(ids[p as usize]);
                    }
                }
            }
            Version::Overlay { parent, removed, by_head, .. } => {
                parent.lookup(h, &mut |i| {
                    if !removed.has(i) {
                        f(i)
                    }
                });
                by_head.get(&h).into_iter().flatten().for_each(|&i| f(i));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Layout0 {
    Source,
    Head,
    Oracle,
}

#[derive(Clone, Copy, Debug)]
struct Policy {
    layout: Layout0,
    /// Fold an overlay into a base when its delta reaches this fraction of its parent (None: never).
    fold: Option<f64>,
    max_depth: usize,
    removed: Removed,
    /// Most ranges a view may have; past it the subset is an overlay.
    max_ranges: usize,
}

fn base_of(pool: &Pool, mut ids: Vec<u32>, layout: Layout0, oracle: Option<&HashMap<u32, u64>>) -> Version {
    match layout {
        Layout0::Source => {}
        Layout0::Head => ids.sort_by_key(|&i| pool.head[i as usize]),
        Layout0::Oracle => match oracle {
            Some(sig) => ids.sort_by_key(|i| (sig.get(i).copied().unwrap_or(0), pool.head[*i as usize])),
            None => ids.sort_by_key(|&i| pool.head[i as usize]),
        },
    }
    let by_head = csr(ids.iter().enumerate().map(|(p, &id)| (pool.head[id as usize], p as u32)));
    let digest = ids.iter().fold(0u64, |a, &id| a.wrapping_add(pool.hash[id as usize]));
    Version::Base { ids, by_head, digest }
}

fn materialize(pool: &Pool, v: &Version, layout: Layout0) -> Version {
    let mut ids = Vec::with_capacity(v.size());
    v.each(&mut |i| ids.push(i));
    base_of(pool, ids, if layout == Layout0::Oracle { Layout0::Head } else { layout }, None)
}

/// Makes `to` from `from`'s version: a view, an overlay or a derivation, and folds by the policy.
fn v_make(pool: &mut Pool, from: &std::rc::Rc<Version>, from_ids: &[u32], d: &Delta, t: &Prog, p: &Policy) -> (Version, &'static str) {
    let removed: Vec<u32> = from_ids.iter().zip(&d.keep).filter(|(_, k)| !**k).map(|(&i, _)| i).collect();
    let to_size = from_ids.len() - removed.len() + d.added.len();
    // A derivation: most of `to` is new.
    if d.added.len() * 2 > to_size {
        let mut ids: Vec<u32> = from_ids.iter().zip(&d.keep).filter(|(_, k)| **k).map(|(&i, _)| i).collect();
        for &a in &d.added {
            ids.push(pool.intern(&t.items[a]));
        }
        return (base_of(pool, ids, if p.layout == Layout0::Oracle { Layout0::Head } else { p.layout }, None), "derivation");
    }
    // A view: a subset of a base, in few ranges.
    if d.added.is_empty()
        && let Version::Base { ids, .. } = &**from
    {
        let mut ranges = Vec::new();
        let mut run: Option<u32> = None;
        for (k, keep) in d.keep.iter().enumerate() {
            match (keep, run) {
                (true, None) => run = Some(k as u32),
                (false, Some(s)) => {
                    ranges.push((s, k as u32));
                    run = None;
                }
                _ => {}
            }
        }
        if let Some(s) = run {
            ranges.push((s, d.keep.len() as u32));
        }
        if ranges.len() <= p.max_ranges {
            let digest = removed.iter().fold(from.digest(), |a, &i| a.wrapping_sub(pool.hash[i as usize]));
            let _ = ids;
            return (Version::View { base: from.clone(), ranges, size: to_size, digest }, "view");
        }
    }
    // An overlay over `from`.
    let added: Vec<u32> = d.added.iter().map(|&a| pool.intern(&t.items[a])).collect();
    let mut by_head: HashMap<u32, Vec<u32>> = HashMap::new();
    for &i in &added {
        by_head.entry(pool.head[i as usize]).or_default().push(i);
    }
    let mut digest = from.digest();
    for &i in &removed {
        digest = digest.wrapping_sub(pool.hash[i as usize]);
    }
    for &i in &added {
        digest = digest.wrapping_add(pool.hash[i as usize]);
    }
    let removed_set = match p.removed {
        Removed::Sorted => {
            let mut r = removed;
            r.sort_unstable();
            RemovedSet::Sorted(r)
        }
        Removed::Hash => RemovedSet::Hash(removed.into_iter().collect()),
        Removed::Bitmap => {
            // A bitmap over the pool's ids: its size is the pool's, not the edit's.
            let mut b = vec![0u64; pool.start.len() / 64 + 1];
            for i in removed {
                b[i as usize / 64] |= 1 << (i % 64);
            }
            RemovedSet::Bitmap(b)
        }
    };
    let change = removed_set.len() + added.len();
    let v = Version::Overlay { parent: from.clone(), removed: removed_set, added, by_head, size: to_size, depth: from.depth() + 1, digest };
    let fold = v.depth() > p.max_depth || p.fold.is_some_and(|f| change as f64 >= f * from.size().max(1) as f64);
    if fold { (materialize(pool, &v, p.layout), "overlay, folded") } else { (v, "overlay") }
}

fn v_read(pool: &Pool, v: &Version, heads: &[u32]) -> u64 {
    let mut s = 0;
    v.each(&mut |i| s += visit(pool.nodes(i)));
    for &h in heads {
        v.lookup(h, &mut |i| s += visit(pool.nodes(i)));
    }
    s
}

// ---- the replay ----

#[derive(Default)]
struct Tally {
    make: Cost,
    read: Cost,
    digest: Cost,
}

fn main() {
    let dirs: Vec<String> = std::env::args().skip(1).collect();
    // Calibrate the measurement's own cost.
    let mut over = u64::MAX;
    for _ in 0..200 {
        let i = retired();
        let j = retired();
        over = over.min(j - i);
    }
    unsafe { OVERHEAD = over };
    println!("measurement overhead: {over} instructions a measure (subtracted)");
    if dirs.first().is_some_and(|d| d == "--validate") {
        for dir in &dirs[1..] {
            validate(Path::new(dir));
        }
        return;
    }
    if dirs.first().is_some_and(|d| d == "--reads") {
        for dir in &dirs[1..] {
            reads(Path::new(dir));
        }
        return;
    }
    for dir in &dirs {
        replay(Path::new(dir));
    }
}

/// What reading a stored program costs, as its text and as its image (decision 0173, stage 1), over every distinct
/// program of the trace: the text parsed (`read_in`, today's warm read); the image decoded into the tree
/// (`decode`, stage 1's read, which every caller still needs); and the image opened alone (`Image::open`, what a
/// reader by id, stage 2, pays). The store's decompression and its query are the same for both and not counted.
fn reads(dir: &Path) {
    use rerail_logic::code_rule_dsl::logic::program_image::{self, Image};
    let lists = Lists::default();
    let mut names: Vec<_> = std::fs::read_dir(dir).expect("trace dir").filter_map(|e| {
        let p = e.ok()?.path();
        (p.extension()? == "rr").then_some(p)
    }).collect();
    names.sort();
    let (mut text, mut image, mut open, mut rules) = (Cost::default(), Cost::default(), Cost::default(), 0usize);
    let (mut text_bytes, mut image_bytes) = (0usize, 0usize);
    for path in &names {
        let t = std::fs::read_to_string(path).expect("program text");
        let (p, c) = measure(|| program_text::read_in(&t, &lists).expect("reads"));
        text += c;
        let bytes = program_image::encode(&p).expect("encodes");
        let (q, c) = measure(|| program_image::decode(&bytes, &lists).expect("decodes"));
        image += c;
        assert_eq!(format!("{p:?}"), format!("{q:?}"), "{}: the image is not the program", path.display());
        let (_, c) = measure(|| Image::open(&bytes).expect("opens"));
        open += c;
        rules += p.rules.len();
        text_bytes += t.len();
        image_bytes += bytes.len();
    }
    println!("\n==== {}: {} programs, {} rules; texts {:.2} MB, images {:.2} MB", dir.display(), names.len(), rules, text_bytes as f64 / 1e6, image_bytes as f64 / 1e6);
    for (what, c) in [("text, parsed", text), ("image, decoded to the tree", image), ("image, opened", open)] {
        println!("  {what:<28} {:>9.1} Minsn {:>8.1} MB allocated {:>9} allocations", c.insn as f64 / 1e6, c.bytes as f64 / 1e6, c.allocs);
    }
}

/// Every making of the trace, made by the real `program_image::Version::edit` from its parent's version (each
/// `to` stated as an edit of its `from`, by `diff`): each version must be its program, rule for rule (an overlay
/// appends what it adds, so rules are compared as a multiset) and part for part, and its digest the digest of that
/// program made whole.
fn validate(dir: &Path) {
    use rerail_logic::code_rule_dsl::logic::program_facts;
    use rerail_logic::code_rule_dsl::logic::program_image::{self, version::Made, Version};
    let text = std::fs::read_to_string(dir.join("trace.tsv")).expect("trace.tsv");
    let ops: Vec<(String, String, String)> = text.lines().filter_map(|l| {
        let mut f = l.split('\t');
        Some((f.next()?.to_owned(), f.next()?.to_owned(), f.next()?.to_owned()))
    }).collect();
    let lists = Lists::default();
    let mut progs: HashMap<String, Program> = HashMap::new();
    let mut load = |d: &str, progs: &mut HashMap<String, Program>| {
        if !progs.contains_key(d) {
            let t = std::fs::read_to_string(dir.join(format!("{d}.rr"))).expect("program text");
            progs.insert(d.to_owned(), program_text::read_in(&t, &lists).expect("reads"));
        }
    };
    let key = |p: &Program| {
        let mut rules: Vec<String> = p.rules.iter().map(|r| format!("{r:?}")).collect();
        rules.sort();
        let mut rest = p.clone();
        rest.rules.clear();
        (rules, format!("{rest:?}"))
    };
    let mut versions: HashMap<String, std::sync::Arc<Version>> = HashMap::new();
    let (mut ok, mut made_count): (usize, BTreeMap<String, usize>) = (0, BTreeMap::new());
    let (mut edit_insn, mut tree_insn, mut delta_bytes, mut whole_bytes) = (0u64, 0u64, 0usize, 0usize);
    for (site, from, to) in &ops {
        load(from, &mut progs);
        load(to, &mut progs);
        let (f, t) = (&progs[from], &progs[to]);
        let parent = versions.entry(from.clone()).or_insert_with(|| Version::open(&program_image::encode(f).expect("encodes")).expect("opens")).clone();
        let mut about = program_facts::Facts::new();
        program_facts::write_about(&mut about, "a", t);
        let ((v, made), c) = measure(|| {
            let edit = parent.diff(t, &about, &lists).expect("diff");
            Version::edit(&parent, edit, &lists).expect("edit")
        });
        edit_insn += c.insn;
        let (_, c) = measure(|| {
            let mut p = f.clone();
            p.rules.retain(|_| true);
            p
        });
        tree_insn += c.insn;
        if let Version::Overlay { delta, .. } = &*v {
            delta_bytes += delta.bytes().len();
            whole_bytes += program_image::encode(t).expect("encodes").len();
        }
        *made_count.entry(format!("{site} {made:?}")).or_default() += 1;
        let got = v.program(&lists).expect("reads");
        assert_eq!(key(&got), key(t), "{site}: {from} → {to}");
        let whole = Version::open(&program_image::encode(t).expect("encodes")).expect("opens");
        assert_eq!(v.digest(&lists).unwrap(), whole.digest(&lists).unwrap(), "{site}: {from} → {to}: digest");
        let _ = Made::View;
        versions.insert(to.clone(), v);
        ok += 1;
    }
    println!("\n==== {}: {ok} of {} makings made by Version::edit are their programs, digests equal", dir.display(), ops.len());
    for (k, n) in &made_count {
        println!("  {k:<36} {n:>6}");
    }
    println!("  diff + edit {:.1} Minsn (the diff stands in for a pass's own statement of its edit); overlays' deltas {:.2} MB, against {:.2} MB for the same programs whole", edit_insn as f64 / 1e6, delta_bytes as f64 / 1e6, whole_bytes as f64 / 1e6);
    let _ = tree_insn;
}

fn replay(dir: &Path) {
    let text = std::fs::read_to_string(dir.join("trace.tsv")).expect("trace.tsv");
    let ops: Vec<Op> = text
        .lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            Some(Op { site: f.next()?.to_owned(), from: f.next()?.to_owned(), to: f.next()?.to_owned() })
        })
        .collect();
    println!("\n==== {} : {} ops", dir.display(), ops.len());

    // Setup, unmeasured: every program read and encoded, as each form would already hold it.
    let lists = Lists::default();
    let mut atoms = Atoms::default();
    let mut progs: HashMap<String, Prog> = HashMap::new();
    for op in &ops {
        for d in [&op.from, &op.to] {
            if !progs.contains_key(d) {
                let text = std::fs::read_to_string(dir.join(format!("{d}.rr"))).expect("program text");
                let tree = program_text::read_in(&text, &lists).expect("program reads");
                let items = encode(&tree, &mut atoms);
                progs.insert(d.clone(), Prog { tree, items });
            }
        }
    }
    let deltas: Vec<Delta> = ops.iter().map(|o| delta(&progs[&o.from], &progs[&o.to])).collect();
    let heads_of = |p: &Prog| -> (Vec<String>, Vec<u32>) {
        let mut s: Vec<String> = p.tree.rules.iter().map(|r| r.head.rel.to_string()).collect();
        s.sort();
        s.dedup();
        let mut ids: Vec<u32> = p.items.iter().filter(|i| i.rule).map(|i| i.head).collect();
        ids.sort();
        ids.dedup();
        (s, ids)
    };
    let heads: HashMap<&str, (Vec<String>, Vec<u32>)> = progs.iter().map(|(d, p)| (d.as_str(), heads_of(p))).collect();
    let made: HashSet<&str> = ops.iter().map(|o| o.to.as_str()).collect();
    let roots: Vec<&str> = {
        let mut r: Vec<&str> = ops.iter().map(|o| o.from.as_str()).filter(|f| !made.contains(f)).collect();
        r.sort();
        r.dedup();
        r
    };

    // What the ops are.
    let mut shape: BTreeMap<&str, (u64, u64, u64, u64, u64)> = BTreeMap::new();
    for (o, d) in ops.iter().zip(&deltas) {
        let e = shape.entry(o.site.as_str()).or_default();
        let from = d.keep.len() as u64;
        let removed = d.keep.iter().filter(|k| !**k).count() as u64;
        e.0 += 1;
        e.1 += from;
        e.2 += removed;
        e.3 += d.added.len() as u64;
        e.4 += (from - removed + d.added.len() as u64) as u64;
    }
    println!("\n{:<20} {:>5} {:>10} {:>10} {:>10} {:>10}", "site", "ops", "from items", "removed", "added", "to items");
    for (s, e) in &shape {
        println!("{:<20} {:>5} {:>10} {:>10} {:>10} {:>10}", s, e.0, e.1 / e.0, e.2 / e.0, e.3 / e.0, e.4 / e.0);
    }
    println!("(per op, averages; items are rules and rows of the program's other parts)");

    // Branching: an op whose `from` is made from again later, or read again, cannot change `from` in place.
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for o in &ops {
        *uses.entry(o.from.as_str()).or_default() += 1;
    }
    let branching = ops.iter().filter(|o| uses[o.from.as_str()] > 1).count();
    let same = ops.iter().filter(|o| o.from == o.to).count();
    println!("\nops whose `from` is made from again (in place is wrong for them): {branching} of {}; ops that change nothing: {same}", ops.len());
    let mut by_site_branch: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for o in &ops {
        let e = by_site_branch.entry(o.site.as_str()).or_default();
        e.1 += 1;
        if uses[o.from.as_str()] > 1 {
            e.0 += 1;
        }
    }
    for (s, (b, n)) in &by_site_branch {
        println!("  {s:<20} {b:>5} of {n:>5} branch");
    }

    // The forms.
    let mut tallies: BTreeMap<String, BTreeMap<String, Tally>> = BTreeMap::new();
    let mut forms_used: BTreeMap<String, BTreeMap<&'static str, u64>> = BTreeMap::new();

    // tree
    {
        let t = tallies.entry("tree".into()).or_default();
        for (o, d) in ops.iter().zip(&deltas) {
            let (f, to) = (&progs[&o.from], &progs[&o.to]);
            let (made, c) = measure(|| {
                let mut p = f.tree.clone();
                let rule_keep: Vec<bool> = d.keep[..f.tree.rules.len()].to_vec();
                let mut k = 0;
                p.rules.retain(|_| {
                    k += 1;
                    rule_keep[k - 1]
                });
                for &a in &d.added {
                    if a < to.tree.rules.len() {
                        // A pass makes a new rule: its lists are new.
                        let r = &to.tree.rules[a];
                        let body: Vec<Literal> = r.body.iter().cloned().collect();
                        let args: Vec<Term> = r.head.args.iter().cloned().collect();
                        p.rules.push(Rule { head: Atom { rel: r.head.rel.clone(), args: args.into(), neg: r.head.neg, gated: r.head.gated }, body: body.into() });
                    }
                }
                p.decls = to.tree.decls.clone();
                p.facts = to.tree.facts.clone();
                p.inputs = to.tree.inputs.clone();
                p.outputs = to.tree.outputs.clone();
                p
            });
            let e = t.entry(o.site.clone()).or_default();
            e.make += c;
            let (_, c) = measure(|| tree_read(&made, &heads[o.to.as_str()].0));
            e.read += c;
            let (_, c) = measure(|| tree_digest(&made));
            e.digest += c;
            drop(made);
        }
    }

    // flat
    {
        let flats: HashMap<&str, Flat> = progs.iter().map(|(d, p)| (d.as_str(), flat_of(&p.items))).collect();
        let t = tallies.entry("flat (copy, edit)".into()).or_default();
        for (o, d) in ops.iter().zip(&deltas) {
            let (made, c) = measure(|| flat_make(&flats[o.from.as_str()], d, &progs[&o.to]));
            let e = t.entry(o.site.clone()).or_default();
            e.make += c;
            let (_, c) = measure(|| flat_read(&made, &heads[o.to.as_str()].1));
            e.read += c;
            let (_, c) = measure(|| flat_digest(&made));
            e.digest += c;
        }
    }

    // the pool: every root's items, interned in setup
    let mut pool0 = Pool::default();
    let ids_of: HashMap<&str, Vec<u32>> = progs.iter().map(|(d, p)| (d.as_str(), p.items.iter().map(|i| pool0.intern(i)).collect())).collect();
    // So that added items are new to the pool when first made, the pool holds the roots' items only.
    let mut pool_roots = Pool::default();
    for r in &roots {
        for i in &progs[*r].items {
            pool_roots.intern(i);
        }
    }

    // pool (copy, edit)
    {
        let mut pool = pool_roots.clone();
        let t = tallies.entry("pool (copy list, edit)".into()).or_default();
        let mut listed: HashMap<&str, Listed> = HashMap::new();
        for (o, d) in ops.iter().zip(&deltas) {
            if !listed.contains_key(o.from.as_str()) {
                // A root, or a program made later than this op reads it: held already, as setup.
                let ids: Vec<u32> = progs[&o.from].items.iter().map(|i| pool.intern(i)).collect();
                let by_head = csr(ids.iter().enumerate().map(|(p, &id)| (pool.head[id as usize], p as u32)));
                listed.insert(o.from.as_str(), Listed { ids, by_head });
            }
            let (made, c) = measure(|| listed_make(&mut pool, &listed[o.from.as_str()], d, &progs[&o.to]));
            let e = t.entry(o.site.clone()).or_default();
            e.make += c;
            let (_, c) = measure(|| listed_read(&pool, &made, &heads[o.to.as_str()].1));
            e.read += c;
            let (_, c) = measure(|| listed_digest(&pool, &made));
            e.digest += c;
            listed.insert(o.to.as_str(), made);
        }
    }

    // the proposal, under policies
    let mut policies = Vec::new();
    for layout in [Layout0::Source, Layout0::Head, Layout0::Oracle] {
        for (fold, max_depth) in [(None, usize::MAX), (Some(0.5), usize::MAX), (Some(0.25), usize::MAX), (Some(0.5), 2)] {
            policies.push(Policy { layout, fold, max_depth, removed: Removed::Sorted, max_ranges: 32 });
        }
    }
    for removed in [Removed::Hash, Removed::Bitmap] {
        policies.push(Policy { layout: Layout0::Oracle, fold: Some(0.5), max_depth: usize::MAX, removed, max_ranges: 32 });
    }
    // Oracle signatures: for each root, which of the ops made from it keep each item.
    let mut sig: HashMap<&str, HashMap<u32, u64>> = HashMap::new();
    let mut nth: HashMap<&str, u32> = HashMap::new();
    for (o, d) in ops.iter().zip(&deltas) {
        if roots.contains(&o.from.as_str()) {
            let n = nth.entry(o.from.as_str()).or_default();
            let bit = 1u64 << (*n).min(63);
            *n += 1;
            let s = sig.entry(o.from.as_str()).or_default();
            for (k, keep) in d.keep.iter().enumerate() {
                if *keep {
                    *s.entry(ids_of[o.from.as_str()][k]).or_default() |= bit;
                }
            }
        }
    }
    for p in &policies {
        let name = format!("v: {:?}, fold {}, depth ≤ {}, removed {:?}", p.layout, p.fold.map_or("never".to_owned(), |f| format!("at {f}")), if p.max_depth == usize::MAX { "any".to_owned() } else { p.max_depth.to_string() }, p.removed);
        let mut pool = pool_roots.clone();
        let mut versions: HashMap<&str, std::rc::Rc<Version>> = HashMap::new();
        let mut order_of: HashMap<&str, Vec<u32>> = HashMap::new();
        for r in &roots {
            let ids: Vec<u32> = progs[*r].items.iter().map(|i| pool.intern(i)).collect();
            let v = base_of(&pool, ids.clone(), p.layout, sig.get(r));
            // The delta's keep mask is over `from`'s items in source order; a base laid out otherwise is given the mask in its own order.
            let mut laid = Vec::new();
            v.each(&mut |i| laid.push(i));
            order_of.insert(r, laid);
            versions.insert(r, std::rc::Rc::new(v));
            let _ = ids;
        }
        let t = tallies.entry(name.clone()).or_default();
        let used = forms_used.entry(name).or_default();
        for (o, d) in ops.iter().zip(&deltas) {
            if !versions.contains_key(o.from.as_str()) {
                let ids: Vec<u32> = progs[&o.from].items.iter().map(|i| pool.intern(i)).collect();
                let v = base_of(&pool, ids, if p.layout == Layout0::Oracle { Layout0::Head } else { p.layout }, None);
                let mut laid = Vec::new();
                v.each(&mut |i| laid.push(i));
                order_of.insert(o.from.as_str(), laid);
                versions.insert(o.from.as_str(), std::rc::Rc::new(v));
            }
            let from = versions[o.from.as_str()].clone();
            // `from`'s items in its version's order, and the keep mask in that order.
            let src_ids = &ids_of[o.from.as_str()];
            let laid = order_of[o.from.as_str()].clone();
            let mut keep_by_id: HashMap<u32, Vec<bool>> = HashMap::new();
            for (k, &id) in src_ids.iter().enumerate() {
                keep_by_id.entry(id).or_default().push(d.keep[k]);
            }
            let keep: Vec<bool> = laid.iter().map(|id| keep_by_id.get_mut(id).and_then(|v| v.pop()).unwrap_or(false)).collect();
            let d2 = Delta { keep, added: d.added.clone() };
            let ((v, form), c) = measure(|| v_make(&mut pool, &from, &laid, &d2, &progs[&o.to], p));
            *used.entry(form).or_default() += 1;
            let e = t.entry(o.site.clone()).or_default();
            e.make += c;
            let (_, c) = measure(|| v_read(&pool, &v, &heads[o.to.as_str()].1));
            e.read += c;
            let (_, c) = measure(|| v.digest());
            e.digest += c;
            let mut l = Vec::new();
            v.each(&mut |i| l.push(i));
            order_of.insert(o.to.as_str(), l);
            versions.insert(o.to.as_str(), std::rc::Rc::new(v));
        }
    }

    // Report: per form, totals over the run, and per site.
    println!("\n{:<62} {:>10} {:>10} {:>9} {:>10} {:>10}", "form", "make Minsn", "make MB", "allocs k", "read Minsn", "digest Minsn");
    for (form, sites) in &tallies {
        let mut tot = Tally::default();
        for e in sites.values() {
            tot.make += e.make;
            tot.read += e.read;
            tot.digest += e.digest;
        }
        println!(
            "{:<62} {:>10.1} {:>10.1} {:>9.1} {:>10.1} {:>10.2}",
            form,
            tot.make.insn as f64 / 1e6,
            tot.make.bytes as f64 / 1e6,
            tot.make.allocs as f64 / 1e3,
            tot.read.insn as f64 / 1e6,
            tot.digest.insn as f64 / 1e6
        );
    }
    println!("\nper site, make Minsn (read Minsn):");
    let sites: Vec<&str> = shape.keys().copied().collect();
    print!("{:<62}", "form");
    for s in &sites {
        print!(" {:>19}", s);
    }
    println!();
    for (form, by) in &tallies {
        print!("{:<62}", form);
        for s in &sites {
            let e = by.get(&s.to_string());
            print!(" {:>19}", e.map_or(String::new(), |e| format!("{:.1} ({:.1})", e.make.insn as f64 / 1e6, e.read.insn as f64 / 1e6)));
        }
        println!();
    }
    println!("\nforms the proposal chose:");
    for (form, used) in &forms_used {
        println!("  {form}: {}", used.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "));
    }

    // A one-rule edit of each root that the split is made from (the stored prepared program).
    let split_roots: Vec<&str> = {
        let mut r: Vec<&str> = ops.iter().filter(|o| o.site.starts_with("split.")).map(|o| o.from.as_str()).filter(|f| roots.contains(f)).collect();
        r.sort();
        r.dedup();
        r
    };
    if let Some(root) = split_roots.iter().max_by_key(|r| progs[**r].items.len()) {
        let p = &progs[*root];
        let k = p.tree.rules.len() / 2;
        let mut changed = p.items[k].clone();
        changed.nodes[0].val ^= 1;
        changed.hash ^= 1;
        let fake = Prog { tree: Program::default(), items: vec![changed] };
        let mut keep = vec![true; p.items.len()];
        keep[k] = false;
        let d = Delta { keep, added: vec![0] };
        println!("\none-rule edit of the prepared program ({} items):", p.items.len());
        let (_, c) = measure(|| {
            let mut t = p.tree.clone();
            t.rules[k] = Rule { head: t.rules[k].head.clone(), body: t.rules[k].body.iter().cloned().collect::<Vec<_>>().into() };
            t
        });
        println!("  {:<40} {:>10.3} Minsn {:>8.3} MB", "tree (clone, replace)", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
        let flat = flat_of(&p.items);
        let (_, c) = measure(|| flat_make(&flat, &d, &fake));
        println!("  {:<40} {:>10.3} Minsn {:>8.3} MB", "flat (copy, edit)", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
        let mut pool = pool_roots.clone();
        let ids: Vec<u32> = p.items.iter().map(|i| pool.intern(i)).collect();
        let listed = Listed { by_head: csr(ids.iter().enumerate().map(|(q, &id)| (pool.head[id as usize], q as u32))), ids: ids.clone() };
        let (_, c) = measure(|| listed_make(&mut pool, &listed, &d, &fake));
        println!("  {:<40} {:>10.3} Minsn {:>8.3} MB", "pool (copy list, edit)", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
        let mut flat2 = flat_of(&p.items);
        let (_, c) = measure(|| {
            flat2.dead[k] = 2;
            let i = &fake.items[0];
            flat2.start.push(flat2.nodes.len() as u32);
            flat2.len.push(i.nodes.len() as u32);
            flat2.nodes.extend_from_slice(&i.nodes);
            flat2.head.push(i.head);
            flat2.hash.push(i.hash);
            flat2.dead.push(0);
        });
        println!("  {:<40} {:>10.3} Minsn {:>8.3} MB  (Vec growth may copy; see the growth test)", "in place (one line of history)", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
        let v = std::rc::Rc::new(base_of(&pool, ids.clone(), Layout0::Head, None));
        let mut laid = Vec::new();
        v.each(&mut |i| laid.push(i));
        let mut keep2 = vec![true; laid.len()];
        if let Some(q) = laid.iter().position(|&x| x == ids[k]) {
            keep2[q] = false;
        }
        let d2 = Delta { keep: keep2, added: vec![0] };
        let pol = Policy { layout: Layout0::Head, fold: Some(0.5), max_depth: usize::MAX, removed: Removed::Sorted, max_ranges: 32 };
        let ((_, form), c) = measure(|| v_make(&mut pool, &v, &laid, &d2, &fake, &pol));
        println!("  {:<40} {:>10.3} Minsn {:>8.3} MB  ({form})", "proposal (overlay)", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
    }

    growth(&progs, &ops);
    stored(dir, &progs, &roots);
}

/// Open question 1: a region grown by realloc (moves), by doubling extents (never moves), or reserved up front.
fn growth(progs: &HashMap<String, Prog>, ops: &[Op]) {
    let mut seq: Vec<Node> = Vec::new();
    let mut seen = HashSet::new();
    for o in ops {
        for d in [&o.from, &o.to] {
            for i in &progs[d].items {
                if seen.insert(i.hash) {
                    seq.extend_from_slice(&i.nodes);
                }
            }
        }
    }
    let chunk = 1024;
    println!("\ngrowth of a run's pool: {} nodes appended in {} appends of {chunk}", seq.len(), seq.len() / chunk);
    // realloc
    let (v, c) = measure(|| {
        let mut v: Vec<Node> = Vec::new();
        for c in seq.chunks(chunk) {
            v.extend_from_slice(c);
        }
        v
    });
    println!("  {:<32} append {:>8.2} Minsn {:>8.1} MB allocated (moves)", "realloc", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
    // extents
    struct Ext {
        shift: u32,
        ext: Vec<Vec<Node>>,
        len: usize,
    }
    impl Ext {
        fn push(&mut self, mut xs: &[Node]) {
            while !xs.is_empty() {
                if self.ext.last().is_none_or(|e| e.len() == e.capacity()) {
                    let k = self.ext.len();
                    self.ext.push(Vec::with_capacity(1 << (self.shift as usize + k)));
                }
                let last = self.ext.last_mut().unwrap();
                let n = (last.capacity() - last.len()).min(xs.len());
                last.extend_from_slice(&xs[..n]);
                xs = &xs[n..];
                self.len += n;
            }
        }
        #[inline]
        fn get(&self, i: usize) -> Node {
            // extent k holds [(2^k − 1)·first, (2^(k+1) − 1)·first), first = 2^shift
            let q = (i >> self.shift) + 1;
            let k = (usize::BITS - 1 - q.leading_zeros()) as usize;
            let base = ((1usize << k) - 1) << self.shift;
            self.ext[k][i - base]
        }
    }
    let (e, c) = measure(|| {
        let mut e = Ext { shift: 12, ext: Vec::new(), len: 0 };
        for c in seq.chunks(chunk) {
            e.push(c);
        }
        e
    });
    println!("  {:<32} append {:>8.2} Minsn {:>8.1} MB allocated ({} extents, never moved)", "doubling extents", c.insn as f64 / 1e6, c.bytes as f64 / 1e6, e.ext.len());
    let (r, c) = measure(|| {
        let mut v: Vec<Node> = Vec::with_capacity(seq.len());
        for c in seq.chunks(chunk) {
            v.extend_from_slice(c);
        }
        v
    });
    println!("  {:<32} append {:>8.2} Minsn {:>8.1} MB allocated (capacity known in advance)", "reserved", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
    // reads: a scan, and random access
    let n = seq.len();
    let probes: Vec<usize> = (0..1_000_000u64).map(|i| (i.wrapping_mul(0x9E3779B97F4A7C15) >> 11) as usize % n).collect();
    let (_, c1) = measure(|| visit(&v));
    let (_, c2) = measure(|| e.ext.iter().map(|x| visit(x)).sum::<u64>());
    let (_, c3) = measure(|| probes.iter().map(|&i| v[i].val as u64).sum::<u64>());
    let (_, c4) = measure(|| probes.iter().map(|&i| e.get(i).val as u64).sum::<u64>());
    println!("  scan: one array {:.2} Minsn, extents {:.2} Minsn;  1 M random reads: one array {:.2} Minsn, extents {:.2} Minsn", c1.insn as f64 / 1e6, c2.insn as f64 / 1e6, c3.insn as f64 / 1e6, c4.insn as f64 / 1e6);
    drop((r, e.len));
}

/// Open question 2: an image's bytes in SQLite (a blob, read by copying) or in a file beside it (mapped).
fn stored(dir: &Path, progs: &HashMap<String, Prog>, roots: &[&str]) {
    let Some(root) = roots.iter().max_by_key(|r| progs[**r].items.len()) else { return };
    let flat = flat_of(&progs[*root].items);
    stored_one(dir, "the largest root", node_bytes(&flat.nodes).to_vec());
    // The run's whole pool, every distinct item once; and sixteen of it, as a larger pack's would be.
    let mut seen = HashSet::new();
    let mut all: Vec<Node> = Vec::new();
    for p in progs.values() {
        for i in &p.items {
            if seen.insert(i.hash) {
                all.extend_from_slice(&i.nodes);
            }
        }
    }
    let one = node_bytes(&all).to_vec();
    stored_one(dir, "the run's pool", one.clone());
    stored_one(dir, "16 × the run's pool", one.repeat(16));
}

fn stored_one(dir: &Path, what: &str, bytes: Vec<u8>) {
    let flat = Flat { nodes: unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<Node>(), bytes.len() / std::mem::size_of::<Node>()) }.to_vec(), start: vec![], len: vec![], head: vec![], hash: vec![], dead: vec![], by_head: csr(std::iter::empty()) };
    let db_path = dir.join("images.sqlite");
    let file_path = dir.join("image.bin");
    let _ = std::fs::remove_file(&db_path);
    let db = rusqlite::Connection::open(&db_path).unwrap();
    db.execute_batch("CREATE TABLE image(id INTEGER PRIMARY KEY, data BLOB)").unwrap();
    db.execute("INSERT INTO image VALUES (1, ?1)", [&bytes]).unwrap();
    std::fs::write(&file_path, &bytes).unwrap();
    println!("\nstored image: {what}, {} nodes, {:.1} MB", flat.nodes.len(), bytes.len() as f64 / 1e6);
    let probe = |nodes: &[Node]| -> u64 { (0..64).map(|k| nodes[(k * 7919) % nodes.len()].val as u64).sum() };
    for round in ["first", "again"] {
        let (_, c) = measure(|| {
            let data: Vec<u8> = db.query_row("SELECT data FROM image WHERE id = 1", [], |r| r.get(0)).unwrap();
            let nodes: &[Node] = unsafe { std::slice::from_raw_parts(data.as_ptr().cast(), data.len() / std::mem::size_of::<Node>()) };
            probe(nodes)
        });
        println!("  {round:<6} blob, open and read 64 nodes    {:>8.2} Minsn {:>8.1} MB copied", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
        let (_, c) = measure(|| {
            let f = std::fs::File::open(&file_path).unwrap();
            let len = f.metadata().unwrap().len() as usize;
            use std::os::fd::AsRawFd;
            let p = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, f.as_raw_fd(), 0) };
            let nodes: &[Node] = unsafe { std::slice::from_raw_parts(p.cast(), len / std::mem::size_of::<Node>()) };
            let s = probe(nodes);
            unsafe { libc::munmap(p, len) };
            s
        });
        println!("  {round:<6} mapped file, open and read 64 nodes {:>8.2} Minsn {:>8.1} MB copied", c.insn as f64 / 1e6, c.bytes as f64 / 1e6);
    }
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(&file_path);
}
