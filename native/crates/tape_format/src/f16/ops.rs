//! Format 16 written by the writer's ops in CLIF (decision 0399 §6), through their Rust (`generated_writer`): the
//! host's region, each part its own buffer, named in the state by its offset from the state's address. Before each
//! op the host reserves what the op can need, as a `Vec` reserves before a push, so an op never finds a part full:
//! a full part is the device's failure, whose document is parsed again on the host.

use super::{Finished, K, Lang, Presence, Tape16, Tape16Z, Written, Z, before, bytes_for, mask, stride_of};
use crate::generated_writer as ops;
use rerail_clif::writer::{self as w, LAID_AT, OPEN, Part, SIDE_AREA, State, TABLE, part};

/// The most a block's four side-list parts take, laid (`w_lay`): each a u16 a sub-block, a byte and 8 bytes an
/// entry, and 8 bytes of room.
const LAID_MAX: usize = 4 * (2 * (Z / 256) + 9 * Z) + 8;
/// The most a 16z piece holds raw (a column of 8-byte cells and its part), and the most a final block adds to
/// the payload while its pieces are stored (each raw at the end, the last one's LZ4 room past it).
const PIECE_MAX: usize = 8 * Z + (2 * (Z / 256) + 9 * Z);
const ZBLOCK_MAX: usize = 4 * PIECE_MAX + w::lz4_bound(PIECE_MAX as u64) as usize + 64;

/// What a fresh byte of the tape's buffer holds: in tests not zero, so a byte the ops do not write shows.
const FRESH: u8 = if cfg!(test) { 0xa5 } else { 0 };

/// A document's writer whose ops are the CLIF ones: `f16::Writer`'s API.
pub struct OpsWriter {
    state: Box<State>,
    /// The parts' buffers, by `part::*`, each its capacity long.
    parts: [Vec<u8>; part::COUNT],
    presence_words: (usize, usize),
    keep_blocks: bool,
    z: bool,
    /// A part held at its capacity (a test's: the device's region, which the host does not grow).
    #[cfg(test)]
    fixed: Option<(u32, u64)>,
}

impl OpsWriter {
    /// `f16::Writer::new`: 16z where `z`.
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
            if z { w::HEAD16 as usize + ZBLOCK_MAX + 64 } else { w::HEAD16 as usize + count * stride + 64 },
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
        let mut wr = OpsWriter { state: Box::new(State::default()), parts, presence_words: (kind_words, field_words), keep_blocks: blocks, z, #[cfg(test)] fixed: None };
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
        let mut s = *self.state;
        self.part_mut(&mut s, k as usize).len as usize
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

    /// `f16::Writer::open`.
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
        unsafe { ops::w_open(desc, number, start) };
        i
    }

    /// `f16::Writer::close`.
    pub fn close(&mut self, end: u64) {
        self.reserve_final(1);
        let desc = self.desc();
        // SAFETY: as `open`'s.
        unsafe { ops::w_close(desc, end) };
    }

    /// `f16::Writer::leaf`.
    pub fn leaf(&mut self, number: u32, start: u64, end: u64) -> usize {
        let i = self.open(number, start);
        self.close(end);
        i
    }

    /// `f16::Writer::commit`.
    pub fn commit(&mut self) {
        let blocks = (self.state.n as usize / Z).saturating_sub(self.state.frontier as usize / Z);
        self.reserve_final(blocks);
        let desc = self.desc();
        // SAFETY: as `open`'s.
        unsafe { ops::w_commit(desc) };
    }

    /// `f16::Writer::retag`.
    pub fn retag(&mut self, i: usize, number: u32) {
        let desc = self.desc();
        // SAFETY: as `open`'s; a code's entry takes no room past its list's.
        unsafe { ops::w_retag(desc, i as u32, number) };
    }

    /// `f16::Writer::set_start`.
    pub fn set_start(&mut self, i: usize, start: u64) {
        let desc = self.desc();
        // SAFETY: as `open`'s.
        unsafe { ops::w_set_start(desc, i as u32, start) };
    }

    /// The failure the ops kept, as a code and its node.
    pub fn failure(&self) -> Option<(u32, u32)> {
        (self.state.bad != 0).then_some((self.state.bad, self.state.bad_node))
    }

    /// `f16::Writer::finish`: the tape and its presence.
    pub fn finish(mut self) -> Result<Finished, String> {
        let count = (self.state.n as usize).div_ceil(Z);
        self.reserve_final(count);
        let desc = self.desc();
        // SAFETY: as `open`'s.
        let mut r = unsafe { ops::w_finish_blocks(desc) };
        if r == 0 {
            // The tail's room, the blocks' laid parts now known.
            let need = self.len_of(part::OUT) + self.len_of(part::LAID) + 16 * (count + 1) + self.len_of(part::LINES) + self.len_of(part::CKPT) + 9 * 4 * count + 64;
            self.reserve(part::OUT, need);
            let desc = self.desc();
            // SAFETY: as `open`'s.
            r = unsafe { ops::w_finish_tail(desc) };
        }
        if r != 0 {
            return Err(format!("the writer's ops failed: {r:#x} at node {}", self.state.bad_node as i32));
        }
        let (kind_words, field_words) = self.presence_words;
        let words = kind_words + field_words;
        // The presence leaves the writer as the store keeps it, words in a `Vec` (`Presence`).
        let word = |k: usize| u64::from_le_bytes(self.parts[part::PRESENCE as usize][8 * k..8 * k + 8].try_into().expect("8 bytes"));
        let doc = (0..words).map(word).collect();
        let blocks = if self.keep_blocks { (words..words * (1 + count)).map(word).collect() } else { Vec::new() };
        let presence = Presence { kind_words, field_words, doc, blocks };
        let end = self.len_of(part::OUT);
        let mut out = std::mem::take(&mut self.parts[part::OUT as usize]);
        out.truncate(end);
        let tape = if self.z { Written::F16Z(Tape16Z::from_bytes(out)?) } else { Written::F16(Tape16::from_bytes(out)?) };
        Ok(Finished { tape, presence })
    }
}

/// `f16::write_from` by the ops in CLIF.
pub fn write_from<T: crate::tape::Tape>(lang: &Lang, t: &T, number: &[u32], source: &[u8], tag: u8, z: bool) -> Result<Written, String> {
    super::write_with(OpsWriter::new(lang, source, tag, z, false, t.len()), t.len(), super::nodes_of(t, number))
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
