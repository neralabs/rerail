//! The floor's formats, written and read back in place, in a block on the stack: no allocator anywhere.

use crate::block::{BODY_AT, REFS_AT, REFS_MAGIC, Ref, refs};
use crate::pod::{set_word, word};
use crate::runs::{IndexW, MAX_COLS, MAX_IX, MAX_KEY, NONE, Runs, ShapeW, init_runs, runs_bytes, runs_claim_in, runs_set_in};

/// A block's bytes, aligned as a block's are.
#[repr(C, align(64))]
struct Bytes([u8; 4096]);

/// Three columns: 64, 32 and 64 bits wide, in their own order.
fn shape() -> ShapeW {
    let mut types = [0u8; MAX_COLS];
    let mut order = [0u8; MAX_COLS];
    types[0] = 1;
    types[2] = 1;
    for (c, o) in order.iter_mut().enumerate().take(3) {
        *o = c as u8;
    }
    let ix = IndexW { kind: 0, ncols: 0, cols: [0; MAX_KEY] };
    ShapeW { arity: 3, partition: NONE, levels: 0, encoding: 0, placement: 0, retention: 0, nidx: 0, pad: 0, types, order, indexes: [ix; MAX_IX] }
}

#[test]
fn a_runs_block_written_and_read_in_place() {
    let mut b = Bytes([0; 4096]);
    let s = shape();
    assert!(BODY_AT + runs_bytes(&s, 4) <= b.0.len());
    init_runs(&mut b.0, &s, 4);
    let rows: [([i64; 3], i32); 3] = [([7, -2, 1 << 40], 1), ([3, 9, -5], -1), ([0, 0, 0], 1)];
    let first = runs_claim_in(&mut b.0, rows.len() as u32).expect("room for three");
    for (k, (r, d)) in rows.iter().enumerate() {
        runs_set_in(&mut b.0, first + k as u32, r, *d);
    }
    let runs = Runs::new(&b.0);
    assert_eq!((runs.len(), runs.needed()), (3, 0));
    for (i, (r, d)) in rows.iter().enumerate() {
        for (c, &v) in r.iter().enumerate() {
            assert_eq!(runs.get(i, c), v, "row {i}, column {c}");
        }
        assert_eq!(runs.diff(i), *d);
    }
    // Past the capacity: no place, and the claims past it counted for the rerun.
    assert_eq!(runs_claim_in(&mut b.0, 3), None);
    let runs = Runs::new(&b.0);
    assert_eq!((runs.len(), runs.needed()), (4, 2));
}

#[test]
fn a_reference_table_read_in_place() {
    let mut b = Bytes([0; 4096]);
    assert!(refs(&b.0).is_empty(), "a block without a table refers to nothing");
    set_word::<u32>(&mut b.0, REFS_AT, REFS_MAGIC);
    set_word::<u32>(&mut b.0, REFS_AT + 4, 2);
    set_word(&mut b.0, REFS_AT + 8, Ref { id: 5, at: 320 });
    set_word(&mut b.0, REFS_AT + 16, Ref { id: 9, at: 1024 });
    assert_eq!(refs(&b.0), &[Ref { id: 5, at: 320 }, Ref { id: 9, at: 1024 }]);
    assert_eq!(word::<u32>(&b.0, REFS_AT + 4), 2);
}
