//! `k0_pass` on the host (decision 0686, B6; *0686: Operations* §10: read tapes, facts · write facts · rw control ·
//! emit): its signature, its twin registered as its reference (`rerail_delta_twins::k0`, today's walk, a statement
//! an item), and the call that runs it over one document into L1's δ runs blocks, a K-form relation each.

use crate::dir::{BlockId, Directory};
use crate::kform::{RELATIONS, Refused};
use crate::op::{Across, Found, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use rerail_blocks::block::kind;
use rerail_blocks::gens::Table;
use rerail_blocks::layer::Runs;
use rerail_blocks::region::pin_block;
use rerail_delta_twins::Params;
use rerail_delta_twins::k0::{self as twin, K0P};

const fn ph(name: &'static str, across: Across, output: Output) -> Phase {
    Phase { name, found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across, output, bounds: &[] }
}

/// Emit (shape 11): count, scan, place; output in order, no atomics.
static K0_PHASES: [Phase; 3] = [ph("count", Across::Nothing, Output::ByIndex), ph("scan", Across::Ordered, Output::ByScan), ph("place", Across::Nothing, Output::ByScan)];

const fn runs(name: &'static str) -> SlotSpec {
    SlotSpec { name, mode: Mode::Write, kind: kind::SCRATCH }
}

/// `k0_pass`: read the document's tape block, its symbols and the atoms · rw control · write each K-form relation's
/// δ runs.
pub static K0_PASS: Signature = Signature::new(
    "k0_pass",
    &[
        SlotSpec { name: "tape", mode: Mode::Read, kind: kind::TAPE },
        SlotSpec { name: "syms", mode: Mode::Read, kind: kind::FACTS },
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
    ],
    ParamsLayout::of::<K0P>(),
    &K0_PHASES,
    Independence::Along("statements: a statement's rows are its own"),
    k0_facts,
);

/// The written slots' facts by meaning: control's refusal and rows placed; a relation's runs, its `(row, diff)`s.
pub fn k0_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == twin::CONTROL {
        let r = twin::refused(bytes).map_or(vec![-1, -1], |(n, why)| vec![n as i64, twin::WHY.iter().position(|w| *w == why).map_or(-1, |w| w as i64)]);
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

/// One document for `k0_pass`: its tape block, where its source and tape lie in it, its node count, and its symbols
/// block.
#[derive(Clone, Copy, Debug)]
pub struct K0Doc {
    pub tape: BlockId,
    pub src: (usize, usize),
    pub at: (usize, usize),
    pub nodes: usize,
    pub syms: BlockId,
}

/// Runs `k0_pass` over document `doc` as module `d`, its rows pushed into `runs` (a K-form relation's δ runs block
/// each) with sign `diff`: the rows it placed, or its first refusal by node order.
#[allow(clippy::too_many_arguments)]
pub fn k0_pass(reg: &Registry, ops: &K0Ops, dir: &mut Directory, atoms: BlockId, doc: K0Doc, d: i64, diff: i32, runs: &[BlockId]) -> Result<u64, Refused> {
    let fail = |why: String| Refused { module: d, rule: -1, why };
    let control = dir.make(kind::CONTROL, twin::control_bytes(doc.nodes));
    let ran = (|| {
        {
            let b = dir.block(control).expect("just made");
            let mut w = pin_block(&b, "k0_pass", true).map_err(fail)?;
            twin::init_control(w.bytes_mut().expect("a write pin"));
        }
        let mut p = vec![0u8; K0P::SIZE];
        let (src_at, src_len, tape_at, tape_len) = (doc.src.0 as u64, doc.src.1 as u64, doc.at.0 as u64, doc.at.1 as u64);
        K0P { src_at, src_len, tape_at, tape_len, module: d, diff: i64::from(diff) }.write(&mut p);
        let slots: Vec<BlockId> = [doc.tape, doc.syms, atoms, control].into_iter().chain(runs.iter().copied()).collect();
        run_cpu(reg, dir, ops.k0_pass, &slots, &p, Tiling::WHOLE).map_err(fail)?;
        let l = dir.read("k0_pass", &[control]).map_err(fail)?;
        match twin::refused(l.block(0)) {
            Some((n, why)) => Err(Refused { module: d, rule: n as i64, why: why.to_string() }),
            None => Ok(twin::placed(l.block(0))),
        }
    })();
    dir.release(control);
    ran
}
