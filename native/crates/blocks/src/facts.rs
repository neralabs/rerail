//! A `Facts` block: a phase's relations (or a run's outputs: the same form), in stratum order. Each stratum's
//! structures are one range of the block, because they are all written while that stratum runs. A range moves
//! whole by `memcpy`.
//!
//! A `Facts` block is full, or a delta of a full one (its `base`). A delta is cumulative: the next run carries
//! it forward into a new delta of the same base, and a roll-up writes a new full block.

use crate::block::{Block, Frozen, kind};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering::Relaxed};
use crate::refs::{Ptr, Span};
use crate::relation::{self, FreezeStats, RelW, Relation, Spec, View};
use crate::structure;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug)]
pub struct RelDef {
    pub name: String,
    pub stratum: usize,
    pub spec: Spec,
}

/// The relations a block holds: by number, each in its stratum. The program owns it; blocks hold numbers.
#[derive(Debug)]
pub struct Schema {
    pub rels: Vec<RelDef>,
    pub nstrata: usize,
}

impl Schema {
    pub fn id(&self, name: &str) -> usize {
        self.rels.iter().position(|r| r.name == name).unwrap_or_else(|| panic!("no relation {name}"))
    }

    pub fn in_stratum(&self, s: usize) -> impl Iterator<Item = usize> + '_ {
        self.rels.iter().enumerate().filter(move |(_, r)| r.stratum == s).map(|(i, _)| i)
    }
}

structure! {
    pub struct Range { pub start: AtomicU64, pub end: AtomicU64 }
}

structure! {
    pub struct FactsRoot {
        pub phase: u32,
        pub nrel: u32,
        pub rels: Span<Ptr<Relation>>,
        pub strata: Span<Range>,
        /// A chunk's documents (empty in a phase's facts).
        pub docs: Span<crate::chunk::Doc>,
        /// A chunk's memo entries copied in, `nhits` of them used.
        pub hits: Span<Ptr<crate::chunk::MemoEntry>>,
        pub nhits: AtomicU32,
        pub pad: u32,
    }
}

/// An open `Facts` block: one run writes it.
pub struct Facts {
    pub block: Block,
    pub schema: Arc<Schema>,
    locks: Vec<Mutex<()>>,
    pub base: Option<FrozenFacts>,
}

/// A frozen `Facts` block, and the full block it is a delta of.
#[derive(Clone)]
pub struct FrozenFacts {
    pub block: Frozen,
    pub schema: Arc<Schema>,
    pub base: Option<Box<FrozenFacts>>,
}

pub fn root_of(b: &Block) -> &FactsRoot {
    b.root()
}

fn rel_in<'b>(b: &'b Block, r: usize) -> Option<&'b Relation> {
    root_of(b).rels.as_slice()[r].get()
}

impl Range {
    pub fn get(&self) -> (usize, usize) {
        (self.start.load(Relaxed) as usize, self.end.load(Relaxed) as usize)
    }

    pub fn set(&self, start: usize, end: usize) {
        self.start.store(start as u64, Relaxed);
        self.end.store(end as u64, Relaxed);
    }
}

impl Facts {
    /// A new block: full, or a delta of `base` (which must be full).
    pub fn open(kind: u32, phase: u32, schema: Arc<Schema>, reserve: usize, base: Option<FrozenFacts>) -> Facts {
        if let Some(b) = &base {
            assert!(b.base.is_none(), "a delta names a full block, never another delta");
        }
        let block = crate::alloc::host(kind, reserve);
        let root: *mut FactsRoot = block.alloc_t(1);
        let rels: *mut Ptr<Relation> = block.alloc_t(schema.rels.len());
        let strata: *mut crate::facts::Range = block.alloc_t(schema.nstrata);
        unsafe {
            (*root).phase = phase;
            (*root).nrel = schema.rels.len() as u32;
            Span::set(&raw mut (*root).rels, rels, schema.rels.len());
            Span::set(&raw mut (*root).strata, strata, schema.nstrata);
            if base.is_some() {
                block.header_mut().delta = 1;
            }
        }
        block.set_root(root);
        let locks = (0..schema.rels.len()).map(|_| Mutex::new(())).collect();
        Facts { block, schema, locks, base }
    }

    pub fn full(phase: u32, schema: Arc<Schema>, reserve: usize) -> Facts {
        Facts::open(kind::FACTS, phase, schema, reserve, None)
    }

    pub fn root(&self) -> &FactsRoot {
        root_of(&self.block)
    }

    fn strata(&self) -> &[crate::facts::Range] {
        self.root().strata.as_slice()
    }

    pub fn begin(&self, s: usize) {
        self.strata()[s].start.store(self.block.mark(64) as u64, Relaxed);
    }

    pub fn end(&self, s: usize) {
        self.strata()[s].end.store(self.block.used() as u64, Relaxed);
    }

    fn base_rel(&self, r: usize) -> Option<&Relation> {
        self.base.as_ref().map(|b| b.rel(r))
    }

    /// Makes relation `r` in the current stratum: a delta of the base's `r` when this block is a delta.
    pub fn create(&self, r: usize) -> &Relation {
        assert!(self.rel(r).is_none(), "{} made twice", self.schema.rels[r].name);
        let rel = Relation::create(&self.block, &self.schema.rels[r].spec, self.base_rel(r));
        let slot = &self.root().rels.as_slice()[r] as *const Ptr<Relation> as *mut Ptr<Relation>;
        unsafe { Ptr::set(slot, rel) };
        rel
    }

    pub fn rel(&self, r: usize) -> Option<&Relation> {
        rel_in(&self.block, r)
    }

    /// Relation `r` as readers see it: this block's, over its base's.
    pub fn view(&self, r: usize) -> View<'_> {
        View { base: self.base_rel(r), top: self.rel(r).unwrap_or_else(|| panic!("{} not made", self.schema.rels[r].name)) }
    }

    pub fn w(&self, r: usize) -> RelW<'_> {
        RelW { block: &self.block, rel: self.rel(r).expect("made"), lock: &self.locks[r] }
    }

    pub fn freeze_rel(&self, r: usize) -> FreezeStats {
        relation::freeze(&self.block, self.rel(r).expect("made"), &self.schema.rels[r].spec, self.base_rel(r))
    }

    /// Copies stratum `s` of `src` in whole: one `memcpy`, every reference inside it still valid. `src` is this
    /// block's base, or a delta of the same base (a stratum this run leaves as it was).
    pub fn copy_stratum(&self, src: &FrozenFacts, s: usize) -> usize {
        let range = &root_of(&src.block).strata.as_slice()[s];
        let (start, end) = range.get();
        let dst = self.block.copy_in(src.block.bytes(start, end), start);
        for r in self.schema.in_stratum(s) {
            let off = src.block.offset_of(src.rel(r));
            let rel: *const Relation = self.block.ptr(dst + off - start);
            unsafe { Ptr::set(&self.root().rels.as_slice()[r], rel) };
        }
        self.strata()[s].set(dst, dst + end - start);
        self.add_cost(end - start);
        end - start
    }

    /// Makes relation `r` by carrying `old`'s (a delta of the same base) forward, open for this run's changes:
    /// its rows pushed again in id order and its kills repeated, so every id stays what it was.
    pub fn carry(&self, old: &FrozenFacts, r: usize) -> usize {
        let rel = self.create(r);
        let o = old.rel(r);
        assert_eq!(o.base_rows, rel.base_rows);
        let w = self.w(r);
        let a = o.arity as usize;
        let mut flat = Vec::with_capacity(o.rows.len() as usize * a);
        for i in 0..o.rows.len() {
            for c in 0..a {
                flat.push(o.rows.get(i, c));
            }
        }
        w.extend(&flat);
        for id in 0..o.base_rows + o.rows.len() {
            if o.dead_bit(id) {
                w.kill(id);
            }
        }
        let bytes = o.rows.value_bytes() + o.dead.len() * 8;
        self.add_cost(bytes);
        bytes
    }

    fn add_cost(&self, bytes: usize) {
        self.block.header().cost.fetch_add(bytes as u64, Relaxed);
    }

    /// Readers' extra work because this is a delta, in bytes, for the roll-up rule.
    pub fn charge(&self, bytes: usize) {
        self.add_cost(bytes)
    }

    pub fn freeze(self) -> FrozenFacts {
        for (r, d) in self.schema.rels.iter().enumerate() {
            if let Some(rel) = self.rel(r) {
                assert!(rel.is_frozen(), "{} still open at the block's freeze", d.name);
            }
        }
        FrozenFacts { block: self.block.freeze(), schema: self.schema, base: self.base.map(Box::new) }
    }
}

impl FrozenFacts {
    pub fn rel(&self, r: usize) -> &Relation {
        rel_in(&self.block, r).unwrap_or_else(|| panic!("{} not in the block", self.schema.rels[r].name))
    }

    pub fn view(&self, r: usize) -> View<'_> {
        View { base: self.base.as_ref().map(|b| b.rel(r)), top: self.rel(r) }
    }

    pub fn is_delta(&self) -> bool {
        self.base.is_some()
    }

    /// Whether stratum `s` of this delta changes its base: rows added or killed.
    pub fn changed(&self, s: usize) -> bool {
        self.schema.in_stratum(s).any(|r| {
            let rel = self.rel(r);
            !rel.rows.is_empty() || !rel.killed.is_empty()
        })
    }

    /// What carrying this delta has cost so far (bytes), against what a roll-up costs: the base's bytes.
    pub fn cost(&self) -> (u64, u64) {
        (self.block.header().cost.load(Relaxed), self.base.as_ref().map_or(0, |b| b.block.used() as u64))
    }

    /// Writes a new full block from this delta and its base: unchanged strata copied whole, changed ones written
    /// in canonical order (so the new block's physical order is its canonical order).
    pub fn rollup(&self, reserve: usize) -> FrozenFacts {
        let base = self.base.as_ref().expect("roll up a delta");
        let root = root_of(&self.block);
        let out = Facts::open(self.block.header().kind, root.phase, self.schema.clone(), reserve, None);
        for s in 0..self.schema.nstrata {
            if !self.changed(s) {
                out.copy_stratum(base, s);
                continue;
            }
            out.begin(s);
            for r in self.schema.in_stratum(s) {
                out.create(r);
                let v = self.view(r);
                let w = out.w(r);
                let mut flat = Vec::with_capacity(v.arity() * v.live_len());
                for id in v.ordered() {
                    for c in 0..v.arity() {
                        flat.push(v.get(id, c));
                    }
                }
                w.extend(&flat);
                out.freeze_rel(r);
            }
            out.end(s);
        }
        out.block.header().cost.store(0, Relaxed);
        out.freeze()
    }
}
