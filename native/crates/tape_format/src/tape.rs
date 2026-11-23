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
//! The formats behind it (`AnyTape`): format 16 over its arrays (`crate::f16::F16`), and 16z over its blocks
//! (`crate::f16::F16Z`). A caller dispatches once a document (`with_tape!`), so the per-node reads are compiled
//! for each format.

/// What a code is: the grammar's kind and field, the flags (`CODE_EXTRA`, `CODE_ERROR`, `CODE_MISSING`), a
/// contextual alias (its displayed name and whether it is named, held by name) and the supertype classes the node
/// is a member of, ascending.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Entry {
    pub kind: u16,
    pub field: u16,
    pub flags: u8,
    pub alias: Option<(String, bool)>,
    pub classes: Vec<u16>,
}

/// The codes a tape's numbers name, by number, and the table's index of them by kind and by class (made once,
/// with the table, on first use): a kind's codes without a scan of the table, which on format 16 is the language's
/// every code (`f16::Lang`), not the document's few.
#[derive(Clone, Debug, Default)]
pub struct CodeTable {
    entries: Vec<Entry>,
    index: std::sync::OnceLock<CodeIndex>,
}

impl PartialEq for CodeTable {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries
    }
}
impl Eq for CodeTable {}

/// A table's codes by kind (those with no `CODE_ERROR`), by class, and those with `CODE_ERROR` and with
/// `CODE_MISSING`, each as the set a kind test reads: made once per table, so a test of a document borrows it
/// (on format 16 every document of a language shares the table).
#[derive(Clone, Debug, Default)]
struct CodeIndex {
    /// By kind (a u16 symbol, so a vector).
    kinds: Vec<CodeSet>,
    classes: Vec<CodeSet>,
    errors: CodeSet,
    missing: CodeSet,
    /// `errors` and `missing`.
    faults: CodeSet,
    empty: CodeSet,
}

impl CodeTable {
    pub fn new(entries: Vec<Entry>) -> Self {
        CodeTable { entries, index: std::sync::OnceLock::new() }
    }
    fn index(&self) -> &CodeIndex {
        self.index.get_or_init(|| {
            let mut kinds: Vec<Vec<u32>> = Vec::new();
            let mut classes: Vec<Vec<u32>> = Vec::new();
            let slot = |v: &mut Vec<Vec<u32>>, k: u16, n: u32| {
                if v.len() <= k as usize {
                    v.resize_with(k as usize + 1, Vec::new);
                }
                v[k as usize].push(n);
            };
            let (mut errors, mut missing) = (Vec::new(), Vec::new());
            for (n, e) in self.entries.iter().enumerate() {
                let n = n as u32;
                if e.flags & crate::CODE_MISSING != 0 {
                    missing.push(n);
                }
                if e.flags & crate::CODE_ERROR != 0 {
                    errors.push(n);
                } else {
                    slot(&mut kinds, e.kind, n);
                }
                for &c in &e.classes {
                    slot(&mut classes, c, n);
                }
            }
            let faults = CodeSet::of_numbers(errors.iter().chain(&missing).copied());
            CodeIndex {
                kinds: kinds.into_iter().map(CodeSet::of_numbers).collect(),
                classes: classes.into_iter().map(CodeSet::of_numbers).collect(),
                errors: CodeSet::of_numbers(errors),
                missing: CodeSet::of_numbers(missing),
                faults,
                empty: CodeSet::default(),
            }
        })
    }
    /// The codes of kind `kind` with no `CODE_ERROR`.
    pub fn of_kind(&self, kind: u16) -> &CodeSet {
        let x = self.index();
        x.kinds.get(kind as usize).unwrap_or(&x.empty)
    }
    /// The codes whose classes hold `class`.
    pub fn of_class(&self, class: u16) -> &CodeSet {
        let x = self.index();
        x.classes.get(class as usize).unwrap_or(&x.empty)
    }
    /// The codes with `CODE_ERROR`.
    pub fn errors(&self) -> &CodeSet {
        &self.index().errors
    }
    /// The codes with `CODE_MISSING`.
    pub fn missing(&self) -> &CodeSet {
        &self.index().missing
    }
    /// The codes with `CODE_ERROR` or `CODE_MISSING`.
    pub fn faults(&self) -> &CodeSet {
        &self.index().faults
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
        CodeSet { repr: Members::Dense(self.entries.iter().map(member).collect()) }
    }
}

/// A set of codes of one table, by number: what a kind test reads. Either a flag per code of the table (a set
/// made by a test of every entry), or the few numbers it holds (a kind's or a class's, from the table's index).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeSet {
    repr: Members,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Members {
    Dense(Vec<bool>),
    Few(Box<[u32]>),
}

impl Default for CodeSet {
    fn default() -> Self {
        CodeSet { repr: Members::Few(Box::default()) }
    }
}

impl CodeSet {
    /// The set of `numbers` (any order, repeats allowed).
    pub fn of_numbers(numbers: impl IntoIterator<Item = u32>) -> CodeSet {
        let mut v: Vec<u32> = numbers.into_iter().collect();
        v.sort_unstable();
        v.dedup();
        CodeSet { repr: Members::Few(v.into_boxed_slice()) }
    }
    #[inline(always)]
    pub fn contains(&self, number: u32) -> bool {
        match &self.repr {
            Members::Dense(member) => member.get(number as usize).copied().unwrap_or(false),
            Members::Few(few) if few.len() <= 8 => few.contains(&number),
            Members::Few(few) => few.binary_search(&number).is_ok(),
        }
    }
    /// The numbers of a set made from numbers, ascending; a set of a flag per code has none to give.
    pub fn numbers(&self) -> Option<&[u32]> {
        match &self.repr {
            Members::Dense(_) => None,
            Members::Few(few) => Some(few),
        }
    }
    /// Marks in `out` each number of the set below `cut` (at most 256), clearing the rest: a set of a few numbers
    /// costs its members, not a test of every number below the cut.
    pub fn mark_below(&self, cut: usize, out: &mut [bool; 256]) {
        let cut = cut.min(256);
        *out = [false; 256];
        match &self.repr {
            Members::Dense(member) => out[..cut.min(member.len())].copy_from_slice(&member[..cut.min(member.len())]),
            Members::Few(few) => few.iter().take_while(|&&n| (n as usize) < cut).for_each(|&n| out[n as usize] = true),
        }
    }
    /// Whether the set holds a number `from` or past it.
    pub fn any_from(&self, from: usize) -> bool {
        match &self.repr {
            Members::Dense(member) => member.iter().skip(from).any(|&m| m),
            Members::Few(few) => few.last().is_some_and(|&n| n as usize >= from),
        }
    }
    pub fn is_empty(&self) -> bool {
        match &self.repr {
            Members::Dense(member) => !member.iter().any(|&m| m),
            Members::Few(few) => few.is_empty(),
        }
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
    F16(crate::f16::F16<'a>),
    F16Z(crate::f16::F16Z<'a>),
}

/// `$body` with `$t` bound to the tape behind `$tape` as its own type: one dispatch a document, the reads inside
/// compiled for each format.
#[macro_export]
macro_rules! with_tape {
    ($tape:expr, |$t:ident| $body:expr) => {
        match $tape {
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
            AnyTape::F16(_) => "16",
            AnyTape::F16Z(_) => "16z",
        }
    }
}

/// A tape as plain text, the oracles' form (the engine's `python_tape_plain`, each lean parser's `tape_text`): a node
/// a line, indented by depth, its kind's name, its field's (`name:`), its flags, its classes' names in its code's
/// order (`{a,b}`), and its span. Names are the language's (`f16::Names`); `?` for an id it does not name.
pub fn plain_text<T: Tape>(t: &T, names: &crate::f16::Names) -> String {
    let mut out = String::new();
    let mut open: Vec<usize> = Vec::new();
    let entries = t.table().entries();
    for i in 0..t.len() {
        while open.last().is_some_and(|&end| end <= i) {
            open.pop();
        }
        let e = &entries[t.code(i) as usize];
        let classes: Vec<&str> = e.classes.iter().map(|&c| names.kind_name(c).unwrap_or("?")).collect();
        out.push_str(&format!(
            "{}{}{}{}{} [{}..{}]\n",
            "  ".repeat(open.len()),
            names.kind_name(e.kind).unwrap_or("?"),
            if e.field == 0 { String::new() } else { format!(" {}:", names.field_name(e.field).unwrap_or("?")) },
            if e.flags == 0 { String::new() } else { format!(" flags={}", e.flags) },
            if classes.is_empty() { String::new() } else { format!(" {{{}}}", classes.join(",")) },
            t.start(i),
            t.end(i),
        ));
        open.push(t.subtree_end(i));
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A tree as a list of nodes in preorder, each its code (a number in its table), start, end and subtree size:
    /// the tree a tape is written from, and what the tape's answers are held to.
    pub(crate) struct Plain<'a>(pub(crate) &'a [(u32, u64, u64, usize)], pub(crate) &'a CodeTable);
    impl Tape for Plain<'_> {
        fn len(&self) -> usize {
            self.0.len()
        }
        fn table(&self) -> &CodeTable {
            self.1
        }
        fn code(&self, i: usize) -> u32 {
            self.0[i].0
        }
        fn subtree_end(&self, i: usize) -> usize {
            i + self.0[i].3
        }
        fn parent(&self, i: usize) -> Option<usize> {
            (0..i).rev().find(|&j| self.subtree_end(j) > i)
        }
        fn start(&self, i: usize) -> u64 {
            self.0[i].1
        }
        fn end(&self, i: usize) -> u64 {
            self.0[i].2
        }
    }

    /// A tree of 41 nodes, three levels, over a source of 2,000 bytes, and its code table: a root (code 0), eight
    /// statements under field 1 (code 1), each with four leaves under field 2 of two kinds (codes 2 and 3).
    pub(crate) fn sample() -> (Vec<(u32, u64, u64, usize)>, CodeTable) {
        let entry = |kind: u16, field: u16| Entry { kind, field, flags: 0, alias: None, classes: Vec::new() };
        let table = CodeTable::new(vec![entry(1, 0), entry(2, 1), entry(3, 2), entry(4, 2)]);
        let mut nodes = vec![(0, 0, 1_990, 41)];
        for i in 0..8u64 {
            nodes.push((1, i * 200, i * 200 + 150, 5));
            for j in 0..4u64 {
                nodes.push((2 + j as u32 % 2, i * 200 + j * 30, i * 200 + j * 30 + 20, 1));
            }
        }
        (nodes, table)
    }

    /// `nearest_all` gives each node's nearest kept ancestor in the set, as climbing its parents does.
    #[test]
    fn nearest_all_is_the_climb() {
        let (nodes, table) = sample();
        let t = Plain(&nodes, &table);
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

    /// The answers `Tape` gives of its own (a path, the children, a code set's matches, the spans in order) are the
    /// tree's.
    #[test]
    fn a_tapes_own_answers_are_its_trees() {
        let (nodes, table) = sample();
        let t = Plain(&nodes, &table);
        for i in 0..t.len() {
            let mut path = Vec::new();
            t.path(i, &mut path);
            let mut climbed = Vec::new();
            let mut q = t.parent(i);
            while let Some(a) = q {
                climbed.push(a);
                q = t.parent(a);
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
        let want: Vec<usize> = (0..nodes.len()).filter(|&i| nodes[i].0 == 2).collect();
        assert_eq!(hits, want);
        let mut spans = Vec::new();
        t.spans(&mut spans);
        assert_eq!(spans, nodes.iter().map(|&(_, s, e, _)| (s, e)).collect::<Vec<_>>());
    }
}
