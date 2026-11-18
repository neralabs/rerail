//! A projection's numbered nodes and tokens as a rank index over the tape (decision 0353 step 2): no per-node array.
//!
//! A projection numbers some of a document's tape nodes: its nodes (a local id each, in preorder) and, in `Shape::Core`,
//! its tokens (the anonymous children with text of its nodes). Its nodes below the root that hang from their parent
//! under a field are marked too: the `field` view's rows. Which ones is a function of the codes and the parents,
//! derived once by the projection's walk. It is kept as bits, 64 tape nodes a group, with the counts before each group,
//! so:
//! - a local id is a node's rank among the numbered nodes (`rank`), and a tape node's local id is its rank when its bit
//!   is set (`local`);
//! - the tape node of a local id is a select (`select`): a sample gives the group of every 64th item, and the bits from
//!   there;
//! - a pass over the nodes walks the set bits (`iter`).
//!
//! Each group is six words: the nodes', the tokens', the fielded nodes' and the nodes after a sibling's bits, the
//! nodes and tokens before it (low and high 32 bits), and the fielded nodes and nodes after a sibling before it. 48
//! bytes for 64 tape nodes, in the batch's block beside the projection's other cells. `local`, `core`, `up` and `fielded` were up to 16 bytes a node.

use super::cells::{Block, Cells, Desc};
use super::id_reads;
use rerail_clif::ids::{Numbering, Run};
use std::sync::Arc;

/// What a numbered tape node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Item {
    Node = 0,
    Token = 1,
    /// A node below the root that hangs from its parent under a field (the `field` view's rows).
    Field = 2,
    /// A node after a numbered sibling: its parent's child past the first (the `next_sibling` view's rows, by the
    /// later node).
    Sibling = 3,
}

/// The items, as `Item`'s numbers.
const ITEMS: usize = rerail_clif::ids::ITEMS as usize;
/// A group's words: one of bits a kind of item, then the counts before it.
const WORDS: usize = rerail_clif::ids::WORDS as usize;

/// Item `item`'s count before group `g` (the words after the bits).
#[inline]
fn before_of(g: &[u64; WORDS], item: Item) -> u32 {
    match item {
        Item::Node => g[ITEMS] as u32,
        Item::Token => (g[ITEMS] >> 32) as u32,
        Item::Field => g[ITEMS + 1] as u32,
        Item::Sibling => (g[ITEMS + 1] >> 32) as u32,
    }
}

/// Items a select sample stands for.
const SAMPLE: u32 = rerail_clif::ids::SAMPLE;

pub struct Ranks {
    /// The first group's number (its first tape node over 64).
    first: u32,
    /// Six words a group: the bits of each item, the nodes before | the tokens before << 32, the fielded nodes
    /// before | the nodes after a sibling before << 32. One group past the last numbered item, so a rank at the end reads a group.
    groups: Cells<[u64; WORDS]>,
    /// The group of item `SAMPLE * k`, by item.
    samples: [Cells<u32>; ITEMS],
    count: [u32; ITEMS],
    /// The descriptor the reads written once take (`rerail_clif::ids::Numbering`), in the same block: its runs point
    /// at `groups` and `samples`, and the granule table's at the table's columns (`granules::Table::new`).
    desc: Desc<Numbering>,
}

/// The bits being set by a projection's walk: `mark`, and `has` for a tape node marked before.
pub struct Marks<'a> {
    first: u32,
    groups: &'a mut [[u64; WORDS]],
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
        let mut count = [0u32; ITEMS];
        let groups = Cells::build(block, n, |groups| {
            walk(&mut Marks { first, groups });
            for g in groups.iter_mut() {
                g[ITEMS] = u64::from(count[0]) | u64::from(count[1]) << 32;
                g[ITEMS + 1] = u64::from(count[2]) | u64::from(count[3]) << 32;
                for (c, bits) in count.iter_mut().zip(&g[..ITEMS]) {
                    *c += bits.count_ones();
                }
            }
            n
        })
        .expect("a batch's block holds its projection");
        let sample = |item: Item| {
            Cells::build(block, count[item as usize].div_ceil(SAMPLE) as usize, |out| {
                let mut k = 0;
                for (i, g) in groups.iter().enumerate() {
                    let before = before_of(g, item);
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
        let samples = [sample(Item::Node), sample(Item::Token), sample(Item::Field), sample(Item::Sibling)];
        let desc = Desc::<Numbering>::new(block);
        let d = desc.as_ptr();
        // SAFETY: the descriptor is this build's alone, and the runs point at cells of the same block, which live
        // as long as it.
        unsafe {
            (*d).first = first;
            (*d).count = count;
            Run::set(&raw mut (*d).groups, groups.as_ptr(), groups.len());
            for (i, s) in samples.iter().enumerate() {
                Run::set(&raw mut (*d).samples[i], s.as_ptr(), s.len());
            }
        }
        Ranks { first, groups, samples, count, desc }
    }

    /// The descriptor's address, for the reads written once (`super::id_reads`).
    #[inline]
    pub fn desc(&self) -> *const u8 {
        self.desc.as_ptr().cast()
    }

    /// The descriptor, for the granule table to fill its runs (before the projection is shared).
    pub(super) fn desc_mut(&self) -> *mut Numbering {
        self.desc.as_ptr()
    }

    /// The numbered items of `item`.
    #[inline]
    pub fn count(&self, item: Item) -> u32 {
        self.count[item as usize]
    }

    /// Whether tape node `x` is a numbered `item` (`num_is`, written once in CLIF).
    #[inline]
    pub fn is(&self, item: Item, x: u32) -> bool {
        // SAFETY: this index's descriptor, whose runs point at its cells.
        unsafe { id_reads::num_is(self.desc(), item as u32, x) != 0 }
    }

    /// Numbered `item`s before tape node `x` (`num_rank`).
    #[inline]
    pub fn rank(&self, item: Item, x: u32) -> u32 {
        // SAFETY: as `is`.
        unsafe { id_reads::num_rank(self.desc(), item as u32, x) }
    }

    /// Tape node `x`'s number among the `item`s, if it is one.
    #[inline]
    pub fn local(&self, item: Item, x: u32) -> Option<u32> {
        self.is(item, x).then(|| self.rank(item, x))
    }

    /// The tape node of `item` number `r`: from the sample's group, forward (`num_select`).
    #[inline]
    pub fn select(&self, item: Item, r: u32) -> Option<u32> {
        // SAFETY: as `is`.
        let x = unsafe { id_reads::num_select(self.desc(), item as u32, r) };
        (x != u32::MAX).then_some(x)
    }

    /// Every `item`, in order: `(number, tape node)`.
    pub fn iter(&self, item: Item) -> Iter<'_> {
        Iter { groups: &self.groups, item: item as usize, g: 0, base: self.first * 64, bits: 0, k: 0 }
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
        8 * WORDS * self.groups.len() + 4 * self.samples.iter().map(|s| s.len()).sum::<usize>()
    }

    /// The block the index is in.
    pub fn block(&self) -> &Arc<Block> {
        self.groups.block()
    }
}

/// `Ranks::iter`: the set bits of one item, group by group.
pub struct Iter<'a> {
    groups: &'a [[u64; WORDS]],
    item: usize,
    /// The next group to read.
    g: usize,
    /// The tape node of the current group's bit 0.
    base: u32,
    /// The current group's bits not yet given.
    bits: u64,
    /// The next item's number.
    k: u32,
}

impl Iterator for Iter<'_> {
    type Item = (u32, u32);
    #[inline]
    fn next(&mut self) -> Option<(u32, u32)> {
        while self.bits == 0 {
            let group = self.groups.get(self.g)?;
            if self.g > 0 {
                self.base += 64;
            }
            self.bits = group[self.item];
            self.g += 1;
        }
        let x = self.base + self.bits.trailing_zeros();
        self.bits &= self.bits - 1;
        self.k += 1;
        Some((self.k - 1, x))
    }
}
