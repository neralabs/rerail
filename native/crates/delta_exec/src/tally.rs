//! `tally` (decision 0686; *0686: Operations* §10, "A rule body"; 0678 step 9): a head's derivations' change into
//! its derivation counts and its own change. A head row changes where its count crosses 0: added when it leaves 0,
//! gone when it reaches 0. O(1) a derivation gained or lost, with no search for another derivation.
//!
//! Its slots: the counts' state (read: a control block that refers to the counts trace's head, its layers below
//! reached by reference), the derivations' δ runs (read), the counts' δ runs and the head's δ runs (write), and
//! control (read-write: the head rows changed, the loop's flag; a count below 0, refused).
//!
//! The counts are a relation like any other (0678 step 9: "each a relation in a block with a version and a delta"): a
//! head's companion trace, the head's columns and the count, a row once a head row, its count above 0.

use crate::dir::{BlockId, Directory};
use crate::op::{Across, Found, Independence, Inside, Mode, OpId, Output, ParamsLayout, Phase, Registry, Signature, SlotSpec, Tiling, Unit, Version, run_cpu};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::block::kind;
use rerail_blocks::gens::{BODY_AT, Table, set_word, word};
use rerail_blocks::layer::{Runs, runs_claim_in, runs_set_in};
use rerail_blocks::region::pin_block;
use rerail_delta_twins::{Ctx, NoParams, Tile, TwinPhase};

const STATE: usize = 0;
const DERIVS: usize = 1;
const COUNTS: usize = 2;
const HEAD: usize = 3;
const CONTROL: usize = 4;
// The state block's words: the counts' head (−1: ∅) and the head's arity.
const S_HEAD: usize = BODY_AT;
const S_ARITY: usize = BODY_AT + 8;
// Control: the head rows changed, and a refusal (1: a count below 0).
const C_CHANGED: usize = BODY_AT;
const C_REFUSED: usize = BODY_AT + 8;

fn tally(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let table = c.table;
    let ([state, derivs], [counts, head, ctl]) = c.views([STATE, DERIVS], [COUNTS, HEAD, CONTROL]);
    let arity = word::<i64>(state, S_ARITY) as usize;
    let h = word::<i64>(state, S_HEAD);
    let kept = rerail_blocks::relation::View::chain(table, (h >= 0).then_some(h as u32), arity + 1).expect("the counts' state, reached by reference");
    let d = Runs::new(derivs);
    // The derivations in their rows' order: an order of their places, the rows read where they lie.
    let row_of = |i: usize| (0..arity).map(move |k| d.get(i, k));
    let mut order: Vec<usize> = (0..d.len()).collect();
    order.sort_by(|&a, &b| row_of(a).cmp(row_of(b)));
    let cols: Vec<ColumnIndex> = (0..arity as u16).map(ColumnIndex::new).collect();
    let mut key = vec![0i64; arity];
    // Each distinct row's summed change, its count before and after: `f(first place, old, new)`.
    let groups = |f: &mut dyn FnMut(usize, i64, i64), key: &mut Vec<i64>| {
        let mut i = 0;
        while i < order.len() {
            let mut j = i;
            let mut sum = 0i64;
            while j < order.len() && row_of(order[j]).eq(row_of(order[i])) {
                sum += i64::from(d.diff(order[j]));
                j += 1;
            }
            if sum != 0 {
                for (k, v) in key.iter_mut().zip(row_of(order[i])) {
                    *k = v;
                }
                let old = Read::matching(&kept, &cols, key).next().map_or(0, |id| Read::get(&kept, id, ColumnIndex::new(arity as u16)));
                f(order[i], old, old + sum);
            }
            i = j;
        }
    };
    let (mut nc, mut nh, mut below) = (0u32, 0u32, false);
    groups(
        &mut |_, old, new| {
            below |= new < 0;
            nc += u32::from(old > 0) + u32::from(new > 0);
            nh += u32::from((old > 0) != (new > 0));
        },
        &mut key,
    );
    if below {
        set_word::<i64>(ctl, C_REFUSED, 1);
        return;
    }
    let (Some(mut ac), Some(mut ah)) = (runs_claim_in(counts, nc), runs_claim_in(head, nh)) else { return };
    let mut row = vec![0i64; arity + 1];
    groups(
        &mut |at, old, new| {
            for (k, v) in row.iter_mut().zip(row_of(at)) {
                *k = v;
            }
            for (n, sign) in [(old, -1), (new, 1)] {
                if n > 0 {
                    row[arity] = n;
                    runs_set_in(counts, ac, &row, sign);
                    ac += 1;
                }
            }
            if (old > 0) != (new > 0) {
                runs_set_in(head, ah, &row[..arity], if new > 0 { 1 } else { -1 });
                ah += 1;
            }
        },
        &mut key,
    );
    set_word::<i64>(ctl, C_CHANGED, i64::from(nh));
}

fn one(_: &Ctx<'_, '_, '_>) -> u32 {
    1
}

static PHASES: [Phase; 1] = [Phase { name: "tally", found: Found::Fixed, unit: Unit::Threadgroup, inside: Inside::Nothing, across: Across::Nothing, output: Output::ByScan, bounds: &[] }];

/// `tally`: read the counts' state, the derivations' δ · write the counts' δ, the head's δ · rw control.
pub static TALLY: Signature = Signature::new(
    "tally",
    &[
        SlotSpec { name: "state", mode: Mode::Read, kind: kind::CONTROL },
        SlotSpec { name: "derivations", mode: Mode::Read, kind: kind::SCRATCH },
        SlotSpec { name: "counts", mode: Mode::Write, kind: kind::SCRATCH },
        SlotSpec { name: "head", mode: Mode::Write, kind: kind::SCRATCH },
        SlotSpec { name: "control", mode: Mode::ReadWrite, kind: kind::CONTROL },
    ],
    ParamsLayout::of::<NoParams>(),
    &PHASES,
    Independence::Along("rows: a row's count is its own"),
    tally_facts,
);

/// A written slot's facts, by meaning: a runs block's `(row, diff)`s, sorted; control's words.
fn tally_facts(_: &Table<'_>, slot: usize, bytes: &[u8]) -> Vec<Vec<i64>> {
    if slot == CONTROL {
        return vec![vec![word::<i64>(bytes, C_CHANGED), word::<i64>(bytes, C_REFUSED)]];
    }
    let r = Runs::new(bytes);
    let mut rows: Vec<Vec<i64>> = (0..r.len()).map(|i| (0..r.head.shape.arity()).map(|c| r.get(i, c)).chain([i64::from(r.diff(i))]).collect()).collect(); // copy: derived (a check's reading of facts)
    rows.sort();
    rows
}

static HOST: [TwinPhase; 1] = [TwinPhase { name: "tally", items: one, run: tally, again: None }];

#[derive(Clone, Copy)]
pub struct TallyOps {
    pub tally: OpId,
}

impl TallyOps {
    pub fn declare(reg: &mut Registry) -> Result<TallyOps, String> {
        let tally = reg.declare(&TALLY)?;
        reg.register_reference(Version::host(tally, "tally on the host", &HOST, &PHASES))?;
        Ok(TallyOps { tally })
    }
}

/// Tallies the derivations' δ `derivs` of a head of `arity` columns against its counts' state at `counts` (`None`:
/// ∅): the counts' δ into `counts_runs`, the head's into `head_runs`. How many head rows changed. Refused when a
/// count falls below 0 (a derivation gone that was never counted).
#[allow(clippy::too_many_arguments)]
pub fn run(reg: &Registry, ops: &TallyOps, dir: &mut Directory, counts: Option<BlockId>, arity: usize, derivs: BlockId, counts_runs: BlockId, head_runs: BlockId) -> Result<u64, String> {
    let state = dir.make(kind::CONTROL, 16);
    let ctl = dir.make(kind::CONTROL, 16);
    let ran = (|| {
        {
            let b = dir.block(state).expect("just made");
            let mut w = pin_block(&b, "tally", true)?;
            let bytes = w.bytes_mut().expect("a write pin");
            set_word::<i64>(bytes, S_HEAD, counts.map_or(-1, |h| i64::from(h.0)));
            set_word::<i64>(bytes, S_ARITY, arity as i64);
        }
        if let Some(h) = counts {
            dir.refer(state, h, BODY_AT)?;
        }
        run_cpu(reg, dir, ops.tally, &[state, derivs, counts_runs, head_runs, ctl], &[], Tiling::WHOLE)?;
        let l = dir.read("tally", &[ctl])?;
        if word::<i64>(l.block(0), C_REFUSED) != 0 {
            return Err("tally: a count below 0 (a derivation gone that was never counted)".to_string());
        }
        Ok(word::<i64>(l.block(0), C_CHANGED) as u64)
    })();
    dir.release(state);
    dir.release(ctl);
    ran
}
