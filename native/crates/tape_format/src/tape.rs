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

use crate::{CHECKPOINT_STRIDE, HEADER_BYTES, MAGIC, FORMAT_VERSION, TAIL_FROM_START, header, padded};

/// What a code is: the grammar's kind and field, the flags (`CODE_EXTRA`, `CODE_ERROR`, `CODE_MISSING`), a
/// contextual alias (its displayed name and whether it is named; format 15 numbers them per tape, so a table
/// holds the name) and the supertype classes the node is a member of, ascending.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entry {
    pub kind: u16,
    pub field: u16,
    pub flags: u8,
    pub alias: Option<(String, bool)>,
    pub classes: Vec<u16>,
}

/// The codes a tape's numbers name, by number.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeTable {
    entries: Vec<Entry>,
}

impl CodeTable {
    pub fn new(entries: Vec<Entry>) -> Self {
        CodeTable { entries }
    }
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    /// Code `number`'s entry.
    #[inline(always)]
    pub fn entry(&self, number: u32) -> &Entry {
        &self.entries[number as usize]
    }
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// The codes for which `member` holds, as a set by number.
    pub fn set(&self, member: impl Fn(&Entry) -> bool) -> CodeSet {
        CodeSet { member: self.entries.iter().map(member).collect() }
    }
}

/// A set of codes of one table, by number: what a kind test reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeSet {
    member: Vec<bool>,
}

impl CodeSet {
    #[inline(always)]
    pub fn contains(&self, number: u32) -> bool {
        self.member.get(number as usize).copied().unwrap_or(false)
    }
    pub fn by_number(&self) -> &[bool] {
        &self.member
    }
    pub fn is_empty(&self) -> bool {
        !self.member.iter().any(|&m| m)
    }
}

/// One document's tree, read in place. Ids are preorder numbers.
pub trait Tape {
    /// The nodes.
    fn len(&self) -> usize;
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
}

/// Format 15, read where its columns lie. Its codes are the tape's own table (numbered by first use); its parent
/// is not stored, so it is found back over the nodes before (the format's cost: 0156 §1.7).
#[derive(Clone, Copy)]
pub struct F15<'a> {
    n: usize,
    c: F15Columns<'a>,
    table: &'a CodeTable,
}

#[inline(always)]
fn cell(column: &[u8], width: u8, i: usize) -> u32 {
    if width == 2 {
        u32::from(u16::from_le_bytes([column[2 * i], column[2 * i + 1]]))
    } else {
        u32::from_le_bytes(column[4 * i..4 * i + 4].try_into().expect("4 bytes"))
    }
}

impl<'a> F15<'a> {
    /// The tape of `columns`, whose code ids `table` names. Checked: the widths, the columns' lengths, each code
    /// id and size in range.
    pub fn new(columns: F15Columns<'a>, table: &'a CodeTable) -> Result<Self, String> {
        if columns.widths.iter().any(|&w| w != 2 && w != 4) {
            return Err("a tape column of another width".into());
        }
        let n = columns.code.len() / 2;
        let w = columns.widths.map(usize::from);
        if columns.size.len() < n * w[0] || columns.off.len() < n * w[1] || columns.tail.len() < n * w[2] || columns.base.len() < 4 * n.div_ceil(CHECKPOINT_STRIDE) {
            return Err("a short tape column".into());
        }
        let t = F15 { n, c: columns, table };
        for i in 0..n {
            if u16::from_le_bytes([columns.code[2 * i], columns.code[2 * i + 1]]) as usize >= table.len() {
                return Err("a node's code is not in the table".into());
            }
            if i + t.size(i) > n {
                return Err("a node's size runs past the tape".into());
            }
        }
        Ok(t)
    }
    /// The tape of columns a writer made in this process (the engine's own image, whose shape its restore
    /// checked): not checked again, as it is read once a document a pass. A wrong column panics where it is read.
    pub fn over(columns: F15Columns<'a>, table: &'a CodeTable) -> Self {
        F15 { n: columns.code.len() / 2, c: columns, table }
    }
    #[inline(always)]
    fn size(&self, i: usize) -> usize {
        (cell(self.c.size, self.c.widths[0], i) as usize).max(1)
    }
}

impl Tape for F15<'_> {
    #[inline(always)]
    fn len(&self) -> usize {
        self.n
    }
    #[inline(always)]
    fn table(&self) -> &CodeTable {
        self.table
    }
    #[inline(always)]
    fn code(&self, i: usize) -> u32 {
        u32::from(u16::from_le_bytes([self.c.code[2 * i], self.c.code[2 * i + 1]]))
    }
    #[inline(always)]
    fn subtree_end(&self, i: usize) -> usize {
        i + self.size(i)
    }
    /// Not stored: the nearest node before `i` whose subtree holds it, searched back over at most `BACK` nodes,
    /// else found by descending from the root over the siblings' subtree ends (`Tape::path`): a search is never
    /// longer than the path's siblings.
    fn parent(&self, i: usize) -> Option<usize> {
        const BACK: usize = 64;
        if let Some(p) = (i.saturating_sub(BACK)..i).rev().find(|&j| j + self.size(j) > i) {
            return Some(p);
        }
        if i == 0 {
            return None;
        }
        let mut at = 0;
        loop {
            let mut c = at + 1;
            while c + self.size(c) <= i {
                c += self.size(c);
            }
            if c == i {
                return Some(at);
            }
            at = c;
        }
    }
    #[inline(always)]
    fn start(&self, i: usize) -> u64 {
        u64::from(cell(self.c.base, 4, i / CHECKPOINT_STRIDE).wrapping_add(cell(self.c.off, self.c.widths[1], i)))
    }
    #[inline(always)]
    fn end(&self, i: usize) -> u64 {
        let from = if self.c.from_start { self.start(i) } else { self.start(i + self.size(i) - 1) };
        u64::from((from as u32).wrapping_add(cell(self.c.tail, self.c.widths[2], i)))
    }
}

/// A stored format 15 image's columns and its code table (`MAGIC`, `FORMAT_VERSION`), read where they lie.
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
    Ok((F15Columns { code, size, base, off, tail, widths, from_start }, CodeTable::new(entries)))
}

// ---- bulk answers ------------------------------------------------------------------------------------------

/// No node: `nearest_all`'s answer for a node with no ancestor in the set.
pub const NONE: u32 = u32::MAX;

/// Every node's nearest strict ancestor among the set's nodes that `keep` keeps (`NONE`: none), by preorder id: the
/// lab's `prim::axis_groups` (E12). The codes are matched 64 at a time and only a match's subtree end is read; between
/// two events (a match, the end of an open one) the answer is one value, written as a run. No parent is read.
pub fn nearest_all<T: Tape>(t: &T, set: &CodeSet, keep: impl Fn(usize) -> bool, out: &mut Vec<u32>) {
    let n = t.len();
    out.clear();
    out.reserve(n);
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(32);
    // Every node's answer before `upto`.
    let fill = |out: &mut Vec<u32>, stack: &mut Vec<(u32, usize)>, upto: usize| {
        while out.len() < upto {
            while stack.last().is_some_and(|&(_, end)| end <= out.len()) {
                stack.pop();
            }
            let (v, lim) = match stack.last() {
                Some(&(a, end)) => (a, end.min(upto)),
                None => (NONE, upto),
            };
            out.resize(lim, v);
        }
    };
    t.match_groups(set, |base, mut m| {
        while m != 0 {
            let s = base + m.trailing_zeros() as usize;
            m &= m - 1;
            if keep(s) {
                fill(out, &mut stack, s + 1);
                stack.push((s as u32, t.subtree_end(s)));
            }
        }
    });
    fill(out, &mut stack, n);
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

    pub(crate) fn sample() -> (Vec<u8>, Nodes) {
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
        let t = F15::new(columns, &table).unwrap();
        for k in 0..table.len() as u32 {
            let set = table.set(|e| *e == *table.entry(k) || e.kind % 3 == 0);
            let keep = |i: usize| i % 5 != 2;
            let mut out = Vec::new();
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
        let t = F15::new(columns, &table).unwrap();
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
        let t = F15::new(columns, &table).unwrap();
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
