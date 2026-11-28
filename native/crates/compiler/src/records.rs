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

use rerail_blocks::vector::Vector;
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
    /// before up to its first byte (an extra's bits are only those it keeps of its own gap,
    /// from the record before: `own`). The other marks are their outputs' bits at the token's
    /// first byte.
    pub gaps: u32,
    /// The mark whose bit tells an extra's token (a comment), if any. An extra has no gap mark
    /// but those it keeps of the gap before it (`own`), as the lean lexer's `skip` knows a
    /// line end before each comment it passes (tier 2 reads them where it places comments).
    pub extra: Option<u8>,
    /// The first of the bits that tell a token mark's kind by its rows' starts
    /// (`TokenElem::rows`), after the marks' and the extras': no mark's bit.
    pub kind_rows: Option<u8>,
    /// The outputs of the errors the token loop checks (`error … by tokens`): a byte it starts a
    /// token at that no token begins with refuses the file. The CPU makes none of them.
    pub untaken: Vector<u16>,
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
    /// The token marks among the code's (bits of `marks`) that hold no bit (decision 0407): ones
    /// that refuse nowhere alone, in the code only because a mark after them is. Tier 2 tests
    /// their tokens alone, as those past the code's; their masks are zero.
    pub alone: u32,
    /// The marks (bits of `marks`) a token pattern's `without` reads (0430): their bits are in
    /// the code on every target, as the patterns read them there (no gap mark tier 2 makes
    /// itself, `native_cpu_block::lazy_marks`).
    pub read: u32,
    /// The gap marks the CPU's token loop reads from the blanks (decision 0430): each `(mark,
    /// class)`, the class an output, a byte class. Proved: the mark's mask holds at no byte of a
    /// token and at every byte of the class between tokens, so its bit at a grammar token is
    /// whether the class holds in the blank runs before it and before the extras between, as
    /// the lean lexer's `skip` sets `nl`. Metal and the interpreter read the mask, as other gap
    /// marks'.
    pub blanks: Vec<(u8, u16)>,
    /// The gap marks an extra's code keeps of its own gap, from the byte after the record
    /// before's first to its start (`extra`): the blank marks and the settling marks (`mark …
    /// , settles`), as the lean lexer's `skip` knows a line end before each comment it passes.
    /// Tier 2's gap patterns and `SETTLE` read them.
    pub own: u32,
    /// The masks tier 2's local checks read (`program::LocalCheck::mask`, indexes into the block
    /// program's masks), and nothing else does: on the CPU, tier 2 reads each from the bytes
    /// where they make it (`native_cpu_bytes`), so the CPU's body makes none of them.
    pub check_masks: Vector<u16>,
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
pub fn gap_lasts(gap: &[u64], starts: &[u32], len: usize) -> Vector<u32> {
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
pub fn test_holds(records: &Records, c: &TokenCheck, s: usize, l: usize, outputs: &[Vector<u64>]) -> bool {
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
pub fn cut(records: &Records, rule: u8, s: usize, l: usize, outputs: &[Vector<u64>]) -> usize {
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
pub fn check_hits(records: &Records, rule: u8, s: usize, l: usize, outputs: &[Vector<u64>]) -> u32 {
    // A region open at the file's end (`ReadNone::Bad`) is no token of its kind: none of its
    // kind's tests, as the readers read none.
    if let Some(&EndRule::Read(i)) = records.rules.get(usize::from(rule))
        && records.readers[usize::from(i)].open_kind(s, l, outputs).is_some()
    {
        return 0;
    }
    records.checks.iter().filter(|c| c.rule == rule && test_holds(records, c, s, l, outputs)).fold(0, |a, c| match c.then {
        CheckThen::Escape | CheckThen::Cut => a,
        CheckThen::At(k) => a | 1 << k,
        CheckThen::After(k) => a | 1 << (AFTER_SHIFT + u32::from(k)),
    })
}

/// Whether a token `s..=l` passes a test (`TokenCheck`), over the file's output words.
pub fn check_holds(c: &TokenCheck, s: usize, l: usize, outputs: &[Vector<u64>]) -> bool {
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
/// two-byte token where it holds, one load each, as the lean lexer's table of tokens. Its
/// three-byte spellings (`thirds`, a few): a pair and the byte after, as the lean lexer reads
/// `===` from `==`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PairTable {
    pub row: Vec<u8>,
    pub sets: Vector<[u64; 4]>,
    pub thirds: Vector<[u8; 3]>,
}

impl PairTable {
    /// Whether `a` then `b` is one of the pairs.
    pub fn holds(&self, a: u8, b: u8) -> bool {
        let row = usize::from(self.row.get(usize::from(a)).copied().unwrap_or(0));
        self.sets.get(row).is_some_and(|set| set[usize::from(b) / 64] >> (b % 64) & 1 == 1)
    }

    /// The last byte of a token from `s`: its first, the next where they are a pair, the one
    /// after where the three are a triple.
    pub fn last(&self, bytes: &[u8], s: usize) -> usize {
        match (bytes.get(s), bytes.get(s + 1)) {
            (Some(&a), Some(&b)) if self.holds(a, b) => s + 1 + usize::from(bytes.get(s + 2).is_some_and(|&c| self.thirds.contains(&[a, b, c]))),
            _ => s,
        }
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
    /// The kind of its region's bytes between its parts (`else K`), where it is one of its own (a
    /// string's text beside its closer `"`, decision 0509); None: each token's first byte's. Its
    /// rows' start masks give it too (`Records::retags`, `part`).
    pub content: Option<u8>,
    /// Its closers' own kinds (decision 0509): a closer that begins with byte `.0` has kind `.1`,
    /// where its byte begins a token outside (a template's `"` and a string's opener). Their rows'
    /// start masks give them too (`Records::retags`, `part`).
    pub closers: Vec<(u8, u8)>,
    /// Where its parts are split (decision 0323): by tier 1's token loop (`Split::Tier1`, its
    /// records the parts'), or by tier 2's token fetch (`Split::Tier2`, tier 1's record the
    /// whole region, as a reader without parts gives it).
    pub split: Split,
    /// Its region read in parts holds no mask mark's bit and no retag's, from the byte after its
    /// opener through its closer (proved, `bitlex_masks`): its tokens' codes are their kind and
    /// word alone (`native_cpu_block::plain_code`), whatever marks the grammar has.
    pub quiet: bool,
    /// Its regions `resume` a token's (decision 0600): its read makes no record of its own. It
    /// ends the record a template left open (`ReadArm::template`), or, closed at a template
    /// again, leaves it open over the next one's code.
    pub resumes: bool,
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
        self.extra.or(self.kind_rows).map_or(self.marks.len(), usize::from)
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

impl Records {
    /// Whether tier 2 makes each token's record from its start (page 40): where tier 1 is the
    /// block of masks (no `skip`, so no token loop) and the tokens end by their rules, tier 1
    /// writes each token's start alone, as simdjson's stage 1 its structural indexes, and tier 2
    /// finds the token's last by its first byte's rule and makes its code, as simdjson's stage 2
    /// switches on the byte at the index. The CPU's tier 1 then has no record pass.
    pub fn fetched_in_tier2(&self) -> bool {
        !self.rules.is_empty() && self.skip.is_none() && self.gap.is_none()
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
        let open = self.arms.iter().filter_map(|a| if let ReadNone::Bad(k) = a.none { Some((k, 0)) } else { None });
        self.retag.into_iter().chain(self.arms.iter().filter_map(|a| a.retag)).chain(open)
    }

    /// The kind and marks it gives every token it reads, where they are one: its `retag`, with
    /// no arm of a kind of its own.
    pub fn tag(&self) -> Option<(u8, u32)> {
        self.retag.filter(|&t| self.arms.iter().all(|a| a.retag.is_none_or(|x| x == t) && !matches!(a.none, ReadNone::Bad(_))))
    }

    /// The kind of its token `s..=l` where the region it read is open at the file's end
    /// (`ReadNone::Bad`): the first arm whose opener holds at `s` (as `read_end` reads it) with
    /// no closer's first byte where the token's last says (`off` before it, after `s`). A word of
    /// output at most: what the reader read, said again.
    pub fn open_kind(&self, s: usize, l: usize, outputs: &[Vector<u64>]) -> Option<u8> {
        let a = self.arms.iter().find(|a| bit_at(outputs, a.opener, s))?;
        let ReadNone::Bad(k) = a.none else { return None };
        let c = l.checked_add_signed(-isize::from(a.off)).filter(|&c| c > s);
        (!c.is_some_and(|c| bit_at(outputs, a.closer, c))).then_some(k)
    }
}

/// Whether retag `t` is a token's own kind that no reader gives and no part is (a `0x` beside a
/// number, a `.5` beside the operators, decision 0509): the token loop reads its mask.
pub fn own_retag(r: &Records, t: &Retag) -> bool {
    !t.part && !r.readers.iter().any(|rd| rd.tags().any(|(k, _)| k == t.kind))
}

/// A part of a region read in parts (`Reader::parts`): at a bit of output `start`, `bytes`
/// bytes, as a lexer reads an escape in a string; or, with `last`, through the first bit of that
/// output at or after its start (`bytes` 0), as a lexer reads the name after Kotlin's `$`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ReadPart {
    pub start: u16,
    pub bytes: u8,
    pub last: Option<u16>,
    /// Its kind of its own (`Retag::part`), the records' kind; None: its first byte's.
    pub kind: Option<u8>,
}

impl ReadPart {
    /// Its last byte, where it starts at `e`; `None` where its `last` never comes.
    pub fn last_at(&self, e: usize, outputs: &[Vector<u64>]) -> Option<usize> {
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
    /// A part's kind in a region read in parts (`ReadPart::kind`), where another of the region's
    /// tokens begins with its byte: its word stays (a `$name`'s), and on the CPU the reader's walk
    /// says it, as the lean lexer's string reader says what it read.
    pub part: bool,
    /// The bytes its row's tokens begin with (its own, before its kind's byte was moved): where a
    /// token loop's arm reads it (decision 0509).
    pub first: [u64; 4],
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
    /// Where a region kind `resumes` this arm's (decision 0600): the output of its second
    /// closer (`or`, a template's `$`). A read that closes there leaves its token's record open
    /// over the template's code, to the end of the reader that `resumes` it.
    pub template: Option<u16>,
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
    /// `open K`: to the file's end, a token of kind `K` (no word, none of the reader's marks), as
    /// the lean lexer makes the rest of the file one `Bad` token where it finds no closer.
    Bad(u8),
}

/// The first bit of output `o` at or after byte `from`, a word at a time; `None`, none.
pub fn first_bit(outputs: &[Vector<u64>], o: u16, from: usize) -> Option<usize> {
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
/// bytes the last token. `None` where the closer never comes (the file is refused).
pub fn read_parts(reader: &Reader, s: usize, outputs: &[Vector<u64>]) -> Option<Vec<(usize, usize)>> {
    let a = reader.arms.first()?;
    let mut out = vec![(s, s)];
    let mut at = s + 1;
    loop {
        let c = first_bit(outputs, a.closer, at)?;
        let part = reader.parts.iter().filter_map(|p| first_bit(outputs, p.start, at).filter(|&e| e < c).map(|e| (e, p.bytes, p))).min_by_key(|&(e, n, p)| (e, n, p.start));
        let to = part.map_or(c, |(e, ..)| e);
        if to > at {
            out.push((at, to - 1));
        }
        match part {
            Some((e, _, p)) => {
                let l = p.last_at(e, outputs)?;
                out.push((e, l));
                at = l + 1;
            }
            None => {
                let l = match a.last {
                    Some(o) => first_bit(outputs, o, c)?,
                    None => c.wrapping_add_signed(isize::from(a.off)),
                };
                out.push((c, l));
                return Some(out);
            }
        }
    }
}

/// A reader's token's last byte from its first, `s` (`Reader`), a word of the closer's bits at
/// a time; `Err` where the file is refused. `n`: the file's length.
#[allow(clippy::result_unit_err, reason = "a refusal says nothing more: the caller leaves the file to the grammar's parser")]
pub fn read_last(records: &Records, reader: &Reader, s: usize, bytes: &[u8], outputs: &[Vector<u64>]) -> Result<usize, ()> {
    read_end(records, reader, s, bytes, outputs).map(|e| e.0)
}

/// A reader's token's last byte from its first, `s` (`read_last`), and whether its closer is a
/// template's (`ReadArm::template`, decision 0600): its record stays open over the code.
#[allow(clippy::result_unit_err, reason = "a refusal says nothing more: the caller leaves the file to the grammar's parser")]
pub fn read_end(records: &Records, reader: &Reader, s: usize, bytes: &[u8], outputs: &[Vector<u64>]) -> Result<(usize, bool), ()> {
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
                let template = a.template.is_some_and(|t| bit(t, c));
                // A closer to its last (`ReadArm::last`, a `${`'s two bytes), as the walk ends it.
                return match a.last {
                    Some(o) => first_bit(outputs, o, c).ok_or(()).map(|l| (l, template)),
                    None => Ok((c.wrapping_add_signed(isize::from(a.off)), template)),
                };
            }
            q += 1;
            match w.get(q) {
                Some(&y) => x = y,
                None => break,
            }
        }
        return match a.none {
            ReadNone::Refuse => Err(()),
            ReadNone::End | ReadNone::Bad(_) => Ok((n.saturating_sub(1), false)),
            ReadNone::Bytes(m) => Ok((s + usize::from(m) - 1, false)),
        };
    }
    match records.rules.get(usize::from(reader.other)) {
        Some(&EndRule::Read(_)) | None => Err(()),
        Some(&rule) => Ok((last_by_rule(records, rule, s, bytes, outputs).unwrap_or(s), false)),
    }
}

/// A token's last byte from its first, `s`, by its rule, over the file's output words; `None`
/// where a `Last` has no bit left. A run past the words ends at their end.
pub fn last_by_rule(records: &Records, rule: EndRule, s: usize, bytes: &[u8], outputs: &[Vector<u64>]) -> Option<usize> {
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
            Some(t.last(bytes, s))
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
pub fn lasts(records: &Records, bytes: &[u8], starts: &[u32], outputs: &[Vector<u64>]) -> Option<Vector<u32>> {
    starts
        .iter()
        .map(|&s| {
            let first = bytes.get(s as usize).copied().unwrap_or(0);
            let k = records.ends[usize::from(first)];
            match records.rules.get(usize::from(k)) {
                // A region's token: to its closer (`read_last`), a region tier 2 reads in parts
                // whole (`Split::Tier2`).
                // Split in tier 1, its parts are rows of their own, and its opener is a byte.
                Some(&EndRule::Read(i)) if records.readers[usize::from(i)].parts_in_tier1() => Some(s),
                Some(&EndRule::Read(i)) => read_last(records, &records.readers[usize::from(i)], s as usize, bytes, outputs).ok().map(|l| l.max(s as usize) as u32),
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
pub fn walk(records: &Records, skip: u16, bytes: &[u8], outputs: &[Vector<u64>]) -> (Vector<u32>, Vector<u32>, bool, bool, bool) {
    let n = bytes.len();
    let blank = outputs.get(usize::from(skip)).map_or(&[][..], |v| v.as_slice());
    let held = |q: usize| !blank.get(q).copied().unwrap_or(0);
    let (mut starts, mut lasts) = (rerail_blocks::vector::Vector::new(), rerail_blocks::vector::Vector::new());
    let mut at = records.start(bytes);
    let mut untaken = false;
    let mut checked = false;
    let mut read = false;
    // The records a template left open (decision 0600): a string's, over its template's code, as
    // the lean lexer's string reader keeps its string over the code.
    let mut open: Vec<usize> = Vec::new();
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
                    Some(tokens) => {
                        let l = tokens.last().map_or(s, |t| t.1);
                        for (a, b) in tokens {
                            starts.push(a as u32);
                            lasts.push(b as u32);
                        }
                        at = l + 1;
                    }
                    None => {
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
                let reader = &records.readers[usize::from(i)];
                let (l, template) = match read_end(records, reader, s, bytes, outputs) {
                    Ok((l, t)) => (l.max(s), t),
                    Err(()) => {
                        read = true;
                        (s, false)
                    }
                };
                // A region open at the file's end (`ReadNone::Bad`) is no token of its kind: none
                // of its kind's tests, as the lean lexer reads no escape in its `Bad`.
                let bad = reader.open_kind(s, l, outputs).is_some();
                checked |= !bad && records.checks.iter().any(|c| c.rule == k && c.then == CheckThen::Escape && test_holds(records, c, s, l, outputs));
                // A region that resumes a token's: no record, the open one's end (decision 0600).
                let opens = reader.arms.iter().any(|a| bit_at(outputs, a.opener, s));
                if reader.resumes && opens {
                    match open.last() {
                        Some(&r) => {
                            lasts[r] = l as u32;
                            if !template {
                                open.pop();
                            }
                        }
                        None => read = true,
                    }
                    at = l + 1;
                    continue;
                }
                if template {
                    // A template in a template's code: refused, as by the CPU's loop.
                    read |= !open.is_empty();
                    open.push(starts.len());
                }
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

/// Whether output `o` has a bit at byte `i`.
fn bit_at(outputs: &[Vector<u64>], o: u16, i: usize) -> bool {
    outputs.get(usize::from(o)).and_then(|w| w.get(i / 64)).is_some_and(|w| w >> (i % 64) & 1 == 1)
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
    pub keys: Vector<u64>,
    pub meta: Vector<u32>,
    /// Per word (code − 1): its bytes 8.. as seven words, zeros past its end.
    pub tails: Vector<[u64; 7]>,
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
                let mut table = WordTable { mult: m, bits, keys: rerail_blocks::vector![0; 1 << bits], meta: rerail_blocks::vector![0; 1 << bits], tails: rerail_blocks::vector::Vector::new() };
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
pub fn pass(records: &Records, table: &WordTable, bytes: &[u8], starts: &[u32], lasts: &[u32], outputs: &[Vector<u64>]) -> (Vector<u32>, bool) {
    let mut bad = starts.len() != lasts.len();
    let n = starts.len().min(lasts.len());
    let mut codes = rerail_blocks::vector::Vector::with_capacity(n);
    let mut after = 0u64;
    // The records open around the token (decision 0600): a string's whose template's code it is
    // in, each its last; a token starts after the one before, or inside the open one's, and ends
    // in it.
    let mut around: Vec<u64> = Vec::new();
    for k in 0..n {
        let (s, l) = (starts[k] as usize, lasts[k] as usize);
        while around.last().is_some_and(|&e| (s as u64) > e) {
            around.pop();
        }
        let inside = around.last().is_some_and(|&e| l as u64 <= e) && k > 0 && starts[k - 1] < starts[k];
        bad |= l < s || (s as u64) < after && !inside;
        after = l as u64 + 1;
        let rule = records.ends.get(usize::from(bytes.get(s).copied().unwrap_or(0))).and_then(|&e| records.rules.get(usize::from(e)));
        if rule.is_some_and(|r| matches!(r, EndRule::Read(i) if records.readers[usize::from(*i)].arms.iter().any(|a| a.template.is_some()))) {
            around.push(l as u64);
            after = s as u64 + 1;
        }
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
            if !t.part {
                word = 0;
            }
        }
        // A region open at the file's end (`ReadNone::Bad`): its kind, no word, none of its
        // reader's marks.
        if let Some(EndRule::Read(i)) = rule
            && let Some(k) = records.readers[usize::from(*i)].open_kind(s, l, outputs)
        {
            (kind, word, marks) = (u32::from(k), 0, 0);
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
pub fn any_in(outputs: &[Vector<u64>], o: u16, a: usize, b: usize) -> bool {
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
pub fn gap_marks(records: &Records, bytes: &[u8], starts: &[u32], lasts: &[u32], outputs: &[Vector<u64>], codes: &mut [u32]) -> bool {
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
    // The record before's first byte, past it: an extra's own gap starts there.
    let mut from_record = 0usize;
    for (k, &s) in starts.iter().enumerate().take(codes.len()) {
        let s = s as usize;
        let h = hits(k);
        if extra(codes[k]) {
            // An extra's own: the marks of the gap before it it keeps (`Records::own`), as the
            // lean lexer's `skip` knows a line end before each comment it passes.
            let own = (0..records.marks.len()).filter(|&m| records.own >> m & 1 == 1 && records.gaps >> m & 1 == 1).fold(0u32, |a, m| if any_in(outputs, records.marks[m].0, from_record, s) { a | 1 << m } else { a });
            codes[k] = codes[k] & !(records.gaps << MARK_SHIFT) | own << MARK_SHIFT;
            pending |= h & low | h >> AFTER_SHIFT;
            from_record = s + 1;
            continue;
        }
        from_record = s + 1;
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
        let words = outputs.get(usize::from(o)).map_or(0, |w| w.len());
        pending >> m & 1 == 1 || records.tested_only >> m & 1 == 0 && words > 0 && any_in(outputs, o, from, 64 * words - 1)
    })
}

/// The marks the token loop sets itself (decision 0177), their masks made from the bytes
/// (`Reads`): each `(mark, output)`. The gap marks' (`lift`), from the bytes between grammar
/// tokens, as `records::gap_marks`; the others', at a token's first byte. A mark a reader gives
/// (`Reader::retag`) or of tests alone is not.
pub fn loop_marks(r: &crate::records::Records, bytewise: &dyn Fn(u16) -> bool) -> (Vec<(u8, u16)>, Vec<(u8, u16)>) {
    if r.skip.is_none() {
        return (Vec::new(), Vec::new());
    }
    let retagged: u32 = r.readers.iter().flat_map(|rd| rd.tags()).fold(0, |a, (_, m)| a | m);
    let blank = blank_bits(r, bytewise);
    let known = extra_known(r, bytewise);
    let mut lift = Vec::new();
    let mut at = Vec::new();
    for (k, &(o, w)) in r.marks.iter().enumerate() {
        if o == crate::records::WORD_ALONE || w != 0 || retagged >> k & 1 == 1 || r.tested_only >> k & 1 == 1 || r.alone >> k & 1 == 1 || blank >> k & 1 == 1 || !bytewise(o) {
            continue;
        }
        if r.gaps >> k & 1 == 1 {
            if known {
                lift.push((k as u8, o));
            }
        } else {
            at.push((k as u8, o));
        }
    }
    (lift, at)
}

/// The gap marks tier 2 on the CPU tests itself, where a state asks (decision 0177): each
/// `(mark, output)`, a gap mark made from the bytes (no `else escape`, not the extras'). The
/// token loop makes none of their gaps; tier 2 reads the mask from the grammar token before to
/// the token, as the lean parser asks whether a token is on a line only where a statement asks
/// it (`on_line`), and reads a JSON document's bytes only where it takes one. A mark with token
/// tests too (`lazy_coded`) keeps them in the code, as a lexer marks what it reads: tier 2
/// takes its bit there first. Metal and the interpreter set them in the codes: the compiler
/// chooses by the target, the GPU's tier 1 making every mark at once.
pub fn lazy_marks(r: &crate::records::Records, bytewise: &dyn Fn(u16) -> bool) -> Vec<(u8, u16)> {
    // A mark with tests and an `unless after` stays the loop's: the word before holds its
    // tests' hits too. So does one a token pattern's `without` reads (`Records::read`).
    let coded = lazy_coded(r);
    let held = |k: u8| coded >> k & 1 == 1 && r.unless.iter().any(|&(m, _)| m == k);
    loop_marks(r, bytewise).0.into_iter().filter(|&(k, _)| r.tested_only >> k & 1 == 0 && r.alone >> k & 1 == 0 && r.read >> k & 1 == 0 && r.unplaced >> k & 1 == 0 && r.extra != Some(k) && !held(k)).collect()
}

/// The code bits of `block`'s gap marks tier 2 on the CPU tests itself (`lazy_marks`): the
/// CPU's tier 1 sets none of them, where Metal and the interpreter do. None without records.
pub fn lazy_code_bits(block: &crate::block_native::BlockNative) -> u32 {
    let Some(r) = block.records.as_ref() else {
        return 0;
    };
    let bw = crate::byte_needs::MadeFromBytes::of(block);
    lazy_marks(r, &|o| bw.can(o)).iter().fold(0, |a, &(k, _)| a | 1 << (crate::records::MARK_SHIFT + u32::from(k)))
}

/// The marks of `lazy_marks` with token tests too: their tests' hits are in the code (the
/// token loop's), their gaps tier 2's.
pub fn lazy_coded(r: &crate::records::Records) -> u32 {
    r.checks.iter().fold(0, |a, c| match c.then {
        crate::records::CheckThen::At(k) | crate::records::CheckThen::After(k) => a | 1 << k,
        crate::records::CheckThen::Escape | crate::records::CheckThen::Cut => a,
    })
}

/// The gap marks the token loop reads from the blanks (`Records::blanks`, decision 0430): each
/// `(mark, class output)`, the class read from the bytes. While it passes the blank run before
/// each token, the loop tests the class there, as the lean lexer's `skip` notes `nl`; an
/// extra's run goes on to the grammar token after it. No mask of the mark is made.
pub fn blank_marks(r: &crate::records::Records, bytewise: &dyn Fn(u16) -> bool) -> Vec<(u8, u16)> {
    if r.skip.is_none() {
        return Vec::new();
    }
    if !extra_known(r, bytewise) {
        return Vec::new();
    }
    r.blanks.iter().copied().filter(|&(_, o)| bytewise(o)).collect()
}

/// Whether the token loop knows an extra at its token (Q347): it tells one by the extra's mark
/// in the code, so the mark must be the loop's there (a reader's, or its first byte's), not
/// set after the tokens (`late_marks`). The gap marks the loop sets (`loop_marks`' lifted,
/// `blank_marks`) go past an extra to the grammar token after it only so.
fn extra_known(r: &crate::records::Records, bytewise: &dyn Fn(u16) -> bool) -> bool {
    let Some(e) = r.extra else { return true };
    let retagged: u32 = r.readers.iter().flat_map(|rd| rd.tags()).fold(0, |a, (_, m)| a | m);
    let (o, w) = r.marks[usize::from(e)];
    let at_first = o != crate::records::WORD_ALONE && w == 0 && r.gaps >> e & 1 == 0 && r.tested_only >> e & 1 == 0 && r.alone >> e & 1 == 0 && bytewise(o);
    retagged >> e & 1 == 1 || at_first
}

/// `blank_marks`' marks, as bits `1 << mark`.
pub fn blank_bits(r: &crate::records::Records, bytewise: &dyn Fn(u16) -> bool) -> u32 {
    blank_marks(r, bytewise).iter().fold(0, |a, &(k, _)| a | 1 << k)
}
