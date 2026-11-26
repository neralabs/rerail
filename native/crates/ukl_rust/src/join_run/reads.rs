//! The join program's reads of a batch's views, written directly in Rust (decisions 0534, 0536): a view's row is a
//! read of the tape where it lies and of its projection's numbering, never a relation made of it (decision 0353). The
//! views' reads in CLIF (`rerail_clif::view_reads`) are the JIT's and the kernels'; these take the same steps over the
//! same bytes: the tape's reads (`rerail_tape_format::reads`: format 16, 15, and 16z through its batch's shared
//! cache), the numbering's (`rerail_clif::ids::Numbering`) and the lines' (format 16's line index, at its base). A test
//! holds each to its CLIF over real batches' directories (`tests_lang`: `the_join_programs_view_reads_are_the_jits`).
//!
//! A document (`Doc`) is found once (`Doc::of`, its directory's row) and every read of it takes it: its entry, its
//! numbering, its tape and view and its lines, each found where it lies.
//!
//! # Safety
//!
//! Every read takes a document of a directory in its batch's block (`Doc::of`'s caller's), and rows and nodes within
//! what it holds, as its CLIF does.
#![allow(unsafe_op_in_unsafe_fn, clippy::missing_safety_doc)]

use rerail_reads::ids::{INDEX_BITS, Numbering, Run, SLOT_BITS, SPAN_SLOTS};
use rerail_reads::view::{F16Z_FORMAT, TapeView};
use rerail_reads::views::{NO_ROW, PLACED_AT, ViewDir, ViewDoc, ViewHome, ViewRow};
use rerail_reads::tape::{InPlace, f15, f16, f16z};

/// A tape where it lies, and its view (the host's: a 16z view's cache at the host's address).
#[derive(Clone, Copy)]
pub struct Tape {
    pub t: InPlace,
    pub v: TapeView,
}

macro_rules! by_format {
    ($self:ident, $name:ident($($a:expr),*)) => {
        match $self.v.format {
            16 => f16::$name($self.t, &$self.v, $($a),*),
            F16Z_FORMAT => f16z::$name($self.t, &$self.v, $($a),*),
            _ => f15::$name($self.t, &$self.v, $($a),*),
        }
    };
}

impl Tape {
    #[inline]
    pub fn size(&self, i: u32) -> u32 {
        by_format!(self, size(i))
    }
    #[inline]
    pub fn number(&self, i: u32) -> u32 {
        by_format!(self, number(i))
    }
    #[inline]
    pub fn start(&self, i: u32) -> u32 {
        by_format!(self, start(i))
    }
    #[inline]
    pub fn end(&self, i: u32) -> u32 {
        by_format!(self, end(i))
    }
    #[inline]
    pub fn parent(&self, i: u32) -> u32 {
        by_format!(self, parent(i))
    }
    /// One past the node's subtree: `i + size`, no further than the tape's nodes and always past `i` (the CLIF's
    /// `hop`: a walk by sizes ends whatever the tape holds, decision 0463).
    #[inline]
    pub fn end_of(&self, i: u32) -> u32 {
        let past = i.wrapping_add(1);
        i.wrapping_add(self.size(i)).max(past).min(self.v.nodes).max(past)
    }
    /// The node after the node's subtree while it is before `end`, else `NO_ROW`.
    #[inline]
    pub fn next_in(&self, i: u32, end: u32) -> u32 {
        let after = self.end_of(i);
        if after < end { after } else { NO_ROW }
    }
    /// The node's first child, or `NO_ROW`.
    #[inline]
    pub fn first_child(&self, i: u32) -> u32 {
        if self.size(i) > 1 { i.wrapping_add(1) } else { NO_ROW }
    }
}

/// A document's line index, at its base (the lines' family's reads: a guest's are its host's).
#[derive(Clone, Copy)]
pub struct Lines(pub InPlace);

/// The line index's view: format 16's line reads at offset 0 of the lines' base.
const LINES_VIEW: TapeView = TapeView { lines: 0, ..zero_view() };

const fn zero_view() -> TapeView {
    // SAFETY: a view is plain words, all zero a valid one.
    unsafe { std::mem::zeroed() }
}

impl Lines {
    /// The 1-based line holding byte `x` (`lines_line`).
    #[inline]
    pub fn line(&self, x: u32) -> u32 {
        f16::line(self.0, &LINES_VIEW, x).wrapping_add(1)
    }
    /// The document's lines, one more than its line ends (`lines_count`).
    #[inline]
    pub fn count(&self) -> u32 {
        // SAFETY: the index's first field, its line ends' count.
        unsafe { self.0.base().cast::<u32>().read_unaligned() }.wrapping_add(1)
    }
    /// Line `k`'s first byte, from 0 (`lines_start`).
    #[inline]
    pub fn start(&self, k: u32) -> u32 {
        f16::line_start(self.0, &LINES_VIEW, k)
    }
}

/// The directory's block: its address less its offset in it.
#[inline]
pub unsafe fn block(d: *const ViewDir) -> *const u8 {
    d.cast::<u8>().wrapping_sub((*d).at as usize)
}

/// Home `h`'s bytes at `at`, by its host address: this executor is the host's (`ViewHome::gpu` is a device's).
#[inline]
unsafe fn home(d: *const ViewDir, h: u32, at: u64) -> *const u8 {
    let homes = block(d).add((*d).homes as usize).cast::<ViewHome>();
    ((*homes.add(h as usize)).host as *const u8).wrapping_add(at as usize)
}

/// Run `r`'s items, where it points.
#[inline(always)]
unsafe fn items<T>(r: &Run) -> *const T {
    (r as *const Run).cast::<u8>().wrapping_offset(r.off as isize).cast::<T>()
}

/// A document of a directory as its reads take it: its entry, its numbering, its tape and its lines.
#[derive(Clone, Copy)]
pub struct Doc<'a> {
    pub e: &'a ViewDoc,
    pub n: &'a Numbering,
    pub tape: Tape,
    pub lines: Lines,
}

/// A view row found (`Doc::at`): its document, its view (`view_reads::VIEWS`), its row in the document and the tape
/// node its columns read (0 where they read none, or where a column finds its own: the `Loc` views').
#[derive(Clone, Copy)]
pub struct At<'a> {
    d: Doc<'a>,
    k: u32,
    r: u32,
    x: u32,
}

impl At<'_> {
    /// Its column `col`.
    #[inline]
    pub unsafe fn col(&self, col: u32) -> u64 {
        let (d, r, x) = (&self.d, self.r, self.x);
        match self.k {
            0 => d.node(r, x, col),
            1 => d.child(r, x, col),
            2 => d.field(x, col),
            3 => d.core(r, x, col),
            4 => d.line(r, x, col),
            5 => d.loc(Place::Line, r, col),
            6 => d.loc(Place::EndLine, r, col),
            7 => d.loc(Place::Col, r, col),
            8 => d.loc(Place::Docof, r, col),
            9 => d.tape(r, x, col),
            10 => d.depth(r, x, col),
            _ => d.order(r, col),
        }
    }
}

/// Which place a `Loc` view's second column holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Line,
    EndLine,
    Col,
    Docof,
}

impl<'a> Doc<'a> {
    /// Directory `d`'s document `m`: its entry, numbering, tape and lines where they lie, and its view as this
    /// executor reads it: the row's own, but for a 16z view of a directory whose cache is a home of its own, the
    /// cache's host address less the tape's (`rerail_clif::view::Reader::new`'s).
    #[inline]
    pub unsafe fn of(d: *const ViewDir, m: u32) -> Doc<'a> {
        let row: &ViewRow = &*block(d).add((*d).rows as usize).cast::<ViewRow>().add(m as usize);
        let e = &*block(d).add(row.entry as usize).cast::<ViewDoc>();
        let t = home(d, row.homes[0], row.tape);
        let mut v = row.view;
        if (*d).cache != 0 && v.format == F16Z_FORMAT {
            let rel = (home(d, (*d).cache - 1, 0) as i64).wrapping_sub(t as i64);
            (v.side[0], v.side[1]) = (rel as u32, (rel >> 32) as u32);
        }
        Doc { e, n: &*items::<Numbering>(&e.numbering), tape: Tape { t: InPlace::new(t), v }, lines: Lines(InPlace::new(home(d, row.homes[1], row.lines))) }
    }

    /// Item `i` of the entry's run `r` (u32s).
    #[inline(always)]
    unsafe fn item(r: &Run, i: u32) -> u64 {
        u64::from(*items::<u32>(r).add(i as usize))
    }

    #[inline]
    fn local(&self) -> bool {
        self.e.preorder != 0
    }

    /// Numbered node `l`'s tape node.
    #[inline]
    pub unsafe fn tape_node(&self, l: u32) -> u32 {
        self.n.select(0, l)
    }

    /// Numbered node `l`'s id: the document's key, then its low bits (`l` itself where ids are local ids).
    #[inline]
    pub unsafe fn node_id(&self, l: u32) -> u64 {
        let low = if self.local() { l } else { self.n.low_of(0, l) };
        u64::from(self.e.key) << 32 | u64::from(low)
    }

    /// Numbered node `l`'s parent's id, its tape node `x`: its tape parent's, or the root's where that is not
    /// numbered (a narrowed projection's anchor); -1 for the root.
    #[inline]
    pub unsafe fn parent_id(&self, l: u32, x: u32) -> u64 {
        if l == 0 {
            return u64::MAX;
        }
        let q = self.tape.parent(x);
        self.node_id(if self.n.is(0, q) { self.n.rank(0, q) } else { 0 })
    }

    /// `node`'s row of numbered node `l`, its tape node `x`: the document (its file), the id, the kind atom, the
    /// start, the end.
    unsafe fn node(&self, l: u32, x: u32, col: u32) -> u64 {
        match col {
            0 => u64::from(self.e.file),
            1 => self.node_id(l),
            2 => Self::item(&self.e.kinds, self.tape.number(x)),
            3 => u64::from(self.tape.start(x)),
            _ => u64::from(self.tape.end(x)),
        }
    }

    /// `child` row `i`, of numbered node `i + 1`, its tape node `x`: the parent's id, the child's id, its place among
    /// the parent's children.
    unsafe fn child(&self, i: u32, x: u32, col: u32) -> u64 {
        let l = i.wrapping_add(1);
        match col {
            0 => self.parent_id(l, x),
            1 => self.node_id(l),
            _ => Self::item(&self.e.places, l),
        }
    }

    /// `field` row `i`, of the document's `i`-th node under a field, its tape node `x`: the parent's id, the field
    /// atom, the child's id.
    unsafe fn field(&self, x: u32, col: u32) -> u64 {
        match col {
            0 => self.parent_id(self.n.rank(0, x), x),
            1 => Self::item(&self.e.fields, self.tape.number(x)),
            _ => self.node_id(self.n.rank(0, x)),
        }
    }

    /// `core_node`'s row of numbered node `l`, its tape node `x`: the document, its tape node, its id.
    unsafe fn core(&self, l: u32, x: u32, col: u32) -> u64 {
        match col {
            0 => u64::from(self.e.key),
            1 => u64::from(x),
            _ => self.node_id(l),
        }
    }

    /// `line`'s row of numbered node `l`, its tape node `x`: its id, the line its start is on.
    unsafe fn line(&self, l: u32, x: u32, col: u32) -> u64 {
        match col {
            0 => self.node_id(l),
            _ => u64::from(self.lines.line(self.tape.start(x))),
        }
    }

    /// `Loc` position `l`'s id: a node's or token's by its numbering, a span's in the top slots; `l` itself where ids
    /// are local ids.
    unsafe fn loc_id(&self, l: u32) -> u64 {
        let e = self.e;
        let low = if self.local() {
            l
        } else if l < e.nodes {
            self.n.low_of(0, l)
        } else if l - e.nodes < e.tokens {
            self.n.low_of(1, l - e.nodes)
        } else {
            let j = l - e.nodes - e.tokens;
            let hi = (j >> INDEX_BITS).min(SPAN_SLOTS - 1);
            ((1u32 << SLOT_BITS) - 1 - hi) << INDEX_BITS | j & ((1 << INDEX_BITS) - 1)
        };
        u64::from(e.key) << 32 | u64::from(low)
    }

    /// `Loc` position `l`'s byte span, `start << 32 | end`: a node's or token's off the tape, the whole text, a
    /// line's (its start for both ends), or a node's end point.
    unsafe fn loc_span(&self, l: u32) -> u64 {
        let e = self.e;
        let pair = |s: u32, t: u32| u64::from(s) << 32 | u64::from(t);
        let of_tape = |x: u32| pair(self.tape.start(x), self.tape.end(x));
        if l < e.nodes {
            return of_tape(self.tape_node(l));
        }
        let i = l - e.nodes;
        if i < e.tokens {
            return of_tape(self.n.select(1, i));
        }
        let j = i - e.tokens;
        // The end points follow the whole text and the lines where the text is the document's own.
        let ends_from = if e.own_text != 0 { self.lines.count().wrapping_add(1) } else { 0 };
        if j >= ends_from {
            let end = self.tape.end(self.tape_node(j - ends_from));
            return pair(end, end);
        }
        if j == 0 {
            return pair(0, e.source_len);
        }
        let s = self.lines.start(j - 1);
        pair(s, s)
    }

    /// A guest's span where it stands in its host: copied, each end moved by `delta` and kept within `within`; at
    /// `within`. A file's own, as it is.
    fn placed(&self, span: u64) -> u64 {
        let e = self.e;
        if e.placement == 0 {
            return span;
        }
        let (lo, hi) = (i64::from(e.within_lo), i64::from(e.within_hi));
        if e.placement == PLACED_AT {
            return (lo << 32 | hi) as u64;
        }
        let clamp = |x: i64| x.wrapping_add(e.delta).max(lo).min(hi);
        let (s, t) = (clamp((span >> 32) as i64), clamp((span & 0xffff_ffff) as i64));
        (s << 32 | t) as u64
    }

    unsafe fn loc(&self, place: Place, l: u32, col: u32) -> u64 {
        if col == 0 {
            return self.loc_id(l);
        }
        if place == Place::Docof {
            return u64::from(self.e.file);
        }
        let span = self.placed(self.loc_span(l));
        let (s, t) = ((span >> 32) as u32, span as u32);
        let line = self.lines.line(s);
        u64::from(match place {
            Place::Line => line,
            // The line holding its end offset: a span ending with a terminator ends on the line after it.
            Place::EndLine => self.lines.line(t),
            _ => s.wrapping_sub(self.lines.start(line.wrapping_sub(1))),
        })
    }

    /// Numbered node `l`'s depth (its tape node `x`), the root's 0: its numbered strict tape ancestors.
    unsafe fn depth_of(&self, x: u32) -> u64 {
        let (mut p, mut d) = (self.tape.parent(x), 0u32);
        while p != NO_ROW {
            d = d.wrapping_add(u32::from(self.n.is(0, p)));
            p = self.tape.parent(p);
        }
        u64::from(d)
    }

    /// `tape`'s row of numbered node `l`, its tape node `x`: its id, its parent's id (-1 for the root), the
    /// numbered nodes under it, its depth.
    unsafe fn tape(&self, l: u32, x: u32, col: u32) -> u64 {
        match col {
            0 => self.node_id(l),
            1 => self.parent_id(l, x),
            2 => u64::from(self.n.rank(0, self.tape.end_of(x)).wrapping_sub(l).wrapping_sub(1)),
            _ => self.depth_of(x),
        }
    }

    /// `ast_depth`'s row of numbered node `l`, its tape node `x`: its id, its depth.
    unsafe fn depth(&self, l: u32, x: u32, col: u32) -> u64 {
        if col == 0 { self.node_id(l) } else { self.depth_of(x) }
    }

    /// `node_order`'s row of numbered node `l`: its id, its order key under the document's key.
    unsafe fn order(&self, l: u32, col: u32) -> u64 {
        if col == 0 {
            return self.node_id(l);
        }
        let low = if self.local() { l } else { self.n.order_of(l) };
        u64::from(self.e.key) << 32 | u64::from(low)
    }

    /// View `k`'s (`view_reads::VIEWS`) row `r`, column `col`.
    #[inline]
    pub unsafe fn view(&self, k: u32, r: u32, col: u32) -> u64 {
        self.at(k, r).col(col)
    }

    /// View `k`'s (`view_reads::VIEWS`) row `r`, found once: its tape node, where its columns read one, selected
    /// here, so a row's columns read it once.
    #[inline]
    pub unsafe fn at(self, k: u32, r: u32) -> At<'a> {
        let x = match k {
            0 | 3 | 4 | 9 | 10 => self.tape_node(r),
            1 => self.tape_node(r.wrapping_add(1)),
            2 => self.n.select(2, r),
            _ => 0,
        };
        At { d: self, k, r, x }
    }

    /// The `child` row of the first numbered node from tape node `c` on, `c` and each sibling after it within
    /// `end`, or `NO_ROW`.
    unsafe fn numbered_from(&self, mut c: u32, end: u32) -> u32 {
        while c != NO_ROW {
            if self.n.is(0, c) {
                return self.n.rank(0, c).wrapping_sub(1);
            }
            c = self.tape.next_in(c, end);
        }
        NO_ROW
    }

    /// The `child` row of numbered node `l`'s first child, or `NO_ROW` (`view_child_first`).
    pub unsafe fn child_first(&self, l: u32) -> u32 {
        let x = self.tape_node(l);
        self.numbered_from(self.tape.first_child(x), self.tape.end_of(x))
    }

    /// The `child` row of the next child of the parent of `child` row `i`, or `NO_ROW` (`view_child_next`).
    pub unsafe fn child_next(&self, i: u32) -> u32 {
        let x = self.tape_node(i.wrapping_add(1));
        let p = self.tape.parent(x);
        if p == NO_ROW {
            return NO_ROW;
        }
        let end = self.tape.end_of(p);
        self.numbered_from(self.tape.next_in(x, end), end)
    }

    /// Numbered node `l`'s nearest numbered strict tape ancestor, or `NO_ROW` (`view_up`).
    pub unsafe fn up(&self, l: u32) -> u32 {
        let mut p = self.tape.parent(self.tape_node(l));
        while p != NO_ROW {
            if self.n.is(0, p) {
                return self.n.rank(0, p);
            }
            p = self.tape.parent(p);
        }
        NO_ROW
    }

    /// The number past numbered node `l`'s numbered descendants (`view_end`).
    pub unsafe fn end(&self, l: u32) -> u32 {
        self.n.rank(0, self.tape.end_of(self.tape_node(l)))
    }

    /// The numbered node whose id's low 32 bits are `low`, or `NO_ROW` (`view_number`).
    pub unsafe fn number(&self, low: u32) -> u32 {
        let l = if self.local() {
            low
        } else {
            match self.n.number_of(low) {
                Some((0, l)) => l,
                _ => NO_ROW,
            }
        };
        if l < self.e.nodes { l } else { NO_ROW }
    }
}

/// An atoms directory's layout (`rerail_blocks::atoms::AtomsDirRoot`, `DirEntry`; a test holds it to it): its
/// runs' count and entries, and each entry's first id, end, the id its strings start at, their ends and bytes.
pub(crate) mod atoms_at {
    pub const DIR_N: usize = 0;
    pub const DIR_ENTRIES: usize = 8;
    pub const E_FIRST: usize = 0;
    pub const E_END: usize = 4;
    pub const E_FROM: usize = 8;
    pub const E_ENDS: usize = 16;
    pub const E_STRINGS: usize = 32;
    pub const E_SIZE: usize = 48;
}

/// The address a span at `p + o` names (its own place plus its offset).
#[inline(always)]
unsafe fn follow(p: *const u8, o: usize) -> *const u8 {
    p.wrapping_add(o).wrapping_add(p.add(o).cast::<i64>().read_unaligned() as usize)
}

/// The entry of directory `dir`'s run holding atom `id`, if one does: the last whose first id is at most `id`, if
/// `id` is below its end.
#[inline]
unsafe fn atoms_entry(dir: *const u8, id: u32) -> Option<*const u8> {
    use atoms_at::*;
    let n = dir.add(DIR_N).cast::<u64>().read_unaligned() as u32;
    let entries = follow(dir, DIR_ENTRIES);
    let entry = |t: u32| entries.wrapping_add(t as usize * E_SIZE);
    let (mut lo, mut hi) = (0u32, n);
    while lo < hi {
        let m = lo + (hi - lo) / 2;
        if entry(m).add(E_FIRST).cast::<u32>().read_unaligned() <= id { lo = m + 1 } else { hi = m }
    }
    let e = entry(lo.max(1) - 1);
    (lo != 0 && id < e.add(E_END).cast::<u32>().read_unaligned()).then_some(e)
}

/// The offsets in its run's strings where atom `id` of entry `e` begins and ends.
#[inline]
unsafe fn atoms_bounds(e: *const u8, id: u32) -> (u64, u64) {
    use atoms_at::*;
    let i = id.wrapping_sub(e.add(E_FROM).cast::<u32>().read_unaligned()) as usize;
    let ends = follow(e, E_ENDS).cast::<u64>();
    (if i == 0 { 0 } else { ends.add(i - 1).read_unaligned() }, ends.add(i).read_unaligned())
}

/// The length of atom `id`'s string in directory `dir`, `u32::MAX` for an id no run holds (0 for id 0): the CLIF's
/// `atoms_len` (`rerail_blocks::atoms::str_in`'s).
#[inline]
pub unsafe fn atoms_len(dir: *const u8, id: u32) -> u32 {
    if id == 0 {
        return 0;
    }
    match atoms_entry(dir, id) {
        Some(e) => {
            let (start, end) = atoms_bounds(e, id);
            end.wrapping_sub(start) as u32
        }
        None => u32::MAX,
    }
}

/// The address of atom `id`'s string's first byte in directory `dir` (the directory's own for an id no run holds, or
/// for id 0's empty string): the CLIF's `atoms_at`.
#[inline]
pub unsafe fn atoms_str(dir: *const u8, id: u32) -> *const u8 {
    if id == 0 {
        return dir;
    }
    match atoms_entry(dir, id) {
        Some(e) => follow(e, atoms_at::E_STRINGS).wrapping_add(atoms_bounds(e, id).0 as usize),
        None => dir,
    }
}
