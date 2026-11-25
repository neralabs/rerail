//! The program trace's operations (B2): `consolidate`, which freezes a run's open layer in its trace's block.
//!
//! The trace itself is `rerail_blocks::trace` (one definition, beside `Relation` and `Bistate`); its work runs here
//! as an operation, through its twin (`rerail_delta_twins::trace::consolidate`).

use crate::op::{OpId, ParamsLayout, Phase, Registry, Shape, Signature, SlotSpec, Mode, Version, run_cpu};
use rerail_blocks::block::kind;
use rerail_blocks::trace::Trace;
use rerail_delta_twins::Params;
use rerail_delta_twins::trace::{ConsolidateP, consolidate};

/// `consolidate`: the trace's block, read and written. Its phases: the layer sorted and its duplicates against the
/// state below merged away, its indexes built, its partitions' change summed.
pub static CONSOLIDATE: Signature = Signature::new(
    "consolidate",
    &[SlotSpec { name: "trace", mode: Mode::ReadWrite, kind: kind::FACTS }],
    ParamsLayout::of::<ConsolidateP>(),
    &[
        Phase { name: "sort", shape: Shape::SORT },
        Phase { name: "merge", shape: Shape::MERGE },
        Phase { name: "index", shape: Shape::HASH_BUILD },
        Phase { name: "digest", shape: Shape::REDUCE },
    ],
);

/// Declares `consolidate` and registers its twin, its reference.
pub fn declare(reg: &mut Registry) -> Result<OpId, String> {
    let op = reg.declare(&CONSOLIDATE)?;
    reg.register(Version::twin(op, "consolidate twin", consolidate, &[Shape::SORT, Shape::MERGE, Shape::HASH_BUILD, Shape::REDUCE]))?;
    Ok(op)
}

/// Runs `consolidate` on `trace`'s open layer, then commits it: the run's layer becomes the committed state's last.
pub fn commit(reg: &Registry, op: OpId, trace: &mut Trace) -> Result<(), String> {
    let mut params = vec![0u8; ConsolidateP::SIZE];
    ConsolidateP::from(&trace.to_consolidate()).write(&mut params);
    run_cpu(reg, op, &[trace.block()], &params)?;
    trace.commit();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rerail_blocks::bistate::{ColumnIndex, Read};
    use rerail_blocks::block::Block;
    use std::collections::BTreeSet;
    use std::sync::Arc;

    fn rows_of<S: Read>(s: &S) -> BTreeSet<(i64, i64)> {
        s.live_ids().map(|id| (s.get(id, ColumnIndex::new(0)), s.get(id, ColumnIndex::new(1)))).collect()
    }

    /// The trace's layers are committed through the `consolidate` operation, its twin run on the trace's block in
    /// place: the state reads as the rows pushed less the rows killed, at both states.
    #[test]
    fn consolidate_runs_as_an_operation_on_the_traces_block() {
        let mut reg = Registry::default();
        let op = declare(&mut reg).unwrap();
        let block = Arc::new(Block::anon(kind::FACTS, 16 << 20).with_free_list());
        let mut t = Trace::new(block, vec![8, 8], None, Some(0));

        t.open().unwrap();
        for r in [[0, 1], [1, 2], [2, 3], [1, 2]] {
            t.writer().push(&r);
        }
        commit(&reg, op, &mut t).unwrap();
        let first: BTreeSet<_> = [(0, 1), (1, 2), (2, 3)].into();
        assert_eq!(rows_of(&t.state().unwrap()), first);

        t.open().unwrap();
        t.writer().push(&[3, 4]);
        t.writer().push(&[0, 1]);
        t.writer().kill(1);
        commit(&reg, op, &mut t).unwrap();
        let change = t.change().unwrap();
        assert_eq!(rows_of(change.before().unwrap()), first);
        assert_eq!(rows_of(change.after()), [(0, 1), (2, 3), (3, 4)].into());
        assert_eq!(change.counts(), (1, 1));
        let mut parts = t.changed_parts();
        parts.sort_unstable();
        assert_eq!(parts, [1, 3], "the partitions it changed: 1's row killed, 3's added");

        // The trace's block is pinned while the operation runs: a reader holding it refuses the start.
        t.open().unwrap();
        t.writer().push(&[5, 6]);
        let held = rerail_blocks::region::pin_block(t.block(), "a reader", false).unwrap();
        let e = commit(&reg, op, &mut t).unwrap_err();
        assert!(e.contains("consolidate could not start: slot 0 (trace)"), "{e}");
        drop(held);
        commit(&reg, op, &mut t).unwrap();
        assert_eq!(rows_of(&t.state().unwrap()), [(0, 1), (2, 3), (3, 4), (5, 6)].into());
    }
}
