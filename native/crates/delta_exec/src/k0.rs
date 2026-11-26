//! `k0_pass` on the host (decision 0686, B6; *0686: Operations* §10: read tapes, facts · write facts · rw control ·
//! emit): its signature, its twin registered as its reference (`rerail_delta_twins::k0`, today's walk, a statement
//! an item), and the call that runs it, one launch over every document of a run, into L1's δ runs blocks, a K-form
//! relation each. The documents are a docs block's (chained past `DOCS_PER_BLOCK`), which refers to each one's source,
//! tape and symbols blocks: the launch reaches them by reference and reads them where they lie.

use crate::dir::{BlockId, Directory};
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
/// control · write each K-form relation's δ runs · rw L1's cache (Bootstrap, Figure 2).
pub static K0_PASS: Signature = Signature::new(
    "k0_pass",
    &[
        SlotSpec { name: "docs", mode: Mode::Read, kind: kind::CONTROL },
        SlotSpec { name: "atoms", mode: Mode::Read, kind: kind::ATOMS },
        SlotSpec { name: "control", mode: Mode::ReadWrite, kind: kind::CONTROL },
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
        let r = twin::refused(bytes).map_or(vec![-1, -1, -1], |(d, n, why)| vec![d as i64, n as i64, twin::WHY.iter().position(|w| *w == why).map_or(-1, |w| w as i64)]);
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

/// The docs blocks of `docs`, chained, each frozen, each referring to its documents' source, tape and symbols blocks
/// and to the next: the first. They are made back to front, since a block refers only to frozen ones.
fn docs_blocks(dir: &mut Directory, docs: &[K0Doc], made: &mut Vec<BlockId>) -> Result<BlockId, String> {
    let mut next = None;
    // No document (only documents gone): one docs block of none.
    let chunks: Vec<&[K0Doc]> = if docs.is_empty() { vec![&[]] } else { docs.chunks(twin::DOCS_PER_BLOCK).collect() };
    for chunk in chunks.into_iter().rev() {
        let id = dir.make(kind::CONTROL, twin::docs_bytes(chunk.len()));
        made.push(id);
        {
            let b = dir.block(id).expect("just made");
            let mut w = pin_block(&b, "k0_pass", true)?;
            let at: Vec<twin::DocAt> = chunk // copy: derived (the documents' places, written once into the docs block)
                .iter()
                .map(|d| twin::DocAt { module: d.module, source: d.src.block.0, tape: d.tape.block.0, syms: d.syms.0, src: (d.src.at as u64, d.src.len as u64), at: (d.tape.at as u64, d.tape.len as u64) })
                .collect();
            twin::init_docs(w.bytes_mut().expect("a write pin"), &at, next.map_or(twin::NO_NEXT, |n: BlockId| n.0));
        }
        for d in chunk {
            dir.refer(id, d.src.block, d.src.at)?;
            dir.refer(id, d.tape.block, d.tape.at)?;
            dir.refer(id, d.syms, rerail_blocks::gens::BODY_AT)?;
        }
        if let Some(n) = next {
            dir.refer(id, n, rerail_blocks::gens::BODY_AT)?;
        }
        dir.freeze(id)?;
        next = Some(id);
    }
    next.ok_or_else(|| "no document".to_string())
}

/// Runs `k0_pass` once over every document of `docs`, their rows pushed into `runs` (a K-form relation's δ runs block
/// each) with sign `diff`, and their sizes kept in `cache` (L1's, opened for the run), where the modules `gone` let go
/// of theirs: the rows it placed, or its first refusal by document and node order. With no cache (the plain function,
/// which reads none) the sizes go to a block of the launch's own.
#[allow(clippy::too_many_arguments)]
pub fn k0_pass(reg: &Registry, ops: &K0Ops, dir: &mut Directory, atoms: BlockId, docs: &[K0Doc], gone: &[i64], diff: i32, runs: &[BlockId], cache: Option<BlockId>) -> Result<u64, Refused> {
    let fail = |why: String| Refused { module: docs.first().map_or(-1, |d| d.module), rule: -1, why };
    if docs.is_empty() && gone.is_empty() {
        return Ok(0);
    }
    let mut nodes = 0;
    for d in docs {
        nodes += crate::files::nodes(dir, d.src, d.tape).map_err(fail)?;
    }
    let control = dir.make(kind::CONTROL, twin::control_bytes(nodes, gone.len()));
    let mut made = vec![control];
    let cache = match cache {
        Some(c) => c,
        None => {
            let c = crate::cache::OpCache::new(dir).0;
            made.push(c);
            c
        }
    };
    let ran = (|| {
        {
            let b = dir.block(control).expect("just made");
            let mut w = pin_block(&b, "k0_pass", true).map_err(fail)?;
            let gone: Vec<u32> = gone.iter().map(|&g| g as u32).collect(); // copy: derived (the gone modules, written once into control)
            twin::init_control(w.bytes_mut().expect("a write pin"), &gone);
        }
        let first = docs_blocks(dir, docs, &mut made).map_err(fail)?;
        let mut p = vec![0u8; K0P::SIZE];
        K0P { docs: u64::from(first.0), diff: i64::from(diff) }.write(&mut p);
        let slots: Vec<BlockId> = [first, atoms, control].into_iter().chain(runs.iter().copied()).chain([cache]).collect();
        run_cpu(reg, dir, ops.k0_pass, &slots, &p, Tiling::WHOLE).map_err(fail)?;
        let l = dir.read("k0_pass", &[control]).map_err(fail)?;
        match twin::refused(l.block(0)) {
            Some((k, n, why)) => Err(Refused { module: docs[k].module, rule: n as i64, why: why.to_string() }),
            None => Ok(twin::placed(l.block(0))),
        }
    })();
    // The first docs block last made: let go from it down, then control.
    for id in made.into_iter().rev() {
        dir.release(id);
    }
    ran
}
