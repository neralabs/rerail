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
use rerail_reads::views::{ANCHOR_ITEM, NO_ROW, PLACE_PAST_COMMENTS, PLACED_AT, PLACED_TABLE, ViewDir, ViewDoc, ViewHome, ViewRow};
use rerail_reads::tape::{InPlace, f16, f16z};

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
            _ => f16z::$name($self.t, &$self.v, $($a),*),
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
    /// Its source where it lies (its row's third home): a node's text is its span of it.
    pub source: *const u8,
}

/// The last view of `view_reads::VIEWS` (`doc`'s): a view's number past it is clamped to it, as `At::col`'s arms end.
pub const LAST_VIEW: u32 = (rerail_reads::views::VIEWS.len() - 1) as u32;

/// A view row found (`Doc::at`): its document, its view (`view_reads::VIEWS`), its row in the document and the tape
/// node its columns read (0 where they read none, or where a column finds its own: the `Loc` views').
#[derive(Clone, Copy)]
pub struct At<'a> {
    d: Doc<'a>,
    k: u32,
    r: u32,
    x: u32,
    /// Its node's id's low bits where its walk read them (`at_node_low`), else `NO_ROW`: read by the numbering.
    low: u32,
}

impl At<'_> {
    /// Its column `col`.
    #[inline]
    pub unsafe fn col(&self, col: u32) -> u64 {
        let (d, r, x) = (&self.d, self.r, self.x);
        match self.k {
            0 if col == 1 && self.low != NO_ROW => u64::from(d.e.key) << 32 | u64::from(self.low),
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
            11 => d.order(r, col),
            _ => d.doc(col),
        }
    }
}

/// A walk of a document's rows of one view in order (`Doc::rows_from`): the next row's number, and its numbering's
/// walk where its rows read a tape node.
pub struct Rows<'a> {
    d: Doc<'a>,
    k: u32,
    r: u32,
    nodes: Option<rerail_reads::ids::Nodes<'a>>,
}

impl<'a> Rows<'a> {
    /// The next row, found (`At`, as `Doc::at` finds it).
    #[inline]
    pub unsafe fn next(&mut self) -> At<'a> {
        let x = self.nodes.as_mut().map_or(0, |n| n.next());
        let at = At { d: self.d, k: self.k, r: self.r, x, low: NO_ROW };
        self.r = self.r.wrapping_add(1);
        at
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
            let rel = (home(d, ((*d).cache - 1) as u32, 0) as i64).wrapping_sub(t as i64);
            (v.side[0], v.side[1]) = (rel as u32, (rel >> 32) as u32);
        }
        Doc { e, n: &*items::<Numbering>(&e.numbering), tape: Tape { t: InPlace::new(t), v }, lines: Lines(InPlace::new(home(d, row.homes[1], row.lines))), source: home(d, row.homes[2], row.source) }
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

    /// Its entry's node table (`ViewDoc::node_table`: by local id, the numbered node's tape node and its id's low
    /// bits, made with the entry), where it has one: the host's reads' (`tape_format::view_reads`), so a node is
    /// one load, not a select or a granule's search.
    #[inline(always)]
    unsafe fn table(&self, l: u32) -> Option<[u32; 2]> {
        (u64::from(l) < self.e.node_table.len).then(|| *items::<[u32; 2]>(&self.e.node_table).add(l as usize))
    }

    /// Numbered node `l`'s tape node: its node table's, else the numbering's select.
    #[inline]
    pub unsafe fn tape_node(&self, l: u32) -> u32 {
        match self.table(l) {
            Some([x, _]) => x,
            None => self.n.select(0, l),
        }
    }

    /// Numbered node `l`'s id's low bits, its granule found from `hint` (`Numbering::low_of_near`): a walk's, whose
    /// nodes lie in one granule or its parents, the granules not searched again for each.
    #[inline]
    pub unsafe fn low_near(&self, l: u32, hint: &mut u32) -> u32 {
        if self.local() {
            return l;
        }
        match self.table(l) {
            Some([_, low]) => low,
            None => self.n.low_of_near(0, l, hint),
        }
    }

    /// Numbered node `l`'s id: the document's key, then its low bits (`l` itself where ids are local ids).
    #[inline]
    pub unsafe fn node_id(&self, l: u32) -> u64 {
        let low = if self.local() {
            l
        } else {
            match self.table(l) {
                Some([_, low]) => low,
                None => self.n.low_of(0, l),
            }
        };
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
            _ => self.place(x),
        }
    }

    /// Numbered node `l`'s place among its parent's children, its tape node `x` (decision 0656 step 4): an outermost
    /// anchor's its rank among the anchors; any other's read in its tape's auxiliary index, as its entry's shape
    /// numbers places (`rerail_reads::tape::place`, the one Rust read of it).
    unsafe fn place(&self, x: u32) -> u64 {
        let anchor = ANCHOR_ITEM as u32;
        if self.n.is(anchor, x) {
            return u64::from(self.n.rank(anchor, x));
        }
        let t = &self.tape;
        u64::from(rerail_reads::tape::place(t.t, &t.v, x, self.e.place == PLACE_PAST_COMMENTS, |c| t.parent(c), |c| t.number(c), |c| t.end_of(c)))
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
            match self.table(l) {
                Some([_, low]) => low,
                None => self.n.low_of(0, l),
            }
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
        // Placed by its table (`PLACED_TABLE`: a placement no word maps, its spans placed once, by local id), else its
        // span as read, placed: the CLIF read's and the host's (`view_reads::Doc::placed_loc`). Without the table a
        // guest at a declared place or in an embed read its spans clamped by a delta it does not have: line 1.
        let span = if self.e.placement == PLACED_TABLE { *items::<u64>(&self.e.placed).add(l as usize) } else { self.placed(self.loc_span(l)) };
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

    /// `doc`'s row (one a file): its key, its path's atom, its module's (`ViewDoc::doc_path`, `doc_module`).
    unsafe fn doc(&self, col: u32) -> u64 {
        u64::from(match col {
            0 => self.e.key,
            1 => self.e.doc_path,
            _ => self.e.doc_module,
        })
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
        At { d: self, k, r, x, low: NO_ROW }
    }

    /// View `k`'s row `r` whose tape node `x` its walk holds (a parent's next child, a node's next numbered ancestor):
    /// no node selected.
    #[inline]
    pub fn at_node(self, k: u32, r: u32, x: u32) -> At<'a> {
        At { d: self, k, r, x, low: NO_ROW }
    }

    /// `node` row `r`, of tape node `x` and id low bits `low`, as its walk read them (a climb's ancestor, a
    /// subtree's node): neither selected nor its granule searched.
    #[inline]
    pub fn at_node_low(self, r: u32, x: u32, low: u32) -> At<'a> {
        At { d: self, k: 0, r, x, low }
    }

    /// View `k`'s rows from `r` on, in order (`Rows`): each one's tape node the next of its numbering's (`Nodes`),
    /// not selected again for each row.
    #[inline]
    pub unsafe fn rows_from(self, k: u32, r: u32) -> Rows<'a> {
        let nodes = match k {
            0 | 3 | 4 | 9 | 10 => Some(self.n.nodes_from(0, r)),
            1 => Some(self.n.nodes_from(0, r.wrapping_add(1))),
            2 => Some(self.n.nodes_from(2, r)),
            _ => None,
        };
        Rows { d: self, k, r, nodes }
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
        } else if self.e.node_by_id.len != 0 {
            // Its entry's nodes by their ids' places (`ViewDoc::node_by_id`, made with the node table): the
            // numbering's slot entry, then one load, as the host's `local_of`; the place's node checked by its id.
            match self.n.own_place_of(low) {
                Some(p) if u64::from(p) < self.e.node_by_id.len => {
                    let l = *items::<u32>(&self.e.node_by_id).add(p as usize);
                    if self.table(l).is_some_and(|[_, w]| w == low) { l } else { NO_ROW }
                }
                _ => NO_ROW,
            }
        } else {
            match self.n.number_of(low) {
                Some((0, l)) => l,
                _ => NO_ROW,
            }
        };
        if l < self.e.nodes { l } else { NO_ROW }
    }

    /// Its node of local id `low`'s preorder key (`@rr_node_order`, `order`'s `k`: its key, its order in the
    /// document), or `None` where no node of it is numbered so.
    #[inline]
    pub unsafe fn order_of_id(&self, low: u32) -> Option<u64> {
        let l = self.number(low);
        (l != NO_ROW).then(|| self.order(l, 1))
    }

    /// The text of the node whose id's low 32 bits are `low`: its numbered node's span of the source, where the
    /// source lies (its row's home, `view_source`'s); `None` for no numbered node (a comment no node is, a token) or
    /// a span past the source.
    #[inline]
    pub unsafe fn text_of(&self, low: u32) -> Option<(*const u8, u32)> {
        let l = self.number(low);
        if l == NO_ROW {
            return None;
        }
        let x = self.tape_node(l);
        let (s, e) = (self.tape.start(x), self.tape.end(x));
        if s > e || e > self.tape.v.source_len {
            return None;
        }
        Some((self.source.wrapping_add(s as usize), e - s))
    }
}

/// An atoms directory's layout (`rerail_blocks::atoms::AtomsDirRoot`, `DirEntry`; a test holds it to it): its
/// runs' count and entries, and each entry's first id, end, the id its strings start at, their ends and bytes (its
/// hash table from hash to id after them, 0666's: `atoms_find`'s).
pub(crate) mod atoms_at {
    pub const DIR_N: usize = 0;
    pub const DIR_ENTRIES: usize = 8;
    pub const E_FIRST: usize = 0;
    pub const E_END: usize = 4;
    pub const E_FROM: usize = 8;
    pub const E_ENDS: usize = 16;
    pub const E_STRINGS: usize = 32;
    pub const E_TABLE: usize = 48;
    pub const E_SIZE: usize = 64;
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

/// The `k`th atom (from 0, the runs in order) of directory `dir` whose slot in its run's table, probed from hash `h`
/// to the first empty slot, has `h`'s high half, whose id is one of the run's own, and whose string is `len` bytes;
/// `u32::MAX` past the last: the CLIF's `atoms_find` (decision 0666). The caller compares each one's bytes.
pub unsafe fn atoms_find(dir: *const u8, h: u64, len: u32, k: u32) -> u32 {
    use atoms_at::*;
    let n = dir.add(DIR_N).cast::<u64>().read_unaligned() as u32;
    let entries = follow(dir, DIR_ENTRIES);
    let tag = h & 0xFFFF_FFFF_0000_0000;
    let mut seen = 0u32;
    for r in 0..n {
        let e = entries.wrapping_add(r as usize * E_SIZE);
        let table = follow(e, E_TABLE).cast::<u64>();
        let slots = e.add(E_TABLE + 8).cast::<u64>().read_unaligned();
        let mask = slots.wrapping_sub(1);
        let (first, end) = (e.add(E_FIRST).cast::<u32>().read_unaligned(), e.add(E_END).cast::<u32>().read_unaligned());
        let mut i = h & mask;
        for _ in 0..slots {
            let v = table.add(i as usize).read_unaligned();
            if v == 0 {
                break;
            }
            let id = (v as u32).wrapping_sub(1);
            if v & 0xFFFF_FFFF_0000_0000 == tag && id >= first && id < end {
                let (s0, s1) = atoms_bounds(e, id);
                if s1.wrapping_sub(s0) as u32 == len {
                    if seen == k {
                        return id;
                    }
                    seen += 1;
                }
            }
            i = (i + 1) & mask;
        }
    }
    u32::MAX
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document of `n` numbered nodes whose entry has its node tables (`ViewDoc::node_table`, `node_by_id`), its
    /// numbering one granule whose nodes' ids are in another order than their numbers: the reads take a node's tape
    /// node and id from the table, and an id's node from `node_by_id`, never the numbering's select or search.
    struct Made {
        e: Box<ViewDoc>,
        n: Box<Numbering>,
        _table: Vec<[u32; 2]>,
        _by_id: Vec<u32>,
        _by_slot: Vec<[u32; 3]>,
        _nodes: Vec<u32>,
        _tokens: Vec<u32>,
    }

    const SLOT: u32 = 3;
    const KEY: u32 = 7;
    /// Node `l`'s place among its granule's ids, and its tape node.
    const PLACE: [u32; 5] = [2, 0, 4, 1, 3];
    const TAPE: [u32; 5] = [0, 5, 9, 12, 20];

    fn low(l: usize) -> u32 {
        SLOT << INDEX_BITS | PLACE[l]
    }

    fn made() -> Made {
        let n = PLACE.len();
        let table: Vec<[u32; 2]> = (0..n).map(|l| [TAPE[l], low(l)]).collect();
        let mut by_id = vec![0u32; n];
        for l in 0..n {
            by_id[PLACE[l] as usize] = l as u32;
        }
        let (by_slot, nodes, tokens) = (vec![[SLOT, 0, 0]], vec![0, n as u32], vec![0, 0]);
        let mut m = Made { e: Box::default(), n: Box::default(), _table: table, _by_id: by_id, _by_slot: by_slot, _nodes: nodes, _tokens: tokens };
        // SAFETY: each run points at a vector the test holds, from a field of a box it holds: none moves.
        unsafe {
            m.e.key = KEY;
            m.e.nodes = n as u32;
            Run::set(&raw mut m.e.node_table, m._table.as_ptr(), n);
            Run::set(&raw mut m.e.node_by_id, m._by_id.as_ptr(), n);
            m.n.granules = 1;
            Run::set(&raw mut m.n.by_slot, m._by_slot.as_ptr(), 1);
            Run::set(&raw mut m.n.nodes, m._nodes.as_ptr(), 2);
            Run::set(&raw mut m.n.tokens, m._tokens.as_ptr(), 2);
        }
        m
    }

    fn doc(m: &Made) -> Doc<'_> {
        // SAFETY: no read here reaches the tape, the lines or the source.
        unsafe { Doc { e: &m.e, n: &m.n, tape: Tape { t: InPlace::new(std::ptr::null()), v: TapeView::default() }, lines: Lines(InPlace::new(std::ptr::null())), source: std::ptr::null() } }
    }

    #[test]
    fn a_nodes_tape_node_and_id_are_its_node_tables() {
        let m = made();
        let d = doc(&m);
        let mut hint = 0;
        for l in 0..PLACE.len() {
            // SAFETY: the document's runs are set (`made`).
            unsafe {
                assert_eq!(d.tape_node(l as u32), TAPE[l]);
                assert_eq!(d.node_id(l as u32), u64::from(KEY) << 32 | u64::from(low(l)));
                assert_eq!(d.low_near(l as u32, &mut hint), low(l));
                assert_eq!(d.at_node_low(l as u32, TAPE[l], low(l)).col(1), d.node_id(l as u32));
            }
        }
    }

    #[test]
    fn an_ids_node_is_its_node_by_id() {
        let m = made();
        let d = doc(&m);
        // SAFETY: the document's runs are set (`made`).
        unsafe {
            for l in 0..PLACE.len() {
                assert_eq!(d.number(low(l)), l as u32);
            }
            // A place past the granule's nodes, and a slot the numbering has none of: no node.
            assert_eq!(d.number(SLOT << INDEX_BITS | PLACE.len() as u32), NO_ROW);
            assert_eq!(d.number((SLOT + 1) << INDEX_BITS), NO_ROW);
        }
    }
}
