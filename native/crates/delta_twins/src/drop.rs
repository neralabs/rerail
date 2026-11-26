//! `drop_parts` (decision 0686; P§3: "the file's last result is its output trace's before"): each live row of a
//! trace's state in the partitions listed, emitted with a sign into a δ runs block. An operator whose input changed
//! for a partition retracts what its output kept there, read where it lies, instead of reading its old input again.
//! Its slots: the state's head layer (read; the layers below it reached by reference), the parts block (read: the
//! partitions) and the δ runs block (write).

use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::gens::{BODY_AT, set_word, word};
use rerail_blocks::layer::{Layer, runs_claim_in, runs_set_in};
use rerail_blocks::relation;

crate::params! {
    /// The state's head (its block id in the descriptor table) and the sign its rows are emitted with; `weighted`
    /// (1): a counts state's rows, each emitted without its last column, its diff the sign times that count.
    pub struct DropP {
        pub head: u64,
        pub diff: i64,
        pub weighted: u64,
    }
}

/// The slots.
pub const HEAD: usize = 0;
pub const PARTS: usize = 1;
pub const RUNS: usize = 2;

const LEN_AT: usize = BODY_AT;
const N_AT: usize = BODY_AT + 8;
const PARTS_AT: usize = BODY_AT + 16;

/// A parts block's body bytes for `n` partitions.
pub fn parts_bytes(n: usize) -> usize {
    16 + 8 * n
}

/// A parts block, written by the host before `drop_parts` runs: the partitions.
pub fn init_parts(bytes: &mut [u8], parts: &[i64]) {
    set_word::<u64>(bytes, LEN_AT, parts_bytes(parts.len()) as u64);
    set_word::<u64>(bytes, N_AT, parts.len() as u64);
    for (i, &p) in parts.iter().enumerate() {
        set_word::<i64>(bytes, PARTS_AT + 8 * i, p);
    }
}

/// The partitions a parts block lists.
pub fn parts(bytes: &[u8]) -> impl Iterator<Item = i64> + '_ {
    (0..word::<u64>(bytes, N_AT) as usize).map(move |i| word::<i64>(bytes, PARTS_AT + 8 * i))
}

/// `drop_parts`'s one item: every listed partition's live rows, counted, claimed at once, then written in order.
fn drop_parts(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <DropP as crate::Params>::read(c.params);
    let table = c.table;
    let arity = Layer::new(c.read(HEAD)).expect("a committed layer").head.shape.arity();
    let state = relation::View::chain(table, Some(p.head as u32), arity).expect("the state's layers, reached by reference");
    let ([ps], [runs]) = c.views([PARTS], [RUNS]);
    let n: usize = parts(ps).map(|q| Read::part(&state, q).count()).sum();
    let Some(mut at) = runs_claim_in(runs, n as u32) else { return };
    let mut row = [0i64; 16];
    for q in parts(ps) {
        for id in Read::part(&state, q) {
            for (k, v) in row[..arity].iter_mut().enumerate() {
                *v = Read::get(&state, id, ColumnIndex::new(k as u16));
            }
            if p.weighted != 0 {
                runs_set_in(runs, at, &row[..arity - 1], (p.diff * row[arity - 1]) as i32);
            } else {
                runs_set_in(runs, at, &row[..arity], p.diff as i32);
            }
            at += 1;
        }
    }
}

/// `drop_parts`: one item, the listed partitions in their order.
pub const DROP_PARTS: &[TwinPhase] = &[TwinPhase { name: "drop", items: |_| 1, run: drop_parts, again: None }];
