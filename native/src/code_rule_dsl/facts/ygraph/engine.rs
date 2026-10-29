//! The maintenance core: keyed global tables (X5's class G base tables) and memoised keyed
//! queries (G derived and R recursive stages, and the per-name link core), each query recording
//! the table keys it probes (absent keys included, by hash) and the queries it reads.
//!
//! State is a shared `Base` plus one `Delta` per worktree. A query's base entry is valid in a
//! worktree until something it read differs there; then it is re-run into the delta. After an
//! edit the changed keys' readers are re-run in level order (every query reads only queries of
//! lower level), and a re-run whose value is unchanged stops there (early cutoff). That is X5's
//! rule — affected outputs are the forward closure of the changed keys, recomputed from the
//! unaffected boundary — with the read sets recorded by the queries as they run, not derived by
//! hand per stage.
use super::intern::{hash_str, Interner, Sym};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BinaryHeap;
use std::rc::Rc;

pub type Row = [u32; 4];
pub type QId = u32;

pub const T_BYNAME: u8 = 0; // key -> (path, idx, flags, name)
pub const T_ALIAS: u8 = 1; // key -> (path, target)
pub const T_ALIAS_T: u8 = 2; // target -> (path, key)
pub const T_OPAQUE: u8 = 3; // key -> ()
pub const T_MODULES: u8 = 4; // name -> (path)
pub const T_SUPERS: u8 = 5; // cls -> (path, target, ord)
pub const T_SUPERS_T: u8 = 6; // target -> (path, cls, ord)
pub const T_SUPERS_S: u8 = 7; // short of a `<base>` target -> (path, target)
pub const T_CLASSSHORT: u8 = 8; // short -> (class def name) counted
pub const T_MARKER: u8 = 9; // cls -> () counted
pub const T_BYNAME_D: u8 = 10; // def name -> (path, key)
pub const T_EDGESRC: u8 = 11; // edge name -> (path, idx)
pub const NT: usize = 12;

/// The dependency key of a probe of table `t` at key text `s` (by hash: absent keys need no id).
#[inline]
pub fn dep_key(t: u8, h: u64) -> u64 {
    h ^ (t as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

type Rows = Vec<(Row, u32)>;

/// A frozen layer's tables. In an overlay layer an empty key is a tombstone (the key has no
/// rows here, whatever the layers below hold).
#[derive(Default)]
pub struct BaseTables {
    pub t: [FxHashMap<Sym, Box<[(Row, u32)]>>; NT],
}

#[derive(Default, Clone)]
pub struct DeltaTables {
    pub t: [FxHashMap<Sym, Rows>; NT],
}

/// The frozen layers under a delta, top first (a worktree's cluster, then the base).
pub fn below_rows<'a>(below: &[&'a BaseTables], t: u8, k: Sym) -> &'a [(Row, u32)] {
    for b in below {
        if let Some(r) = b.t[t as usize].get(&k) {
            return r;
        }
    }
    &[]
}

pub struct TView<'a> {
    pub below: Vec<&'a BaseTables>,
    pub delta: &'a DeltaTables,
}

impl<'a> TView<'a> {
    #[inline]
    pub fn rows(&self, t: u8, k: Sym) -> &'a [(Row, u32)] {
        match self.delta.t[t as usize].get(&k) {
            Some(r) => r,
            None => below_rows(&self.below, t, k),
        }
    }
}

/// Table edits of one event, with the before-rows of every touched key.
#[derive(Default)]
pub struct Touch {
    pub before: FxHashMap<(u8, Sym), Rows>,
}

impl DeltaTables {
    pub fn add(&mut self, below: &[&BaseTables], touch: &mut Touch, t: u8, k: Sym, row: Row, by: i32) {
        let m = &mut self.t[t as usize];
        let rows = m.entry(k).or_insert_with(|| below_rows(below, t, k).to_vec());
        touch.before.entry((t, k)).or_insert_with(|| rows.clone());
        match rows.binary_search_by(|x| x.0.cmp(&row)) {
            Ok(i) => {
                let c = rows[i].1 as i64 + by as i64;
                assert!(c >= 0, "table {t} key {k}: negative count");
                if c == 0 {
                    rows.remove(i);
                } else {
                    rows[i].1 = c as u32;
                }
            }
            Err(i) => {
                assert!(by > 0, "table {t} key {k}: removing an absent row");
                rows.insert(i, (row, by as u32));
            }
        }
    }

    pub fn bytes(&self) -> usize {
        self.t.iter().map(|m| m.capacity() * 32 + m.values().map(|r| r.capacity() * 20).sum::<usize>()).sum()
    }
}

impl BaseTables {
    pub fn bytes(&self) -> usize {
        self.t.iter().map(|m| m.capacity() * 24 + m.values().map(|r| r.len() * 20).sum::<usize>()).sum()
    }
    pub fn rows(&self) -> usize {
        self.t.iter().map(|m| m.values().map(|r| r.len()).sum::<usize>()).sum()
    }
}

// ------------------------------------------------------------------------------------- queries

pub const K_BASECLASS: u8 = 0;
pub const K_FQ: u8 = 1;
pub const K_BASES: u8 = 2;
pub const K_BASETARGETS: u8 = 3;
pub const K_TYPEKEY: u8 = 4;
pub const K_CLASSOF: u8 = 5;
pub const K_CLASSROOT: u8 = 6;
pub const K_SUBCLASS: u8 = 7;
pub const K_ORDER: u8 = 8;
pub const K_OWNREF: u8 = 9;
pub const K_MF0: u8 = 10;
pub const K_MF1: u8 = 11;
pub const K_FIELD: u8 = 12;
pub const K_REFSUP: u8 = 13;
pub const K_COMPOSE: u8 = 14;
pub const K_BODY: u8 = 15;
pub const K_LINK: u8 = 16;
pub const NK: usize = 17;
pub const KIND_NAMES: [&str; NK] = [
    "base_class", "fq_class", "bases", "base_targets", "type_key", "class_of", "class_root", "subclass", "reference_order",
    "own_name_refusal", "method_first", "method_first_super", "field_class", "supers_refusal", "compose_first", "link_body", "link_result",
];

/// Max hops a link is evaluated at: every term and kid range ends at 99 or below.
pub const HMAX: u32 = 99;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Q {
    pub kind: u8,
    pub a: u32,
    pub b: u32,
}

impl Q {
    pub fn level(&self) -> u32 {
        match self.kind {
            K_BASECLASS | K_FQ => 1,
            K_BASES | K_BASETARGETS | K_TYPEKEY | K_CLASSOF => 2,
            K_CLASSROOT | K_SUBCLASS | K_OWNREF => 3,
            K_ORDER => 4,
            K_MF0 | K_MF1 | K_FIELD | K_REFSUP => 5,
            K_COMPOSE => 6,
            K_BODY => 7,
            K_LINK => 8 + (HMAX - self.b),
            _ => unreachable!(),
        }
    }
}

#[derive(Default)]
pub struct QTable {
    pub ids: FxHashMap<Q, QId>,
    pub qs: Vec<Q>,
}

impl QTable {
    pub fn id(&mut self, q: Q) -> QId {
        if let Some(&i) = self.ids.get(&q) {
            return i;
        }
        let i = self.qs.len() as QId;
        self.qs.push(q);
        self.ids.insert(q, i);
        i
    }
    pub fn bytes(&self) -> usize {
        self.ids.capacity() * 17 + self.qs.capacity() * 12
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Body {
    /// (def, lo, hi)
    pub terms: Vec<(u32, i32, i32)>,
    /// (kid name, delta, checked, lo, hi)
    pub kids: Vec<(Sym, u8, bool, i32, i32)>,
    /// form 2/5/6 whose class part is not a class_root (a refusal when the name is an edge's)
    pub root_refusal: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum V {
    /// base_class: None = not a `supers` target; else (class, number of first-rank classes)
    BaseClass(Option<(Option<Sym>, u32)>),
    Syms(Vec<Sym>),
    SymK(Vec<(Sym, i32)>),
    /// bases: (ord, base)
    Pairs(Vec<(Sym, Sym)>),
    /// base_targets: (target, several)
    Targets(Vec<(Sym, u32)>),
    /// type_key: (min class, min k, distinct classes)
    TypeKey(Option<(Sym, i32, u32)>),
    Bool(bool),
    /// reference_order: (class, dist, seq as an interned string), depth ≥ 64 reached
    Order(Vec<(Sym, u8, Sym)>, bool),
    /// field_class: (member class, lo, hi)
    Field(Vec<(Sym, i32, i32)>),
    Body(Box<Body>),
    Link(Box<[u32]>),
}

impl V {
    /// Contribution to the refusal count when the query is registered as a refusal root.
    pub fn refuses(&self) -> bool {
        match self {
            V::Bool(b) => *b,
            V::TypeKey(Some((_, _, several))) => *several > 1,
            V::Body(b) => b.root_refusal,
            _ => false,
        }
    }

    /// The value as words (memos store values this way: one allocation per memo).
    pub fn encode(&self, out: &mut Vec<u32>) {
        match self {
            V::BaseClass(None) => out.push(0),
            V::BaseClass(Some((None, s))) => out.extend([1, *s]),
            V::BaseClass(Some((Some(c), s))) => out.extend([2, *c, *s]),
            V::Syms(v) => out.extend_from_slice(v),
            V::SymK(v) => v.iter().for_each(|&(s, k)| out.extend([s, k as u32])),
            V::Pairs(v) => v.iter().for_each(|&(a, b)| out.extend([a, b])),
            V::Targets(v) => v.iter().for_each(|&(a, b)| out.extend([a, b])),
            V::TypeKey(None) => {}
            V::TypeKey(Some((c, k, s))) => out.extend([*c, *k as u32, *s]),
            V::Bool(b) => out.push(*b as u32),
            V::Order(v, deep) => {
                out.push(*deep as u32);
                v.iter().for_each(|&(c, d, s)| out.extend([c, d as u32, s]));
            }
            V::Field(v) => v.iter().for_each(|&(c, lo, hi)| out.extend([c, lo as u32, hi as u32])),
            V::Body(b) => {
                out.extend([b.root_refusal as u32, b.terms.len() as u32]);
                b.terms.iter().for_each(|&(d, lo, hi)| out.extend([d, lo as u32, hi as u32]));
                b.kids.iter().for_each(|&(k, delta, chk, lo, hi)| out.extend([k, delta as u32 | (chk as u32) << 8, lo as u32, hi as u32]));
            }
            V::Link(v) => out.extend_from_slice(v),
        }
    }

    pub fn decode(kind: u8, w: &[u32]) -> V {
        let pairs = |w: &[u32]| -> Vec<(u32, u32)> { w.chunks_exact(2).map(|c| (c[0], c[1])).collect() };
        match kind {
            K_BASECLASS => V::BaseClass(match w[0] {
                0 => None,
                1 => Some((None, w[1])),
                _ => Some((Some(w[1]), w[2])),
            }),
            K_FQ | K_CLASSOF => V::SymK(w.chunks_exact(2).map(|c| (c[0], c[1] as i32)).collect()),
            K_BASES => V::Pairs(pairs(w)),
            K_BASETARGETS => V::Targets(pairs(w)),
            K_TYPEKEY => V::TypeKey((!w.is_empty()).then(|| (w[0], w[1] as i32, w[2]))),
            K_CLASSROOT | K_OWNREF | K_REFSUP => V::Bool(w[0] != 0),
            K_SUBCLASS | K_MF0 | K_MF1 | K_COMPOSE => V::Syms(w.to_vec()),
            K_ORDER => V::Order(w[1..].chunks_exact(3).map(|c| (c[0], c[1] as u8, c[2])).collect(), w[0] != 0),
            K_FIELD => V::Field(w.chunks_exact(3).map(|c| (c[0], c[1] as i32, c[2] as i32)).collect()),
            K_BODY => {
                let nt = w[1] as usize;
                let terms = w[2..2 + 3 * nt].chunks_exact(3).map(|c| (c[0], c[1] as i32, c[2] as i32)).collect();
                let kids = w[2 + 3 * nt..].chunks_exact(4).map(|c| (c[0], (c[1] & 0xff) as u8, c[1] >> 8 != 0, c[2] as i32, c[3] as i32)).collect();
                V::Body(Box::new(Body { terms, kids, root_refusal: w[0] != 0 }))
            }
            K_LINK => V::Link(w.into()),
            _ => unreachable!(),
        }
    }
}

/// One memo in one allocation: [n_ins | n_qs << 16, ins as (lo, hi) word pairs, qs, value].
#[derive(Clone, PartialEq, Eq)]
pub struct Memo {
    data: Box<[u32]>,
}

impl Memo {
    pub fn new(ins: &[u64], qs: &[QId], val: &[u32]) -> Memo {
        assert!(ins.len() < 1 << 16 && qs.len() < 1 << 16, "memo read set over 65535");
        let mut d = Vec::with_capacity(1 + 2 * ins.len() + qs.len() + val.len());
        d.push(ins.len() as u32 | (qs.len() as u32) << 16);
        for &k in ins {
            d.extend([k as u32, (k >> 32) as u32]);
        }
        d.extend_from_slice(qs);
        d.extend_from_slice(val);
        Memo { data: d.into_boxed_slice() }
    }
    fn n(&self) -> (usize, usize) {
        ((self.data[0] & 0xffff) as usize, (self.data[0] >> 16) as usize)
    }
    pub fn ins(&self) -> impl Iterator<Item = u64> + '_ {
        let (ni, _) = self.n();
        self.data[1..1 + 2 * ni].chunks_exact(2).map(|c| c[0] as u64 | (c[1] as u64) << 32)
    }
    /// The read set (ins and qs) as stored, for equality.
    pub fn reads(&self) -> &[u32] {
        let (ni, nq) = self.n();
        &self.data[..1 + 2 * ni + nq]
    }
    pub fn qs(&self) -> &[QId] {
        let (ni, nq) = self.n();
        &self.data[1 + 2 * ni..1 + 2 * ni + nq]
    }
    pub fn val(&self) -> &[u32] {
        let (ni, nq) = self.n();
        &self.data[1 + 2 * ni + nq..]
    }
    pub fn bytes(&self) -> usize {
        16 + self.data.len() * 4
    }
}

/// Map entry bytes of a memo map (key, boxed slice, control byte).
const MEMO_ENTRY: usize = 4 + 16 + 1;

/// A frozen layer's memos by id, and its reverse indexes.
#[derive(Default)]
pub struct BaseMemos {
    pub memos: FxHashMap<QId, Memo>,
    pub readers: FxHashMap<u64, Box<[QId]>>,
    pub parents: FxHashMap<QId, Box<[QId]>>,
}

#[derive(Default, Clone)]
pub struct DeltaMemos {
    pub memos: FxHashMap<QId, Memo>,
    pub readers: FxHashMap<u64, Vec<QId>>,
    pub parents: FxHashMap<QId, Vec<QId>>,
}

impl DeltaMemos {
    pub fn freeze(self) -> BaseMemos {
        BaseMemos {
            memos: self.memos,
            readers: self.readers.into_iter().map(|(k, mut v)| {
                v.sort_unstable();
                v.dedup();
                (k, v.into_boxed_slice())
            }).collect(),
            parents: self.parents.into_iter().map(|(k, mut v)| {
                v.sort_unstable();
                v.dedup();
                (k, v.into_boxed_slice())
            }).collect(),
        }
    }
    pub fn bytes(&self) -> usize {
        self.memos.capacity() * MEMO_ENTRY + self.memos.values().map(Memo::bytes).sum::<usize>()
            + self.readers.capacity() * 32 + self.readers.values().map(|v| v.capacity() * 4).sum::<usize>()
            + self.parents.capacity() * 28 + self.parents.values().map(|v| v.capacity() * 4).sum::<usize>()
    }
}

impl BaseMemos {
    pub fn bytes(&self) -> usize {
        self.memos.capacity() * MEMO_ENTRY + self.memos.values().map(Memo::bytes).sum::<usize>()
            + self.readers.capacity() * 24 + self.readers.values().map(|v| v.len() * 4).sum::<usize>()
            + self.parents.capacity() * 20 + self.parents.values().map(|v| v.len() * 4).sum::<usize>()
    }
}

/// Counts of one maintenance run.
#[derive(Default, Debug, Clone, Copy, serde::Serialize)]
pub struct Work {
    pub executed: u64,
    pub fresh: u64,
    pub rerun: u64,
    pub changed: u64,
    pub probes: u64,
    pub dirty_seeds: u64,
    pub link_nodes: u64,
    /// re-runs equal to the base entry (dropped from the delta)
    pub backdated: u64,
}

/// Semantic anomalies the SQL could disagree with native on; all must stay 0 (reported).
#[derive(Default, Debug, Clone, Copy, serde::Serialize)]
pub struct Anomalies {
    /// a term or kid range with lo < 0 <= hi (the SQL's `lo = 0` would miss it)
    pub negative_lo: u64,
    /// a non-empty link at hops ≥ 19 (beyond the SQL's link_reach cut)
    pub deep_nonempty: u64,
    /// a class's base ordinals of different lengths (BFS seq order by string)
    pub ord_lengths: u64,
}

struct Frame {
    ins: Vec<u64>,
    qs: Vec<QId>,
}

/// Evaluation context over one worktree (or the base under construction).
pub struct Ctx<'a> {
    pub tv: TView<'a>,
    pub it: &'a mut Interner,
    pub qt: &'a mut QTable,
    pub defs: &'a mut super::world::DefTable,
    /// the frozen layers' memos, top first
    pub base: Vec<&'a BaseMemos>,
    pub delta: &'a mut DeltaMemos,
    frames: Vec<Frame>,
    pub work: Work,
    pub anomalies: &'a mut Anomalies,
    /// queries whose value changed in this run (re-run with a different value)
    pub changed: Vec<QId>,
}

impl<'a> Ctx<'a> {
    pub fn new(
        tv: TView<'a>,
        it: &'a mut Interner,
        qt: &'a mut QTable,
        defs: &'a mut super::world::DefTable,
        base: Vec<&'a BaseMemos>,
        delta: &'a mut DeltaMemos,
        anomalies: &'a mut Anomalies,
    ) -> Self {
        Ctx { tv, it, qt, defs, base, delta, frames: Vec::new(), work: Work::default(), anomalies, changed: Vec::new() }
    }

    #[inline]
    fn note_in(&mut self, k: u64) {
        self.work.probes += 1;
        if let Some(f) = self.frames.last_mut() {
            f.ins.push(k);
        }
    }

    /// Rows of table `t` at key text `s` (recorded as a probe, present or not).
    pub fn rows(&mut self, t: u8, s: &str) -> &'a [(Row, u32)] {
        let h = hash_str(s);
        self.note_in(dep_key(t, h));
        match self.it.get_h(s, h) {
            Some(k) => self.tv.rows(t, k),
            None => &[],
        }
    }

    pub fn rows_sym(&mut self, t: u8, k: Sym) -> &'a [(Row, u32)] {
        let h = self.it.hash_of(k);
        self.note_in(dep_key(t, h));
        self.tv.rows(t, k)
    }

    fn below_memo(&self, q: QId) -> Option<&'a Memo> {
        self.base.iter().find_map(|b| b.memos.get(&q))
    }

    pub fn memo(&self, q: QId) -> Option<&Memo> {
        self.delta.memos.get(&q).or_else(|| self.below_memo(q))
    }

    /// The value of query `q` (computed on first demand), recorded as a read of the caller.
    pub fn get(&mut self, q: Q) -> Rc<V> {
        let id = self.qt.id(q);
        if let Some(f) = self.frames.last_mut() {
            f.qs.push(id);
        }
        if let Some(m) = self.memo(id) {
            return Rc::new(V::decode(q.kind, m.val()));
        }
        self.work.fresh += 1;
        self.run(id, q)
    }

    fn run(&mut self, id: QId, q: Q) -> Rc<V> {
        self.work.executed += 1;
        if q.kind == K_LINK {
            self.work.link_nodes += 1;
        }
        self.frames.push(Frame { ins: Vec::new(), qs: Vec::new() });
        let v = super::link::exec(self, q);
        let mut f = self.frames.pop().unwrap();
        f.ins.sort_unstable();
        f.ins.dedup();
        f.qs.sort_unstable();
        f.qs.dedup();
        let mut words = Vec::new();
        v.encode(&mut words);
        let m = Memo::new(&f.ins, &f.qs, &words);
        // A re-run that reproduces the entry below (value and reads) leaves nothing in the delta:
        // that entry is valid here again, and its reads are in the reverse index below.
        if self.below_memo(id).is_some_and(|b| *b == m) {
            self.delta.memos.remove(&id);
            self.work.backdated += 1;
            return Rc::new(v);
        }
        // A re-run reading what it read before is already in a reverse index (below or delta).
        let same = self.memo(id).is_some_and(|old| old.reads() == m.reads());
        if !same {
            for &k in &f.ins {
                self.delta.readers.entry(k).or_default().push(id);
            }
            for &c in &f.qs {
                self.delta.parents.entry(c).or_default().push(id);
            }
        }
        self.delta.memos.insert(id, m);
        Rc::new(v)
    }

    /// Re-run every reader of the changed keys, in level order, with early cutoff.
    pub fn propagate(&mut self, changed_keys: &[u64]) {
        let mut heap: BinaryHeap<std::cmp::Reverse<(u32, QId)>> = BinaryHeap::new();
        let mut queued: FxHashSet<QId> = FxHashSet::default();
        let push = |heap: &mut BinaryHeap<_>, queued: &mut FxHashSet<QId>, qt: &QTable, q: QId| {
            if queued.insert(q) {
                heap.push(std::cmp::Reverse((qt.qs[q as usize].level(), q)));
            }
        };
        for k in changed_keys {
            let mut rs: Vec<QId> = self.delta.readers.get(k).cloned().unwrap_or_default();
            for b in &self.base {
                rs.extend(b.readers.get(k).map_or(&[][..], |v| v));
            }
            for q in rs {
                self.work.dirty_seeds += 1;
                push(&mut heap, &mut queued, self.qt, q);
            }
        }
        while let Some(std::cmp::Reverse((_, id))) = heap.pop() {
            let Some(old) = self.memo(id).map(|m| m.val().to_vec()) else { continue };
            let q = self.qt.qs[id as usize];
            self.work.rerun += 1;
            self.run(id, q);
            if self.memo(id).map(Memo::val) != Some(&old[..]) {
                self.work.changed += 1;
                self.changed.push(id);
                let mut ps: Vec<QId> = self.delta.parents.get(&id).cloned().unwrap_or_default();
                for b in &self.base {
                    ps.extend(b.parents.get(&id).map_or(&[][..], |v| v));
                }
                for p in ps {
                    push(&mut heap, &mut queued, self.qt, p);
                }
            }
        }
    }
}
