//! The keyword switch: a word's code, from its bytes, in one probe. Every lean parser asks it of each
//! word its stage 1 finds (a word's end is the lowest set bit of the word-stop mask, as simdjson finds
//! its atoms' ends).
//!
//! The probe is simdjson's check of `true`, `false` and `null`, widened to any word list:
//!
//! 1. one 16-byte load of the word (two 8-byte lanes; a short copy only within 16 bytes of the
//!    text's end), masked to its length by a table (`MASKS`), no per-byte loop;
//! 2. for a case-insensitive list (SQL), ASCII case folded by one OR of `0x20` in each byte of the
//!    word (exact on a word's bytes: letters, digits, `_`, `$`);
//! 3. a hash of the two lanes and the length, then a perfect hash by hash and displace (CHD, as the
//!    `phf` crate builds): the hash's bucket gives a displacement, which with the hash gives the
//!    word's one slot;
//! 4. one compare of the slot's 16 bytes and length (and the rest of a word past 16 bytes, rare).
//!
//! A word not in the list fails the compare; its code is 0. A listed word's code is its index in
//! the list the table was built from, so a parser switches on it (`match`, a jump table) or maps it
//! to its own tokens.
//!
//! The table is built at compile time (`Words::build`, a `const fn`) from the parser's word list,
//! and refused at compile time where two words are one (after folding) or the slots are too few.
//! Sizes: `SLOTS` a power of two of at least twice the words, `BUCKETS` a power of two of about a
//! quarter of them (`slots_for`, `buckets_for`).

/// A slot: the word's first 16 bytes (as loaded and folded), its length (0: empty), its code.
#[derive(Clone, Copy)]
struct Slot {
    lo: u64,
    hi: u64,
    len: u32,
    code: u32,
}

const EMPTY: Slot = Slot { lo: 0, hi: 0, len: 0, code: 0 };

/// The most words a table holds.
pub const MAX_WORDS: usize = 4096;

/// Masks of the first `n` bytes of the two lanes, `n` from 0 to 16.
const MASKS: [(u64, u64); 17] = {
    let mut m = [(0u64, 0u64); 17];
    let mut n = 0;
    while n <= 16 {
        let lo = if n >= 8 { u64::MAX } else if n == 0 { 0 } else { u64::MAX >> (64 - 8 * n) };
        let hi = if n >= 16 { u64::MAX } else if n <= 8 { 0 } else { u64::MAX >> (64 - 8 * (n - 8)) };
        m[n] = (lo, hi);
        n += 1;
    }
    m
};

const FOLD: u64 = 0x2020_2020_2020_2020;

/// A power of two of at least twice `n` words.
pub const fn slots_for(n: usize) -> usize {
    let mut s = 16;
    while s < 2 * n {
        s *= 2;
    }
    s
}

/// A power of two of about a quarter of `n` words.
pub const fn buckets_for(n: usize) -> usize {
    let mut b = 4;
    while b * 4 < n {
        b *= 2;
    }
    b
}

/// The word list's words, each up to `MAX_LEN` bytes.
pub const MAX_LEN: usize = 64;

/// The word `w`'s two lanes: its first 16 bytes, folded where `fold`.
const fn lanes(w: &[u8], fold: bool) -> (u64, u64) {
    let mut lo = 0u64;
    let mut hi = 0u64;
    let mut i = 0;
    while i < w.len() && i < 16 {
        let mut b = w[i] as u64;
        if fold {
            b |= 0x20;
        }
        if i < 8 {
            lo |= b << (8 * i);
        } else {
            hi |= b << (8 * (i - 8));
        }
        i += 1;
    }
    (lo, hi)
}

/// The hash of a word's lanes, its last 16 bytes' lanes (`tail`: 0 for a word of 16 bytes or
/// fewer), and its length.
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
    let (a, b) = lanes(last, fold);
    a ^ b.rotate_left(17)
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

/// A word list's switch: `SLOTS` slots and `BUCKETS` displacements; `FOLD_CASE`: ASCII case is
/// folded (a word is found in any case).
pub struct Words<const SLOTS: usize, const BUCKETS: usize, const FOLD_CASE: bool> {
    slots: [Slot; SLOTS],
    disp: [u32; BUCKETS],
    words: &'static [&'static [u8]],
    max_len: usize,
}

impl<const SLOTS: usize, const BUCKETS: usize, const FOLD_CASE: bool> Words<SLOTS, BUCKETS, FOLD_CASE> {
    /// The switch of `words`: a word's code is its index. An empty word is no word (a list may hold
    /// one at 0, so codes start at 1).
    pub const fn build(words: &'static [&'static [u8]]) -> Self {
        assert!(SLOTS.is_power_of_two() && BUCKETS.is_power_of_two(), "SLOTS and BUCKETS are powers of two");
        assert!(words.len() <= MAX_WORDS, "too many words");
        assert!(2 * words.len() <= SLOTS, "SLOTS is under twice the words");
        let bits = SLOTS.trailing_zeros();
        let mut hash = [0u64; MAX_WORDS];
        let mut lane = [(0u64, 0u64); MAX_WORDS];
        let mut size = [0u32; BUCKETS];
        let mut max_len = 0;
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
                lane[i] = lanes(w, FOLD_CASE);
                hash[i] = mix(lane[i].0, lane[i].1, tail_of(w, FOLD_CASE), w.len());
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
        let mut slots = [EMPTY; SLOTS];
        let mut disp = [0u32; BUCKETS];
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
                            if slots[s].len != 0 {
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
                                let i = order[k] as usize;
                                slots[taken[k - from]] = Slot { lo: lane[i].0, hi: lane[i].1, len: words[i].len() as u32, code: i as u32 };
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
        Words { slots, disp, words, max_len }
    }

    /// The code of the word `src[s..e]` (a word: bytes of the grammar's word class), 0 where it is
    /// not in the list.
    #[inline(always)]
    pub fn find(&self, src: &[u8], s: usize, e: usize) -> u32 {
        let len = e - s;
        if len.wrapping_sub(1) >= self.max_len {
            return 0;
        }
        let (lo, hi) = load16(src, s);
        let (ml, mh) = MASKS[if len > 16 { 16 } else { len }];
        let (mut lo, mut hi) = (lo & ml, hi & mh);
        if FOLD_CASE {
            lo |= FOLD & ml;
            hi |= FOLD & mh;
        }
        // A word past 16 bytes: its last 16 bytes too, by an overlapping load (simdjson's way past a
        // vector's width).
        let tail = if len > 16 {
            let (a, b) = load16(src, e - 16);
            if FOLD_CASE { (a | FOLD) ^ (b | FOLD).rotate_left(17) } else { a ^ b.rotate_left(17) }
        } else {
            0
        };
        let h = mix(lo, hi, tail, len);
        let slot = self.slots[slot_of(h, self.disp[bucket_of(h, BUCKETS)], SLOTS.trailing_zeros())];
        if slot.lo != lo || slot.hi != hi || slot.len as usize != len {
            return 0;
        }
        if len > 16 && !self.tail_is(slot.code, &src[s + 16..e]) {
            return 0;
        }
        slot.code
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
}

/// The 16 bytes at `s` as two little-endian lanes; past the text's end, zeros.
#[inline(always)]
fn load16(src: &[u8], s: usize) -> (u64, u64) {
    match src.get(s..s + 16) {
        Some(b) => (u64::from_le_bytes(b[..8].try_into().expect("8")), u64::from_le_bytes(b[8..].try_into().expect("8"))),
        None => {
            let mut b = [0u8; 16];
            let n = src.len().saturating_sub(s).min(16);
            b[..n].copy_from_slice(&src[s..s + n]);
            (u64::from_le_bytes(b[..8].try_into().expect("8")), u64::from_le_bytes(b[8..].try_into().expect("8")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static LIST: &[&[u8]] = &[b"", b"select", b"from", b"current_date", b"current_time", b"current_timestamp", b"characteristics_of_a_long_word", b"a", b"_x", b"$1"];
    static FOLDED: Words<{ slots_for(10) }, { buckets_for(10) }, true> = Words::build(LIST);
    static EXACT: Words<{ slots_for(10) }, { buckets_for(10) }, false> = Words::build(LIST);

    fn code(w: &Words<32, 4, true>, text: &str) -> u32 {
        // Each word in a longer text, and at the text's very end (the short load).
        let padded = format!("{text} xxxxxxxxxxxxxxxxxxxx");
        let a = w.find(padded.as_bytes(), 0, text.len());
        let b = w.find(text.as_bytes(), 0, text.len());
        assert_eq!(a, b, "{text}: the load at the end differs");
        a
    }

    #[test]
    fn every_word_is_found_and_no_other() {
        for (i, w) in LIST.iter().map(|w| std::str::from_utf8(w).unwrap()).enumerate().skip(1) {
            assert_eq!(code(&FOLDED, w), i as u32, "{w}");
            assert_eq!(code(&FOLDED, &w.to_ascii_uppercase()), i as u32, "{w} upper case");
            assert_eq!(EXACT.find(w.as_bytes(), 0, w.len()), i as u32, "{w}");
            assert_eq!(EXACT.find(w.to_ascii_uppercase().as_bytes(), 0, w.len()), if w.bytes().any(|b| b.is_ascii_lowercase()) { 0 } else { i as u32 });
        }
        for w in ["selec", "selectx", "current_dat", "current_timestamq", "characteristics_of_a_long_wore", "b", "", "fromm", "_", "x"] {
            assert_eq!(code(&FOLDED, w), 0, "{w}");
        }
        assert_eq!(FOLDED.text(3), b"current_date");
    }

    #[test]
    fn a_word_inside_a_text() {
        let src = b"x SeLeCt  FROM\tcurrent_TIME";
        assert_eq!(FOLDED.find(src, 2, 8), 1);
        assert_eq!(FOLDED.find(src, 10, 14), 2);
        assert_eq!(FOLDED.find(src, 15, src.len()), 4);
        assert_eq!(FOLDED.find(src, 15, src.len() - 1), 0);
    }

    #[test]
    fn a_large_list_is_placed() {
        // 2,000 words, many sharing their first 16 bytes.
        let words: Vec<&'static [u8]> = (0..2000).map(|i| &*Box::leak(format!("keyword_with_a_long_prefix_{i}").into_bytes().into_boxed_slice())).collect();
        let words: &'static [&'static [u8]] = Box::leak(words.into_boxed_slice());
        let t: Box<Words<{ slots_for(2000) }, { buckets_for(2000) }, true>> = Box::new(Words::build(words));
        for (i, w) in words.iter().enumerate() {
            assert_eq!(t.find(w, 0, w.len()), i as u32);
        }
        assert_eq!(t.find(b"keyword_with_a_long_prefix_2000", 0, 31), 0);
    }
}
