//! One API over every tape format, read in place (decision 0293).
//!
//! A `Tape` is one document's tree as its format holds it. Every question is answered on the format's own
//! bytes, where they lie: nothing turns a tape into an array of nodes first. Node ids are preorder numbers,
//! `0..len`.
//!
//! - **Codes**: a node's code is a number in a `CodeTable` (`Tape::table`): kind, field, flags, alias and
//!   supertype classes. A kind test is a number test; a set of kinds is a `CodeSet` made once a table.
//! - **Structure**: the subtree's end, the first child, the next sibling, the parent.
//! - **Spans**: a node's start and end at a point, and a cursor's step in preorder.
//! - **In bulk**: a code set tested 64 nodes at a time, every span in order.
//!
//! The formats behind it (`AnyTape`): format 15 over its image's columns (`F15`), format 16 over its arrays
//! (`crate::f16::F16`). A caller dispatches once a document (`with_tape!`), so the per-node reads are compiled
//! for each format.

use crate::reads::f15 as reads;
use crate::{CHECKPOINT_STRIDE, HEADER_BYTES, MAGIC, FORMAT_VERSION, TAIL_FROM_START, header, padded};

/// What a code is: the grammar's kind and field, the flags (`CODE_EXTRA`, `CODE_ERROR`, `CODE_MISSING`), a
/// contextual alias (its displayed name and whether it is named; format 15 numbers them per tape, so a table
/// holds the name) and the supertype classes the node is a member of, ascending. A table is made of these
/// (`CodeTable::new`, `encode`); it is read as `EntryRef`s, where its words lie.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entry {
    pub kind: u16,
    pub field: u16,
    pub flags: u8,
    pub alias: Option<(String, bool)>,
    pub classes: Vec<u16>,
}

impl Entry {
    /// The entry as a table holds it.
    pub fn as_ref(&self) -> EntryRef<'_> {
        EntryRef { kind: self.kind, field: self.field, flags: self.flags, alias: self.alias.as_ref().map(|(s, n)| (s.as_str(), *n)), classes: &self.classes }
    }
}

impl<'a> From<&'a Entry> for EntryRef<'a> {
    fn from(e: &'a Entry) -> EntryRef<'a> {
        e.as_ref()
    }
}

/// A code as its table's words hold it (decision 0569 §2): read in place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EntryRef<'a> {
    pub kind: u16,
    pub field: u16,
    pub flags: u8,
    pub alias: Option<(&'a str, bool)>,
    pub classes: &'a [u16],
}

impl EntryRef<'_> {
    pub fn to_owned(&self) -> Entry {
        Entry { kind: self.kind, field: self.field, flags: self.flags, alias: self.alias.map(|(s, n)| (s.to_owned(), n)), classes: self.classes.to_vec() }
    }
    fn hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut mix = |x: u64| h = (h ^ x).wrapping_mul(0x0000_0100_0000_01b3);
        mix(u64::from(self.kind) | u64::from(self.field) << 16 | u64::from(self.flags) << 32);
        match self.alias {
            None => mix(1),
            Some((s, named)) => {
                mix(2 + u64::from(named));
                s.bytes().for_each(|b| mix(u64::from(b)));
            }
        }
        self.classes.iter().for_each(|&c| mix(u64::from(c) | 1 << 20));
        h
    }
}

/// The words a code table or a set is read from, where its owner keeps them: the engine's in a block, a tool's or a
/// test's on the heap.
pub type Words = std::sync::Arc<dyn AsRef<[u32]> + Send + Sync>;

/// Where a table made with no owner's words (`CodeTable::new`) is placed: the engine registers a block's (decision
/// 0569 §2), once, at load; without it the words are the heap's.
static PLACE: std::sync::OnceLock<fn(&[u32]) -> Words> = std::sync::OnceLock::new();

/// The engine's placing of a table's words (`PLACE`).
pub fn place_tables_with(place: fn(&[u32]) -> Words) {
    let _ = PLACE.set(place);
}

// The table's words: a header, then the codes' records, their classes (u16s), their aliases' bytes, its index by
// kind and by class (starts, then numbers), the codes with `CODE_ERROR`, with `CODE_MISSING` and either, and a
// hash of the entries (a code + 1 a slot, 0 for none).
const H_N: usize = 0;
const H_KINDS: usize = 1;
const H_CLASSES: usize = 2;
const H_REC: usize = 3;
const H_CLS: usize = 4;
const H_CLS_N: usize = 5;
const H_STR: usize = 6;
const H_STR_N: usize = 7;
const H_KIND_STARTS: usize = 8;
const H_KIND_NUMS: usize = 9;
const H_CLASS_STARTS: usize = 10;
const H_CLASS_NUMS: usize = 11;
const H_ERR: usize = 12;
const H_ERR_N: usize = 13;
const H_MISS: usize = 14;
const H_MISS_N: usize = 15;
const H_FAULT: usize = 16;
const H_FAULT_N: usize = 17;
const H_HASH: usize = 18;
const H_HASH_N: usize = 19;
const HEADER_WORDS: usize = 20;
/// A record: kind | field << 16; flags | has an alias << 8 | named << 9; its classes' start and count; its alias's
/// byte start and length.
const REC: usize = 6;

/// The codes a tape's numbers name, by number, and its index of them by kind and by class: encoded words, read where
/// they lie (decision 0569 §2). On format 16 every document of a language shares its table (`f16::Lang`).
#[derive(Clone)]
pub struct CodeTable {
    words: Words,
}

impl std::fmt::Debug for CodeTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.entries()).finish()
    }
}

impl Default for CodeTable {
    fn default() -> Self {
        CodeTable::new(Vec::new())
    }
}

impl PartialEq for CodeTable {
    fn eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.entries().zip(other.entries()).all(|(a, b)| a == b)
    }
}
impl Eq for CodeTable {}

impl CodeTable {
    /// The table of `entries`, its words where the engine places tables (`place_tables_with`), else the heap's.
    pub fn new(entries: Vec<Entry>) -> Self {
        let words = Self::encode(&entries);
        // copy: derived: the table encoded once, then placed where the engine keeps tables
        CodeTable { words: match PLACE.get() { Some(place) => place(&words), None => std::sync::Arc::new(words.into_boxed_slice()) } }
    }

    /// The words of the table of `entries` (`over` reads them).
    pub fn encode(entries: &[Entry]) -> Vec<u32> {
        let n = entries.len();
        let kinds = entries.iter().map(|e| usize::from(e.kind) + 1).max().unwrap_or(0);
        let classes = entries.iter().flat_map(|e| e.classes.iter().map(|&c| usize::from(c) + 1)).max().unwrap_or(0);
        let cls_n: usize = entries.iter().map(|e| e.classes.len()).sum();
        let str_n: usize = entries.iter().map(|e| e.alias.as_ref().map_or(0, |a| a.0.len())).sum();
        let by = |key: &dyn Fn(&Entry) -> Vec<usize>, of: usize| -> (Vec<u32>, Vec<u32>) {
            let mut starts = vec![0u32; of + 1];
            for e in entries {
                for k in key(e) {
                    starts[k + 1] += 1;
                }
            }
            for k in 0..of {
                starts[k + 1] += starts[k];
            }
            let mut at = starts.clone();
            let mut nums = vec![0u32; starts[of] as usize];
            for (i, e) in entries.iter().enumerate() {
                for k in key(e) {
                    nums[at[k] as usize] = i as u32;
                    at[k] += 1;
                }
            }
            (starts, nums)
        };
        let (kind_starts, kind_nums) = by(&|e| if e.flags & crate::CODE_ERROR == 0 { vec![usize::from(e.kind)] } else { Vec::new() }, kinds);
        let (class_starts, class_nums) = by(&|e| e.classes.iter().map(|&c| usize::from(c)).collect(), classes);
        let flagged = |f: u8| -> Vec<u32> { (0..n as u32).filter(|&i| entries[i as usize].flags & f != 0).collect() };
        let (errors, missing, faults) = (flagged(crate::CODE_ERROR), flagged(crate::CODE_MISSING), flagged(crate::CODE_ERROR | crate::CODE_MISSING));
        let slots = (2 * n).next_power_of_two().max(2);
        let mut w = vec![0u32; HEADER_WORDS];
        let section = |w: &mut Vec<u32>, at: usize, words: &[u32]| {
            w[at] = w.len() as u32;
            w.extend_from_slice(words);
        };
        w[H_N] = n as u32;
        w[H_KINDS] = kinds as u32;
        w[H_CLASSES] = classes as u32;
        w[H_REC] = w.len() as u32;
        let (mut cls_at, mut str_at) = (0u32, 0u32);
        for e in entries {
            let (alias, named, len) = e.alias.as_ref().map_or((0, 0, 0), |(s, named)| (1, u32::from(*named), s.len() as u32));
            w.extend_from_slice(&[u32::from(e.kind) | u32::from(e.field) << 16, u32::from(e.flags) | alias << 8 | named << 9, cls_at, e.classes.len() as u32, str_at, len]);
            cls_at += e.classes.len() as u32;
            str_at += len;
        }
        // The classes, two to a word.
        let cls: Vec<u16> = entries.iter().flat_map(|e| e.classes.iter().copied()).collect();
        let cls_words: Vec<u32> = cls.chunks(2).map(|p| u32::from(p[0]) | u32::from(*p.get(1).unwrap_or(&0)) << 16).collect();
        section(&mut w, H_CLS, &cls_words);
        w[H_CLS_N] = cls_n as u32;
        let bytes: Vec<u8> = entries.iter().filter_map(|e| e.alias.as_ref()).flat_map(|a| a.0.bytes()).collect();
        let str_words: Vec<u32> = bytes.chunks(4).map(|c| {
            let mut b = [0u8; 4];
            b[..c.len()].copy_from_slice(c);
            u32::from_le_bytes(b)
        }).collect();
        section(&mut w, H_STR, &str_words);
        w[H_STR_N] = str_n as u32;
        section(&mut w, H_KIND_STARTS, &kind_starts);
        section(&mut w, H_KIND_NUMS, &kind_nums);
        section(&mut w, H_CLASS_STARTS, &class_starts);
        section(&mut w, H_CLASS_NUMS, &class_nums);
        section(&mut w, H_ERR, &errors);
        w[H_ERR_N] = errors.len() as u32;
        section(&mut w, H_MISS, &missing);
        w[H_MISS_N] = missing.len() as u32;
        section(&mut w, H_FAULT, &faults);
        w[H_FAULT_N] = faults.len() as u32;
        let mut hash = vec![0u32; slots];
        for (i, e) in entries.iter().enumerate() {
            let mut s = e.as_ref().hash() as usize & (slots - 1);
            while hash[s] != 0 {
                s = (s + 1) & (slots - 1);
            }
            hash[s] = i as u32 + 1;
        }
        section(&mut w, H_HASH, &hash);
        w[H_HASH_N] = slots as u32;
        w
    }

    /// The table in `words` (`encode`'s), wherever its owner keeps them: its header, its sections' bounds, its
    /// aliases' text checked.
    pub fn over(words: Words) -> Result<Self, String> {
        let w = (*words).as_ref();
        if w.len() < HEADER_WORDS {
            return Err("a short code table".into());
        }
        let ends = [
            (H_REC, w[H_N] as usize * REC),
            (H_CLS, (w[H_CLS_N] as usize).div_ceil(2)),
            (H_STR, (w[H_STR_N] as usize).div_ceil(4)),
            (H_KIND_STARTS, w[H_KINDS] as usize + 1),
            (H_CLASS_STARTS, w[H_CLASSES] as usize + 1),
            (H_ERR, w[H_ERR_N] as usize),
            (H_MISS, w[H_MISS_N] as usize),
            (H_FAULT, w[H_FAULT_N] as usize),
            (H_HASH, w[H_HASH_N] as usize),
        ];
        if ends.iter().any(|&(at, n)| w[at] as usize + n > w.len()) {
            return Err("a code table's section past its words".into());
        }
        let t = CodeTable { words };
        std::str::from_utf8(t.str_bytes()).map_err(|_| "a code table's alias is not UTF-8")?;
        if (0..t.len() as u32).any(|i| {
            let r = t.rec(i);
            r[2] as usize + r[3] as usize > t.words()[H_CLS_N] as usize || r[4] as usize + r[5] as usize > t.str_bytes().len()
        }) {
            return Err("a code's classes or alias past its table".into());
        }
        Ok(t)
    }

    #[inline(always)]
    fn words(&self) -> &[u32] {
        (*self.words).as_ref()
    }
    #[inline(always)]
    fn rec(&self, n: u32) -> &[u32] {
        let at = self.words()[H_REC] as usize + n as usize * REC;
        &self.words()[at..at + REC]
    }
    fn section(&self, at: usize, n: usize) -> &[u32] {
        let s = self.words()[at] as usize;
        &self.words()[s..s + n]
    }
    fn str_bytes(&self) -> &[u8] {
        let s = self.section(H_STR, (self.words()[H_STR_N] as usize).div_ceil(4));
        // SAFETY: the section's words, read as their bytes, `H_STR_N` of them.
        unsafe { std::slice::from_raw_parts(s.as_ptr().cast::<u8>(), self.words()[H_STR_N] as usize) }
    }
    fn classes_all(&self) -> &[u16] {
        let s = self.section(H_CLS, (self.words()[H_CLS_N] as usize).div_ceil(2));
        // SAFETY: the section's words, read as their u16s (little-endian, as `encode` packed them), `H_CLS_N` of them.
        unsafe { std::slice::from_raw_parts(s.as_ptr().cast::<u16>(), self.words()[H_CLS_N] as usize) }
    }
    /// The words it is read from, for an owner that keeps them elsewhere.
    pub fn as_words(&self) -> &[u32] {
        self.words()
    }

    /// The codes of kind `kind` with no `CODE_ERROR`.
    pub fn of_kind(&self, kind: u16) -> CodeSet<'_> {
        self.index(H_KIND_STARTS, H_KIND_NUMS, H_KINDS, kind)
    }
    /// The codes whose classes hold `class`.
    pub fn of_class(&self, class: u16) -> CodeSet<'_> {
        self.index(H_CLASS_STARTS, H_CLASS_NUMS, H_CLASSES, class)
    }
    fn index(&self, starts: usize, nums: usize, of: usize, k: u16) -> CodeSet<'_> {
        if usize::from(k) >= self.words()[of] as usize {
            return CodeSet::default();
        }
        let s = self.section(starts, self.words()[of] as usize + 1);
        let (a, b) = (s[usize::from(k)] as usize, s[usize::from(k) + 1] as usize);
        CodeSet::Few(&self.section(nums, b)[a..b])
    }
    /// The codes with `CODE_ERROR`.
    pub fn errors(&self) -> CodeSet<'_> {
        CodeSet::Few(self.section(H_ERR, self.words()[H_ERR_N] as usize))
    }
    /// The codes with `CODE_MISSING`.
    pub fn missing(&self) -> CodeSet<'_> {
        CodeSet::Few(self.section(H_MISS, self.words()[H_MISS_N] as usize))
    }
    /// The codes with `CODE_ERROR` or `CODE_MISSING`.
    pub fn faults(&self) -> CodeSet<'_> {
        CodeSet::Few(self.section(H_FAULT, self.words()[H_FAULT_N] as usize))
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.words()[H_N] as usize
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Code `number`'s entry.
    #[inline(always)]
    pub fn entry(&self, number: u32) -> EntryRef<'_> {
        let r = self.rec(number);
        let alias = (r[1] >> 8 & 1 != 0).then(|| {
            let b = &self.str_bytes()[r[4] as usize..(r[4] + r[5]) as usize];
            // SAFETY: `over` checked the aliases' bytes are UTF-8, and `encode` wrote each alias whole.
            (unsafe { std::str::from_utf8_unchecked(b) }, r[1] >> 9 & 1 != 0)
        });
        EntryRef { kind: r[0] as u16, field: (r[0] >> 16) as u16, flags: r[1] as u8, alias, classes: &self.classes_all()[r[2] as usize..(r[2] + r[3]) as usize] }
    }
    /// The entries, by number.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = EntryRef<'_>> + '_ {
        (0..self.len() as u32).map(|n| self.entry(n))
    }
    /// `entry`'s number, if the table has it.
    pub fn number(&self, entry: EntryRef<'_>) -> Option<u32> {
        let slots = self.section(H_HASH, self.words()[H_HASH_N] as usize);
        let mut s = entry.hash() as usize & (slots.len() - 1);
        loop {
            match slots[s] {
                0 => return None,
                k if self.entry(k - 1) == entry => return Some(k - 1),
                _ => s = (s + 1) & (slots.len() - 1),
            }
        }
    }
    /// The codes for which `member` holds, as a set by number, its bits into `out` (`set_words` of them).
    pub fn set_into(&self, member: impl Fn(EntryRef<'_>) -> bool, out: &mut [u32]) {
        out.fill(0);
        for (n, e) in self.entries().enumerate() {
            if member(e) {
                out[n / 32] |= 1 << (n % 32);
            }
        }
    }
    /// The words a set of this table's codes by bits takes (`set_into`).
    pub fn set_words(&self) -> usize {
        self.len().div_ceil(32)
    }
    /// The codes for which `member` holds, its bits on the heap (a tool's, a test's).
    pub fn set(&self, member: impl Fn(EntryRef<'_>) -> bool) -> CodeSetBuf {
        let mut bits = vec![0u32; self.set_words()];
        self.set_into(member, &mut bits);
        CodeSetBuf { words: std::sync::Arc::new(bits.into_boxed_slice()), dense: true, n: self.len() as u32 }
    }
}

/// A set of codes of one table, by number: what a kind test reads, where its words lie. Either a bit per code of the
/// table (a set made by a test of every entry), or the few numbers it holds, ascending (a kind's or a class's, from
/// the table's index).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeSet<'a> {
    Dense(&'a [u32], u32),
    Few(&'a [u32]),
}

impl Default for CodeSet<'_> {
    fn default() -> Self {
        CodeSet::Few(&[])
    }
}

impl<'a> CodeSet<'a> {
    #[inline(always)]
    pub fn contains(&self, number: u32) -> bool {
        match *self {
            CodeSet::Dense(bits, n) => number < n && bits[number as usize / 32] >> (number % 32) & 1 != 0,
            CodeSet::Few(few) if few.len() <= 8 => few.contains(&number),
            CodeSet::Few(few) => few.binary_search(&number).is_ok(),
        }
    }
    /// The numbers of a set made from numbers, ascending; a set of a bit per code has none to give.
    pub fn numbers(&self) -> Option<&'a [u32]> {
        match *self {
            CodeSet::Dense(..) => None,
            CodeSet::Few(few) => Some(few),
        }
    }
    /// Marks in `out` each number of the set below `cut` (at most 256), clearing the rest: a set of a few numbers
    /// costs its members, not a test of every number below the cut.
    pub fn mark_below(&self, cut: usize, out: &mut [bool; 256]) {
        let cut = cut.min(256);
        *out = [false; 256];
        match *self {
            CodeSet::Dense(_, n) => (0..cut.min(n as usize)).for_each(|i| out[i] = self.contains(i as u32)),
            CodeSet::Few(few) => few.iter().take_while(|&&n| (n as usize) < cut).for_each(|&n| out[n as usize] = true),
        }
    }
    /// Whether the set holds a number `from` or past it.
    pub fn any_from(&self, from: usize) -> bool {
        match *self {
            CodeSet::Dense(_, n) => (from..n as usize).any(|i| self.contains(i as u32)),
            CodeSet::Few(few) => few.last().is_some_and(|&n| n as usize >= from),
        }
    }
    pub fn is_empty(&self) -> bool {
        match *self {
            CodeSet::Dense(bits, _) => bits.iter().all(|&w| w == 0),
            CodeSet::Few(few) => few.is_empty(),
        }
    }
}

/// A set's words, held: a tool's or a test's on the heap (`CodeTable::set`, `of_numbers`), the engine's in a block.
#[derive(Clone)]
pub struct CodeSetBuf {
    words: Words,
    dense: bool,
    n: u32,
}

impl std::fmt::Debug for CodeSetBuf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.view().fmt(f)
    }
}

impl CodeSetBuf {
    /// The set of `numbers` (any order, repeats allowed), on the heap.
    pub fn of_numbers(numbers: impl IntoIterator<Item = u32>) -> CodeSetBuf {
        let mut v: Vec<u32> = numbers.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        CodeSetBuf { words: std::sync::Arc::new(v.into_boxed_slice()), dense: false, n: 0 }
    }
    /// The set in `words`: a bit per code of a table of `n` (`dense`), or numbers, ascending.
    pub fn over(words: Words, dense: bool, n: u32) -> CodeSetBuf {
        CodeSetBuf { words, dense, n }
    }
    pub fn view(&self) -> CodeSet<'_> {
        let w = (*self.words).as_ref();
        if self.dense { CodeSet::Dense(w, self.n) } else { CodeSet::Few(w) }
    }
}

/// One document's tree, read in place. Ids are preorder numbers.
pub trait Tape {
    /// The nodes.
    fn len(&self) -> usize;
    /// Whether block `block` (nodes `f16::Z * block ..`; `u32::MAX`: the document) may have a node of kind `kind`,
    /// by the tape's presence in its home (`tape_has_kind`, decision 0434 §4, step 6): true for a tape that has none.
    fn has_kind(&self, block: u32, kind: u16) -> bool {
        let _ = (block, kind);
        true
    }
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The table the codes are numbers of.
    fn table(&self) -> &CodeTable;
    /// Node `i`'s code, a number in `table`.
    fn code(&self, i: usize) -> u32;
    /// The id just past `i`'s subtree.
    fn subtree_end(&self, i: usize) -> usize;
    /// `i`'s first child.
    #[inline(always)]
    fn first_child(&self, i: usize) -> Option<usize> {
        (self.subtree_end(i) > i + 1).then_some(i + 1)
    }
    /// The sibling after `i`, a child of a node whose subtree ends at `parent_end`.
    #[inline(always)]
    fn next_sibling(&self, i: usize, parent_end: usize) -> Option<usize> {
        let e = self.subtree_end(i);
        (e < parent_end).then_some(e)
    }
    /// `i`'s parent.
    fn parent(&self, i: usize) -> Option<usize>;
    /// `i`'s parent, where the caller knows an ancestor of it, `a` (whose subtree holds `i`): a format whose parent
    /// read is a search may start it there. Its parent read by default.
    #[inline(always)]
    fn parent_within(&self, i: usize, _a: usize) -> Option<usize> {
        self.parent(i)
    }
    fn start(&self, i: usize) -> u64;
    fn end(&self, i: usize) -> u64;
    /// A cursor's step in preorder: `i`'s start, given the start of the node before it (unused for the first). A
    /// format that stores starts relatively answers with one gap.
    #[inline(always)]
    fn start_next(&self, i: usize, _prev_start: u64) -> u64 {
        self.start(i)
    }
    /// `i`'s end, its start in hand.
    #[inline(always)]
    fn end_with_start(&self, i: usize, _start: u64) -> u64 {
        self.end(i)
    }
    /// `match_groups` over the blocks `block` admits alone (block `b`: nodes `f16::Z * b ..`), as a planner tells
    /// them from the tape's presence (`has_kind`): a format 16 or 16z tape reads no codes of a block left out.
    fn match_groups_in(&self, set: &CodeSet, block: &dyn Fn(u32) -> bool, mut f: impl FnMut(usize, u64)) {
        self.match_groups(set, |base, m| {
            if block((base / crate::f16::Z) as u32) {
                f(base, m);
            }
        });
    }
    /// The codes 64 nodes at a time: `f(base, m)`, bit j of `m` set when node `base + j` is in `set`.
    fn match_groups(&self, set: &CodeSet, mut f: impl FnMut(usize, u64)) {
        let n = self.len();
        for base in (0..n).step_by(64) {
            let mut m = 0u64;
            for j in 0..64.min(n - base) {
                m |= u64::from(set.contains(self.code(base + j))) << j;
            }
            f(base, m);
        }
    }
    /// `i`'s ancestors, the root first, into `out`: one descent from the root, a step a sibling it passes (read
    /// from the subtree ends), so no parent is read. A climb's whole path at the cost of one.
    fn path(&self, i: usize, out: &mut Vec<usize>) {
        out.clear();
        if i == 0 || i >= self.len() {
            return;
        }
        let mut at = 0;
        loop {
            out.push(at);
            let end = self.subtree_end(at);
            let mut c = at + 1;
            loop {
                let e = self.subtree_end(c);
                if i < e {
                    break;
                }
                debug_assert!(e < end, "node {i} is in {at}'s subtree");
                c = e;
            }
            if c == i {
                return;
            }
            at = c;
        }
    }
    /// Every node's span, in preorder.
    fn spans(&self, out: &mut Vec<(u64, u64)>) {
        out.clear();
        out.reserve(self.len());
        let mut s = 0;
        for i in 0..self.len() {
            s = self.start_next(i, s);
            out.push((s, self.end_with_start(i, s)));
        }
    }
}

/// `i`'s children, in order.
pub fn children<T: Tape + ?Sized>(t: &T, i: usize) -> impl Iterator<Item = usize> + '_ {
    let end = t.subtree_end(i);
    let mut at = t.first_child(i);
    std::iter::from_fn(move || {
        let c = at?;
        at = t.next_sibling(c, end);
        Some(c)
    })
}

// ---- format 15, in place -----------------------------------------------------------------------------------

/// A format 15 image's columns, at their widths: `code` (2 bytes a node, the tape's own code ids), `size`, `base`
/// (4 bytes each `CHECKPOINT_STRIDE` nodes), `off`, `tail` (`widths`, 2 or 4 bytes; `from_start`: a tail is from
/// the node's own start, else from its last descendant's).
#[derive(Clone, Copy)]
pub struct F15Columns<'a> {
    pub code: &'a [u8],
    pub size: &'a [u8],
    pub base: &'a [u8],
    pub off: &'a [u8],
    pub tail: &'a [u8],
    pub widths: [u8; 3],
    pub from_start: bool,
    /// The source's length (its header's, or its image's): a view's spans are held to it (decision 0463).
    pub source_len: u64,
}

/// Format 15, read where its columns lie, through the tape's reads (`crate::reads`, written directly,
/// decision 0534) over its view. Its codes are the tape's own table (numbered by first use); its parent is not
/// stored, so it is found back over the nodes before (the format's cost: 0156 §1.7).
#[derive(Clone, Copy)]
pub struct F15<'a> {
    bytes: &'a [u8],
    view: rerail_clif::view::TapeView,
    table: &'a CodeTable,
}

impl<'a> F15<'a> {
    /// The tape of `columns`, which lie in `within`, whose code ids `table` names. Checked: the widths, the
    /// columns' lengths, each code id and size in range.
    pub fn new(within: &'a [u8], columns: F15Columns<'a>, table: &'a CodeTable) -> Result<Self, String> {
        if columns.widths.iter().any(|&w| w != 2 && w != 4) {
            return Err("a tape column of another width".into());
        }
        let n = columns.code.len() / 2;
        let w = columns.widths.map(usize::from);
        if columns.size.len() < n * w[0] || columns.off.len() < n * w[1] || columns.tail.len() < n * w[2] || columns.base.len() < 4 * n.div_ceil(CHECKPOINT_STRIDE) {
            return Err("a short tape column".into());
        }
        let t = F15::over(within, columns.view(within, 0)?, table);
        for i in 0..n {
            if t.code(i) as usize >= table.len() {
                return Err("a node's code is not in the table".into());
            }
            if t.subtree_end(i) > n {
                return Err("a node's size runs past the tape".into());
            }
        }
        Ok(t)
    }
    /// The tape `view` places in `bytes` (the engine's own image, whose shape its restore checked): not checked
    /// again, as it is read once a document a pass. A read past `bytes` panics where it is made.
    pub fn over(bytes: &'a [u8], view: rerail_clif::view::TapeView, table: &'a CodeTable) -> Self {
        F15 { bytes, view, table }
    }
}

impl Tape for F15<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.view.nodes as usize
    }
    #[inline(always)]
    fn table(&self) -> &CodeTable {
        self.table
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        reads::number(self.bytes, &self.view, i as u32)
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + reads::size(self.bytes, &self.view, i as u32) as usize
    }
    /// Not stored: the nearest node before `i` whose subtree holds it, searched back over at most 64 nodes, else
    /// found by descending from the root over the siblings' subtree ends (`Tape::path`): a search is never longer
    /// than the path's siblings.
    fn parent(&self, i: usize) -> Option<usize> {
        let p = reads::parent(self.bytes, &self.view, i as u32);
        (p != u32::MAX).then_some(p as usize)
    }
    /// The node before when it holds `i` (a first child), else a descent from `a` over the siblings' subtree ends:
    /// as long as the path's siblings below `a`, not the root's. From the root, a child of a module's top level
    /// hopped every statement before it.
    fn parent_within(&self, i: usize, a: usize) -> Option<usize> {
        if i == 0 {
            return None;
        }
        if self.subtree_end(i - 1) > i {
            return Some(i - 1);
        }
        if a >= i || self.subtree_end(a) <= i {
            return self.parent(i);
        }
        let mut p = a;
        loop {
            let mut c = p + 1;
            loop {
                let end = self.subtree_end(c);
                if end > i {
                    break;
                }
                c = end;
            }
            if c == i {
                return Some(p);
            }
            p = c;
        }
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        u64::from(reads::start(self.bytes, &self.view, i as u32))
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        u64::from(reads::end(self.bytes, &self.view, i as u32))
    }
    /// A scan's span is the tape's as stated (decision 0463: what its check reads; its slices are checked).
    fn start_next(&self, i: usize, _prev_start: u64) -> u64 {
        u64::from(reads::start_stated(self.bytes, &self.view, i as u32))
    }
    fn end_with_start(&self, i: usize, _start: u64) -> u64 {
        u64::from(reads::end_stated(self.bytes, &self.view, i as u32))
    }
}

/// A stored format 15 image's columns and its code table (`MAGIC`, `FORMAT_VERSION`), read where they lie.
/// The view a format 15 tape's reads take (decision 0310), the stored tape at byte `base` of what the view's
/// offsets count from (0: the tape's own bytes).
pub fn f15_view(bytes: &[u8], base: usize) -> Result<rerail_clif::view::TapeView, String> {
    f15_columns(bytes)?.0.view(bytes, base)
}

impl F15Columns<'_> {
    /// The view the reads take (decision 0310): every column lies in `within`, which lies at byte `base` of what
    /// the view's offsets count from.
    pub fn view(&self, within: &[u8], base: usize) -> Result<rerail_clif::view::TapeView, String> {
        let range = within.as_ptr_range();
        let at = |column: &[u8]| {
            let inside = range.start <= column.as_ptr() && column.as_ptr_range().end <= range.end;
            if !inside {
                return Err("a column outside the tape's bytes".to_owned());
            }
            u32::try_from(base + (column.as_ptr() as usize - range.start as usize)).map_err(|_| "a tape placed past 4 GiB".to_owned())
        };
        let widths = u32::from(self.widths[0]) | u32::from(self.widths[1]) << 4 | u32::from(self.widths[2]) << 8 | u32::from(self.from_start) << 12;
        Ok(rerail_clif::view::TapeView {
            nodes: u32::try_from(self.code.len() / 2).map_err(|_| "a tape past 4 G nodes".to_owned())?,
            source_len: u32::try_from(self.source_len).map_err(|_| "a source past 4 GiB".to_owned())?,
            format: 15,
            widths,
            columns: [at(self.code)?, at(self.size)?, at(self.base)?, at(self.off)?, at(self.tail)?],
            ..Default::default()
        })
    }
}

pub fn f15_columns(bytes: &[u8]) -> Result<(F15Columns<'_>, CodeTable), String> {
    let u64_at = |at: usize| -> Result<usize, String> {
        let b = bytes.get(at..at + 8).ok_or("a short tape header")?;
        usize::try_from(u64::from_le_bytes(b.try_into().expect("8 bytes"))).map_err(|_| "a tape length overflows".to_owned())
    };
    if bytes.len() < HEADER_BYTES || &bytes[..8] != MAGIC {
        return Err("not a format 15 tape".into());
    }
    if u32::from_le_bytes(bytes[header::VERSION..header::VERSION + 4].try_into().expect("4 bytes")) != FORMAT_VERSION {
        return Err("a tape of another format".into());
    }
    let widths = [bytes[header::WIDTHS], bytes[header::WIDTHS + 1], bytes[header::WIDTHS + 2] & !TAIL_FROM_START];
    let from_start = bytes[header::WIDTHS + 2] & TAIL_FROM_START != 0;
    let (n, count) = (u64_at(header::NODES)?, u64_at(header::CODES)?);
    let w = widths.map(usize::from);
    let mut at = HEADER_BYTES;
    let mut column = |width: usize, len: usize| -> Result<&[u8], String> {
        let c = bytes.get(at..at + width * len).ok_or("a short tape column")?;
        at = padded(at + width * len);
        Ok(c)
    };
    let (code, size, base, off, tail) = (column(2, n)?, column(w[0], n)?, column(4, n.div_ceil(CHECKPOINT_STRIDE))?, column(w[1], n)?, column(w[2], n)?);
    let mut raw = Vec::with_capacity(count);
    let mut c = at;
    for _ in 0..count {
        let e = bytes.get(c..c + 6).ok_or("a short code table")?;
        let kf = u16::from_le_bytes([e[0], e[1]]);
        let k = usize::from(e[5]);
        let classes: Vec<u16> = bytes.get(c + 6..c + 6 + 2 * k).ok_or("a short code table")?.as_chunks::<2>().0.iter().map(|&p| u16::from_le_bytes(p)).collect();
        raw.push((kf, e[2], u16::from_le_bytes([e[3], e[4]]), classes));
        c += 6 + 2 * k;
    }
    // The alias table: each alias's named byte, its name's length (u32) and its name.
    let mut aliases = Vec::new();
    for _ in 0..u64_at(header::ALIASES)? {
        let named = *bytes.get(c).ok_or("a short alias table")? != 0;
        let len = u32::from_le_bytes(bytes.get(c + 1..c + 5).ok_or("a short alias table")?.try_into().expect("4 bytes")) as usize;
        let name = std::str::from_utf8(bytes.get(c + 5..c + 5 + len).ok_or("a short alias table")?).map_err(|_| "an alias's name is not UTF-8")?;
        aliases.push((name.to_owned(), named));
        c += 5 + len;
    }
    let entries = raw
        .into_iter()
        .map(|(kf, flags, alias, classes)| {
            let alias = if alias == crate::SENTINEL { None } else { Some(aliases.get(usize::from(alias)).ok_or("a code's alias is not in the table")?.clone()) };
            Ok(Entry { kind: kf & ((1 << crate::KIND_BITS) - 1), field: kf >> crate::KIND_BITS, flags, alias, classes })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let source_len = u64_at(header::SOURCE_LEN)? as u64;
    Ok((F15Columns { code, size, base, off, tail, widths, from_start, source_len }, CodeTable::new(entries)))
}

// ---- bulk answers ------------------------------------------------------------------------------------------

/// No node: `nearest_all`'s answer for a node with no ancestor in the set.
pub const NONE: u32 = u32::MAX;

/// Every node's nearest strict ancestor among the set's nodes that `keep` keeps (`NONE`: none), by preorder id, into
/// `out` (at least the tape's nodes long; the caller's memory): the lab's `prim::axis_groups` (E12). The codes are
/// matched 64 at a time and only a match's subtree end is read; between two events (a match, the end of an open one)
/// the answer is one value, written as a run. No parent is read, and nothing is kept but `out`: the open matches are
/// a chain through `out` itself (an open match's own answer is the match open around it).
pub fn nearest_all<T: Tape>(t: &T, set: &CodeSet, keep: impl Fn(usize) -> bool, out: &mut [u32]) {
    let n = t.len();
    assert!(out.len() >= n, "nearest_all: {} answers for {n} nodes", out.len());
    // The nodes answered (`0..done`), and the innermost match open there with its subtree's end.
    let (mut done, mut top, mut end) = (0usize, NONE, n);
    let fill = |out: &mut [u32], upto: usize, done: &mut usize, top: &mut u32, end: &mut usize| {
        while *done < upto {
            while *top != NONE && *end <= *done {
                *top = out[*top as usize];
                *end = if *top == NONE { n } else { t.subtree_end(*top as usize) };
            }
            let lim = if *top == NONE { upto } else { (*end).min(upto) };
            out[*done..lim].fill(*top);
            *done = lim;
        }
    };
    t.match_groups(set, |base, mut m| {
        while m != 0 {
            let s = base + m.trailing_zeros() as usize;
            m &= m - 1;
            if keep(s) {
                fill(out, s + 1, &mut done, &mut top, &mut end);
                (top, end) = (s as u32, t.subtree_end(s));
            }
        }
    });
    fill(out, n, &mut done, &mut top, &mut end);
}

// ---- every format ------------------------------------------------------------------------------------------

/// A document's tape in whichever format it is held. A 16z reader keeps the blocks it decoded, so it is one
/// thread's.
#[derive(Clone, Copy)]
pub enum AnyTape<'a> {
    F15(F15<'a>),
    F16(crate::f16::F16<'a>),
    F16Z(crate::f16::F16Z<'a>),
}

/// `$body` with `$t` bound to the tape behind `$tape` as its own type: one dispatch a document, the reads inside
/// compiled for each format.
#[macro_export]
macro_rules! with_tape {
    ($tape:expr, |$t:ident| $body:expr) => {
        match $tape {
            $crate::tape::AnyTape::F15($t) => $body,
            $crate::tape::AnyTape::F16($t) => $body,
            $crate::tape::AnyTape::F16Z($t) => $body,
        }
    };
}

/// A read at a time, dispatched on the format: for a caller that reads a node here and there (a matcher's node).
/// A pass over many nodes dispatches once (`with_tape!`).
impl Tape for AnyTape<'_> {
    #[inline]
    fn len(&self) -> usize {
        with_tape!(self, |t| t.len())
    }
    #[inline]
    fn table(&self) -> &CodeTable {
        with_tape!(self, |t| t.table())
    }
    #[inline]
    fn code(&self, i: usize) -> u32 {
        with_tape!(self, |t| t.code(i))
    }
    #[inline]
    fn subtree_end(&self, i: usize) -> usize {
        with_tape!(self, |t| t.subtree_end(i))
    }
    #[inline]
    fn first_child(&self, i: usize) -> Option<usize> {
        with_tape!(self, |t| t.first_child(i))
    }
    #[inline]
    fn next_sibling(&self, i: usize, parent_end: usize) -> Option<usize> {
        with_tape!(self, |t| t.next_sibling(i, parent_end))
    }
    #[inline]
    fn parent(&self, i: usize) -> Option<usize> {
        with_tape!(self, |t| t.parent(i))
    }
    #[inline]
    fn parent_within(&self, i: usize, a: usize) -> Option<usize> {
        with_tape!(self, |t| t.parent_within(i, a))
    }
    #[inline]
    fn start(&self, i: usize) -> u64 {
        with_tape!(self, |t| t.start(i))
    }
    #[inline]
    fn end(&self, i: usize) -> u64 {
        with_tape!(self, |t| t.end(i))
    }
    #[inline]
    fn start_next(&self, i: usize, prev_start: u64) -> u64 {
        with_tape!(self, |t| t.start_next(i, prev_start))
    }
    #[inline]
    fn end_with_start(&self, i: usize, start: u64) -> u64 {
        with_tape!(self, |t| t.end_with_start(i, start))
    }
    fn match_groups(&self, set: &CodeSet, f: impl FnMut(usize, u64)) {
        with_tape!(self, |t| t.match_groups(set, f))
    }
    fn match_groups_in(&self, set: &CodeSet, block: &dyn Fn(u32) -> bool, f: impl FnMut(usize, u64)) {
        with_tape!(self, |t| t.match_groups_in(set, block, f))
    }
    fn has_kind(&self, block: u32, kind: u16) -> bool {
        with_tape!(self, |t| t.has_kind(block, kind))
    }
    fn path(&self, i: usize, out: &mut Vec<usize>) {
        with_tape!(self, |t| t.path(i, out))
    }
    fn spans(&self, out: &mut Vec<(u64, u64)>) {
        with_tape!(self, |t| t.spans(out))
    }
}

impl AnyTape<'_> {
    /// The format's name, as a run reports it.
    pub fn format(&self) -> &'static str {
        match self {
            AnyTape::F15(_) => "15",
            AnyTape::F16(_) => "16",
            AnyTape::F16Z(_) => "16z",
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{key, preorder_image};

    /// A tree of 41 nodes, three levels, and the image format 15 makes of it.
    /// Each node: its key, start, end and subtree size.
    pub(crate) type Nodes = Vec<(u32, u32, u32, u32)>;

    pub(crate) fn sample() -> (crate::Vector<u8>, Nodes) {
        let src = vec![b' '; 2_000];
        let mut nodes = vec![(key(1, 0, 0), 0, 1_990, 41)];
        for i in 0..8u32 {
            nodes.push((key(2, 1, 0), i * 200, i * 200 + 150, 5));
            for j in 0..4u32 {
                nodes.push((key(3 + j as u16 % 2, 2, 0), i * 200 + j * 30, i * 200 + j * 30 + 20, 1));
            }
        }
        (preorder_image(1, &src, &nodes), nodes)
    }

    /// Every question format 15 answers in place equals the tree it was written from.
    /// `nearest_all` gives each node's nearest kept ancestor in the set, as climbing its parents does.
    #[test]
    fn nearest_all_is_the_climb() {
        let (image, _) = sample();
        let (columns, table) = f15_columns(&image).unwrap();
        let t = F15::new(&image, columns, &table).unwrap();
        for k in 0..table.len() as u32 {
            let set = table.set(|e| e == table.entry(k) || e.kind % 3 == 0);
            let set = set.view();
            let keep = |i: usize| i % 5 != 2;
            let mut out = vec![0; t.len()];
            nearest_all(&t, &set, keep, &mut out);
            for (i, &got) in out.iter().enumerate() {
                let mut q = t.parent(i);
                while let Some(a) = q.filter(|&a| !(set.contains(t.code(a)) && keep(a))) {
                    q = t.parent(a);
                }
                assert_eq!(got, q.map_or(NONE, |a| a as u32), "node {i}, set {k}");
            }
            assert_eq!(out.len(), t.len());
        }
    }

    /// Format 15's parent past its back search (`BACK`): a root of 300 statements of two leaves each and one of 100
    /// leaves, every parent found by the descent, as the tree has it, and every path the climb.
    #[test]
    fn format_15_parent_descends_past_its_back_search() {
        let src = vec![b' '; 10_000];
        let mut nodes = vec![(key(1, 0, 0), 0, 9_000, 0)];
        for i in 0..300u32 {
            nodes.push((key(2, 0, 0), i * 20, i * 20 + 15, 3));
            nodes.push((key(3, 0, 0), i * 20, i * 20 + 5, 1));
            nodes.push((key(3, 0, 0), i * 20 + 6, i * 20 + 15, 1));
        }
        nodes.push((key(2, 0, 0), 6_000, 8_000, 101));
        for i in 0..100u32 {
            nodes.push((key(3, 0, 0), 6_000 + i * 20, 6_000 + i * 20 + 10, 1));
        }
        nodes[0].3 = nodes.len() as u32;
        let image = preorder_image(1, &src, &nodes);
        let (columns, table) = f15_columns(&image).unwrap();
        let t = F15::new(&image, columns, &table).unwrap();
        let mut parents = vec![None; nodes.len()];
        let mut open: Vec<(usize, usize)> = Vec::new();
        for (i, &(.., size)) in nodes.iter().enumerate() {
            while open.last().is_some_and(|&(_, end)| end <= i) {
                open.pop();
            }
            parents[i] = open.last().map(|&(p, _)| p);
            open.push((i, i + size as usize));
        }
        let mut path = Vec::new();
        for (i, &p) in parents.iter().enumerate() {
            assert_eq!(t.parent(i), p, "parent of {i}");
            t.path(i, &mut path);
            assert_eq!(path.last().copied(), p, "path to {i}");
            assert!(path.windows(2).all(|w| parents[w[1]] == Some(w[0])), "path to {i}");
        }
    }

    #[test]
    fn format_15_in_place_answers_as_its_tree() {
        let (image, nodes) = sample();
        let (columns, table) = f15_columns(&image).unwrap();
        let t = F15::new(&image, columns, &table).unwrap();
        assert_eq!(t.len(), nodes.len());
        let mut parents = vec![None; nodes.len()];
        let mut open: Vec<(usize, usize)> = Vec::new();
        for (i, &(k, s, e, size)) in nodes.iter().enumerate() {
            while open.last().is_some_and(|&(_, end)| end <= i) {
                open.pop();
            }
            parents[i] = open.last().map(|&(p, _)| p);
            open.push((i, i + size as usize));
            let entry = table.entry(t.code(i));
            assert_eq!((u32::from(entry.kind) | u32::from(entry.field) << crate::KIND_BITS | u32::from(entry.flags) << 16), k, "code of {i}");
            assert_eq!((t.start(i), t.end(i)), (u64::from(s), u64::from(e)), "span of {i}");
            assert_eq!(t.subtree_end(i), i + size as usize, "subtree of {i}");
        }
        for (i, &p) in parents.iter().enumerate() {
            assert_eq!(t.parent(i), p, "parent of {i}");
            let mut path = Vec::new();
            t.path(i, &mut path);
            let mut climbed = Vec::new();
            let mut q = p;
            while let Some(a) = q {
                climbed.push(a);
                q = parents[a];
            }
            climbed.reverse();
            assert_eq!(path, climbed, "path to {i}");
        }
        assert_eq!(children(&t, 0).count(), 8);
        assert_eq!(children(&t, 1).collect::<Vec<_>>(), vec![2, 3, 4, 5]);
        let set = table.set(|e| e.kind == 3);
        let set = set.view();
        let mut hits = Vec::new();
        t.match_groups(&set, |base, mut m| {
            while m != 0 {
                hits.push(base + m.trailing_zeros() as usize);
                m &= m - 1;
            }
        });
        let want: Vec<usize> = (0..nodes.len()).filter(|&i| nodes[i].0 & 1023 == 3).collect();
        assert_eq!(hits, want);
        let mut spans = Vec::new();
        t.spans(&mut spans);
        assert_eq!(spans, nodes.iter().map(|&(_, s, e, _)| (u64::from(s), u64::from(e))).collect::<Vec<_>>());
    }
}
