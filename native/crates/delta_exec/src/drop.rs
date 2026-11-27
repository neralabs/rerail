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

/// A parts block: the partitions a run retracts (L1's documents gone), written where it lies as they are found, then
/// sealed, sorted and each once (`twin::seal_parts`). `drop_parts` and `k0_pass` read it there; its maker lets it go
/// (`release`).
pub struct Parts {
    id: BlockId,
    n: usize,
}

impl Parts {
    /// A parts block with room for `cap` partitions, each `fill` puts written in place, then sealed.
    pub fn write(dir: &mut Directory, cap: usize, fill: impl FnOnce(&mut dyn FnMut(i64))) -> Result<Parts, String> {
        let id = dir.make(kind::CONTROL, twin::parts_bytes(cap));
        let b = dir.block(id).expect("just made");
        let mut w = match pin_block(&b, "a parts block", true) {
            Ok(w) => w,
            Err(e) => {
                drop(b);
                dir.release(id);
                return Err(e);
            }
        };
        let bytes = w.bytes_mut().expect("a write pin");
        twin::open_parts(bytes);
        let mut n = 0;
        fill(&mut |p| {
            assert!(n < cap, "a parts block's room: {cap} partitions");
            twin::put_part(bytes, n, p);
            n += 1;
        });
        let n = twin::seal_parts(bytes, n);
        Ok(Parts { id, n })
    }

    pub fn id(&self) -> BlockId {
        self.id
    }

    /// How many partitions it lists.
    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// `f` of each partition it lists, read where the block holds them.
    pub fn each(&self, dir: &Directory, f: &mut dyn FnMut(i64)) -> Result<(), String> {
        let l = dir.read("a parts block", &[self.id])?;
        twin::parts(l.block(0)).for_each(f);
        Ok(())
    }

    pub fn release(self, dir: &mut Directory) {
        dir.release(self.id);
    }
}

/// Pushes every live row of the state at `head` in the partitions `parts` lists into `runs` with sign `diff`: how many.
pub fn drop_parts(reg: &Registry, ops: &DropOps, dir: &mut Directory, head: BlockId, parts: &Parts, diff: i32, runs: BlockId) -> Result<u64, String> {
    if parts.is_empty() {
        return Ok(0);
    }
    let before = Runs::new(dir.read("drop_parts", &[runs])?.block(0)).head.claimed;
    let mut p = [0u8; DropP::SIZE];
    DropP { head: u64::from(head.0), diff: i64::from(diff) }.write(&mut p);
    run_cpu(reg, dir, ops.drop_parts, &[head, parts.id(), runs], &p, Tiling::WHOLE)?;
    Ok(u64::from(Runs::new(dir.read("drop_parts", &[runs])?.block(0)).head.claimed - before))
}

#[cfg(test)]
#[path = "drop_tests.rs"]
mod tests;
