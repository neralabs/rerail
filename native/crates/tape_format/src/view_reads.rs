//! The views' reads in Rust, written directly (decision 0353 step 3 on the CPU, the user's standing rule: all
//! generated Rust is written directly, never translated from CLIF): a view's row is a read of the document's tape and
//! of its projection's numbering where they lie, through its entry (`ViewDoc`) in its batch's views' directory. The
//! reads in CLIF (`rerail_clif::view_reads`) are the JIT's and the kernels'; these take the same steps over the same
//! bytes, so the host's executor reads the same blocks as the device's. Tests hold them to the JIT's and to the
//! host's rows (`the_view_reads_are_the_hosts_rows`, and an embedder's placement in `the_rust_reads_place_an_embedders_spans_as_its_segments_do`).
//!
//! The views no CLIF reads yet (`tape`, `ast_depth`, `node_order`, `next_sibling`, `rr_report_location`) are read here
//! from the same entry and tape: their first definition, the host's rows the reference.

use crate::reads::{NONE, f16, f16z::Z};
use rerail_clif::ids::{INDEX_BITS, Numbering, SLOT_BITS, SPAN_SLOTS};
use rerail_clif::view::{F16Z_FORMAT, TapeView};
use rerail_clif::view_reads::{NO_ROW, NO_SPAN, PLACED_AT, PLACED_COPIED, PLACED_SEGMENTS, PLACED_TABLE, ViewDoc};

/// A document's tape as the views read it: its bytes from its view's base, its view, in its format; a 16z tape with
/// the cache its reads share.
#[derive(Clone, Copy)]
pub enum Tape<'a> {
    F16(&'a [u8], &'a TapeView),
    F16Z(Z<'a>),
}

impl<'a> Tape<'a> {
    /// The tape `t` of view `v`; `cache`, the cache a 16z tape's reads share (a live `view::cache::Spec` of at least
    /// the view's entry size), unread for another format.
    pub fn new(t: &'a [u8], v: &'a TapeView, cache: *const u8) -> Tape<'a> {
        match v.format {
            F16Z_FORMAT => Tape::F16Z(Z { t, v, cache }),
            _ => Tape::F16(t, v),
        }
    }
    fn view(&self) -> &TapeView {
        match self {
            Tape::F16(_, v) => v,
            Tape::F16Z(z) => z.v,
        }
    }
    #[inline]
    pub fn number(&self, i: u32) -> u32 {
        match *self {
            Tape::F16(t, v) => f16::number(t, v, i),
            Tape::F16Z(z) => z.number(i),
        }
    }
    #[inline]
    pub fn size(&self, i: u32) -> u32 {
        match *self {
            Tape::F16(t, v) => f16::size(t, v, i),
            Tape::F16Z(z) => z.size(i),
        }
    }
    #[inline]
    pub fn start(&self, i: u32) -> u32 {
        match *self {
            Tape::F16(t, v) => f16::start(t, v, i),
            Tape::F16Z(z) => z.start(i),
        }
    }
    #[inline]
    pub fn end(&self, i: u32) -> u32 {
        match *self {
            Tape::F16(t, v) => f16::end(t, v, i),
            Tape::F16Z(z) => z.end(i),
        }
    }
    /// The node's parent, `NONE` at the root (`tape_parent`).
    #[inline]
    pub fn parent(&self, i: u32) -> u32 {
        if i == 0 {
            return NONE;
        }
        match *self {
            Tape::F16(t, v) => f16::parent(t, v, i),
            Tape::F16Z(z) => z.parent(i),
        }
    }
    /// One past the node's subtree, no further than the tape's nodes (`tape_end_of`, `hop`).
    #[inline]
    pub fn end_of(&self, i: u32) -> u32 {
        i.wrapping_add(self.size(i)).max(i.wrapping_add(1)).min(self.view().nodes)
    }
    /// The node's first child, or `NONE` (`tape_first_child`).
    #[inline]
    pub fn first_child(&self, i: u32) -> u32 {
        if self.size(i) > 1 { i + 1 } else { NONE }
    }
    /// The node after `i`'s subtree while it is before `end`, else `NONE` (`tape_next_in`).
    #[inline]
    pub fn next_in(&self, i: u32, end: u32) -> u32 {
        let after = self.end_of(i);
        if after < end { after } else { NONE }
    }
    /// Its auxiliary index in its home (decision 0656), after its presence: the presence's bytes from its header
    /// rounded up to 16 (`f16::presence::aux_at`), as `tape_place` finds it. `None` without a presence, or where the
    /// bytes there are no index.
    fn aux(&self) -> Option<crate::aux_index::AuxIndex<'a>> {
        let (t, v) = match *self {
            Tape::F16(t, v) => (t, v),
            Tape::F16Z(z) => (z.t, z.v),
        };
        if v.presence == 0 {
            return None;
        }
        let at = v.presence as usize;
        let head = t.get(at..at + crate::f16::presence::HEAD)?;
        let w = |k: usize| u32::from_le_bytes([head[4 * k], head[4 * k + 1], head[4 * k + 2], head[4 * k + 3]]) as usize;
        let len = crate::f16::presence::HEAD + 8 * (w(0) + w(1)) * w(2) + 4 * w(3);
        crate::aux_index::AuxIndex::read(t.get(crate::f16::presence::aux_at(at, at + len)..)?).ok()
    }
    /// Named node `node`'s place among its parent's named children (`tape_place`, `aux_index::place`): 0 for the
    /// first, and for the root; from the last every-K-th named sibling at or before it, the siblings stepped to it,
    /// each named one counted. `NONE` where the home holds no index.
    pub fn place(&self, node: u32) -> u32 {
        use crate::aux_index::{K, NONE as NO_ENTRY};
        let Some(aux) = self.aux() else { return NONE };
        let p = self.parent(node);
        if p == NONE {
            return 0;
        }
        let (mut i, mut c) = match aux.eighth_at_or_before(p, node) {
            (_, NO_ENTRY) => (0, p + 1),
            (k, e) => (k * K, e),
        };
        while c < node {
            i += u32::from(aux.named(self.number(c)));
            c = self.end_of(c);
        }
        i
    }
    /// Named node `node`'s place among its parent's named children that are no comment (`tape_place_past_comments`,
    /// `aux_index::place_past_comments`: T's shape's places): as `place`, the comments before the every-K-th named
    /// sibling taken off, and the steps after it counting no comment. `NONE` where the home holds no index.
    pub fn place_past_comments(&self, node: u32) -> u32 {
        use crate::aux_index::K;
        let Some(aux) = self.aux() else { return NONE };
        let p = self.parent(node);
        if p == NONE {
            return 0;
        }
        let (mut i, mut c) = match aux.eighth_past_comments(p, node) {
            None => (0, p + 1),
            Some((k, e, q)) => (k * K - q, e),
        };
        while c < node {
            let code = self.number(c);
            i += u32::from(aux.named(code) && !aux.comment(code));
            c = self.end_of(c);
        }
        i
    }
}

/// Which place a `Loc` view's second column holds (`view_loc_*`), and core.rerail's `rr_report_location`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    Line,
    EndLine,
    Col,
    Docof,
    /// `rr_report_location(n, line, column, end_line, end_column)`: each end's line and its column from 1.
    Report,
}

/// A document as its views' reads take it: its entry, its tape, and its line index (decision 0612: its language's
/// one, `newlines`, with its word's room after it), where its directory row places them; and its source where it
/// lies, whose bytes say where a line's text ends (its terminator's length: `Report`'s end column).
#[derive(Clone)]
pub struct Doc<'a> {
    pub entry: &'a ViewDoc,
    pub tape: Tape<'a>,
    pub lines: &'a [u8],
    pub source: &'a [u8],
    /// The numbered node last selected and its tape node (`tape_node`), and the `field` view's row it is the node of
    /// (`fielded`; `u32::MAX`: none): a row's columns read one after another select its node once. `u32::MAX`: none
    /// yet.
    last: std::cell::Cell<Selected>,
}

/// What a `Doc` last selected (`Doc::selected`): numbered node `l`, its tape node `x`, and the `field` view's row
/// `field_row` whose node it is (`u32::MAX`: none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selected {
    pub l: u32,
    pub x: u32,
    pub field_row: u32,
}

impl Selected {
    /// Nothing selected.
    pub const NONE: Selected = Selected { l: u32::MAX, x: 0, field_row: u32::MAX };
}

impl<'a> Doc<'a> {
    /// The document's reads, through its directory row's `entry`.
    #[inline]
    pub fn new(entry: &'a ViewDoc, tape: Tape<'a>, lines: &'a [u8], source: &'a [u8]) -> Doc<'a> {
        Doc { entry, tape, lines, source, last: std::cell::Cell::new(Selected::NONE) }
    }
    #[inline(always)]
    fn numbering(&self) -> &'a Numbering {
        // SAFETY: the entry's numbering, one, set where the entry was made (`ViewEntry::new`) to its projection's in
        // the same block, which lives as long as the entry.
        unsafe { &self.entry.numbering.as_slice::<Numbering>()[0] }
    }
    #[inline(always)]
    fn items(&self, run: &'a rerail_clif::ids::Run) -> &'a [u32] {
        // SAFETY: one of the entry's runs of u32s, set where it was made, in its block.
        unsafe { run.as_slice::<u32>() }
    }
    /// What it last selected: what a caller keeps to read the document again without selecting the node again.
    #[inline]
    pub fn selected(&self) -> Selected {
        self.last.get()
    }
    /// It, knowing numbered node `l`'s tape node `x` (a probe's hop found it), and the `field` view's row it is the
    /// node of (`u32::MAX`: not known).
    #[inline]
    pub fn knowing(self, l: u32, x: u32, field_row: u32) -> Self {
        self.last.set(Selected { l, x, field_row });
        self
    }
    /// Numbered node `l`'s tape node (`num_select`).
    #[inline]
    pub fn tape_node(&self, l: u32) -> u32 {
        let s = self.last.get();
        if s.l == l {
            return s.x;
        }
        // A scan's next row: the next numbered node from the last one, no sample read.
        // SAFETY: the entry's numbering (`numbering`).
        let x = unsafe { if s.l != u32::MAX && s.l + 1 == l { self.numbering().next_at(0, s.x + 1) } else { self.numbering().select(0, l) } };
        self.last.set(Selected { l, x, field_row: u32::MAX });
        x
    }
    /// The numbered nodes before tape node `x` (`num_rank`).
    #[inline]
    fn rank(&self, x: u32) -> u32 {
        // SAFETY: as `tape_node`.
        unsafe { self.numbering().rank(0, x) }
    }
    /// Whether tape node `x` is numbered (`num_is`).
    #[inline]
    fn is(&self, item: u32, x: u32) -> bool {
        // SAFETY: as `tape_node`.
        unsafe { self.numbering().is(item, x) }
    }
    /// Numbered node `l`'s id: the document's key, then its low bits (`l` itself where ids are local ids).
    #[inline]
    pub fn node_id(&self, l: u32) -> i64 {
        let low = match self.entry.preorder {
            // SAFETY: as `tape_node`.
            0 => unsafe { self.numbering().low_of(0, l) },
            rerail_clif::view_reads::IDS_TAPE => self.tape_node(l),
            _ => l,
        };
        (i64::from(self.entry.key) << 32) | i64::from(low)
    }
    /// Numbered node `l`'s parent's id: its tape parent's, or the root's where that is not numbered (a narrowed
    /// projection's anchor); -1 for the root.
    #[inline]
    pub fn parent_id(&self, l: u32, x: u32) -> i64 {
        if l == 0 {
            return -1;
        }
        let q = self.tape.parent(x);
        self.node_id(if self.is(0, q) { self.rank(q) } else { 0 })
    }

    /// `view_node(l, col)`: the document (a program's file), the id, the kind atom, the start, the end.
    #[inline]
    pub fn node(&self, l: u32, col: u32) -> i64 {
        match col {
            0 => i64::from(self.entry.file),
            1 => self.node_id(l),
            _ => {
                let x = self.tape_node(l);
                match col {
                    2 => i64::from(self.items(&self.entry.kinds)[self.tape.number(x) as usize]),
                    3 => i64::from(self.tape.start(x)),
                    _ => i64::from(self.tape.end(x)),
                }
            }
        }
    }
    /// `view_child(i, col)`, of numbered node `i + 1`: the parent's id, the child's id, its place.
    #[inline]
    pub fn child(&self, i: u32, col: u32) -> i64 {
        let l = i + 1;
        match col {
            0 => self.parent_id(l, self.tape_node(l)),
            1 => self.node_id(l),
            // An outermost anchor's, its rank among the anchors; any other's read in the tape's auxiliary index, as the
            // entry's shape numbers places (decision 0656 step 4).
            _ => {
                let x = self.tape_node(l);
                let anchor = rerail_clif::view_reads::ANCHOR_ITEM as u32;
                i64::from(if self.is(anchor, x) {
                    // SAFETY: as `tape_node`.
                    unsafe { self.numbering().rank(anchor, x) }
                } else if self.entry.place == rerail_clif::view_reads::PLACE_PAST_COMMENTS {
                    self.tape.place_past_comments(x)
                } else {
                    self.tape.place(x)
                })
            }
        }
    }
    /// The numbered node of the `field` view's row `i`, and its tape node.
    #[inline]
    pub fn fielded(&self, i: u32) -> (u32, u32) {
        let s = self.last.get();
        if s.field_row == i {
            return (s.l, s.x);
        }
        // SAFETY: as `tape_node`.
        let x = unsafe { self.numbering().select(2, i) };
        let l = self.rank(x);
        self.last.set(Selected { l, x, field_row: i });
        (l, x)
    }
    /// `view_field(i, col)`: the parent's id, the field atom, the child's id.
    #[inline]
    pub fn field(&self, i: u32, col: u32) -> i64 {
        let (l, x) = self.fielded(i);
        match col {
            0 => self.parent_id(l, x),
            1 => i64::from(self.items(&self.entry.fields)[self.tape.number(x) as usize]),
            _ => self.node_id(l),
        }
    }
    /// `view_core(l, col)`: the document, its tape node, its id.
    #[inline]
    pub fn core(&self, l: u32, col: u32) -> i64 {
        match col {
            0 => i64::from(self.entry.key),
            1 => i64::from(self.tape_node(l)),
            _ => self.node_id(l),
        }
    }
    /// `view_line(l, col)`: its id, the line its start is on (from 1, `lines_line`).
    #[inline]
    pub fn line(&self, l: u32, col: u32) -> i64 {
        match col {
            0 => self.node_id(l),
            _ => i64::from(lines_line(self.lines, self.tape.start(self.tape_node(l)))),
        }
    }

    /// `tape(n, p, size, depth)`: its id, its parent's, the numbered nodes under it, its depth.
    #[inline]
    pub fn tape_row(&self, l: u32, col: u32) -> i64 {
        match col {
            0 => self.node_id(l),
            1 => self.parent_id(l, self.tape_node(l)),
            2 => i64::from(self.size(l)),
            _ => i64::from(self.depth(l)),
        }
    }
    /// The numbered nodes under numbered node `l`: those of its tape subtree, by rank.
    #[inline]
    pub fn size(&self, l: u32) -> u32 {
        let x = self.tape_node(l);
        self.rank(x.wrapping_add(self.tape.size(x))).wrapping_sub(l).wrapping_sub(1)
    }
    /// Numbered node `l`'s depth, the root's 0: its numbered tape ancestors, by a climb.
    #[inline]
    pub fn depth(&self, l: u32) -> u32 {
        let mut x = self.tape_node(l);
        let mut d = 0;
        loop {
            let q = self.tape.parent(x);
            if q == NONE {
                return d;
            }
            d += u32::from(self.is(0, q));
            x = q;
        }
    }
    /// `ast_depth(n, k)`: its id, its depth.
    #[inline]
    pub fn depth_row(&self, l: u32, col: u32) -> i64 {
        if col == 0 { self.node_id(l) } else { i64::from(self.depth(l)) }
    }
    /// `node_order(n, k)`: its id, `document << 32 |` its order key (`num_order_of`).
    #[inline]
    pub fn order_row(&self, l: u32, col: u32) -> i64 {
        if col == 0 {
            return self.node_id(l);
        }
        // SAFETY: as `tape_node`.
        let order = unsafe { self.numbering().order_of(l) };
        (i64::from(self.entry.key) << 32) | i64::from(order)
    }
    /// The `op` view's row `i`: the document's `i`-th anonymous child of a numbered node (`Item::Op`), its tape node.
    #[inline]
    pub fn op_child(&self, i: u32) -> u32 {
        // SAFETY: as `tape_node`.
        unsafe { self.numbering().select(rerail_clif::ids::OP_ITEM, i) }
    }
    /// `view_op(i, col)`: row `i`'s numbered node's id (an anonymous child is no root), its name atom (by its tape
    /// code, `ViewDoc::names`).
    #[inline]
    pub fn op(&self, i: u32, col: u32) -> i64 {
        let x = self.op_child(i);
        match col {
            0 => self.parent_id(1, x),
            _ => i64::from(self.items(&self.entry.names)[self.tape.number(x) as usize]),
        }
    }
    /// The numbered node of the `next_sibling` view's row `i` (a node after a numbered sibling), and its tape node.
    #[inline]
    pub fn after_sibling(&self, i: u32) -> (u32, u32) {
        // SAFETY: as `tape_node`.
        let x = unsafe { self.numbering().select(3, i) };
        (self.rank(x), x)
    }
    /// `next_sibling(a, b)`'s row `i`: `a`, the numbered sibling before `b`, by id (`-1` for none: a climb from the
    /// tape node before `b` to its parent's child, past siblings left out); `b`'s id.
    #[inline]
    pub fn sibling(&self, i: u32, col: u32) -> i64 {
        let (l, x) = self.after_sibling(i);
        if col != 0 {
            return self.node_id(l);
        }
        let q = self.tape.parent(x);
        if q == NONE || x == 0 {
            return -1;
        }
        let mut c = x - 1;
        loop {
            loop {
                let up = self.tape.parent(c);
                if up == NONE || up == q {
                    break;
                }
                c = up;
            }
            if self.is(0, c) {
                return self.node_id(self.rank(c));
            }
            if c == 0 {
                return -1;
            }
            c -= 1;
        }
    }

    /// `Loc` position `l`'s id: a node's or token's by its numbering, a span's in the top slots; `l` itself where
    /// ids are local ids.
    #[inline]
    pub fn loc_id(&self, l: u32) -> i64 {
        let e = self.entry;
        let tape = e.preorder == rerail_clif::view_reads::IDS_TAPE;
        let low = if e.preorder != 0 && !tape {
            l
        } else if l < e.nodes {
            if tape {
                self.tape_node(l)
            } else {
                // SAFETY: as `tape_node`.
                unsafe { self.numbering().low_of(0, l) }
            }
        } else {
            let i = l - e.nodes;
            if i < e.tokens {
                // SAFETY: as `tape_node`.
                unsafe { if tape { self.numbering().select(1, i) } else { self.numbering().low_of(1, i) } }
            } else {
                let j = i - e.tokens;
                let hi = (j >> INDEX_BITS).min(SPAN_SLOTS - 1);
                let slot = ((1u32 << SLOT_BITS) - 1) - hi;
                slot << INDEX_BITS | j & ((1 << INDEX_BITS) - 1)
            }
        };
        (i64::from(e.key) << 32) | i64::from(low)
    }
    /// The document's lines (`lines_count`), read off its line index: the one definition of a line and a place
    /// (decision 0612); a guest's are its host's.
    #[inline(always)]
    fn loc_count(&self) -> u32 {
        crate::newlines::count(self.lines).expect("a line index") + 1
    }
    /// Line `k`'s first byte (from 0, `lines_start`).
    #[inline(always)]
    fn loc_start(&self, k: u32) -> u32 {
        f16::line_start(self.lines, &TapeView::default(), k)
    }
    /// Where line `k`'s (from 0) text ends: its terminator's first byte, or the text's end on the last line
    /// (`LineTable::end`).
    #[inline]
    fn loc_end(&self, k: u32) -> u32 {
        if k + 1 >= self.loc_count() {
            return self.entry.source_len;
        }
        let last = self.loc_start(k + 1) - 1;
        last + 1 - crate::newlines::length_ending_at(self.source, last as usize) as u32
    }
    /// `Loc` position `l`'s byte span: a node's or token's off the tape, the whole text, a line's text, or a node's
    /// end point.
    #[inline]
    pub fn loc_span(&self, l: u32) -> (u32, u32) {
        let e = self.entry;
        let of_tape = |x: u32| (self.tape.start(x), self.tape.end(x));
        if l < e.nodes {
            return of_tape(self.tape_node(l));
        }
        let i = l - e.nodes;
        if i < e.tokens {
            // SAFETY: as `tape_node`.
            return of_tape(unsafe { self.numbering().select(1, i) });
        }
        let j = i - e.tokens;
        // The end points follow the whole text and the lines where the text is the document's own.
        let ends_from = if e.own_text != 0 { self.loc_count() + 1 } else { 0 };
        if j >= ends_from {
            let end = self.tape.end(self.tape_node(j - ends_from));
            (end, end)
        } else if j == 0 {
            (0, e.source_len)
        } else {
            // Line `j`'s (from 1) text: its start, and its end before its terminator (on the line, as the CLIF reads'
            // start alone stands for both ends where only its line and column are read).
            (self.loc_start(j - 1), self.loc_end(j - 1))
        }
    }
    /// A guest's span where it stands in its host: copied, each end moved by `delta` and kept within `within`; at
    /// `within`; an embedder's, by its segments (`placed_segments`). A file's own, as it is. The CLIF read's
    /// (`placed`); a placement by its table is `placed_loc`'s.
    #[inline]
    pub fn placed(&self, (s, e): (u32, u32)) -> (u32, u32) {
        let d = self.entry;
        match d.placement {
            0 => (s, e),
            PLACED_AT => (d.within_lo, d.within_hi),
            PLACED_SEGMENTS => self.placed_segments((s, e)),
            PLACED_COPIED => {
                let (lo, hi) = (i64::from(d.within_lo), i64::from(d.within_hi));
                let clamp = |x: u32| (i64::from(x).wrapping_add(d.delta)).max(lo).min(hi) as u32;
                (clamp(s), clamp(e))
            }
            other => unreachable!("placement {other} places no span as read"),
        }
    }
    /// An embedder's program's span `s..e` where it stands in its host (`SegmentMap::file_ranges(s..e).primary()`),
    /// read off its segments where they lie (`ViewDoc::segments`): the CLIF read's (`placed_segments`), step for step.
    /// The segment holding `s` (or the last before it) and those starting before `e` give one piece each, a copied
    /// one's overlapped part, a written one's whole anchor, pieces of no bytes left out; the span is the run of pieces,
    /// overlapping or touching, that holds the first one, or, with none, the point `s` maps to.
    fn placed_segments(&self, (s, e): (u32, u32)) -> (u32, u32) {
        // SAFETY: the entry's segments (`SEGMENT_WORDS` u32s each), set where it was made, in its block.
        let segs = unsafe { self.entry.segments.as_slice::<[u32; 5]>() };
        if segs.is_empty() {
            return (s, e);
        }
        // `[start, end, from, to, copied]`: its program range, its host range, 1 where it is copied.
        let point = |g: &[u32; 5], at: u32| if g[4] != 0 { g[2].wrapping_add(at.max(g[0]).min(g[1]).wrapping_sub(g[0])) } else { g[2] };
        let first = segs.partition_point(|g| g[1] <= s).min(segs.len() - 1);
        let at = point(&segs[first], s);
        if s == e {
            return (at, at);
        }
        let last = segs.partition_point(|g| g[0] < e);
        let (mut cs, mut ce, mut have) = (0u32, 0u32, false);
        loop {
            let mut grew = false;
            for g in segs.iter().take(last).skip(first) {
                let (ps, pe) = if g[4] != 0 { (point(g, s), point(g, e)) } else { (g[2], g[3]) };
                if g[0] == g[1] || ps >= pe {
                    continue;
                }
                if !have {
                    (cs, ce, have) = (ps, pe, true);
                } else if ps <= ce && cs <= pe && (ps < cs || pe > ce) {
                    (cs, ce, grew) = (cs.min(ps), ce.max(pe), true);
                }
            }
            if !grew {
                break;
            }
        }
        if have { (cs, ce) } else { (at, at) }
    }
    /// `Loc` position `l`'s span where it stands in its host: by the entry's table where its placement is one
    /// (`PLACED_TABLE`: no word maps it, its spans placed once), else its span as read, placed (`placed`). The CLIF
    /// read's (`loc`).
    #[inline]
    fn placed_loc(&self, l: u32) -> (u32, u32) {
        if self.entry.placement == PLACED_TABLE {
            // SAFETY: the entry's table of u64s (`ViewDoc::placed`), one a position, set where it was made, in its block.
            let span = unsafe { self.entry.placed.as_slice::<u64>() }[l as usize];
            return ((span >> 32) as u32, span as u32);
        }
        self.placed(self.loc_span(l))
    }
    /// The 1-based line holding byte `at` (`LineTable::line` + 1: the `line` view's read, `lines_line`).
    #[inline]
    fn loc_line(&self, at: u32) -> u32 {
        lines_line(self.lines, at)
    }
    /// A `Loc` view's row `l`: the id, then its place (`view_loc_line`, `view_loc_end`, `view_loc_col`,
    /// `view_loc_docof`; `rr_report_location`'s four).
    #[inline]
    pub fn loc(&self, place: Place, l: u32, col: u32) -> i64 {
        if col == 0 {
            return self.loc_id(l);
        }
        if place == Place::Docof {
            return i64::from(self.entry.file);
        }
        let (s, e) = self.placed_loc(l);
        if place == Place::Report {
            let at = if col <= 2 { s } else { e };
            let line = self.loc_line(at);
            return i64::from(if col % 2 == 1 { line } else { at.wrapping_sub(self.loc_start(line - 1)).wrapping_add(1) });
        }
        let line = self.loc_line(s);
        i64::from(match place {
            Place::Line => line,
            // The line holding its end offset: a span ending with a terminator ends on the line after it.
            Place::EndLine => self.loc_line(e),
            _ => s.wrapping_sub(self.loc_start(line - 1)),
        })
    }

    /// The `child` row of the first numbered node from tape node `c` on, `c` and each sibling after it within
    /// `end`, or `NO_ROW`.
    fn numbered_from(&self, mut c: u32, end: u32) -> u32 {
        loop {
            if c == NO_ROW {
                return NO_ROW;
            }
            if self.is(0, c) {
                return self.rank(c).wrapping_sub(1);
            }
            c = self.tape.next_in(c, end);
        }
    }
    /// `view_child_first(l)`: the `child` row of numbered node `l`'s first child, or `NO_ROW`.
    #[inline]
    pub fn child_first(&self, l: u32) -> u32 {
        let x = self.tape_node(l);
        self.numbered_from(self.tape.first_child(x), self.tape.end_of(x))
    }
    /// `view_child_next(i)`: the `child` row of the next child of the parent of `child` row `i`, or `NO_ROW`.
    #[inline]
    pub fn child_next(&self, i: u32) -> u32 {
        let x = self.tape_node(i + 1);
        let p = self.tape.parent(x);
        if p == NO_ROW {
            return NO_ROW;
        }
        let end = self.tape.end_of(p);
        self.numbered_from(self.tape.next_in(x, end), end)
    }
    /// `view_number(low)`: the numbered node whose id's low 32 bits are `low`, or `NO_ROW`.
    #[inline]
    pub fn number(&self, low: u32) -> u32 {
        let l = if self.entry.preorder == rerail_clif::view_reads::IDS_TAPE {
            if self.is(0, low) { self.rank(low) } else { NO_ROW }
        } else if self.entry.preorder != 0 {
            low
        } else {
            // SAFETY: as `tape_node`.
            match unsafe { self.numbering().number_of(low) } {
                Some((0, number)) => number,
                _ => NO_ROW,
            }
        };
        if l < self.entry.nodes { l } else { NO_ROW }
    }
    /// `view_text_span(l)`: numbered node `l`'s bytes' range in its document's own source, `start << 32 | end`.
    #[inline]
    pub fn text_span(&self, l: u32) -> u64 {
        let x = self.tape_node(l);
        u64::from(self.tape.start(x)) << 32 | u64::from(self.tape.end(x))
    }
    /// `view_line_span(low)`: the line whose span's id has low bits `low` (`LINE_SLOT << INDEX_BITS | l`: line `l`
    /// from 1, 0 the whole source) as its bytes in the document's own source, `start << 32 | stop`, `stop` one past
    /// its terminator's last byte, or the source's end on its last line and on line 0; `NO_SPAN` for another id, a
    /// guest's (its lines are its host's) or a line past its last.
    #[inline]
    pub fn line_span(&self, low: u32) -> u64 {
        let l = low & ((1 << INDEX_BITS) - 1);
        let count = self.loc_count();
        if low >> INDEX_BITS != rerail_clif::ids::LINE_SLOT || self.entry.own_text == 0 || l > count {
            return NO_SPAN;
        }
        let len = self.entry.source_len;
        if l == 0 {
            return u64::from(len);
        }
        let stop = if l >= count { len } else { self.loc_start(l) };
        u64::from(self.loc_start(l - 1)) << 32 | u64::from(stop)
    }
    /// `view_line_terminated(low)`: whether that line ends with a terminator (from 1, before the last).
    #[inline]
    pub fn line_terminated(&self, low: u32) -> bool {
        let l = low & ((1 << INDEX_BITS) - 1);
        low >> INDEX_BITS == rerail_clif::ids::LINE_SLOT && l != 0 && l < self.loc_count()
    }
    /// The text of the line whose span's id has low bits `low` (`line_span`), read where the source lies, its
    /// terminator's length off the source's bytes where it has one (`lines_terminator_length`), as
    /// `LineTable::text_of` gives it.
    pub fn line_text(&self, low: u32) -> Option<&'a str> {
        let span = self.line_span(low);
        if span == NO_SPAN {
            return None;
        }
        let (start, mut stop) = ((span >> 32) as usize, span as u32 as usize);
        if self.line_terminated(low) {
            stop -= crate::newlines::length_ending_at(self.source, stop - 1);
        }
        std::str::from_utf8(self.source.get(start..stop)?).ok()
    }
    /// Numbered node `l`'s text, read where its source lies (`text_span`): none where the range is not the source's
    /// text (a tape and a source that disagree).
    pub fn text(&self, l: u32) -> Option<&'a str> {
        let span = self.text_span(l);
        std::str::from_utf8(self.source.get((span >> 32) as usize..span as u32 as usize)?).ok()
    }
}

/// The 1-based line holding byte `at` (`lines_line`: `LineTable::line` + 1), by a line index (`newlines`, decision
/// 0612) at the start of `index`.
#[inline]
pub fn lines_line(index: &[u8], at: u32) -> u32 {
    crate::newlines::line(index, at) + 1
}
