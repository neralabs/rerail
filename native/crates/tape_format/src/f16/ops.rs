//! Format 16 written by the writer's ops in CLIF (decision 0399 §6), through their Rust (`generated_writer`): the
//! host's region, each part its own buffer, named in the state by its offset from the state's address. Before each
//! op the host reserves what the op can need, as a `Vec` reserves before a push, so an op never finds a part full:
//! a full part is the device's failure, whose document is parsed again on the host.

use super::{Finished, Lang, Presence, Tape16, Written, Z, before, bytes_for, mask, stride_of};
use crate::generated_writer as ops;
use rerail_clif::writer::{self as w, LAID_AT, OPEN, Part, SIDE_AREA, State, TABLE, part};

/// The most a block's four side-list parts take, laid (`w_lay`): each a u16 a sub-block, a byte and 8 bytes an
/// entry, and 8 bytes of room.
const LAID_MAX: usize = 4 * (2 * (Z / 256) + 9 * Z) + 8;

/// A document's writer whose ops are the CLIF ones: `f16::Writer`'s API.
pub struct OpsWriter {
    state: Box<State>,
    /// The parts' buffers, by `part::*`, each its capacity long.
    parts: [Vec<u8>; 10],
    presence_words: (usize, usize),
    keep_blocks: bool,
    /// A part held at its capacity (a test's: the device's region, which the host does not grow).
    #[cfg(test)]
    fixed: Option<(u32, u64)>,
}

impl OpsWriter {
    /// `f16::Writer::new` (format 16; 16z is not written by the ops yet).
    pub fn new(lang: &Lang, source: &[u8], tag: u8, blocks: bool, nodes: usize) -> Self {
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
            flags: u32::from(blocks),
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
        let mut parts: [Vec<u8>; 10] = Default::default();
        let lens = [kinds.len() as u64, 0, 0, 0, 0, 0, 0, 0, words as u64, lines.len() as u64];
        let caps = [
            kinds.len() + 8,
            64 * OPEN as usize,
            w::HEAD16 as usize + count * state.stride as usize + 64,
            count * TABLE as usize,
            2 * SIDE_AREA as usize,
            64,
            LAID_MAX,
            count * LAID_AT as usize,
            words * (1 + if blocks { count + 1 } else { 0 }) + 8,
            lines.len() + 8,
        ];
        // The kinds' table and the newline index are their parts, made where they are computed.
        parts[part::KINDS as usize] = kinds;
        parts[part::LINES as usize] = lines;
        for (buf, &cap) in parts.iter_mut().zip(&caps) {
            buf.resize(cap, 0);
        }
        let mut wr = OpsWriter { state: Box::new(State::default()), parts, presence_words: (kind_words, field_words), keep_blocks: blocks, #[cfg(test)] fixed: None };
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
            _ => &mut state.lines,
        }
    }

    /// Each part's place and capacity in the state, from its buffer (8 bytes of each kept as room past its last).
    fn bind(&mut self) {
        let base = std::ptr::from_ref::<State>(&self.state) as i64;
        let mut state = *self.state;
        for k in 0..10 {
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
        self.parts[k as usize].resize(cap, 0);
        self.bind();
    }

    /// Room for `blocks` blocks made final: their laid parts, their side areas given back, their entries.
    fn reserve_final(&mut self, blocks: usize) {
        let (laid, free) = (self.len_of(part::LAID), self.len_of(part::FREE));
        self.reserve(part::LAID, laid + blocks * LAID_MAX);
        self.reserve(part::FREE, free + 4 * blocks);
        let count = self.state.blocks as usize;
        self.reserve(part::LAID_AT, count * LAID_AT as usize);
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
            self.reserve(part::OUT, w::HEAD16 as usize + (b + 1) * stride);
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
        let need = self.len_of(part::OUT) + self.len_of(part::LAID) + 16 * (count + 1) + self.len_of(part::LINES) + 64;
        self.reserve(part::OUT, need);
        let desc = self.desc();
        // SAFETY: as `open`'s.
        let r = unsafe { ops::w_finish(desc) };
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
        Ok(Finished { tape: Written::F16(Tape16::from_bytes(out)?), presence })
    }
}
