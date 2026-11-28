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

use rerail_reads::ids::{INDEX_BITS, LINE_SLOT, Numbering, OP_ITEM, Run, SLOT_BITS, SPAN_SLOTS};
use rerail_reads::view::{F16Z_FORMAT, TapeView};
use rerail_reads::views::{ANCHOR_ITEM, IDS_TAPE, NO_ROW, PLACE_PAST_COMMENTS, PLACED_AT, PLACED_SEGMENTS, PLACED_TABLE, SEGMENT_WORDS, ViewDir, ViewDoc, ViewHome, ViewRow};
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
}

impl At<'_> {
    /// Its tape node, as found when the row was (`at`, `at_node`): a reader that keeps it finds the row again with
    /// no select (`Doc::at_node`).
    #[inline]
    pub fn node(&self) -> u32 {
        self.x
    }

    /// Its column `col`.
    #[inline]
    pub unsafe fn col(&self, col: u32) -> u64 {
        let (d, r, x) = (&self.d, self.r, self.x);
        match self.k {
            0 => d.node(r, x, col),
            1 => d.child(r, x, col),
            2 => d.field_at(r, x, col),
            3 => d.core(r, x, col),
            4 => d.line(r, x, col),
            5 => d.loc(Place::Line, r, col),
            6 => d.loc(Place::EndLine, r, col),
            7 => d.loc(Place::Col, r, col),
            8 => d.loc(Place::Docof, r, col),
            9 => d.tape(r, x, col),
            10 => d.depth(r, x, col),
            11 => d.order(r, col),
            12 => d.op(x, col),
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
        let at = At { d: self.d, k: self.k, r: self.r, x };
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

    /// Whether ids are local ids (`RERAIL_NODE_IDS=preorder`).
    #[inline]
    fn local(&self) -> bool {
        self.e.preorder == 1
    }

    /// Whether ids are tape positions (`IDS_TAPE`, `RERAIL_NODE_IDS=tape`): a node's or token's id is its tape node.
    #[inline]
    fn ids_tape(&self) -> bool {
        self.e.preorder == IDS_TAPE
    }

    /// Tape node `x`'s kind atom (`node`'s column 2), read off the tape: no number, no select.
    #[inline]
    pub unsafe fn kind_of(&self, x: u32) -> u64 {
        Self::item(&self.e.kinds, self.tape.number(x))
    }

    /// The entry's node table (`ViewDoc::node_table`): by local id, the numbered node's tape node and its id's low
    /// bits, derived once at the entry's making in its block; empty where it has none.
    #[inline(always)]
    unsafe fn node_table(&self) -> &'a [[u32; 2]] {
        let r = &self.e.node_table;
        if r.len == 0 { &[] } else { core::slice::from_raw_parts(items::<[u32; 2]>(r), r.len as usize) }
    }

    /// Numbered node `l`'s tape node: its node table's entry (one load), else a select of the numbering.
    #[inline]
    pub unsafe fn tape_node(&self, l: u32) -> u32 {
        match self.node_table().get(l as usize) {
            Some(e) => e[0],
            None => self.n.select(0, l),
        }
    }

    /// Numbered node `l`'s id: the document's key, then its low bits (`l` itself where ids are local ids).
    #[inline]
    pub unsafe fn node_id(&self, l: u32) -> u64 {
        let low = if self.local() {
            l
        } else if self.ids_tape() {
            self.tape_node(l)
        } else if let Some(e) = self.node_table().get(l as usize) {
            e[1]
        } else {
            self.n.low_of(0, l)
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
    /// `field` row `r`, its tape node `x`: `field`, its node's number read off its table where there is one.
    unsafe fn field_at(&self, r: u32, x: u32, col: u32) -> u64 {
        match self.field_table().get(r as usize) {
            Some(&[_, l]) if col != 1 => if col == 0 { self.parent_id(l, x) } else { self.node_id(l) },
            _ => self.field(x, col),
        }
    }

    /// The `field` rows' table (`ViewDoc::field_table`): by row, its tape node and number; empty where none is.
    unsafe fn field_table(&self) -> &'a [[u32; 2]] {
        let r = &self.e.field_table;
        if r.len == 0 { &[] } else { core::slice::from_raw_parts(items::<[u32; 2]>(r), r.len as usize) }
    }

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
            if self.ids_tape() { self.n.select(0, l) } else { self.n.low_of(0, l) }
        } else if l - e.nodes < e.tokens {
            if self.ids_tape() { self.n.select(1, l - e.nodes) } else { self.n.low_of(1, l - e.nodes) }
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
    /// `within`; an embedder's, by its segments (`placed_segments`). A file's own, as it is.
    fn placed(&self, span: u64) -> u64 {
        let e = self.e;
        if e.placement == 0 {
            return span;
        }
        if e.placement == PLACED_SEGMENTS {
            // SAFETY: the entry's segments run, where it points (`ViewDoc::segments`).
            return unsafe { self.placed_segments(span) };
        }
        let (lo, hi) = (i64::from(e.within_lo), i64::from(e.within_hi));
        if e.placement == PLACED_AT {
            return (lo << 32 | hi) as u64;
        }
        let clamp = |x: i64| x.wrapping_add(e.delta).max(lo).min(hi);
        let (s, t) = (clamp((span >> 32) as i64), clamp((span & 0xffff_ffff) as i64));
        (s << 32 | t) as u64
    }

    /// Segment `i` of the entry's (`SEGMENT_WORDS` u32s: its program range, its host range, whether it is copied).
    #[inline]
    unsafe fn segment(&self, i: u32) -> [u32; 5] {
        let at = items::<u32>(&self.e.segments).add(i as usize * SEGMENT_WORDS as usize);
        core::array::from_fn(|k| *at.add(k))
    }

    /// The host position program byte `at` maps to in segment `i` (`SegmentMap::point`): its copy, kept within the
    /// segment, or the start of a written segment's anchor.
    unsafe fn segment_point(&self, i: u32, at: u32) -> u32 {
        let [start, end, from, _, copied] = self.segment(i);
        if copied != 0 { from.wrapping_add(at.max(start).min(end).wrapping_sub(start)) } else { from }
    }

    /// An embedder's program's span `s..e` where it stands in its host (the CLIF's `placed_segments`:
    /// `SegmentMap::file_ranges(s..e).primary()`, read off its segments where they lie). The segment holding `s` (or
    /// the last before it) and those starting before `e` give one piece each, a copied one's overlapped part, a
    /// written one's whole anchor, pieces of no bytes left out; the span stands for the run of pieces, overlapping or
    /// touching, that holds the first one, or, with none, for the point `s` maps to.
    unsafe fn placed_segments(&self, span: u64) -> u64 {
        let n = self.e.segments.len as u32;
        let (s, e) = ((span >> 32) as u32, span as u32);
        if n == 0 {
            return span;
        }
        let pair = |a: u32, b: u32| u64::from(a) << 32 | u64::from(b);
        let partition = |pred: &dyn Fn([u32; 5]) -> bool| {
            let (mut lo, mut hi) = (0u32, n);
            while lo < hi {
                let mid = (lo + hi) >> 1;
                if pred(self.segment(mid)) { lo = mid + 1 } else { hi = mid }
            }
            lo
        };
        let first = partition(&|g| g[1] <= s).min(n - 1);
        let at = self.segment_point(first, s);
        let point = pair(at, at);
        if s == e {
            return point;
        }
        let last = partition(&|g| g[0] < e);
        // Passes over the pieces grow the run by each one it overlaps or touches, until a pass grows it no more.
        let (mut cs, mut ce, mut have) = (0u32, 0u32, false);
        loop {
            let mut grew = false;
            for i in first..last {
                let [start, end, from, to, copied] = self.segment(i);
                let (ps, pe) = if copied != 0 { (self.segment_point(i, s), self.segment_point(i, e)) } else { (from, to) };
                let piece = start != end && ps < pe;
                if piece && !have {
                    (cs, ce, have) = (ps, pe, true);
                } else if piece && ps <= ce && cs <= pe && (ps < cs || pe > ce) {
                    (cs, ce, grew) = (cs.min(ps), ce.max(pe), true);
                }
            }
            if !grew {
                break;
            }
        }
        if have { pair(cs, ce) } else { point }
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
        // Where ids are tape positions, the order key is the tape node: the tape is in preorder.
        let low = if self.local() {
            l
        } else if self.ids_tape() {
            self.tape_node(l)
        } else {
            self.n.order_of(l)
        };
        u64::from(self.e.key) << 32 | u64::from(low)
    }

    /// `op`'s row of the anonymous child at tape node `x` (`Item::Op`, `view_op`): its numbered node's id (an
    /// anonymous child is no root), its name atom by its tape code (`ViewDoc::names`).
    unsafe fn op(&self, x: u32, col: u32) -> u64 {
        if col == 0 { self.parent_id(1, x) } else { Self::item(&self.e.names, self.tape.number(x)) }
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
            2 => match self.field_table().get(r as usize) {
                Some(e) => e[0],
                None => self.n.select(2, r),
            },
            12 => self.n.select(OP_ITEM, r),
            _ => 0,
        };
        At { d: self, k, r, x }
    }

    /// View `k`'s row `r` whose tape node `x` its walk holds (a parent's next child, a node's next numbered ancestor):
    /// no node selected.
    #[inline]
    pub fn at_node(self, k: u32, r: u32, x: u32) -> At<'a> {
        At { d: self, k, r, x }
    }

    /// View `k`'s rows from `r` on, in order (`Rows`): each one's tape node the next of its numbering's (`Nodes`),
    /// not selected again for each row.
    #[inline]
    pub unsafe fn rows_from(self, k: u32, r: u32) -> Rows<'a> {
        let nodes = match k {
            0 | 3 | 4 | 9 | 10 => Some(self.n.nodes_from(0, r)),
            1 => Some(self.n.nodes_from(0, r.wrapping_add(1))),
            2 => Some(self.n.nodes_from(2, r)),
            12 => Some(self.n.nodes_from(OP_ITEM, r)),
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

    /// Numbered node `l`'s `field` row, or `NO_ROW` where it is under no field (`view_field_row`).
    pub unsafe fn field_row(&self, l: u32) -> u32 {
        let x = self.tape_node(l);
        if self.n.is(2, x) { self.n.rank(2, x) } else { NO_ROW }
    }

    /// The `field` row of the first fielded node from tape node `c` on, `c` and each sibling after it within `end`,
    /// or `NO_ROW` (`view_field_first`, `view_field_next`).
    pub unsafe fn fielded_from(&self, mut c: u32, end: u32) -> u32 {
        while c != NO_ROW {
            if self.n.is(2, c) {
                return self.n.rank(2, c);
            }
            c = self.tape.next_in(c, end);
        }
        NO_ROW
    }

    /// The `field` row of numbered node `l`'s first fielded tape child, or `NO_ROW` (`view_field_first`).
    pub unsafe fn field_first(&self, l: u32) -> u32 {
        let x = self.tape_node(l);
        self.fielded_from(self.tape.first_child(x), self.tape.end_of(x))
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
        self.number_near(low, &mut { u32::MAX })
    }

    /// `number`, its slot's entry first looked for at `hint` (`Numbering::number_of_near`: checked before use, any
    /// hint gives the same number).
    pub unsafe fn number_near(&self, low: u32, hint: &mut u32) -> u32 {
        let l = if self.local() {
            low
        } else if self.ids_tape() {
            // A tape position: the node there, if it is numbered, is its rank.
            if self.n.is(0, low) { self.n.rank(0, low) } else { NO_ROW }
        } else if self.e.node_by_id.len != 0 {
            // Its place among the granules' own nodes, then its number there in one load (`ViewDoc::node_by_id`):
            // no search of the granule's children.
            let place = if self.e.slot_buckets.len != 0 {
                let b = core::slice::from_raw_parts(items::<u32>(&self.e.slot_buckets), self.e.slot_buckets.len as usize);
                self.n.own_place_by_buckets(low, b)
            } else {
                self.n.own_place_of_near(low, hint)
            };
            match place {
                Some(p) if p < self.e.node_by_id.len as u32 => *items::<u32>(&self.e.node_by_id).add(p as usize),
                _ => NO_ROW,
            }
        } else {
            match self.n.number_of_near(low, hint) {
                Some((0, l)) => l,
                _ => NO_ROW,
            }
        };
        if l < self.e.nodes { l } else { NO_ROW }
    }

    /// The `Loc` position (`loc_id`'s inverse: a node, a token, a span) whose id's low 32 bits are `low`, or `NO_ROW`
    /// (`view_loc_number`): `low` itself where ids are local ids. Past the document's positions is the caller's to
    /// refuse.
    pub unsafe fn loc_number(&self, low: u32) -> u32 {
        let e = self.e;
        if self.local() {
            return low;
        }
        let slot = low >> INDEX_BITS;
        if slot >= (1 << SLOT_BITS) - SPAN_SLOTS {
            let j = ((1 << SLOT_BITS) - 1 - slot) << INDEX_BITS | low & ((1 << INDEX_BITS) - 1);
            return e.nodes.wrapping_add(e.tokens).wrapping_add(j);
        }
        if self.ids_tape() {
            return if self.n.is(0, low) {
                self.n.rank(0, low)
            } else if self.n.is(1, low) {
                e.nodes.wrapping_add(self.n.rank(1, low))
            } else {
                NO_ROW
            };
        }
        match self.n.number_of(low) {
            Some((0, l)) => l,
            Some((1, t)) => e.nodes.wrapping_add(t),
            _ => NO_ROW,
        }
    }

    /// Its node of local id `low`'s preorder key (`@rr_node_order`, `order`'s `k`: its key, its order in the
    /// document), or `None` where no node of it is numbered so.
    #[inline]
    pub unsafe fn order_of_id(&self, low: u32) -> Option<u64> {
        let l = self.number(low);
        (l != NO_ROW).then(|| self.order(l, 1))
    }

    /// The line whose span's id has low bits `low` (`LINE_SLOT << INDEX_BITS | l`: a `src_line` span, from 1; 0 the
    /// document's `source` span), as its bytes in its document's own source, `(start, stop)`: from its first byte to
    /// one past its terminator's last, or to the source's end on its last line and on line 0; and whether it ends with
    /// a terminator (`0 < l < lines_count`), whose length is read off the source's bytes (`view_line_span`,
    /// `view_line_terminated`). `None` for another id, a line past the last, or a document whose lines are its host's.
    #[inline]
    pub unsafe fn line_span(&self, low: u32) -> Option<(u32, u32, bool)> {
        let l = low & ((1u32 << INDEX_BITS) - 1);
        let count = self.lines.count();
        if low >> INDEX_BITS != LINE_SLOT || self.e.own_text == 0 || l > count {
            return None;
        }
        let len = self.e.source_len;
        if l == 0 {
            return Some((0, len, false));
        }
        let stop = if l >= count { len } else { self.lines.start(l) };
        Some((self.lines.start(l - 1), stop, l < count))
    }

    /// A node's text as the join program reads it (`node_txt`, decision 0287 §2): the line's text where `low` is a
    /// line's span (`lines`: `rr_node_text_any`'s; without, `Err(true)`: no row, as the host's `long_text`), else its
    /// numbered node's span of the source (`text_of`). `Err(false)`: no text read here (the stratum is the host's).
    #[inline]
    pub unsafe fn node_txt(&self, low: u32, lines: bool) -> Result<(*const u8, u32), bool> {
        if let Some((from, mut to, terminated)) = self.line_span(low) {
            if !lines {
                return Err(true);
            }
            if terminated {
                to -= terminator_length(self.source, to - 1);
            }
            return Ok((self.source.wrapping_add(from as usize), to.saturating_sub(from)));
        }
        self.text_of(low).ok_or(false)
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

/// The length of the line terminator whose last byte is at `last` of `src` (`lines_terminator_length`): 2 for CR LF
/// and NEL (C2 85), 3 for LS and PS (E2 80 A8, E2 80 A9), else 1.
unsafe fn terminator_length(src: *const u8, last: u32) -> u32 {
    let at = |i: u32| *src.add(i as usize);
    let x = at(last);
    if last < 1 {
        return 1;
    }
    let p1 = at(last - 1);
    if (x == 0x0a && p1 == 0x0d) || (x == 0x85 && p1 == 0xc2) {
        return 2;
    }
    if (x == 0xa8 || x == 0xa9) && p1 == 0x80 && last >= 2 && at(last - 2) == 0xe2 {
        return 3;
    }
    1
}
