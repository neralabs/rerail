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
    /// The header, then the code column: the image's start, as it will be.
    out: Vec<u8>,
    size: Vec<u8>,
    off: Vec<u8>,
    tail: Vec<u8>,
    base: Vec<u32>,
    /// Each node's start and key: the parser's working state, what an edit recomputes from.
    starts: Vec<u32>,
    keys: Vec<u32>,
    /// The codes by id: each key and the node that first used it.
    codes: Vec<(u32, u32)>,
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

thread_local! {
    /// The code ids by key, kept between a thread's tapes: each tape clears what it set.
    static IDS: std::cell::RefCell<Vec<u16>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The widths a tape needs, where the writer's did not hold its values: parse again at these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redo(pub Widths);

impl Writer {
    pub fn new(tag: u8, widths: Widths, nodes: usize) -> Self {
        let mut out = Vec::with_capacity(HEADER_BYTES + 2 * nodes + COLUMN_ALIGN);
        out.resize(HEADER_BYTES, 0);
        Writer {
            out,
            size: Vec::with_capacity(nodes * usize::from(widths.size)),
            off: Vec::with_capacity(nodes * usize::from(widths.off)),
            tail: Vec::with_capacity(nodes * usize::from(widths.tail)),
            base: Vec::with_capacity(nodes / CHECKPOINT_STRIDE + 1),
            starts: Vec::with_capacity(nodes),
            keys: Vec::with_capacity(nodes),
            codes: Vec::new(),
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

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn key(&self, node: usize) -> u32 {
        self.keys[node]
    }

    pub fn start(&self, node: usize) -> u32 {
        self.starts[node]
    }

    fn put(column: &mut [u8], at: usize, width: u8, value: u32) {
        let w = usize::from(width);
        column[at * w..at * w + w].copy_from_slice(&value.to_le_bytes()[..w]);
    }

    fn set_code(&mut self, node: usize, id: u16) {
        let at = HEADER_BYTES + 2 * node;
        self.out[at..at + 2].copy_from_slice(&id.to_le_bytes());
    }

    /// The id of `key`, numbered by first use: new at `node` if no node before used it.
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

    /// Whether some code was first used at `node` or after: then an edit there renumbers.
    fn fresh_from(&self, node: usize) -> bool {
        self.codes.last().is_some_and(|&(_, first)| first as usize >= node)
    }

    /// The codes first used from `node` on, numbered again over the nodes from there.
    fn renumber_from(&mut self, node: usize) {
        while let Some(&(key, first)) = self.codes.last() {
            if (first as usize) < node {
                break;
            }
            self.id_of[key as usize] = NO_CODE;
            self.codes.pop();
        }
        for n in node..self.len() {
            let id = self.id(self.keys[n], n);
            self.set_code(n, id);
        }
    }

    /// Block `b`'s base and its nodes' offsets, from their starts.
    fn block(&mut self, b: usize) {
        let from = b * CHECKPOINT_STRIDE;
        let to = (from + CHECKPOINT_STRIDE).min(self.len());
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
    /// its close.
    #[inline]
    pub fn push(&mut self, key: u32, start: u32) -> usize {
        let node = self.len();
        self.keys.push(key);
        self.starts.push(start);
        let id = self.id(key, node);
        self.out.extend_from_slice(&id.to_le_bytes());
        self.size.resize(self.size.len() + usize::from(self.widths.size), 0);
        self.off.resize(self.off.len() + usize::from(self.widths.off), 0);
        self.tail.resize(self.tail.len() + usize::from(self.widths.tail), 0);
        Self::put(&mut self.size, node, self.widths.size, 1);
        let b = node / CHECKPOINT_STRIDE;
        match self.base.get(b) {
            Some(&base) if base <= start => {
                let off = start - base;
                if off > 0xffff {
                    self.mark(node, OFF_WIDE, true);
                }
                Self::put(&mut self.off, node, self.widths.off, off);
            }
            _ => self.block(b),
        }
        node
    }

    /// `node`'s end: its subtree is the nodes from it on, and its tail from its last one.
    #[inline]
    pub fn close(&mut self, node: usize, end: u32) {
        let size = (self.len() - node) as u32;
        let last = self.starts[node + size as usize - 1];
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
    pub fn set_key(&mut self, node: usize, key: u32) {
        if self.keys[node] == key {
            return;
        }
        self.keys[node] = key;
        if self.fresh_from(node) || self.id_of[key as usize] == NO_CODE {
            self.renumber_from(node);
        } else {
            self.set_code(node, self.id_of[key as usize]);
        }
    }

    /// A node at `at`, before the nodes from there on, which move up one: a node found after
    /// the first of its children (an operator after its left operand). It is open: its close
    /// gives its size.
    pub fn insert(&mut self, at: usize, key: u32, start: u32) {
        if at == self.len() {
            self.push(key, start);
            return;
        }
        let shift = |column: &mut Vec<u8>, width: u8| {
            let w = usize::from(width);
            column.resize(column.len() + w, 0);
            let n = column.len();
            column.copy_within(at * w..n - w, at * w + w);
        };
        let ws = self.widths;
        self.out.resize(self.out.len() + 2, 0);
        let n = self.out.len();
        self.out.copy_within(HEADER_BYTES + 2 * at..n - 2, HEADER_BYTES + 2 * at + 2);
        shift(&mut self.size, ws.size);
        shift(&mut self.tail, ws.tail);
        self.off.resize(self.off.len() + usize::from(ws.off), 0);
        self.keys.insert(at, key);
        self.starts.insert(at, start);
        Self::put(&mut self.size, at, ws.size, 1);
        Self::put(&mut self.tail, at, ws.tail, 0);
        // The codes from `at` on: numbered again where any was first used there or after.
        for c in self.codes.iter_mut().filter(|c| c.1 as usize >= at) {
            c.1 += 1;
        }
        for m in self.marks.iter_mut().filter(|m| m.0 as usize >= at) {
            m.0 += 1;
        }
        if self.fresh_from(at) || self.id_of[key as usize] == NO_CODE {
            self.renumber_from(at);
        } else {
            self.set_code(at, self.id_of[key as usize]);
        }
        // The blocks from `at`'s on.
        for b in at / CHECKPOINT_STRIDE..self.len().div_ceil(CHECKPOINT_STRIDE) {
            self.block(b);
        }
    }

    /// The nodes from `len` on taken back (a parse that did not hold).
    pub fn truncate(&mut self, len: usize) {
        if len >= self.len() {
            return;
        }
        self.keys.truncate(len);
        self.starts.truncate(len);
        self.out.truncate(HEADER_BYTES + 2 * len);
        self.size.truncate(len * usize::from(self.widths.size));
        self.off.truncate(len * usize::from(self.widths.off));
        self.tail.truncate(len * usize::from(self.widths.tail));
        while let Some(&(key, first)) = self.codes.last() {
            if (first as usize) < len {
                break;
            }
            self.id_of[key as usize] = NO_CODE;
            self.codes.pop();
        }
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
        let has = |what: u8| self.marks.iter().any(|m| m.1 == what);
        let from_start = has(EARLY);
        let want = Widths {
            size: width_for(self.len() as u32),
            off: if has(OFF_WIDE) { 4 } else { 2 },
            // Tails measured from the other place say nothing of the width: 2, the guess.
            tail: if from_start == self.widths.from_start && has(TAIL_WIDE) { 4 } else { 2 },
            from_start,
        };
        if want != self.widths {
            return Err(Redo(want));
        }
        let w = self.widths;
        let nodes = self.len();
        let code_bytes = 6 * self.codes.len();
        let pad = |out: &mut Vec<u8>| out.resize(padded(out.len()), 0);
        let mut out = std::mem::take(&mut self.out);
        out.reserve(padded(self.size.len()) + padded(4 * self.base.len()) + padded(self.off.len()) + padded(self.tail.len()) + code_bytes + 3 * COLUMN_ALIGN);
        pad(&mut out);
        out.extend_from_slice(&self.size);
        pad(&mut out);
        for b in &self.base {
            out.extend_from_slice(&b.to_le_bytes());
        }
        pad(&mut out);
        out.extend_from_slice(&self.off);
        pad(&mut out);
        out.extend_from_slice(&self.tail);
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
