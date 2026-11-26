//! The engine's tape (format 16 and 16z, decisions 0293 and 0434) and the writer a lean parser writes it with as
//! it parses (decision 0140).
//!
//! A lean parser gives its nodes in preorder to a `Writer`, which passes each statement's nodes to format 16's ops
//! (`f16::Writer`) as the parse commits them: its own columns hold only the nodes since the last commit, the window
//! the parse may still edit. The format's constants are these, and a format change is made here, once, for every
//! writer.

pub mod tape;
pub mod f16;
pub mod home;
pub mod newlines;
/// The tape's reads in Rust, written directly (decision 0534): this crate's readers'.
pub mod reads;
/// The views' reads in Rust, written directly (decision 0353 step 3 on the CPU): the host executor's reads of a
/// document's views' rows, through its entry in its batch's views' directory.
pub mod view_reads;
/// The tape's writer in Rust, written directly (decision 0532): the host's, and a generated tier 2's `ops` module.
pub mod writer16;
/// A tape's auxiliary index (decision 0656): the names stream, the vocabulary, sibling places, code counts and rare
/// codes, made once from a tape and read in place, in Rust and in UKL.
pub mod aux_index;
pub mod doc_index;

pub use rerail_vector::{Pod, vector::Vector};

pub const CODE_EXTRA: u8 = 1;
pub const CODE_ERROR: u8 = 2;
pub const CODE_MISSING: u8 = 4;
pub const KIND_LIMIT: usize = 1024;
pub const FIELD_LIMIT: usize = 64;
/// No value: a cell of an index past its width (the engine's `tape::index`).
pub const SENTINEL: u16 = u16::MAX;
/// ERROR's `kf`.
pub const ERROR_CODE: u16 = writer16::ERROR_CODE;
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

/// A source's SHA-256 (not in the header since format 12; the engine keeps it in memory).
pub fn digest(src: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(src).into()
}

/// The narrowest width (2 or 4 bytes) that holds `max`.
pub fn width_for(max: u32) -> u8 {
    if max <= 0xffff { 2 } else { 4 }
}

/// The widths a writer's window holds its nodes at (`Writer::new`): `size` and `tail` in bytes (2 or 4), and whether
/// tails are from the node's own start (else from its last descendant's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Widths {
    pub size: u8,
    pub tail: u8,
    pub from_start: bool,
}

impl Widths {
    /// The guess for a source of `len` bytes (decision 0140). A tail is at most the source's
    /// length, and a `.rerail` file has fewer nodes than bytes: below 64 KiB every column fits 2
    /// bytes, so nearly every file is written once. Above it, tails stay 2 (they are short
    /// distances), and sizes take 4 from `SIZE_GUESS_BYTES` on.
    pub fn guess(len: usize) -> Self {
        Widths { size: if len < SIZE_GUESS_BYTES { 2 } else { 4 }, tail: 2, from_start: false }
    }
}

/// A writer's widths, as a type: `Narrow`, the widths nearly every file is written at, fixed
/// when the writer is compiled, so that a store is one instruction; or `Widths`, any, read
/// as the writer runs.
pub trait Layout: Copy {
    fn widths(self) -> Widths;
}

/// Sizes and tails at 2 bytes, tails from the last descendant.
#[derive(Clone, Copy, Debug, Default)]
pub struct Narrow;

impl Narrow {
    pub const WIDTHS: Widths = Widths { size: 2, tail: 2, from_start: false };
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

/// A key is below this (`kf` 16 bits, flags 3).
const ID_SLOTS: usize = 1 << 19;
/// An empty slot of the writer's id table (`Ids`).
const NO_ID: u64 = u64::MAX;
/// The id table's first size, in slots (a power of two): a document uses a few hundred codes at most.
const IDS_FIRST: u32 = 9;

/// The writer's codes' ids by key, open-addressed (linear probing, at most half full): each slot a key and its id
/// (`key << 16 | id`), or `NO_ID`. Indexed by the key itself it was the whole key space, a 1 MB table filled at
/// each writer, which was most of a small document's parse (a rule file's, written five times a run).
struct Ids {
    slots: Vector<u64>,
    bits: u32,
}

impl Ids {
    fn new() -> Self {
        Ids { slots: filled(1 << IDS_FIRST, NO_ID), bits: IDS_FIRST }
    }

    #[inline(always)]
    fn home(&self, key: u32) -> usize {
        (u64::from(key).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> (64 - self.bits)) as usize
    }

    /// `key`'s id, or where it goes (`Err`: its empty slot).
    #[inline(always)]
    fn find(&self, key: u32) -> Result<u16, usize> {
        let mask = self.slots.len() - 1;
        let mut at = self.home(key);
        loop {
            let e = self.slots[at];
            if e == NO_ID {
                return Err(at);
            }
            if (e >> 16) as u32 == key {
                return Ok(e as u16);
            }
            at = (at + 1) & mask;
        }
    }

    /// `key`'s id, given it as `next` where it has none (`codes` the ids' keys, in order, which a full table is
    /// made again from).
    #[inline(always)]
    fn id_of(&mut self, key: u32, next: usize, codes: &[[u32; 2]]) -> (u16, bool) {
        match self.find(key) {
            Ok(id) => (id, false),
            Err(at) => {
                let at = if 2 * (next + 1) > self.slots.len() { self.grow(key, codes) } else { at };
                self.slots[at] = u64::from(key) << 16 | next as u64;
                (next as u16, true)
            }
        }
    }

    /// Twice the slots, each code put again in its order (as if inserted so: `forget` stays exact); `key`'s slot.
    #[cold]
    #[inline(never)]
    fn grow(&mut self, key: u32, codes: &[[u32; 2]]) -> usize {
        self.bits += 1;
        self.slots = filled(1 << self.bits, NO_ID);
        for (id, &[k, _]) in codes.iter().enumerate() {
            let Err(at) = self.find(k) else { unreachable!("a code twice") };
            self.slots[at] = u64::from(k) << 16 | id as u64;
        }
        self.find(key).expect_err("a new code")
    }

    /// `key`, the code given last, taken back: its slot emptied. Exact because codes are taken back last first
    /// (`uncode_from`): no code given before it was placed past its slot.
    fn forget(&mut self, key: u32) {
        let mask = self.slots.len() - 1;
        let mut at = self.home(key);
        while (self.slots[at] >> 16) as u32 != key {
            debug_assert_ne!(self.slots[at], NO_ID, "a code forgotten that was never given");
            at = (at + 1) & mask;
        }
        self.slots[at] = NO_ID;
    }
}

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

/// A tape written as a parser parses, in format 16 or 16z (`to`). The parser gives each node in preorder (`push`),
/// its end and so its size at its close (`close`), and edits only the nodes it made last: a code changed
/// (`set_key`), a node put before the ones from a place on (`insert`), the nodes from a place on taken back
/// (`truncate`). Each edit rewrites only the nodes from its place on. At a commit the nodes so far are final, and are
/// passed to format 16's ops; at the end (`finish_16`) the rest are, and the ops finish the tape.
///
/// A node's key is its code: `kf | flags << 16`.
pub struct Writer<L: Layout = Widths> {
    /// The window's columns, each `cap` cells; the first `n - base` are the nodes' from `base` on (`cell`): each
    /// node's code id (written at its commit), and its size and tail at their widths.
    code: Vector<u16>,
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
    ids: Ids,
    layout: L,
    /// An offset or tail from which a push or close takes the long way: 0x10000, or 0 once a
    /// node is marked (then every node does, and its marks are kept exact).
    wide_from: u32,
    /// The nodes whose tail needs 4 bytes, or that end before their last descendant starts
    /// (`TAIL_WIDE`, `EARLY`): few, kept exact through every edit, so that `finish_16` knows
    /// whether the widths held every tail without reading the tail column.
    marks: Vector<[u32; 2]>,
    /// The open chains (`open_chain`), innermost last: each its node and where its wraps begin in
    /// `held`. `chain_at` is the innermost's node (`usize::MAX`: none), the one compare `close`,
    /// `key` and `set_key` make.
    chains: Vector<[usize; 2]>,
    chain_at: usize,
    /// The wraps of the open chains, not yet on the tape: each its key, start, and at its close the
    /// nodes there were then and its end (0 nodes: open).
    held: Vector<Held>,
    /// Where the run writes format 16 or 16z (`to`): the nodes before `passed` are given to its ops, and an edit
    /// reaches before them only to retag a node or set an open node's start (decision 0434 §2). `Err`: why the
    /// writer writes no tape (no target, or a language without a format 16 table); its columns then hold every node.
    sixteen: Result<Box<Passing>, String>,
    passed: usize,
    /// The node the columns' first cells are: the nodes before it were passed to format 16's ops, and their cells
    /// given back (the window, decision 0434 §2).
    base: usize,
    /// The most nodes the window has held at a commit: the sizes its cells held.
    widest: usize,
    /// The source ranges of the placeholders a program's tape leaves out (`omitting`), sorted, apart: a node whose
    /// span (not empty) lies in one is written with none of its subtree, as an embedded program's tape is
    /// (`embed::segments::SegmentMap::omitted`). Empty for every other tape.
    omit: Vector<[u32; 2]>,
}

/// Where a lean parser's writer writes (decision 0434 §2): format 16 or 16z, each node given to `f16::Writer`'s ops
/// as it is committed, the writer's columns the edit window; or why it writes no tape (a language without a format
/// 16 table).
pub type Target = Result<Sixteen, String>;

/// A format 16 target: the language's table, the line terminators of its ecosystem (its line index's, decision
/// 0612), its tag, 16z where `z`, each block's presence where `blocks`, and each key's number in the table (`None`: a
/// code the table lacks, and the writer writes no tape).
#[derive(Clone)]
pub struct Sixteen {
    pub lang: &'static f16::Lang,
    pub terminators: newlines::Terminators,
    pub tag: u8,
    pub z: bool,
    pub blocks: bool,
    pub number: std::sync::Arc<dyn Fn(u32) -> Option<u32> + Send + Sync>,
    /// Where the tape goes: rooms every reader reads it in (decision 0353 step 4a), or `None`, a vector of its own.
    pub rooms: Option<std::sync::Arc<dyn home::Rooms>>,
}

/// A node's key named as its grammar names it: what a parser without the grammar (a lean parser's own tests and
/// tools) gives for each key, numbered in its language's table by these names (`f16::Lang::number_named`, `Named`).
#[derive(Clone, Copy, Debug)]
pub struct KeyNames {
    pub kind: &'static str,
    pub named: bool,
    /// `""`: no field.
    pub field: &'static str,
    pub flags: u8,
    pub classes: &'static [&'static str],
}

/// A lean parser's own sink (decision 0631): the tape in format 16 in its language's table (tag `tag`), each key
/// numbered by its names (`name`), the language's line `terminators`. What the parser's crate writes its tapes with,
/// without the grammar; the engine gives its own (its ids from the grammar), whose tapes are the same.
pub struct Named {
    target: Result<Sixteen, String>,
}

impl Named {
    pub fn new(tag: u8, terminators: newlines::Terminators, name: impl Fn(u32) -> Option<KeyNames> + Send + Sync + 'static) -> Self {
        let target = f16::lang(tag).ok_or_else(|| format!("tape tag {tag} has no format 16 table")).map(|lang| Sixteen {
            lang,
            terminators,
            tag,
            z: false,
            blocks: true,
            number: std::sync::Arc::new(move |key| lang.number_named(&name(key)?)),
            rooms: None,
        });
        Named { target }
    }
}

impl Sink<Result<f16::Finished, String>> for Named {
    fn target(&self) -> Target {
        self.target.clone()
    }
    fn finish(&mut self, mut w: Writer) -> Result<Result<f16::Finished, String>, Redo> {
        w.finish_16()
    }
}

impl Finish for Named {
    type Out = Result<f16::Finished, String>;
    fn target(&self) -> Target {
        self.target.clone()
    }
    fn finish<L: Layout>(&mut self, mut w: Writer<L>) -> Result<Self::Out, Redo> {
        w.finish_16()
    }
}

/// What a lean parser's writer writes to and is finished into: its target, given when the writer is made
/// (`Writer::to`), and its finish.
pub trait Sink<Out> {
    fn target(&self) -> Target;
    /// The source ranges the tape leaves out (`Writer::omitting`): an embedded program's placeholders. None for
    /// every other tape.
    fn omissions(&self) -> Vector<[u32; 2]> {
        Vector::new()
    }
    fn finish(&mut self, w: Writer) -> Result<Out, Redo>;
    /// Whether the parse is to run again at the widths it had, where `finish` asked for those widths again (a
    /// sink that changed what it writes). Asked once a `Redo`.
    fn again(&mut self) -> bool {
        false
    }
}

/// A sink's output kept aside, so that a parser writes through `Sink<()>`: its code is then one instance, compiled in
/// its own crate (optimized in dev builds, `Cargo.toml`'s profiles), not one for each output type in the crate that
/// calls it (the engine's, unoptimized there). A parser's entry is a shell over its `_kept` function
/// (`rerail_python_lean::tape_into`): `Kept::new(finish)`, the parse, then `take`.
pub struct Kept<'a, T> {
    sink: &'a mut dyn Sink<T>,
    out: Option<T>,
}

impl<'a, T> Kept<'a, T> {
    pub fn new(sink: &'a mut dyn Sink<T>) -> Self {
        Kept { sink, out: None }
    }

    /// What the sink's `finish` gave, once the parse finished its tape.
    pub fn take(&mut self) -> T {
        self.out.take().expect("a parse that finished its tape")
    }
}

impl<T> Sink<()> for Kept<'_, T> {
    fn target(&self) -> Target {
        self.sink.target()
    }
    fn omissions(&self) -> Vector<[u32; 2]> {
        self.sink.omissions()
    }
    fn finish(&mut self, w: Writer) -> Result<(), Redo> {
        self.out = Some(self.sink.finish(w)?);
        Ok(())
    }
    fn again(&mut self) -> bool {
        self.sink.again()
    }
}

/// The format 16 writer the nodes are passed to as they are committed (`Writer::pass`).
struct Passing {
    /// The thread's format 16 writer (`f16::Writer::reused`), given back when the parse is done.
    w: std::mem::ManuallyDrop<f16::Writer>,
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

impl Drop for Passing {
    fn drop(&mut self) {
        // SAFETY: taken once, here, and not used after.
        unsafe { std::mem::ManuallyDrop::take(&mut self.w) }.give_back();
    }
}

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

/// The cells a writer's columns hold at least.
const MIN_CELLS: usize = 16;

/// `n` copies of `v`, in the run's room (`rerail_vector::default_room`).
fn filled<T: Pod + Copy>(n: usize, v: T) -> Vector<T> {
    let mut c = Vector::with_capacity(n);
    c.resize(n, v);
    c
}

/// What a parser's writer is finished into, whatever the type of its widths (`Layout`): format 16 or 16z
/// (`Writer::finish_16`). For a parser generic over its layout; others take a `Sink`.
pub trait Finish {
    type Out;
    /// What the writer writes to (`Writer::to`).
    fn target(&self) -> Target;
    fn finish<L: Layout>(&mut self, w: Writer<L>) -> Result<Self::Out, Redo>;
    /// Whether the parse is to run again at the widths it had, where `finish` asked for those widths again (as
    /// `Sink::again`).
    fn again(&mut self) -> bool {
        false
    }
}

/// The widths a tape needs, where the writer's did not hold its values: parse again at these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Redo(pub Widths);

impl<L: Layout> Writer<L> {
    /// A writer for about `nodes` nodes (more grow it) at `layout`'s widths. It writes no tape until it is given
    /// its target (`to`).
    pub fn new(layout: L, nodes: usize) -> Self {
        let widths = layout.widths();
        let cap = nodes.max(MIN_CELLS);
        let cells = |w: u8| filled(cap * usize::from(w), 0u8);
        Writer {
            code: filled(cap, 0),
            size: cells(widths.size),
            tail: cells(widths.tail),
            starts: filled(cap, 0),
            keys: filled(cap, 0),
            n: 0,
            cap,
            codes: Vector::new(),
            coded: 0,
            ids: Ids::new(),
            layout,
            wide_from: 0x10000,
            marks: Vector::new(),
            chains: Vector::new(),
            chain_at: usize::MAX,
            held: Vector::new(),
            sixteen: Err("the writer was given no target".into()),
            passed: 0,
            base: 0,
            widest: 0,
            omit: Vector::new(),
        }
    }

    /// Node `node`'s cell in the columns (`base`).
    #[inline(always)]
    fn cell(&self, node: usize) -> usize {
        debug_assert!(node >= self.base, "node {node} before the window's {}", self.base);
        node - self.base
    }

    /// The writer leaving out the subtrees in the source ranges `omit` (sorted, apart; `Sink::omissions`): an
    /// embedded program's placeholders, left out as the parse writes them (format 16's ops are never given them,
    /// `pass`), never by a walk of the tape after.
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

    /// Whether an open node starting at `start` may yet be omitted (its end inside the range it starts in), its end
    /// at least `reached` (the start of a node it holds): one holding a node past the range's end is kept.
    #[inline]
    fn may_be_omitted(&self, start: u32, reached: u32) -> bool {
        let k = self.omit.partition_point(|r| r[1] <= start);
        self.omit.get(k).is_some_and(|r| r[0] <= start && reached <= r[1])
    }

    /// The writer writing to `target` for the source `src`: format 16's nodes passed to its ops as they are
    /// committed (decision 0434 §2). Before the first push.
    pub fn to(mut self, target: Target, src: &[u8]) -> Self {
        assert_eq!(self.n, 0, "a target before the first node");
        let t = match target {
            Ok(t) => t,
            Err(e) => {
                self.sixteen = Err(e);
                return self;
            }
        };
        // The thread's writer, used again (t60 step 5), its tape written in the target's rooms (0353 row 17).
        let w = std::mem::ManuallyDrop::new(f16::Writer::reused_in(t.rooms.clone(), t.lang, src, t.terminators, t.tag, t.z, t.blocks, self.cap));
        self.sixteen = Ok(Box::new(Passing { w, number: t.number, numbers: Vector::new(), open: Vector::new(), bad: None, lost: Default::default() }));
        // The columns hold the window: the nodes since the last commit (more grow it).
        if self.cap > WINDOW {
            let (cap, widths) = (WINDOW, self.layout.widths());
            let cells = |w: u8| filled(cap * usize::from(w), 0u8);
            (self.code, self.size, self.tail, self.starts, self.keys, self.cap) = (filled(cap, 0), cells(widths.size), cells(widths.tail), filled(cap, 0), filled(cap, 0), cap);
        }
        self
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
        let s = self.sixteen.as_ref().ok()?;
        let found = s.open.iter().rev().find(|e| e.node == node).copied();
        if found.is_none() {
            s.lost.set(Some(node));
        }
        found
    }

    /// The committed nodes not yet passed on given to the format 16 writer's ops in preorder: each opened, its number
    /// its code's, and closed where its subtree ends before the window.
    fn pass(&mut self) {
        let mut s = match std::mem::replace(&mut self.sixteen, Err(String::new())) {
            Ok(s) => s,
            Err(e) => {
                self.sixteen = Err(e);
                return;
            }
        };
        let to = self.coded;
        if s.bad.is_none() {
            let mut i = self.passed;
            while i < to {
                s.close_to(i);
                // A closed subtree in an omitted placeholder (`omitting`): its ops never given. An open node holds
                // every node pushed after it (preorder), so it ends at least where the last one starts: past its
                // range's end, it is kept. One that may yet be omitted cannot be told, and the parse is refused.
                if !self.omit.is_empty() {
                    let (start, size) = (self.starts[self.cell(i)], self.size(i) as usize);
                    if size > 0 && self.omitted(start, self.end(i)) {
                        i += size;
                        continue;
                    }
                    if size == 0 && self.may_be_omitted(start, self.starts[self.cell(self.n - 1)]) {
                        s.fail(|| format!("node {i}, open at a commit, may be in an omitted placeholder"));
                        break;
                    }
                }
                let c = self.cell(i);
                let id = usize::from(self.code[c]);
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
        self.sixteen = Ok(s);
    }

    /// An edit at `at` (an insert, a truncate): one before the nodes passed on means the tape is not format 16's.
    #[cold]
    #[inline(never)]
    fn edit_before_passed(&mut self, at: usize) {
        let passed = self.passed;
        if let Ok(s) = &mut self.sixteen {
            s.fail(|| format!("an edit at node {at}, before the last commit's {passed}"));
        }
    }

    /// `close` of a node passed on while open: its close given to the ops once its subtree is.
    #[cold]
    #[inline(never)]
    fn close_passed(&mut self, node: usize, end: u32) {
        let n = self.n;
        let Ok(s) = &mut self.sixteen else { return };
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
        let Ok(s) = &mut self.sixteen else { return };
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
        let Ok(s) = &mut self.sixteen else { return };
        s.w.set_start(node, u64::from(start));
        match s.open.iter_mut().rev().find(|e| e.node == node) {
            Some(e) => e.start = start,
            None => s.lost.set(Some(node)),
        }
    }

    /// Room for twice the nodes: every column at once.
    #[cold]
    #[inline(never)]
    fn grow(&mut self) {
        let cap = 2 * self.cap;
        let w = self.layout.widths();
        self.code.resize(cap, 0);
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
            // A node given back reads as a leaf: a walk over siblings (`c += size(c)`) still moves on, and the tape
            // is refused (`passed_node`).
            return self.passed_node(node).map_or(1, |e| if e.end_at == usize::MAX { 0 } else { (e.end_at - node) as u32 });
        }
        self.size_here(node)
    }

    #[inline(always)]
    fn size_here(&self, node: usize) -> u32 {
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
        self.end_here(node)
    }

    #[inline(always)]
    fn end_here(&self, node: usize) -> u32 {
        let w = self.layout.widths();
        let c = self.cell(node);
        let at = c * usize::from(w.tail);
        let tail = if w.tail == 2 { u32::from(u16::from_le_bytes([self.tail[at], self.tail[at + 1]])) } else { u32::from_le_bytes(self.tail[at..at + 4].try_into().expect("4 bytes")) };
        let from = if w.from_start { self.starts[c] } else { self.starts[c + self.size_here(node) as usize - 1] };
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
            self.ids.forget(key);
            self.codes.pop();
        }
        self.coded = node;
        if let Ok(s) = &mut self.sixteen {
            s.numbers.truncate(self.codes.len());
        }
    }

    /// The nodes so far are final (a statement parsed or recovered): their codes, numbered by
    /// first use, written to the code column, and the nodes passed to format 16's ops.
    pub fn commit(&mut self) {
        let (from, to) = (self.coded, self.n);
        let (a, b) = (self.cell(from), self.cell(to));
        // The columns as locals: a store to one is not a store to the writer's fields.
        let keys = &self.keys[a..b];
        let code = &mut self.code[a..b];
        let (ids, codes) = (&mut self.ids, &mut self.codes);
        for (k, (&key, cell)) in keys.iter().zip(code).enumerate() {
            let (id, new) = ids.id_of(key, codes.len(), codes);
            if new {
                codes.push([key, (from + k) as u32]);
            }
            *cell = id;
        }
        self.coded = to;
        if self.sixteen.is_ok() {
            self.pass();
        }
    }

    /// A node at the end, in preorder: its key and start. Its size and tail are written at its
    /// close: every node is closed before `finish_16`. Its code is written when it is committed.
    #[inline(always)]
    pub fn push(&mut self, key: u32, start: u32) -> usize {
        debug_assert!((key as usize) < ID_SLOTS);
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

    /// A closed leaf at the end, in preorder: `push` and `close` at once, as a token is written (its tail from its start,
    /// as `insert_leaf`'s). At a chain's node, the two.
    #[inline(always)]
    pub fn push_leaf(&mut self, key: u32, start: u32, end: u32) -> usize {
        if self.n == self.chain_at {
            let at = self.push(key, start);
            self.close(at, end);
            return at;
        }
        self.leaf_here(key, start, end)
    }

    #[inline(always)]
    fn leaf_here(&mut self, key: u32, start: u32, end: u32) -> usize {
        let at = self.push(key, start);
        let tail = end.wrapping_sub(start);
        if tail >= self.wide_from {
            self.close_marks(at, end, start, start, tail);
        }
        let (c, ws) = (self.cell(at), self.layout.widths());
        Self::put(&mut self.size, c, ws.size, 1);
        Self::put(&mut self.tail, c, ws.tail, tail);
        at
    }

    /// `node`'s key given the bits `field` where its bits `mask` are none (a field set after the node was made, where
    /// it has none): `key` and `set_key` at once.
    #[inline]
    pub fn set_field(&mut self, node: usize, mask: u32, field: u32) {
        if node == self.chain_at || node < self.base {
            let k = self.key(node);
            if k & mask == 0 {
                self.set_key(node, k | field);
            }
            return;
        }
        self.field_here(node, mask, field);
    }

    #[inline(always)]
    fn field_here(&mut self, node: usize, mask: u32, field: u32) {
        let c = self.cell(node);
        let k = self.keys[c];
        if k & mask == 0 {
            self.uncode_from(node);
            self.keys[c] = k | field;
        }
    }

    /// `node` closed at `end`, or at its start where that is later (a node of no token), as `close`: its key, read before.
    /// `key`, `start` and `close` at once.
    #[inline(always)]
    pub fn close_from(&mut self, node: usize, end: u32) -> u32 {
        if node == self.chain_at || node < self.base {
            let k = self.key(node);
            let e = end.max(self.start(node));
            self.close(node, e);
            return k;
        }
        self.close_from_here(node, end)
    }

    #[inline(always)]
    fn close_from_here(&mut self, node: usize, end: u32) -> u32 {
        let c = self.cell(node);
        let (k, s) = (self.keys[c], self.starts[c]);
        self.close_at(node, end.max(s), self.n);
        k
    }

    /// A node of key `key` put over node `at`, open (an operator found after its left operand, as `insert`): it takes
    /// `at`'s key's bits `mask`, and those become `bits` (the wrapped node's field). `key`, `set_key`, `start` and
    /// `insert` at once.
    #[inline(always)]
    pub fn wrap(&mut self, at: usize, mask: u32, bits: u32, key: u32) {
        if at == self.chain_at || at < self.base {
            let k = self.key(at);
            self.set_key(at, k & !mask | bits);
            let start = self.start(at);
            self.insert(at, key | (k & mask), start);
            return;
        }
        self.wrap_here(at, mask, bits, key);
    }

    #[inline(always)]
    fn wrap_here(&mut self, at: usize, mask: u32, bits: u32, key: u32) {
        let c = self.cell(at);
        let (k, start) = (self.keys[c], self.starts[c]);
        self.uncode_from(at);
        self.keys[c] = k & !mask | bits;
        self.insert_node(at, key | (k & mask), start);
    }

    /// Whether the tape holds no chain and has passed no node on: a parse that opens no chain and commits only at its
    /// finish (tier 2's), whose calls are then the `plain_` ones.
    fn plain(&self) -> bool {
        self.chain_at == usize::MAX && self.chains.is_empty() && self.base == 0
    }

    /// `key`, on a tape with no chain held and no node passed on (`plain`): neither checked.
    #[inline(always)]
    pub fn plain_key(&self, node: usize) -> u32 {
        debug_assert!(self.plain());
        self.keys[node]
    }

    /// `start`, on a plain tape.
    #[inline(always)]
    pub fn plain_start(&self, node: usize) -> u32 {
        debug_assert!(self.plain());
        self.starts[node]
    }

    /// `set_key`, on a plain tape.
    #[inline(always)]
    pub fn plain_set_key(&mut self, node: usize, key: u32) {
        debug_assert!(self.plain());
        self.uncode_from(node);
        self.keys[node] = key;
    }

    /// `set_start`, on a plain tape.
    #[inline(always)]
    pub fn plain_set_start(&mut self, node: usize, start: u32) {
        debug_assert!(self.plain() && self.is_open(node));
        self.starts[node] = start;
    }

    /// `close`, on a plain tape.
    #[inline(always)]
    pub fn plain_close(&mut self, node: usize, end: u32) {
        debug_assert!(self.plain());
        self.close_at(node, end, self.n);
    }

    /// `push_leaf`, on a plain tape.
    #[inline(always)]
    pub fn plain_leaf(&mut self, key: u32, start: u32, end: u32) -> usize {
        debug_assert!(self.plain());
        self.leaf_here(key, start, end)
    }

    /// `set_field`, on a plain tape.
    #[inline(always)]
    pub fn plain_field(&mut self, node: usize, mask: u32, field: u32) {
        debug_assert!(self.plain());
        self.field_here(node, mask, field);
    }

    /// `close_from`, on a plain tape.
    #[inline(always)]
    pub fn plain_close_from(&mut self, node: usize, end: u32) -> u32 {
        debug_assert!(self.plain());
        self.close_from_here(node, end)
    }

    /// `size`, on a plain tape.
    #[inline(always)]
    pub fn plain_size(&self, node: usize) -> u32 {
        debug_assert!(self.plain());
        self.size_here(node)
    }

    /// `end`, on a plain tape.
    #[inline(always)]
    pub fn plain_end(&self, node: usize) -> u32 {
        debug_assert!(self.plain());
        self.end_here(node)
    }

    /// `truncate`, on a plain tape.
    #[inline]
    pub fn plain_truncate(&mut self, len: usize) {
        debug_assert!(self.plain());
        if len < self.n {
            self.truncate_here(len);
        }
    }

    /// `wrap`, on a plain tape.
    #[inline(always)]
    pub fn plain_wrap(&mut self, at: usize, mask: u32, bits: u32, key: u32) {
        debug_assert!(self.plain());
        self.wrap_here(at, mask, bits, key);
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
        self.truncate_here(len);
    }

    #[inline(always)]
    fn truncate_here(&mut self, len: usize) {
        self.n = len;
        self.uncode_from(len);
        self.marks.retain(|m| (m[0] as usize) < len);
        self.wide_from = if self.marks.is_empty() { 0x10000 } else { 0 };
    }

    /// The tape in format 16 or 16z and its presence, where the writer writes it (`to`; decision 0434 §2): the nodes
    /// not yet passed on given to the ops, then their `finish`. `Redo` where the writer's widths did not keep every
    /// size and end exactly; `Ok(Err)` where the writer writes no tape, or the nodes are not a format 16 tape (a code
    /// the language's table lacks, an edit before a commit, a node past format 16's bounds).
    pub fn finish_16(&mut self) -> Result<Result<f16::Finished, String>, Redo> {
        self.finish_16_with(None)
    }

    /// `finish_16`, the tape's auxiliary index made by `aux` and laid where the tape lies (`f16::Writer::finish_with`).
    pub fn finish_16_with(&mut self, aux: Option<f16::AuxMaker<'_>>) -> Result<Result<f16::Finished, String>, Redo> {
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
        let want = Widths { size, tail, from_start };
        if want != w {
            return Err(Redo(want));
        }
        let mut s = match std::mem::replace(&mut self.sixteen, Err("the writer was finished".into())) {
            Ok(s) => s,
            Err(e) => return Ok(Err(e)),
        };
        if let Some(bad) = s.bad.take() {
            return Ok(Err(bad));
        }
        if let Some(node) = s.lost.get() {
            return Ok(Err(format!("node {node} read after its commit")));
        }
        s.close_to(self.n);
        Ok(s.w.finish_with(aux))
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::tape::{CodeTable, Entry, Tape};

    /// Each node: its code, its subtree's end, its parent, its start and its end.
    type Row = (u32, usize, Option<usize>, u64, u64);

    /// A language whose table holds kinds 0 to 15 in fields 0 to 7, each numbered its kind and 16 its field.
    fn lang() -> &'static f16::Lang {
        static LANG: std::sync::OnceLock<f16::Lang> = std::sync::OnceLock::new();
        LANG.get_or_init(|| {
            let entries = (0..8).flat_map(|field| (0..16).map(move |kind| Entry { kind, field, flags: 0, alias: None, classes: Vec::new() })).collect();
            f16::Lang::new(CodeTable::new(entries), [1, 1, 1], 1).unwrap()
        })
    }

    /// Format 16 (16z where `z`) in `lang`, each key numbered its kind and 16 its field: a key of no field, its kind.
    fn target(z: bool) -> Target {
        let number = |k: u32| {
            let (kind, field) = (k & 0x3ff, k >> KIND_BITS & 0x3f);
            (k >> 16 == 0 && kind < 16 && field < 8).then_some(kind + 16 * field)
        };
        Ok(Sixteen { lang: lang(), terminators: newlines::Terminators::Lf, tag: 1, z, blocks: true, number: std::sync::Arc::new(number), rooms: None })
    }

    /// The rows of the tape `w` finishes.
    fn rows(w: &mut Writer, z: bool) -> Result<Vec<Row>, Redo> {
        fn of<T: Tape>(t: &T) -> Vec<Row> {
            (0..t.len()).map(|i| (t.code(i), t.subtree_end(i), t.parent(i), t.start(i), t.end(i))).collect()
        }
        let f = w.finish_16()?.unwrap();
        Ok(match f.tape {
            f16::Written::F16(t) => of(&t.reader(lang()).unwrap()),
            f16::Written::F16Z(t) => {
                assert!(z);
                let cache = f16::ZCache::of([&t]);
                of(&t.reader(lang(), Some(&cache)).unwrap())
            }
        })
    }

    /// The rows of nodes in preorder, each its key, start, end and subtree size.
    fn tree(nodes: &[(u32, u64, u64, usize)]) -> Vec<Row> {
        let mut open: Vec<(usize, usize)> = Vec::new();
        let mut out = Vec::new();
        for (i, &(key, start, end, size)) in nodes.iter().enumerate() {
            while open.last().is_some_and(|&(_, e)| e <= i) {
                open.pop();
            }
            out.push((key, i + size, open.last().map(|&(p, _)| p), start, end));
            open.push((i, i + size));
        }
        out
    }

    /// Leaves put in at once (`insert_leaves`) give the tape an insert each gives, runs between
    /// them long and short, leaves at one place, at the end, a wide tail among them.
    #[test]
    fn leaves_inserted_at_once_are_an_insert_each() {
        let src = vec![b' '; 200_000];
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
                    // Before node `at` (at 10 at) and past the one before it (ended by 10 at - 9), in order: a
                    // format 16 tape's starts never go back in preorder.
                    let start = 10 * at - 8 + j as u32;
                    let end = if next() % 9 == 0 { start + 70_000 } else { start + 1 };
                    Leaf { key: key(4 + j as u16, 0, 0), start, end, at }
                })
                .collect();
            let build = |batch: bool| {
                let mut w = Widths::guess(src.len());
                loop {
                    let mut o = Writer::new(w, 8).to(target(false), &src);
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
                    match rows(&mut o, false) {
                        Ok(t) => break t,
                        // Wider columns where the guess was short (the wide tails).
                        Err(Redo(wider)) => w = wider,
                    }
                }
            };
            let got = build(true);
            assert_eq!(build(false), got, "{places:?} over {n}");
            assert_eq!(got.len(), n + k);
        }
    }

    /// A leaf written at once (`push_leaf`), a field set where there is none (`set_field`), a wrap (`wrap`) and its close
    /// (`close_from`) give the tape the calls they join give: wide tails among the leaves, a field set where one is and
    /// where none is, a wrap closed past its leaf and at its start.
    #[test]
    fn a_leaf_at_once_and_a_field_are_the_calls_they_join() {
        // A table of kinds 0 to 15 under fields 0 to 7, a key numbered `kind + 16 * field`.
        static LANG: std::sync::OnceLock<f16::Lang> = std::sync::OnceLock::new();
        let lang = LANG.get_or_init(|| {
            let entries = (0..8).flat_map(|field| (0..16).map(move |kind| Entry { kind, field, flags: 0, alias: None, classes: Vec::new() })).collect();
            f16::Lang::new(CodeTable::new(entries), [1, 1, 1], 1).unwrap()
        });
        let number = |k: u32| (k >> 16 == 0 && k & 0x3ff < 16 && k >> 10 < 8).then(|| (k & 0x3ff) + 16 * (k >> 10));
        let target = || Ok(Sixteen { lang, terminators: newlines::Terminators::Lf, tag: 1, z: false, blocks: true, number: std::sync::Arc::new(number), rooms: None });
        let src = vec![b' '; 200_000];
        // 0: the calls joined; 1: the joined calls; 2: the plain ones (no chain, no node passed on).
        let build = |way: u8| {
            let joined = way > 0;
            let mut w = Widths::guess(src.len());
            loop {
                let mut o = Writer::new(w, 8).to(target(), &src);
                let r = o.push(key(1, 0, 0), 0);
                for i in 0..40u32 {
                    let (s, e) = (10 * i, if i % 7 == 3 { 10 * i + 70_000 } else { 10 * i + 3 });
                    let k = key(2 + (i % 3) as u16, if i % 5 == 0 { 3 } else { 0 }, 0);
                    let x = if way == 2 {
                        o.plain_leaf(k, s, e)
                    } else if joined {
                        o.push_leaf(k, s, e)
                    } else {
                        let x = o.push(k, s);
                        o.close(x, e);
                        x
                    };
                    let field = u32::from(1 + (i % 4) as u16) << 10;
                    if way == 2 {
                        o.plain_field(x, 0x3f << 10, field);
                    } else if joined {
                        o.set_field(x, 0x3f << 10, field);
                    } else if o.key(x) >> 10 & 0x3f == 0 {
                        let k = o.key(x);
                        o.set_key(x, k | field);
                    }
                    // A wrap over the leaf, closed past it or at its start (a later end, an earlier one).
                    if i % 3 == 1 {
                        let wk = key(9, 0, 0);
                        let end = if i % 2 == 0 { e + 4 } else { s };
                        if way == 2 {
                            o.plain_wrap(x, 0x3f << 10, 5 << 10, wk);
                            let _ = o.plain_close_from(x, end);
                        } else if joined {
                            o.wrap(x, 0x3f << 10, 5 << 10, wk);
                            let _ = o.close_from(x, end);
                        } else {
                            let k = o.key(x);
                            o.set_key(x, k & !(0x3f << 10) | 5 << 10);
                            let st = o.start(x);
                            o.insert(x, wk | (k & (0x3f << 10)), st);
                            let e2 = end.max(o.start(x));
                            o.close(x, e2);
                        }
                    }
                }
                o.close(r, 199_999);
                match o.finish_16() {
                    Ok(t) => {
                        let t = t.unwrap();
                        let f16::Written::F16(t) = t.tape else { panic!("format 16") };
                        let r = t.reader(lang).unwrap();
                        break (0..r.len()).map(|i| (r.code(i), r.subtree_end(i), r.parent(i), r.start(i), r.end(i))).collect::<Vec<Row>>();
                    }
                    Err(Redo(wider)) => w = wider,
                }
            }
        };
        let joined = build(0);
        assert!(joined.len() > 40, "every leaf and wrap on the tape");
        assert_eq!(joined, build(1));
        assert_eq!(joined, build(2));
    }

    /// A leaf put in later, between closed nodes (`insert_leaf`), gives the tape written in order.
    #[test]
    fn a_leaf_inserted_later_is_the_tape_in_order() {
        let src = vec![b' '; 100];
        let w = Widths::guess(src.len());
        // In order: root(a(x), ",", b(y)).
        let mut o = Writer::new(w, 8).to(target(false), &src);
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
        let mut l = Writer::new(w, 8).to(target(false), &src);
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
        let want = tree(&[(1, 0, 9, 6), (2, 0, 3, 2), (3, 0, 3, 1), (4, 4, 5, 1), (2, 6, 9, 2), (3, 6, 9, 1)]);
        assert_eq!(rows(&mut o, false).unwrap(), want);
        assert_eq!(rows(&mut l, false).unwrap(), want);
    }

    /// A parser's writer gives the tree its edits make: a rewind, a node put before its first child, a key changed.
    /// Committed as it goes, the nodes are passed to the format 16 writer's ops before the edits that reach them: a
    /// close, a retag (decision 0434 §2).
    #[test]
    fn a_parsers_writer_finishes_as_its_edits_make_the_tree() {
        let src = vec![b' '; 100];
        let make = |target: Target, commit: bool| {
            let mut l = Writer::new(Widths::guess(src.len()), 8).to(target, &src);
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
        let want = tree(&[(1, 0, 10, 5), (5, 0, 9, 3), (3, 0, 3, 1), (3, 6, 9, 1), (4, 9, 10, 1)]);
        for (z, commit) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(rows(&mut make(target(z), commit), z).unwrap(), want, "16z {z}, committed {commit}");
        }
        // A writer with no target writes no tape, and says why.
        let e = make(Err("no table".into()), true).finish_16().unwrap().err().expect("no tape");
        assert_eq!(e, "no table");
    }

    /// A format 16 writer's columns hold the window, the nodes since the last commit: a file of many statements
    /// keeps them at their first cells, and its tape is the tree. The parse reads its open ancestors after a commit;
    /// a read of a node given back, or an edit before the window, says the tape is not format 16's (`finish_16`'s
    /// reason; decision 0434 §2).
    #[test]
    fn a_format_16_writers_columns_are_the_window() {
        let statements = 3 * WINDOW / 4;
        let src = vec![b' '; 4 * statements + 1];
        // A module of statements, each an expression statement over two names, committed as it ends.
        let make = |target: Target, misread: bool, misedit: bool| {
            let mut l = Writer::new(Widths::guess(src.len()), 8).to(target, &src);
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
        let mut nodes = vec![(1, 0, src.len() as u64, 1 + 3 * statements)];
        for s in 0..statements as u64 {
            nodes.extend([(2, 4 * s, 4 * s + 3, 3), (3, 4 * s, 4 * s + 1, 1), (3, 4 * s + 2, 4 * s + 3, 1)]);
        }
        let want = tree(&nodes);
        // A large file's guess is cut to the window.
        assert_eq!(Writer::new(Widths::guess(1 << 20), 1 << 16).to(target(false), &src).cap, WINDOW);
        for z in [false, true] {
            let mut w = make(target(z), false, false);
            // Made for 8 nodes: the cells it began with hold every statement's.
            assert_eq!(w.cap, MIN_CELLS, "the columns stay at the window");
            assert_eq!(rows(&mut w, z).unwrap(), want, "16z {z}");
            for (misread, misedit, why) in [(true, false, "read after its commit"), (false, true, "before the last commit")] {
                let mut w = make(target(z), misread, misedit);
                let e = w.finish_16().unwrap().err().expect("not format 16's");
                assert!(e.contains(why), "{e}");
            }
        }
    }
}
