//! `drop_parts` (decision 0686; P§3: "the file's last result is its output trace's before"): each live row of a
//! trace's state in the partitions listed, emitted with a sign into a δ runs block. An operator whose input changed
//! for a partition retracts what its output kept there, read where it lies, instead of reading its old input again.
//! Its slots: the state's head layer (read; the layers below it reached by reference), the parts block (read: the
//! partitions) and the δ runs block (write).

use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::bistate::{ColumnIndex, Read};
use rerail_blocks::gens::{BODY_AT, set_word, word, words_mut};
use rerail_blocks::layer::{Layer, runs_claim_in, runs_set_in};
use rerail_blocks::relation;

crate::params! {
    /// The state's head (its block id in the descriptor table) and the sign its rows are emitted with.
    pub struct DropP {
        pub head: u64,
        pub diff: i64,
    }
}

/// The slots.
pub const HEAD: usize = 0;
pub const PARTS: usize = 1;
pub const RUNS: usize = 2;

const LEN_AT: usize = BODY_AT;
pub const N_AT: usize = BODY_AT + 8;
pub const PARTS_AT: usize = BODY_AT + 16;

/// A parts block's body bytes for `n` partitions.
pub fn parts_bytes(n: usize) -> usize {
    16 + 8 * n
}

/// A parts block opened by the host: it lists none yet. Its partitions are written where they lie as they are found
/// (`put_part`), then sealed (`seal_parts`), before a launch reads it.
pub fn open_parts(bytes: &mut [u8]) {
    set_word::<u64>(bytes, LEN_AT, parts_bytes(0) as u64);
    set_word::<u64>(bytes, N_AT, 0);
}

/// Partition `p` at place `i` of an open parts block (its room made for `i + 1`: `parts_bytes`).
pub fn put_part(bytes: &mut [u8], i: usize, p: i64) {
    set_word::<i64>(bytes, PARTS_AT + 8 * i, p);
}

/// The open parts block's first `n` places sealed: sorted and each partition once, where they lie. How many it lists.
pub fn seal_parts(bytes: &mut [u8], n: usize) -> usize {
    let ps = words_mut::<i64>(bytes, PARTS_AT, n);
    ps.sort_unstable();
    let mut k = 0;
    for i in 0..n {
        if k == 0 || ps[i] != ps[k - 1] {
            ps[k] = ps[i];
            k += 1;
        }
    }
    set_word::<u64>(bytes, LEN_AT, parts_bytes(k) as u64);
    set_word::<u64>(bytes, N_AT, k as u64);
    k
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
            runs_set_in(runs, at, &row[..arity], p.diff as i32);
            at += 1;
        }
    }
}

/// `drop_parts`: one item, the listed partitions in their order.
pub const DROP_PARTS: &[TwinPhase] = &[TwinPhase { name: "drop", items: |_| 1, run: drop_parts, again: None }];
