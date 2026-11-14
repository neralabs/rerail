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
    /// Each byte's end rule as a token's first byte (decision 0177): an index into `rules`.
    /// Empty: every token's last byte is the `lasts` extract's.
    pub ends: Vec<u8>,
    pub rules: Vec<EndRule>,
    /// The bytes between tokens (decision 0177, stage 2a), an output: with it, the tokens are
    /// the loop's (`walk`), each at the first byte after the one before that it does not hold,
    /// not the starts' extract's.
    pub skip: Option<u16>,
    /// The marks (bits of `marks`) a grammar token has from the bytes before it (decision 0177):
    /// mark `k`'s bit at a token is whether its output holds anywhere after the grammar token
    /// before (an extra's are not) up to its first byte. The other marks are their outputs' bits
    /// at the token's first byte.
    pub gaps: u32,
    /// The mark whose bit tells an extra's token (a comment), if any: an extra has no gap mark.
    pub extra: Option<u8>,
    /// The outputs of the errors the token loop checks (`error … by tokens`): a byte it starts a
    /// token at that no token begins with refuses the file. The CPU makes none of them.
    pub untaken: Vec<u16>,
    /// The gap marks' `unless after` words: mark `k` is none at a token whose grammar token
    /// before has word code `w` (`(k, w)`), as lean's parser asks whether a `,` ends a line.
    pub unless: Vec<(u8, u16)>,
    /// The gap marks (bits of `marks`) with an `else escape`: a bit of one with no grammar token
    /// at or after it (past the last one's start) refuses the file.
    pub unplaced: u32,
    /// The token tests (`escape … at KIND when …`), each at the tokens its end rule ends: a
    /// token that passes one refuses the file, as a lexer checks the token it reads.
    pub checks: Vec<TokenCheck>,
    /// The readers (`EndRule::Read`).
    pub readers: Vec<Reader>,
    /// The readers' kinds by their start masks (`Retag`), for the record passes that read the
    /// masks (Metal's, the interpreter's).
    pub retags: Vec<Retag>,
    /// The second bytes of each `EndRule::Next`'s pairs.
    pub nexts: Vec<Vec<u8>>,
    /// The gap marks (bits of `marks`) made by token tests alone (`CheckThen`): no mask to read.
    pub tested_only: u32,
}

/// A token test (`Records::checks`): over the tokens whose first byte's end rule is `rule`,
/// output `o`'s bits in the token at least `least` (`count`), or `o` at each of the token's
/// first `least` bytes (not `count`: a run, the token that long; `least` at most 64).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenCheck {
    pub rule: u8,
    pub count: bool,
    pub o: u16,
    pub least: u32,
    pub then: CheckThen,
}

/// What a token that passes a test (`TokenCheck`) does: refuses the file (`escape … at KIND`),
/// or is a bit of gap mark `k` at its first byte (`At`) or at the byte after its last (`After`):
/// the mark of the grammar token at or after that byte (`mark … , at KIND when …`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CheckThen {
    Escape,
    At(u8),
    After(u8),
}

/// A token's hits of the marks' tests (`CheckThen::At`, `After`): the `At` marks' bits, and the
/// `After` marks' at `AFTER_SHIFT`.
pub const AFTER_SHIFT: u32 = 16;

/// The marks' tests' hits at token `s..=l` of rule `rule` (`AFTER_SHIFT`).
pub fn check_hits(records: &Records, rule: u8, s: usize, l: usize, outputs: &[Vec<u64>]) -> u32 {
    records.checks.iter().filter(|c| c.rule == rule && check_holds(c, s, l, outputs)).fold(0, |a, c| match c.then {
        CheckThen::Escape => a,
        CheckThen::At(k) => a | 1 << k,
        CheckThen::After(k) => a | 1 << (AFTER_SHIFT + u32::from(k)),
    })
}

/// Whether a token `s..=l` passes a test (`TokenCheck`), over the file's output words.
pub fn check_holds(c: &TokenCheck, s: usize, l: usize, outputs: &[Vec<u64>]) -> bool {
    let w = outputs.get(usize::from(c.o)).map_or(&[][..], |v| v.as_slice());
    let bit = |i: usize| w.get(i / 64).is_some_and(|x| x >> (i % 64) & 1 == 1);
    let n = c.least as usize;
    if c.count {
        (s..=l).filter(|&i| bit(i)).count() >= n
    } else {
        l + 1 - s >= n && (s..s + n).all(bit)
    }
}

/// A byte's end rule (`Records::ends`) when no token begins with it: its token is the byte
/// alone, which the grammar's checks refuse.
pub const NO_RULE: u8 = u8::MAX;

/// How a token's last byte is found from its first (`Records::rules`), each by an output's
/// words (an index into the program's outputs):
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndRule {
    /// The first byte, then the output's run after it: the last is the run's.
    Run(u16),
    /// The first byte, and the next where the output holds at the first.
    Pair(u16),
    /// The output's first bit at or after the first byte (its last-byte mask).
    Last(u16),
    /// A region's token, by a reader (`Records::readers`, an index): `read K | …`.
    Read(u16),
    /// The first byte, and the next where it is one of the bytes of `Records::nexts` (an
    /// index): `pair "AB" …`, as a lexer reads its next byte.
    Next(u16),
}

/// A reader (`EndRule::Read`, decision 0177): a lexer's reader of a string or a comment. The
/// region whose opener's bit holds at the token's first byte, in order, then the first bit of
/// its closer after it; none holds, the byte's other rule (`other`, an index into `rules`, or
/// `NO_RULE`: the file is refused).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Reader {
    pub arms: Vec<ReadArm>,
    pub other: u8,
    /// Where its kind's tokens share a first byte with another kind's (a comment and a `/`):
    /// the code's kind for its token, which no first byte gives, and the marks it has (the
    /// extra's), as the lean lexer's reader says what it read (`Records::retags`).
    pub retag: Option<(u8, u32)>,
}

/// A reader's kind and marks as a mask says them (`Reader::retag`), where no reader runs: at a
/// token whose first byte has output `o`'s bit, the kind `kind` and the marks `marks`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Retag {
    pub o: u16,
    pub kind: u8,
    pub marks: u32,
}

/// A region a reader reads (`Reader`): its opener's and closer's outputs; the last byte at
/// `off` from the closer's first (its bytes less one; `-1`, a closer outside it); and with no
/// closer after the opener, what the token is (`ReadNone`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadArm {
    pub opener: u16,
    pub closer: u16,
    pub off: i8,
    pub none: ReadNone,
}

/// A reader's region with no closer after its opener (`ReadArm::none`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReadNone {
    /// Open at the file's end: the file is refused (the grammar's error says so).
    Refuse,
    /// To the file's end (a line comment): the file's last byte.
    End,
    /// `else E bytes M`: the opener's first `M` bytes.
    Bytes(u8),
}

/// A reader's token's last byte from its first, `s` (`Reader`), a word of the closer's bits at
/// a time; `Err` where the file is refused. `n`: the file's length.
pub fn read_last(records: &Records, reader: &Reader, s: usize, bytes: &[u8], outputs: &[Vec<u64>]) -> Result<usize, ()> {
    let n = bytes.len();
    let words = |o: u16| outputs.get(usize::from(o)).map_or(&[][..], |v| v.as_slice());
    let bit = |o: u16, i: usize| words(o).get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1);
    for a in &reader.arms {
        if !bit(a.opener, s) {
            continue;
        }
        let w = words(a.closer);
        let mut q = (s + 1) / 64;
        let mut x = w.get(q).copied().unwrap_or(0) & (!0u64 << ((s + 1) % 64));
        loop {
            if x != 0 {
                let c = 64 * q + x.trailing_zeros() as usize;
                return Ok(c.wrapping_add_signed(isize::from(a.off)));
            }
            q += 1;
            match w.get(q) {
                Some(&y) => x = y,
                None => break,
            }
        }
        return match a.none {
            ReadNone::Refuse => Err(()),
            ReadNone::End => Ok(n.saturating_sub(1)),
            ReadNone::Bytes(m) => Ok(s + usize::from(m) - 1),
        };
    }
    match records.rules.get(usize::from(reader.other)) {
        Some(&EndRule::Read(_)) | None => Err(()),
        Some(&rule) => Ok(last_by_rule(records, rule, s, bytes, outputs).unwrap_or(s)),
    }
}

/// A token's last byte from its first, `s`, by its rule, over the file's output words; `None`
/// where a `Last` has no bit left. A run past the words ends at their end.
pub fn last_by_rule(records: &Records, rule: EndRule, s: usize, bytes: &[u8], outputs: &[Vec<u64>]) -> Option<usize> {
    let words = |o: u16| outputs.get(usize::from(o)).map_or(&[][..], |v| v.as_slice());
    let bit = |o: u16, i: usize| words(o).get(i / 64).is_some_and(|w| w >> (i % 64) & 1 == 1);
    match rule {
        EndRule::Pair(o) => Some(s + usize::from(bit(o, s))),
        EndRule::Run(o) => {
            // The run from `s + 1`, a word at a time.
            let w = words(o);
            let mut i = s + 1;
            loop {
                let Some(&x) = w.get(i / 64) else { return Some(i - 1) };
                let ones = (!(x >> (i % 64))).trailing_zeros() as usize;
                let left = 64 - i % 64;
                if ones < left {
                    return Some(i + ones - 1);
                }
                i += left;
            }
        }
        // A reader's (`read_last`): not by a rule's words alone.
        EndRule::Read(_) => None,
        EndRule::Next(i) => {
            let seconds = records.nexts.get(usize::from(i)).map_or(&[][..], |v| v.as_slice());
            Some(s + usize::from(bytes.get(s + 1).is_some_and(|b| seconds.contains(b))))
        }
        EndRule::Last(o) => {
            let w = words(o);
            let mut q = s / 64;
            let mut x = w.get(q)? & (!0u64 << (s % 64));
            while x == 0 {
                q += 1;
                x = *w.get(q)?;
            }
            Some(64 * q + x.trailing_zeros() as usize)
        }
    }
}

/// Every token's last byte by the rules (`last_by_rule`), from its first; `None` where a
/// token has none (the file goes to the fallback).
pub fn lasts(records: &Records, bytes: &[u8], starts: &[u32], outputs: &[Vec<u64>]) -> Option<Vec<u32>> {
    starts
        .iter()
        .map(|&s| {
            let first = bytes.get(s as usize).copied().unwrap_or(0);
            match records.rules.get(usize::from(records.ends[usize::from(first)])) {
                Some(&rule) => last_by_rule(records, rule, s as usize, bytes, outputs).map(|l| l as u32),
                None => Some(s),
            }
        })
        .collect()
}

/// The tokens by the loop (decision 0177, stage 2a), as the lean lexer's: each starts at the
/// first byte from the end of the one before that `skip` (an output) does not hold, a word of
/// it at a time, and ends by its first byte's rule (`last_by_rule`; no rule, the byte alone).
/// A region's token ends at its region's last byte, so the loop never starts one inside it.
/// The starts, the lasts, and whether a token starts at a byte with no rule (`NO_RULE`), which
/// the errors by tokens (`Records::untaken`) refuse.
///
/// And whether a reader refuses the file (`read_last`), and whether a token passes a token test
/// (`Records::checks`) of its rule.
pub fn walk(records: &Records, skip: u16, bytes: &[u8], outputs: &[Vec<u64>]) -> (Vec<u32>, Vec<u32>, bool, bool, bool) {
    let n = bytes.len();
    let blank = outputs.get(usize::from(skip)).map_or(&[][..], |v| v.as_slice());
    let held = |q: usize| !blank.get(q).copied().unwrap_or(0);
    let (mut starts, mut lasts) = (Vec::new(), Vec::new());
    let mut at = 0usize;
    let mut untaken = false;
    let mut checked = false;
    let mut read = false;
    'tokens: while at < n {
        let mut q = at / 64;
        let mut x = held(q) & (!0u64 << (at % 64));
        while x == 0 {
            q += 1;
            if 64 * q >= n {
                break 'tokens;
            }
            x = held(q);
        }
        let s = 64 * q + x.trailing_zeros() as usize;
        if s >= n {
            break;
        }
        let k = records.ends[usize::from(bytes[s])];
        let l = match records.rules.get(usize::from(k)) {
            Some(&EndRule::Read(i)) => {
                let l = match read_last(records, &records.readers[usize::from(i)], s, bytes, outputs) {
                    Ok(l) => l.max(s),
                    Err(()) => {
                        read = true;
                        s
                    }
                };
                checked |= records.checks.iter().any(|c| c.rule == k && c.then == CheckThen::Escape && check_holds(c, s, l, outputs));
                l
            }
            Some(&rule) => {
                let l = last_by_rule(records, rule, s, bytes, outputs).unwrap_or(s).max(s);
                checked |= records.checks.iter().any(|c| c.rule == k && c.then == CheckThen::Escape && check_holds(c, s, l, outputs));
                l
            }
            None => {
                untaken = true;
                s
            }
        };
        starts.push(s as u32);
        lasts.push(l as u32);
        at = l + 1;
    }
    (starts, lasts, untaken, read, checked)
}

/// The most marks a code holds.
pub const MARKS: usize = 16;

/// A mark's output in `Records::marks` when it has none: its bit is its word's alone (`word ==
/// W`), no mask read. A token mark's (decision 0129): its mask is every grammar token's start,
/// and no other token (an extra's) can have the word.
pub const WORD_ALONE: u16 = u16::MAX;

/// Whether a record's mark `(o, w)` holds: its mask's bit at the start (`bit`, read only when it
/// has one) and its word.
pub fn mark_holds(o: u16, w: u16, word: u32, bit: impl FnOnce() -> bool) -> bool {
    (w == 0 || u32::from(w) == word) && (o == WORD_ALONE || bit())
}

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
        let mut kind = u32::from(records.kinds[usize::from(bytes.get(s).copied().unwrap_or(0))]);
        let word = table.word(bytes, s, len);
        let mut marks = 0u32;
        let bit_of = |o: u16| outputs.get(usize::from(o)).and_then(|v| v.get(s / 64)).is_some_and(|x| x >> (s % 64) & 1 == 1);
        for t in records.retags.iter().filter(|t| bit_of(t.o)) {
            kind = u32::from(t.kind);
            marks |= t.marks;
        }
        let retagged: u32 = records.retags.iter().fold(0, |a, t| a | t.marks);
        for (k, &(o, w)) in records.marks.iter().enumerate().filter(|&(k, _)| retagged >> k & 1 == 0) {
            let bit = || outputs.get(usize::from(o)).and_then(|v| v.get(s / 64)).is_some_and(|x| x >> (s % 64) & 1 == 1);
            if mark_holds(o, w, word, bit) {
                marks |= 1 << k;
            }
        }
        codes.push(kind | word << KIND_BITS | marks << MARK_SHIFT);
    }
    let unplaced = gap_marks(records, bytes, starts, lasts, outputs, &mut codes);
    (codes, bad || unplaced)
}

/// Whether output `o` holds anywhere in bytes `a..=b` (`a > b`: none), a word at a time.
pub fn any_in(outputs: &[Vec<u64>], o: u16, a: usize, b: usize) -> bool {
    let w = outputs.get(usize::from(o)).map_or(&[][..], |v| v.as_slice());
    if a > b {
        return false;
    }
    (a / 64..=b / 64).any(|q| {
        let mut x = w.get(q).copied().unwrap_or(0);
        if q == a / 64 {
            x &= !0u64 << (a % 64);
        }
        if q == b / 64 && b % 64 != 63 {
            x &= (1u64 << (b % 64 + 1)) - 1;
        }
        x != 0
    })
}

/// The gap marks (`Records::gaps`) into the codes: each grammar token's, from its mark's output
/// after the grammar token before it up to its first byte. An extra's token has none. Whether
/// a bit of an `else escape` mark has no grammar token to hold it (`Records::unplaced`).
///
/// The marks' token tests (`CheckThen`) are bits too: a token's `At` hits at its first byte, its
/// `After` hits at the byte after its last. So a grammar token has its own `At` hits, and the
/// grammar token before's and the extras' between `After` hits and the extras' `At` hits
/// (`pending`), as a lexer carries what it read to the next token.
pub fn gap_marks(records: &Records, bytes: &[u8], starts: &[u32], lasts: &[u32], outputs: &[Vec<u64>], codes: &mut [u32]) -> bool {
    if records.gaps == 0 {
        return false;
    }
    let extra = |c: u32| records.extra.is_some_and(|e| c >> (MARK_SHIFT + u32::from(e)) & 1 == 1);
    let hits = |k: usize| -> u32 {
        let s = starts[k] as usize;
        let l = lasts.get(k).map_or(s, |&l| l as usize);
        match bytes.get(s) {
            Some(&b) if !records.checks.is_empty() => check_hits(records, records.ends[usize::from(b)], s, l, outputs),
            _ => 0,
        }
    };
    let low = (1u32 << AFTER_SHIFT) - 1;
    let mut from = 0usize;
    // The word of the grammar token before (0: none, or the file's first).
    let mut word_before = 0u32;
    let mut pending = 0u32;
    for (k, &s) in starts.iter().enumerate().take(codes.len()) {
        let s = s as usize;
        let h = hits(k);
        if extra(codes[k]) {
            codes[k] &= !(records.gaps << MARK_SHIFT);
            pending |= h & low | h >> AFTER_SHIFT;
            continue;
        }
        let mut gap = 0u32;
        for (m, &(o, _)) in records.marks.iter().enumerate().filter(|&(m, _)| records.gaps >> m & 1 == 1) {
            let none = records.unless.iter().any(|&(u, w)| usize::from(u) == m && u32::from(w) == word_before);
            let tested = (pending | h) >> m & 1 == 1;
            if !none && (tested || records.tested_only >> m & 1 == 0 && any_in(outputs, o, from, s)) {
                gap |= 1 << m;
            }
        }
        codes[k] = codes[k] & !(records.gaps << MARK_SHIFT) | gap << MARK_SHIFT;
        from = s + 1;
        word_before = codes[k] >> KIND_BITS & ((1 << (MARK_SHIFT - KIND_BITS)) - 1);
        pending = h >> AFTER_SHIFT;
    }
    // The bits past the last grammar token's start.
    records.marks.iter().enumerate().filter(|&(m, _)| records.unplaced >> m & 1 == 1).any(|(m, &(o, _))| {
        let words = outputs.get(usize::from(o)).map_or(0, Vec::len);
        pending >> m & 1 == 1 || records.tested_only >> m & 1 == 0 && words > 0 && any_in(outputs, o, from, 64 * words - 1)
    })
}
