//! A node as it is named outside a chunk's numbering, and a document's key: their encodings, defined once (plan
//! "Chunk node numbers").
//!
//! - [`Located`]: a node, token or span by its document's key (high 32 bits) and its low bits (low 32 bits). It is
//!   how a node leaves a chunk's per-file part (a finding's location, a document's key through `docof`, a guest's
//!   host place) and how kept facts and the stores name one.
//! - [`DocKey`]: a document's key.
//! - A location value ([`document_of`]): a row's column that names its document, a node's located id or a document's
//!   key, told apart by value: a located id of a document keyed 1 or more is at least `1 << 32`; a document keyed 0
//!   has located ids below it, each taken as its own "document".
//!
//! Every split of a located id into its key and low bits, every join of them, and every test of a location value is
//! here: `scripts/check/check_node_ids.py` refuses a shift by 32 elsewhere in the crates that handle ids, unless it
//! is marked as bits that are no id (`// bits:`).

/// A document's key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct DocKey(pub u32);

/// A node, token or span named by its document's key and its low bits: `key << 32 | low`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Located(pub i64);

/// The low bits' width: a located id is its document's key above them.
pub const LOW_BITS: u32 = 32;

impl Located {
    /// The id of `low` in the document of key `key`.
    #[inline]
    pub const fn new(key: u32, low: u32) -> Located {
        Located((key as i64) << LOW_BITS | low as i64)
    }
    /// `new` of a key held as an `i64` (a relation's cell): its low 32 bits.
    #[inline]
    pub const fn of(key: i64, low: u32) -> Located {
        Located(key << LOW_BITS | low as i64)
    }
    /// Its document's key.
    #[inline]
    pub const fn key(self) -> u32 {
        (self.0 >> LOW_BITS) as u32
    }
    /// Its document's key as a relation's cell holds a key: `None` for a negative value (no id).
    #[inline]
    pub const fn key_of(value: i64) -> Option<u32> {
        if value < 0 { None } else { Some((value >> LOW_BITS) as u32) }
    }
    /// Its document's key, widened: `value >> 32` of any cell (a negative one's is negative).
    #[inline]
    pub const fn key_wide(value: i64) -> i64 {
        value >> LOW_BITS
    }
    /// Its low bits.
    #[inline]
    pub const fn low(self) -> u32 {
        self.0 as u32
    }
    /// The least and greatest located ids of the documents keyed `lo` to `hi`.
    #[inline]
    pub const fn range(lo: i64, hi: i64) -> [i64; 2] {
        [lo << LOW_BITS, (hi << LOW_BITS) | u32::MAX as i64]
    }
}

/// A located id's key times this is its document's first (the SQL route's `doc(n)` is `n / SCALE`).
pub const SCALE: i64 = 1 << LOW_BITS;

/// A place (`.rerail`'s `@place(path, line)`): `-2 - (path << 32 | line)`, the path's symbol atom and the line,
/// below `unset` (-1) and every located id; `None` for a line outside `0 .. 2^32` (`unset`).
#[inline]
pub const fn place(path: i64, line: i64) -> Option<i64> {
    if line < 0 || line >= SCALE {
        return None;
    }
    match path.checked_mul(SCALE) {
        Some(x) => match x.checked_add(line) {
            Some(x) => (-2i64).checked_sub(x),
            None => None,
        },
        None => None,
    }
}

/// A place's path and line (`place`'s parts), `None` where it is no place's encoding.
#[inline]
pub const fn place_parts(value: i64) -> Option<(i64, i64)> {
    match (-2i64).checked_sub(value) {
        Some(n) => Some((n / SCALE, n - n / SCALE * SCALE)),
        None => None,
    }
}

/// Whether a location value is a node's located id (`>= 1 << 32`), not a document's key.
#[inline]
pub const fn is_located(value: i64) -> bool {
    value >= 1 << LOW_BITS
}

/// The document a location value names: a node's located id's key, or the value itself, a document's key.
#[inline]
pub const fn document_of(value: i64) -> i64 {
    if is_located(value) { value >> LOW_BITS } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_located_id_is_its_key_and_low_bits() {
        for (key, low) in [(0u32, 0u32), (1, 0), (7, 9), (u32::MAX >> 1, u32::MAX), (1 << 30, 5)] {
            let id = Located::new(key, low);
            assert_eq!((id.key(), id.low()), (key, low));
            assert_eq!(Located::of(i64::from(key), low), id);
            let [lo, hi] = Located::range(i64::from(key), i64::from(key));
            assert!((lo..=hi).contains(&id.0));
            assert_eq!(Located::key_of(id.0), Some(key));
            assert_eq!(document_of(id.0), if key == 0 { id.0 } else { i64::from(key) }, "{key} {low}");
        }
        assert_eq!(Located::key_of(-3), None);
        assert_eq!(Located::range(2, 3), [2 << 32, (3 << 32) | 0xffff_ffff]);
        assert_eq!(document_of(41), 41, "a document's key names itself");
    }
}
