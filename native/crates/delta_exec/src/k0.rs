//! `k0_pass` on the host (decision 0686, B6; *0686: Operations* §10: read tapes, facts · write facts · rw control ·
//! emit): its signature, its twin registered as its reference (`rerail_delta_twins::k0`, today's walk, a statement
//! an item), and the call that runs it, one launch over every document of a run, into L1's δ runs blocks, a K-form
//! relation each. The documents are a docs block's entries (`docs_of`'s, the selection joined with the blobs' rows),
//! and the blocks they name are bound by reference: the launch reaches them and reads them where they lie.

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
use rerail_delta_twins::k0::{self as twin, DocsOfP, K0P};

/// Control's reservation: the statements' places a launch can grow it to (address space, resident as written).
const CONTROL_RESERVE: usize = 64 << 20;

const fn ph(name: &'static str, across: Across, output: Output) -> Phase {
    Phase { name, found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across, output, bounds: &[] }
}

/// Emit (shape 11): count, scan, place; output in order, no atomics. Then keep: L1's cache written in place, grown
/// within its reservation.
static K0_PHASES: [Phase; 4] = [
    Phase { name: "count", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::ByIndex, bounds: &[Bound::Capacity(Over::Grow)] },
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

static DOCS_OF_PHASE: [Phase; 1] = [ph("docs_of", Across::Nothing, Output::ByScan)];

/// `docs_of`: read the selection's rows (a runs block) and the blobs' rows (their head; the layers below by reference)
/// · write the docs block · write the blocks it names (a parts block).
pub static DOCS_OF: Signature = Signature::new(
    "docs_of",
    &[
        SlotSpec { name: "rows", mode: Mode::Read, kind: kind::SCRATCH },
        SlotSpec { name: "blobs", mode: Mode::Read, kind: kind::FACTS },
        SlotSpec { name: "docs", mode: Mode::Write, kind: kind::CONTROL },
        SlotSpec { name: "blocks", mode: Mode::Write, kind: kind::CONTROL },
    ],
    ParamsLayout::of::<DocsOfP>(),
    &DOCS_OF_PHASE,
    Independence::Along("documents: a row's document is its own"),
    docs_of_facts,
);

/// `docs_of`'s written slots by meaning: the docs block's documents (module, blocks, places) and the first missing;
/// the blocks' list.
pub fn docs_of_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == twin::D_BLOCKS {
        return rerail_delta_twins::drop::parts(bytes).map(|p| vec![p]).collect(); // copy: derived (a check's reading of facts)
    }
    let mut rows: Vec<Vec<i64>> = (0..twin::docs_n(bytes)).map(|i| vec![twin::docs_module(bytes, i)]).collect(); // copy: derived (a check's reading of facts)
    rows.push(vec![twin::docs_missing(bytes).unwrap_or(i64::MIN)]);
    rows
}

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
    pub docs_of: OpId,
}

impl K0Ops {
    pub fn declare(reg: &mut Registry) -> Result<K0Ops, String> {
        let k0_pass = reg.declare(&K0_PASS)?;
        reg.register_reference(Version::twin(k0_pass, "k0_pass twin", twin::K0_PASS, &K0_PHASES))?;
        let docs_of = reg.declare(&DOCS_OF)?;
        reg.register_reference(Version::twin(docs_of, "docs_of twin", twin::DOCS_OF, &DOCS_OF_PHASE))?;
        Ok(K0Ops { k0_pass, docs_of })
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

/// A run's documents for `k0_pass`: one docs block, frozen (its entries, `twin::put_doc`), and the blocks its entries
/// name bound to the launch by reference. The binding is the directory's (the shell's): a chain of holder blocks,
/// each referring to at most `PER_HOLDER` of those blocks and to the holder made before it, the last referred to by the
/// docs block. The entries are `docs_of`'s, the selection's rows joined with the blobs' rows where they lie.
pub struct DocsIn {
    pub first: BlockId,
    pub n: usize,
    holders: Option<BlockId>,
}

/// The blocks a holder refers to besides the holder before it.
const PER_HOLDER: usize = rerail_blocks::gens::MAX_REFS - 1;

/// Where a holder keeps the id of the holder before it (`u32::MAX`: none), for its release.
const PREV_AT: usize = rerail_blocks::gens::BODY_AT;

fn docs_write(dir: &Directory, id: BlockId, f: impl FnOnce(&mut [u8])) -> Result<(), String> {
    let b = dir.block(id).ok_or("a docs block, held by nothing")?;
    let mut w = pin_block(&b, "k0_pass's docs", true)?;
    f(w.bytes_mut().expect("a write pin"));
    Ok(())
}

/// Binds the blocks the sealed list `list` names (`(block << 32) | at`, sorted, so a block's entries lie together and
/// its last is its furthest place) to the open docs block `docs`: holders made in turn, each referring to the next
/// `PER_HOLDER` blocks at their furthest places and to the holder before it, frozen; the docs block referring to the
/// last. The last holder.
fn bind(dir: &mut Directory, docs: BlockId, list: BlockId) -> Result<Option<BlockId>, String> {
    let n = Parts::sealed(dir, list)?.len();
    let at = |dir: &Directory, i: usize| -> Result<(u32, usize), String> {
        let l = dir.read("k0_pass's blocks", &[list])?;
        let v = rerail_delta_twins::drop::parts(l.block(0)).nth(i).expect("a listed block");
        Ok(((v >> 32) as u32, (v & 0xffff_ffff) as usize))
    };
    let (mut prev, mut i): (Option<BlockId>, usize) = (None, 0);
    while i < n {
        let h = dir.make(kind::CONTROL, 8);
        docs_write(dir, h, |b| rerail_blocks::gens::set_word::<u32>(b, PREV_AT, prev.map_or(u32::MAX, |p| p.0)))?;
        let mut held = 0;
        while i < n && held < PER_HOLDER {
            let (b, a) = at(dir, i)?;
            // The block's last entry is its furthest place: referred to once, there.
            if i + 1 < n && at(dir, i + 1)?.0 == b {
                i += 1;
                continue;
            }
            dir.refer(h, BlockId(b), a)?;
            (held, i) = (held + 1, i + 1);
        }
        if let Some(p) = prev {
            dir.refer(h, p, rerail_blocks::gens::BODY_AT)?;
        }
        dir.freeze(h)?;
        prev = Some(h);
    }
    if let Some(p) = prev {
        dir.refer(docs, p, rerail_blocks::gens::BODY_AT)?;
    }
    Ok(prev)
}

/// Lets go of the holders from `last` down their chain.
fn holders_release(dir: &mut Directory, last: Option<BlockId>) {
    let mut at = last;
    while let Some(id) = at {
        let prev = dir.read("k0_pass's holders", &[id]).map(|l| rerail_blocks::gens::word::<u32>(l.block(0), PREV_AT)).unwrap_or(u32::MAX);
        dir.release(id);
        at = (prev != u32::MAX).then_some(BlockId(prev));
    }
}

/// Where a selection row's module and its source's, tape's and symbols' blob ids lie (`docs_of`'s params).
#[derive(Clone, Copy)]
pub struct RowCols {
    pub module: usize,
    pub source: usize,
    pub tape: usize,
    pub syms: usize,
}

impl DocsIn {
    /// The documents the runs block `rows` reads (its +1 rows, of at most `cap` rows), their blobs found where the
    /// blobs' rows at `blobs` lie: one launch of `docs_of` writes the docs block and the blocks it names, which the
    /// directory then binds. Refused, naming the document, when a blob a row names is in no blobs' row.
    #[allow(clippy::too_many_arguments)]
    pub fn of_rows(reg: &Registry, ops: &K0Ops, dir: &mut Directory, rows: BlockId, cap: usize, cols: RowCols, blobs: Option<BlockId>) -> Result<DocsIn, String> {
        let docs = dir.make(kind::CONTROL, twin::docs_bytes(cap.max(1)));
        let list = match docs_write(dir, docs, twin::open_docs).and_then(|()| Parts::open(dir, 3 * cap)) {
            Ok(l) => l,
            Err(e) => {
                dir.release(docs);
                return Err(e);
            }
        };
        let made = (|| {
            if let (Some(b), true) = (blobs, cap > 0) {
                let mut p = [0u8; DocsOfP::SIZE];
                DocsOfP { blobs: u64::from(b.0), module: cols.module as u32, source: cols.source as u32, tape: cols.tape as u32, syms: cols.syms as u32 }.write(&mut p);
                run_cpu(reg, dir, ops.docs_of, &[rows, b, docs, list], &p, Tiling::WHOLE)?;
            }
            let (n, missing) = {
                let l = dir.read("k0_pass's docs", &[docs])?;
                (twin::docs_n(l.block(0)), twin::docs_missing(l.block(0)))
            };
            if let Some(d) = missing {
                return Err(format!("document {d}: a blob the tape port names, which nothing holds"));
            }
            Ok(n)
        })();
        let n = match made {
            Ok(n) => n,
            Err(e) => {
                dir.release(list);
                dir.release(docs);
                return Err(e);
            }
        };
        DocsIn::bound(dir, docs, list, n)
    }

    /// The open docs block `docs` of `n` documents, sealed, its blocks (the list `list` names) bound, and frozen.
    fn bound(dir: &mut Directory, docs: BlockId, list: BlockId, n: usize) -> Result<DocsIn, String> {
        let holders = bind(dir, docs, list);
        dir.release(list);
        match holders.and_then(|h| dir.freeze(docs).map(|()| h)) {
            Ok(holders) => Ok(DocsIn { first: docs, n, holders }),
            Err(e) => {
                dir.release(docs);
                Err(e)
            }
        }
    }

    /// A run's documents `docs`, written in turn (the tests' documents in hand).
    #[cfg(test)]
    pub fn of(dir: &mut Directory, docs: &[K0Doc]) -> Result<DocsIn, String> {
        let id = dir.make(kind::CONTROL, twin::docs_bytes(docs.len().max(1)));
        let list = Parts::write(dir, 3 * docs.len(), |put| {
            for d in docs {
                for (b, at) in [(d.src.block, d.src.at), (d.tape.block, d.tape.at), (d.syms, rerail_blocks::gens::BODY_AT)] {
                    put((i64::from(b.0) << 32) | at as i64);
                }
            }
        })?;
        docs_write(dir, id, |b| {
            twin::open_docs(b);
            for (i, d) in docs.iter().enumerate() {
                twin::put_doc(b, i, &twin::DocAt { module: d.module, source: d.src.block.0, tape: d.tape.block.0, syms: d.syms.0, src: (d.src.at as u64, d.src.len as u64), at: (d.tape.at as u64, d.tape.len as u64) });
            }
            twin::seal_docs(b, docs.len());
        })?;
        let list_id = list.id();
        DocsIn::bound(dir, id, list_id, docs.len())
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// `f` of each document's module, read where its docs block holds it.
    pub fn each_module(&self, dir: &Directory, f: &mut dyn FnMut(i64)) -> Result<(), String> {
        let l = dir.read("k0_pass's docs", &[self.first])?;
        let b = l.block(0);
        (0..twin::docs_n(b)).for_each(|i| f(twin::docs_module(b, i)));
        Ok(())
    }

    /// Lets go of its docs block and its holders.
    pub fn release(self, dir: &mut Directory) {
        dir.release(self.first);
        holders_release(dir, self.holders);
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
    // Control's room for a first guess of statements; `count` grows it to what they need (its capacity bound).
    let control = dir.make_capped(kind::CONTROL, twin::control_bytes(64), 0, CONTROL_RESERVE);
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
        let mut p = [0u8; K0P::SIZE];
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
