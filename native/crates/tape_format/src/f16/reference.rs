//! The reference writer of format 16 and 16z (decision 0434 §2), written by hand: the tests' oracle (R16), and the
//! lab's, which turns on the `reference` feature. The engine's writer is the ops in CLIF (`f16::Writer`), which
//! write its bytes.

use std::collections::HashMap;
use super::*;

/// An open node: its index, its children so far and its last child (`NONE`: none), its start.
#[derive(Clone, Copy, Debug)]
struct Open {
    node: u32,
    children: u32,
    last: u32,
    start: u64,
}

/// 16z's output as blocks become final (decision 0434 §3): the header's room then the payload, each piece's start
/// in the payload, its length and its kind (by piece: `PIECES` a block, in block order, whatever order the blocks
/// finish in), the checkpoints (raw, in node order), and each block's side-list parts' lengths.
struct ZOut {
    out: crate::Vector<u8>,
    start: Vec<u32>,
    len: Vec<u32>,
    kinds: Vec<u8>,
    ckpt: Vec<u8>,
    part_len: [Vec<u32>; 4],
    /// The pieces stored so far, by their bytes: what a repeat names.
    seen: HashMap<Vec<u8>, u32>,
    piece: Vec<u8>,
}

impl ZOut {
    fn new() -> Self {
        ZOut { out: crate::Vector::from([0u8; HEAD16]), start: Vec::new(), len: Vec::new(), kinds: Vec::new(), ckpt: Vec::new(), part_len: Default::default(), seen: HashMap::new(), piece: Vec::new() }
    }

    /// Block `b`'s four pieces from its columns (codes, slots, gaps, lengths, at its rows) and its side lists'
    /// parts (`part_into`), and its checkpoints: each piece stored as its bytes, as one LZ4 block (HC 4) when that
    /// shrinks it by a tenth, or as a repeat of an earlier piece of the tape with the same bytes (0156 §2.1).
    fn block(&mut self, b: usize, columns: [&[u8]; 4], parts: [&[u8]; 4], ckpt: &[u8], cw: usize) {
        let pieces = PIECES * (b + 1);
        if self.kinds.len() < pieces {
            self.start.resize(pieces, 0);
            self.len.resize(pieces, 0);
            self.kinds.resize(pieces, RAW);
            for k in 0..4 {
                self.part_len[k].resize(b + 1, 0);
            }
        }
        let c0 = b * Z / K * cw;
        if self.ckpt.len() < c0 + ckpt.len() {
            self.ckpt.resize(c0 + ckpt.len(), 0);
        }
        self.ckpt[c0..c0 + ckpt.len()].copy_from_slice(ckpt);
        let mut piece = std::mem::take(&mut self.piece);
        for q in 0..PIECES {
            piece.clear();
            piece.extend_from_slice(columns[q]);
            piece.extend_from_slice(parts[q]);
            self.part_len[q][b] = parts[q].len() as u32;
            let j = PIECES * b + q;
            let at = self.out.len() - HEAD16;
            let kind = match self.seen.get(piece.as_slice()) {
                Some(&earlier) => {
                    self.out.extend_from_slice(&earlier.to_le_bytes());
                    REPEAT
                }
                None => {
                    if piece.len() >= REPEAT_MIN {
                        self.seen.insert(piece.clone(), j as u32);
                    }
                    let z = if piece.len() >= LZ4_MIN { lz4::compress(&piece) } else { Vec::new() };
                    if !z.is_empty() && 10 * z.len() <= 9 * piece.len() {
                        self.out.extend_from_slice(&z);
                        LZ4
                    } else {
                        self.out.extend_from_slice(&piece);
                        RAW
                    }
                }
            };
            self.start[j] = at as u32;
            self.len[j] = (self.out.len() - HEAD16 - at) as u32;
            self.kinds[j] = kind;
        }
        self.piece = piece;
    }

    /// The 16z tape: the payload, then the part starts, the checkpoints, the newline index and the piece index, the
    /// header written last.
    fn finish(mut self, mut p: Placement, tag: u8, mark: u8, source_len: u64, lines: &[u8]) -> Result<Tape16Z, String> {
        let count = p.block_count();
        let sides: [usize; 4] = std::array::from_fn(|k| self.part_len[k].iter().map(|&l| l as usize).sum());
        p = Placement::of(HEAD16, p.n, p.cw, p.widths, p.vw, sides, lines.len(), p.newlines, source_len);
        let o = &mut self.out;
        o.resize(o.len().next_multiple_of(4), 0);
        let parts = o.len();
        for k in 0..4 {
            let mut at = 0u32;
            o.extend_from_slice(&at.to_le_bytes());
            for b in 0..count {
                at += self.part_len[k][b];
                o.extend_from_slice(&at.to_le_bytes());
            }
        }
        let ckpt = o.len();
        o.extend_from_slice(&self.ckpt);
        o.extend_from_slice(&[0; 8]);
        let lines_at = o.len();
        o.extend_from_slice(lines);
        o.extend_from_slice(&[0; 8]);
        o.resize(o.len().next_multiple_of(4), 0);
        let starts = o.len();
        for s in &self.start {
            o.extend_from_slice(&s.to_le_bytes());
        }
        let lens = o.len();
        for l in &self.len {
            o.extend_from_slice(&l.to_le_bytes());
        }
        let kinds = o.len();
        o.extend_from_slice(&self.kinds);
        let end = o.len();
        p.head_into(o, MAGIC16Z, tag, mark, source_len)?;
        for (t, at) in [parts, ckpt, lines_at, starts, lens, kinds, end].into_iter().enumerate() {
            put_u32(o, head::Z + 4 * t, at)?;
        }
        Tape16Z::from_bytes(std::mem::take(o))
    }
}

/// Format 16 or 16z written in one pass as a parser goes (decision 0434 §2): its ops (`open`, `close`, `leaf`,
/// `commit`, `finish`) write each cell once, at its final place in its block. A node's code, gap and checkpoint (a
/// `K`-th node's) as it opens; its length, and an inner node's slot (`size << 1`), at its close; a leaf's slot as
/// the node after it opens (the distance to that node's previous sibling, or on a `K`-th sibling to its parent).
///
/// A block is final once every node in it is closed, the node after its last has opened, and the commit frontier
/// (`commit`) is past it: no op changes it after. Then its presence is taken, and in 16z its pieces are compressed
/// and its room given to the next block; a 16 tape's block stays where it is. That is the one place 16 and 16z
/// differ, once a block: the ops are the same. `finish` moves the last block's columns down to its rows and appends
/// the tail (the side lists, from the entries each block kept, their part starts and the newline index).
///
/// The side lists' value widths are known before the first node: a slot 4 bytes (`MAX_NODES`), a gap or a length
/// the source's length's. A node that breaks the tape's bounds (a start before the one before it, a span past the
/// source, a node outside the root) makes `finish` an `Err`.
pub struct Writer<'l> {
    lang: &'l Lang,
    tag: u8,
    source_len: u64,
    /// The source's newline index (laid in the tail) and its newlines.
    lines: crate::Vector<u8>,
    newlines: u32,
    w: [usize; 3],
    cw: usize,
    vw: [usize; 4],
    esc: [u64; 3],
    /// Bytes before each column in a full block (`Z * before`).
    at: [usize; COLUMNS],
    stride: usize,
    /// 16: the tape (the header's room, then each block at its place, at `Z` rows until `finish`); 16z: the rooms
    /// of the blocks not yet final.
    buf: crate::Vector<u8>,
    /// Each block's start in `buf`.
    pub(super) base: Vec<usize>,
    /// 16z: the rooms of blocks compressed, for the next blocks.
    pub(super) free: Vec<usize>,
    n: usize,
    open: Vec<Open>,
    prev_start: u64,
    frontier: usize,
    /// Each block's side-list entries (its row, the value), by list, in row order.
    side: Vec<[Vec<(u16, u64)>; 4]>,
    pub(super) done: Vec<bool>,
    presence: Presence,
    keep_blocks: bool,
    z: Option<ZOut>,
    /// 16z: a block's side-list parts as laid.
    laid: [crate::Vector<u8>; 4],
    bad: Option<String>,
}

impl<'l> Writer<'l> {
    /// A writer of a document of `source` in `lang`, of about `nodes` nodes, in 16z where `z`, with each block's
    /// presence where `blocks`. The tape's room is reserved for `nodes` nodes, their side lists' usual share and the
    /// source's newline index: more nodes grow it.
    pub fn new(lang: &'l Lang, source: &[u8], terminators: crate::newlines::Terminators, tag: u8, z: bool, blocks: bool, nodes: usize) -> Self {
        let source_len = source.len() as u64;
        let w = lang.widths;
        let cw = bytes_for(source_len);
        let stride = stride_of(w, cw);
        let (kind_words, field_words) = lang.presence_words();
        let lines = crate::newlines::index(source, terminators);
        let newlines = crate::newlines::count(&lines).unwrap_or(0);
        let count = nodes.div_ceil(Z).max(1);
        let tail = 4 * 8 + nodes / 2 + 64 + 16 * (count + 1) + lines.len() + 16;
        let buf = if z { crate::Vector::with_capacity(2 * stride) } else { crate::Vector::with_capacity(HEAD16 + count * stride + 8 + tail) };
        let mut wr = Writer {
            lang,
            tag,
            source_len,
            lines,
            newlines,
            w,
            cw,
            vw: [lang.low_w, 4, bytes_for(source_len), bytes_for(source_len)],
            esc: [mask(w[0]), mask(w[1]), mask(w[2])],
            at: std::array::from_fn(|r| Z * before(w, r)),
            stride,
            buf,
            base: Vec::new(),
            free: Vec::new(),
            n: 0,
            open: Vec::with_capacity(64),
            prev_start: 0,
            frontier: 0,
            side: Vec::new(),
            done: Vec::new(),
            presence: Presence { kind_words, field_words, doc: vec![0; kind_words + field_words], blocks: Vec::new(), faults: Vec::new() },
            keep_blocks: blocks,
            z: z.then(ZOut::new),
            laid: Default::default(),
            bad: None,
        };
        if !z {
            wr.buf.resize(HEAD16, 0);
        }
        wr
    }

    /// The nodes so far.
    pub fn len(&self) -> usize {
        self.n
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    fn fail(&mut self, why: String) {
        if self.bad.is_none() {
            self.bad = Some(why);
        }
    }

    /// Room for block `b`: its place (16), or a free room (16z).
    fn new_block(&mut self, b: usize) {
        debug_assert_eq!(b, self.base.len());
        let at = match self.free.pop() {
            Some(at) => at,
            None => {
                let at = self.buf.len();
                self.buf.resize(at + self.stride, 0);
                at
            }
        };
        self.base.push(at);
        self.side.push(Default::default());
        self.done.push(false);
    }

    /// Value `v` of node `i`'s cell in array `a` (0 slots, 1 gaps, 2 lengths), or its escape and its entry in its
    /// block's side list.
    #[inline(always)]
    fn put(&mut self, a: usize, i: usize, v: u64) {
        let (b, j) = (i / Z, i % Z);
        let list = &mut self.side[b][a + 1];
        let v = if v >= self.esc[a] {
            // Entries come in row order but for an ancestor's late close.
            let at = list.partition_point(|e| usize::from(e.0) < j);
            if list.get(at).is_some_and(|e| usize::from(e.0) == j) {
                list[at].1 = v;
            } else {
                list.insert(at, (j as u16, v));
            }
            self.esc[a]
        } else {
            // A cell written again (`set_start`) drops the entry it had.
            if list.last().is_some_and(|e| usize::from(e.0) >= j) {
                let at = list.partition_point(|e| usize::from(e.0) < j);
                if list.get(at).is_some_and(|e| usize::from(e.0) == j) {
                    list.remove(at);
                }
            }
            v
        };
        let w = self.w[a];
        let at = self.base[b] + self.at[1 + a] + j * w;
        self.buf[at..at + w].copy_from_slice(&v.to_le_bytes()[..w]);
    }

    /// Node `i`'s value in array `a` as written: its cell, or its side-list entry where the cell is the escape.
    fn value(&self, a: usize, i: usize) -> u64 {
        let (b, j) = (i / Z, i % Z);
        let w = self.w[a];
        let at = self.base[b] + self.at[1 + a] + j * w;
        let mut v = [0u8; 8];
        v[..w].copy_from_slice(&self.buf[at..at + w]);
        let v = u64::from_le_bytes(v);
        if v < self.esc[a] {
            return v;
        }
        let list = &self.side[b][a + 1];
        list.get(list.partition_point(|e| usize::from(e.0) < j)).map_or(v, |e| e.1)
    }

    /// Node `i`'s code byte and its side-list entry, for `number`.
    fn put_code(&mut self, i: usize, number: u32) {
        let (b, j) = (i / Z, i % Z);
        let lang = self.lang;
        let list = &mut self.side[b][0];
        self.buf[self.base[b] + j] = if number < lang.cut {
            number as u8
        } else {
            let t = number - lang.cut;
            // Entries come in row order but for a retag.
            let at = list.partition_point(|e| usize::from(e.0) < j);
            list.insert(at, (j as u16, u64::from(t % lang.per)));
            (lang.cut + t / lang.per) as u8
        };
    }

    /// Whether node `i` may still change: it is written, and its block not final.
    fn changeable(&mut self, i: usize, what: &str) -> bool {
        let ok = i < self.n && !self.done[i / Z];
        if !ok {
            self.fail(format!("node {i}'s {what} changed after its block was final"));
        }
        ok
    }

    /// Node `i`'s code given again (a kind or field found after it opened): `number` in the language's table. Its
    /// block is not final (decision 0434 §2: an edit that reaches a node before the window).
    pub fn retag(&mut self, i: usize, number: u32) {
        if !self.changeable(i, "code") {
            return;
        }
        let (b, j) = (i / Z, i % Z);
        let list = &mut self.side[b][0];
        let at = list.partition_point(|e| usize::from(e.0) < j);
        if list.get(at).is_some_and(|e| usize::from(e.0) == j) {
            list.remove(at);
        }
        self.put_code(i, number);
    }

    /// Open node `i`'s start given again (a node opened before the child whose start it takes): its gap, its
    /// checkpoint and the next node's gap written again. It stays between its neighbours' starts, and its block and
    /// the next node's are not final.
    pub fn set_start(&mut self, i: usize, start: u64) {
        let Some(k) = self.open.iter().rposition(|o| o.node as usize == i) else {
            self.fail(format!("node {i}'s start set when it is not open"));
            return;
        };
        if !self.changeable(i, "start") || (i + 1 < self.n && !self.changeable(i + 1, "start")) {
            return;
        }
        let old = self.open[k].start;
        let prev = if i == 0 { 0 } else { old - self.value(1, i) };
        let next = if i + 1 < self.n { old + self.value(1, i + 1) } else { self.source_len };
        if start < prev || start > next {
            self.fail(format!("node {i}'s start {start} breaks the tape's bounds"));
            return;
        }
        if i > 0 {
            self.put(1, i, start - prev);
        }
        if i + 1 < self.n {
            self.put(1, i + 1, next - start);
        } else {
            self.prev_start = start;
        }
        let (b, j) = (i / Z, i % Z);
        if j.is_multiple_of(K) {
            let at = self.base[b] + self.at[4] + j / K * self.cw;
            let cw = self.cw;
            self.buf[at..at + cw].copy_from_slice(&start.to_le_bytes()[..cw]);
        }
        self.open[k].start = start;
    }

    /// A node at the end, in preorder, open: its code's number in the language's table, and its start.
    pub fn open(&mut self, number: u32, start: u64) -> usize {
        let i = self.n;
        if start < self.prev_start || start > self.source_len || i + 1 >= MAX_NODES {
            self.fail(format!("node {i}'s start {start} breaks the tape's bounds"));
        }
        if i > 0 && self.open.is_empty() {
            self.fail(format!("node {i} lies outside the root's subtree"));
        }
        let (b, j) = (i / Z, i % Z);
        if j == 0 {
            self.new_block(b);
        }
        // The node before is a leaf unless it is the parent: its slot describes this node.
        if let Some(top) = self.open.last_mut() {
            let (idx, prev, p) = (top.children, top.last, top.node as usize);
            top.children = idx + 1;
            top.last = i as u32;
            if i - 1 != p {
                let v = if (idx as usize).is_multiple_of(K) { ((i - p) as u64) << 2 | 3 } else { ((i - prev as usize) as u64) << 2 | 1 };
                self.put(0, i - 1, v);
            }
        }
        let base = self.base[b];
        self.put_code(i, number);
        self.put(1, i, if i == 0 { 0 } else { start.wrapping_sub(self.prev_start) });
        if j.is_multiple_of(K) {
            let at = base + self.at[4] + j / K * self.cw;
            let cw = self.cw;
            self.buf[at..at + cw].copy_from_slice(&start.to_le_bytes()[..cw]);
        }
        self.prev_start = start;
        self.n = i + 1;
        self.open.push(Open { node: i as u32, children: 0, last: NONE, start });
        if j == 0 && b > 0 {
            // The block before has its last slot now.
            self.settle(b - 1);
        }
        i
    }

    /// The innermost open node's end: its length, and an inner node's slot, written.
    pub fn close(&mut self, end: u64) {
        let Some(o) = self.open.pop() else {
            self.fail("a close with no node open".into());
            return;
        };
        if end < o.start || end > self.source_len {
            self.fail(format!("node {}'s end {end} breaks the tape's bounds", o.node));
        }
        let i = o.node as usize;
        self.put(2, i, end.wrapping_sub(o.start));
        let size = self.n - i;
        if size > 1 {
            self.put(0, i, (size as u64) << 1);
        }
        self.settle(i / Z);
    }

    /// A closed node with no children.
    pub fn leaf(&mut self, number: u32, start: u64, end: u64) -> usize {
        let i = self.open(number, start);
        self.close(end);
        i
    }

    /// The nodes so far are final but for the open ones: no edit reaches before them (decision 0434 §2).
    pub fn commit(&mut self) {
        let was = self.frontier / Z;
        self.frontier = self.n;
        for b in was..self.n / Z {
            self.settle(b);
        }
    }

    /// Block `b` made final, if it is: every node in it closed, the node after its last opened, the frontier past it.
    fn settle(&mut self, b: usize) {
        let end = (b + 1) * Z;
        if self.done[b] || self.n <= end || self.frontier < end {
            return;
        }
        // The open nodes' indices ascend: the last below the block's end is the one that may be in it.
        let k = self.open.partition_point(|o| (o.node as usize) < end);
        if k > 0 && self.open[k - 1].node as usize >= b * Z {
            return;
        }
        self.finalize(b, Z);
    }

    /// Block `b`, of `rows` rows, is final: its presence taken, and in 16z its pieces compressed and its room freed.
    fn finalize(&mut self, b: usize, rows: usize) {
        self.done[b] = true;
        let base = self.base[b];
        let lang = self.lang;
        let words = self.presence.words();
        let mut block = vec![0u64; words];
        let codes = &self.buf[base..base + rows];
        let mut low = self.side[b][0].iter().peekable();
        let kw = self.presence.kind_words;
        for (j, &byte) in codes.iter().enumerate() {
            let number = if u32::from(byte) < lang.cut {
                u32::from(byte)
            } else {
                let l = low.next().filter(|e| usize::from(e.0) == j).map_or(0, |e| e.1 as u32);
                lang.cut + (u32::from(byte) - lang.cut) * lang.per + l
            };
            let kf = if (number as usize) < lang.table().len() { let e = lang.table().entry(number); super::presence::kind_word(e.kind, e.field, e.flags) } else { 0 };
            let (kind, field) = ((kf & super::presence::KIND_MASK) as u16, (kf >> 16) as u16);
            block[usize::from(kind) / 64] |= 1 << (kind % 64);
            block[0] |= u64::from(kf & super::presence::FAULT != 0) << crate::ERROR_CODE;
            block[kw + usize::from(field) / 64] |= 1 << (field % 64);
            // An ERROR node or a MISSING one listed (`presence::faults`), beside the fault bit it set.
            if kind == crate::ERROR_CODE || kf & super::presence::FAULT != 0 {
                self.presence.faults.push((b * Z + j) as u32);
            }
        }
        for (d, x) in self.presence.doc.iter_mut().zip(&block) {
            *d |= x;
        }
        if self.keep_blocks {
            if self.presence.blocks.len() < (b + 1) * words {
                self.presence.blocks.resize((b + 1) * words, 0);
            }
            self.presence.blocks[b * words..(b + 1) * words].copy_from_slice(&block);
        }
        if let Some(z) = &mut self.z {
            let col = |r: usize, len: usize| &self.buf[base + self.at[r]..base + self.at[r] + len];
            let columns = [col(0, rows), col(1, rows * self.w[0]), col(2, rows * self.w[1]), col(3, rows * self.w[2])];
            let ckpt = col(4, rows.div_ceil(K) * self.cw);
            for (k, laid) in self.laid.iter_mut().enumerate() {
                laid.clear();
                part_into(laid, &self.side[b][k], rows, self.vw[k]);
            }
            let parts = [self.laid[0].as_slice(), self.laid[1].as_slice(), self.laid[2].as_slice(), self.laid[3].as_slice()];
            z.block(b, columns, parts, ckpt, self.cw);
            self.side[b] = Default::default();
            self.free.push(base);
        }
    }

    /// The tape, and its presence: the last node's slot, the blocks not yet final, then (16) the last block's
    /// columns moved down to its rows and the tail appended, or (16z) the pieces' index and the header.
    pub fn finish(mut self) -> Result<Finished, String> {
        if !self.open.is_empty() {
            self.fail(format!("{} nodes not closed", self.open.len()));
        }
        if let Some(bad) = self.bad.take() {
            return Err(bad);
        }
        let n = self.n;
        if n > 0 {
            // The last node in preorder is a leaf with nothing after it.
            self.put(0, n - 1, 1);
        }
        let (lines, newlines) = (std::mem::take(&mut self.lines), self.newlines);
        let count = n.div_ceil(Z);
        for b in 0..count {
            if !self.done[b] {
                let rows = Z.min(n - b * Z);
                self.finalize(b, rows);
            }
        }
        let mut presence = std::mem::take(&mut self.presence);
        // In preorder: a block made final after a later one listed its faults after the later one's.
        presence.faults.sort_unstable();
        let mark = self.lang.revision;
        let p = Placement::of(HEAD16, n, self.cw, self.w, self.vw, [0; 4], lines.len(), newlines, self.source_len);
        if let Some(z) = self.z.take() {
            let tape = z.finish(p, self.tag, mark, self.source_len, &lines)?;
            let mut out = crate::Vector::from(tape.as_bytes());
            super::presence::lay(&mut out, &presence);
            return Ok(Finished { tape: Written::F16Z(Tape16Z::from_bytes(out)?) });
        }
        // The last block's columns down to its rows.
        if count > 0 {
            let (b, rows) = (count - 1, n - (count - 1) * Z);
            let base = self.base[b];
            for r in 1..COLUMNS {
                let len = if r == 4 { rows.div_ceil(K) * self.cw } else { rows * self.w[r - 1] };
                let from = base + self.at[r];
                self.buf.copy_within(from..from + len, base + rows * before(self.w, r));
            }
        }
        let mut buf = std::mem::take(&mut self.buf);
        buf.truncate(p.blocks_end());
        buf.extend_from_slice(&[0; 8]);
        // The tail: each side list's parts, their starts, the newline index.
        let mut starts: [Vec<u32>; 4] = Default::default();
        for (k, st) in starts.iter_mut().enumerate() {
            let at = buf.len();
            for b in 0..count {
                st.push((buf.len() - at) as u32);
                part_into(&mut buf, &self.side[b][k], Z.min(n - b * Z), self.vw[k]);
            }
            st.push((buf.len() - at) as u32);
            buf.extend_from_slice(&[0; 8]);
        }
        buf.resize(buf.len().next_multiple_of(4), 0);
        for st in &starts {
            for s in st {
                buf.extend_from_slice(&s.to_le_bytes());
            }
        }
        buf.extend_from_slice(&lines);
        buf.extend_from_slice(&[0; 8]);
        let sides = std::array::from_fn(|k| starts[k][count] as usize);
        let p = Placement::of(HEAD16, n, self.cw, self.w, self.vw, sides, lines.len(), newlines, self.source_len);
        debug_assert_eq!(buf.len(), p.end);
        p.head_into(&mut buf, MAGIC16, self.tag, mark, self.source_len)?;
        super::presence::lay(&mut buf, &presence);
        Ok(Finished { tape: Written::F16(Tape16::from_bytes(buf)?) })
    }
}

/// A block's side-list part as stored: for each of its `BLOCK`-node blocks a u16 end, then each entry's node offset
/// in its block, then the values at `vw` bytes. Nothing for a part with no entries.
fn part_into(out: &mut crate::Vector<u8>, e: &[(u16, u64)], nb: usize, vw: usize) {
    if e.is_empty() {
        return;
    }
    let nsub = nb.div_ceil(BLOCK);
    let mut k = 0;
    for b in 0..nsub {
        while k < e.len() && usize::from(e[k].0) / BLOCK == b {
            k += 1;
        }
        out.extend_from_slice(&(k as u16).to_le_bytes());
    }
    out.extend(e.iter().map(|x| (usize::from(x.0) % BLOCK) as u8));
    for x in e {
        out.extend_from_slice(&x.1.to_le_bytes()[..vw]);
    }
}

/// `f16::write_from` by the reference writer.
pub fn write_from<T: Tape>(lang: &Lang, t: &T, number: &[u32], source: &[u8], tag: u8, z: bool) -> Result<Written, String> {
    write_with(Writer::new(lang, source, crate::newlines::Terminators::Lf, tag, z, false, t.len()), t.len(), nodes_of(t, number)).map(|f| f.tape)
}

impl Tape16 {
    /// This tape stored in fixed blocks (16z), each piece compressed alone (no dictionary yet: 0293, not done), its
    /// blocks in order: by the reference's 16z sink (the tests').
    pub fn blocks(&self) -> Tape16Z {
        let mut z = ZOut::new();
        for b in 0..self.p.block_count() {
            let columns = [self.column(b, 0), self.column(b, 1), self.column(b, 2), self.column(b, 3)];
            let part = |k: usize| &self.buf[self.p.side[k] + self.part_start(k, b)..self.p.side[k] + self.part_start(k, b + 1)];
            z.block(b, columns, [part(0), part(1), part(2), part(3)], self.column(b, 4), self.p.cw);
        }
        z.finish(self.p, self.tag(), self.mark, self.source_len, &self.buf[self.p.lines..self.p.end - 8]).expect("a format 16 tape's blocks are a 16z tape")
    }
}
