//! Tier 1's token records: what a lexer gives its parser, a record a token.
//!
//! The extract pass writes two streams, zipped by index: each token's first byte (`starts`)
//! and its last byte (`lasts`), as simdjson's stage 1 writes its structural indexes. The record
//! pass then writes a third column, a code a token, as a lexer gives each token its kind and
//! its word:
//!
//! - **the kind:** the class of its first byte, one table load (`Records::kinds`);
//! - **the word:** which of the program's words its bytes are, if any: its first eight bytes
//!   and its length, hashed to one slot of a table with no two words in a slot
//!   (`WordTable`), then compared. A word past eight bytes compares the rest eight at a time.
//!
//! The pass is a loop over the tokens, never over bytes: each token's work is straight-line
//! word operations. It also checks the two streams agree (as many lasts as starts, each last
//! in its token, each token after the one before); where they do not, the file goes to the
//! fallback, through the totals' flag.
//!
//! - **the marks:** which of the program's marks hold at its first byte (`Records::marks`):
//!   each mark's bit at the start, one load of its mask's word, and (a mark of one word) its
//!   word compared.
//!
//! A code is `kind | word << 8 | marks << 16`, as the lean lexer's token has its kind and word a
//! byte each: the word is its index in `Records::words` plus one, 0 for none; mark `k` is bit
//! `16 + k`. A parser then tests a kind, a word or a mark with one load of the token's code.

/// A block program's records (`BlockProgram::records`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Records {
    /// The extracts of the tokens' first and last bytes (indexes into `extracts`).
    pub starts: u16,
    pub lasts: u16,
    /// Each byte's kind as a token's first byte: 256 of them.
    pub kinds: Vec<u8>,
    /// The words a token's code names, at most 64 bytes each.
    pub words: Vec<Vec<u8>>,
    /// The marks a token's code has, by order: each the output (an index into the program's
    /// outputs) of its bits at the starts, and the word it needs too (its code, 0 for any). At
    /// most `MARKS`.
    pub marks: Vec<(u16, u16)>,
}

/// The most marks a code holds.
pub const MARKS: usize = 16;

/// The most words a code names (a byte: 0 is none).
pub const WORDS: usize = 255;

/// The bit of a code where its marks start.
pub const MARK_SHIFT: u32 = 16;

/// A code's word: 0 for none, else the word's index plus one.
pub fn word_of(code: u32) -> u32 {
    code >> KIND_BITS & 0xff
}

/// The longest word.
pub const MOST: usize = 64;

/// The bits of a code that hold the kind.
pub const KIND_BITS: u32 = 8;

/// A word's first eight bytes, little-endian, zeros past its end.
pub fn key8(word: &[u8]) -> u64 {
    let mut v = 0u64;
    for (i, &b) in word.iter().take(8).enumerate() {
        v |= u64::from(b) << (8 * i);
    }
    v
}

/// The words by slot, a perfect hash: `slot = ((key ^ len) * mult) >> (64 - bits)`. A slot
/// holds its word's key and `len | code << 8`; a word past eight bytes has its other bytes, eight
/// at a time, in `tails` (by its code).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WordTable {
    pub mult: u64,
    pub bits: u32,
    pub keys: Vec<u64>,
    pub meta: Vec<u32>,
    /// Per word (code − 1): its bytes 8.. as seven words, zeros past its end.
    pub tails: Vec<[u64; 7]>,
}

impl WordTable {
    pub fn slot(&self, key: u64, len: u64) -> usize {
        ((key ^ len).wrapping_mul(self.mult) >> (64 - self.bits)) as usize
    }

    /// The table of `words`: the first multiplier, of a fixed sequence, under which no two
    /// share a slot. Refused: two words with the same first eight bytes and length (no
    /// multiplier parts them), or a word past `MOST` bytes or empty.
    pub fn new(words: &[Vec<u8>]) -> Result<Self, String> {
        if words.len() > WORDS {
            return Err(format!("{} record words (at most {WORDS})", words.len()));
        }
        if let Some(w) = words.iter().find(|w| w.is_empty() || w.len() > MOST) {
            return Err(format!("a record's word of {} bytes (1 to {MOST})", w.len()));
        }
        let keys: Vec<(u64, u64)> = words.iter().map(|w| (key8(w), w.len() as u64)).collect();
        let mut seen = std::collections::HashSet::new();
        if let Some(w) = words.iter().zip(&keys).find(|(_, k)| !seen.insert(**k)) {
            return Err(format!("two record words share their first eight bytes and length: {:?}", String::from_utf8_lossy(w.0)));
        }
        let mut bits = 2u32;
        while (1usize << bits) < 4 * words.len().max(1) {
            bits += 1;
        }
        let tails = words
            .iter()
            .map(|w| std::array::from_fn(|j| w.get(8 * (j + 1)..).map_or(0, key8)))
            .collect();
        loop {
            // An odd multiplier a try, from a fixed sequence (the same table on every run).
            let mut m = 0x9e37_79b9_7f4a_7c15u64;
            for _ in 0..4096 {
                m = m.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) | 1;
                let mut table = WordTable { mult: m, bits, keys: vec![0; 1 << bits], meta: vec![0; 1 << bits], tails: Vec::new() };
                let mut ok = true;
                for (i, &(k, len)) in keys.iter().enumerate() {
                    let s = table.slot(k, len);
                    if table.meta[s] != 0 {
                        ok = false;
                        break;
                    }
                    table.keys[s] = k;
                    table.meta[s] = len as u32 | ((i as u32 + 1) << 8);
                }
                if ok {
                    table.tails = tails;
                    return Ok(table);
                }
            }
            bits += 1;
        }
    }

    /// The word of the `len` bytes at `at` in `bytes` (with `MOST` readable bytes past the
    /// token): its code, or 0. Word operations only: one load, a slot, a compare, and for a
    /// long word its other words.
    pub fn word(&self, bytes: &[u8], at: usize, len: usize) -> u32 {
        let load = |o: usize| -> u64 {
            let mut w = [0u8; 8];
            let have = bytes.len().saturating_sub(o).min(8);
            w[..have].copy_from_slice(&bytes[o..o + have]);
            u64::from_le_bytes(w)
        };
        let cut = |w: u64, n: usize| if n >= 8 { w } else { w & ((1u64 << (8 * n)) - 1) };
        let key = cut(load(at), len);
        let s = self.slot(key, len as u64);
        let (k, meta) = (self.keys[s], self.meta[s]);
        if meta == 0 || k != key || (meta & 0xff) as usize != len {
            return 0;
        }
        let code = meta >> 8;
        if len > 8 {
            let tail = &self.tails[code as usize - 1];
            for (j, &t) in tail.iter().enumerate() {
                let from = 8 * (j + 1);
                if from < len && cut(load(at + from), len - from) != t {
                    return 0;
                }
            }
        }
        code
    }
}

/// The record pass over one file (the interpreter's): each token's code, and whether the two
/// streams disagree (then the file goes to the fallback). `outputs` are the file's output words.
pub fn pass(records: &Records, table: &WordTable, bytes: &[u8], starts: &[u32], lasts: &[u32], outputs: &[Vec<u64>]) -> (Vec<u32>, bool) {
    let mut bad = starts.len() != lasts.len();
    let n = starts.len().min(lasts.len());
    let mut codes = Vec::with_capacity(n);
    let mut after = 0u64;
    for k in 0..n {
        let (s, l) = (starts[k] as usize, lasts[k] as usize);
        bad |= l < s || (s as u64) < after;
        after = l as u64 + 1;
        let len = (l + 1).saturating_sub(s);
        let kind = u32::from(records.kinds[usize::from(bytes.get(s).copied().unwrap_or(0))]);
        let word = table.word(bytes, s, len);
        let mut marks = 0u32;
        for (k, &(o, w)) in records.marks.iter().enumerate() {
            let bit = outputs.get(usize::from(o)).and_then(|v| v.get(s / 64)).is_some_and(|x| x >> (s % 64) & 1 == 1);
            if bit && (w == 0 || u32::from(w) == word) {
                marks |= 1 << k;
            }
        }
        codes.push(kind | word << KIND_BITS | marks << MARK_SHIFT);
    }
    (codes, bad)
}
