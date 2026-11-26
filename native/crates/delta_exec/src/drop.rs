//! `drop_parts` on the host (decision 0686; P§3): its signature, its twin registered as its reference
//! (`rerail_delta_twins::drop`), and the call that retracts a trace state's rows in some partitions into a δ runs
//! block. L1 retracts a changed document's kept rows with it: read in its output trace, where they lie.

use crate::dir::{BlockId, Directory};
use crate::op::{Across, Found, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use rerail_blocks::block::kind;
use rerail_blocks::gens::Table;
use rerail_blocks::layer::Runs;
use rerail_blocks::region::pin_block;
use rerail_delta_twins::Params;
use rerail_delta_twins::drop::{self as twin, DropP};

/// One phase, one item: count, claim, write (an emit with one writer).
static DROP_PHASE: [Phase; 1] = [Phase { name: "drop", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::ByScan, bounds: &[] }];

/// `drop_parts`: read facts (the state's head; the layers below by reference) · read the parts · write δ runs.
pub static DROP_PARTS: Signature = Signature::new(
    "drop_parts",
    &[
        SlotSpec { name: "head", mode: Mode::Read, kind: kind::FACTS },
        SlotSpec { name: "parts", mode: Mode::Read, kind: kind::CONTROL },
        SlotSpec { name: "runs", mode: Mode::Write, kind: kind::SCRATCH },
    ],
    ParamsLayout::of::<DropP>(),
    &DROP_PHASE,
    Independence::Along("partitions: a partition's rows are its own"),
    drop_facts,
);

/// The written slot's facts by meaning: its `(row, diff)`s, sorted.
pub fn drop_facts(_: &Table<'_>, _: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    let r = Runs::new(bytes);
    let arity = r.head.shape.arity();
    let mut rows: Vec<Vec<i64>> = (0..r.len()).map(|i| (0..arity).map(|c| r.get(i, c)).chain([i64::from(r.diff(i))]).collect()).collect(); // copy: derived (a check's reading of facts)
    rows.sort();
    rows
}

#[derive(Clone, Copy)]
pub struct DropOps {
    pub drop_parts: OpId,
}

impl DropOps {
    pub fn declare(reg: &mut Registry) -> Result<DropOps, String> {
        let drop_parts = reg.declare(&DROP_PARTS)?;
        reg.register_reference(Version::twin(drop_parts, "drop_parts twin", twin::DROP_PARTS, &DROP_PHASE))?;
        Ok(DropOps { drop_parts })
    }
}

/// Pushes every live row of the state at `head` in partitions `parts` into `runs` with sign `diff`: how many.
pub fn drop_parts(reg: &Registry, ops: &DropOps, dir: &mut Directory, head: BlockId, parts: &[i64], diff: i32, runs: BlockId) -> Result<u64, String> {
    drop_in(reg, ops, dir, head, parts, diff, false, runs)
}

/// Pushes every live row of the counts state at `head` in partitions `parts` into `runs`, each without its count, its
/// diff `diff` times its count: a head's derivations there, retracted (a file's grain, read where they lie).
pub fn drop_counted(reg: &Registry, ops: &DropOps, dir: &mut Directory, head: BlockId, parts: &[i64], diff: i32, runs: BlockId) -> Result<u64, String> {
    drop_in(reg, ops, dir, head, parts, diff, true, runs)
}

#[allow(clippy::too_many_arguments)]
fn drop_in(reg: &Registry, ops: &DropOps, dir: &mut Directory, head: BlockId, parts: &[i64], diff: i32, weighted: bool, runs: BlockId) -> Result<u64, String> {
    if parts.is_empty() {
        return Ok(0);
    }
    let block = dir.make(kind::CONTROL, twin::parts_bytes(parts.len()));
    let ran = (|| {
        {
            let b = dir.block(block).expect("just made");
            let mut w = pin_block(&b, "drop_parts", true)?;
            twin::init_parts(w.bytes_mut().expect("a write pin"), parts);
        }
        let before = Runs::new(dir.read("drop_parts", &[runs])?.block(0)).head.claimed;
        let mut p = vec![0u8; DropP::SIZE];
        DropP { head: u64::from(head.0), diff: i64::from(diff), weighted: u64::from(weighted) }.write(&mut p);
        run_cpu(reg, dir, ops.drop_parts, &[head, block, runs], &p, Tiling::WHOLE)?;
        Ok(u64::from(Runs::new(dir.read("drop_parts", &[runs])?.block(0)).head.claimed - before))
    })();
    dir.release(block);
    ran
}

#[cfg(test)]
#[path = "drop_tests.rs"]
mod tests;
