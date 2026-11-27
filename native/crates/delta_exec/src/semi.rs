//! `semi_change` and `change_rows` on the host (decision 0686, B6): their signatures, their twins registered as their
//! references (`rerail_delta_twins::semi`), and the calls that write a trace's change, under another trace's
//! (`A ⋉ B`) or alone, into a δ runs block, the partitions it retracts into a parts block, and `B`'s keys' change into
//! a second runs block. L1's selection is one of them: which documents it retracts, which it reads, and its asks.

use crate::dir::{BlockId, Directory};
use crate::op::{Across, Found, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use rerail_blocks::block::kind;
use rerail_blocks::gens::Table;
use rerail_blocks::layer::Runs;
use rerail_delta_twins::Params;
use rerail_delta_twins::drop::parts;
use rerail_delta_twins::semi::{self as twin, NONE, SemiP};

/// One phase, one item: walk, count, claim, write (an emit with one writer).
static SEMI_PHASE: [Phase; 1] = [Phase { name: "semi", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::ByScan, bounds: &[] }];
static CHANGE_PHASE: [Phase; 1] = [Phase { name: "change", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::ByScan, bounds: &[] }];

/// `semi_change`: read facts (`A`'s head, `B`'s; the layers below each by reference) · write δ runs (the rows) · write
/// control (the parts) · write δ runs (the keys).
pub static SEMI_CHANGE: Signature = Signature::new(
    "semi_change",
    &[
        SlotSpec { name: "a", mode: Mode::Read, kind: kind::FACTS },
        SlotSpec { name: "b", mode: Mode::Read, kind: kind::FACTS },
        SlotSpec { name: "rows", mode: Mode::Write, kind: kind::SCRATCH },
        SlotSpec { name: "parts", mode: Mode::Write, kind: kind::CONTROL },
        SlotSpec { name: "keys", mode: Mode::Write, kind: kind::SCRATCH },
    ],
    ParamsLayout::of::<SemiP>(),
    &SEMI_PHASE,
    Independence::Along("keys: a key's rows are its own"),
    semi_facts,
);

/// `change_rows`: read facts (`A`'s head; the layers below by reference) · write δ runs · write control (the parts).
pub static CHANGE_ROWS: Signature = Signature::new(
    "change_rows",
    &[
        SlotSpec { name: "a", mode: Mode::Read, kind: kind::FACTS },
        SlotSpec { name: "rows", mode: Mode::Write, kind: kind::SCRATCH },
        SlotSpec { name: "parts", mode: Mode::Write, kind: kind::CONTROL },
    ],
    ParamsLayout::of::<SemiP>(),
    &CHANGE_PHASE,
    Independence::Along("keys: a key's rows are its own"),
    change_facts,
);

/// A written runs slot's facts by meaning: its `(row, diff)`s, sorted.
fn runs_facts(bytes: &[u8]) -> Vec<Vec<i64>> {
    let r = Runs::new(bytes);
    let arity = r.head.shape.arity();
    let mut rows: Vec<Vec<i64>> = (0..r.len()).map(|i| (0..arity).map(|c| r.get(i, c)).chain([i64::from(r.diff(i))]).collect()).collect(); // copy: derived (a check's reading of facts)
    rows.sort();
    rows
}

/// A parts slot's facts: its partitions, in order.
fn parts_facts(bytes: &[u8]) -> Vec<Vec<i64>> {
    parts(bytes).map(|p| vec![p]).collect() // copy: derived (a check's reading of facts)
}

pub fn semi_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == twin::PARTS { parts_facts(bytes) } else { runs_facts(bytes) }
}

pub fn change_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == twin::C_PARTS { parts_facts(bytes) } else { runs_facts(bytes) }
}

#[derive(Clone, Copy)]
pub struct SemiOps {
    pub semi: OpId,
    pub change: OpId,
}

impl SemiOps {
    pub fn declare(reg: &mut Registry) -> Result<SemiOps, String> {
        let semi = reg.declare(&SEMI_CHANGE)?;
        reg.register_reference(Version::twin(semi, "semi_change twin", twin::SEMI_CHANGE, &SEMI_PHASE))?;
        let change = reg.declare(&CHANGE_ROWS)?;
        reg.register_reference(Version::twin(change, "change_rows twin", twin::CHANGE_ROWS, &CHANGE_PHASE))?;
        Ok(SemiOps { semi, change })
    }
}

/// A trace between two heads (`None`: the empty state), its arity and the column it is joined on.
#[derive(Clone, Copy)]
pub struct Side {
    pub before: Option<BlockId>,
    pub after: Option<BlockId>,
    pub arity: usize,
    pub col: usize,
}

fn head(h: Option<BlockId>) -> u64 {
    h.map_or(NONE, |b| u64::from(b.0))
}

/// How the rows are selected: `A ▷ B` (`neg`) or `A ⋉ B`, and the sign whose rows' keys the parts block lists.
#[derive(Clone, Copy)]
pub struct How {
    pub neg: bool,
    pub parts_sign: i32,
}

impl How {
    /// `A ⋉ B`, the parts the partitions it retracts (L1's).
    pub const RETRACTED: How = How { neg: false, parts_sign: -1 };
    /// `A ▷ B`, the parts the keys of the rows it adds (a cold one's: every row of `A` whose key `B` does not hold).
    pub const UNMATCHED: How = How { neg: true, parts_sign: 1 };
}

fn params(a: &Side, b: Option<&Side>, how: How) -> [u8; SemiP::SIZE] {
    let mut p = [0u8; SemiP::SIZE];
    let b = b.copied().unwrap_or(Side { before: None, after: None, arity: 0, col: 0 });
    SemiP {
        a_before: head(a.before),
        a_after: head(a.after),
        b_before: head(b.before),
        b_after: head(b.after),
        a_arity: a.arity as u32,
        b_arity: b.arity as u32,
        a_col: a.col as u32,
        b_col: b.col as u32,
        neg: u32::from(how.neg),
        parts_sign: how.parts_sign,
    }
    .write(&mut p);
    p
}

/// The change of `A ⋉ B` (`A ▷ B`, `how.neg`; `b`: none, `A`'s change alone) into the open runs block `rows`, the
/// keys of its rows of sign `how.parts_sign` into the open parts block `parts` (`Parts::open`), and `B`'s keys'
/// change into the open runs block `keys` (with `b`). One launch; nothing when `A` is empty, or `B` is and every row
/// must match (`parts` then lists none). With `B` empty under `neg`, every row of `A` is unmatched: `A`'s change alone.
#[allow(clippy::too_many_arguments)]
pub fn select(reg: &Registry, ops: &SemiOps, dir: &mut Directory, a: &Side, b: Option<(&Side, BlockId)>, how: How, rows: BlockId, parts: BlockId) -> Result<(), String> {
    match (a.after, b) {
        // `B`'s keys' change is written with `A` empty too (the boot's first round asks for l0 of an empty set): an
        // empty `A` binds `B`'s head in its slot, and the launch reads it as `NONE`.
        (ah, Some((b, keys))) if b.after.is_some() => {
            let bh = b.after.expect("a head");
            run_cpu(reg, dir, ops.semi, &[ah.unwrap_or(bh), bh, rows, parts, keys], &params(a, Some(b), how), Tiling::WHOLE)
        }
        (None, _) => Ok(()),
        (Some(_), Some(_)) if !how.neg => Ok(()),
        (Some(ah), _) => run_cpu(reg, dir, ops.change, &[ah, rows, parts], &params(a, None, how), Tiling::WHOLE),
    }
}

#[cfg(test)]
#[path = "semi_tests.rs"]
mod tests;
