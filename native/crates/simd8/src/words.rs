//! The keyword switch: a word's code, from its bytes, in one probe. Every lean parser asks it of each
//! word its stage 1 finds (a word's end is the lowest set bit of the word-stop mask, as simdjson finds
//! its atoms' ends).
//!
//! The probe is simdjson's check of `true`, `false` and `null`, widened to any word list:
//!
//! 1. a length outside the list's shortest and longest word: no word, before any load;
//! 2. one 8-byte load of the word (a short copy only within 8 bytes of the text's end), masked to its
//!    length by one shift, no per-byte loop;
//! 3. for a case-insensitive list (SQL, PHP), ASCII case folded by one OR of `0x20` in each byte
//!    (exact on a word's bytes: letters, digits, `_`, `$`);
//! 4. one multiply hash and the word's one slot (a perfect hash), then one compare of the slot's first
//!    8 bytes and length; the next 8 bytes only for a word past 8 bytes, and the rest (a cold
//!    compare) only past 16.
//!
//! The perfect hash takes one of two shapes, chosen at compile time from the list itself:
//!
//! - **Direct** (`BUCKETS == 0`): the hash of the first 8 bytes and the length, under a multiplier
//!   searched for at compile time so that every word has a slot of its own (the shape the lean
//!   lexers had each built by hand). For a list of up to 181 words whose first 8 bytes and length
//!   tell them apart: the keyword lists of ECMAScript, Java, Rust, C#, PHP and Python.
//! - **Hash and displace** (CHD, as the `phf` crate builds): for a larger list (SQL's 517 words),
//!   or one whose words share their first 8 bytes and length (`current_date`, `current_time`). The
//!   hash covers the first 16 bytes and the last 16; its bucket's displacement, with it, gives the
//!   slot. One load more than the direct shape.
//!
//! A word not in the list fails the compare; its code is 0. A listed word's code is its index in
//! the list the table was built from, so a parser switches on it (`match`, a jump table) or maps it
//! to its own tokens.
//!
//! The table is built at compile time (`Words::build`, a `const fn`) from the parser's word list,
//! and refused at compile time where two words are one (after folding). The sizes come from the list
//! (`slots_for`, `buckets_for`); `keyword_switch!` writes them.

/// The keyword switch of a word list, sized from the list: `keyword_switch!(static NAME = LIST,
/// fold: false);`, `LIST` a constant `&'static [&'static [u8]]` (its word 0 the empty word).
#[macro_export]
macro_rules! keyword_switch {
    ($vis:vis static $name:ident = $list:expr, fold: $fold:literal) => {
        $vis static $name: $crate::words::Words<{ $crate::words::slots_for($list, $fold) }, { $crate::words::buckets_for($list, $fold) }, { $list.len() }, $fold> =
            $crate::words::Words::build($list);
    };
}

/// A slot: its word's first 8 bytes (as loaded and folded), length (0: an empty slot) and code, in
/// 16 bytes, so the probe is one load of the slot and one compare, with no load that waits on
/// another; the word's next 8 bytes are apart (`Words::his`, by code), read only for a word past 8
/// bytes.
#[derive(Clone, Copy)]
#[repr(C, align(16))]
struct Slot {
    lo: u64,
    len: u32,
    code: u32,
}

const EMPTY: Slot = Slot { lo: 0, len: 0, code: 0 };

/// The most words a table holds.
pub const MAX_WORDS: usize = 4096;

/// The word list's words, each up to `MAX_LEN` bytes.
pub const MAX_LEN: usize = 64;

/// The most words the direct shape takes (4,096 slots).
const DIRECT_MAX: usize = 256;

/// The multipliers tried for the direct shape, at each size.
const DIRECT_TRIES: u32 = 4096;

/// The mask of a lane's first `n` bytes, `n` from 1 (one shift, no table).
#[inline(always)]
const fn mask8(n: usize) -> u64 {
    if n >= 8 { u64::MAX } else { u64::MAX >> (64 - 8 * n) }
}

const FOLD: u64 = 0x2020_2020_2020_2020;

/// The words' lane `k` (bytes `8k..8k + 8`), folded where `fold`.
const fn lane(w: &[u8], k: usize, fold: bool) -> u64 {
    let mut x = 0u64;
    let mut i = 8 * k;
    while i < w.len() && i < 8 * k + 8 {
        let mut b = w[i] as u64;
        if fold {
            b |= 0x20;
        }
        x |= b << (8 * (i - 8 * k));
        i += 1;
    }
    x
}

/// The direct shape's slot: the first lane and the length, one multiply.
#[inline(always)]
const fn direct_slot(lo: u64, len: usize, mult: u64, bits: u32) -> usize {
    ((lo ^ (len as u64).wrapping_mul(0x9e37_79b9)).wrapping_mul(mult) >> (64 - bits)) as usize
}

/// The direct shape's multiplier for `words` in `2^bits` slots: the first (of a fixed sequence) under
/// which every word's slot is its own; 0 where none of `DIRECT_TRIES` is.
const fn direct_mult(words: &[&[u8]], fold: bool, bits: u32) -> u64 {
    // Each word's hash input once; a try's slots are marked with the try's number, so no table is
    // cleared between tries.
    let mut key = [0u64; DIRECT_MAX + 1];
    let mut i = 1;
    while i < words.len() {
        key[i] = lane(words[i], 0, fold) ^ (words[i].len() as u64).wrapping_mul(0x9e37_79b9);
        i += 1;
    }
    let mut mark = [0u16; 1 << 12];
    let mut m = 0x8f58_640b_360e_7c81u64;
    let mut t = 1u32;
    while t <= DIRECT_TRIES {
        let mut ok = true;
        let mut i = 1;
        while ok && i < words.len() {
            let s = (key[i].wrapping_mul(m) >> (64 - bits)) as usize;
            if mark[s] == t as u16 {
                ok = false;
            }
            mark[s] = t as u16;
            i += 1;
        }
        if ok {
            return m;
        }
        m = m.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407) | 1;
        t += 1;
    }
    0
}

/// Whether `words`' first lanes and lengths tell them apart (the direct shape's hash sees no more).
const fn first_lanes_differ(words: &[&[u8]], fold: bool) -> bool {
    let mut i = 1;
    while i < words.len() {
        let mut j = 1;
        while j < i {
            if words[i].len() == words[j].len() && lane(words[i], 0, fold) == lane(words[j], 0, fold) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// The direct shape's slots for `words`: 1,024, 2,048 or 4,096 (two bytes each), where a multiplier
/// is found; 0 otherwise.
const fn direct_bits(words: &[&[u8]], fold: bool) -> u32 {
    if words.len() > DIRECT_MAX + 1 || !first_lanes_differ(words, fold) {
        return 0;
    }
    let mut bits = 10;
    while bits <= 12 {
        if direct_mult(words, fold, bits) != 0 {
            return bits;
        }
        bits += 1;
    }
    0
}

/// The slots for `words`: the direct shape's where it holds, else a power of two of at least twice
/// the words.
pub const fn slots_for(words: &[&[u8]], fold: bool) -> usize {
    let bits = direct_bits(words, fold);
    if bits != 0 {
        return 1 << bits;
    }
    let mut s = 16;
    while s < 2 * words.len() {
        s *= 2;
    }
    s
}

/// The displacements for `words`: 0 for the direct shape, else a power of two of about a quarter of
/// the words.
pub const fn buckets_for(words: &[&[u8]], fold: bool) -> usize {
    if direct_bits(words, fold) != 0 {
        return 0;
    }
    let mut b = 4;
    while b * 4 < words.len() {
        b *= 2;
    }
    b
}

/// The hash and displace shape's hash: the first 16 bytes' lanes, its last 16 bytes' lanes (`tail`:
/// 0 for a word of 16 bytes or fewer), and its length.
#[inline(always)]
const fn mix(lo: u64, hi: u64, tail: u64, len: usize) -> u64 {
    // One multiply: a change in an input bit reaches the product's bits above it, so its top half
    // depends on every input bit; the bucket and the slot are taken from there.
    (lo ^ hi.rotate_left(29) ^ tail.rotate_left(47) ^ (len as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)).wrapping_mul(0xbf58_476d_1ce4_e5b9)
}

/// A word's last 16 bytes folded into one lane (`a ^ b.rotate_left(17)` of its two lanes), for a
/// word past 16 bytes; 0 otherwise.
const fn tail_of(w: &[u8], fold: bool) -> u64 {
    if w.len() <= 16 {
        return 0;
    }
    let (_, last) = w.split_at(w.len() - 16);
    lane(last, 0, fold) ^ lane(last, 1, fold).rotate_left(17)
}

/// The slot of hash `h` under displacement `d`, of `2^bits` slots.
#[inline(always)]
const fn slot_of(h: u64, d: u32, bits: u32) -> usize {
    ((((h >> 32) as u32) ^ d.wrapping_mul(0x9e37_79b1)).wrapping_mul(0x85eb_ca6b) >> (32 - bits)) as usize
}

/// The bucket of hash `h`, of `buckets` (a power of two).
#[inline(always)]
const fn bucket_of(h: u64, buckets: usize) -> usize {
    (h >> (64 - buckets.trailing_zeros())) as usize
}

/// A word list's switch: `SLOTS` slots and `BUCKETS` displacements (0: the direct shape), over `N`
/// words; `FOLD_CASE`: ASCII case is folded (a word is found in any case).
///
/// A slot holds its word's key and code (16 bytes): 1,024 slots are 16 KB, in the first-level cache.
pub struct Words<const SLOTS: usize, const BUCKETS: usize, const N: usize, const FOLD_CASE: bool> {
    slots: [Slot; SLOTS],
    /// Each word's second 8 bytes, by its code.
    his: [u64; N],
    disp: [u32; BUCKETS],
    /// The direct shape's multiplier.
    mult: u64,
    words: &'static [&'static [u8]],
    min_len: usize,
    /// The longest word's length less the shortest's.
    span: usize,
}

impl<const SLOTS: usize, const BUCKETS: usize, const N: usize, const FOLD_CASE: bool> Words<SLOTS, BUCKETS, N, FOLD_CASE> {
    /// The switch of `words`: a word's code is its index. An empty word is no word (a list may hold
    /// one at 0, so codes start at 1).
    pub const fn build(words: &'static [&'static [u8]]) -> Self {
        assert!(SLOTS.is_power_of_two() && (BUCKETS == 0 || BUCKETS.is_power_of_two()), "SLOTS and BUCKETS are powers of two");
        assert!(words.len() <= MAX_WORDS, "too many words");
        assert!(words.len() == N, "N is the words' count");
        assert!(2 * words.len() <= SLOTS, "SLOTS is under twice the words");
        let bits = SLOTS.trailing_zeros();
        let (mut min_len, mut max_len) = (usize::MAX, 0);
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            assert!(w.len() <= MAX_LEN, "a word past MAX_LEN bytes");
            if !w.is_empty() {
                if FOLD_CASE {
                    let mut j = 0;
                    while j < w.len() {
                        let c = w[j];
                        assert!(c.is_ascii_alphanumeric() || c == b'_' || c == b'$', "a folded list's words are word bytes");
                        j += 1;
                    }
                }
                if w.len() > max_len {
                    max_len = w.len();
                }
                if w.len() < min_len {
                    min_len = w.len();
                }
            }
            i += 1;
        }
        if max_len == 0 {
            min_len = 1;
        }
        let mut codes = [0u16; SLOTS];
        let mut his = [0u64; N];
        let mut i = 1;
        while i < words.len() {
            his[i] = lane(words[i], 1, FOLD_CASE);
            i += 1;
        }
        let mut disp = [0u32; BUCKETS];
        let mut mult = 0;
        if BUCKETS == 0 {
            mult = direct_mult(words, FOLD_CASE, bits);
            assert!(mult != 0, "no multiplier places the words: build with slots_for and buckets_for");
            let mut i = 1;
            while i < words.len() {
                let w = words[i];
                if !w.is_empty() {
                    let s = direct_slot(lane(w, 0, FOLD_CASE), w.len(), mult, bits);
                    assert!(codes[s] == 0, "two words are one");
                    codes[s] = i as u16;
                }
                i += 1;
            }
        } else {
            Self::displace(words, &mut codes, &mut disp);
        }
        let mut slots = [EMPTY; SLOTS];
        let mut s = 0;
        while s < SLOTS {
            let code = codes[s] as usize;
            if code != 0 {
                let w = words[code];
                slots[s] = Slot { lo: lane(w, 0, FOLD_CASE), len: w.len() as u32, code: code as u32 };
            }
            s += 1;
        }
        Words { slots, his, disp, mult, words, min_len, span: max_len - min_len }
    }

    /// The hash and displace shape's slots and displacements.
    const fn displace(words: &'static [&'static [u8]], slots: &mut [u16; SLOTS], disp: &mut [u32; BUCKETS]) {
        let bits = SLOTS.trailing_zeros();
        let mut hash = [0u64; MAX_WORDS];
        let mut size = [0u32; BUCKETS];
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            if !w.is_empty() {
                hash[i] = mix(lane(w, 0, FOLD_CASE), lane(w, 1, FOLD_CASE), tail_of(w, FOLD_CASE), w.len());
                size[bucket_of(hash[i], BUCKETS)] += 1;
            }
            i += 1;
        }
        // The words by bucket (a counting sort): bucket `b`'s are `order[start[b]..start[b + 1]]`.
        let mut start = [0u32; MAX_WORDS + 1];
        assert!(BUCKETS <= MAX_WORDS, "more buckets than MAX_WORDS");
        let mut b = 0;
        while b < BUCKETS {
            start[b + 1] = start[b] + size[b];
            b += 1;
        }
        let mut order = [0u32; MAX_WORDS];
        let mut fill = [0u32; BUCKETS];
        let mut i = 0;
        while i < words.len() {
            if !words[i].is_empty() {
                let b = bucket_of(hash[i], BUCKETS);
                order[(start[b] + fill[b]) as usize] = i as u32;
                fill[b] += 1;
            }
            i += 1;
        }
        // The buckets, the largest first: each takes the first displacement that puts all its
        // words in free slots, each its own.
        let mut biggest = 0;
        let mut b = 0;
        while b < BUCKETS {
            if size[b] > biggest {
                biggest = size[b];
            }
            b += 1;
        }
        assert!(biggest <= 64, "a bucket of over 64 words");
        let mut n = biggest;
        while n > 0 {
            let mut b = 0;
            while b < BUCKETS {
                if size[b] == n {
                    let (from, to) = (start[b] as usize, start[b + 1] as usize);
                    let mut d = 0u32;
                    loop {
                        assert!(d < 1 << 20, "no displacement places a bucket: two words are one, or SLOTS is too few");
                        let mut ok = true;
                        let mut taken = [0usize; 64];
                        let mut k = from;
                        while ok && k < to {
                            let s = slot_of(hash[order[k] as usize], d, bits);
                            if slots[s] != 0 {
                                ok = false;
                            }
                            let mut j = 0;
                            while ok && j < k - from {
                                if taken[j] == s {
                                    ok = false;
                                }
                                j += 1;
                            }
                            taken[k - from] = s;
                            k += 1;
                        }
                        if ok {
                            let mut k = from;
                            while k < to {
                                slots[taken[k - from]] = order[k] as u16;
                                k += 1;
                            }
                            disp[b] = d;
                            break;
                        }
                        d += 1;
                    }
                }
                b += 1;
            }
            n -= 1;
        }
    }

    /// The code of the word `src[s..e]` (a word: bytes of the grammar's word class), 0 where it is
    /// not in the list.
    #[inline(always)]
    pub fn find(&self, src: &[u8], s: usize, e: usize) -> u32 {
        let len = e - s;
        if len.wrapping_sub(self.min_len) > self.span {
            return 0;
        }
        let m = mask8(len);
        let lo = if FOLD_CASE { (load8(src, s) | FOLD) & m } else { load8(src, s) & m };
        let i = if BUCKETS == 0 {
            direct_slot(lo, len, self.mult, SLOTS.trailing_zeros())
        } else {
            let hi = self.second_lane(src, s, len);
            // A word past 16 bytes: its last 16 bytes too, by an overlapping load (simdjson's way
            // past a vector's width).
            let tail = if len > 16 {
                let (a, b) = (load8(src, e - 16), load8(src, e - 8));
                if FOLD_CASE { (a | FOLD) ^ (b | FOLD).rotate_left(17) } else { a ^ b.rotate_left(17) }
            } else {
                0
            };
            let h = mix(lo, hi, tail, len);
            slot_of(h, self.disp[bucket_of(h, BUCKETS)], SLOTS.trailing_zeros())
        };
        let slot = self.slots[i];
        if slot.lo != lo || slot.len as usize != len {
            return 0;
        }
        // The build gives every filled slot a code below N.
        if len > 8 && (self.his[slot.code as usize] != self.second_lane(src, s, len) || len > 16 && !self.tail_is(slot.code, &src[s + 16..e])) {
            return 0;
        }
        slot.code
    }

    /// The word's bytes 8 to 16, masked and folded (0 for a word of 8 bytes or fewer).
    #[inline(always)]
    fn second_lane(&self, src: &[u8], s: usize, len: usize) -> u64 {
        if len <= 8 {
            return 0;
        }
        let m = mask8(len - 8);
        if FOLD_CASE { (load8(src, s + 8) | FOLD) & m } else { load8(src, s + 8) & m }
    }

    /// Whether word `code`'s bytes past its 16th are `tail`.
    #[cold]
    fn tail_is(&self, code: u32, tail: &[u8]) -> bool {
        let w = &self.words[code as usize][16..];
        if FOLD_CASE { w.eq_ignore_ascii_case(tail) } else { w == tail }
    }

    /// Word `code`'s text.
    pub fn text(&self, code: u32) -> &'static [u8] {
        self.words[code as usize]
    }

    /// Whether the switch has the direct shape (one multiply, no displacement).
    pub const fn is_direct(&self) -> bool {
        BUCKETS == 0
    }
}

/// The 8 bytes at `s` as a little-endian lane; past the text's end, zeros.
#[inline(always)]
fn load8(src: &[u8], s: usize) -> u64 {
    match src.get(s..s + 8) {
        Some(b) => u64::from_le_bytes(b.try_into().expect("8")),
        None => load8_short(src, s),
    }
}

/// `load8` within 8 bytes of the text's end: out of line, so the probe's registers stay few.
#[cold]
#[inline(never)]
fn load8_short(src: &[u8], s: usize) -> u64 {
    let mut b = [0u8; 8];
    let n = src.len().saturating_sub(s).min(8);
    b[..n].copy_from_slice(&src[s..s + n]);
    u64::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Words sharing their first 8 bytes and length (`current_date`, `current_time`): hash and
    // displace.
    const LIST: &[&[u8]] = &[b"", b"select", b"from", b"current_date", b"current_time", b"current_timestamp", b"characteristics_of_a_long_word", b"a", b"_x", b"$1"];
    keyword_switch!(static FOLDED = LIST, fold: true);
    keyword_switch!(static EXACT = LIST, fold: false);
    // Words the first 8 bytes and length tell apart: the direct shape.
    const SMALL: &[&[u8]] = &[b"", b"if", b"else", b"while", b"function", b"instanceof", b"implements", b"a_very_long_keyword_indeed", b"x"];
    keyword_switch!(static DIRECT = SMALL, fold: false);
    keyword_switch!(static DIRECT_FOLDED = SMALL, fold: true);

    fn both<const S: usize, const B: usize, const N: usize, const F: bool>(w: &Words<S, B, N, F>, text: &str) -> u32 {
        // Each word in a longer text, and at the text's very end (the short load).
        let padded = format!("{text} xxxxxxxxxxxxxxxxxxxx");
        let a = w.find(padded.as_bytes(), 0, text.len());
        let b = w.find(text.as_bytes(), 0, text.len());
        assert_eq!(a, b, "{text}: the load at the end differs");
        a
    }

    #[test]
    fn the_shape_is_chosen_from_the_list() {
        assert!(!FOLDED.is_direct() && !EXACT.is_direct());
        assert!(DIRECT.is_direct() && DIRECT_FOLDED.is_direct());
    }

    #[test]
    fn every_word_is_found_and_no_other() {
        for (i, w) in LIST.iter().map(|w| std::str::from_utf8(w).unwrap()).enumerate().skip(1) {
            assert_eq!(both(&FOLDED, w), i as u32, "{w}");
            assert_eq!(both(&FOLDED, &w.to_ascii_uppercase()), i as u32, "{w} upper case");
            assert_eq!(both(&EXACT, w), i as u32, "{w}");
            assert_eq!(both(&EXACT, &w.to_ascii_uppercase()), if w.bytes().any(|b| b.is_ascii_lowercase()) { 0 } else { i as u32 });
        }
        for w in ["selec", "selectx", "current_dat", "current_timestamq", "characteristics_of_a_long_wore", "b", "", "fromm", "_", "x", "current_daTe_"] {
            assert_eq!(both(&FOLDED, w), 0, "{w}");
        }
        assert_eq!(FOLDED.text(3), b"current_date");
        for (i, w) in SMALL.iter().map(|w| std::str::from_utf8(w).unwrap()).enumerate().skip(1) {
            assert_eq!(both(&DIRECT, w), i as u32, "{w}");
            assert_eq!(both(&DIRECT_FOLDED, &w.to_ascii_uppercase()), i as u32, "{w} upper case");
            assert_eq!(both(&DIRECT, &w.to_ascii_uppercase()), 0, "{w} upper case, exact");
        }
        // Same first 8 bytes and length as a word, differing past them; and unlisted lengths.
        for w in ["instanceog", "implementz", "a_very_long_keyword_indeeD", "a_very_long_keyword_indeex", "functio", "functions", "y", "ifx", ""] {
            assert_eq!(both(&DIRECT, w), 0, "{w}");
        }
    }

    #[test]
    fn a_word_inside_a_text() {
        let src = b"x SeLeCt  FROM\tcurrent_TIME";
        assert_eq!(FOLDED.find(src, 2, 8), 1);
        assert_eq!(FOLDED.find(src, 10, 14), 2);
        assert_eq!(FOLDED.find(src, 15, src.len()), 4);
        assert_eq!(FOLDED.find(src, 15, src.len() - 1), 0);
        let src = b"(while)instanceof";
        assert_eq!(DIRECT.find(src, 1, 6), 3);
        assert_eq!(DIRECT.find(src, 7, src.len()), 5);
    }

    #[test]
    fn a_large_list_is_placed() {
        // 2,000 words, many sharing their first 16 bytes.
        let words: Vec<&'static [u8]> = std::iter::once(&b""[..]).chain((1..2000).map(|i| &*Box::leak(format!("keyword_with_a_long_prefix_{i}").into_bytes().into_boxed_slice()))).collect();
        let words: &'static [&'static [u8]] = Box::leak(words.into_boxed_slice());
        let t: Box<Words<4096, 512, 2000, true>> = Box::new(Words::build(words));
        for (i, w) in words.iter().enumerate().skip(1) {
            assert_eq!(t.find(w, 0, w.len()), i as u32);
        }
        assert_eq!(t.find(b"keyword_with_a_long_prefix_2000", 0, 31), 0);
    }
}
