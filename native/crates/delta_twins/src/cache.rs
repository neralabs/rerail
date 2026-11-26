//! An operator's cache (P§2's contract, L5; P§11.3; *0686: Operations* §6.1): the blocks a delta operator takes from
//! its last run and hands on, holding what it reads to go fast and could make again, such as a layer's row ids in
//! another column order. A cache is not a trace: it is one state, mutated in place, never frozen, with no generations.
//! Each of its blocks holds a header naming the state it describes, the digest of the inputs it was made from:
//!
//! - `cache_open` marks the header open before a run's first write. A header that is open (a run died part-way) or
//!   names another state than the run's before is reset to ∅ where it lies: its entries' length to 0.
//! - The operator's operations read and write its entries. `order` adds a layer's row ids in a column order, with
//!   fences, when the cache does not hold them already; a reader finds them by the layer's block id and the columns.
//! - `cache_commit` writes the after state's digest and clears the mark, last.
//!
//! So dropping a cache is always correct, and a run that dies part-way costs the next run the cache and nothing else.
//! The cache holds row ids and keys, never rows: an index in it is a derived fact, not a copy (0353).
//!
//! The block's body: its length; the state's digest (0: ∅'s, a header just made); the open mark; whether the last
//! `cache_open` found the state it was asked for; the bytes its entries take; then the entries, each a header (the
//! layer's block id, the columns' count and the columns, the rows' and fences' counts), its row ids and its fences (the
//! first key column's value at every `FENCE`th row of the order). The entries reach past the body, up to the block's
//! capacity, which the `order` phase grows (`Bound::Capacity(Grow)`).

use crate::{Ctx, Tile, TwinPhase};
use rerail_blocks::gens::{BODY_AT, set_word, word, words, words_mut};
use rerail_blocks::layer::Layer;

crate::params! {
    /// `cache_open`'s state the run starts from; `cache_commit`'s state it made.
    pub struct CacheP {
        pub state: u64,
    }
}

crate::params! {
    /// `order`'s layer, by its block id, and its columns: up to four, two bytes each, the first in the lowest bytes;
    /// `ncols` of them.
    pub struct OrderP {
        pub cols: u64,
        pub ncols: u32,
        pub layer: u32,
    }
}

/// The header's body bytes.
pub const CACHE_BYTES: usize = 40;
const STATE_AT: usize = BODY_AT + 8;
const OPEN_AT: usize = BODY_AT + 16;
const HELD_AT: usize = BODY_AT + 24;
const USED_AT: usize = BODY_AT + 32;
/// Where the entries start.
pub const ENTRIES_AT: usize = BODY_AT + CACHE_BYTES;
/// An entry's header: the layer, the columns' count, four columns, the rows' and the fences' counts.
const ENTRY_BYTES: usize = 32;
/// A fence every so many rows of an order.
pub const FENCE: usize = 64;
/// The cache operations' slots: `cache_open`'s and `cache_commit`'s one, the cache; `order`'s, the layer read and the
/// cache written.
const CACHE: usize = 0;
const LAYER: usize = 0;
const ORDER_CACHE: usize = 1;

/// The state the header names.
pub fn state(cache: &[u8]) -> u64 {
    word::<u64>(cache, STATE_AT)
}

/// Whether a run marked the header open and has not committed.
pub fn is_open(cache: &[u8]) -> bool {
    word::<u64>(cache, OPEN_AT) != 0
}

/// Whether the cache holds `state`: not open, and naming it.
pub fn holds(cache: &[u8], state: u64) -> bool {
    !is_open(cache) && self::state(cache) == state
}

/// After `cache_open`: whether the cache held the state the run starts from (else it was reset to ∅).
pub fn held(cache: &[u8]) -> bool {
    word::<u64>(cache, HELD_AT) != 0
}

/// The bytes its entries take (0: ∅).
pub fn used(cache: &[u8]) -> usize {
    word::<u64>(cache, USED_AT) as usize
}

/// An order the cache holds: a layer's row ids sorted by the columns, and the first column's value at every
/// `FENCE`th of them.
pub struct Order<'a> {
    pub ids: &'a [u32],
    pub fences: &'a [i64],
}

/// The columns `OrderP` packs.
pub fn cols_of(p: &OrderP) -> Vec<u16> {
    (0..p.ncols as usize).map(|i| (p.cols >> (16 * i)) as u16).collect()
}

/// Layer `layer`'s order by `cols`, packed for `OrderP`; at most four columns.
pub fn order_params(layer: u32, cols: &[u16]) -> OrderP {
    assert!(!cols.is_empty() && cols.len() <= 4, "an order of one to four columns");
    OrderP { cols: cols.iter().enumerate().fold(0, |p, (i, &c)| p | (u64::from(c) << (16 * i))), ncols: cols.len() as u32, layer }
}

fn entry_len(n: usize, nf: usize) -> usize {
    ENTRY_BYTES + (4 * n).next_multiple_of(8) + 8 * nf
}

/// One entry of the cache: where it starts, its layer, its columns, its rows' and fences' counts.
pub struct Entry {
    pub at: usize,
    pub layer: u32,
    pub cols: Vec<u16>,
    pub n: usize,
    pub nf: usize,
}

/// The cache's entries, in the order they were added.
pub fn entries(cache: &[u8]) -> impl Iterator<Item = Entry> + '_ {
    let end = ENTRIES_AT + used(cache);
    let mut at = ENTRIES_AT;
    std::iter::from_fn(move || {
        if at >= end {
            return None;
        }
        let layer = word::<u32>(cache, at);
        let ncols = word::<u32>(cache, at + 4) as usize;
        let cols = (0..ncols).map(|i| word::<u16>(cache, at + 8 + 2 * i)).collect();
        let (n, nf) = (word::<u32>(cache, at + 24) as usize, word::<u32>(cache, at + 28) as usize);
        let e = Entry { at, layer, cols, n, nf };
        at += entry_len(n, nf);
        Some(e)
    })
}

/// The order of layer `layer` by `cols`, if the cache holds it.
pub fn order<'a>(cache: &'a [u8], layer: u32, cols: &[u16]) -> Option<Order<'a>> {
    let Entry { at, n, nf, .. } = entries(cache).find(|e| e.layer == layer && e.cols == cols)?;
    let ids = words::<u32>(cache, at + ENTRY_BYTES, n);
    let fences = words::<i64>(cache, at + ENTRY_BYTES + (4 * n).next_multiple_of(8), nf);
    Some(Order { ids, fences })
}

/// How many entries the cache holds.
pub fn count(cache: &[u8]) -> usize {
    entries(cache).count()
}

fn open(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let from = <CacheP as crate::Params>::read(c.params).state;
    let ([], [h]) = c.views([], [CACHE]);
    let was = holds(h, from);
    set_word::<u64>(h, BODY_AT, CACHE_BYTES as u64);
    set_word::<u64>(h, HELD_AT, u64::from(was));
    if !was {
        // ∅, where it lies: the entries' length to 0, and the state ∅'s.
        set_word::<u64>(h, USED_AT, 0);
        set_word::<u64>(h, STATE_AT, 0);
    }
    set_word::<u64>(h, OPEN_AT, 1);
}

fn commit(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let to = <CacheP as crate::Params>::read(c.params).state;
    let ([], [h]) = c.views([], [CACHE]);
    assert!(is_open(h), "a cache committed by the run that opened it");
    set_word::<u64>(h, STATE_AT, to);
    // The mark cleared last: a reader that sees it clear sees the state's digest.
    set_word::<u64>(h, OPEN_AT, 0);
}

/// `order`'s one item: the layer's own rows' ids sorted by the columns (ties by id), with their fences, added to the
/// cache unless it holds them. Past the cache's capacity it writes nothing and asks for room: the phase runs again
/// over the grown block, finds no entry, and adds it.
fn add_order(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let p = <OrderP as crate::Params>::read(c.params);
    let cols = cols_of(&p);
    let layer_id = p.layer;
    let (n, ids, fences) = {
        let l = Layer::new(c.read(LAYER)).expect("a committed layer");
        if order(c.read(ORDER_CACHE), layer_id, &cols).is_some() {
            return;
        }
        let mut ids: Vec<u32> = l.own.ids().collect(); // copy: derived (an index's row ids, written once into the cache)
        ids.sort_by_key(|&id| (cols.iter().map(|&k| l.get(id, k as usize)).collect::<Vec<i64>>(), id));
        let fences: Vec<i64> = ids.iter().step_by(FENCE).map(|&id| l.get(id, cols[0] as usize)).collect(); // copy: derived (the index's fences)
        (ids.len(), ids, fences)
    };
    let at = ENTRIES_AT + used(c.read(ORDER_CACHE));
    let len = entry_len(n, fences.len());
    if !c.reach(ORDER_CACHE, at + len) {
        return;
    }
    let ([], [b]) = c.views([], [ORDER_CACHE]);
    set_word::<u32>(b, at, layer_id);
    set_word::<u32>(b, at + 4, cols.len() as u32);
    for i in 0..4 {
        set_word::<u16>(b, at + 8 + 2 * i, cols.get(i).copied().unwrap_or(0));
    }
    set_word::<u32>(b, at + 24, n as u32);
    set_word::<u32>(b, at + 28, fences.len() as u32);
    words_mut::<u32>(b, at + ENTRY_BYTES, n).copy_from_slice(&ids);
    let fat = at + ENTRY_BYTES + (4 * n).next_multiple_of(8);
    words_mut::<i64>(b, fat, fences.len()).copy_from_slice(&fences);
    // The entry counted last: a reader never finds one half written.
    set_word::<u64>(b, USED_AT, (at + len - ENTRIES_AT) as u64);
}

/// `cache_open`: one item, the header marked open, the cache reset to ∅ when it holds another state.
pub const CACHE_OPEN: &[TwinPhase] = &[TwinPhase { name: "open", items: |_| 1, run: open, again: None }];

/// `cache_commit`: one item, the state written, then the mark cleared.
pub const CACHE_COMMIT: &[TwinPhase] = &[TwinPhase { name: "commit", items: |_| 1, run: commit, again: None }];

/// `order`: one item, a layer's order added to the cache.
pub const ORDER: &[TwinPhase] = &[TwinPhase { name: "order", items: |_| 1, run: add_order, again: None }];
