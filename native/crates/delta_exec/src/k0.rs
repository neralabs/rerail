//! `k0_pass` on the host (decision 0686, B6; *0686: Operations* §10: read tapes, facts · write facts · rw control ·
//! emit): its signature, its twin registered as its reference (`rerail_delta_twins::k0`, today's walk, a statement
//! an item), and the call that runs it, one launch over every document of a run, into L1's δ runs blocks, a K-form
//! relation each. The documents are a docs block's (chained past `DOCS_PER_BLOCK`), which refers to each one's source,
//! tape and symbols blocks: the launch reaches them by reference and reads them where they lie.

use crate::dir::{BlockId, Directory};
use crate::drop::Parts;
use crate::files::Placed;
use crate::kform::{RELATIONS, Refused};
use crate::op::{Across, Bound, Found, Over, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use rerail_blocks::block::kind;
use rerail_blocks::gens::Table;
use rerail_blocks::layer::Runs;
use rerail_blocks::region::pin_block;
use rerail_delta_twins::Params;
use rerail_delta_twins::k0::{self as twin, K0P};

const fn ph(name: &'static str, across: Across, output: Output) -> Phase {
    Phase { name, found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across, output, bounds: &[] }
}

/// Emit (shape 11): count, scan, place; output in order, no atomics. Then keep: L1's cache written in place, grown
/// within its reservation.
static K0_PHASES: [Phase; 4] = [
    ph("count", Across::Nothing, Output::ByIndex),
    ph("scan", Across::Ordered, Output::ByScan),
    ph("place", Across::Nothing, Output::ByScan),
    Phase { name: "keep", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::InPlace, bounds: &[Bound::Capacity(Over::Grow)] },
];

const fn runs(name: &'static str) -> SlotSpec {
    SlotSpec { name, mode: Mode::Write, kind: kind::SCRATCH }
}

/// `k0_pass`: read the documents (a docs block, their tape and symbols blocks by reference) and the atoms · rw
/// control · read the documents gone (a parts block) · write each K-form relation's δ runs · rw L1's cache (Bootstrap, Figure 2).
pub static K0_PASS: Signature = Signature::new(
    "k0_pass",
    &[
        SlotSpec { name: "docs", mode: Mode::Read, kind: kind::CONTROL },
        SlotSpec { name: "atoms", mode: Mode::Read, kind: kind::ATOMS },
        SlotSpec { name: "control", mode: Mode::ReadWrite, kind: kind::CONTROL },
        SlotSpec { name: "gone", mode: Mode::Read, kind: kind::CONTROL },
        runs("k_rule"),
        runs("k_step"),
        runs("k_arg"),
        runs("k_var"),
        runs("k_str"),
        runs("k_num"),
        runs("k_wild"),
        runs("k_order"),
        SlotSpec { name: "cache", mode: Mode::ReadWrite, kind: kind::CACHE },
    ],
    ParamsLayout::of::<K0P>(),
    &K0_PHASES,
    Independence::Along("statements: a statement's rows are its own"),
    k0_facts,
);

/// The written slots' facts by meaning: control's refusal and rows placed; a relation's runs, its `(row, diff)`s; L1's
/// cache, its entries.
pub fn k0_facts(table: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == twin::CACHE {
        return crate::cache::cache_facts(table, slot, bytes);
    }
    if slot == twin::CONTROL {
        let r = twin::refused(bytes).map_or(vec![-1, -1, -1], |(d, n, why)| vec![d, n as i64, twin::WHY.iter().position(|w| *w == why).map_or(-1, |w| w as i64)]);
        return vec![r, vec![twin::placed(bytes) as i64]];
    }
    let r = Runs::new(bytes);
    let arity = RELATIONS[slot - twin::RUNS].1;
    let mut rows: Vec<Vec<i64>> = (0..r.len()).map(|i| (0..arity).map(|c| r.get(i, c)).chain([i64::from(r.diff(i))]).collect()).collect();
    rows.sort();
    rows
}

#[derive(Clone, Copy)]
pub struct K0Ops {
    pub k0_pass: OpId,
}

impl K0Ops {
    pub fn declare(reg: &mut Registry) -> Result<K0Ops, String> {
        let k0_pass = reg.declare(&K0_PASS)?;
        reg.register_reference(Version::twin(k0_pass, "k0_pass twin", twin::K0_PASS, &K0_PHASES))?;
        Ok(K0Ops { k0_pass })
    }
}

/// One document for `k0_pass`: its module, where its source and its tape lie (each a blob, in a frozen block, which
/// may be one), and its symbols block (frozen).
#[derive(Clone, Copy, Debug)]
pub struct K0Doc {
    pub module: i64,
    pub src: Placed,
    pub tape: Placed,
    pub syms: BlockId,
}

/// The documents of a `k0_pass` launch, written into its docs blocks where they lie as they are found (no list of
/// them on the host): a docs block holds `DOCS_PER_BLOCK` documents, refers to each one's source, tape and symbols
/// blocks and to the docs block filled before it, and is frozen when full; the launch reads from the last one made
/// (`finish`). The documents' order is the docs blocks', the last made first.
#[derive(Default)]
pub struct Docs {
    /// The docs block being filled, and how many documents it holds.
    open: Option<(BlockId, usize)>,
    /// The last docs block frozen.
    head: Option<BlockId>,
    nodes: usize,
}

/// A run's documents, their docs blocks sealed (`Docs::finish`): the first docs block the launch reads, how many
/// documents they hold, and their nodes in all.
pub struct DocsIn {
    pub first: BlockId,
    pub n: usize,
    pub nodes: usize,
}

fn docs_write(dir: &Directory, id: BlockId, f: impl FnOnce(&mut [u8])) -> Result<(), String> {
    let b = dir.block(id).ok_or("a docs block, held by nothing")?;
    let mut w = pin_block(&b, "k0_pass's docs", true)?;
    f(w.bytes_mut().expect("a write pin"));
    Ok(())
}

/// Lets go of the docs blocks from `from` down their chain.
fn docs_release(dir: &mut Directory, from: Option<BlockId>) {
    let mut at = from;
    while let Some(id) = at {
        let next = dir.read("k0_pass's docs", &[id]).map(|l| twin::docs_next(l.block(0))).unwrap_or(twin::NO_NEXT);
        dir.release(id);
        at = (next != twin::NO_NEXT).then_some(BlockId(next));
    }
}

impl Docs {
    /// Document `d` written into the open docs block (a new one, over the last frozen, when there is none or it is
    /// full), its blocks referred to.
    pub fn push(&mut self, dir: &mut Directory, d: &K0Doc) -> Result<(), String> {
        self.nodes += crate::files::nodes(dir, d.src, d.tape)?;
        if matches!(self.open, Some((_, n)) if n == twin::DOCS_PER_BLOCK) {
            self.close(dir)?;
        }
        let (id, i) = match self.open {
            Some(o) => o,
            None => self.open_block(dir)?,
        };
        let at = twin::DocAt { module: d.module, source: d.src.block.0, tape: d.tape.block.0, syms: d.syms.0, src: (d.src.at as u64, d.src.len as u64), at: (d.tape.at as u64, d.tape.len as u64) };
        docs_write(dir, id, |b| twin::put_doc(b, i, &at))?;
        dir.refer(id, d.src.block, d.src.at)?;
        dir.refer(id, d.tape.block, d.tape.at)?;
        dir.refer(id, d.syms, rerail_blocks::gens::BODY_AT)?;
        self.open = Some((id, i + 1));
        Ok(())
    }

    fn open_block(&mut self, dir: &mut Directory) -> Result<(BlockId, usize), String> {
        let id = dir.make(kind::CONTROL, twin::docs_bytes(twin::DOCS_PER_BLOCK));
        self.open = Some((id, 0));
        docs_write(dir, id, |b| twin::open_docs(b, self.head.map_or(twin::NO_NEXT, |h| h.0)))?;
        if let Some(h) = self.head {
            dir.refer(id, h, rerail_blocks::gens::BODY_AT)?;
        }
        Ok((id, 0))
    }

    fn close(&mut self, dir: &mut Directory) -> Result<(), String> {
        if let Some((id, n)) = self.open.take() {
            self.head = Some(id);
            docs_write(dir, id, |b| twin::seal_docs(b, n))?;
            dir.freeze(id)?;
        }
        Ok(())
    }

    /// Whether no document was pushed.
    pub fn is_empty(&self) -> bool {
        self.open.is_none() && self.head.is_none()
    }

    /// Its last docs block sealed: the launch's documents (a docs block of none when it has no document). On an
    /// error, its blocks are let go.
    pub fn finish(mut self, dir: &mut Directory) -> Result<DocsIn, String> {
        let sealed = (|| {
            if self.is_empty() {
                self.open_block(dir)?;
            }
            self.close(dir)
        })();
        if let Err(e) = sealed {
            self.release(dir);
            return Err(e);
        }
        let first = self.head.expect("a docs block, sealed");
        Ok(DocsIn { first, n: Self::count(dir, first)?, nodes: self.nodes })
    }

    fn count(dir: &Directory, from: BlockId) -> Result<usize, String> {
        let (mut n, mut at) = (0, Some(from));
        while let Some(id) = at {
            let next = {
                let l = dir.read("k0_pass's docs", &[id])?;
                n += twin::docs_n(l.block(0));
                twin::docs_next(l.block(0))
            };
            at = (next != twin::NO_NEXT).then_some(BlockId(next));
        }
        Ok(n)
    }

    /// Lets go of its docs blocks (a run's documents not launched).
    pub fn release(self, dir: &mut Directory) {
        docs_release(dir, self.open.map(|o| o.0).or(self.head));
    }

    /// A run's documents `docs`, pushed in turn and sealed.
    #[cfg(test)]
    pub fn of(dir: &mut Directory, docs: &[K0Doc]) -> Result<DocsIn, String> {
        let mut w = Docs::default();
        for d in docs {
            if let Err(e) = w.push(dir, d) {
                w.release(dir);
                return Err(e);
            }
        }
        w.finish(dir)
    }
}

impl DocsIn {
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// `f` of each document's module, read where its docs block holds it.
    pub fn each_module(&self, dir: &Directory, f: &mut dyn FnMut(i64)) -> Result<(), String> {
        let mut at = Some(self.first);
        while let Some(id) = at {
            let l = dir.read("k0_pass's docs", &[id])?;
            let b = l.block(0);
            (0..twin::docs_n(b)).for_each(|i| f(twin::docs_module(b, i)));
            let next = twin::docs_next(b);
            at = (next != twin::NO_NEXT).then_some(BlockId(next));
        }
        Ok(())
    }

    /// Lets go of its docs blocks, from the first down.
    pub fn release(self, dir: &mut Directory) {
        docs_release(dir, Some(self.first));
    }
}

/// Runs `k0_pass` once over the documents `docs` (their docs blocks, sealed), their rows pushed into `runs` (a K-form
/// relation's δ runs block each) with sign `diff`, and their sizes kept in `cache` (L1's, opened for the run), where
/// the modules the parts block `gone` lists let go of theirs: the rows it placed, or its first refusal by document and
/// node order. With no cache (the plain function, which reads none) the sizes go to a block of the launch's own; with
/// no `gone`, a parts block of none.
#[allow(clippy::too_many_arguments)]
pub fn k0_pass(reg: &Registry, ops: &K0Ops, dir: &mut Directory, atoms: BlockId, docs: &DocsIn, gone: Option<&Parts>, diff: i32, runs: &[BlockId], cache: Option<BlockId>) -> Result<u64, Refused> {
    let fail = |why: String| Refused { module: -1, rule: -1, why };
    if docs.is_empty() && gone.is_none_or(Parts::is_empty) {
        return Ok(0);
    }
    assert_eq!(runs.len(), RELATIONS.len(), "a δ runs block a K-form relation");
    let control = dir.make(kind::CONTROL, twin::control_bytes(docs.nodes));
    let own_cache = cache.is_none().then(|| crate::cache::OpCache::new(dir).0);
    let own_gone = match gone {
        Some(_) => None,
        None => Some(Parts::write(dir, 0, |_| {}).map_err(fail)?),
    };
    let gone = gone.or(own_gone.as_ref()).expect("a parts block");
    let ran = (|| {
        {
            let b = dir.block(control).expect("just made");
            let mut w = pin_block(&b, "k0_pass", true).map_err(fail)?;
            twin::init_control(w.bytes_mut().expect("a write pin"));
        }
        let mut p = vec![0u8; K0P::SIZE];
        K0P { docs: u64::from(docs.first.0), diff: i64::from(diff) }.write(&mut p);
        let mut slots = [control; twin::CACHE + 1];
        (slots[twin::DOCS], slots[twin::ATOMS], slots[twin::GONE]) = (docs.first, atoms, gone.id());
        slots[twin::RUNS..twin::CACHE].copy_from_slice(runs);
        slots[twin::CACHE] = cache.or(own_cache).expect("a cache");
        run_cpu(reg, dir, ops.k0_pass, &slots, &p, Tiling::WHOLE).map_err(fail)?;
        let l = dir.read("k0_pass", &[control]).map_err(fail)?;
        match twin::refused(l.block(0)) {
            Some((module, n, why)) => Err(Refused { module, rule: n as i64, why: why.to_string() }),
            None => Ok(twin::placed(l.block(0))),
        }
    })();
    dir.release(control);
    if let Some(c) = own_cache {
        dir.release(c);
    }
    if let Some(g) = own_gone {
        g.release(dir);
    }
    ran
}
