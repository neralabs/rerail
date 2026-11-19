//! Format 16 written by the writer's ops in CLIF (decision 0434 §6), through their Rust (`generated_writer`): the
//! host's region, each part its own buffer, named in the state by its offset from the state's address. Before each
//! op the host reserves what the op can need, as a `Vec` reserves before a push, so an op never finds a part full:
//! a full part is the device's failure, whose document is parsed again on the host.

use super::{Finished, K, Lang, Tape16, Tape16Z, Written, Z, before, bytes_for, mask, stride_of};
use crate::generated_writer as ops;
use rerail_clif::writer::{self as w, LAID_AT, OPEN, Part, SIDE_AREA, State, TABLE, part};

/// An op called by its generated Rust; in a test, by the JIT's where the writer has it (`Jit`).
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
    pub fn new(lang: &Lang, source: &[u8], tag: u8, z: bool, blocks: bool, nodes: usize) -> Self {
        let source_len = source.len() as u64;
        let wd = lang.widths;
        let cw = bytes_for(source_len);
        let lines = crate::newlines::index(source);
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
        let mut parts: [Vec<u8>; part::COUNT] = Default::default();
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
        parts[part::KINDS as usize] = kinds;
        parts[part::LINES as usize] = lines;
        for (buf, &cap) in parts.iter_mut().zip(&caps) {
            buf.resize(cap, 0);
        }
        parts[part::OUT as usize].fill(FRESH);
        parts[part::SEEN as usize].fill(0xff);
        let mut wr = Writer { state: Box::new(State::default()), parts, presence_words: (kind_words, field_words), keep_blocks: blocks, z, #[cfg(test)] fixed: None, #[cfg(test)] jit: None };
        for (k, &len) in lens.iter().enumerate() {
            *wr.part_mut(&mut state, k) = Part { at: 0, cap: 0, len };
        }
        wr.state = state;
        wr.bind();
        wr
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
        let desc = self.desc();
        // SAFETY: the state's parts lie in their buffers, each with the room the op can need, reserved above.
        call!(self, w_open(desc, number, start));
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
        let i = self.open(number, start);
        self.close(end);
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
    pub fn finish(mut self) -> Result<Finished, String> {
        self.finish_ops()?;
        let count = (self.state.n as usize).div_ceil(Z);
        let end = self.len_of(part::OUT);
        let mut out = std::mem::take(&mut self.parts[part::OUT as usize]);
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

/// The ops in MSL on the device (decision 0434 §6, R15), a test's: the calls are recorded, then replayed by one
/// thread of a kernel over the region in one buffer, the state at its start and each part at its offset from it, as
/// the host names them. The device's region does not grow: each part gets the room the host's run of the same calls
/// reserved (sizing it from bounds is tier 2's, R17).
#[cfg(all(test, target_os = "macos"))]
pub(super) mod device {
    use super::{FRESH, Writer, State, Z, finished, part};
    use crate::f16::{Finished, Lang};
    use metal::{CompileOptions, Device, MTLResourceOptions, MTLSize};

    /// The kernel: the calls `c` (an op and up to three arguments each) through the ops, then `finish`.
    const REPLAY: &str = r#"
kernel void replay(device uchar* r [[buffer(0)]], device const ulong* c [[buffer(1)]], constant uint& n [[buffer(2)]],
                   device uint* out [[buffer(3)]], uint id [[thread_position_in_grid]]) {
    if (id != 0u) return;
    device const uchar* t = r;
    for (uint k = 0u; k < n; k++) {
        device const ulong* e = c + 4u * k;
        switch (uint(e[0])) {
            case 0u: clif_w_open(t, 0ul, uint(e[1]), e[2]); break;
            case 1u: clif_w_close(t, 0ul, e[1]); break;
            case 2u: clif_w_leaf(t, 0ul, uint(e[1]), e[2], e[3]); break;
            case 3u: clif_w_commit(t, 0ul); break;
            case 4u: clif_w_retag(t, 0ul, uint(e[1]), uint(e[2])); break;
            default: clif_w_set_start(t, 0ul, uint(e[1]), e[2]); break;
        }
    }
    out[0] = clif_w_finish(t, 0ul);
}
"#;

    pub(in crate::f16) struct DeviceOps {
        device: Device,
        /// The host's run of the calls, for the room each part needs; and the writer as made, for the region.
        host: Writer,
        made: Writer,
        calls: Vec<[u64; 4]>,
    }

    impl DeviceOps {
        pub(in crate::f16) fn new(device: &Device, lang: &Lang, source: &[u8], tag: u8, z: bool, blocks: bool, nodes: usize) -> Self {
            DeviceOps { device: device.clone(), host: Writer::new(lang, source, tag, z, blocks, nodes), made: Writer::new(lang, source, tag, z, blocks, nodes), calls: Vec::new() }
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
            let region = self.device.new_buffer(end as u64, MTLResourceOptions::StorageModeShared);
            // SAFETY: the buffer is `end` bytes, shared with the host, and no command uses it yet.
            let bytes = unsafe { std::slice::from_raw_parts_mut(region.contents().cast::<u8>(), end) };
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
            let source = format!("#include <metal_stdlib>\nusing namespace metal;\n{}{REPLAY}", crate::WRITER_MSL);
            let lib = self.device.new_library_with_source(&source, &CompileOptions::new()).expect("the writer's ops compile in MSL");
            let pipe = self.device.new_compute_pipeline_state_with_function(&lib.get_function("replay", None).expect("the kernel")).expect("its pipeline");
            let n = self.calls.len() as u32;
            // At least one call's room: a buffer is never empty.
            self.calls.push([0; 4]);
            let calls = self.device.new_buffer_with_data(self.calls.as_ptr().cast(), 32 * self.calls.len() as u64, MTLResourceOptions::StorageModeShared);
            let result = self.device.new_buffer(4, MTLResourceOptions::StorageModeShared);
            let queue = self.device.new_command_queue();
            let cmd = queue.new_command_buffer();
            let enc = cmd.new_compute_command_encoder();
            enc.set_compute_pipeline_state(&pipe);
            enc.set_buffer(0, Some(&region), 0);
            enc.set_buffer(1, Some(&calls), 0);
            enc.set_bytes(2, 4, (&raw const n).cast());
            enc.set_buffer(3, Some(&result), 0);
            enc.dispatch_threads(MTLSize::new(1, 1, 1), MTLSize::new(1, 1, 1));
            enc.end_encoding();
            cmd.commit();
            cmd.wait_until_completed();
            // SAFETY: the command completed; the kernel wrote the result's u32 and the region.
            let r = unsafe { *result.contents().cast::<u32>() };
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
