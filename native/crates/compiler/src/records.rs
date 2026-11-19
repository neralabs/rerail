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

/// Where tier 1 puts each token's record, its first byte, last byte and code, for tier 2 to
/// read (decision 0231). The compiler chooses it for each program, by who writes the records:
/// no backend assumes one.
///
/// - **columns:** each field a column of the file's positions, as an extract writes its
///   positions, a token's field 4 bytes after the one before's. A GPU's threads read them
///   coalesced.
/// - **rows:** a token's three fields together, 12 bytes, as the lean lexer's `Tok`, where the
///   CPU's token loop writes them all: one base and one index for the three.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RecordLayout {
    #[default]
    Columns,
    Rows,
}

/// A record's fields, in a row's order (`RecordLayout`).
pub const FIELD_START: usize = 0;
pub const FIELD_LAST: usize = 1;
pub const FIELD_CODE: usize = 2;

/// Who writes a program's records, which the layout's choice weighs (`RecordLayout::choose`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordWriter {
    /// One thread's token loop writes each token's fields together (the CPU's).
    TokenLoop,
    /// A group of threads writes a token each, side by side (a GPU's block passes): their
    /// stores coalesce where each field is a column.
    Lanes,
}

impl RecordLayout {
    /// The layout for records `writer` writes; `whole` where it writes all three fields of
    /// each token and nothing else shares their room (`native_cpu_block::record_layout`).
    pub const fn choose(writer: RecordWriter, whole: bool) -> RecordLayout {
        match writer {
            RecordWriter::TokenLoop if whole => RecordLayout::Rows,
            _ => RecordLayout::Columns,
        }
    }

    /// The bytes from a token's field to the next token's.
    pub const fn stride(self) -> u64 {
        match self {
            RecordLayout::Columns => 4,
            RecordLayout::Rows => 12,
        }
    }

    /// Where token `k`'s field `field` is in a file's positions, in `u32`s from the file's
    /// first (`Row::positions`): the columns at `columns` (start, last, code) of `capacity`
    /// positions each; the rows from the first of them.
    pub fn index(self, field: usize, k: u64, columns: [u64; 3], capacity: u64) -> u64 {
        match self {
            RecordLayout::Columns => columns[field] * capacity + k,
            RecordLayout::Rows => columns.iter().min().copied().unwrap_or(0) * capacity + 3 * k + field as u64,
        }
    }
}

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
    /// The pairs of each `EndRule::Next` (`PairTable`).
    pub nexts: Vec<PairTable>,
    /// The gap marks (bits of `marks`) made by token tests alone (`CheckThen`): no mask to read.
    pub tested_only: u32,
    /// The masks tier 2's local checks read (`program::LocalCheck::mask`, indexes into the block
    /// program's masks), and nothing else does: on the CPU, tier 2 reads each from the bytes
    /// where they make it (`native_cpu_bytes`), so the CPU's body makes none of them.
    pub check_masks: Vec<u16>,
    /// The prefix (`prefix B …`, decision 0335): where the file starts with these bytes, the
    /// stream, and the loop, start after them. Empty: no prefix.
    pub prefix: Vec<u8>,
    /// The skip's mask (decision 0411), an output: where set, a token's last byte is the byte
    /// before the next token's start (the file's end) less the run of this mask's bytes before
    /// it (`gap_last`), as the lasts extract's, proved. The CPU's tier 1 then makes no lasts; its
    /// record pass makes them.
    pub gap: Option<u16>,
}

/// A token's last byte (decision 0411, `Records::gap`): the byte before `next` (the next
/// token's start, or the file's length), less the run of `gap`'s bits before it, found a word
/// at a time (the leading zeros of the word's bits not in the run).
pub fn gap_last(gap: &[u64], start: usize, next: usize) -> usize {
    let mut end = next;
    while end > start + 1 {
        let (q, r) = ((end - 1) / 64, (end - 1) % 64);
        // The bits at and below `end - 1` not in the gap: the highest is the last byte.
        let w = !gap.get(q).copied().unwrap_or(0) & (u64::MAX >> (63 - r));
        if w != 0 {
            return (q * 64 + 63 - w.leading_zeros() as usize).max(start);
        }
        end = q * 64;
    }
    start
}

/// The tokens' last bytes by their gaps (`gap_last`), each before the next token's start.
pub fn gap_lasts(gap: &[u64], starts: &[u32], len: usize) -> Vec<u32> {
    (0..starts.len())
        .map(|k| {
            gap_last(
                gap,
                starts[k] as usize,
                starts.get(k + 1).map_or(len, |&n| n as usize),
            ) as u32
        })
        .collect()
}

/// A token test (`Records::checks`): over the tokens whose first byte's end rule is `rule`,
/// output `o`'s bits in the token at least `least` (`count`), or `o` at each of the token's
/// first `least` bytes (not `count`: a run, the token that long; `least` at most 64), or at
/// every byte of a token `least` long at least (`all`, not `count`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TokenCheck {
    pub rule: u8,
    pub count: bool,
    pub o: u16,
    pub least: u32,
    pub then: CheckThen,
    pub all: bool,
}

impl TokenCheck {
    /// A count whose hit a loop makes with the rule's others (fused into one call or one
    /// vector): not a cut, which ends the token before they are made.
    pub fn counted(&self) -> bool {
        self.count && !self.always() && self.then != CheckThen::Cut
    }

    /// A count of at least none: it holds at every token its rule reads (the lowering proves
    /// a count of one so, `bitlex_masks`), so nothing reads its mask. A reader's rule reads a
    /// token where an arm's opener holds at its first byte; at another, its other rule's
    /// (`Reader::other`), the test does not hold (`test_holds`).
    pub fn always(&self) -> bool {
        self.count && self.least == 0
    }
}

/// Whether test `c` holds at token `s..=l` of its rule (`check_holds`), a test that always
/// holds only at a token the rule reads (`TokenCheck::always`).
pub fn test_holds(records: &Records, c: &TokenCheck, s: usize, l: usize, outputs: &[Vec<u64>]) -> bool {
    if !c.always() {
        return check_holds(c, s, l, outputs);
    }
    match records.rules.get(usize::from(c.rule)) {
        Some(&EndRule::Read(i)) => records.readers[usize::from(i)].arms.iter().any(|a| {
            outputs.get(usize::from(a.opener)).and_then(|w| w.get(s / 64)).is_some_and(|w| w >> (s % 64) & 1 == 1)
        }),
        _ => true,
    }
}

/// What a token that passes a test (`TokenCheck`) does: refuses the file (`escape … at KIND`),
/// or is a bit of gap mark `k` at its first byte (`At`) or at the byte after its last (`After`):
/// the mark of the grammar token at or after that byte (`mark … , at KIND when …`). Or, a
/// count's (`Cut`, `run R once D`), the token ends before its mask's `least`th bit (`cut`),
/// before any other test of it is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CheckThen {
    Escape,
    At(u8),
    After(u8),
    Cut,
}

impl CheckThen {
    /// A mark's bit (`At`, `After`).
    pub fn marks(self) -> bool {
        matches!(self, CheckThen::At(_) | CheckThen::After(_))
    }
}

/// Token `s..=l` of rule `rule` cut by its rule's `Cut` test (`run R once D`): it ends before
/// the test's mask's `least`th bit in it, as a lexer stops at a number's second `.`.
pub fn cut(records: &Records, rule: u8, s: usize, l: usize, outputs: &[Vec<u64>]) -> usize {
    records.checks.iter().filter(|c| c.rule == rule && c.then == CheckThen::Cut).fold(l, |l, c| {
        let w = outputs.get(usize::from(c.o)).map_or(&[][..], |v| v.as_slice());
        let bit = |i: usize| w.get(i / 64).is_some_and(|x| x >> (i % 64) & 1 == 1);
        (s..=l).filter(|&i| bit(i)).nth(c.least as usize - 1).map_or(l, |i| i - 1)
    })
}

/// A token's hits of the marks' tests (`CheckThen::At`, `After`): the `At` marks' bits, and the
/// `After` marks' at `AFTER_SHIFT`.
pub const AFTER_SHIFT: u32 = 16;

/// The marks' tests' hits at token `s..=l` of rule `rule` (`AFTER_SHIFT`).
pub fn check_hits(records: &Records, rule: u8, s: usize, l: usize, outputs: &[Vec<u64>]) -> u32 {
    records.checks.iter().filter(|c| c.rule == rule && test_holds(records, c, s, l, outputs)).fold(0, |a, c| match c.then {
        CheckThen::Escape | CheckThen::Cut => a,
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
    } else if c.all {
        l + 1 - s >= n && (s..=l).all(bit)
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
    /// The first byte, and the next where its pair is one of `Records::nexts`' table (an
    /// index): `pair P`, as a lexer reads its next byte.
    Next(u16),
}

/// A kind's two-byte spellings (`EndRule::Next`): by a token's first byte its row (`row`, 256
/// of them; row 0 none), and by the next byte, its bit in the row's set (`sets`, 256 bits): a
/// two-byte token where it holds, one load each, as the lean lexer's table of tokens.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PairTable {
    pub row: Vec<u8>,
    pub sets: Vec<[u64; 4]>,
}

impl PairTable {
    /// Whether `a` then `b` is one of the pairs.
    pub fn holds(&self, a: u8, b: u8) -> bool {
        let row = usize::from(self.row.get(usize::from(a)).copied().unwrap_or(0));
        self.sets.get(row).is_some_and(|set| set[usize::from(b) / 64] >> (b % 64) & 1 == 1)
    }
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
    /// Its region read in parts (`read K parts { … }`), one arm: each part's start (an output)
    /// and bytes. Its tokens are the opener's byte, each part, each run of the bytes between,
    /// and the closer (`read_parts`). None: the region is one token.
    pub parts: Vec<ReadPart>,
    /// Where its parts are split (decision 0323): by tier 1's token loop (`Split::Tier1`, its
    /// records the parts'), or by tier 2's token fetch (`Split::Tier2`, tier 1's record the
    /// whole region, as a reader without parts gives it).
    pub split: Split,
    /// Its region read in parts holds no mask mark's bit and no retag's, from the byte after its
    /// opener through its closer (proved, `bitlex_masks`): its tokens' codes are their kind and
    /// word alone (`native_cpu_block::plain_code`), whatever marks the grammar has.
    pub quiet: bool,
}

/// Where a region read in parts is split into its tokens (`Reader::split`, decision 0323).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Split {
    /// Tier 1's records are the parts (lowering A).
    #[default]
    Tier1,
    /// Tier 1's record is the region; tier 2's token fetch splits it (lowering B).
    Tier2,
}

impl Records {
    /// The marks the code holds, by order: the first `mark_bits` marks (decision 0407). The
    /// extras' bits come after them; a mark of a later order (a token mark that tier 2 tests
    /// alone, a mark past `MARKS`) holds no bit of the code.
    pub fn mark_bits(&self) -> usize {
        self.extra.map_or(self.marks.len(), usize::from)
    }

    /// Where the stream starts in `bytes`: past the prefix, where the file starts with it.
    pub fn start(&self, bytes: &[u8]) -> usize {
        if !self.prefix.is_empty() && bytes.starts_with(&self.prefix) { self.prefix.len() } else { 0 }
    }

    /// Every region read in parts split where `at` says (decision 0323): a compile option, the
    /// grammar the same either way.
    pub fn split_parts_in(&mut self, at: Split) {
        for rd in self.readers.iter_mut().filter(|rd| !rd.parts.is_empty()) {
            rd.split = at;
        }
    }
}

impl Reader {
    /// Whether tier 1 splits its region into parts (`Split::Tier1`).
    pub fn parts_in_tier1(&self) -> bool {
        !self.parts.is_empty() && self.split == Split::Tier1
    }
}

impl Reader {
    /// Every kind and marks it gives a token (`retag`, and its arms' own, `ReadArm::retag`).
    pub fn tags(&self) -> impl Iterator<Item = (u8, u32)> + '_ {
        self.retag.into_iter().chain(self.arms.iter().filter_map(|a| a.retag))
    }

    /// The kind and marks it gives every token it reads, where they are one: its `retag`, with
    /// no arm of a kind of its own.
    pub fn tag(&self) -> Option<(u8, u32)> {
        self.retag.filter(|&t| self.arms.iter().all(|a| a.retag.is_none_or(|x| x == t)))
    }
}

/// A part of a region read in parts (`Reader::parts`): at a bit of output `start`, `bytes`
/// bytes, as a lexer reads an escape in a string; or, with `last`, through the first bit of that
/// output at or after its start (`bytes` 0), as a lexer reads the name after Kotlin's `$`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadPart {
    pub start: u16,
    pub bytes: u8,
    pub last: Option<u16>,
}

impl ReadPart {
    /// Its last byte, where it starts at `e`; `None` where its `last` never comes.
    pub fn last_at(&self, e: usize, outputs: &[Vec<u64>]) -> Option<usize> {
        match self.last {
            Some(o) => first_bit(outputs, o, e),
            None => Some(e + usize::from(self.bytes) - 1),
        }
    }
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
    /// A region read in parts whose closers differ in length (Kotlin's `"` and a template's
    /// `${`): the output whose first bit at or after the closer is its last; `None`, `off`.
    pub last: Option<u16>,
    pub none: ReadNone,
    /// Where the reader reads the regions of several kinds (Kotlin's line and block comments,
    /// both at `/`): this region's kind and marks, as `Reader::retag` says them for the reader's
    /// own (`None`: the reader's).
    pub retag: Option<(u8, u32)>,
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

/// The first bit of output `o` at or after byte `from`, a word at a time; `None`, none.
fn first_bit(outputs: &[Vec<u64>], o: u16, from: usize) -> Option<usize> {
    let w = outputs.get(usize::from(o)).map_or(&[][..], |v| v.as_slice());
    let mut q = from / 64;
    let mut x = w.get(q)? & (!0u64 << (from % 64));
    while x == 0 {
        q += 1;
        x = *w.get(q)?;
    }
    Some(64 * q + x.trailing_zeros() as usize)
}

/// A region read in parts from its opener at `s` (`Reader::parts`): its tokens' starts and
/// lasts, in order, as a lexer reads a string: the opener's byte; then from the byte after,
/// the nearer of the closer's next bit and each part's next bit, the bytes before it a token
/// of their own (the content's), a part its bytes, and on after it, until the closer, its
/// bytes the last token. `Err` where the closer never comes (the file is refused).
pub fn read_parts(reader: &Reader, s: usize, outputs: &[Vec<u64>]) -> Result<Vec<(usize, usize)>, ()> {
    let a = reader.arms.first().ok_or(())?;
    let mut out = vec![(s, s)];
    let mut at = s + 1;
    loop {
        let c = first_bit(outputs, a.closer, at).ok_or(())?;
        let part = reader.parts.iter().filter_map(|p| first_bit(outputs, p.start, at).filter(|&e| e < c).map(|e| (e, p.bytes, p))).min_by_key(|&(e, n, p)| (e, n, p.start));
        let to = part.map_or(c, |(e, ..)| e);
        if to > at {
            out.push((at, to - 1));
        }
        match part {
            Some((e, _, p)) => {
                let l = p.last_at(e, outputs).ok_or(())?;
                out.push((e, l));
                at = l + 1;
            }
            None => {
                let l = match a.last {
                    Some(o) => first_bit(outputs, o, c).ok_or(())?,
                    None => c.wrapping_add_signed(isize::from(a.off)),
                };
                out.push((c, l));
                return Ok(out);
            }
        }
    }
}

/// A reader's token's last byte from its first, `s` (`Reader`), a word of the closer's bits at
/// a time; `Err` where the file is refused. `n`: the file's length.
#[allow(clippy::result_unit_err, reason = "a refusal says nothing more: the caller leaves the file to the grammar's parser")]
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
            let t = records.nexts.get(usize::from(i))?;
            Some(s + usize::from(bytes.get(s + 1).is_some_and(|&b| t.holds(bytes[s], b))))
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
            let k = records.ends[usize::from(first)];
            match records.rules.get(usize::from(k)) {
                Some(&rule) => last_by_rule(records, rule, s as usize, bytes, outputs).map(|l| cut(records, k, s as usize, l, outputs) as u32),
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
    let mut at = records.start(bytes);
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
        // A region read in parts: each of its tokens.
        if let Some(&EndRule::Read(i)) = records.rules.get(usize::from(k)) {
            let reader = &records.readers[usize::from(i)];
            let opens = reader.arms.first().is_some_and(|a| outputs.get(usize::from(a.opener)).and_then(|w| w.get(s / 64)).is_some_and(|w| w >> (s % 64) & 1 == 1));
            if reader.parts_in_tier1() && opens {
                match read_parts(reader, s, outputs) {
                    Ok(tokens) => {
                        let l = tokens.last().map_or(s, |t| t.1);
                        for (a, b) in tokens {
                            starts.push(a as u32);
                            lasts.push(b as u32);
                        }
                        at = l + 1;
                    }
                    Err(()) => {
                        read = true;
                        starts.push(s as u32);
                        lasts.push(s as u32);
                        at = s + 1;
                    }
                }
                continue;
            }
        }
        let l = match records.rules.get(usize::from(k)) {
            Some(&EndRule::Read(i)) => {
                let l = match read_last(records, &records.readers[usize::from(i)], s, bytes, outputs) {
                    Ok(l) => l.max(s),
                    Err(()) => {
                        read = true;
                        s
                    }
                };
                checked |= records.checks.iter().any(|c| c.rule == k && c.then == CheckThen::Escape && test_holds(records, c, s, l, outputs));
                l
            }
            Some(&rule) => {
                let l = cut(records, k, s, last_by_rule(records, rule, s, bytes, outputs).unwrap_or(s).max(s), outputs);
                checked |= records.checks.iter().any(|c| c.rule == k && c.then == CheckThen::Escape && test_holds(records, c, s, l, outputs));
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
        let mut word = table.word(bytes, s, len);
        let mut marks = 0u32;
        let bit_of = |o: u16| outputs.get(usize::from(o)).and_then(|v| v.get(s / 64)).is_some_and(|x| x >> (s % 64) & 1 == 1);
        // A token a reader reads and says (a comment) is no word, as a lexer looks no
        // comment up (`Reader::retag`).
        for t in records.retags.iter().filter(|t| bit_of(t.o)) {
            kind = u32::from(t.kind);
            marks |= t.marks;
            word = 0;
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
