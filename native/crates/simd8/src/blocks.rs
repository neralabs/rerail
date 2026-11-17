//! Stage 1's output and stage 2's scans over it, for any lexer in simdjson's shape.
//!
//! Stage 1 (a kernel over `Simd8x64`) keeps `N` masks a 64-byte block, a bit a byte. `Masks` holds
//! them. Stage 2 reads them two ways:
//!
//! - **A scan** (`Masks::next`): the next byte from a position whose bit is set (or clear) in one
//!   mask. A string's or a comment's end, a run's end. A trailing-zero count a block.
//! - **A run of tokens** (page 31 of `docs/parser-generators`; simdjson's structural starts): a
//!   block's token starts as one mask (`starts`, `Masks::starts`), each taken by its lowest bit,
//!   each token's end from its run (`run_len`) or its length. Common tokens are written straight
//!   from the masks; a lexer sends the rare ones (strings, comments, numbers) to its scalar code.
//!
//! The masks past the text's end are the lexer's to choose: runs stop there and stops are set, so a
//! scan ends at the text's length at the latest.

/// A text's masks: `N` a 64-byte block.
pub struct Masks<const N: usize> {
    len: usize,
    m: Vec<[u64; N]>,
}

/// Bits `q` to 63 (`q` below 64).
#[inline(always)]
pub fn from(q: usize) -> u64 {
    debug_assert!(q < 64);
    !0u64 << q
}

/// Bits 0 to `q - 1` (`q` up to 64).
#[inline(always)]
pub fn below(q: usize) -> u64 {
    debug_assert!(q <= 64);
    if q >= 64 { !0 } else { (1u64 << q) - 1 }
}

/// Bits `a` to `b - 1` of a block (`a <= b <= 64`).
#[inline(always)]
pub fn between(a: usize, b: usize) -> u64 {
    below(b) & !below(a)
}

/// The run of set bits of `mask` from bit `q` on: its length, up to `64 - q` (a run on past the
/// block is `64 - q`: the caller scans on).
#[inline(always)]
pub fn run_len(mask: u64, q: usize) -> usize {
    debug_assert!(q < 64);
    (!(mask >> q)).trailing_zeros() as usize
}

/// A block's token starts: every byte that is not a gap (`gap`: blanks, line ends) and is not a
/// word byte after a word byte (`word`; `word_in`: the last byte of the block before was one).
/// A punctuator of two bytes or more is a start a byte: its lexer takes the first and steps past
/// the rest.
#[inline(always)]
pub fn starts(word: u64, gap: u64, word_in: bool) -> u64 {
    !gap & !(word & ((word << 1) | u64::from(word_in)))
}

/// A walk over a text's token starts, a block's mask at a time (page 31's run; simdjson's indexer,
/// read in place). It holds the start mask of the block it last read. Its value is a pure function
/// of the text's masks, so a lexer that jumps (a relex, a rewind) needs nothing reset; one made
/// for a batch of tokens costs one block's mask at most.
#[derive(Clone, Copy, Debug)]
pub struct Cursor {
    b: usize,
    m: u64,
}

impl Default for Cursor {
    #[inline(always)]
    fn default() -> Self {
        Cursor { b: usize::MAX, m: 0 }
    }
}

impl<const N: usize> Masks<N> {
    /// Mask `k` of block `b`, `b` below `blocks()` (the scans' loops keep it so).
    #[inline(always)]
    fn at(&self, b: usize, k: usize) -> u64 {
        debug_assert!(b < self.m.len() && k < N);
        // SAFETY: as above.
        unsafe { *self.m.get_unchecked(b).get_unchecked(k) }
    }

    /// The masks of a text of `len` bytes, a block of `N` each.
    pub fn new(len: usize, m: Vec<[u64; N]>) -> Self {
        debug_assert_eq!(m.len(), len.div_ceil(64));
        Masks { len, m }
    }

    /// The text's length.
    #[inline(always)]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the text is empty.
    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// How many blocks the text has.
    #[inline(always)]
    pub fn blocks(&self) -> usize {
        self.m.len()
    }

    /// Mask `k` of block `b`.
    #[inline(always)]
    pub fn mask(&self, b: usize, k: usize) -> u64 {
        self.m[b][k]
    }

    /// Block `b`'s masks.
    #[inline(always)]
    pub fn block(&self, b: usize) -> &[u64; N] {
        &self.m[b]
    }

    /// The first byte from `at` on whose bit in mask `k` is `set` (the text's length: none).
    #[inline(always)]
    pub fn next(&self, at: usize, k: usize, set: bool) -> usize {
        if at >= self.len {
            return self.len;
        }
        let flip = if set { 0 } else { !0 };
        let mut b = at / 64;
        let mut m = (self.at(b, k) ^ flip) & from(at % 64);
        loop {
            if m != 0 {
                return (b * 64 + m.trailing_zeros() as usize).min(self.len);
            }
            b += 1;
            if b >= self.m.len() {
                return self.len;
            }
            m = self.at(b, k) ^ flip;
        }
    }

    /// The first byte in `at..end` whose bit in mask `k` is set (`None`: none). It reads the blocks
    /// of `at..end` only: a gap's line end, say, with no scan on to the next line's.
    #[inline(always)]
    pub fn first_in(&self, at: usize, end: usize, k: usize) -> Option<usize> {
        let end = end.min(self.len);
        if at >= end {
            return None;
        }
        let mut b = at / 64;
        let mut m = self.at(b, k) & from(at % 64);
        loop {
            if m != 0 {
                let i = b * 64 + m.trailing_zeros() as usize;
                return (i < end).then_some(i);
            }
            b += 1;
            if b * 64 >= end {
                return None;
            }
            m = self.at(b, k);
        }
    }

    /// The first token start at `at` or after (`starts` over `word` and `gaps`; the text's length:
    /// none). A byte right at `at` counts only if it is a start: a lexer whose token ended inside
    /// a word run checks that byte itself.
    #[inline(always)]
    pub fn next_start<const G: usize>(&self, cur: &mut Cursor, at: usize, word: usize, gaps: [usize; G]) -> usize {
        let b = at / 64;
        if b >= self.m.len() {
            return self.len;
        }
        if b != cur.b {
            cur.b = b;
            cur.m = self.starts(b, word, &gaps);
        }
        let mut m = cur.m & from(at % 64);
        loop {
            if m != 0 {
                return (cur.b * 64 + m.trailing_zeros() as usize).min(self.len);
            }
            if cur.b + 1 >= self.m.len() {
                return self.len;
            }
            cur.b += 1;
            cur.m = self.starts(cur.b, word, &gaps);
            m = cur.m;
        }
    }

    /// Block `b`'s token starts (`starts`), by its word mask `word` and its gap masks `gaps` (or-ed).
    #[inline(always)]
    pub fn starts(&self, b: usize, word: usize, gaps: &[usize]) -> u64 {
        assert!(b < self.m.len() && word < N && gaps.iter().all(|&k| k < N), "a block and masks of the text");
        let gap = gaps.iter().fold(0, |g, &k| g | self.at(b, k));
        let word_in = b > 0 && self.at(b - 1, word) >> 63 != 0;
        starts(self.at(b, word), gap, word_in)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bits_and_runs() {
        assert_eq!(from(0), !0);
        assert_eq!(from(63), 1 << 63);
        assert_eq!(below(0), 0);
        assert_eq!(below(64), !0);
        assert_eq!(between(2, 5), 0b11100);
        assert_eq!(run_len(0b0111_0000, 4), 3);
        assert_eq!(run_len(0b0111_0000, 3), 0);
        assert_eq!(run_len(!0, 60), 4);
    }

    /// Starts against their definition, byte by byte, over random words and gaps.
    #[test]
    fn starts_are_the_first_byte_of_each_run_and_every_other_non_gap() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..2000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let word = x;
            let gap = x.rotate_left(17) & !word;
            let word_in = x & 1 << 40 != 0;
            let got = starts(word, gap, word_in);
            for i in 0..64 {
                let w = word >> i & 1 != 0;
                let before = if i == 0 { word_in } else { word >> (i - 1) & 1 != 0 };
                let want = gap >> i & 1 == 0 && !(w && before);
                assert_eq!(got >> i & 1 != 0, want, "bit {i}");
            }
        }
    }

    #[test]
    fn scans_cross_blocks_and_stop_at_the_end() {
        let mut m = vec![[0u64; 1]; 3];
        m[2][0] = 1 << 5;
        let masks = Masks::new(150, m);
        assert_eq!(masks.next(3, 0, true), 133);
        assert_eq!(masks.next(134, 0, true), 150);
        assert_eq!(masks.next(10, 0, false), 10);
        assert_eq!(masks.next(200, 0, true), 150);
        assert_eq!(masks.first_in(3, 150, 0), Some(133));
        assert_eq!(masks.first_in(3, 133, 0), None);
        assert_eq!(masks.first_in(134, 400, 0), None);
        let s = Masks::new(70, vec![[!0u64, 0], [1, !1]]);
        assert_eq!(s.starts(1, 0, &[1]), 0, "a word on from the block before starts nothing");
        // Word bytes 0..3 and 70..72, gaps everywhere else: starts at 0 and 70.
        let t = Masks::new(100, vec![[0b111, !0b111], [0b11 << 6, !(0b11u64 << 6)]]);
        let mut cur = Cursor::default();
        assert_eq!(t.next_start(&mut cur, 0, 0, [1]), 0);
        assert_eq!(t.next_start(&mut cur, 1, 0, [1]), 70);
        assert_eq!(t.next_start(&mut cur, 5, 0, [1]), 70, "the cursor goes back a block");
        assert_eq!(t.next_start(&mut cur, 71, 0, [1]), 100);
        assert_eq!(t.next_start(&mut cur, 300, 0, [1]), 100);
    }
}
