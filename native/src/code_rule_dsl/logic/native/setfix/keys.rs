//! A lattice relation's key map (decision 0384 step 5): an index of its relation, in its relation's block, from a key
//! to the key's one live row.
//!
//! A lattice relation (decision 0124) holds one live row a key: its last column is the value, the columns before it
//! the key. The map is an open-addressed table of slots, a power of two of them, each `SLOT` words:
//!
//! ```text
//! slot: [row, changes, round]   // row: the key's live row (EMPTY: no key); changes: its value's changes so far
//!                               // (its first value counts one, `widen`'s count); round: the round its row was
//!                               // written in (where the relation keeps levels)
//! ```
//!
//! A key's slot is found from the hash of its columns (`rerail_blocks::key_hash`, the device's `key_hash` too), then
//! the next slots in turn until the key's row (its key columns read in the relation's rows where they lie) or an
//! empty slot. The host's fold (`Rel::fold`) and the device's (`Batch::lattice_fold`) probe the same slots: the map the
//! device grows is the one the host's next fold probes. It grows (every key placed again) past half full.

use super::rows::{BlockVec, Facts, Home};
use super::runs::RowData;
use std::sync::Arc;

/// A slot no key holds.
pub const EMPTY: u32 = u32::MAX;
/// Words a slot: the key's live row, its changes, its round.
pub const SLOT: usize = 3;
/// The fewest slots a map holds.
const LEAST: usize = 16;
/// Column places 0, 1, 2, …: a key of a relation's first columns (`find`), as one of any columns (`find_at`).
const LEADING: [usize; 64] = {
    let mut out = [0; 64];
    let mut c = 0;
    while c < 64 {
        out[c] = c;
        c += 1;
    }
    out
};

/// A lattice relation's key map: see the module's doc.
#[derive(Clone)]
pub struct KeyMap {
    slots: BlockVec<u32>,
    /// Keys held.
    keys: usize,
}

impl KeyMap {
    /// An empty map of room for `keys` keys (twice as many slots, a power of two), near `home`.
    pub fn new(home: Option<&Home>, keys: usize) -> KeyMap {
        let places = (2 * keys).next_power_of_two().max(LEAST);
        KeyMap { slots: BlockVec::from_iter_in(home, std::iter::repeat_n(EMPTY, places * SLOT)), keys: 0 }
    }

    /// A map of the slots `slots` (another writer's to fill, `held`), holding `keys` keys.
    pub fn of(slots: BlockVec<u32>, keys: usize) -> KeyMap {
        debug_assert!((slots.len() / SLOT).is_power_of_two() && slots.len() % SLOT == 0, "a map of {} words", slots.len());
        KeyMap { slots, keys }
    }

    /// Its slots.
    pub fn places(&self) -> usize {
        self.slots.len() / SLOT
    }

    /// Keys held.
    pub fn keys(&self) -> usize {
        self.keys
    }

    /// Its slots' words: where they lie, for a reader that binds the block (the device).
    pub fn words(&self) -> &BlockVec<u32> {
        &self.slots
    }

    /// `keys` keys held: what another writer (the device) filled it with.
    pub fn held(&mut self, keys: usize) {
        debug_assert!(2 * keys <= self.places(), "{keys} keys in {} slots", self.places());
        self.keys = keys;
    }

    /// The block it lies in.
    pub fn facts(&self) -> Option<&Arc<Facts>> {
        self.slots.facts()
    }

    /// The first slot a key of hash `h` is looked for at.
    #[inline]
    fn start(&self, h: u64) -> usize {
        (h as usize) & (self.places() - 1)
    }

    /// The slot of `key` (the first `key.len()` columns of the relation `data`, `k` columns a row): `Ok`, the slot
    /// holding it; `Err`, the empty slot it would go in.
    pub(super) fn find<D: RowData + ?Sized>(&self, data: &D, k: usize, key: &[i64]) -> Result<usize, usize> {
        self.find_at(data, k, &LEADING[..key.len()], key)
    }

    /// The slot of `key`, the values of columns `cols` (in order) of the relation `data`: `find` of any columns, as a
    /// declared key's map probes them (decision 0361 step 6).
    pub(super) fn find_at<D: RowData + ?Sized>(&self, data: &D, k: usize, cols: &[usize], key: &[i64]) -> Result<usize, usize> {
        let mask = self.places() - 1;
        let mut s = self.start(rerail_blocks::key_hash(key));
        // Bounded: a map past half full grows before a key goes in, so an empty slot is always found.
        for _ in 0..self.places() {
            let row = self.slots[s * SLOT];
            if row == EMPTY {
                return Err(s);
            }
            if key.iter().zip(cols).all(|(&v, &c)| data.value(row, k, c) == v) {
                return Ok(s);
            }
            s = (s + 1) & mask;
        }
        unreachable!("a key map with no empty slot")
    }

    /// Slot `s`'s live row.
    #[inline]
    pub fn row(&self, s: usize) -> u32 {
        self.slots[s * SLOT]
    }

    /// Slot `s`'s changes.
    #[inline]
    pub fn changes(&self, s: usize) -> u32 {
        self.slots[s * SLOT + 1]
    }

    /// Slot `s`'s round.
    #[inline]
    pub fn round(&self, s: usize) -> u32 {
        self.slots[s * SLOT + 2]
    }

    /// Slot `s` (`find`'s, held or empty) holding `row`, with `changes` and its `round`; a key new to the map counted.
    pub fn set(&mut self, s: usize, row: u32, changes: u32, round: u32) {
        if self.slots[s * SLOT] == EMPTY {
            self.keys += 1;
        }
        // copy: derived: a key's slot written where it lies.
        self.slots[s * SLOT..s * SLOT + SLOT].copy_from_slice(&[row, changes, round]);
    }

    /// Room for one more key: past half full, every key placed again in twice the slots (its rows' keys read where
    /// they lie, `nkeys` columns of `k`), near `home`.
    pub(super) fn reserve<D: RowData + ?Sized>(&mut self, data: &D, k: usize, nkeys: usize, home: Option<&Home>) {
        self.reserve_at(data, k, &LEADING[..nkeys], home)
    }

    /// `reserve` of a map keyed by columns `cols` (`find_at`).
    pub(super) fn reserve_at<D: RowData + ?Sized>(&mut self, data: &D, k: usize, cols: &[usize], home: Option<&Home>) {
        if 2 * (self.keys + 1) <= self.places() {
            return;
        }
        let mut grown = KeyMap::new(home, 2 * self.places());
        // copy: not block data: one key's values at a time, what the hash reads.
        let mut key = vec![0i64; cols.len()];
        for s in 0..self.places() {
            let row = self.row(s);
            if row == EMPTY {
                continue;
            }
            for (v, &c) in key.iter_mut().zip(cols) {
                *v = data.value(row, k, c);
            }
            let Err(at) = grown.find_at(data, k, cols, &key) else { unreachable!("a key held twice") };
            grown.set(at, row, self.changes(s), self.round(s));
        }
        *self = grown;
    }

    /// Each key's slot: (its live row, its changes, its round).
    pub fn each(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        self.slots.chunks_exact(SLOT).filter(|w| w[0] != EMPTY).map(|w| (w[0], w[1], w[2]))
    }
}

/// A relation's rows pushed since its orders last took them (`Rel::pending`, the dedup test's table until the next
/// wave sorts them in): an open-addressed table of their ids in the relation's block (decision 0384), not on the heap.
/// A slot is two words: the row's id (`EMPTY`: none) and a tag of its hash, so a probe reads a row only on a tag's
/// match. A row is hashed as a key map hashes its keys (`key_hash`), its values read where they lie. It grows (every
/// row placed again) past half full.
#[derive(Default)]
pub struct RowSet {
    slots: BlockVec<u32>,
    rows: usize,
}

/// Words a row set's slot: the row's id, its hash's tag.
const ROW_SLOT: usize = 2;

impl RowSet {
    /// Its slots.
    fn places(&self) -> usize {
        self.slots.len() / ROW_SLOT
    }

    /// Rows held.
    pub fn len(&self) -> usize {
        self.rows
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// The block it lies in.
    pub fn facts(&self) -> Option<&Arc<Facts>> {
        self.slots.facts()
    }

    #[inline]
    fn tag(h: u64) -> u32 {
        // Never `EMPTY`'s value: a tag is any but one.
        (h >> 32) as u32 & !1
    }

    /// A held row equal to `row` and not `skip`'s (rows of `data`, `k` columns a row).
    pub(super) fn find<D: RowData + ?Sized>(&self, data: &D, k: usize, row: &[i64], skip: impl Fn(u32) -> bool) -> Option<u32> {
        if self.rows == 0 {
            return None;
        }
        let h = rerail_blocks::key_hash(row);
        let (mask, tag) = (self.places() - 1, RowSet::tag(h));
        let mut s = (h as usize) & mask;
        loop {
            let id = self.slots[s * ROW_SLOT];
            if id == EMPTY {
                return None;
            }
            if self.slots[s * ROW_SLOT + 1] == tag && !skip(id) && *data.row(id, k) == *row {
                return Some(id);
            }
            s = (s + 1) & mask;
        }
    }

    /// Row `id` of `data` put in: a row it does not hold.
    pub(super) fn insert<D: RowData + ?Sized>(&mut self, data: &D, k: usize, id: u32) {
        self.reserve(data, k, 1);
        self.place(rerail_blocks::key_hash(&data.row(id, k)), id);
        self.rows += 1;
    }

    fn place(&mut self, h: u64, id: u32) {
        let mask = self.places() - 1;
        let mut s = (h as usize) & mask;
        while self.slots[s * ROW_SLOT] != EMPTY {
            s = (s + 1) & mask;
        }
        // copy: derived: a row's slot written where it lies.
        self.slots[s * ROW_SLOT..s * ROW_SLOT + ROW_SLOT].copy_from_slice(&[id, RowSet::tag(h)]);
    }

    /// Room for `extra` more rows of `data`: past half full, every row placed again in a table of room for them, in
    /// their relation's block (`RowData::home`).
    pub(super) fn reserve<D: RowData + ?Sized>(&mut self, data: &D, k: usize, extra: usize) {
        let need = self.rows + extra;
        if 2 * need <= self.places() {
            return;
        }
        let places = (2 * need).next_power_of_two().max(LEAST);
        let old = std::mem::replace(&mut self.slots, BlockVec::from_iter_in(data.home().as_ref(), std::iter::repeat_n(EMPTY, places * ROW_SLOT)));
        for w in old.chunks_exact(ROW_SLOT).filter(|w| w[0] != EMPTY) {
            self.place(rerail_blocks::key_hash(&data.row(w[0], k)), w[0]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{is_dead, Rel};
    use super::*;

    /// Decision 0384 step 5 (R2): a lattice relation's key map lies in its relation's block, grows there past half
    /// full, and holds each key's live row, its changes and its round: the fold's answers, no `HashMap` beside it.
    #[test]
    fn a_lattices_key_map_lies_in_its_relations_block() {
        let mut rel = Rel::new("t", b"ii");
        rel.set_lattice("min", 1, Arc::new(crate::code_rule_dsl::logic::native::Min), None).unwrap();
        // 3,000 keys a wave, each key's value lower in the next wave for every third key.
        let mut want: std::collections::HashMap<i64, (i64, u32)> = std::collections::HashMap::new();
        for wave in 0..3i64 {
            let rows: Vec<i64> = (0..3000i64).flat_map(|key| [key, if key % 3 == 0 { 100 - wave } else { 100 }]).collect();
            rel.fold(&rows);
            for key in 0..3000i64 {
                let v = if key % 3 == 0 { 100 - wave } else { 100 };
                let e = want.entry(key).or_insert((v, 1));
                if v < e.0 {
                    *e = (v, e.1 + 1);
                }
            }
        }
        let lat = rel.lat.as_ref().expect("a lattice");
        let map = &lat.map;
        assert_eq!(map.keys(), 3000);
        assert!(map.places() >= 6000, "grown past half full: {} slots", map.places());
        let home = rel.data.home().expect("a home");
        assert!(map.facts().is_some_and(|f| Arc::ptr_eq(f, home.facts())), "the map lies in its relation's block");
        for key in 0..3000i64 {
            let s = map.find(&rel.data, rel.k, &[key]).expect("each key held");
            let row = map.row(s);
            assert!(!is_dead(&rel.dead, row), "the key's live row");
            assert_eq!((rel.data.value(row, 2, 1), map.changes(s)), want[&key], "key {key}");
        }
        assert_eq!(rel.n - rel.ndead, 3000, "one live row a key");
    }

    /// Decision 0384: the rows a relation holds past its orders (`pending`, the dedup test's until the next wave) are
    /// a table in its block, grown there: each found, a row not pushed not found, a dead one skipped.
    #[test]
    fn a_relations_pending_rows_lie_in_its_block() {
        let mut rel = Rel::new("t", b"ii");
        for i in 0..5000i64 {
            assert!(rel.push(&[i, i * 7]));
        }
        assert!(!rel.push(&[42, 294]), "held: not pushed again");
        assert_eq!(rel.pending.len(), 5000);
        let home = rel.data.home().expect("a home");
        assert!(rel.pending.facts().is_some_and(|f| Arc::ptr_eq(f, home.facts())), "in the relation's block");
        for i in (0..5000i64).step_by(97) {
            assert_eq!(rel.pending.find(&rel.data, 2, &[i, i * 7], |_| false), Some(i as u32));
        }
        assert_eq!(rel.pending.find(&rel.data, 2, &[3, 22], |_| false), None);
        assert_eq!(rel.pending.find(&rel.data, 2, &[3, 21], |id| id == 3), None, "skipped");
    }
}
