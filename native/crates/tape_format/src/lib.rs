//! The engine's tape (v9np, decision 0025; format 15) as bytes, and a writer a parser writes it
//! with as it parses (decision 0140).
//!
//! An image is a header, then five columns, each padded to `COLUMN_ALIGN` bytes, then the code
//! table and the alias table:
//!
//! - header: magic, language tag, the three column widths, version, source length, and five
//!   lengths: nodes, codes, the code table's bytes, aliases, the alias table's bytes (`header`);
//! - `code`: each node's code id, 2 bytes, codes numbered by first use;
//! - `size`: each node's subtree, itself counted (width 0);
//! - `base`: each 16 nodes' least start, 4 bytes;
//! - `off`: each node's start less its block's base (width 1);
//! - `tail`: each node's end less its last descendant's start (width 2; `TAIL_FROM_START`:
//!   less its own start);
//! - a code: kind and field (`kf`, kind in the low 10 bits), flags, alias, supertype classes.
//!
//! The engine reads and writes images (`code_rule_dsl::tape`); its constants are these, and a
//! format change is made here, once, for every writer.

pub const MAGIC: &[u8; 8] = b"TMPTAP9N";
/// 10: supertype memberships folded into the codes; 11: columns aligned (`COLUMN_ALIGN`); 12:
/// named-only at rest, no stored source digest (RER-131); 14: the tokens the grammar implies
/// never written (`tokens::TokenGrammar::stored_children`); 15: a keyword or operator left out
/// only where its kind always implies it (`tokens::kept`).
pub const FORMAT_VERSION: u32 = 15;
/// Where an image's header fields are, as byte offsets: the one place a writer or a reader of
/// stored bytes (`resident`, tests) learns the layout, so a format change moves them here.
pub mod header {
    /// The magic, 8 bytes.
    pub const MAGIC: usize = 0;
    /// The language's tape tag, one byte.
    pub const LANGUAGE: usize = 8;
    /// The `size`, `off` and `tail` widths, a byte each (with `TAIL_FROM_START`).
    pub const WIDTHS: usize = 9;
    /// `FORMAT_VERSION`, u32.
    pub const VERSION: usize = 12;
    /// The source's length in bytes, u64.
    pub const SOURCE_LEN: usize = 16;
    /// The node count, u64; then the code count, the code table's bytes, the alias count and
    /// the alias table's bytes.
    pub const NODES: usize = 24;
    pub const CODES: usize = 32;
    pub const CODE_BYTES: usize = 40;
    pub const ALIASES: usize = 48;
    pub const ALIAS_BYTES: usize = 56;
}
/// Magic 8, language 1, widths 3, version 4, source length 8, five lengths 40.
pub const HEADER_BYTES: usize = header::ALIAS_BYTES + 8;
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

/// A kind's bits in a code's `kf`: the field is above them.
pub const KIND_BITS: u32 = 10;

/// A node's key (`Writer`'s): its kind and field, and its flags (`CODE_EXTRA`, …).
pub const fn key(kind: u16, field: u16, flags: u8) -> u32 {
    (kind as u32 | (field as u32) << KIND_BITS) | (flags as u32) << 16
}

/// The image of nodes in preorder, each its key, start, end and the nodes in its subtree
/// (itself counted): each closed where its subtree ends. The widths are the narrowest, from
/// `Widths::guess` and again where it does not hold.
pub fn preorder_image(tag: u8, src: &[u8], nodes: &[(u32, u32, u32, u32)]) -> Vec<u8> {
    let mut widths = Widths::guess(src.len());
    loop {
        let mut w = Writer::new(tag, widths, nodes.len());
        // The open nodes, each its place and where its subtree ends.
        let mut open: Vec<(usize, usize, u32)> = Vec::new();
        for (i, &(key, start, end, size)) in nodes.iter().enumerate() {
            while let Some(&(o, to, e)) = open.last() {
                if i < to {
                    break;
                }
                w.close(o, e);
                open.pop();
            }
            w.push(key, start);
            open.push((i, i + size.max(1) as usize, end));
        }
        while let Some((o, _, e)) = open.pop() {
            w.close(o, e);
        }
        match w.finish(src) {
            Ok(bytes) => return bytes,
            Err(Redo(x)) => widths = x,
        }
    }
}

/// A source's SHA-256 (not in the header since format 12; the engine keeps it in memory).
pub fn digest(src: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(src).into()
}

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

/// A writer's widths, as a type: `Narrow`, the widths nearly every file is written at, fixed
/// when the writer is compiled, so that a store is one instruction; or `Widths`, any, read
/// as the writer runs.
pub trait Layout: Copy {
    fn widths(self) -> Widths;
}

/// Sizes, offsets and tails at 2 bytes, tails from the last descendant.
#[derive(Clone, Copy, Debug, Default)]
pub struct Narrow;

impl Narrow {
    pub const WIDTHS: Widths = Widths { size: 2, off: 2, tail: 2, from_start: false };
}

impl Layout for Narrow {
    #[inline(always)]
    fn widths(self) -> Widths {
        Self::WIDTHS
    }
}

impl Layout for Widths {
    #[inline(always)]
    fn widths(self) -> Widths {
        self
    }
}

/// From this many source bytes on, `Widths::guess` writes sizes at 4 bytes. Calibrated on the
/// repository's 1,362 `.rr` files (`rr_syntax`'s `widths_guess_calibration`, 2026-11-13): the
/// densest has 0.334 nodes a byte, the densest of 64 KiB or more 0.205 (42,081 nodes in
/// 205,423 bytes); none needs 4. At 0.205 a file has 65,535 nodes at about 320,000 bytes. With
/// this bound no file is parsed twice; at 160,000, six were.
pub const SIZE_GUESS_BYTES: usize = 320_000;

const NO_CODE: u16 = u16::MAX;
/// The id table's slots: a key is below this (`kf` 16 bits, flags 3).
const ID_SLOTS: usize = 1 << 19;

/// A tape written as a parser parses: each node's columns at their final widths, at its place,
/// as it is made. The parser gives each node in preorder (`push`), its end and so its size at
/// its close (`close`), and edits only the nodes it made last: a code changed (`set_key`), a
/// node put before the ones from a place on (`insert`), the nodes from a place on taken back
/// (`truncate`). Each edit rewrites only the nodes from its place on. At the end (`finish`) the
/// columns are moved into place after the code column, and the header and code table written.
///
/// A node's key is its code: `kf | flags << 16`.
pub struct Writer<L: Layout = Widths> {
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
    /// The last block's base: a push checks its start against it.
    block_base: u32,
    cap: usize,
    /// The codes by id: each key and the node that first used it.
    codes: Vec<(u32, u32)>,
    /// The nodes before this have their codes written; the rest only their keys.
    coded: usize,
    id_of: Vec<u16>,
    layout: L,
    /// An offset or tail from which a push or close takes the long way: 0x10000, or 0 once a
    /// node is marked (then every node does, and its marks are kept exact).
    wide_from: u32,
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

impl<L: Layout> Writer<L> {
    /// A writer for about `nodes` nodes (more grow it) at `layout`'s widths.
    pub fn new(tag: u8, layout: L, nodes: usize) -> Self {
        let widths = layout.widths();
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
            block_base: 0,
            cap,
            codes: Vec::new(),
            coded: 0,
            id_of: IDS.with(|ids| {
                let mut ids = std::mem::take(&mut *ids.borrow_mut());
                ids.resize(ID_SLOTS, NO_CODE);
                ids
            }),
            layout,
            wide_from: 0x10000,
            marks: Vec::new(),
            tag,
        }
    }

    /// Room for twice the nodes: every column at once.
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let cap = 2 * self.cap;
        let w = self.layout.widths();
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
    #[cold]
    #[inline(never)]
    fn mark(&mut self, node: usize, what: u8, on: bool) {
        let at = self.marks.iter().position(|&m| m == (node as u32, what));
        match (on, at) {
            (true, None) => self.marks.push((node as u32, what)),
            (false, Some(k)) => {
                self.marks.swap_remove(k);
            }
            _ => {}
        }
        self.wide_from = if self.marks.is_empty() { 0x10000 } else { 0 };
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
        // Every column holds `cap` cells, and a node's index is below `cap` (`push`, `grow`).
        debug_assert!((at + 1) * usize::from(width) <= column.len());
        let p = column.as_mut_ptr();
        // SAFETY: the cell is inside the column, as the assertion says.
        unsafe {
            if width == 2 {
                std::ptr::write_unaligned(p.add(2 * at).cast::<u16>(), (value as u16).to_le());
            } else {
                std::ptr::write_unaligned(p.add(4 * at).cast::<u32>(), value.to_le());
            }
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
        let (from, to) = (self.coded, self.n);
        // The columns as locals: a store to one is not a store to the writer's fields.
        let keys = &self.keys[from..to];
        let code = &mut self.out[HEADER_BYTES + 2 * from..HEADER_BYTES + 2 * to];
        let (id_of, codes) = (&mut self.id_of[..], &mut self.codes);
        for (k, (&key, cell)) in keys.iter().zip(code.chunks_exact_mut(2)).enumerate() {
            // A key is below `ID_SLOTS` (`push`'s assertion): the mask is no change, and no check.
            let slot = &mut id_of[(key as usize) & (ID_SLOTS - 1)];
            if *slot == NO_CODE {
                *slot = codes.len() as u16;
                codes.push((key, (from + k) as u32));
            }
            cell.copy_from_slice(&slot.to_le_bytes());
        }
        self.coded = to;
    }

    /// Block `b`'s base and its nodes' offsets, from their starts.
    #[cold]
    #[inline(never)]
    fn block(&mut self, b: usize) {
        let from = b * CHECKPOINT_STRIDE;
        let to = (from + CHECKPOINT_STRIDE).min(self.n);
        let base = self.starts[from..to].iter().copied().min().expect("a node in the block");
        if b == self.base.len() {
            self.base.push(base);
        } else {
            self.base[b] = base;
        }
        if b + 1 == self.base.len() {
            self.block_base = base;
        }
        for n in from..to {
            let off = self.starts[n] - base;
            if off > 0xffff || !self.marks.is_empty() {
                self.mark(n, OFF_WIDE, off > 0xffff);
            }
            Self::put(&mut self.off, n, self.layout.widths().off, off);
        }
    }

    /// A node at the end, in preorder: its key and start. Its size and tail are written at its
    /// close: every node is closed before `finish`. Its code is written when it is committed.
    #[inline(always)]
    pub fn push(&mut self, key: u32, start: u32) -> usize {
        debug_assert!((key as usize) < ID_SLOTS);
        let node = self.n;
        if node == self.cap {
            self.grow();
        }
        self.n = node + 1;
        // SAFETY: `node < cap`, and `keys` and `starts` hold `cap` values.
        unsafe {
            *self.keys.get_unchecked_mut(node) = key;
            *self.starts.get_unchecked_mut(node) = start;
        }
        // The block's base holds unless the node starts a block, or starts before its base
        // (its offset wraps), or its offset needs 4 bytes, or nodes are marked.
        let off = start.wrapping_sub(self.block_base);
        if node % CHECKPOINT_STRIDE == 0 || off >= self.wide_from {
            self.block(node / CHECKPOINT_STRIDE);
        } else {
            Self::put(&mut self.off, node, self.layout.widths().off, off);
        }
        node
    }

    /// `node`'s end: its subtree is the nodes from it on, and its tail from its last one.
    #[inline(always)]
    pub fn close(&mut self, node: usize, end: u32) {
        let n = self.n;
        debug_assert!(node < n);
        // SAFETY: `node < n <= cap`.
        let (last, own) = unsafe { (*self.starts.get_unchecked(n - 1), *self.starts.get_unchecked(node)) };
        let from_start = self.layout.widths().from_start;
        let from = if from_start { own } else { last };
        let tail = end.wrapping_sub(from);
        // From the last descendant, an end before it wraps: a tail of 0x10000 or more.
        if tail >= self.wide_from || (from_start && end < last) {
            self.close_marks(node, end, last, from, tail);
        }
        Self::put(&mut self.size, node, self.layout.widths().size, (n - node) as u32);
        Self::put(&mut self.tail, node, self.layout.widths().tail, tail);
    }

    /// `close`'s marks: a node that ends early or whose tail needs 4 bytes, or any node
    /// once some node is marked (its marks kept exact).
    #[cold]
    #[inline(never)]
    fn close_marks(&mut self, node: usize, end: u32, last: u32, from: u32, tail: u32) {
        self.mark(node, EARLY, end < last);
        self.mark(node, TAIL_WIDE, tail > 0xffff && end >= from);
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
        let (n, ws) = (self.n, self.layout.widths());
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
        } else {
            // The next push starts a block.
            self.block_base = 0;
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
            tail: if from_start == self.layout.widths().from_start && has(TAIL_WIDE) { 4 } else { 2 },
            from_start,
        };
        if want != self.layout.widths() {
            return Err(Redo(want));
        }
        let w = self.layout.widths();
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
        for length in [nodes, self.codes.len(), code_bytes, 0, 0] {
            h.extend_from_slice(&(length as u64).to_le_bytes());
        }
        out[..HEADER_BYTES].copy_from_slice(&h);
        Ok(out)
    }
}

impl<L: Layout> Drop for Writer<L> {
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
    let n = u64::from_le_bytes(bytes[header::NODES..header::NODES + 8].try_into().expect("8 bytes")) as usize;
    let w = [usize::from(bytes[header::WIDTHS]), usize::from(bytes[header::WIDTHS + 1]), usize::from(bytes[header::WIDTHS + 2] & !TAIL_FROM_START)];
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

/// A node as an image holds it: its code's key (`kf | flags << 16`, as `Writer::push` takes it),
/// its span, and its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decoded {
    pub key: u32,
    pub start: u32,
    pub end: u32,
    pub parent: Option<u32>,
}

/// The nodes of the image `bytes`, in preorder: `Writer`'s inverse, read in place (each node's
/// code through the code table, its start from its block's base, its end from its tail and its
/// last descendant's start, its parent from the sizes).
pub fn decode(bytes: &[u8]) -> Result<Vec<Decoded>, String> {
    let u64_at = |at: usize| -> Result<usize, String> {
        let b = bytes.get(at..at + 8).ok_or("a short tape header")?;
        usize::try_from(u64::from_le_bytes(b.try_into().expect("8 bytes"))).map_err(|_| "a tape length overflows".to_owned())
    };
    if bytes.len() < HEADER_BYTES || &bytes[..8] != MAGIC {
        return Err("not a tape image".into());
    }
    if u32::from_le_bytes(bytes[header::VERSION..header::VERSION + 4].try_into().expect("4 bytes")) != FORMAT_VERSION {
        return Err("a tape of another format".into());
    }
    let w = [usize::from(bytes[header::WIDTHS]), usize::from(bytes[header::WIDTHS + 1]), usize::from(bytes[header::WIDTHS + 2] & !TAIL_FROM_START)];
    let from_start = bytes[header::WIDTHS + 2] & TAIL_FROM_START != 0;
    let (n, code_count) = (u64_at(header::NODES)?, u64_at(header::CODES)?);
    let mut at = HEADER_BYTES;
    let mut column = |width: usize, count: usize| -> Result<&[u8], String> {
        let c = bytes.get(at..at + width * count).ok_or("a short tape column")?;
        at = padded(at + width * count);
        Ok(c)
    };
    let (code, size, base, off, tail) = (column(2, n)?, column(w[0], n)?, column(4, n.div_ceil(CHECKPOINT_STRIDE))?, column(w[1], n)?, column(w[2], n)?);
    let mut keys = Vec::with_capacity(code_count);
    let mut c = at;
    for _ in 0..code_count {
        let entry = bytes.get(c..c + 6).ok_or("a short code table")?;
        keys.push(u32::from(u16::from_le_bytes([entry[0], entry[1]])) | u32::from(entry[2]) << 16);
        c += 6 + 2 * usize::from(entry[5]);
    }
    let get = |col: &[u8], width: usize, i: usize| -> u32 {
        let mut v = [0u8; 4];
        v[..width].copy_from_slice(&col[i * width..i * width + width]);
        u32::from_le_bytes(v)
    };
    let start = |i: usize| get(base, 4, i / CHECKPOINT_STRIDE).wrapping_add(get(off, w[1], i));
    let mut out = Vec::with_capacity(n);
    let mut open: Vec<(u32, usize)> = Vec::new();
    for i in 0..n {
        while open.last().is_some_and(|&(_, end)| end <= i) {
            open.pop();
        }
        let s = get(size, w[0], i) as usize;
        let id = usize::from(u16::from_le_bytes([code[2 * i], code[2 * i + 1]]));
        let key = *keys.get(id).ok_or("a node's code is not in the table")?;
        let last = i + s.max(1) - 1;
        if last >= n {
            return Err("a node's size runs past the tape".into());
        }
        let from = if from_start { start(i) } else { start(last) };
        out.push(Decoded { key, start: start(i), end: from.wrapping_add(get(tail, w[2], i)), parent: open.last().map(|&(p, _)| p) });
        open.push((i as u32, i + s.max(1)));
    }
    Ok(out)
}
