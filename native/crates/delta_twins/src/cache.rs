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
//! A per-file operator's entries are partitioned by file (Operations §6.1): a document's sizes, the rows its operator
//! wrote for it last by output port (`sizes`, `keep_sizes`), so its next run opens its outputs' runs at the room it
//! will need; a document that goes takes its entry with it (`forget`), and its room goes to the next.
//!
//! So dropping a cache is always correct, and a run that dies part-way costs the next run the cache and nothing else.
//! The cache holds row ids and keys, never rows: an index in it is a derived fact, not a copy (0353).
//!
//! The block's body: its length; the state's digest (0: ∅'s, a header just made); the open mark; whether the last
//! `cache_open` found the state it was asked for; the bytes its entries take; where its index lies, and how many of
//! the index's slots are filled; the first let-go sizes entry; then the entries, each a header (the layer's block id,
//! the columns' count and the columns, the rows' and fences' counts), its row ids and its fences (the first key
//! column's value at every `FENCE`th row of the order). The entries reach past the body, up to the block's capacity,
//! which the `order` phase grows (`Bound::Capacity(Grow)`).
//!
//! An entry is found by its index (P1: a run's lookups cost what it looks up, not how many entries the cache holds, one
//! a document): an entry of its own (`INDEX_COLS` columns), an open-addressed table of the entries' places by their
//! keys' hash (`key_hash`), probed from the key's slot to an empty one, each slot's entry's key checked (a let-go
//! entry's slot stays, and is passed over). When one more key would fill more than half its slots, a table twice the
//! size is made at the cursor from the entries (`reindex`): each entry is indexed O(1) times amortized. A let-go sizes
//! entry is pushed on a list through its unused columns' words (`FREE_AT`), and the next document's sizes of its
//! length take it.

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
pub const CACHE_BYTES: usize = 64;
pub const STATE_AT: usize = BODY_AT + 8;
pub const OPEN_AT: usize = BODY_AT + 16;
pub const HELD_AT: usize = BODY_AT + 24;
pub const USED_AT: usize = BODY_AT + 32;
/// Where the index lies (0: none, the cache holds no entry), and how many of its slots are filled.
pub const INDEX_AT: usize = BODY_AT + 40;
pub const FILLED_AT: usize = BODY_AT + 48;
/// The first let-go sizes entry (0: none); each one's next at its columns' words (`at + 8`).
pub const FREE_AT: usize = BODY_AT + 56;
/// The index entry's columns' count: none other has more than four.
pub const INDEX_COLS: u32 = 5;
/// The least index's slots.
pub const INDEX_LEAST: usize = 16;
/// Where the entries start.
pub const ENTRIES_AT: usize = BODY_AT + CACHE_BYTES;
/// An entry's header: the layer, the columns' count, four columns, the rows' and the fences' counts.
pub const ENTRY_BYTES: usize = 32;
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

/// An order's columns: at most four (`order_params`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cols {
    n: u8,
    c: [u16; 4],
}

impl Cols {
    pub fn new(cols: &[u16]) -> Cols {
        assert!(cols.len() <= 4, "an order of at most four columns");
        let mut c = [0; 4];
        c[..cols.len()].copy_from_slice(cols);
        Cols { n: cols.len() as u8, c }
    }
}

impl core::ops::Deref for Cols {
    type Target = [u16];

    fn deref(&self) -> &[u16] {
        &self.c[..self.n as usize]
    }
}

/// The columns `OrderP` packs.
pub fn cols_of(p: &OrderP) -> Cols {
    let mut c = Cols { n: p.ncols as u8, c: [0; 4] };
    for (i, x) in c.c.iter_mut().enumerate().take(p.ncols as usize) {
        *x = (p.cols >> (16 * i)) as u16;
    }
    c
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
    pub cols: Cols,
    pub n: usize,
    pub nf: usize,
    /// The index's own entry.
    pub index: bool,
}

/// An entry's key's hash (`(layer, columns)`: a sizes entry's layer is its document, its columns none), the slot its
/// probe starts from. The kernels' (`cache_hash`) is the same.
pub fn key_hash(layer: u32, cols: &[u16]) -> u32 {
    let c = cols.iter().enumerate().fold(0u64, |p, (i, &c)| p | (u64::from(c) << (16 * i)));
    let mut h = (u64::from(layer) ^ ((cols.len() as u64) << 40)).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h ^= c.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    (h >> 32) as u32
}

/// Whether the entry at `at` is a live one of key `(layer, cols)`.
fn is_key(cache: &[u8], at: usize, layer: u32, cols: &[u16]) -> bool {
    word::<u32>(cache, at) == layer && word::<u32>(cache, at + 4) as usize == cols.len() && cols.iter().enumerate().all(|(i, &c)| word::<u16>(cache, at + 8 + 2 * i) == c)
}

/// The index's slots' count, and where they start (none: 0).
fn index(cache: &[u8]) -> (usize, usize) {
    match word::<u64>(cache, INDEX_AT) as usize {
        0 => (0, 0),
        at => (word::<u32>(cache, at + 24) as usize, at + ENTRY_BYTES),
    }
}

/// The entry of key `(layer, cols)`: where it lies, by the index.
pub fn find(cache: &[u8], layer: u32, cols: &[u16]) -> Option<usize> {
    find_probed(cache, layer, cols, &mut 0)
}

/// `find`, its index's slots read counted into `probes` (the validation mode's reads: `Table::touch`).
pub fn find_probed(cache: &[u8], layer: u32, cols: &[u16], probes: &mut u64) -> Option<usize> {
    let (n, slots) = index(cache);
    if n == 0 {
        return None;
    }
    let mut i = key_hash(layer, cols) as usize & (n - 1);
    loop {
        *probes += 1;
        match word::<u32>(cache, slots + 4 * i) as usize {
            0 => return None,
            at if is_key(cache, at, layer, cols) => return Some(at),
            _ => i = (i + 1) & (n - 1),
        }
    }
}

/// Whether indexing one more key makes a new index (more than half its slots filled).
fn grows(cache: &[u8]) -> bool {
    2 * (word::<u64>(cache, FILLED_AT) as usize + 1) > index(cache).0
}

/// The bytes a new index takes at the cursor before one more key is indexed (0: none is made).
pub fn index_room(cache: &[u8]) -> usize {
    if grows(cache) { ENTRY_BYTES + 4 * (2 * index(cache).0).max(INDEX_LEAST) } else { 0 }
}

/// The entry at `at` indexed by its key, in the index's first empty slot from its key's; room made first (`reindex`).
pub fn index_one(b: &mut [u8], at: usize) {
    let (n, slots) = index(b);
    let ncols = word::<u32>(b, at + 4) as usize;
    let cols: [u16; 4] = core::array::from_fn(|i| word::<u16>(b, at + 8 + 2 * i));
    let mut i = key_hash(word::<u32>(b, at), &cols[..ncols]) as usize & (n - 1);
    while word::<u32>(b, slots + 4 * i) != 0 {
        i = (i + 1) & (n - 1);
    }
    set_word::<u32>(b, slots + 4 * i, at as u32);
    set_word::<u64>(b, FILLED_AT, word::<u64>(b, FILLED_AT) + 1);
}

/// Room in the index for one more key (`index_room` bytes at the cursor, reached first): when it would be over half
/// full, a new index twice the size at the cursor, of every live entry in the cache's order. Made before the entry it
/// makes room for is written (an entry appended lies past it, and is indexed once, by `index_one`).
pub fn reindex(b: &mut [u8]) {
    if !grows(b) {
        return;
    }
    let n = (2 * index(b).0).max(INDEX_LEAST);
    let at = ENTRIES_AT + used(b);
    set_word::<u32>(b, at, 0);
    set_word::<u32>(b, at + 4, INDEX_COLS);
    for i in 0..4 {
        set_word::<u16>(b, at + 8 + 2 * i, 0);
    }
    set_word::<u32>(b, at + 24, n as u32);
    set_word::<u32>(b, at + 28, 0);
    // An empty slot is 0: the slots are written before they are read.
    words_mut::<u32>(b, at + ENTRY_BYTES, n).fill(0);
    set_word::<u64>(b, USED_AT, (at + ENTRY_BYTES + 4 * n - ENTRIES_AT) as u64);
    set_word::<u64>(b, INDEX_AT, at as u64);
    set_word::<u64>(b, FILLED_AT, 0);
    let mut e = ENTRIES_AT;
    while e < at {
        let (layer, ncols) = (word::<u32>(b, e), word::<u32>(b, e + 4));
        let len = entry_len(word::<u32>(b, e + 24) as usize, word::<u32>(b, e + 28) as usize);
        if ncols <= 4 && layer != DEAD {
            index_one(b, e);
        }
        e += len;
    }
}

/// The cache's entries, in the order they were added.
pub fn entries(cache: &[u8]) -> impl Iterator<Item = Entry> + '_ {
    let end = ENTRIES_AT + used(cache);
    let mut at = ENTRIES_AT;
    core::iter::from_fn(move || {
        if at >= end {
            return None;
        }
        let layer = word::<u32>(cache, at);
        let ncols = (word::<u32>(cache, at + 4) as usize).min(4);
        let mut cols = Cols { n: ncols as u8, c: [0; 4] };
        for (i, x) in cols.c.iter_mut().enumerate().take(ncols) {
            *x = word::<u16>(cache, at + 8 + 2 * i);
        }
        let (n, nf) = (word::<u32>(cache, at + 24) as usize, word::<u32>(cache, at + 28) as usize);
        let e = Entry { at, layer, cols, n, nf, index: word::<u32>(cache, at + 4) == INDEX_COLS };
        at += entry_len(n, nf);
        Some(e)
    })
}

/// The order of layer `layer` by `cols`, if the cache holds it.
pub fn order<'a>(cache: &'a [u8], layer: u32, cols: &[u16]) -> Option<Order<'a>> {
    let at = find(cache, layer, cols)?;
    let (n, nf) = (word::<u32>(cache, at + 24) as usize, word::<u32>(cache, at + 28) as usize);
    let ids = words::<u32>(cache, at + ENTRY_BYTES, n);
    let fences = words::<i64>(cache, at + ENTRY_BYTES + (4 * n).next_multiple_of(8), nf);
    Some(Order { ids, fences })
}

/// A sizes entry's layer word when its document went: room for the next document's.
pub const DEAD: u32 = u32::MAX;

/// Document `d`'s sizes, if the cache holds them: the rows its operator wrote for it, by output port. A sizes entry
/// has no columns (an order has one to four); its layer word is the document.
pub fn sizes(cache: &[u8], d: u32) -> Option<&[u32]> {
    sizes_probed(cache, d, &mut 0)
}

/// `sizes`, its probes counted (`find_probed`).
pub fn sizes_probed<'a>(cache: &'a [u8], d: u32, probes: &mut u64) -> Option<&'a [u32]> {
    let at = find_probed(cache, d, &[], probes)?;
    Some(words::<u32>(cache, at + ENTRY_BYTES, word::<u32>(cache, at + 24) as usize))
}

/// Document `d`'s entry let go, if the cache holds one (a document that goes takes its entry with it).
pub fn forget(cache: &mut [u8], d: u32) -> u64 {
    let mut probes = 0;
    if let Some(at) = find_probed(cache, d, &[], &mut probes) {
        set_word::<u32>(cache, at, DEAD);
        // On the let-go list, through its columns' words (a sizes entry has none).
        set_word::<u32>(cache, at + 8, word::<u64>(cache, FREE_AT) as u32);
        set_word::<u64>(cache, FREE_AT, at as u64);
    }
    probes
}

/// Writes document `d`'s sizes into slot `slot`'s cache: over its entry, or a let-go one of the same length, in
/// place; else appended at the cursor. False past the cache's capacity, room asked (`Ctx::reach`): the phase runs
/// again over the grown block, and writes the same.
pub fn keep_sizes(c: &mut Ctx<'_, '_, '_>, slot: usize, d: u32, counts: &[u32]) -> bool {
    let mut probes = 0;
    let (own, free, room) = {
        let b = c.read(slot);
        let fits = |at: usize| at != 0 && word::<u32>(b, at + 24) as usize == counts.len();
        let own = find_probed(b, d, &[], &mut probes).filter(|&at| fits(at));
        let free = Some(word::<u64>(b, FREE_AT) as usize).filter(|&at| own.is_none() && fits(at));
        // A new key: the index's room, and the entry's when it is appended.
        let room = match (own, free) {
            (Some(_), _) => 0,
            (None, Some(_)) => index_room(b),
            (None, None) => index_room(b) + entry_len(counts.len(), 0),
        };
        (own, free, room)
    };
    c.table.touch(probes);
    if room > 0 && !c.reach(slot, ENTRIES_AT + used(c.read(slot)) + room) {
        return false;
    }
    let ([], [b]) = c.views([], [slot]);
    if own.is_none() {
        reindex(b);
    }
    if let Some(at) = free {
        set_word::<u64>(b, FREE_AT, u64::from(word::<u32>(b, at + 8)));
    }
    let (at, append) = match own.or(free) {
        Some(at) => (at, false),
        None => (ENTRIES_AT + used(b), true),
    };
    put_sizes(b, at, d, counts, append);
    if own.is_none() {
        index_one(b, at);
    }
    true
}

/// Document `d`'s sizes entry written at `at` of the cache `b`, its room there; counted in the cache's bytes when it
/// is `append`ed at the cursor (`ENTRIES_AT + used`).
pub fn put_sizes(b: &mut [u8], at: usize, d: u32, counts: &[u32], append: bool) {
    set_word::<u32>(b, at + 4, 0);
    for i in 0..4 {
        set_word::<u16>(b, at + 8 + 2 * i, 0);
    }
    set_word::<u32>(b, at + 24, counts.len() as u32);
    set_word::<u32>(b, at + 28, 0);
    words_mut::<u32>(b, at + ENTRY_BYTES, counts.len()).copy_from_slice(counts);
    // The document named last, and an appended entry counted last: a reader never finds one half written.
    set_word::<u32>(b, at, d);
    if append {
        set_word::<u64>(b, USED_AT, (at + entry_len(counts.len(), 0) - ENTRIES_AT) as u64);
    }
}

/// Document `d`'s sizes appended at the cursor and indexed (a cache made where it lies, a test's case): its room, and
/// the index's, the block's already.
pub fn append_sizes(b: &mut [u8], d: u32, counts: &[u32]) {
    reindex(b);
    let at = ENTRIES_AT + used(b);
    put_sizes(b, at, d, counts, true);
    index_one(b, at);
}

/// How many entries the cache holds.
pub fn count(cache: &[u8]) -> usize {
    entries(cache).filter(|e| !e.index).count()
}

fn open(c: &mut Ctx<'_, '_, '_>, _: Tile) {
    let from = <CacheP as crate::Params>::read(c.params).state;
    let ([], [h]) = c.views([], [CACHE]);
    let was = holds(h, from);
    set_word::<u64>(h, BODY_AT, CACHE_BYTES as u64);
    set_word::<u64>(h, HELD_AT, u64::from(was));
    if !was {
        // ∅, where it lies: the entries' length to 0, no index and no let-go entry, and the state ∅'s.
        set_word::<u64>(h, USED_AT, 0);
        set_word::<u64>(h, INDEX_AT, 0);
        set_word::<u64>(h, FILLED_AT, 0);
        set_word::<u64>(h, FREE_AT, 0);
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
    let mut probes = 0;
    let held = find_probed(c.read(ORDER_CACHE), layer_id, &cols, &mut probes).is_some();
    c.table.touch(probes);
    if held {
        return;
    }
    let n = Layer::new(c.read(LAYER)).expect("a committed layer").n();
    let nf = n.div_ceil(FENCE);
    let len = entry_len(n, nf);
    let (cursor, room) = (ENTRIES_AT + used(c.read(ORDER_CACHE)), index_room(c.read(ORDER_CACHE)));
    if !c.reach(ORDER_CACHE, cursor + room + len) {
        return;
    }
    let ([lb], [b]) = c.views([LAYER], [ORDER_CACHE]);
    let l = Layer::new(lb).expect("a committed layer");
    // The index's room first: the entry lies past it.
    reindex(b);
    let at = ENTRIES_AT + used(b);
    set_word::<u32>(b, at, layer_id);
    set_word::<u32>(b, at + 4, cols.len() as u32);
    for i in 0..4 {
        set_word::<u16>(b, at + 8 + 2 * i, cols.get(i).copied().unwrap_or(0));
    }
    set_word::<u32>(b, at + 24, n as u32);
    set_word::<u32>(b, at + 28, nf as u32);
    // The layer's own ids, written into the entry where they lie and sorted there by the columns, ties by id (a total
    // order: an unstable sort gives the one answer); then its fences, every `FENCE`th id's first column.
    let ids = words_mut::<u32>(b, at + ENTRY_BYTES, n);
    for (x, id) in ids.iter_mut().zip(l.own.ids()) {
        *x = id;
    }
    let vals = |id: u32| cols.iter().map(move |&k| l.get(id, k as usize));
    ids.sort_unstable_by(|&p, &q| vals(p).cmp(vals(q)).then(p.cmp(&q)));
    let fat = at + ENTRY_BYTES + (4 * n).next_multiple_of(8);
    for f in 0..nf {
        let id = word::<u32>(b, at + ENTRY_BYTES + 4 * f * FENCE);
        set_word::<i64>(b, fat + 8 * f, l.get(id, cols[0] as usize));
    }
    // The entry counted last, then indexed: a reader never finds one half written.
    set_word::<u64>(b, USED_AT, (at + len - ENTRIES_AT) as u64);
    index_one(b, at);
}

/// `cache_open`: one item, the header marked open, the cache reset to ∅ when it holds another state.
pub const CACHE_OPEN: &[TwinPhase] = &[TwinPhase { name: "open", items: |_| 1, run: open, again: None }];

/// `cache_commit`: one item, the state written, then the mark cleared.
pub const CACHE_COMMIT: &[TwinPhase] = &[TwinPhase { name: "commit", items: |_| 1, run: commit, again: None }];

/// `order`: one item, a layer's order added to the cache.
pub const ORDER: &[TwinPhase] = &[TwinPhase { name: "order", items: |_| 1, run: add_order, again: None }];

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::{vec, vec::Vec};

    /// A cache's bytes, ∅: its header's words, then room for `entries` bytes.
    fn empty(entries: usize) -> Vec<u8> {
        vec![0u8; ENTRIES_AT + entries]
    }

    /// Every entry found by its key through the index, as the index is made again past half full; a let-go one found
    /// no more, its slot passed over; and an index's probes a few a lookup however many entries the cache holds.
    #[test]
    fn entries_are_found_by_the_index_whatever_their_count() {
        let mut b = empty(1 << 20);
        let counts = |d: u32| [d, d + 1, d + 2];
        for d in 0..1000u32 {
            append_sizes(&mut b, d * 7919, &counts(d));
        }
        assert_eq!(count(&b), 1000);
        for d in (0..1000u32).filter(|d| d % 3 == 0) {
            forget(&mut b, d * 7919);
        }
        let mut probes = 0;
        for d in 0..1000u32 {
            let s = sizes_probed(&b, d * 7919, &mut probes);
            assert_eq!(s, (d % 3 != 0).then_some(&counts(d)[..]), "document {d}");
        }
        assert!(probes < 4 * 1000, "{probes} probes for 1000 lookups");
        // The let-go entries are on the list, the last let go first.
        assert_eq!(word::<u64>(&b, FREE_AT) as usize, find_slot_of_dead(&b, 999 * 7919));
        // An order's key and a sizes key of the same layer are two keys.
        assert!(find(&b, 7919, &[1]).is_none());
    }

    /// Where document `d`'s let-go entry lies: the entry whose sizes are `d`'s first count (`counts`).
    fn find_slot_of_dead(b: &[u8], d: u32) -> usize {
        let first = d / 7919;
        entries(b).find(|e| !e.index && e.layer == DEAD && word::<u32>(b, e.at + ENTRY_BYTES) == first).expect("let go").at
    }
}
