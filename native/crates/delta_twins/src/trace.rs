//! The program trace's operations (decision 0686 B2).

use crate::{Params, Slot};
use rerail_blocks::scratch::Scratch;
use rerail_blocks::trace::{self, Consolidate, Layer};

/// Address space a call's scratch reserves: nothing resident until written, given back when the call ends.
const SCRATCH: usize = 1 << 32;

/// No offset, no column.
pub const NONE: u64 = u64::MAX;

crate::params! {
    pub struct ConsolidateP {
        /// The open layer's relations in the trace's block: its rows, its partitions' change, its kills by partition.
        pub rows: u64,
        pub parts: u64,
        pub gone: u64,
        /// The layer below it, `NONE` for the first.
        pub below: u64,
        /// Its rows' sort column and partition column, -1 for none.
        pub sort: i32,
        pub partition: i32,
    }
}

impl From<&Consolidate> for ConsolidateP {
    fn from(c: &Consolidate) -> ConsolidateP {
        let col = |c: Option<u16>| c.map_or(-1, i32::from);
        ConsolidateP {
            rows: c.layer.rows as u64,
            parts: c.layer.parts as u64,
            gone: c.layer.gone as u64,
            below: c.below.map_or(NONE, |b| b as u64),
            sort: col(c.sort),
            partition: col(c.partition),
        }
    }
}

impl From<&ConsolidateP> for Consolidate {
    fn from(p: &ConsolidateP) -> Consolidate {
        let col = |c: i32| u16::try_from(c).ok();
        Consolidate {
            layer: Layer { rows: p.rows as usize, parts: p.parts as usize, gone: p.gone as usize },
            below: (p.below != NONE).then_some(p.below as usize),
            sort: col(p.sort),
            partition: col(p.partition),
        }
    }
}

/// `consolidate`: freezes a trace's open layer where it lies, and writes its partitions' change beside it
/// (`rerail_blocks::trace::consolidate`). Slot 0: the trace's block, written. Its temporaries are a scratch of the
/// call's own.
pub fn consolidate(slots: &mut [Slot<'_>], params: &[u8]) {
    let c = Consolidate::from(&ConsolidateP::read(params));
    let Slot::Write(pin) = &slots[0] else { panic!("consolidate writes its trace's block") };
    trace::consolidate(pin.block(), &c, &Scratch::anon(SCRATCH));
}
