//! A projection's numbered nodes and tokens as a rank index over the tape (decision 0353 step 2): no per-node array.
//!
//! A projection numbers some of a document's tape nodes: its nodes (a local id each, in preorder) and, in `Shape::Core`,
//! its tokens (the anonymous children with text of its nodes). Which ones is a function of the codes and the parents,
//! derived once by the projection's walk. It is kept as bits, 64 tape nodes a group, with the counts before each group,
//! so:
//! - a local id is a node's rank among the numbered nodes (`rank`), and a tape node's local id is its rank when its bit
//!   is set (`local`);
//! - the tape node of a local id is a select (`select`): a sample gives the group of every 64th item, and the bits from
//!   there;
//! - a pass over the nodes walks the set bits (`iter`).
//!
//! Each group is three words: the nodes' bits, the tokens' bits, and the nodes and tokens before it (low and high 32
//! bits). 24 bytes for 64 tape nodes, in the batch's block beside the projection's other cells. `local`, `core` and `up`
//! were 12 bytes a node.

use super::cells::{Block, Cells};
use std::sync::Arc;

/// What a numbered tape node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Node = 0,
    Token = 1,
}

/// Items a select sample stands for.
const SAMPLE: u32 = 64;

pub struct Ranks {
    /// The first group's number (its first tape node over 64).
    first: u32,
    /// Three words a group: bits of the nodes, bits of the tokens, the nodes before | the tokens before << 32. One
    /// group past the last numbered item, so a rank at the end reads a group.
    groups: Cells<[u64; 3]>,
    /// The group of item `SAMPLE * k`, by item.
    samples: [Cells<u32>; 2],
    count: [u32; 2],
}

/// The bits being set by a projection's walk: `mark`, and `has` for a tape node marked before.
pub struct Marks<'a> {
    first: u32,
    groups: &'a mut [[u64; 3]],
}

impl Marks<'_> {
    #[inline]
    pub fn mark(&mut self, item: Item, x: u32) {
        self.groups[(x / 64 - self.first) as usize][item as usize] |= 1 << (x % 64);
    }
    #[inline]
    pub fn has(&self, item: Item, x: u32) -> bool {
        (x / 64).checked_sub(self.first).and_then(|g| self.groups.get(g as usize)).is_some_and(|g| g[item as usize] >> (x % 64) & 1 != 0)
    }
}

impl Ranks {
    /// The numbered items among tape nodes `a..b`, marked by `walk`, in `block`.
    pub fn build(block: &Arc<Block>, a: u32, b: u32, walk: impl FnOnce(&mut Marks<'_>)) -> Self {
        let first = a / 64;
        let n = (b.div_ceil(64).max(first) - first) as usize + 1;
        let mut count = [0u32; 2];
        let groups = Cells::build(block, n, |groups| {
            walk(&mut Marks { first, groups });
            for g in groups.iter_mut() {
                g[2] = u64::from(count[0]) | u64::from(count[1]) << 32;
                count[0] += g[0].count_ones();
                count[1] += g[1].count_ones();
            }
            n
        })
        .expect("a batch's block holds its projection");
        let sample = |item: Item| {
            Cells::build(block, count[item as usize].div_ceil(SAMPLE) as usize, |out| {
                let mut k = 0;
                for (i, g) in groups.iter().enumerate() {
                    let before = (g[2] >> (32 * item as u32)) as u32;
                    let after = before + g[item as usize].count_ones();
                    while k < out.len() && (SAMPLE * k as u32) < after {
                        out[k] = i as u32;
                        k += 1;
                    }
                }
                k
            })
            .expect("a batch's block holds its projection")
        };
        let samples = [sample(Item::Node), sample(Item::Token)];
        Ranks { first, groups, samples, count }
    }

    /// The numbered items of `item`.
    #[inline]
    pub fn count(&self, item: Item) -> u32 {
        self.count[item as usize]
    }

    #[inline]
    fn before(&self, g: usize, item: Item) -> u32 {
        (self.groups[g][2] >> (32 * item as u32)) as u32
    }

    /// Whether tape node `x` is a numbered `item`.
    #[inline]
    pub fn is(&self, item: Item, x: u32) -> bool {
        (x / 64).checked_sub(self.first).and_then(|g| self.groups.get(g as usize)).is_some_and(|g| g[item as usize] >> (x % 64) & 1 != 0)
    }

    /// Numbered `item`s before tape node `x`.
    #[inline]
    pub fn rank(&self, item: Item, x: u32) -> u32 {
        let Some(g) = (x / 64).checked_sub(self.first) else { return 0 };
        match self.groups.get(g as usize) {
            Some(group) => self.before(g as usize, item) + (group[item as usize] & ((1u64 << (x % 64)) - 1)).count_ones(),
            None => self.count(item),
        }
    }

    /// Tape node `x`'s number among the `item`s, if it is one.
    #[inline]
    pub fn local(&self, item: Item, x: u32) -> Option<u32> {
        self.is(item, x).then(|| self.rank(item, x))
    }

    /// The tape node of `item` number `r`: from the sample's group, forward.
    #[inline]
    pub fn select(&self, item: Item, r: u32) -> Option<u32> {
        if r >= self.count(item) {
            return None;
        }
        let mut g = self.samples[item as usize][(r / SAMPLE) as usize] as usize;
        loop {
            let m = self.groups[g][item as usize];
            let k = r - self.before(g, item);
            if k < m.count_ones() {
                return Some((self.first + g as u32) * 64 + select64(m, k));
            }
            g += 1;
        }
    }

    /// Every `item`, in order: `(number, tape node)`.
    pub fn iter(&self, item: Item) -> impl Iterator<Item = (u32, u32)> + '_ {
        let first = self.first;
        self.groups.iter().enumerate().flat_map(move |(g, group)| {
            let base = (first + g as u32) * 64;
            let before = (group[2] >> (32 * item as u32)) as u32;
            let mut m = group[item as usize];
            let mut k = before;
            std::iter::from_fn(move || {
                (m != 0).then(|| {
                    let x = base + m.trailing_zeros();
                    m &= m - 1;
                    k += 1;
                    (k - 1, x)
                })
            })
        })
    }

    /// Both items in tape order: `(tape node, item)`.
    pub fn iter_both(&self) -> impl Iterator<Item = (u32, Item)> + '_ {
        let first = self.first;
        self.groups.iter().enumerate().flat_map(move |(g, group)| {
            let base = (first + g as u32) * 64;
            let (nodes, mut m) = (group[0], group[0] | group[1]);
            std::iter::from_fn(move || {
                (m != 0).then(|| {
                    let j = m.trailing_zeros();
                    m &= m - 1;
                    (base + j, if nodes >> j & 1 != 0 { Item::Node } else { Item::Token })
                })
            })
        })
    }

    /// The bytes the index takes.
    pub fn bytes(&self) -> usize {
        24 * self.groups.len() + 4 * (self.samples[0].len() + self.samples[1].len())
    }

    /// The block the index is in.
    pub fn block(&self) -> &Arc<Block> {
        self.groups.block()
    }
}

/// The position of `m`'s set bit `k` (from 0; `k` below its popcount): the byte by the bytes' running popcounts, then
/// the bit within it.
#[inline]
pub fn select64(m: u64, k: u32) -> u32 {
    const ONES: u64 = 0x0101_0101_0101_0101;
    let mut x = m - ((m >> 1) & 0x5555_5555_5555_5555);
    x = (x & 0x3333_3333_3333_3333) + ((x >> 2) & 0x3333_3333_3333_3333);
    x = (x + (x >> 4)) & 0x0f0f_0f0f_0f0f_0f0f;
    // Byte b of `sums`: the set bits of bytes 0..=b.
    let sums = x.wrapping_mul(ONES);
    // The bytes whose running count is at most `k` come before the bit's byte: one high bit for each.
    let k8 = u64::from(k) * ONES;
    let le = ((k8 | 0x8080_8080_8080_8080) - sums) & 0x8080_8080_8080_8080;
    let byte = le.count_ones();
    let before = if byte == 0 { 0 } else { (sums >> (8 * (byte - 1))) & 0xff } as u32;
    let mut b = (m >> (8 * byte)) & 0xff;
    for _ in 0..k - before {
        b &= b - 1;
    }
    8 * byte + b.trailing_zeros()
}
