//! Duplicated blocks from a suffix array, as Jeff's rule reports them (`docs/rule-syntax/spikes/duplicate-blocks/
//! no-duplicate-blocks.rerail` on `jeffreysnell/dup-blocks-rule`), with the window a parameter instead of 30:
//!
//! - a window is `min` units (counted lines, or nodes) from a position; it is duplicated when another position's
//!   window is the same: an adjacent suffix shares at least `min` units with it (Jeff's `dup_key`);
//! - consecutive duplicated windows in a document are one block, from the first window's first unit to the last
//!   window's last (Jeff's `run` and `block`);
//! - the block's other copies are the other places of its first window (Jeff's `dup_place`): the LCP interval of at
//!   least `min` around its first window's suffix.

use crate::multi::MultiSuffixArray;

pub struct Block {
    pub doc: u32,
    /// The block's first unit and one past its last, in the document.
    pub first: u32,
    pub end: u32,
    /// The other places of the first window, (document, unit).
    pub others: Vec<(u32, u32)>,
}

pub fn blocks(a: &MultiSuffixArray, min: u32) -> Vec<Block> {
    let min = min.max(1);
    let n = a.len();
    let ranks = a.ranks();
    let dup = |r: usize| a.lcp[r] >= min || (r + 1 < n && a.lcp[r + 1] >= min);
    let mut out = Vec::new();
    for d in 0..a.offsets.len() - 1 {
        let (lo, hi) = (a.offsets[d] as usize, a.offsets[d + 1] as usize - 1);
        let mut j = 0;
        while lo + j < hi {
            let r = ranks[lo + j] as usize;
            if !dup(r) {
                j += 1;
                continue;
            }
            let first = j;
            while lo + j + 1 < hi && dup(ranks[lo + j + 1] as usize) {
                j += 1;
            }
            let (mut l, mut h) = (r, r);
            while l > 0 && a.lcp[l] >= min {
                l -= 1;
            }
            while h + 1 < n && a.lcp[h + 1] >= min {
                h += 1;
            }
            let mut others: Vec<(u32, u32)> = (l..=h).filter(|&x| x != r).map(|x| a.entries[x]).collect();
            others.sort_unstable();
            out.push(Block { doc: d as u32, first: first as u32, end: (j as u32) + min, others });
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multi::Docs;

    struct Strs(Vec<Vec<u32>>);

    impl Docs for Strs {
        fn count(&self) -> usize {
            self.0.len()
        }
        fn doc_len(&self, d: usize) -> usize {
            self.0[d].len()
        }
        fn at(&self, d: usize, j: usize) -> u32 {
            self.0[d][j]
        }
        fn upper(&self) -> u32 {
            100
        }
    }

    #[test]
    fn a_shared_run_is_one_block_in_each_copy() {
        let shared: Vec<u32> = (10..50).collect();
        let a: Vec<u32> = [vec![1, 2], shared.clone(), vec![3]].concat();
        let b: Vec<u32> = [vec![4], shared.clone(), vec![5, 6]].concat();
        let c: Vec<u32> = shared[..29].to_vec();
        let docs = Strs(vec![a, b, c]);
        let s = MultiSuffixArray::new(&docs);
        let got: Vec<_> = blocks(&s, 30).into_iter().map(|b| (b.doc, b.first, b.end, b.others)).collect();
        assert_eq!(got, vec![(0, 2, 42, vec![(1, 1)]), (1, 1, 41, vec![(0, 2)])]);
    }

    #[test]
    fn a_block_twice_in_one_document() {
        let shared: Vec<u32> = (10..45).collect();
        let a: Vec<u32> = [shared.clone(), vec![1, 2, 3], shared.clone()].concat();
        let docs = Strs(vec![a]);
        let s = MultiSuffixArray::new(&docs);
        let got: Vec<_> = blocks(&s, 30).into_iter().map(|b| (b.first, b.end, b.others)).collect();
        assert_eq!(got, vec![(0, 35, vec![(0, 38)]), (38, 73, vec![(0, 0)])]);
    }
}
