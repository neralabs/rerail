//! A document's line index's reads (decision 0156 §1.6, 0612): the newlines' count it holds, a byte's line, and a
//! terminator's length off the source's bytes. The index's layout and its writer are `rerail_tape_format::newlines`',
//! which re-exports these; a line's first byte is the tape's read (`tape::f16::line_start`).

use crate::tape::{TapeView, f16};

/// The length of the terminator whose last byte is `bytes[last]`, the line index's offset: its bytes alone say it,
/// whatever the set (a set's terminators never end alike), so a line's end is read off the index and the source.
pub fn length_ending_at(bytes: &[u8], last: usize) -> usize {
    let before = |k: usize| last.checked_sub(k).and_then(|at| bytes.get(at).copied());
    match bytes[last] {
        b'\n' if before(1) == Some(b'\r') => 2,
        0x85 if before(1) == Some(0xC2) => 2,
        0xA8 | 0xA9 if before(1) == Some(0x80) && before(2) == Some(0xE2) => 3,
        _ => 1,
    }
}

/// The 0-based line of byte `x`, by an index at the start of `index` (its word's room after it): the line ends before
/// `x`, the tape's read (`tape::f16::line`).
pub fn line(index: &[u8], x: u32) -> u32 {
    f16::line(index, &TapeView::default(), x)
}

/// Line `k`'s (0-based) first byte, by an index at the start of `index` (`tape::f16::line_start`).
pub fn start(index: &[u8], k: u32) -> u32 {
    f16::line_start(index, &TapeView::default(), k)
}

/// The newlines' count an index holds (a document has one line more).
pub fn count(index: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(index.get(..4)?.try_into().ok()?))
}

/// The index's first fields, u32s: `m`, `L`, zero samples, one samples, high words.
pub const HEAD: usize = 20;

/// The bytes of the index that starts `index` (its first fields say), without the room after it.
pub fn len_of(index: &[u8]) -> Option<usize> {
    let field = |k: usize| index.get(4 * k..4 * k + 4).map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")) as usize);
    let (m, l, zs, os, words) = (field(0)?, field(1)?, field(2)?, field(3)?, field(4)?);
    Some(HEAD + 8 * (zs + os + words) + (m * l).div_ceil(8))
}
