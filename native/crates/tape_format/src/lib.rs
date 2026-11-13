//! The engine's tape (v9np, decision 0025; format 11) as bytes, and a writer a parser writes it
//! with as it parses (decision 0140).
//!
//! An image is a header, then five columns, each padded to `COLUMN_ALIGN` bytes, then the code
//! table and the alias table:
//!
//! - header: magic, language tag, the three column widths, version, source length, source
//!   digest (SHA-256), and five lengths: nodes, codes, the code table's bytes, aliases, the
//!   alias table's bytes;
//! - `code`: each node's code id, 2 bytes, codes numbered by first use;
//! - `size`: each node's subtree, itself counted (width 0);
//! - `base`: each 16 nodes' least start, 4 bytes;
//! - `off`: each node's start less its block's base (width 1);
//! - `tail`: each node's end less its last descendant's start (width 2; `TAIL_FROM_START`:
//!   less its own start);
//! - a code: kind and field (`kf`, kind in the low 10 bits), flags, alias, supertype classes.
//!
//! The engine reads and writes images (`code_rule_dsl::tape`); its constants are these.

use sha2::{Digest, Sha256};

pub const MAGIC: &[u8; 8] = b"TMPTAP9N";
pub const FORMAT_VERSION: u32 = 11;
/// Magic 8, language 1, widths 3, version 4, source length 8, digest 32, five lengths 40.
pub const HEADER_BYTES: usize = 56 + 5 * 8;
pub const COLUMN_ALIGN: usize = 16;
/// Nodes a `base` covers.
pub const CHECKPOINT_STRIDE: usize = 16;
/// `widths[2]`'s high bit: a tail is measured from the node's own start.
pub const TAIL_FROM_START: u8 = 0x80;
pub const CODE_EXTRA: u8 = 1;
pub const CODE_ERROR: u8 = 2;
pub const CODE_MISSING: u8 = 4;
pub const KIND_LIMIT: usize = 1024;
pub const FIELD_LIMIT: usize = 64;
/// No alias.
pub const SENTINEL: u16 = u16::MAX;
/// ERROR's `kf`.
pub const ERROR_CODE: u16 = 0;
/// The `.rr` dialect's language tag (`languages/rr.json`'s `tapeTag`).
pub const RR_TAG: u8 = 13;

/// `length` rounded up to `COLUMN_ALIGN`.
pub const fn padded(length: usize) -> usize {
    length.div_ceil(COLUMN_ALIGN) * COLUMN_ALIGN
}

/// The narrowest width (2 or 4 bytes) that holds `max`.
pub fn width_for(max: u32) -> u8 {
    if max <= 0xffff { 2 } else { 4 }
}

/// The widths a writer writes at (`Writer::new`): `size`, `off` and `tail` in bytes (2 or 4),
/// and whether tails are from the node's own start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Widths {
    pub size: u8,
    pub off: u8,
    pub tail: u8,
    pub from_start: bool,
}

impl Widths {
    /// The guess for a source of `len` bytes (decision 0140). An offset and a tail are at most
    /// the source's length, and a `.rr` file has fewer nodes than bytes: below 64 KiB every
    /// column fits 2 bytes, so nearly every file is written once. Above it, offsets and tails
    /// stay 2 (they are short distances), and sizes take 4 from `SIZE_GUESS_BYTES` on.
    pub fn guess(len: usize) -> Self {
        Widths { size: if len < SIZE_GUESS_BYTES { 2 } else { 4 }, off: 2, tail: 2, from_start: false }
    }
}

/// From this many source bytes on, `Widths::guess` writes sizes at 4 bytes. Calibrated on the
/// repository's 1,362 `.rr` files (`rr_syntax`'s `widths_guess_calibration`, 2026-11-13): the
/// densest has 0.334 nodes a byte, the densest of 64 KiB or more 0.205 (42,081 nodes in
/// 205,423 bytes); none needs 4. At 0.205 a file has 65,535 nodes at about 320,000 bytes. With
/// this bound no file is parsed twice; at 160,000, six were.
pub const SIZE_GUESS_BYTES: usize = 320_000;

const NO_CODE: u16 = u16::MAX;

/// A tape written as a parser parses: each node's columns at their final widths, at its place,
/// as it is made. The parser gives each node in preorder (`push`), its end and so its size at
/// its close (`close`), and edits only the nodes it made last: a code changed (`set_key`), a
/// node put before the ones from a place on (`insert`), the nodes from a place on taken back
/// (`truncate`). Each edit rewrites only the nodes from its place on. At the end (`finish`) the
/// columns are moved into place after the code column, and the header and code table written.
///
/// A node's key is its code: `kf | flags << 16`.
pub struct Writer {
    /// The header, then the code column: the image's start, as it will be. Reserved for the
    /// whole image at `cap` nodes, so that `finish` appends the columns in place.
    out: Vec<u8>,
    /// The columns, each `cap` cells at its width; the first `n` are the nodes'.
    size: Vec<u8>,
    off: Vec<u8>,
    tail: Vec<u8>,
    base: Vec<u32>,
    /// Each node's start and key: the parser's working state, what an edit recomputes from.
    starts: Vec<u32>,
    keys: Vec<u32>,
    /// The nodes, and the nodes the cells have room for.
    n: usize,
    cap: usize,
    /// The codes by id: each key and the node that first used it.
    codes: Vec<(u32, u32)>,
    /// The nodes before this have their codes written; the rest only their keys.
    coded: usize,
    id_of: Vec<u16>,
    widths: Widths,
    /// The nodes whose offset or tail needs 4 bytes, or that end before their last descendant
    /// starts (`OFF_WIDE`, `TAIL_WIDE`, `EARLY`): few, kept exact through every edit, so that
    /// `finish` knows the narrowest widths without reading the columns.
    marks: Vec<(u32, u8)>,
    tag: u8,
}

const OFF_WIDE: u8 = 0;
const TAIL_WIDE: u8 = 1;
const EARLY: u8 = 2;

/// Code table bytes reserved for at first: room for 256 codes.
const CODES_RESERVED: usize = 6 * 256;

thread_local! {
    /// The code ids by key, kept between a thread's tapes: each tape clears what it set.
    static IDS: std::cell::RefCell<Vec<u16>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The widths a tape needs, where the writer's did not hold its values: parse again at these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redo(pub Widths);

/// An image's bytes at `nodes` nodes and `widths`, with the code table reserved.
fn image_bytes(nodes: usize, w: Widths) -> usize {
    HEADER_BYTES
        + padded(2 * nodes)
        + padded(usize::from(w.size) * nodes)
        + padded(4 * nodes.div_ceil(CHECKPOINT_STRIDE))
        + padded(usize::from(w.off) * nodes)
        + padded(usize::from(w.tail) * nodes)
        + CODES_RESERVED
}

impl Writer {
    /// A writer for about `nodes` nodes (more grow it) at `widths`.
    pub fn new(tag: u8, widths: Widths, nodes: usize) -> Self {
        let cap = nodes.max(CHECKPOINT_STRIDE);
        let mut out = Vec::with_capacity(image_bytes(cap, widths));
        out.resize(HEADER_BYTES + 2 * cap, 0);
        let cells = |w: u8| vec![0u8; cap * usize::from(w)];
        Writer {
            out,
            size: cells(widths.size),
            off: cells(widths.off),
            tail: cells(widths.tail),
            base: Vec::with_capacity(cap / CHECKPOINT_STRIDE + 1),
            starts: vec![0; cap],
            keys: vec![0; cap],
            n: 0,
            cap,
            codes: Vec::new(),
            coded: 0,
            id_of: IDS.with(|ids| {
                let mut ids = std::mem::take(&mut *ids.borrow_mut());
                ids.resize(1 << 19, NO_CODE);
                ids
            }),
            widths,
            marks: Vec::new(),
            tag,
        }
    }

    /// Room for twice the nodes: every column at once.
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let cap = 2 * self.cap;
        let w = self.widths;
        self.out.reserve_exact(image_bytes(cap, w) - self.out.len());
        self.out.resize(HEADER_BYTES + 2 * cap, 0);
        self.size.resize(cap * usize::from(w.size), 0);
        self.off.resize(cap * usize::from(w.off), 0);
        self.tail.resize(cap * usize::from(w.tail), 0);
        self.starts.resize(cap, 0);
        self.keys.resize(cap, 0);
        self.cap = cap;
    }

    /// Whether node `node` has mark `what`, as `on` says.
    #[inline]
    fn mark(&mut self, node: usize, what: u8, on: bool) {
        let at = self.marks.iter().position(|&m| m == (node as u32, what));
        match (on, at) {
            (true, None) => self.marks.push((node as u32, what)),
            (false, Some(k)) => {
                self.marks.swap_remove(k);
            }
            _ => {}
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    #[inline]
    pub fn key(&self, node: usize) -> u32 {
        self.keys[node]
    }

    #[inline]
    pub fn start(&self, node: usize) -> u32 {
        self.starts[node]
    }

    /// Cell `at` of a column of `width`-byte cells: a 2- or 4-byte store.
    #[inline(always)]
    fn put(column: &mut [u8], at: usize, width: u8, value: u32) {
        if width == 2 {
            column[2 * at..2 * at + 2].copy_from_slice(&(value as u16).to_le_bytes());
        } else {
            column[4 * at..4 * at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }

    #[inline(always)]
    fn set_code(&mut self, node: usize, id: u16) {
        let at = HEADER_BYTES + 2 * node;
        self.out[at..at + 2].copy_from_slice(&id.to_le_bytes());
    }

    /// The id of `key`, numbered by first use: new at `node` if no node before used it.
    #[inline]
    fn id(&mut self, key: u32, node: usize) -> u16 {
        match self.id_of[key as usize] {
            NO_CODE => {
                let id = self.codes.len() as u16;
                self.codes.push((key, node as u32));
                self.id_of[key as usize] = id;
                id
            }
            id => id,
        }
    }

    /// The nodes from `node` on have no code yet: the codes first used there are dropped.
    #[inline]
    fn uncode_from(&mut self, node: usize) {
        if node >= self.coded {
            return;
        }
        while let Some(&(key, first)) = self.codes.last() {
            if (first as usize) < node {
                break;
            }
            self.id_of[key as usize] = NO_CODE;
            self.codes.pop();
        }
        self.coded = node;
    }

    /// The nodes so far are final (a statement parsed or recovered): their codes, numbered by
    /// first use, written to the code column.
    pub fn commit(&mut self) {
        for n in self.coded..self.n {
            let id = self.id(self.keys[n], n);
            self.set_code(n, id);
        }
        self.coded = self.n;
    }

    /// Block `b`'s base and its nodes' offsets, from their starts.
    fn block(&mut self, b: usize) {
        let from = b * CHECKPOINT_STRIDE;
        let to = (from + CHECKPOINT_STRIDE).min(self.n);
        let base = self.starts[from..to].iter().copied().min().expect("a node in the block");
        if b == self.base.len() {
            self.base.push(base);
        } else {
            self.base[b] = base;
        }
        for n in from..to {
            let off = self.starts[n] - base;
            if off > 0xffff || !self.marks.is_empty() {
                self.mark(n, OFF_WIDE, off > 0xffff);
            }
            Self::put(&mut self.off, n, self.widths.off, off);
        }
    }

    /// A node at the end, in preorder: its key and start. Its size is 1 and its tail 0 until
    /// its close; its code is written when it is committed.
    #[inline]
    pub fn push(&mut self, key: u32, start: u32) -> usize {
        let node = self.n;
        if node == self.cap {
            self.grow();
        }
        self.n = node + 1;
        self.keys[node] = key;
        self.starts[node] = start;
        let w = self.widths;
        Self::put(&mut self.size, node, w.size, 1);
        Self::put(&mut self.tail, node, w.tail, 0);
        match self.base.get(node / CHECKPOINT_STRIDE) {
            Some(&base) if base <= start => {
                let off = start - base;
                if off > 0xffff {
                    self.mark(node, OFF_WIDE, true);
                }
                Self::put(&mut self.off, node, w.off, off);
            }
            _ => self.block(node / CHECKPOINT_STRIDE),
        }
        node
    }

    /// `node`'s end: its subtree is the nodes from it on, and its tail from its last one.
    #[inline]
    pub fn close(&mut self, node: usize, end: u32) {
        let size = (self.n - node) as u32;
        let last = self.starts[self.n - 1];
        let from = if self.widths.from_start { self.starts[node] } else { last };
        let tail = end.wrapping_sub(from);
        let (early, wide) = (end < last, tail > 0xffff && end >= from);
        if early || wide || !self.marks.is_empty() {
            self.mark(node, EARLY, early);
            self.mark(node, TAIL_WIDE, wide);
        }
        Self::put(&mut self.size, node, self.widths.size, size);
        Self::put(&mut self.tail, node, self.widths.tail, tail);
    }

    /// `node`'s key changed (a field set after it was made, a kind found later).
    #[inline]
    pub fn set_key(&mut self, node: usize, key: u32) {
        self.uncode_from(node);
        self.keys[node] = key;
    }

    /// A node at `at`, before the nodes from there on, which move up one: a node found after
    /// the first of its children (an operator after its left operand). It is open: its close
    /// gives its size. The code column from `at` on is written again at the commit.
    pub fn insert(&mut self, at: usize, key: u32, start: u32) {
        if at == self.n {
            self.push(key, start);
            return;
        }
        if self.n == self.cap {
            self.grow();
        }
        let (n, ws) = (self.n, self.widths);
        let shift = |column: &mut [u8], width: u8| {
            let w = usize::from(width);
            column.copy_within(at * w..n * w, at * w + w);
        };
        shift(&mut self.size, ws.size);
        shift(&mut self.tail, ws.tail);
        self.keys.copy_within(at..n, at + 1);
        self.starts.copy_within(at..n, at + 1);
        self.n = n + 1;
        self.keys[at] = key;
        self.starts[at] = start;
        Self::put(&mut self.size, at, ws.size, 1);
        Self::put(&mut self.tail, at, ws.tail, 0);
        self.uncode_from(at);
        for m in self.marks.iter_mut().filter(|m| m.0 as usize >= at) {
            m.0 += 1;
        }
        // The blocks from `at`'s on.
        for b in at / CHECKPOINT_STRIDE..self.n.div_ceil(CHECKPOINT_STRIDE) {
            self.block(b);
        }
    }

    /// The nodes from `len` on taken back (a parse that did not hold).
    pub fn truncate(&mut self, len: usize) {
        if len >= self.n {
            return;
        }
        self.n = len;
        self.uncode_from(len);
        self.marks.retain(|m| (m.0 as usize) < len);
        let blocks = len.div_ceil(CHECKPOINT_STRIDE);
        self.base.truncate(blocks);
        if len % CHECKPOINT_STRIDE != 0 {
            self.block(blocks - 1);
        }
    }

    /// The image of the source `src`: the columns moved into place after the code column, then
    /// the code table, and the header. `Redo` when a column's values do not fit its width, or a
    /// tail from its last descendant is negative: the widths to parse again at.
    pub fn finish(mut self, src: &[u8]) -> Result<Vec<u8>, Redo> {
        self.commit();
        let has = |what: u8| self.marks.iter().any(|m| m.1 == what);
        let from_start = has(EARLY);
        let want = Widths {
            size: width_for(self.n as u32),
            off: if has(OFF_WIDE) { 4 } else { 2 },
            // Tails measured from the other place say nothing of the width: 2, the guess.
            tail: if from_start == self.widths.from_start && has(TAIL_WIDE) { 4 } else { 2 },
            from_start,
        };
        if want != self.widths {
            return Err(Redo(want));
        }
        let w = self.widths;
        let nodes = self.n;
        let code_bytes = 6 * self.codes.len();
        let pad = |out: &mut Vec<u8>| out.resize(padded(out.len()), 0);
        let mut out = std::mem::take(&mut self.out);
        out.truncate(HEADER_BYTES + 2 * nodes);
        // Reserved at `new` or `grow` for these nodes and 256 codes: more codes grow it.
        out.reserve(image_bytes(nodes, w) - CODES_RESERVED + code_bytes - out.len());
        pad(&mut out);
        out.extend_from_slice(&self.size[..nodes * usize::from(w.size)]);
        pad(&mut out);
        for b in &self.base {
            out.extend_from_slice(&b.to_le_bytes());
        }
        pad(&mut out);
        out.extend_from_slice(&self.off[..nodes * usize::from(w.off)]);
        pad(&mut out);
        out.extend_from_slice(&self.tail[..nodes * usize::from(w.tail)]);
        pad(&mut out);
        for &(key, _) in &self.codes {
            out.extend_from_slice(&((key & 0xffff) as u16).to_le_bytes());
            out.push((key >> 16) as u8);
            out.extend_from_slice(&SENTINEL.to_le_bytes());
            out.push(0);
        }
        let mut h = Vec::with_capacity(HEADER_BYTES);
        h.extend_from_slice(MAGIC);
        h.push(self.tag);
        h.extend_from_slice(&[w.size, w.off, w.tail | if w.from_start { TAIL_FROM_START } else { 0 }]);
        h.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
        h.extend_from_slice(&(src.len() as u64).to_le_bytes());
        h.extend_from_slice(&Sha256::digest(src));
        for length in [nodes, self.codes.len(), code_bytes, 0, 0] {
            h.extend_from_slice(&(length as u64).to_le_bytes());
        }
        out[..HEADER_BYTES].copy_from_slice(&h);
        Ok(out)
    }
}

impl Drop for Writer {
    /// The code ids back to the thread, cleared.
    fn drop(&mut self) {
        for &(key, _) in &self.codes {
            self.id_of[key as usize] = NO_CODE;
        }
        let ids = std::mem::take(&mut self.id_of);
        IDS.with(|t| *t.borrow_mut() = ids);
    }
}

/// Where byte `at` of image `bytes` lies, as text: the header, a column's node, or a table.
pub fn place(bytes: &[u8], at: usize) -> String {
    if at < HEADER_BYTES || bytes.len() < HEADER_BYTES {
        return format!("the header (byte {at})");
    }
    let n = u64::from_le_bytes(bytes[56..64].try_into().expect("8 bytes")) as usize;
    let w = [usize::from(bytes[9]), usize::from(bytes[10]), usize::from(bytes[11] & !TAIL_FROM_START)];
    let mut from = HEADER_BYTES;
    for (name, width, count) in [("code", 2, n), ("size", w[0], n), ("base", 4, n.div_ceil(CHECKPOINT_STRIDE)), ("off", w[1], n), ("tail", w[2], n)] {
        let to = padded(from + width * count);
        if at < to {
            return format!("{name} of node {}", (at - from) / width.max(1));
        }
        from = to;
    }
    format!("the code table (byte {})", at - from)
}
