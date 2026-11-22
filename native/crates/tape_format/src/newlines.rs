//! A document's line index, part of its format 16 tape (decision 0156 §1.6, 0336 §1): the offset of each line
//! terminator's last byte, the terminators its language's ecosystem reads (`Terminators`, decision 0612), in
//! Elias–Fano form. A byte's line is the rank of its offset among them (`tape_line`), so a terminator's bytes are
//! the line's they end; a line's first byte is the one past the terminator before it (`tape_line_start`); its text
//! ends at that terminator's first byte, `length_ending_at` bytes before. The reads are the tape's, written once in
//! CLIF (`rerail_clif::reads`); this module writes the index and says where its parts lie.
//!
//! The index, all little-endian:
//!
//! - five u32s: the newlines' count `m`, the low parts' width `L` in bits, the zero samples' count, the one samples'
//!   count, and the high bits' length in u64 words;
//! - the zero samples, then the one samples: for every `SAMPLE`-th zero (one) of the high bits, a u32 pair: the word
//!   it lies in, and how many zeros (ones) the words before it hold;
//! - the high bits: newline `i`'s offset `o` sets bit `(o >> L) + i`; bucket `h`'s newlines are the ones before the
//!   `h`-th zero;
//! - the low parts: newline `i`'s low `L` bits at bit `i * L`.
//!
//! A word's load past the low parts reads the region's 8 bytes of room.

/// The line terminators of a language's ecosystem: its compiler's, its linters' and its editors' (decision 0612), the
/// one definition of a line and of every place on it. A `\n\r` is two of them wherever `\r` is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Terminators {
    /// ESLint's `\r\n|[\r\n\u2028\u2029]` (`astUtils.createGlobalLinebreakMatcher`), TypeScript's: ECMAScript's
    /// line terminators. And a file of no language's.
    Eslint,
    /// `\r\n`, `\r` and `\n`: CPython's tokenizer and Ruff's (`ruff_source_file`), the JVM languages' lexers, PHP's,
    /// clang's, Swift's, HTML's and CommonMark's, and the Language Server Protocol's.
    Newlines,
    /// `\n` alone, a `\r` before it left out of the line's text: rustc's (`SourceFile::lines`) and Go's (`go/token`),
    /// which their linters report on. A lone `\r` stays inside a line.
    Lf,
    /// Roslyn's (`SyntaxFacts::IsNewLine`): `\r\n`, `\r`, `\n`, U+0085, U+2028 and U+2029.
    Roslyn,
    /// CSS Syntax's (§3.3, its preprocessing): `\r\n`, `\r`, `\n` and a form feed.
    Css,
}

impl Terminators {
    /// The sets' number: `COUNT` of them.
    pub const fn index(self) -> usize {
        match self {
            Self::Eslint => 0,
            Self::Newlines => 1,
            Self::Lf => 2,
            Self::Roslyn => 3,
            Self::Css => 4,
        }
    }
    pub const COUNT: usize = 5;

    /// The set numbered `k` (`index`).
    pub const fn of_index(k: usize) -> Option<Self> {
        match k {
            0 => Some(Self::Eslint),
            1 => Some(Self::Newlines),
            2 => Some(Self::Lf),
            3 => Some(Self::Roslyn),
            4 => Some(Self::Css),
            _ => None,
        }
    }

    /// Each terminator of `bytes`, `(at, length)`, in order: a chunked search for their first bytes (`memchr`),
    /// never a byte at a time.
    pub fn each(self, bytes: &[u8], mut each: impl FnMut(usize, usize)) {
        let mut skip = usize::MAX;
        let mut one = |at: usize| {
            if at == skip {
                return;
            }
            let length = match bytes[at] {
                // `\n` alone ends a line there: a `\r` before it is the terminator's, out of the line's text.
                b'\n' if self == Self::Lf && at > 0 && bytes[at - 1] == b'\r' => {
                    each(at - 1, 2);
                    return;
                }
                b'\n' => 1,
                b'\r' if bytes.get(at + 1) == Some(&b'\n') => {
                    skip = at + 1;
                    2
                }
                b'\r' => 1,
                0x0C => 1,
                // U+0085: C2 85.
                0xC2 if bytes.get(at + 1) == Some(&0x85) => 2,
                // U+2028 and U+2029: E2 80 A8 and E2 80 A9.
                0xE2 if bytes.get(at + 1) == Some(&0x80) && matches!(bytes.get(at + 2), Some(0xA8 | 0xA9)) => 3,
                _ => return,
            };
            each(at, length);
        };
        match self {
            Self::Eslint => memchr::memchr3_iter(b'\n', b'\r', 0xE2, bytes).for_each(&mut one),
            Self::Newlines => memchr::memchr2_iter(b'\n', b'\r', bytes).for_each(&mut one),
            Self::Lf => memchr::memchr_iter(b'\n', bytes).for_each(&mut one),
            Self::Css => memchr::memchr3_iter(b'\n', b'\r', 0x0C, bytes).for_each(&mut one),
            // Four first bytes: two searches, merged in order.
            Self::Roslyn => {
                let mut wide = memchr::memchr_iter(0xC2, bytes).peekable();
                for at in memchr::memchr3_iter(b'\n', b'\r', 0xE2, bytes) {
                    while let Some(c) = wide.next_if(|&c| c < at) {
                        one(c);
                    }
                    one(at);
                }
                wide.for_each(&mut one);
            }
        }
    }
}

impl Terminators {
    /// The bytes a terminator of the set can end with: a producer's candidates (tier 1's class), each kept where
    /// `ends_at` says it ends one.
    pub const fn last_bytes(self) -> &'static [u8] {
        match self {
            Self::Lf => b"\n",
            Self::Newlines => b"\n\r",
            Self::Eslint => &[b'\n', b'\r', 0xA8, 0xA9],
            Self::Roslyn => &[b'\n', b'\r', 0xA8, 0xA9, 0x85],
            Self::Css => &[b'\n', b'\r', 0x0C],
        }
    }

    /// Whether `bytes[at]`, one of `last_bytes`, is the last byte of one of the set's terminators: a `\r` not before
    /// a `\n`, U+2028's and U+2029's `A8`/`A9` after `E2 80`, U+0085's `85` after `C2`; a `\n` or a form feed alone.
    pub fn ends_at(self, bytes: &[u8], at: usize) -> bool {
        let before = |k: usize| at.checked_sub(k).and_then(|i| bytes.get(i).copied());
        match bytes[at] {
            b'\n' => true,
            b'\r' => self != Self::Lf && bytes.get(at + 1) != Some(&b'\n'),
            0xA8 | 0xA9 => matches!(self, Self::Eslint | Self::Roslyn) && before(1) == Some(0x80) && before(2) == Some(0xE2),
            0x85 => self == Self::Roslyn && before(1) == Some(0xC2),
            0x0C => self == Self::Css,
            _ => false,
        }
    }
}

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

/// Every `SAMPLE`-th zero and one of the high bits is sampled.
pub const SAMPLE_SHIFT: u32 = 8;
pub const SAMPLE: usize = 1 << SAMPLE_SHIFT;
/// The index's first fields, u32s: `m`, `L`, zero samples, one samples, high words.
pub const HEAD: usize = 20;

/// The index of `source`'s lines as `terminators` end them: its bytes, without the room a region keeps after them.
pub fn index(source: &[u8], terminators: Terminators) -> Vec<u8> {
    let mut out = Vec::new();
    index_into(&mut out, source, terminators);
    out
}

/// `index`, into `out` (its bytes replaced): a scan of the source for its terminators, each one's last byte.
pub fn index_into(out: &mut Vec<u8>, source: &[u8], terminators: Terminators) {
    let mut ends = Vec::new();
    terminators.each(source, |at, length| ends.push((at + length - 1) as u64));
    index_of(out, source.len() as u64, ends.len() as u64, ends.into_iter());
}

/// The index of `m` newlines at `offsets` (ascending) in a source of `u` bytes, into `out` (its bytes replaced, its
/// room kept): made in place, one pass over the offsets, so a producer that has the newlines (tier 1's bits) needs no
/// scan of the source.
pub fn index_of(out: &mut Vec<u8>, u: u64, m: u64, offsets: impl Iterator<Item = u64>) {
    let len = index_len(u, m);
    out.clear();
    out.resize(len + 8, 0);
    index_in(out, u, m, offsets);
    out.truncate(len);
}

/// The bytes of the index of `m` line ends in a source of `u` bytes (`index_in` takes 8 more: a word's room).
pub fn index_len(u: u64, m: u64) -> usize {
    let (l, words, zs, os) = shape(u, m);
    HEAD + 8 * (zs + os + words) + (m * u64::from(l)).div_ceil(8) as usize
}

/// The index's low parts' width, its high bits' words, and its zero and one samples' counts.
fn shape(u: u64, m: u64) -> (u32, usize, usize, usize) {
    let l = if m == 0 || u <= m { 0 } else { 63 - (u / m).leading_zeros() };
    let buckets = (u >> l) + 1;
    (l, (m + buckets).div_ceil(64) as usize, buckets.div_ceil(SAMPLE as u64) as usize, m.div_ceil(SAMPLE as u64) as usize)
}

/// `index_of`, into `out`: `index_len(u, m) + 8` bytes, zeroed (a room the index is placed in, decision 0612).
pub fn index_in(out: &mut [u8], u: u64, m: u64, offsets: impl Iterator<Item = u64>) {
    let (l, words, zs, os) = shape(u, m);
    let buckets = (u >> l) + 1;
    let bits = m + buckets;
    let low_len = (m * u64::from(l)).div_ceil(8) as usize;
    let (high_at, low_at) = (HEAD + 8 * (zs + os), HEAD + 8 * (zs + os + words));
    debug_assert_eq!(out.len(), low_at + low_len + 8, "an index's room");
    let store = |out: &mut [u8], at: usize, word: u64| out[at..at + 8].copy_from_slice(&word.to_le_bytes());
    // The high bits a word at a time and the low parts a word of bits at a time, each stored once it is whole: the
    // offsets ascend, so every bit of a word comes before the next word's.
    let mask = if l > 0 { (1u64 << l) - 1 } else { 0 };
    let (mut word, mut w_at) = (0u64, 0usize);
    let (mut low, mut low_bits, mut low_byte) = (0u64, 0u32, low_at);
    let mut n = 0u64;
    for o in offsets {
        let at = (o >> l) + n;
        let w = (at / 64) as usize;
        if w != w_at {
            store(out, high_at + 8 * w_at, word);
            (word, w_at) = (0, w);
        }
        word |= 1 << (at % 64);
        if l > 0 {
            let part = o & mask;
            low |= part << low_bits;
            low_bits += l;
            if low_bits >= 32 {
                // Four whole bytes out; the bits past them kept (`l` is at most 32).
                out[low_byte..low_byte + 4].copy_from_slice(&(low as u32).to_le_bytes());
                (low, low_bits, low_byte) = (low >> 32, low_bits - 32, low_byte + 4);
            }
        }
        n += 1;
    }
    debug_assert_eq!(n, m, "the newlines counted are the offsets given");
    if n > 0 {
        store(out, high_at + 8 * w_at, word);
    }
    store(out, low_byte, low);
    // Samples, in one pass over the high words: the word holding every SAMPLE-th zero (one), and the zeros (ones)
    // before that word.
    let word_at = |out: &[u8], at: usize| u64::from_le_bytes(out[at..at + 8].try_into().expect("8 bytes"));
    let (mut kz, mut ko, mut zeros, mut ones) = (0u64, 0u64, 0u64, 0u64);
    let sample = SAMPLE as u64;
    for w in 0..words {
        let x = word_at(out, high_at + 8 * w);
        let valid = if (w + 1) * 64 <= bits as usize { 64 } else { bits - 64 * w as u64 };
        let set = u64::from(x.count_ones());
        let (z_here, o_here) = (valid - set, set);
        while kz * sample < zeros + z_here && kz * sample < buckets {
            let at = HEAD + 8 * kz as usize;
            out[at..at + 4].copy_from_slice(&(w as u32).to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&(zeros as u32).to_le_bytes());
            kz += 1;
        }
        while ko * sample < ones + o_here && ko * sample < m {
            let at = HEAD + 8 * (zs + ko as usize);
            out[at..at + 4].copy_from_slice(&(w as u32).to_le_bytes());
            out[at + 4..at + 8].copy_from_slice(&(ones as u32).to_le_bytes());
            ko += 1;
        }
        zeros += z_here;
        ones += o_here;
    }
    debug_assert_eq!((kz as usize, ko as usize), (zs, os), "the samples' count");
    for (k, v) in [m as u32, l, zs as u32, os as u32, words as u32].into_iter().enumerate() {
        out[4 * k..4 * k + 4].copy_from_slice(&v.to_le_bytes());
    }
}

/// The offsets of the set bits of a bitmap of `u` bytes, bit `i + delay` of word `w` (`word(w)`) byte `64 w + i`: a
/// producer's newline bits as `index_of` takes them, and their count.
pub fn bits_of(u: u64, delay: u32, words: usize, word: impl Fn(usize) -> u64 + Copy) -> (u64, impl Iterator<Item = u64>) {
    let d = u64::from(delay);
    let inside = move |w: usize| {
        let x = word(w);
        // Bits before the source (the delay's) and past its end are none of its bytes.
        let lo = if w == 0 { !0u64 << d.min(63) & if d >= 64 { 0 } else { !0 } } else { !0 };
        let end = u + d;
        let hi = if (w as u64 + 1) * 64 <= end { !0 } else if (w as u64) * 64 >= end { 0 } else { (1u64 << (end - w as u64 * 64)) - 1 };
        x & lo & hi
    };
    let m = (0..words).map(|w| u64::from(inside(w).count_ones())).sum();
    let offsets = (0..words).flat_map(move |w| {
        let mut x = inside(w);
        std::iter::from_fn(move || {
            (x != 0).then(|| {
                let b = u64::from(x.trailing_zeros());
                x &= x - 1;
                w as u64 * 64 + b - d
            })
        })
    });
    (m, offsets)
}

/// The bytes of the index that starts `index` (its first fields say), without the room after it.
pub fn len_of(index: &[u8]) -> Option<usize> {
    let field = |k: usize| index.get(4 * k..4 * k + 4).map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")) as usize);
    let (m, l, zs, os, words) = (field(0)?, field(1)?, field(2)?, field(3)?, field(4)?);
    Some(HEAD + 8 * (zs + os + words) + (m * l).div_ceil(8))
}

/// The newlines' count an index holds (a document has one line more).
pub fn count(index: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(index.get(..4)?.try_into().ok()?))
}

/// The index checked against itself and its source's length: its fields' sizes add up to its length, and its
/// buckets cover the source.
pub fn check(index: &[u8], source_len: u64) -> Result<(), String> {
    let field = |k: usize| index.get(4 * k..4 * k + 4).map(|b| u64::from(u32::from_le_bytes(b.try_into().expect("4 bytes"))));
    let bad = || "a newline index that does not fit its tape".to_owned();
    let (m, l, zs, os, words) = (field(0).ok_or_else(bad)?, field(1).ok_or_else(bad)?, field(2).ok_or_else(bad)?, field(3).ok_or_else(bad)?, field(4).ok_or_else(bad)?);
    if l > 32 || m > source_len {
        return Err(bad());
    }
    let buckets = (source_len >> l) + 1;
    let len = HEAD as u64 + 8 * (zs + os + words) + (m * l).div_ceil(8);
    if len != index.len() as u64 || words != (m + buckets).div_ceil(64) || zs != buckets.div_ceil(SAMPLE as u64) || os != m.div_ceil(SAMPLE as u64) {
        return Err(bad());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Sources whose newlines are dense, sparse, bunched and absent.
    pub(crate) fn sources() -> Vec<Vec<u8>> {
        let mut out = vec![Vec::new(), b"no newline".to_vec(), b"\n".to_vec(), b"a\nb\n".to_vec(), vec![b'\n'; 700]];
        let mut long = Vec::new();
        for k in 0..3_000 {
            long.extend(std::iter::repeat_n(b'x', (k * 37) % 150));
            long.push(b'\n');
            if k % 400 == 0 {
                long.extend(vec![b'\n'; 300]);
            }
        }
        out.push(long);
        out
    }

    /// The index from a bitmap of the newlines (`bits_of`, at any delay) is the source scan's, byte for byte.
    #[test]
    fn an_index_from_bits_is_the_scans() {
        for s in sources() {
            let want = index(&s, Terminators::Lf);
            for delay in [0u32, 1, 5, 63] {
                let total = s.len() + delay as usize;
                let words = total.div_ceil(64) + 1;
                let mut bits = vec![0u64; words];
                for (i, &b) in s.iter().enumerate() {
                    if b == b'\n' {
                        let at = i + delay as usize;
                        bits[at / 64] |= 1 << (at % 64);
                    }
                }
                // Bits outside the source are not its newlines.
                bits[words - 1] |= 1 << 63;
                if delay > 0 {
                    bits[0] |= 1;
                }
                let (m, offsets) = bits_of(s.len() as u64, delay, words, |w| bits[w]);
                let mut got = vec![0xa5; 3];
                index_of(&mut got, s.len() as u64, m, offsets);
                assert_eq!(got, want, "{} bytes, delay {delay}", s.len());
            }
        }
    }

    /// Each set's terminators (decision 0612), and each one's length read back off its last byte, the index's
    /// offset, by the bytes alone: what a line's end is found from.
    #[test]
    fn a_terminator_is_known_by_its_last_byte() {
        let source = "a\r\nb\rc\n\rd\u{85}e\u{2028}f\u{2029}g\x0ch\n".as_bytes();
        let sets = [Terminators::Eslint, Terminators::Newlines, Terminators::Lf, Terminators::Roslyn, Terminators::Css];
        let mut all = Vec::new();
        for set in sets {
            let mut found = Vec::new();
            set.each(source, |at, length| found.push((at, length)));
            for &(at, length) in &found {
                assert_eq!(length_ending_at(source, at + length - 1), length, "{set:?}: the terminator at {at}");
            }
            // The candidates a producer marks (`last_bytes`), kept by `ends_at`, are the terminators' last bytes.
            let kept: Vec<usize> = (0..source.len()).filter(|&p| set.last_bytes().contains(&source[p]) && set.ends_at(source, p)).collect();
            assert_eq!(kept, found.iter().map(|&(at, l)| at + l - 1).collect::<Vec<_>>(), "{set:?}: the candidates kept");
            let index = index(source, set);
            check(&index, source.len() as u64).unwrap();
            assert_eq!(count(&index), Some(found.len() as u32), "{set:?}");
            all.push(found.into_iter().map(|(at, l)| std::str::from_utf8(&source[at..at + l]).unwrap().to_owned()).collect::<Vec<_>>());
        }
        let s = |t: &[&str]| t.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert_eq!(all[0], s(&["\r\n", "\r", "\n", "\r", "\u{2028}", "\u{2029}", "\n"]), "ESLint's");
        assert_eq!(all[1], s(&["\r\n", "\r", "\n", "\r", "\n"]), "the newlines");
        assert_eq!(all[2], s(&["\r\n", "\n", "\n"]), "rustc's and Go's");
        assert_eq!(all[3], s(&["\r\n", "\r", "\n", "\r", "\u{85}", "\u{2028}", "\u{2029}", "\n"]), "Roslyn's");
        assert_eq!(all[4], s(&["\r\n", "\r", "\n", "\r", "\x0c", "\n"]), "CSS's");
    }

    #[test]
    fn every_index_checks_against_its_source() {
        for s in sources() {
            let index = index(&s, Terminators::Lf);
            check(&index, s.len() as u64).unwrap();
            assert_eq!(count(&index), Some(s.iter().filter(|&&b| b == b'\n').count() as u32));
            assert!(check(&index[..index.len() - 1], s.len() as u64).is_err(), "a short index");
        }
    }
}
