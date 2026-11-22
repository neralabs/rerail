//! Format 16 written by the writer (decision 0434 §6), its Rust written directly (`writer16`, decision 0532): the
//! host's region, each part its own buffer, named in the state by its offset from the state's address. Before each
//! op the host reserves what the op can need, as a `Vec` reserves before a push, so an op never finds a part full:
//! a full part is the device's failure, whose document is parsed again on the host.

use super::{Finished, K, Lang, Tape16, Tape16Z, Written, Z, before, bytes_for, mask, stride_of};
use crate::writer16 as ops;
use rerail_clif::writer::{self as w, LAID_AT, OPEN, Part, SIDE_AREA, State, TABLE, part};

/// An op called in its Rust (`writer16`); in a test, by the JIT's where the writer has it (`Jit`).
macro_rules! call {
    ($w:expr, $op:ident($($a:expr),*)) => {{
        #[cfg(test)]
        // SAFETY: the op's own: the JIT's function is the same op, of the same signature.
        let r = match $w.jit { Some(j) => unsafe { (j.$op)($($a),*) }, None => unsafe { ops::$op($($a),*) } };
        // SAFETY: the op's own (its caller's).
        #[cfg(not(test))]
        let r = unsafe { ops::$op($($a),*) };
        r
    }};
}

/// The ops compiled by the JIT (decision 0434 §6, R15), a test's: what `Writer` calls in place of their Rust.
#[cfg(test)]
pub(super) struct Jit {
    _module: rerail_clif::jit::Writer,
    w_open: unsafe extern "C" fn(*const u8, u32, u64) -> u32,
    w_close: unsafe extern "C" fn(*const u8, u64) -> u32,
    w_leaf: unsafe extern "C" fn(*const u8, u32, u64, u64) -> u32,
    w_commit: unsafe extern "C" fn(*const u8) -> u32,
    w_retag: unsafe extern "C" fn(*const u8, u32, u32) -> u32,
    w_set_start: unsafe extern "C" fn(*const u8, u32, u64) -> u32,
    w_finish_blocks: unsafe extern "C" fn(*const u8) -> u32,
    w_finish_tail: unsafe extern "C" fn(*const u8) -> u32,
}

#[cfg(test)]
impl Jit {
    /// The JIT's ops, for the test's life.
    pub(super) fn leaked() -> &'static Jit {
        let m = rerail_clif::jit::Writer::new();
        // SAFETY: each function is the op of that name, compiled to the signature it is typed with here
        // (`writer::signature`: the state's address, then the op's parameters), in the C convention.
        unsafe {
            Box::leak(Box::new(Jit {
                w_open: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8, u32, u64) -> u32>(m.function("w_open")),
                w_close: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8, u64) -> u32>(m.function("w_close")),
                w_leaf: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8, u32, u64, u64) -> u32>(m.function("w_leaf")),
                w_commit: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8) -> u32>(m.function("w_commit")),
                w_retag: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8, u32, u32) -> u32>(m.function("w_retag")),
                w_set_start: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8, u32, u64) -> u32>(m.function("w_set_start")),
                w_finish_blocks: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8) -> u32>(m.function("w_finish_blocks")),
                w_finish_tail: std::mem::transmute::<*const u8, unsafe extern "C" fn(*const u8) -> u32>(m.function("w_finish_tail")),
                _module: m,
            }))
        }
    }
}

/// The most a block's four side-list parts take, laid (`w_lay`): each a u16 a sub-block, a byte and 8 bytes an
/// entry, and 8 bytes of room.
const LAID_MAX: usize = 4 * (2 * (Z / 256) + 9 * Z) + 8;
/// The most a 16z piece holds raw (a column of 8-byte cells and its part), and the most a final block adds to
/// the payload while its pieces are stored (each raw at the end, the last one's LZ4 room past it).
const PIECE_MAX: usize = 8 * Z + (2 * (Z / 256) + 9 * Z);
const ZBLOCK_MAX: usize = 4 * PIECE_MAX + w::lz4_bound(PIECE_MAX as u64) as usize + 64;

/// What a fresh byte of the tape's buffer holds: in tests not zero, so a byte the ops do not write shows.
const FRESH: u8 = if cfg!(test) { 0xa5 } else { 0 };

/// What a writer writes a tape of (`Writer::begin_with`): a document of `source` in `lang`, of about `nodes` nodes,
/// the producer's `tag`, in 16z where `z`, with each block's presence where `blocks`.
pub struct Document<'a> {
    pub lang: &'a Lang,
    pub source: &'a [u8],
    pub tag: u8,
    pub z: bool,
    pub blocks: bool,
    pub nodes: usize,
}

/// Format 16 or 16z written in one pass as a parser goes (decision 0434 §2), by the writer's ops in CLIF (§6)
/// through their Rust: `f16::Writer`. Its API and its bytes are the hand-written reference writer's
/// (`reference::Writer`, whose doc says what each op writes where), which the tests check it against (R13, R16).
pub struct Writer {
    state: Box<State>,
    /// The parts' buffers, by `part::*`, each its capacity long.
    parts: [Vec<u8>; part::COUNT],
    presence_words: (usize, usize),
    keep_blocks: bool,
    z: bool,
    /// The tape `finish` gave out: its buffer's address and the bytes of it written (its length before it was cut to
    /// the tape's), which `recycle` takes back as they are.
    given: (usize, usize),
    /// A part held at its capacity (a test's: the device's region, which the host does not grow).
    #[cfg(test)]
    fixed: Option<(u32, u64)>,
    /// The JIT's ops, called in place of their Rust (a test's).
    #[cfg(test)]
    pub(super) jit: Option<&'static Jit>,
}

impl Writer {
    /// A writer of a document of `source` in `lang`, of about `nodes` nodes, in 16z where `z`, with each block's
    /// presence where `blocks`. Its parts' room is reserved for `nodes` nodes, their side lists' usual share and the
    /// source's newline index: more nodes grow them.
    pub fn new(lang: &Lang, source: &[u8], terminators: crate::newlines::Terminators, tag: u8, z: bool, blocks: bool, nodes: usize) -> Self {
        let mut w = Writer::empty();
        w.begin(lang, source, terminators, tag, z, blocks, nodes);
        w
    }

    /// A writer of no document yet, its parts empty: `begin` starts each.
    pub fn empty() -> Self {
        Writer { state: Box::default(), parts: Default::default(), presence_words: (0, 0), keep_blocks: false, z: false, given: (0, 0), #[cfg(test)] fixed: None, #[cfg(test)] jit: None }
    }

    /// The next document, in this writer: as `new`, its parts' buffers kept from the documents before (each grown only
    /// where it is short, so a writer used again zeroes no room), as simdjson's parser keeps its tape from one document to
    /// the next. A buffer's bytes are the last document's: the ops write each byte before they read it (in tests every
    /// part kept is filled with `FRESH`, so a byte read before it is written shows).
    /// The line index is of the source's lines as its language's `terminators` end them (decision 0612).
    pub fn begin(&mut self, lang: &Lang, source: &[u8], terminators: crate::newlines::Terminators, tag: u8, z: bool, blocks: bool, nodes: usize) {
        self.begin_with(&Document { lang, source, tag, z, blocks, nodes }, |out| crate::newlines::index_into(out, source, terminators));
    }

    /// `begin`, the newline index written into its part by `lines` (`newlines::index_of`), in the part's own buffer: a
    /// producer that has the source's newlines (tier 1's bits) makes it with no scan of the source.
    pub fn begin_with(&mut self, doc: &Document<'_>, lines: impl FnOnce(&mut Vec<u8>)) {
        let &Document { lang, source, tag, z, blocks, nodes } = doc;
        let source_len = source.len() as u64;
        let wd = lang.widths;
        let cw = bytes_for(source_len);
        let mut index = std::mem::take(&mut self.parts[part::LINES as usize]);
        lines(&mut index);
        let lines = index;
        let newlines = crate::newlines::count(&lines).unwrap_or(0);
        let (kind_words, field_words) = lang.presence_words();
        let w3 = [wd[0] as u32, wd[1] as u32, wd[2] as u32];
        let mut state = Box::new(State {
            esc: [mask(wd[0]), mask(wd[1]), mask(wd[2])],
            source_len,
            prev_start: 0,
            w: w3,
            cw: cw as u32,
            vw: [lang.low_w as u32, 4, bytes_for(source_len) as u32, bytes_for(source_len) as u32],
            cut: lang.cut,
            per: lang.per,
            stride: stride_of(wd, cw) as u32,
            at: std::array::from_fn(|r| (Z * before(wd, r)) as u32),
            flags: u32::from(blocks) | u32::from(z) << 1,
            kw: kind_words as u32,
            fw: field_words as u32,
            tag: u32::from(tag),
            mark: u32::from(lang.revision),
            newlines,
            ..State::default()
        });
        debug_assert_eq!(state.stride, w::stride_of(w3, cw as u32));
        let kinds: Vec<u8> = lang.kind_field.iter().flat_map(|&(k, f)| (u32::from(k) | u32::from(f) << 16).to_le_bytes()).collect();
        let words = 8 * (kind_words + field_words);
        let count = nodes.div_ceil(Z).max(1);
        let buckets = (4 * count).next_power_of_two().max(64);
        state.zbuckets = buckets as u32;
        let stride = state.stride as usize;
        let mut lens = [0u64; part::COUNT];
        (lens[part::KINDS as usize], lens[part::PRESENCE as usize], lens[part::LINES as usize]) = (kinds.len() as u64, words as u64, lines.len() as u64);
        if z {
            // 16z's payload follows its header's room; its seen pieces' buckets start empty.
            lens[part::OUT as usize] = w::HEAD16 as u64;
            lens[part::SEEN as usize] = 4 * buckets as u64;
            lens[part::LZ as usize] = u64::from(w::LZ_TABLES);
        }
        let caps: [usize; part::COUNT] = [
            kinds.len() + 8,
            64 * OPEN as usize,
            // 16: the tape is this part (R4): its blocks, then the tail's room at `finish` (the side lists' usual share,
            // their part starts, the newline index) and the presence's after it, so a tape within the hint is not
            // moved.
            if z { w::HEAD16 as usize + ZBLOCK_MAX + 64 } else { w::HEAD16 as usize + count * stride + 8 + nodes / 2 + 16 * (count + 1) + lines.len() + 9 * 4 * count + 64 + 8 + 8 + super::presence::HEAD + words * (1 + if blocks { count } else { 0 }) },
            count * TABLE as usize,
            2 * SIDE_AREA as usize,
            64,
            LAID_MAX,
            count * LAID_AT as usize,
            words * (1 + if blocks { count + 1 } else { 0 }) + 8,
            lines.len() + 8,
            if z { 2 * stride + 8 } else { 8 },
            64,
            if z { 4 * count * w::PIECE as usize + 8 } else { 8 },
            if z { 4 * buckets + 8 } else { 8 },
            if z { count * (Z / K) * cw + 8 } else { 8 },
            if z { w::LZ_TABLES as usize + 8 } else { 8 },
        ];
        // The kinds' table and the newline index are their parts, made where they are computed.
        self.parts[part::KINDS as usize] = kinds;
        self.parts[part::LINES as usize] = lines;
        for (k, (buf, &cap)) in self.parts.iter_mut().zip(&caps).enumerate() {
            // Kept from the document before; in tests made `FRESH`, as a byte the ops read before they write it shows.
            #[cfg(test)]
            if k as u32 != part::KINDS && k as u32 != part::LINES {
                buf.fill(FRESH);
            }
            let _ = k;
            if buf.len() < cap {
                buf.resize(cap, if cfg!(test) { FRESH } else { 0 });
            }
        }
        // What the ops read before they write: the presence's words (each node's bits are or-ed in), and in 16z the
        // seen pieces' buckets (empty) and the LZ4 tables.
        self.parts[part::PRESENCE as usize][..words * (1 + if blocks { count + 1 } else { 0 })].fill(0);
        if z {
            self.parts[part::SEEN as usize][..4 * buckets].fill(0xff);
            self.parts[part::LZ as usize][..w::LZ_TABLES as usize].fill(0);
        }
        (self.presence_words, self.keep_blocks, self.z) = ((kind_words, field_words), blocks, z);
        #[cfg(test)]
        {
            self.fixed = None;
        }
        for (k, &len) in lens.iter().enumerate() {
            *self.part_mut(&mut state, k) = Part { at: 0, cap: 0, len };
        }
        self.state = state;
        self.bind();
    }

    /// A tape this writer finished, given back: its bytes are the next document's room (`begin` keeps them), so a writer
    /// used again allocates no tape.
    pub fn recycle(&mut self, tape: super::Written) {
        let Some(mut buf) = tape.into_own() else { return };
        let (at, written) = std::mem::take(&mut self.given);
        if buf.as_ptr() as usize != at || written <= self.parts[part::OUT as usize].len() {
            return;
        }
        // SAFETY: the same allocation `finish` gave out (laying the presence did not move it), whose first `written`
        // bytes were written (each initialized before `finish` cut the length to the tape's), within its capacity.
        unsafe { buf.set_len(written.min(buf.capacity())) };
        self.parts[part::OUT as usize] = buf;
        self.bind();
    }

    fn part_mut<'s>(&self, state: &'s mut State, k: usize) -> &'s mut Part {
        match k as u32 {
            part::KINDS => &mut state.kinds,
            part::OPEN => &mut state.open,
            part::OUT => &mut state.out,
            part::TABLE => &mut state.table,
            part::SIDES => &mut state.sides,
            part::FREE => &mut state.free,
            part::LAID => &mut state.laid,
            part::LAID_AT => &mut state.laid_at,
            part::PRESENCE => &mut state.presence,
            part::LINES => &mut state.lines,
            part::ROOMS => &mut state.rooms,
            part::ROOM_FREE => &mut state.room_free,
            part::PIECES => &mut state.pieces,
            part::SEEN => &mut state.seen,
            part::CKPT => &mut state.ckpt,
            _ => &mut state.lz,
        }
    }

    /// Each part's place and capacity in the state, from its buffer (8 bytes of each kept as room past its last).
    fn bind(&mut self) {
        let base = std::ptr::from_ref::<State>(&self.state) as i64;
        let mut state = *self.state;
        for k in 0..part::COUNT {
            let (at, cap) = (self.parts[k].as_ptr() as i64, self.parts[k].len().saturating_sub(8) as u64);
            #[cfg(test)]
            let cap = match self.fixed {
                Some((f, fixed)) if f as usize == k => fixed,
                _ => cap,
            };
            let p = self.part_mut(&mut state, k);
            p.at = at.wrapping_sub(base);
            p.cap = cap;
        }
        *self.state = state;
    }

    fn len_of(&self, k: u32) -> usize {
        let s = &self.state;
        let p = match k {
            part::KINDS => &s.kinds,
            part::OPEN => &s.open,
            part::OUT => &s.out,
            part::TABLE => &s.table,
            part::SIDES => &s.sides,
            part::FREE => &s.free,
            part::LAID => &s.laid,
            part::LAID_AT => &s.laid_at,
            part::PRESENCE => &s.presence,
            part::LINES => &s.lines,
            part::ROOMS => &s.rooms,
            part::ROOM_FREE => &s.room_free,
            part::PIECES => &s.pieces,
            part::SEEN => &s.seen,
            part::CKPT => &s.ckpt,
            _ => &s.lz,
        };
        p.len as usize
    }

    /// Part `k` with room for `need` bytes: its buffer grown (the writer's own memory, moved as a `Vec` grows).
    fn reserve(&mut self, k: u32, need: usize) {
        #[cfg(test)]
        if self.fixed.is_some_and(|f| f.0 == k) {
            return;
        }
        if need + 8 <= self.parts[k as usize].len() {
            return;
        }
        let cap = (need + 8).max(2 * self.parts[k as usize].len());
        self.parts[k as usize].resize(cap, if k == part::OUT { FRESH } else { 0 });
        self.bind();
    }

    /// Room for `blocks` blocks made final: their laid parts, their side areas given back, their entries; in 16z
    /// their pieces and their rooms given back.
    fn reserve_final(&mut self, blocks: usize) {
        if blocks == 0 {
            return;
        }
        let (laid, free) = (self.len_of(part::LAID), self.len_of(part::FREE));
        self.reserve(part::LAID, laid + blocks * LAID_MAX);
        self.reserve(part::FREE, free + 4 * blocks);
        let count = self.state.blocks as usize;
        self.reserve(part::LAID_AT, count * LAID_AT as usize);
        if self.z {
            let (out, rooms) = (self.len_of(part::OUT), self.len_of(part::ROOM_FREE));
            self.reserve(part::OUT, out + blocks * ZBLOCK_MAX);
            self.reserve(part::ROOM_FREE, rooms + 4 * blocks);
        }
    }

    /// Part `k` held at `cap` bytes, not grown, its buffer's bytes past `cap + 8` (the op's room past its last cell)
    /// set to `canary`.
    #[cfg(test)]
    pub(super) fn fix(&mut self, k: u32, cap: usize, canary: u8) {
        let len = self.len_of(k);
        assert!(len <= cap, "the part's bytes fit");
        let buf = &mut self.parts[k as usize];
        buf.resize(buf.len().max(cap + 8 + 64), 0);
        buf[cap + 8..].fill(canary);
        self.fixed = Some((k, cap as u64));
        self.bind();
    }

    /// Part `k`'s buffer (a test's).
    #[cfg(test)]
    pub(super) fn buffer(&self, k: u32) -> &[u8] {
        &self.parts[k as usize]
    }

    /// Room for a run of up to `nodes` nodes, `depth` open at once, whose ops code calls itself, with no host
    /// between them to reserve (tier 2's words, decision 0434 §6, R17): each block's share as `open` reserves it,
    /// and room for every block made final as `reserve_final` does. A part found full after it is the run's
    /// failure (`fail::FULL`), and the run goes again with more room.
    pub fn reserve_run(&mut self, nodes: usize, depth: usize) {
        let count = nodes.div_ceil(Z).max(1);
        let stride = self.state.stride as usize;
        self.reserve(part::OPEN, (depth + 1) * OPEN as usize);
        if self.z {
            let rooms = self.len_of(part::ROOMS);
            self.reserve(part::ROOMS, rooms + count * stride);
            self.reserve(part::PIECES, 4 * count * w::PIECE as usize);
            self.reserve(part::CKPT, count * (Z / K) * self.state.cw as usize);
        } else {
            self.reserve(part::OUT, w::HEAD16 as usize + count * stride);
        }
        self.reserve(part::TABLE, count * TABLE as usize);
        let sides = self.len_of(part::SIDES);
        self.reserve(part::SIDES, sides + count * SIDE_AREA as usize);
        if self.keep_blocks {
            let words = 8 * (self.presence_words.0 + self.presence_words.1);
            self.reserve(part::PRESENCE, (count + 1) * words);
        }
        self.reserve_final(count);
    }

    /// The state's address: what code that calls the ops itself passes them (`reserve_run` first).
    pub fn state_address(&mut self) -> *mut u8 {
        std::ptr::from_mut::<State>(&mut self.state).cast::<u8>()
    }

    /// The nodes so far.
    pub fn len(&self) -> usize {
        self.state.n as usize
    }
    pub fn is_empty(&self) -> bool {
        self.state.n == 0
    }

    fn desc(&mut self) -> *const u8 {
        std::ptr::from_mut::<State>(&mut self.state).cast::<u8>()
    }

    /// The next node opened, of `number`, at `start` (`w_open`): its index.
    pub fn open(&mut self, number: u32, start: u64) -> usize {
        let i = self.room_to_open();
        let desc = self.desc();
        // SAFETY: the state's parts lie in their buffers, each with the room the op can need, reserved above.
        call!(self, w_open(desc, number, start));
        i
    }

    /// The room the next node's `w_open` (or `w_leaf`) can need: the open nodes' one more, and at a block's first
    /// node the block's. The node's index.
    fn room_to_open(&mut self) -> usize {
        let (i, open) = (self.state.n as usize, self.state.open_len as usize);
        self.reserve(part::OPEN, (open + 1) * OPEN as usize);
        if i.is_multiple_of(Z) {
            let b = i / Z;
            let stride = self.state.stride as usize;
            if self.z {
                let rooms = self.len_of(part::ROOMS);
                self.reserve(part::ROOMS, rooms + stride);
                self.reserve(part::PIECES, 4 * (b + 1) * w::PIECE as usize);
                self.reserve(part::CKPT, (b + 1) * (Z / K) * self.state.cw as usize);
            } else {
                self.reserve(part::OUT, w::HEAD16 as usize + (b + 1) * stride);
            }
            self.reserve(part::TABLE, (b + 1) * TABLE as usize);
            let sides = self.len_of(part::SIDES);
            self.reserve(part::SIDES, sides + SIDE_AREA as usize);
            if self.keep_blocks {
                let words = 8 * (self.presence_words.0 + self.presence_words.1);
                self.reserve(part::PRESENCE, (b + 2) * words);
            }
            self.reserve_final(1);
        }
        i
    }

    /// The innermost open node closed at `end` (`w_close`).
    pub fn close(&mut self, end: u64) {
        // A close makes the closing node's block final only once a later block has begun.
        let open = self.state.open_len as usize;
        if open > 0 {
            let at = (open - 1) * OPEN as usize;
            let node = u32::from_le_bytes(self.parts[part::OPEN as usize][at..at + 4].try_into().expect("4 bytes")) as usize;
            if node / Z < self.state.n as usize / Z {
                self.reserve_final(1);
            }
        }
        let desc = self.desc();
        // SAFETY: as `open`'s.
        call!(self, w_close(desc, end));
    }

    /// A closed node with no children.
    pub fn leaf(&mut self, number: u32, start: u64, end: u64) -> usize {
        // A leaf's close makes no block final (`w_leaf`): the room is its open's.
        let i = self.room_to_open();
        let desc = self.desc();
        // SAFETY: as `open`'s.
        call!(self, w_leaf(desc, number, start, end));
        i
    }

    /// The nodes so far final but for the open ones (`w_commit`): no edit reaches before them (decision 0434 §2).
    pub fn commit(&mut self) {
        let blocks = (self.state.n as usize / Z).saturating_sub(self.state.frontier as usize / Z);
        self.reserve_final(blocks);
        let desc = self.desc();
        // SAFETY: as `open`'s.
        call!(self, w_commit(desc));
    }

    /// Node `i`'s number made `number` (`w_retag`): an edit of a node not yet final.
    pub fn retag(&mut self, i: usize, number: u32) {
        let desc = self.desc();
        // SAFETY: as `open`'s; a code's entry takes no room past its list's.
        call!(self, w_retag(desc, i as u32, number));
    }

    /// Node `i`'s start made `start` (`w_set_start`): an edit of a node not yet final.
    pub fn set_start(&mut self, i: usize, start: u64) {
        let desc = self.desc();
        // SAFETY: as `open`'s.
        call!(self, w_set_start(desc, i as u32, start));
    }

    /// The failure the ops kept, as a code and its node.
    pub fn failure(&self) -> Option<(u32, u32)> {
        (self.state.bad != 0).then_some((self.state.bad, self.state.bad_node))
    }

    /// The tape and its presence (`w_finish_blocks`, then `w_finish_tail`); `Err` where an op failed, with the
    /// first failure's code (`fail`) and node.
    pub fn finish(&mut self) -> Result<Finished, String> {
        self.finish_ops()?;
        let count = (self.state.n as usize).div_ceil(Z);
        let end = self.len_of(part::OUT);
        let mut out = std::mem::take(&mut self.parts[part::OUT as usize]);
        self.given = (out.as_ptr() as usize, out.len());
        out.truncate(end);
        finished(out, &self.parts[part::PRESENCE as usize], self.presence_words, self.keep_blocks.then_some(count), self.z)
    }

    /// `finish`'s ops, each with the room it can need reserved first.
    fn finish_ops(&mut self) -> Result<(), String> {
        let count = (self.state.n as usize).div_ceil(Z);
        self.reserve_final(count);
        let desc = self.desc();
        // SAFETY: as `open`'s.
        let mut r = call!(self, w_finish_blocks(desc));
        if r == 0 {
            // The tail's room, the blocks' laid parts now known. In 16, `w_finish_tail`'s own: the last block's
            // columns moved down to its rows, then the laid parts, their starts, the newline index (so a tape within
            // the writer's hint stays where it is: R4). In 16z, its checkpoints and piece index too.
            let (laid, lines) = (self.len_of(part::LAID), self.len_of(part::LINES));
            let need = if self.z {
                self.len_of(part::OUT) + laid + 16 * (count + 1) + lines + self.len_of(part::CKPT) + 9 * 4 * count + 64
            } else {
                let s = &self.state;
                let blocks_end = if count == 0 {
                    w::HEAD16 as usize
                } else {
                    let rows = s.n as usize - (count - 1) * Z;
                    w::HEAD16 as usize + (count - 1) * s.stride as usize + rows * (s.at[4] as usize / Z) + rows.div_ceil(K) * s.cw as usize
                };
                blocks_end + laid + 16 * (count + 1) + lines + 8 * 6 + 8
            };
            // And the presence's, laid after the tape (`finished`), so the tape stays where it is (R4).
            let (kw, fw) = self.presence_words;
            let need = need + 8 + super::presence::HEAD + 8 * (kw + fw) * (1 + if self.keep_blocks { count } else { 0 });
            self.reserve(part::OUT, need);
            let desc = self.desc();
            // SAFETY: as `open`'s.
            r = call!(self, w_finish_tail(desc));
        }
        if r != 0 {
            return Err(format!("the writer's ops failed: {r:#x} at node {}", self.state.bad_node as i32));
        }
        Ok(())
    }
}

/// A finished writer's tape (`out`, its OUT part at its length), with its presence laid after it in its home
/// (`presence::lay_words`), from its PRESENCE part: the document's, and a presence a block where `blocks` counts them.
fn finished(mut out: Vec<u8>, presence: &[u8], words: (usize, usize), blocks: Option<usize>, z: bool) -> Result<Finished, String> {
    let word = |k: usize| u64::from_le_bytes(presence[8 * k..8 * k + 8].try_into().expect("8 bytes"));
    let count = 1 + blocks.unwrap_or(0);
    super::presence::lay_words(&mut out, words, count, &mut (0..(words.0 + words.1) * count).map(word));
    let tape = if z { Written::F16Z(Tape16Z::from_bytes(out)?) } else { Written::F16(Tape16::from_bytes(out)?) };
    Ok(Finished { tape })
}


/// The CLIF LZ4 compressor alone (a test's): its state, its tables, a piece's room.
#[cfg(test)]
pub(super) struct Lz {
    state: Box<State>,
    tables: Vec<u8>,
}

#[cfg(test)]
impl Lz {
    pub(super) fn new() -> Self {
        let mut lz = Lz { state: Box::default(), tables: vec![0; w::LZ_TABLES as usize + 8] };
        let base = std::ptr::from_ref::<State>(&lz.state) as i64;
        lz.state.lz = Part { at: (lz.tables.as_ptr() as i64).wrapping_sub(base), cap: w::LZ_TABLES as u64, len: w::LZ_TABLES as u64 };
        lz
    }

    /// `src` as one LZ4 block, by the ops.
    pub(super) fn compress(&mut self, src: &[u8]) -> Vec<u8> {
        let n = src.len();
        // The piece with 3 readable bytes before it and 8 after, then the output's room.
        let mut room = vec![0xeeu8; 8 + n + 8 + w::lz4_bound(n as u64) as usize + 8];
        room[8..8 + n].copy_from_slice(src);
        let (s, d) = (room.as_mut_ptr().wrapping_add(8), room.as_mut_ptr().wrapping_add(8 + n + 8));
        let desc = std::ptr::from_mut::<State>(&mut self.state).cast::<u8>();
        // SAFETY: the piece and the output lie in `room`, with the bytes the op reads and writes past each.
        let len = unsafe { ops::w_lz4(desc, s as u64, n as u64, d as u64) } as usize;
        room[8 + n + 8..8 + n + 8 + len].to_vec()
    }
}

/// The ops in UKL on the device (decisions 0434 §6 R15, 0440), a test's: the calls are recorded, then replayed by one
/// thread of a kernel over the region in one buffer, the state at its start and each part at its offset from it, as
/// the host names them. The device's region does not grow: each part gets the room the host's run of the same calls
/// reserved (sizing it from bounds is tier 2's, R17). On the process's device, whatever its backend.
#[cfg(test)]
pub(super) mod device {
    use super::{FRESH, Writer, State, Z, finished, part};
    use crate::f16::{Finished, Lang};

    /// The kernel: the calls `c` (an op and up to three arguments each) through the ops (`<ukl/writer.h>`), then
    /// `finish`.
    const REPLAY: &str = r#"
#include <ukl/writer.h>

typedef struct ReplayArgs {
    uint32_t n;
    uint32_t pad;
} ReplayArgs;
UKL_REFLECT_STRUCT(ReplayArgs, n, pad)

DEFINE_KERNEL2(writer, replay,
               ARGS,     ReplayArgs, args, ,
               RWBUFFER, uint8_t,    r,    "[bytes]",
               ROBUFFER, uint64_t,   c,    "[4 * n]",
               RWBUFFER, uint32_t,   out,  "[1]",
               GID0,     uint32_t,   id,   )
{
    if (id != 0u)
        return;
    UKL_GLOBAL const uint8_t* t = r;
    for (uint32_t k = 0u; k < args.n; k++) {
        ROBUFFER(uint64_t) e = c + 4u * k;
        switch ((uint32_t)e[0]) {
            case 0u: clif_w_open(t, 0ul, (uint32_t)e[1], e[2]); break;
            case 1u: clif_w_close(t, 0ul, e[1]); break;
            case 2u: clif_w_leaf(t, 0ul, (uint32_t)e[1], e[2], e[3]); break;
            case 3u: clif_w_commit(t, 0ul); break;
            case 4u: clif_w_retag(t, 0ul, (uint32_t)e[1], (uint32_t)e[2]); break;
            default: clif_w_set_start(t, 0ul, (uint32_t)e[1], e[2]); break;
        }
    }
    out[0] = clif_w_finish(t, 0ul);
}
"#;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ReplayArgs {
        n: u32,
        pad: u32,
    }
    // SAFETY: two u32s, no padding.
    unsafe impl rerail_gpu::Pod for ReplayArgs {}

    pub(in crate::f16) struct DeviceOps {
        /// The host's run of the calls, for the room each part needs; and the writer as made, for the region.
        host: Writer,
        made: Writer,
        calls: Vec<[u64; 4]>,
    }

    impl DeviceOps {
        pub(in crate::f16) fn new(lang: &Lang, source: &[u8], terminators: crate::newlines::Terminators, tag: u8, z: bool, blocks: bool, nodes: usize) -> Self {
            DeviceOps { host: Writer::new(lang, source, terminators, tag, z, blocks, nodes), made: Writer::new(lang, source, terminators, tag, z, blocks, nodes), calls: Vec::new() }
        }
        pub(in crate::f16) fn open(&mut self, number: u32, start: u64) -> usize {
            self.calls.push([0, number.into(), start, 0]);
            self.host.open(number, start)
        }
        pub(in crate::f16) fn close(&mut self, end: u64) {
            self.calls.push([1, end, 0, 0]);
            self.host.close(end);
        }
        pub(in crate::f16) fn leaf(&mut self, number: u32, start: u64, end: u64) -> usize {
            self.calls.push([2, number.into(), start, end]);
            self.host.leaf(number, start, end)
        }
        pub(in crate::f16) fn commit(&mut self) {
            self.calls.push([3, 0, 0, 0]);
            self.host.commit();
        }
        pub(in crate::f16) fn retag(&mut self, i: usize, number: u32) {
            self.calls.push([4, i as u64, number.into(), 0]);
            self.host.retag(i, number);
        }
        pub(in crate::f16) fn set_start(&mut self, i: usize, start: u64) {
            self.calls.push([5, i as u64, start, 0]);
            self.host.set_start(i, start);
        }

        pub(in crate::f16) fn finish(mut self) -> Result<Finished, String> {
            self.host.finish_ops()?;
            // The region: the state, then each part at a 64-byte boundary, as long as the host's buffer for it.
            let mut offsets = [0usize; part::COUNT];
            let mut end = size_of::<State>().next_multiple_of(64);
            for (k, at) in offsets.iter_mut().enumerate() {
                *at = end;
                end = (end + self.host.parts[k].len()).next_multiple_of(64);
            }
            let mut bytes = vec![0u8; end];
            let mut state = *self.made.state;
            for (k, &at) in offsets.iter().enumerate() {
                let len = self.host.parts[k].len();
                // A part's fresh bytes as the host's: the tape's marked, the seen buckets empty; the kinds' table
                // and the newline index as made.
                bytes[at..at + len].fill(match k as u32 {
                    part::OUT => FRESH,
                    part::SEEN => 0xff,
                    _ => 0,
                });
                if matches!(k as u32, part::KINDS | part::LINES) {
                    let made = &self.made.parts[k];
                    bytes[at..at + made.len()].copy_from_slice(made);
                }
                let p = self.made.part_mut(&mut state, k);
                p.at = at as i64;
                p.cap = len.saturating_sub(8) as u64;
            }
            // SAFETY: the state is plain integers (`repr(C)`), and the region starts with its room.
            unsafe { std::ptr::write_unaligned(bytes.as_mut_ptr().cast::<State>(), state) };
            use rerail_gpu::kernels::{buffer_of, launch, read};
            let shared = rerail_gpu::shared::get().ok_or("no device")?;
            let program = rerail_gpu::kernels::program("tape_writer_replay", REPLAY).unwrap_or_else(|e| panic!("{}: the writer's ops in UKL: {e}", rerail_gpu::device::backend()));
            let n = self.calls.len() as u32;
            // At least one call's room: a buffer is never empty.
            self.calls.push([0; 4]);
            let calls: Vec<u64> = self.calls.iter().flatten().copied().collect();
            let (region, calls, result) = (buffer_of(&bytes)?, buffer_of(&calls)?, buffer_of(&[0u32])?);
            let mut q = rerail_gpu::device::ComputeQueue::new(&shared.queue);
            launch(&q, program.function("replay")?)
                .value("args", &ReplayArgs { n, pad: 0 })
                .buffer("r", &region)
                .buffer("c", &calls)
                .buffer("out", &result)
                .run(&[1], &[1])?;
            q.flush().await_()?;
            let r = read::<u32>(&result, 1)[0];
            let bytes: Vec<u8> = read(&region, end);
            // SAFETY: the region starts with the state, plain integers.
            let state: State = unsafe { std::ptr::read_unaligned(bytes.as_ptr().cast::<State>()) };
            if r != 0 {
                return Err(format!("the writer's ops failed on the device: {r:#x} at node {}", state.bad_node as i32));
            }
            let (at, len) = (state.out.at as usize, state.out.len as usize);
            let count = (state.n as usize).div_ceil(Z);
            let presence = &bytes[state.presence.at as usize..][..self.host.parts[part::PRESENCE as usize].len()];
            finished(bytes[at..at + len].to_vec(), presence, self.made.presence_words, self.made.keep_blocks.then_some(count), self.made.z)
        }
    }
}

/// The writer's Rust reads the state the JIT's and the device's ops read: the same layout, the same constants.
#[cfg(test)]
mod layout {
    use crate::writer16 as d;
    use rerail_clif::writer as c;
    use std::mem::{offset_of, size_of};

    #[test]
    fn the_writers_state_is_the_ops() {
        assert_eq!(size_of::<d::State>(), size_of::<c::State>());
        assert_eq!(size_of::<d::Part>(), size_of::<c::Part>());
        let pairs = [
            (offset_of!(d::State, esc), offset_of!(c::State, esc)),
            (offset_of!(d::State, source_len), offset_of!(c::State, source_len)),
            (offset_of!(d::State, prev_start), offset_of!(c::State, prev_start)),
            (offset_of!(d::State, w), offset_of!(c::State, w)),
            (offset_of!(d::State, cw), offset_of!(c::State, cw)),
            (offset_of!(d::State, vw), offset_of!(c::State, vw)),
            (offset_of!(d::State, cut), offset_of!(c::State, cut)),
            (offset_of!(d::State, per), offset_of!(c::State, per)),
            (offset_of!(d::State, stride), offset_of!(c::State, stride)),
            (offset_of!(d::State, at), offset_of!(c::State, at)),
            (offset_of!(d::State, flags), offset_of!(c::State, flags)),
            (offset_of!(d::State, kw), offset_of!(c::State, kw)),
            (offset_of!(d::State, fw), offset_of!(c::State, fw)),
            (offset_of!(d::State, tag), offset_of!(c::State, tag)),
            (offset_of!(d::State, mark), offset_of!(c::State, mark)),
            (offset_of!(d::State, newlines), offset_of!(c::State, newlines)),
            (offset_of!(d::State, n), offset_of!(c::State, n)),
            (offset_of!(d::State, frontier), offset_of!(c::State, frontier)),
            (offset_of!(d::State, bad), offset_of!(c::State, bad)),
            (offset_of!(d::State, bad_node), offset_of!(c::State, bad_node)),
            (offset_of!(d::State, open_len), offset_of!(c::State, open_len)),
            (offset_of!(d::State, blocks), offset_of!(c::State, blocks)),
            (offset_of!(d::State, zbuckets), offset_of!(c::State, zbuckets)),
            (offset_of!(d::State, zbase), offset_of!(c::State, zbase)),
            (offset_of!(d::State, zprev), offset_of!(c::State, zprev)),
            (offset_of!(d::State, znext), offset_of!(c::State, znext)),
            (offset_of!(d::State, zoff), offset_of!(c::State, zoff)),
            (offset_of!(d::State, zback), offset_of!(c::State, zback)),
            (offset_of!(d::State, parts), offset_of!(c::State, kinds)),
            (offset_of!(d::State, parts) + d::part::LZ * size_of::<d::Part>(), offset_of!(c::State, lz)),
            (offset_of!(d::Part, at), offset_of!(c::Part, at)),
            (offset_of!(d::Part, cap), offset_of!(c::Part, cap)),
            (offset_of!(d::Part, len), offset_of!(c::Part, len)),
        ];
        for (k, (a, b)) in pairs.into_iter().enumerate() {
            assert_eq!(a, b, "field {k}");
        }
        let parts = [
            (d::part::KINDS, c::part::KINDS),
            (d::part::OPEN, c::part::OPEN),
            (d::part::OUT, c::part::OUT),
            (d::part::TABLE, c::part::TABLE),
            (d::part::SIDES, c::part::SIDES),
            (d::part::FREE, c::part::FREE),
            (d::part::LAID, c::part::LAID),
            (d::part::LAID_AT, c::part::LAID_AT),
            (d::part::PRESENCE, c::part::PRESENCE),
            (d::part::LINES, c::part::LINES),
            (d::part::ROOMS, c::part::ROOMS),
            (d::part::ROOM_FREE, c::part::ROOM_FREE),
            (d::part::PIECES, c::part::PIECES),
            (d::part::SEEN, c::part::SEEN),
            (d::part::CKPT, c::part::CKPT),
            (d::part::LZ, c::part::LZ),
        ];
        for (a, b) in parts {
            assert_eq!(a, b as usize);
        }
        assert_eq!(d::part::COUNT, c::part::COUNT);
        let consts = [
            (u64::from(d::Z), u64::from(c::Z)),
            (u64::from(d::K), u64::from(c::K)),
            (u64::from(d::BLOCK), u64::from(c::BLOCK)),
            (d::HEAD16, u64::from(c::HEAD16)),
            (u64::from(d::MAX_NODES), u64::from(c::MAX_NODES)),
            (u64::from(d::NONE), u64::from(c::NONE)),
            (d::MAGIC16, c::MAGIC16),
            (d::MAGIC16Z, c::MAGIC16Z),
            (d::OPEN, u64::from(c::OPEN)),
            (d::TABLE, u64::from(c::TABLE)),
            (d::LAID_AT, u64::from(c::LAID_AT)),
            (d::SIDE_LIST, u64::from(c::SIDE_LIST)),
            (d::SIDE_AREA, u64::from(c::SIDE_AREA)),
            (d::PIECE, u64::from(c::PIECE)),
            (u64::from(d::RAW), u64::from(c::RAW)),
            (u64::from(d::LZ4), u64::from(c::LZ4)),
            (u64::from(d::REPEAT), u64::from(c::REPEAT)),
            (d::REPEAT_MIN, u64::from(c::REPEAT_MIN)),
            (d::LZ4_MIN, u64::from(c::LZ4_MIN)),
            (u64::from(d::LZ_HASH_LOG), u64::from(c::LZ_HASH_LOG)),
            (u64::from(d::LZ_TABLES), u64::from(c::LZ_TABLES)),
            (d::LZ_ATTEMPTS as u64, u64::from(c::LZ_ATTEMPTS)),
            (u64::from(d::fail::BOUNDS), u64::from(c::fail::BOUNDS)),
            (u64::from(d::fail::OUTSIDE), u64::from(c::fail::OUTSIDE)),
            (u64::from(d::fail::FINAL), u64::from(c::fail::FINAL)),
            (u64::from(d::fail::NOT_OPEN), u64::from(c::fail::NOT_OPEN)),
            (u64::from(d::fail::NO_OPEN), u64::from(c::fail::NO_OPEN)),
            (u64::from(d::fail::UNCLOSED), u64::from(c::fail::UNCLOSED)),
            (u64::from(d::fail::NUMBER), u64::from(c::fail::NUMBER)),
            (u64::from(d::fail::PASSED), u64::from(c::fail::PASSED)),
            (u64::from(d::fail::FULL), u64::from(c::fail::FULL)),
        ];
        for (k, (a, b)) in consts.into_iter().enumerate() {
            assert_eq!(a, b, "constant {k}");
        }
    }
}
