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

pub mod tape;
pub mod f16;
pub mod home;
pub mod newlines;
/// The tape's reads in Rust, written directly (decision 0534): this crate's readers'.
pub mod reads;
/// The tape's writer in Rust, written directly (decision 0532): the host's, and a generated tier 2's `ops` module.
pub mod writer16;

pub use rerail_vector::{Pod, vector::Vector};


/// **Deprecated** (decision 0550): only `rust_stream`, itself deprecated, reads it.
/// The writer as Rust source (`writer16`, the host's own): a tier 2 generated ahead of time that writes format 16
/// holds it as its module `w` (`rerail_rust_stream::rust_stream`).
pub const WRITER_RUST: &str = include_str!("writer16.rs");

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
/// The `.rerail` dialect's language tag (`languages/rr.json`'s `tapeTag`).
pub const RR_TAG: u8 = 13;

/// A kind's bits in a code's `kf`: the field is above them.
pub const KIND_BITS: u32 = 10;

/// A node's key (`Writer`'s): its kind and field, and its flags (`CODE_EXTRA`, …). A kind past
/// `KIND_BITS` or a field past the 6 bits above it would spill into the next: a grammar's ids
/// stay inside them.
pub const fn key(kind: u16, field: u16, flags: u8) -> u32 {
    debug_assert!((kind as u32) < 1 << KIND_BITS && (field as u32) < 1 << (16 - KIND_BITS), "a kind or field past its bits");
    (kind as u32 | (field as u32) << KIND_BITS) | (flags as u32) << 16
}

/// The image of nodes in preorder, each its key, start, end and the nodes in its subtree
/// (itself counted): each closed where its subtree ends. The widths are the narrowest, from
/// `Widths::guess` and again where it does not hold.
pub fn preorder_image(tag: u8, src: &[u8], nodes: &[(u32, u32, u32, u32)]) -> Vector<u8> {
    preorder_image_in(tag, &[], src, nodes)
}

/// `preorder_image`, the keys naming the supertype `classes` (`Writer::classes`).
pub fn preorder_image_in(tag: u8, classes: &[u16], src: &[u8], nodes: &[(u32, u32, u32, u32)]) -> Vector<u8> {
    let mut widths = Widths::guess(src.len());
    loop {
        let mut w = Writer::new(tag, widths, nodes.len()).classes(classes);
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
    /// the source's length, and a `.rerail` file has fewer nodes than bytes: below 64 KiB every
    /// column fits 2 bytes, so nearly every file is written once. Above it, offsets and tails
    /// stay 2 (they are short distances), and sizes take 4 from `SIZE_GUESS_BYTES` on.
    pub fn guess(len: usize) -> Self {
        Widths { size: if len < SIZE_GUESS_BYTES { 2 } else { 4 }, off: 2, tail: 2, from_start: false }
    }

    /// Every column at 4 bytes: the widths a file is parsed again at when 2 do not hold it
    /// (docs/parser-generators/37). A start and an end are `u32`s, so every value fits.
    pub const WIDE: Widths = Widths { size: 4, off: 4, tail: 4, from_start: false };
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
/// repository's 1,362 `.rerail` files (`rr_syntax`'s `widths_guess_calibration`, 2026-11-13): the
/// densest has 0.334 nodes a byte, the densest of 64 KiB or more 0.205 (42,081 nodes in
/// 205,423 bytes); none needs 4. At 0.205 a file has 65,535 nodes at about 320,000 bytes. With
/// this bound no file is parsed twice; at 160,000, six were.
pub const SIZE_GUESS_BYTES: usize = 320_000;

const NO_CODE: u16 = u16::MAX;
/// The id table's slots: a key is below this (`kf` 16 bits, flags 3), with no supertype
/// classes (`Writer::classes`): each class doubles it.
const ID_SLOTS: usize = 1 << 19;
/// A key's first class bit (`Writer::classes`): bit `CLASS_SHIFT + j` puts the node in the
/// writer's class `j`.
pub const CLASS_SHIFT: u32 = 19;
/// The most supertype classes a writer's keys name.
pub const MAX_CLASSES: usize = 4;

/// `v[from..to]` moved up by `by` cells (an insert's room). A span that fits a fixed window of
/// `W` cells, with room for the window past it, moves as one fixed-size copy: a vector load and
/// store, not a call of `memmove` (most wraps move a few nodes). The writer's windows are 16 nodes
/// of each column (64 bytes of a size or tail column, 2 or 4 bytes a node). The cells past `to` it writes
/// are free capacity.
#[inline(always)]
fn shift_up<T: Copy + Default, const W: usize>(v: &mut [T], from: usize, to: usize, by: usize) {
    if to - from <= W && from + by + W <= v.len() {
        let mut win = [T::default(); W];
        win.copy_from_slice(&v[from..from + W]);
        v[from + by..from + by + W].copy_from_slice(&win);
    } else {
        v.copy_within(from..to, from + by);
    }
}

/// A tape written as a parser parses: each node's columns at their final widths, at its place,
/// as it is made. The parser gives each node in preorder (`push`), its end and so its size at
/// its close (`close`), and edits only the nodes it made last: a code changed (`set_key`), a
/// node put before the ones from a place on (`insert`), the nodes from a place on taken back
/// (`truncate`). Each edit rewrites only the nodes from its place on. At the end (`finish`) the
/// columns are moved into place after the code column, and the header and code table written.
///
/// A node's key is its code: `kf | flags << 16`, and a bit for each supertype class it is in
/// (`classes`, from `CLASS_SHIFT`).
pub struct Writer<L: Layout = Widths> {
    /// The header, then the code column: the image's start, as it will be. Reserved for the
    /// whole image at `cap` nodes, so that `finish` appends the columns in place.
    out: Vector<u8>,
    /// The columns, each `cap` cells at its width; the first `n - base` are the nodes' from `base` on (`cell`).
    size: Vector<u8>,
    tail: Vector<u8>,
    /// Each node's start and key: the parser's working state, what an edit recomputes from.
    starts: Vector<u32>,
    keys: Vector<u32>,
    /// The nodes, and the nodes the cells have room for.
    n: usize,
    cap: usize,
    /// The codes by id: each key and the node that first used it.
    codes: Vector<[u32; 2]>,
    /// The nodes before this have their codes written; the rest only their keys.
    coded: usize,
    id_of: Vector<u16>,
    /// The id table's slots in use: `ID_SLOTS`, doubled for each class.
    slots: usize,
    /// The supertype classes a key's bits name, ascending (none for most languages).
    classes: Vector<u16>,
    layout: L,
    /// An offset or tail from which a push or close takes the long way: 0x10000, or 0 once a
    /// node is marked (then every node does, and its marks are kept exact).
    wide_from: u32,
    /// The nodes whose tail needs 4 bytes, or that end before their last descendant starts
    /// (`TAIL_WIDE`, `EARLY`): few, kept exact through every edit, so that `finish` knows the
    /// narrowest widths without reading the tail column. (Offsets and bases are not kept as the
    /// parse goes: `finish` computes them from the starts in one pass, `offsets`.)
    marks: Vector<[u32; 2]>,
    /// The open chains (`open_chain`), innermost last: each its node and where its wraps begin in
    /// `held`. `chain_at` is the innermost's node (`usize::MAX`: none), the one compare `close`,
    /// `key` and `set_key` make.
    chains: Vector<[usize; 2]>,
    chain_at: usize,
    /// The wraps of the open chains, not yet on the tape: each its key, start, and at its close the
    /// nodes there were then and its end (0 nodes: open).
    held: Vector<Held>,
    tag: u8,
    /// Where the run writes format 16 or 16z (`to`): the nodes before `passed` are given to its ops, and an edit
    /// reaches before them only to retag a node or set an open node's start (decision 0434 §2). 0 for format 15.
    sixteen: Option<Box<Passing>>,
    passed: usize,
    /// The node the columns' first cells are: the nodes before it were passed to format 16's ops, and their cells
    /// given back (the window, decision 0434 §2). 0 for format 15.
    base: usize,
    /// The most nodes the window has held at a commit: the sizes its cells held.
    widest: usize,
    /// The source ranges of the placeholders a program's tape leaves out (`omitting`), sorted, apart: a node whose
    /// span (not empty) lies in one is written with none of its subtree, as an embedded program's tape is
    /// (`embed::segments::SegmentMap::omitted`). Empty for every other tape.
    omit: Vector<[u32; 2]>,
}

/// Where a lean parser's writer writes (decision 0434 §2): format 15's columns, finished at the end, or format 16
/// or 16z, each node given to `f16::Writer`'s ops as it is committed. The writer's columns are then the edit window.
#[derive(Clone, Default)]
pub enum Target {
    #[default]
    F15,
    F16(Sixteen),
}

/// A format 16 target: the language's table, its tag, 16z where `z`, each block's presence where `blocks`, and each
/// key's number in the table (`None`: a code the table lacks, and the writer's tape is format 15's).
#[derive(Clone)]
pub struct Sixteen {
    pub lang: &'static f16::Lang,
    pub tag: u8,
    pub z: bool,
    pub blocks: bool,
    pub number: std::sync::Arc<dyn Fn(u32) -> Option<u32> + Send + Sync>,
}

/// What a lean parser's writer writes to and is finished into: its target, given when the writer is made
/// (`Writer::to`), and its finish. A closure is a sink for format 15.
pub trait Sink<Out> {
    fn target(&self) -> Target {
        Target::F15
    }
    /// The source ranges the tape leaves out (`Writer::omitting`): an embedded program's placeholders. None for
    /// every other tape.
    fn omissions(&self) -> Vector<[u32; 2]> {
        Vector::new()
    }
    fn finish(&mut self, w: Writer) -> Result<Out, Redo>;
    /// Whether the parse is to run again at the widths it had: `finish` could not write the target and changed it
    /// (format 16's nodes went to its ops, and a node the table lacks leaves format 15, which the window does not
    /// hold). Asked once a `Redo`.
    fn again(&mut self) -> bool {
        false
    }
}

impl<Out, F: FnMut(Writer) -> Result<Out, Redo>> Sink<Out> for F {
    fn finish(&mut self, w: Writer) -> Result<Out, Redo> {
        self(w)
    }
}

/// The format 16 writer the nodes are passed to as they are committed (`Writer::pass`).
struct Passing {
    w: f16::Writer,
    number: std::sync::Arc<dyn Fn(u32) -> Option<u32> + Send + Sync>,
    /// Each code id's number, as far as resolved.
    numbers: Vector<u32>,
    /// The nodes passed on whose close the ops have not had, outermost first: the parse's open ancestors.
    open: Vector<Passed>,
    /// Why the nodes cannot be written in format 16 (a code the table lacks, an edit before the last commit).
    bad: Option<String>,
    /// A node read after its commit that the window no longer holds (`Writer::passed_node`).
    lost: std::cell::Cell<Option<usize>>,
}

/// A node passed to format 16's ops before its close: what the parse may still read of it, and its close.
#[derive(Clone, Copy)]
#[repr(C)]
struct Passed {
    node: usize,
    /// Its subtree's end (`usize::MAX`: open), and its end.
    end_at: usize,
    key: u32,
    start: u32,
    end: u32,
    pad: u32,
}

// SAFETY: `repr(C)`, two words and four `u32`s: no padding, no address.
unsafe impl Pod for Passed {}

impl Passing {
    fn fail(&mut self, why: impl FnOnce() -> String) {
        if self.bad.is_none() {
            self.bad = Some(why());
        }
    }

    /// The nodes passed on whose subtrees end at or before `i`, closed.
    fn close_to(&mut self, i: usize) {
        while let Some(&Passed { end_at, end, .. }) = self.open.last()
            && end_at <= i
        {
            self.w.close(u64::from(end));
            self.open.pop();
        }
    }
}

/// A wrap of an open chain, put on the tape when its chain ends (`Writer::end_chain`).
#[derive(Clone, Copy)]
#[repr(C)]
struct Held {
    key: u32,
    start: u32,
    nodes: u32,
    end: u32,
}

// SAFETY: `repr(C)`, four `u32`s.
unsafe impl Pod for Held {}

/// A closed leaf to put in before the node at `at` (`Writer::insert_leaves`).
#[derive(Clone, Copy, Debug)]
pub struct Leaf {
    pub key: u32,
    pub start: u32,
    pub end: u32,
    pub at: u32,
}

/// Where a writer was (`Writer::checkpoint`): its nodes, and its open chains and their wraps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    n: usize,
    chains: usize,
    held: usize,
}


/// The nodes a format 16 writer's columns hold at first (`Writer::to`): a statement's, the nodes since the last
/// commit, rarely more (a longer one grows them).
const WINDOW: usize = 4096;

const TAIL_WIDE: u8 = 1;
const EARLY: u8 = 2;

/// Code table bytes reserved for at first: room for 256 codes.
const CODES_RESERVED: usize = 6 * 256;

/// `n` copies of `v`, in the run's room (`rerail_vector::default_room`).
fn filled<T: Pod + Copy>(n: usize, v: T) -> Vector<T> {
    let mut c = Vector::with_capacity(n);
    c.resize(n, v);
    c
}

/// What a parser's writer is finished into, whatever the type of its widths (`Layout`): format 15's image or
/// columns, or format 16 (`Writer::finish_16`). For a parser generic over its layout; others take a `Sink`.
pub trait Finish {
    type Out;
    /// What the writer writes to (`Writer::to`).
    fn target(&self) -> Target {
        Target::F15
    }
    fn finish<L: Layout>(&mut self, w: Writer<L>) -> Result<Self::Out, Redo>;
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
        let mut out = Vector::with_capacity(image_bytes(cap, widths));
        out.resize(HEADER_BYTES + 2 * cap, 0);
        let cells = |w: u8| filled(cap * usize::from(w), 0u8);
        Writer {
            out,
            size: cells(widths.size),
            tail: cells(widths.tail),
            starts: filled(cap, 0),
            keys: filled(cap, 0),
            n: 0,
            cap,
            codes: Vector::new(),
            coded: 0,
            id_of: filled(ID_SLOTS, NO_CODE),
            slots: ID_SLOTS,
            classes: Vector::new(),
            layout,
            wide_from: 0x10000,
            marks: Vector::new(),
            chains: Vector::new(),
            chain_at: usize::MAX,
            held: Vector::new(),
            tag,
            sixteen: None,
            passed: 0,
            base: 0,
            widest: 0,
            omit: Vector::new(),
        }
    }

    /// The writer leaving out the subtrees in the source ranges `omit` (sorted, apart; `Sink::omissions`): an
    /// embedded program's placeholders, left out as the parse writes them (format 16's ops are never given them,
    /// `pass`; format 15's columns are made without them, `drop_omitted`), never by a walk of the tape after.
    pub fn omitting(mut self, omit: Vector<[u32; 2]>) -> Self {
        debug_assert!(omit.windows(2).all(|w| w[0][1] <= w[1][0]), "omitted ranges sorted and apart");
        self.omit = omit;
        self
    }

    /// Whether the span `start..end` (not empty) lies in an omitted range.
    #[inline]
    fn omitted(&self, start: u32, end: u32) -> bool {
        start < end && {
            let k = self.omit.partition_point(|r| r[1] <= start);
            self.omit.get(k).is_some_and(|r| r[0] <= start && end <= r[1])
        }
    }

    /// Whether an open node starting at `start` may yet be omitted (its end inside the range it starts in).
    #[inline]
    fn may_be_omitted(&self, start: u32) -> bool {
        let k = self.omit.partition_point(|r| r[1] <= start);
        self.omit.get(k).is_some_and(|r| r[0] <= start)
    }

    /// Format 15's columns without the omitted subtrees (`omitting`): every node closed and committed, all of them
    /// held (`holds_all`). The kept nodes' starts, keys and codes move down in order, their codes numbered again
    /// by first use, and each one's size and tail written again from its end (`close_at`), its marks with them.
    fn drop_omitted(&mut self) {
        if self.omit.is_empty() || self.n == 0 || self.base != 0 {
            return;
        }
        let n = self.n;
        let mut kept: Vector<u32> = Vector::with_capacity(n);
        let mut i = 0;
        while i < n {
            let size = self.size(i) as usize;
            if size > 0 && self.omitted(self.starts[i], self.end(i)) {
                i += size;
                continue;
            }
            kept.push(i as u32);
            i += 1;
        }
        if kept.len() == n {
            return;
        }
        // Each old node's count of kept nodes before it: a kept node's new size is its old subtree's kept nodes.
        let mut before: Vector<u32> = filled(n + 1, 0);
        for &o in kept.iter() {
            before[o as usize + 1] = 1;
        }
        for j in 0..n {
            before[j + 1] += before[j];
        }
        let ends: Vector<u32> = kept.iter().map(|&o| self.end(o as usize)).collect();
        let sizes: Vector<u32> = kept.iter().map(|&o| before[o as usize + self.size(o as usize) as usize] - before[o as usize]).collect();
        // The codes numbered again by first use among the kept nodes.
        let mut renumber: Vector<u16> = filled(self.codes.len(), NO_CODE);
        let mut codes: Vector<[u32; 2]> = Vector::new();
        for (k, &o) in kept.iter().enumerate() {
            let o = o as usize;
            let at = HEADER_BYTES + 2 * o;
            let id = usize::from(u16::from_le_bytes([self.out[at], self.out[at + 1]]));
            if renumber[id] == NO_CODE {
                renumber[id] = codes.len() as u16;
                codes.push([self.codes[id][0], k as u32]);
            }
            let to = HEADER_BYTES + 2 * k;
            self.out[to..to + 2].copy_from_slice(&renumber[id].to_le_bytes());
            self.starts[k] = self.starts[o];
            self.keys[k] = self.keys[o];
        }
        for &[key, _] in self.codes.iter() {
            self.id_of[key as usize] = NO_CODE;
        }
        for (id, &[key, _]) in codes.iter().enumerate() {
            self.id_of[key as usize] = id as u16;
        }
        self.codes = codes;
        (self.n, self.coded) = (kept.len(), kept.len());
        self.marks.clear();
        self.wide_from = 0x10000;
        for k in 0..self.n {
            self.close_at(k, ends[k], k + sizes[k] as usize);
        }
    }

    /// Node `node`'s cell in the columns (`base`).
    #[inline(always)]
    fn cell(&self, node: usize) -> usize {
        debug_assert!(node >= self.base, "node {node} before the window's {}", self.base);
        node - self.base
    }

    /// The writer writing to `target` for the source `src`: format 16's nodes passed to its ops as they are
    /// committed (decision 0434 §2). Before the first push.
    pub fn to(mut self, target: Target, src: &[u8]) -> Self {
        assert_eq!(self.n, 0, "a target before the first node");
        if let Target::F16(t) = target {
            let w = f16::Writer::new(t.lang, src, t.tag, t.z, t.blocks, self.cap);
            self.sixteen = Some(Box::new(Passing { w, number: t.number, numbers: Vector::new(), open: Vector::new(), bad: None, lost: Default::default() }));
            // The columns hold the window: the nodes since the last commit (more grow it).
            if self.cap > WINDOW {
                let (cap, widths) = (WINDOW, self.layout.widths());
                let cells = |w: u8| filled(cap * usize::from(w), 0u8);
                self.out = Vector::with_capacity(image_bytes(cap, widths));
                self.out.resize(HEADER_BYTES + 2 * cap, 0);
                (self.size, self.tail, self.starts, self.keys, self.cap) = (cells(widths.size), cells(widths.tail), filled(cap, 0), filled(cap, 0), cap);
            }
        }
        self
    }

    /// Whether the columns hold every node (format 15's finish can take them): none was passed to format 16's ops
    /// and given back.
    pub fn holds_all(&self) -> bool {
        self.base == 0
    }

    /// The widths the writer's columns are at.
    pub fn widths(&self) -> Widths {
        self.layout.widths()
    }

    /// What the parse reads of node `node`, passed on before its close: its entry, or `None` (said at the finish)
    /// where it was closed and given back.
    #[cold]
    #[inline(never)]
    fn passed_node(&self, node: usize) -> Option<Passed> {
        let s = self.sixteen.as_ref()?;
        let found = s.open.iter().rev().find(|e| e.node == node).copied();
        if found.is_none() {
            s.lost.set(Some(node));
        }
        found
    }

    /// Whether the writer writes format 16 or 16z (`to`): `finish_16` gives its tape.
    pub fn writes_16(&self) -> bool {
        self.sixteen.is_some()
    }

    /// The committed nodes not yet passed on given to the format 16 writer's ops in preorder: each opened, its number
    /// its code's, and closed where its subtree ends before the window.
    fn pass(&mut self) {
        let Some(mut s) = self.sixteen.take() else { return };
        let to = self.coded;
        if s.bad.is_none() {
            let mut i = self.passed;
            while i < to {
                s.close_to(i);
                // A closed subtree in an omitted placeholder (`omitting`): its ops never given. An open node that
                // may yet be one cannot be told: format 15 takes the parse, its columns made without them.
                if !self.omit.is_empty() {
                    let (start, size) = (self.starts[self.cell(i)], self.size(i) as usize);
                    if size > 0 && self.omitted(start, self.end(i)) {
                        i += size;
                        continue;
                    }
                    if size == 0 && self.may_be_omitted(start) {
                        s.fail(|| format!("node {i}, open at a commit, may be in an omitted placeholder"));
                        break;
                    }
                }
                let c = self.cell(i);
                let id = usize::from(u16::from_le_bytes([self.out[HEADER_BYTES + 2 * c], self.out[HEADER_BYTES + 2 * c + 1]]));
                while s.numbers.len() <= id {
                    let key = self.codes[s.numbers.len()][0];
                    match (s.number)(key) {
                        Some(number) => s.numbers.push(number),
                        None => break,
                    }
                }
                let Some(&number) = s.numbers.get(id) else {
                    let key = self.codes[id][0];
                    s.fail(|| format!("key {key:#x}: a code its format 16 table lacks"));
                    break;
                };
                let (key, start) = (self.keys[c], self.starts[c]);
                match self.size(i) as usize {
                    0 => {
                        s.w.open(number, u64::from(start));
                        s.open.push(Passed { node: i, key, start, end_at: usize::MAX, end: 0, pad: 0 });
                    }
                    1 => {
                        s.w.leaf(number, u64::from(start), u64::from(self.end(i)));
                    }
                    size => {
                        s.w.open(number, u64::from(start));
                        s.open.push(Passed { node: i, key, start, end_at: i + size, end: self.end(i), pad: 0 });
                    }
                }
                i += 1;
            }
            s.close_to(to);
            s.w.commit();
        }
        // Every node is passed on: the window is empty, and its cells are the next nodes'.
        debug_assert_eq!(to, self.n);
        self.widest = self.widest.max(to - self.base);
        self.passed = to;
        self.base = to;
        self.sixteen = Some(s);
    }

    /// An edit at `at` (an insert, a truncate): one before the nodes passed on means the tape is not format 16's.
    #[cold]
    #[inline(never)]
    fn edit_before_passed(&mut self, at: usize) {
        let passed = self.passed;
        if let Some(s) = &mut self.sixteen {
            s.fail(|| format!("an edit at node {at}, before the last commit's {passed}"));
        }
    }

    /// `close` of a node passed on while open: its close given to the ops once its subtree is.
    #[cold]
    #[inline(never)]
    fn close_passed(&mut self, node: usize, end: u32) {
        let n = self.n;
        let Some(s) = &mut self.sixteen else { return };
        match s.open.iter().rposition(|e| e.node == node) {
            Some(k) => {
                s.open[k].end_at = n;
                s.open[k].end = end;
                s.close_to(self.passed);
            }
            None => s.fail(|| format!("node {node} closed again")),
        }
    }

    /// `set_key` of a node passed on: retagged in the format 16 writer.
    #[cold]
    #[inline(never)]
    fn retag_passed(&mut self, node: usize, key: u32) {
        let Some(s) = &mut self.sixteen else { return };
        match (s.number)(key) {
            Some(number) => s.w.retag(node, number),
            None => s.fail(|| format!("key {key:#x}: a code its format 16 table lacks")),
        }
        // A closed node is retagged in the format 16 writer alone: a read of it after is lost (`passed_node`).
        if let Some(e) = s.open.iter_mut().rev().find(|e| e.node == node) {
            e.key = key;
        }
    }

    /// `set_start` of a node passed on while open: set in the format 16 writer.
    #[cold]
    #[inline(never)]
    fn set_start_passed(&mut self, node: usize, start: u32) {
        let Some(s) = &mut self.sixteen else { return };
        s.w.set_start(node, u64::from(start));
        match s.open.iter_mut().rev().find(|e| e.node == node) {
            Some(e) => e.start = start,
            None => s.lost.set(Some(node)),
        }
    }

    /// The supertype classes a key's bits from `CLASS_SHIFT` name, ascending, at most
    /// `MAX_CLASSES`: each node's code carries the classes its key has. Before the first push.
    pub fn classes(mut self, classes: &[u16]) -> Self {
        assert!(classes.len() <= MAX_CLASSES && classes.windows(2).all(|w| w[0] < w[1]) && self.n == 0);
        self.slots = ID_SLOTS << classes.len();
        if self.id_of.len() < self.slots {
            self.id_of.resize(self.slots, NO_CODE);
        }
        self.classes = Vector::from(classes);
        self
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
        self.tail.resize(cap * usize::from(w.tail), 0);
        self.starts.resize(cap, 0);
        self.keys.resize(cap, 0);
        self.cap = cap;
    }

    /// Whether node `node` has mark `what`, as `on` says.
    #[cold]
    #[inline(never)]
    fn mark(&mut self, node: usize, what: u8, on: bool) {
        let at = self.marks.iter().position(|&m| m == [node as u32, u32::from(what)]);
        match (on, at) {
            (true, None) => self.marks.push([node as u32, u32::from(what)]),
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
        if node == self.chain_at && let Some(h) = self.held_last() {
            return h.key;
        }
        if node < self.base {
            return self.passed_node(node).map_or(0, |e| e.key);
        }
        self.keys[self.cell(node)]
    }

    #[inline]
    pub fn start(&self, node: usize) -> u32 {
        if node < self.base {
            return self.passed_node(node).map_or(0, |e| e.start);
        }
        self.starts[self.cell(node)]
    }

    /// A closed node's subtree: itself and its descendants (0: the node is open).
    #[inline]
    pub fn size(&self, node: usize) -> u32 {
        if node < self.base {
            // A node given back reads as a leaf: a walk over siblings (`c += size(c)`) still moves on, and the parse
            // runs again for format 15 (`passed_node`).
            return self.passed_node(node).map_or(1, |e| if e.end_at == usize::MAX { 0 } else { (e.end_at - node) as u32 });
        }
        let w = self.layout.widths().size;
        let at = self.cell(node) * usize::from(w);
        if w == 2 { u32::from(u16::from_le_bytes([self.size[at], self.size[at + 1]])) } else { u32::from_le_bytes(self.size[at..at + 4].try_into().expect("4 bytes")) }
    }

    /// Whether `node` is open: pushed or inserted, not yet closed.
    #[inline]
    pub fn is_open(&self, node: usize) -> bool {
        self.size(node) == 0
    }

    /// A closed node's end.
    pub fn end(&self, node: usize) -> u32 {
        if node < self.base {
            return self.passed_node(node).map_or(0, |e| e.end);
        }
        let w = self.layout.widths();
        let c = self.cell(node);
        let at = c * usize::from(w.tail);
        let tail = if w.tail == 2 { u32::from(u16::from_le_bytes([self.tail[at], self.tail[at + 1]])) } else { u32::from_le_bytes(self.tail[at..at + 4].try_into().expect("4 bytes")) };
        let from = if w.from_start { self.starts[c] } else { self.starts[c + self.size(node) as usize - 1] };
        from.wrapping_add(tail)
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
        while let Some(&[key, first]) = self.codes.last() {
            if (first as usize) < node {
                break;
            }
            self.id_of[key as usize] = NO_CODE;
            self.codes.pop();
        }
        self.coded = node;
        if let Some(s) = &mut self.sixteen {
            s.numbers.truncate(self.codes.len());
        }
    }

    /// The nodes so far are final (a statement parsed or recovered): their codes, numbered by
    /// first use, written to the code column.
    pub fn commit(&mut self) {
        let (from, to) = (self.coded, self.n);
        let (a, b) = (self.cell(from), self.cell(to));
        // The columns as locals: a store to one is not a store to the writer's fields.
        let keys = &self.keys[a..b];
        let code = &mut self.out[HEADER_BYTES + 2 * a..HEADER_BYTES + 2 * b];
        let (id_of, codes, mask) = (&mut self.id_of[..], &mut self.codes, self.slots - 1);
        for (k, (&key, cell)) in keys.iter().zip(code.as_chunks_mut::<2>().0).enumerate() {
            // A key is below `slots` (`push`'s assertion): the mask is no change, and no check.
            let slot = &mut id_of[(key as usize) & mask];
            if *slot == NO_CODE {
                *slot = codes.len() as u16;
                codes.push([key, (from + k) as u32]);
            }
            cell.copy_from_slice(&slot.to_le_bytes());
        }
        self.coded = to;
        if self.sixteen.is_some() {
            self.pass();
        }
    }

    /// A node at the end, in preorder: its key and start. Its size and tail are written at its
    /// close: every node is closed before `finish`. Its code is written when it is committed.
    #[inline(always)]
    pub fn push(&mut self, key: u32, start: u32) -> usize {
        debug_assert!((key as usize) < self.slots);
        let node = self.n;
        let c = node - self.base;
        if c == self.cap {
            self.grow();
        }
        self.n = node + 1;
        // SAFETY: `c < cap`, and `keys` and `starts` hold `cap` values.
        unsafe {
            *self.keys.get_unchecked_mut(c) = key;
            *self.starts.get_unchecked_mut(c) = start;
        }
        // Size 0 until its close: an open node (`is_open`).
        Self::put(&mut self.size, c, self.layout.widths().size, 0);
        node
    }

    /// `node`'s end: its subtree is the nodes from it on, and its tail from its last one.
    #[inline(always)]
    pub fn close(&mut self, node: usize, end: u32) {
        if node == self.chain_at && self.close_held(end) {
            return;
        }
        if node < self.base {
            return self.close_passed(node, end);
        }
        self.close_at(node, end, self.n);
    }

    /// `close` as it was when the tape had `n` nodes: the nodes before `n` are its subtree's.
    #[inline(always)]
    fn close_at(&mut self, node: usize, end: u32, n: usize) {
        debug_assert!(node < n);
        let (c, l) = (self.cell(node), self.cell(n - 1));
        // SAFETY: `c <= l < cap`.
        let (last, own) = unsafe { (*self.starts.get_unchecked(l), *self.starts.get_unchecked(c)) };
        let from_start = self.layout.widths().from_start;
        let from = if from_start { own } else { last };
        let tail = end.wrapping_sub(from);
        // From the last descendant, an end before it wraps: a tail of 0x10000 or more.
        if tail >= self.wide_from || (from_start && end < last) {
            self.close_marks(node, end, last, from, tail);
        }
        Self::put(&mut self.size, c, self.layout.widths().size, (n - node) as u32);
        Self::put(&mut self.tail, c, self.layout.widths().tail, tail);
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
        if node == self.chain_at && let Some(h) = self.held_last_mut() {
            h.key = key;
            return;
        }
        if node < self.base {
            return self.retag_passed(node, key);
        }
        self.uncode_from(node);
        let c = self.cell(node);
        self.keys[c] = key;
    }

    /// A node at `at`, before the nodes from there on, which move up one: a node found after
    /// the first of its children (an operator after its left operand). It is open: its close
    /// gives its size. The code column from `at` on is written again at the commit.
    pub fn insert(&mut self, at: usize, key: u32, start: u32) {
        if at == self.chain_at {
            self.held.push(Held { key, start, nodes: 0, end: 0 });
            return;
        }
        self.insert_node(at, key, start);
    }

    /// `insert` on the tape, a chain at `at` or not (a leaf: a chain's wraps go before it): the node it put in.
    fn insert_node(&mut self, at: usize, key: u32, start: u32) -> usize {
        // Before the window: the tape is not format 16's (`edit_before_passed`), and the node goes at the window's
        // start, so that the parse's nodes count as it counts them.
        let at = if at < self.base {
            self.edit_before_passed(at);
            self.base
        } else {
            at
        };
        if at == self.n {
            return self.push(key, start);
        }
        if self.n - self.base == self.cap {
            self.grow();
        }
        let node = at;
        let (at, n, ws) = (self.cell(at), self.cell(self.n), self.layout.widths());
        // Most inserts wrap a leaf or a short subtree (a call's function, an operator's left operand): 58% of
        // C++'s move one node, 93% fewer than 16 (decision 0313). `shift_up`'s window holds 16 nodes of every
        // column, so those are one fixed-size copy each, not a call to `memmove`.
        let shift = |column: &mut [u8], width: u8| {
            let w = usize::from(width);
            shift_up::<u8, 64>(column, at * w, n * w, w);
        };
        shift(&mut self.size, ws.size);
        shift(&mut self.tail, ws.tail);
        shift_up::<u32, 16>(&mut self.keys, at, n, 1);
        shift_up::<u32, 16>(&mut self.starts, at, n, 1);
        self.n += 1;
        self.keys[at] = key;
        self.starts[at] = start;
        Self::put(&mut self.size, at, ws.size, 0);
        Self::put(&mut self.tail, at, ws.tail, 0);
        self.uncode_from(node);
        for m in self.marks.iter_mut().filter(|m| m[0] as usize >= node) {
            m[0] += 1;
        }
        node
    }

    /// A chain at node `at`: the nodes wrapped there one after another (`a + b + c`, `a.b.c`), each
    /// inserted at `at` before the last. Until the chain ends, each `insert` at `at` is held, not
    /// put on the tape, and `at` names the last held one (for `key`, `set_key` and `close`); the
    /// nodes after `at` stay where they are. `end_chain` puts them all on the tape in one move, so a
    /// chain of k wraps over n nodes costs n + k, where an insert each cost k times n. What is opened
    /// at `at` before the first wrap is the chain's own (a member chain inside an operand).
    /// Returns the chain's depth, which `end_chain` takes.
    pub fn open_chain(&mut self, at: usize) -> usize {
        debug_assert!(at <= self.n);
        let depth = self.chains.len();
        // An outer chain at the same node holds no wraps yet: they come after this chain ends.
        debug_assert!(self.chains.last().is_none_or(|&[a, base]| a != at || base == self.held.len()));
        self.chains.push([at, self.held.len()]);
        self.chain_at = at;
        depth
    }

    /// Ends the chains from `depth` on (`open_chain`'s), innermost first: their wraps put on the
    /// tape at their nodes, the last held outermost, each closed as it was.
    pub fn end_chain(&mut self, depth: usize) {
        while self.chains.len() > depth {
            let [at, base] = self.chains.pop().expect("a chain");
            self.chain_at = self.chains.last().map_or(usize::MAX, |c| c[0]);
            self.put_held(at, base);
        }
    }

    /// Where the innermost chain holds a wrap at `node`: the end of the last held one's subtree
    /// on the tape (its children's; the tape's end while it is open).
    #[inline]
    pub fn held_end(&self, node: usize) -> Option<usize> {
        if node != self.chain_at {
            return None;
        }
        self.held_last().map(|h| if h.nodes != 0 { h.nodes as usize } else { self.n })
    }

    #[inline]
    fn held_last(&self) -> Option<&Held> {
        let &[_, base] = self.chains.last()?;
        self.held[base..].last()
    }

    #[inline]
    fn held_last_mut(&mut self) -> Option<&mut Held> {
        let &[_, base] = self.chains.last()?;
        self.held[base..].last_mut()
    }

    /// `close` of the innermost chain's node: its last held wrap's, kept for `end_chain`. False
    /// when the chain holds none (the node is on the tape).
    fn close_held(&mut self, end: u32) -> bool {
        let n = self.n as u32;
        match self.held_last_mut() {
            Some(h) => {
                h.nodes = n;
                h.end = end;
                true
            }
            None => false,
        }
    }

    /// The wraps held from `base` on, put at `at` in one move: the last held first (outermost).
    /// Each closes as it would have: at its close the tape had `nodes` nodes, and every wrap held
    /// moved those up by the k wraps now before them.
    fn put_held(&mut self, at: usize, from: usize) {
        let k = self.held.len() - from;
        if k == 0 {
            return;
        }
        // Before the window: as `insert_node`.
        let at = if at < self.base {
            self.edit_before_passed(at);
            self.base
        } else {
            at
        };
        while self.n + k - self.base > self.cap {
            self.grow();
        }
        let (c, n, ws) = (self.cell(at), self.cell(self.n), self.layout.widths());
        let shift = |column: &mut [u8], width: u8| {
            let w = usize::from(width);
            column.copy_within(c * w..n * w, (c + k) * w);
        };
        shift(&mut self.size, ws.size);
        shift(&mut self.tail, ws.tail);
        self.keys.copy_within(c..n, c + k);
        self.starts.copy_within(c..n, c + k);
        self.n += k;
        self.uncode_from(at);
        for m in self.marks.iter_mut().filter(|m| m[0] as usize >= at) {
            m[0] += k as u32;
        }
        for j in 0..k {
            let h = self.held[from + j];
            let node = at + k - 1 - j;
            let cell = self.cell(node);
            self.keys[cell] = h.key;
            self.starts[cell] = h.start;
            Self::put(&mut self.size, cell, ws.size, 0);
            Self::put(&mut self.tail, cell, ws.tail, 0);
            if h.nodes != 0 {
                self.close_at(node, h.end, h.nodes as usize + k);
            }
        }
        self.held.truncate(from);
    }

    /// Where the writer is: what `rewind` goes back to.
    #[inline]
    pub fn checkpoint(&self) -> Checkpoint {
        Checkpoint { n: self.n, chains: self.chains.len(), held: self.held.len() }
    }

    /// Back to `c` (a parse that did not hold): the nodes, chains and wraps since taken back.
    pub fn rewind(&mut self, c: Checkpoint) {
        self.chains.truncate(c.chains);
        self.chain_at = self.chains.last().map_or(usize::MAX, |c| c[0]);
        self.held.truncate(c.held);
        self.truncate(c.n);
    }

    /// A closed leaf at `at`, before the nodes from there on, which move up one: a token whose
    /// place is known before its parent knows whether the tape stores it (a held token, written at
    /// its parent's close). The nodes before `at` that are closed do not contain it.
    pub fn insert_leaf(&mut self, at: usize, key: u32, start: u32, end: u32) {
        let at = self.insert_node(at, key, start);
        let ws = self.layout.widths();
        // A leaf is its own last descendant: its tail is from its start either way.
        let tail = end.wrapping_sub(start);
        if tail >= self.wide_from {
            self.close_marks(at, end, start, start, tail);
        }
        let c = self.cell(at);
        Self::put(&mut self.size, c, ws.size, 1);
        Self::put(&mut self.tail, c, ws.tail, tail);
    }

    /// Closed leaves put in at once, as `insert_leaf` would put each in turn: `leaves[j]` before
    /// the node at `leaves[j].at` of the tape as it is now (`at` not decreasing; leaves at one
    /// place in their order). One backward pass: each node after the first place moves once, by
    /// the leaves before it, where an insert each moved it once a leaf.
    pub fn insert_leaves(&mut self, leaves: &[Leaf]) {
        let k = leaves.len();
        match k {
            0 => return,
            1 => {
                let l = leaves[0];
                return self.insert_leaf(l.at as usize, l.key, l.start, l.end);
            }
            _ => {}
        }
        debug_assert!(leaves.windows(2).all(|w| w[0].at <= w[1].at));
        if (leaves[0].at as usize) < self.base {
            // Before the window: as `insert_node`, each such leaf at the window's start.
            self.edit_before_passed(leaves[0].at as usize);
            let base = self.base as u32;
            let moved: Vec<Leaf> = leaves.iter().map(|l| Leaf { at: l.at.max(base), ..*l }).collect();
            return self.insert_leaves(&moved);
        }
        debug_assert!(leaves.iter().all(|l| l.at as usize <= self.n));
        while self.n + k - self.base > self.cap {
            self.grow();
        }
        let (n, ws, base) = (self.cell(self.n), self.layout.widths(), self.base);
        let (wz, wt) = (usize::from(ws.size), usize::from(ws.tail));
        // From the last place down: the nodes from leaf j's place to the next leaf's go up by
        // j + 1, then leaf j goes at its place moved up by the j leaves before it.
        let mut to = n;
        for (j, l) in leaves.iter().enumerate().rev() {
            let at = l.at as usize - base;
            let by = j + 1;
            if at < to && to == n {
                // The top run: `shift_up`'s window may write past it, where nothing is yet.
                shift_up::<u8, 64>(&mut self.size, at * wz, to * wz, by * wz);
                shift_up::<u8, 64>(&mut self.tail, at * wt, to * wt, by * wt);
                shift_up::<u32, 16>(&mut self.keys, at, to, by);
                shift_up::<u32, 16>(&mut self.starts, at, to, by);
            } else if at < to {
                // A run below one already moved: exactly its nodes.
                self.size.copy_within(at * wz..to * wz, (at + by) * wz);
                self.tail.copy_within(at * wt..to * wt, (at + by) * wt);
                self.keys.copy_within(at..to, at + by);
                self.starts.copy_within(at..to, at + by);
            }
            to = at;
        }
        self.n += k;
        self.uncode_from(leaves[0].at as usize);
        // A mark's node moves up by the leaves at or before it.
        for m in self.marks.iter_mut() {
            m[0] += leaves.partition_point(|l| l.at <= m[0]) as u32;
        }
        for (j, l) in leaves.iter().enumerate() {
            let node = l.at as usize + j;
            let c = node - base;
            self.keys[c] = l.key;
            self.starts[c] = l.start;
            // A leaf is its own last descendant: its tail is from its start either way.
            let tail = l.end.wrapping_sub(l.start);
            if tail >= self.wide_from {
                self.close_marks(node, l.end, l.start, l.start, tail);
            }
            Self::put(&mut self.size, c, ws.size, 1);
            Self::put(&mut self.tail, c, ws.tail, tail);
        }
    }

    /// `node`'s start, given after it was pushed (a node put before its first child, whose start
    /// it takes; an empty node's). `node` is open: no closed node's tail is measured from it.
    #[inline]
    pub fn set_start(&mut self, node: usize, start: u32) {
        debug_assert!(self.is_open(node));
        if node < self.base {
            return self.set_start_passed(node, start);
        }
        let c = self.cell(node);
        self.starts[c] = start;
    }

    /// The nodes from `len` on taken back (a parse that did not hold).
    pub fn truncate(&mut self, len: usize) {
        if len >= self.n {
            return;
        }
        // Before the window: as `insert_node`, the nodes taken back to the window's start.
        let len = if len < self.base {
            self.edit_before_passed(len);
            self.base
        } else {
            len
        };
        self.n = len;
        self.uncode_from(len);
        self.marks.retain(|m| (m[0] as usize) < len);
        self.wide_from = if self.marks.is_empty() { 0x10000 } else { 0 };
    }

    /// Each block's base (its nodes' least start) and each node's offset from it, in one pass
    /// over the starts at `width` bytes, and whether an offset needs 4 bytes. A block is 16
    /// starts: a min and 16 subtractions, straight-line, which the compiler vectorizes.
    fn offsets(&self, width: u8) -> (Vector<u32>, Vector<u8>, bool) {
        assert!(self.holds_all(), "format 15's columns of a writer whose nodes were passed to format 16's ops");
        let starts = &self.starts[..self.n];
        let mut base = Vector::with_capacity(starts.len().div_ceil(CHECKPOINT_STRIDE));
        let mut off = Vector::with_capacity(starts.len() * usize::from(width));
        let mut high = 0u32;
        for block in starts.chunks(CHECKPOINT_STRIDE) {
            let b = block.iter().copied().fold(u32::MAX, u32::min);
            base.push(b);
            if width == 2 {
                if let Ok(block) = <&[u32; CHECKPOINT_STRIDE]>::try_from(block) {
                    let mut cells = [0u8; 2 * CHECKPOINT_STRIDE];
                    for (cell, &s) in cells.as_chunks_mut::<2>().0.iter_mut().zip(block) {
                        let o = s - b;
                        high |= o;
                        cell.copy_from_slice(&(o as u16).to_le_bytes());
                    }
                    off.extend_from_slice(&cells);
                    continue;
                }
                for &s in block {
                    high |= s - b;
                    off.extend_from_slice(&((s - b) as u16).to_le_bytes());
                }
            } else {
                for &s in block {
                    high |= s - b;
                    off.extend_from_slice(&(s - b).to_le_bytes());
                }
            }
        }
        (base, off, high > 0xffff)
    }

    /// The widths the nodes need, and the bases and offsets (`offsets`): `Redo` where the
    /// writer's widths do not hold them.
    fn check_widths(&mut self) -> Result<(Vector<u32>, Vector<u8>), Redo> {
        debug_assert!(self.chains.is_empty(), "a chain not ended");
        self.commit();
        self.drop_omitted();
        let (base, off, wide) = self.offsets(self.layout.widths().off);
        let has = |what: u8| self.marks.iter().any(|m| m[1] == u32::from(what));
        let from_start = has(EARLY);
        let want = Widths {
            size: width_for(self.n as u32),
            off: if wide { 4 } else { 2 },
            // Tails measured from the other place say nothing of the width: 2, the guess.
            tail: if from_start == self.layout.widths().from_start && has(TAIL_WIDE) { 4 } else { 2 },
            from_start,
        };
        if want != self.layout.widths() {
            return Err(Redo(want));
        }
        Ok((base, off))
    }

    /// The keys of the codes the nodes use, by code id (numbered by first use): what a caller maps to a language's
    /// format 16 numbers for `finish_16`.
    pub fn code_keys(&mut self) -> Vector<u32> {
        self.commit();
        self.codes.iter().map(|&[key, _]| key).collect()
    }

    /// The tape in format 16 or 16z and its presence, where the writer writes it (`to`; decision 0434 §2): the nodes
    /// not yet passed on given to the ops, then their `finish`. Format 15's columns are not made. `Redo` where the
    /// writer's widths did not keep every size and end exactly (format 15's offsets do not matter here); `Ok(Err)`
    /// where the nodes are not a format 16 tape (a code the language's table lacks, an edit before a commit, a node
    /// past format 16's bounds): the writer is kept, and format 15's `finish_columns` takes it.
    pub fn finish_16(&mut self) -> Result<Result<f16::Finished, String>, Redo> {
        debug_assert!(self.chains.is_empty(), "a chain not ended");
        self.commit();
        let w = self.layout.widths();
        let has = |what: u8| self.marks.iter().any(|m| m[1] == u32::from(what));
        // Sizes past their width, an end before the last descendant's start (tails from it), or a tail past its
        // width: not kept exactly.
        // A cell held a size of at most the window's nodes.
        let size = w.size.max(width_for(self.widest.max(self.n - self.base) as u32));
        let from_start = w.from_start || has(EARLY);
        let tail = if from_start == w.from_start && has(TAIL_WIDE) { 4 } else { w.tail };
        let want = Widths { size, off: w.off, tail, from_start };
        if want != w {
            return Err(Redo(want));
        }
        let Some(mut s) = self.sixteen.take() else { return Ok(Err("the writer writes format 15".into())) };
        if let Some(bad) = s.bad.take() {
            return Ok(Err(bad));
        }
        if let Some(node) = s.lost.get() {
            return Ok(Err(format!("node {node} read after its commit")));
        }
        s.close_to(self.n);
        Ok(s.w.finish())
    }

    /// The columns alone, for a parser whose codes the caller makes (a key is the parser's own, not
    /// `kf | flags << 16`): each at its width, and the keys by code id. `Redo` as `finish`. The
    /// size and tail columns are moved out, not copied.
    pub fn finish_columns(mut self) -> Result<Columns, Redo> {
        let (base, off) = self.check_widths()?;
        let w = self.layout.widths();
        let n = self.n;
        let column = |c: &mut Vector<u8>, width: u8| {
            let mut c = std::mem::take(c);
            c.truncate(n * usize::from(width));
            c
        };
        Ok(Columns {
            code: {
                // The code column moved down over the header, in its region: not copied out.
                let mut code = std::mem::take(&mut self.out);
                code.copy_within(HEADER_BYTES..HEADER_BYTES + 2 * n, 0);
                code.truncate(2 * n);
                code
            },
            size: column(&mut self.size, w.size),
            base: base.iter().flat_map(|b| b.to_le_bytes()).collect(),
            off,
            tail: column(&mut self.tail, w.tail),
            widths: [w.size, w.off, w.tail | if w.from_start { TAIL_FROM_START } else { 0 }],
            keys: self.codes.iter().map(|&[key, _]| key).collect(),
        })
    }

    /// The image of the source `src`: the columns moved into place after the code column, then
    /// the code table, and the header. `Redo` when a column's values do not fit its width, or a
    /// tail from its last descendant is negative: the widths to parse again at.
    pub fn finish(mut self, src: &[u8]) -> Result<Vector<u8>, Redo> {
        let (base, off) = self.check_widths()?;
        let w = self.layout.widths();
        let nodes = self.n;
        let class_count = |key: u32| (key >> CLASS_SHIFT).count_ones() as usize;
        let code_bytes: usize = self.codes.iter().map(|&[key, _]| 6 + 2 * class_count(key)).sum();
        let pad = |out: &mut Vector<u8>| out.resize(padded(out.len()), 0);
        let mut out = std::mem::take(&mut self.out);
        out.truncate(HEADER_BYTES + 2 * nodes);
        // Reserved at `new` or `grow` for these nodes and 256 codes: more codes grow it.
        out.reserve(image_bytes(nodes, w) - CODES_RESERVED + code_bytes - out.len());
        pad(&mut out);
        out.extend_from_slice(&self.size[..nodes * usize::from(w.size)]);
        pad(&mut out);
        for b in &base {
            out.extend_from_slice(&b.to_le_bytes());
        }
        pad(&mut out);
        out.extend_from_slice(&off);
        pad(&mut out);
        out.extend_from_slice(&self.tail[..nodes * usize::from(w.tail)]);
        pad(&mut out);
        for &[key, _] in &self.codes {
            out.extend_from_slice(&((key & 0xffff) as u16).to_le_bytes());
            out.push((key >> 16 & 0x7) as u8);
            out.extend_from_slice(&SENTINEL.to_le_bytes());
            out.push(class_count(key) as u8);
            for (j, c) in self.classes.iter().enumerate() {
                if key >> (CLASS_SHIFT + j as u32) & 1 != 0 {
                    out.extend_from_slice(&c.to_le_bytes());
                }
            }
        }
        // The header, written in place at the image's start.
        let mut at = 0;
        let mut put = |b: &[u8]| {
            out[at..at + b.len()].copy_from_slice(b);
            at += b.len();
        };
        put(MAGIC);
        put(&[self.tag]);
        put(&[w.size, w.off, w.tail | if w.from_start { TAIL_FROM_START } else { 0 }]);
        put(&FORMAT_VERSION.to_le_bytes());
        put(&(src.len() as u64).to_le_bytes());
        for length in [nodes, self.codes.len(), code_bytes, 0, 0] {
            put(&(length as u64).to_le_bytes());
        }
        debug_assert_eq!(at, HEADER_BYTES);
        Ok(out)
    }
}

/// A tape's columns as `Writer::finish_columns` gives them: `code` (2 bytes a node, code ids by
/// first use), `size`, `base` (4 bytes each 16 nodes), `off`, `tail`, their widths (`tail`'s with
/// `TAIL_FROM_START`), and each code id's key.
#[derive(Clone, Debug, Default)]
pub struct Columns {
    pub code: Vector<u8>,
    pub size: Vector<u8>,
    pub base: Vector<u8>,
    pub off: Vector<u8>,
    pub tail: Vector<u8>,
    pub widths: [u8; 3],
    pub keys: Vector<u32>,
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

/// An image read in place: each node's key, subtree size and span, straight from the columns, with nothing decoded
/// ahead (`decode` makes every node at once). A node's children are the nodes after it, each the next one's
/// predecessor by its size (`children`).
#[derive(Clone)]
pub struct View<'a> {
    n: usize,
    code: &'a [u8],
    size: &'a [u8],
    base: &'a [u8],
    off: &'a [u8],
    tail: &'a [u8],
    widths: [u8; 3],
    from_start: bool,
    /// Each code's key, by id: read from the code table, or a holder's (`with_keys`).
    keys: std::borrow::Cow<'a, [u32]>,
    /// Where the code table starts, and how many codes it holds.
    table: usize,
    codes: usize,
}

impl<'a> View<'a> {
    /// The image `bytes`, checked as `decode` checks it: the header, the columns' lengths, the code table, and
    /// each code id and size in range.
    pub fn new(bytes: &'a [u8]) -> Result<Self, String> {
        let view = Self::columns(bytes)?;
        view.check()?;
        Ok(view)
    }

    /// How many codes the image's table holds: the length `checked_into`'s keys take.
    pub fn code_count(bytes: &'a [u8]) -> Result<usize, String> {
        Ok(Self::columns_only(bytes)?.codes)
    }

    /// The image `bytes`, checked as `new` checks it, its code table's keys read into `keys` (`code_count` long),
    /// which the caller holds (a block): the view reads them there, and `with_keys` reads them again later.
    pub fn checked_into(bytes: &'a [u8], keys: &'a mut [u32]) -> Result<Self, String> {
        let mut v = Self::columns_only(bytes)?;
        if keys.len() != v.codes {
            return Err(format!("{} keys for a table of {} codes", keys.len(), v.codes));
        }
        Self::read_keys(bytes, v.table, keys)?;
        v.keys = std::borrow::Cow::Borrowed(keys);
        v.check()?;
        Ok(v)
    }

    /// Each node's code in the table and its size within the tape.
    fn check(&self) -> Result<(), String> {
        for i in 0..self.n {
            if usize::from(u16::from_le_bytes([self.code[2 * i], self.code[2 * i + 1]])) >= self.keys.len() {
                return Err("a node's code is not in the table".into());
            }
            if i + self.size(i).max(1) > self.n {
                return Err("a node's size runs past the tape".into());
            }
        }
        Ok(())
    }

    /// The code table at `table` of `bytes`: each code's key, into `out`.
    fn read_keys(bytes: &[u8], table: usize, out: &mut [u32]) -> Result<(), String> {
        let mut c = table;
        for k in out.iter_mut() {
            let entry = bytes.get(c..c + 6).ok_or("a short code table")?;
            *k = u32::from(u16::from_le_bytes([entry[0], entry[1]])) | u32::from(entry[2]) << 16;
            c += 6 + 2 * usize::from(entry[5]);
        }
        Ok(())
    }

    /// An image `new` checked before, `keys` its code table's keys as `keys` gave them then (its holder keeps both,
    /// unchanged): its header and columns read again, the nodes not checked again and the code table not read.
    pub fn with_keys(bytes: &'a [u8], keys: &'a [u32]) -> Self {
        let mut v = Self::columns_only(bytes).expect("an image checked before");
        debug_assert_eq!(keys.len(), v.codes, "the image's own keys");
        v.keys = std::borrow::Cow::Borrowed(keys);
        v
    }

    /// The header, the columns and the code table.
    fn columns(bytes: &'a [u8]) -> Result<Self, String> {
        let mut v = Self::columns_only(bytes)?;
        let mut keys = vec![0; v.codes];
        Self::read_keys(bytes, v.table, &mut keys)?;
        v.keys = std::borrow::Cow::Owned(keys);
        Ok(v)
    }

    /// The header and the columns, and where the code table starts; no key read.
    fn columns_only(bytes: &'a [u8]) -> Result<Self, String> {
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
        let widths = [bytes[header::WIDTHS], bytes[header::WIDTHS + 1], bytes[header::WIDTHS + 2] & !TAIL_FROM_START];
        if widths.iter().any(|&w| w != 2 && w != 4) {
            return Err("a tape column of another width".into());
        }
        let from_start = bytes[header::WIDTHS + 2] & TAIL_FROM_START != 0;
        let (n, code_count) = (u64_at(header::NODES)?, u64_at(header::CODES)?);
        let mut at = HEADER_BYTES;
        let mut column = |width: usize, count: usize| -> Result<&'a [u8], String> {
            let c = bytes.get(at..at + width * count).ok_or("a short tape column")?;
            at = padded(at + width * count);
            Ok(c)
        };
        let w = widths.map(usize::from);
        let (code, size, base, off, tail) = (column(2, n)?, column(w[0], n)?, column(4, n.div_ceil(CHECKPOINT_STRIDE))?, column(w[1], n)?, column(w[2], n)?);
        Ok(View { n, code, size, base, off, tail, widths, from_start, keys: std::borrow::Cow::Borrowed(&[]), table: at, codes: code_count })
    }

    #[inline(always)]
    fn cell(column: &[u8], width: u8, i: usize) -> u32 {
        if width == 2 { u32::from(u16::from_le_bytes([column[2 * i], column[2 * i + 1]])) } else { u32::from_le_bytes(column[4 * i..4 * i + 4].try_into().expect("4 bytes")) }
    }

    /// The nodes.
    #[inline]
    pub fn len(&self) -> usize {
        self.n
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// Node `i`'s key: `kf | flags << 16`, as `Writer::push` took it.
    #[inline(always)]
    pub fn key(&self, i: usize) -> u32 {
        self.keys[usize::from(u16::from_le_bytes([self.code[2 * i], self.code[2 * i + 1]]))]
    }

    /// Node `i`'s subtree, itself counted (at least 1).
    #[inline(always)]
    pub fn size(&self, i: usize) -> usize {
        (Self::cell(self.size, self.widths[0], i) as usize).max(1)
    }

    #[inline(always)]
    pub fn start(&self, i: usize) -> u32 {
        Self::cell(self.base, 4, i / CHECKPOINT_STRIDE).wrapping_add(Self::cell(self.off, self.widths[1], i))
    }

    #[inline(always)]
    pub fn end(&self, i: usize) -> u32 {
        let from = if self.from_start { self.start(i) } else { self.start(i + self.size(i) - 1) };
        from.wrapping_add(Self::cell(self.tail, self.widths[2], i))
    }

    /// Node `i`'s children, in order: from `i + 1`, each the one after the last one's subtree.
    #[inline]
    pub fn children(&self, i: usize) -> Children<'_, 'a> {
        Children { view: self, at: i + 1, end: i + self.size(i) }
    }
}

/// A node's children (`View::children`).
pub struct Children<'v, 'a> {
    view: &'v View<'a>,
    at: usize,
    end: usize,
}

impl Iterator for Children<'_, '_> {
    type Item = usize;

    #[inline(always)]
    fn next(&mut self) -> Option<usize> {
        if self.at >= self.end {
            return None;
        }
        let c = self.at;
        self.at += self.view.size(c);
        Some(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Offsets come from the starts at `finish` (`Writer::offsets`): a block's base is its least
    /// start, and an offset past 0xffff writes the off column at 4 bytes (a `Redo`).
    #[test]
    fn offsets_and_bases_are_computed_at_finish() {
        let src = vec![b' '; 200_000];
        for far in [100u32, 70_000, 150_000] {
            // A root over 40 leaves: the 20th starts `far` in, past its block's base.
            let mut nodes = vec![(key(1, 0, 0), 0, 199_000, 41)];
            for i in 0..40u32 {
                let start = if i == 19 { far } else { i * 10 };
                nodes.push((key(2, 0, 0), start, start + 5, 1));
            }
            let bytes = preorder_image(1, &src, &nodes);
            assert_eq!(bytes[header::WIDTHS + 1], if far > 0xffff { 4 } else { 2 });
            let decoded = decode(&bytes).unwrap();
            let starts: Vec<u32> = decoded.iter().map(|d| d.start).collect();
            let want: Vec<u32> = nodes.iter().map(|n| n.1).collect();
            assert_eq!(starts, want);
            assert_eq!(decoded[20].end, far + 5);
        }
    }

    /// Leaves put in at once (`insert_leaves`) give the tape an insert each gives, runs between
    /// them long and short, leaves at one place, at the end, a wide tail among them.
    #[test]
    fn leaves_inserted_at_once_are_an_insert_each() {
        let src = vec![b' '; 200_000];
        let w = Widths::guess(src.len());
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..300 {
            let n = 1 + (next() % 60) as usize;
            let k = 1 + (next() % 8) as usize;
            let mut places: Vec<u32> = (0..k).map(|_| 1 + (next() % n as u64) as u32).collect();
            places.sort();
            let leaves: Vec<Leaf> = places
                .iter()
                .enumerate()
                .map(|(j, &at)| {
                    let start = 10 * at + j as u32;
                    let end = if next() % 9 == 0 { start + 70_000 } else { start + 1 };
                    Leaf { key: key(4 + j as u16, 0, 0), start, end, at }
                })
                .collect();
            let build = |batch: bool| {
                let mut w = w;
                loop {
                    let mut o = Writer::new(1, w, 8);
                    let r = o.push(key(1, 0, 0), 0);
                    for i in 1..n {
                        let x = o.push(key(2, 0, 0), 10 * i as u32);
                        o.close(x, 10 * i as u32 + 1);
                    }
                    if batch {
                        o.insert_leaves(&leaves);
                    } else {
                        for (j, l) in leaves.iter().enumerate() {
                            o.insert_leaf(l.at as usize + j, l.key, l.start, l.end);
                        }
                    }
                    o.close(r, 199_999);
                    o.commit();
                    match o.finish(&src) {
                        Ok(t) => break decode(&t).unwrap(),
                        // Wider columns where the guess was short (the wide tails).
                        Err(Redo(wider)) => w = wider,
                    }
                }
            };
            assert_eq!(build(false), build(true), "{places:?} over {n}");
        }
    }

    /// A leaf put in later, between closed nodes (`insert_leaf`), gives the tape written in order.
    #[test]
    fn a_leaf_inserted_later_is_the_tape_in_order() {
        let src = vec![b' '; 100];
        let w = Widths::guess(src.len());
        // In order: root(a(x), ",", b(y)).
        let mut o = Writer::new(1, w, 8);
        let r = o.push(key(1, 0, 0), 0);
        let a = o.push(key(2, 0, 0), 0);
        let x = o.push(key(3, 0, 0), 0);
        o.close(x, 3);
        o.close(a, 3);
        let c = o.push(key(4, 0, 0), 4);
        o.close(c, 5);
        let b = o.push(key(2, 0, 0), 6);
        let y = o.push(key(3, 0, 0), 6);
        o.close(y, 9);
        o.close(b, 9);
        o.close(r, 9);
        o.commit();
        // The comma held, then put in at its place before the root closes.
        let mut l = Writer::new(1, w, 8);
        let r = l.push(key(1, 0, 0), 0);
        let a = l.push(key(2, 0, 0), 0);
        let x = l.push(key(3, 0, 0), 0);
        l.close(x, 3);
        l.close(a, 3);
        let b = l.push(key(2, 0, 0), 6);
        let y = l.push(key(3, 0, 0), 6);
        l.close(y, 9);
        l.close(b, 9);
        l.insert_leaf(3, key(4, 0, 0), 4, 5);
        l.close(r, 9);
        l.commit();
        assert_eq!(decode(&o.finish(&src).unwrap()).unwrap(), decode(&l.finish(&src).unwrap()).unwrap());
    }

    /// A parser's writer writing format 16 or 16z (`to`, `finish_16`) gives the tree it finishes in format 15, after
    /// the edits a parse makes: a rewind, a node put before its first child, a key changed. Committed as it goes, the
    /// nodes are passed to the format 16 writer's ops before the edits that reach them: a close, a retag (decision
    /// 0434 §2).
    #[test]
    fn a_parsers_writer_finishes_in_format_16_as_in_format_15() {
        use crate::tape::{CodeTable, Entry, F15, Tape, f15_columns};
        let src = vec![b' '; 100];
        let make = |target: Target, commit: bool| {
            let mut l = Writer::new(1, Widths::guess(src.len()), 8).to(target, &src);
            let r = l.push(key(1, 0, 0), 0);
            let held = l.checkpoint();
            let t = l.push(key(9, 0, 0), 0);
            l.close(t, 1);
            l.rewind(held);
            let x = l.push(key(3, 0, 0), 0);
            l.close(x, 3);
            // An operator found after its left operand: put before it, closed over both operands.
            l.insert(x, key(2, 0, 0), 0);
            let y = l.push(key(3, 0, 0), 6);
            l.close(y, 9);
            if commit {
                l.commit();
            }
            l.close(x, 9);
            l.set_key(x, key(5, 0, 0));
            let c = l.push(key(4, 0, 0), 9);
            l.close(c, 10);
            l.close(r, 10);
            l
        };
        let image = make(Target::F15, false).finish(&src).unwrap();
        let (columns, table) = f15_columns(&image).unwrap();
        let f15 = F15::new(&image, columns, &table).unwrap();
        // The table's codes, and the operator's first key after them: each key's number its place.
        let keys = make(Target::F15, false).code_keys();
        let mut entries: Vec<Entry> = table.entries().map(|e| e.to_owned()).collect();
        entries.push(Entry { kind: 2, field: 0, flags: 0, alias: None, classes: Vec::new() });
        let lang: &'static f16::Lang = Box::leak(Box::new(f16::Lang::new(CodeTable::new(entries), [1, 1, 1], 1).unwrap()));
        for (z, commit) in [(false, false), (true, false), (false, true), (true, true)] {
            let keys = keys.clone();
            let number = move |k: u32| keys.iter().position(|&x| x == k).map(|p| p as u32).or((k == key(2, 0, 0)).then_some(4));
            let target = Target::F16(Sixteen { lang, tag: 1, z, blocks: false, number: std::sync::Arc::new(number) });
            let mut w = make(target, commit);
            let written = w.finish_16().unwrap().unwrap().tape;
            let check = |t: &dyn Fn(usize) -> (u32, usize, Option<usize>, u64, u64), n: usize| {
                assert_eq!(n, f15.len());
                for i in 0..n {
                    assert_eq!(t(i), (f15.code(i), f15.subtree_end(i), f15.parent(i), f15.start(i), f15.end(i)), "node {i}, 16z {z}");
                }
            };
            match written {
                f16::Written::F16(t) => {
                    let r = t.reader(lang).unwrap();
                    check(&|i| (r.code(i), r.subtree_end(i), r.parent(i), r.start(i), r.end(i)), r.len());
                }
                f16::Written::F16Z(t) => {
                    let cache = f16::ZCache::of([&t]);
                    let r = t.reader(lang, Some(&cache)).unwrap();
                    check(&|i| (r.code(i), r.subtree_end(i), r.parent(i), r.start(i), r.end(i)), r.len());
                }
            }
        }
    }

    /// A format 16 writer's columns hold the window, the nodes since the last commit: a file of many statements
    /// keeps them at `WINDOW` cells, and its tape is format 15's tree. The parse reads its open ancestors after a
    /// commit; a read of a node given back, or an edit before the window, says the tape is not format 16's
    /// (`finish_16`'s reason), and `holds_all` says the parse must run again for format 15 (decision 0434 §2).
    #[test]
    fn a_format_16_writers_columns_are_the_window() {
        use crate::tape::{CodeTable, F15, Tape, f15_columns};
        let statements = 3 * WINDOW / 4;
        let src = vec![b' '; 4 * statements + 1];
        // A module of statements, each an expression statement over two names, committed as it ends.
        let make = |target: Target, misread: bool, misedit: bool| {
            let mut l = Writer::new(1, Widths::guess(src.len()), 8).to(target, &src);
            let module = l.push(key(1, 0, 0), 0);
            for s in 0..statements {
                let at = (4 * s) as u32;
                let st = l.push(key(2, 0, 0), at);
                let a = l.push(key(3, 0, 0), at);
                l.close(a, at + 1);
                let b = l.push(key(3, 0, 0), at + 2);
                l.close(b, at + 3);
                // The parse reads its open ancestors after the commits: the module's start and key.
                assert_eq!((l.key(module), l.start(module), l.is_open(module)), (key(1, 0, 0), 0, true));
                l.close(st, at + 3);
                l.commit();
                if s == 2 && misread {
                    l.end(st);
                }
                if s == 2 && misedit {
                    l.insert_leaf(st, key(3, 0, 0), at, at);
                }
            }
            l.close(module, src.len() as u32);
            l
        };
        let keys = make(Target::F15, false, false).code_keys();
        let image = make(Target::F15, false, false).finish(&src).unwrap();
        let (columns, table) = f15_columns(&image).unwrap();
        let f15 = F15::new(&image, columns, &table).unwrap();
        let lang: &'static f16::Lang = Box::leak(Box::new(f16::Lang::new(CodeTable::new(table.entries().map(|e| e.to_owned()).collect()), [1, 1, 1], 1).unwrap()));
        let number = std::sync::Arc::new(move |k: u32| keys.iter().position(|&x| x == k).map(|p| p as u32));
        let target = |z| Target::F16(Sixteen { lang, tag: 1, z, blocks: true, number: number.clone() });
        // A large file's guess is cut to the window.
        assert_eq!(Writer::new(1, Widths::guess(1 << 20), 1 << 16).to(target(false), &src).cap, WINDOW);
        for z in [false, true] {
            let mut w = make(target(z), false, false);
            // Made for 8 nodes: the 16 cells it began with hold every statement's.
            assert_eq!(w.cap, CHECKPOINT_STRIDE, "the columns stay at the window");
            assert!(!w.holds_all());
            let written = w.finish_16().unwrap().unwrap().tape;
            let row = |r: &dyn Fn(usize) -> (u32, usize, Option<usize>, u64, u64), n: usize| {
                assert_eq!(n, f15.len());
                for i in 0..n {
                    assert_eq!(r(i), (f15.code(i), f15.subtree_end(i), f15.parent(i), f15.start(i), f15.end(i)), "node {i}, 16z {z}");
                }
            };
            match written {
                f16::Written::F16(t) => {
                    let r = t.reader(lang).unwrap();
                    row(&|i| (r.code(i), r.subtree_end(i), r.parent(i), r.start(i), r.end(i)), r.len());
                }
                f16::Written::F16Z(t) => {
                    let cache = f16::ZCache::of([&t]);
                    let r = t.reader(lang, Some(&cache)).unwrap();
                    row(&|i| (r.code(i), r.subtree_end(i), r.parent(i), r.start(i), r.end(i)), r.len());
                }
            }
            for (misread, misedit, why) in [(true, false, "read after its commit"), (false, true, "before the last commit")] {
                let mut w = make(target(z), misread, misedit);
                let e = w.finish_16().unwrap().err().expect("not format 16's");
                assert!(e.contains(why), "{e}");
                assert!(!w.holds_all());
            }
        }
    }

}
