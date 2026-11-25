//! A sort as phases (Operations §4, shapes 14 and 16): fixed chunks sorted each on its own, then merge passes, each
//! merging pairs of runs twice as wide as the last pass's. Every item is a chunk or a pair, so the result does not
//! depend on the tiling or the order tiles run in. What a pass leaves for the next is in the blocks: two arrays of the
//! items being sorted (positions or values), the pass's width and which array holds the runs.

use crate::Tile;
use std::cmp::Ordering;

/// Items a chunk: the first phase sorts each chunk by itself.
pub const CHUNK: usize = 256;

/// The chunks of `n` items.
pub fn chunks(n: usize) -> u32 {
    n.div_ceil(CHUNK) as u32
}

/// The pairs of runs `width` wide over `n` items: a pass's items.
pub fn pairs(n: usize, width: usize) -> u32 {
    n.div_ceil(width).div_ceil(2) as u32
}

/// Whether `width` still leaves more than one run (a width of 0: the first phase had no items, and nothing is sorted).
pub fn more(n: usize, width: usize) -> bool {
    width > 0 && width < n
}

/// The sort's first phase, for its tile of chunks: each chunk of `items` sorted by `cmp`.
pub fn sort_chunks(items: &mut [u32], tile: Tile, cmp: impl Fn(u32, u32) -> Ordering) {
    let n = items.len();
    for c in tile.items() {
        let r = c * CHUNK..((c + 1) * CHUNK).min(n);
        items[r].sort_by(|&a, &b| cmp(a, b));
    }
}

/// A merge pass, for its tile of pairs: runs `2p` and `2p + 1`, each `width` wide, of `src` merged into the same
/// place in `dst` (a run with no partner copied).
pub fn merge_pairs(src: &[u32], dst: &mut [u32], tile: Tile, width: usize, cmp: impl Fn(u32, u32) -> Ordering) {
    let n = src.len();
    for p in tile.items() {
        let lo = 2 * p * width;
        let mid = (lo + width).min(n);
        let hi = (lo + 2 * width).min(n);
        let (mut i, mut j, mut o) = (lo, mid, lo);
        while i < mid && j < hi {
            // Stable: the left run's item first on a tie.
            if cmp(src[j], src[i]) == Ordering::Less {
                dst[o] = src[j];
                j += 1;
            } else {
                dst[o] = src[i];
                i += 1;
            }
            o += 1;
        }
        dst[o..o + (mid - i)].copy_from_slice(&src[i..mid]);
        o += mid - i;
        dst[o..o + (hi - j)].copy_from_slice(&src[j..hi]);
    }
}
