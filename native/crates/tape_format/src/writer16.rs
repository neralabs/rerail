// The format 16 writer (decision 0434 §6, 0532), written in Rust directly: the host's writer (`f16::Writer`). The
// shape is simdjson's `tape_builder` and `tape_writer`: the state's named fields, each part a buffer named by its
// offset from the state, each cell a store at its column's offset from its block. The JIT's and the device's
// writer is the same writer in CLIF (`rerail_clif::writer`); the tests hold the three to the same bytes, and the
// state's layout to `rerail_clif::writer::State`'s.
//
// It reads nothing but its state and its parts: no `crate::` path, so that it compiles where it is included.

/// A block's nodes, and a checkpoint's.
pub const Z: u32 = 1024;
pub const K: u32 = 8;
/// A side list's sub-block.
pub const BLOCK: u32 = 256;
/// The header's bytes.
pub const HEAD16: u64 = 120;
pub const MAX_NODES: u32 = 1 << 30;
pub const NONE: u32 = u32::MAX;
/// The state's flag: each node is final as it is written (no `retag` or `set_start`), so the frontier is the count
/// and a block settles once the node after it opens or its last open node closes, as after a commit each node.
pub const FINAL_AS_WRITTEN: u32 = 4;
pub const MAGIC16: u64 = u64::from_le_bytes(*b"TMPTAPB6");
pub const MAGIC16Z: u64 = u64::from_le_bytes(*b"TMPTAPBZ");
/// An open node's frame: its node (u32 at 0), its children so far (4), its last child (8), its start (u64 at 16).
pub const OPEN: u64 = 24;
/// A block's table entry: its room (u32 at 0), its side area (4), whether it is final (8).
pub const TABLE: u64 = 16;
/// A final block's laid side-list parts: for each list, its start and length in `laid` (u32s).
pub const LAID_AT: u64 = 32;
/// A side list: up to `Z` u16 offsets, then up to `Z` u64 values, after 16 bytes; an area: 4 counts, 4 lists.
pub const SIDE_LIST: u64 = 2 * Z as u64 + 8 * Z as u64;
/// The ERROR node's code: the presence's bit a fault sets (`crate::ERROR_CODE` is this one).
pub const ERROR_CODE: u16 = 0;
/// A kind word's bit that says its code is MISSING (`f16::presence::FAULT` is this one), and its kind's bits.
pub const FAULT: u32 = 1 << 15;
pub const KIND_MASK: u32 = FAULT - 1;
pub const SIDE_AREA: u64 = 16 + 4 * SIDE_LIST;
/// 16z: a piece's entry, its kinds, the smallest piece a repeat names and LZ4 is tried on.
pub const PIECE: u64 = 32;
pub const RAW: u32 = 0;
pub const LZ4: u32 = 1;
pub const REPEAT: u32 = 2;
pub const REPEAT_MIN: u64 = 8;
pub const LZ4_MIN: u64 = 16;
/// The LZ4 compressor's tables: a hash of `1 << LZ_HASH_LOG` u32s, then a chain of 64 Ki u16s; its attempts.
pub const LZ_HASH_LOG: u32 = 15;
pub const LZ_TABLES: u32 = (4 << LZ_HASH_LOG) + 2 * 65536;
pub const LZ_ATTEMPTS: i64 = 8;

/// The failures (`State::bad`).
pub mod fail {
    pub const BOUNDS: u32 = 1;
    pub const OUTSIDE: u32 = 2;
    pub const FINAL: u32 = 3;
    pub const NOT_OPEN: u32 = 4;
    pub const NO_OPEN: u32 = 5;
    pub const UNCLOSED: u32 = 6;
    pub const NUMBER: u32 = 7;
    pub const PASSED: u32 = 8;
    /// A part with no room: `FULL + its number`.
    pub const FULL: u32 = 0x100;
}

/// The parts, by their place in `State::parts`.
pub mod part {
    pub const KINDS: usize = 0;
    pub const OPEN: usize = 1;
    pub const OUT: usize = 2;
    pub const TABLE: usize = 3;
    pub const SIDES: usize = 4;
    pub const FREE: usize = 5;
    pub const LAID: usize = 6;
    pub const LAID_AT: usize = 7;
    pub const PRESENCE: usize = 8;
    pub const LINES: usize = 9;
    pub const ROOMS: usize = 10;
    pub const ROOM_FREE: usize = 11;
    pub const PIECES: usize = 12;
    pub const SEEN: usize = 13;
    pub const CKPT: usize = 14;
    pub const LZ: usize = 15;
    pub const ERRORS: usize = 16;
    pub const COUNT: usize = 17;
}

/// A part: where it lies (an offset from the state's address), its capacity and what it holds, in bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Part {
    pub at: i64,
    pub cap: u64,
    pub len: u64,
}

/// The writer's state: the document's constants, the writer's variables, its parts (`rerail_clif::writer::State`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct State {
    /// Each array's escape (slots, gaps, lengths): its widest value, which says the value is its side list's.
    pub esc: [u64; 3],
    pub source_len: u64,
    pub prev_start: u64,
    /// The arrays' widths; the checkpoints'; the side lists' values'.
    pub w: [u32; 3],
    pub cw: u32,
    pub vw: [u32; 4],
    /// The code byte's cutoff, and the numbers a prefix byte covers.
    pub cut: u32,
    pub per: u32,
    /// A block's bytes, and where each column starts in it (codes, slots, gaps, lengths, checkpoints).
    pub stride: u32,
    pub at: [u32; 5],
    /// 1: a presence a block is kept; 2: the tape is 16z; 4: each node final as written (`FINAL_AS_WRITTEN`).
    pub flags: u32,
    pub kw: u32,
    pub fw: u32,
    pub tag: u32,
    pub mark: u32,
    pub newlines: u32,
    /// The nodes so far; the first not yet committed.
    pub n: u32,
    pub frontier: u32,
    /// The first failure, and its node.
    pub bad: u32,
    pub bad_node: u32,
    pub open_len: u32,
    pub blocks: u32,
    /// 16z: the seen pieces' buckets; the LZ4 compressor's base, last input, next index, last offset and back.
    pub zbuckets: u32,
    pub zbase: u32,
    pub zprev: u32,
    pub znext: u32,
    pub zoff: u32,
    pub zback: u32,
    pub parts: [Part; part::COUNT],
}

/// An op's result: `Err` is a failure's code, already kept (`fail`).
type R = Result<(), u32>;

fn code(r: R) -> u32 {
    match r {
        Ok(()) => 0,
        Err(c) => c,
    }
}

// ---- bytes at an address ------------------------------------------------------------------------------------

#[inline(always)]
unsafe fn ld8(a: usize) -> u32 {
    // SAFETY: the caller's: `a` is in a part.
    u32::from(unsafe { (a as *const u8).read() })
}
#[inline(always)]
unsafe fn ld16(a: usize) -> u32 {
    // SAFETY: as `ld8`'s.
    u32::from(unsafe { (a as *const u16).read_unaligned() })
}
#[inline(always)]
unsafe fn ld32(a: usize) -> u32 {
    // SAFETY: as `ld8`'s.
    unsafe { (a as *const u32).read_unaligned() }
}
#[inline(always)]
unsafe fn ld64(a: usize) -> u64 {
    // SAFETY: as `ld8`'s.
    unsafe { (a as *const u64).read_unaligned() }
}
#[inline(always)]
unsafe fn st8(a: usize, v: u32) {
    // SAFETY: as `ld8`'s.
    unsafe { (a as *mut u8).write(v as u8) }
}
#[inline(always)]
unsafe fn st16(a: usize, v: u32) {
    // SAFETY: as `ld8`'s.
    unsafe { (a as *mut u16).write_unaligned(v as u16) }
}
#[inline(always)]
unsafe fn st32(a: usize, v: u32) {
    // SAFETY: as `ld8`'s.
    unsafe { (a as *mut u32).write_unaligned(v) }
}
#[inline(always)]
unsafe fn st64(a: usize, v: u64) {
    // SAFETY: as `ld8`'s.
    unsafe { (a as *mut u64).write_unaligned(v) }
}

/// The low `w` bytes' mask, all ones from 8.
#[inline(always)]
fn mask(w: u64) -> u64 {
    if w >= 8 { u64::MAX } else { (1u64 << (8 * w)) - 1 }
}

/// The cell of `w` bytes at `a`, written as a word: `v`'s bytes, the bytes past it kept.
#[inline(always)]
unsafe fn cell_put(a: usize, w: u64, v: u64) {
    // SAFETY: the caller's: the word at `a` is in a part.
    unsafe { cell_put_masked(a, mask(w), v) }
}
#[inline(always)]
unsafe fn cell_put_masked(a: usize, m: u64, v: u64) {
    // SAFETY: as `cell_put`'s.
    unsafe { st64(a, ld64(a) & !m | v & m) }
}
#[inline(always)]
unsafe fn cell_get(a: usize, w: u64) -> u64 {
    // SAFETY: as `cell_put`'s.
    unsafe { ld64(a) & mask(w) }
}

/// `len` bytes moved down or apart, a word at a time, the last word's bytes past `len` kept.
unsafe fn copy(to: usize, from: usize, len: u64) {
    let mut k = 0u64;
    // SAFETY: the caller's: both runs, and the word past each, are in parts.
    unsafe {
        while k + 8 <= len {
            st64(to + k as usize, ld64(from + k as usize));
            k += 8;
        }
        if len != k {
            cell_put(to + k as usize, len - k, ld64(from + k as usize));
        }
    }
}

/// `len` bytes (a multiple of 8) zeroed.
unsafe fn zero(to: usize, len: u64) {
    let mut k = 0u64;
    while k < len {
        // SAFETY: the caller's.
        unsafe { st64(to + k as usize, 0) };
        k += 8;
    }
}

/// `len` bytes of `v`'s, the bytes past `len` kept.
unsafe fn fill(to: usize, len: u64, v: u64) {
    let mut k = 0u64;
    // SAFETY: the caller's.
    unsafe {
        while k + 8 <= len {
            st64(to + k as usize, v);
            k += 8;
        }
        if len != k {
            cell_put(to + k as usize, len - k, v);
        }
    }
}

// ---- the state and its parts ----------------------------------------------------------------------------------

/// A side list `q` of an area: its count's address, its offsets', its values', and its count.
#[derive(Clone, Copy)]
struct List {
    count_at: usize,
    js: usize,
    vs: usize,
    count: u32,
}

impl List {
    unsafe fn of(area: usize, q: usize) -> List {
        let base = area + q * SIDE_LIST as usize;
        // SAFETY: the caller's: `area` is a side area's.
        let count = unsafe { ld32(area + 4 * q) };
        List { count_at: area + 4 * q, js: base + 16, vs: base + 16 + 2 * Z as usize, count }
    }
    unsafe fn j(&self, k: u32) -> u32 {
        // SAFETY: an entry of the list's room (`Z` of them).
        unsafe { ld16(self.js + 2 * k as usize) }
    }
    unsafe fn v(&self, k: u32) -> u64 {
        // SAFETY: as `j`'s.
        unsafe { ld64(self.vs + 8 * k as usize) }
    }

    /// The first entry not below `j` (the offsets ascend).
    unsafe fn find(&self, j: u32) -> u32 {
        let (mut lo, mut hi) = (0u32, self.count);
        while lo < hi {
            let mid = (lo + hi) >> 1;
            // SAFETY: `mid` is below the count.
            if unsafe { self.j(mid) } < j { lo = mid + 1 } else { hi = mid }
        }
        lo
    }

    /// Where entry `j` goes: past the last where the last is below it (entries come in row order but for a late
    /// close or a retag), else by `find`.
    unsafe fn place(&self, j: u32) -> u32 {
        // SAFETY: the last entry is in the list.
        if self.count != 0 && unsafe { self.j(self.count - 1) } >= j {
            // SAFETY: as above.
            return unsafe { self.find(j) };
        }
        self.count
    }

    /// Whether entry `k` is `j`'s.
    unsafe fn is(&self, k: u32, j: u32) -> bool {
        // SAFETY: `k` is below the count.
        k < self.count && unsafe { self.j(k) } == j
    }

    /// Entry `(j, v)` at `k`, the ones from `k` moved up one.
    unsafe fn insert(&self, k: u32, j: u32, v: u64) {
        // SAFETY: the list's room holds `Z` entries, one past each of a block's.
        unsafe {
            let mut m = self.count;
            while m > k {
                st16(self.js + 2 * m as usize, self.j(m - 1));
                st64(self.vs + 8 * m as usize, self.v(m - 1));
                m -= 1;
            }
            st16(self.js + 2 * k as usize, j);
            st64(self.vs + 8 * k as usize, v);
            st32(self.count_at, self.count + 1);
        }
    }

    /// Entry `k` removed, the ones after it moved down one.
    unsafe fn remove(&self, k: u32) {
        let last = self.count - 1;
        // SAFETY: as `insert`'s.
        unsafe {
            for m in k..last {
                st16(self.js + 2 * m as usize, self.j(m + 1));
                st64(self.vs + 8 * m as usize, self.v(m + 1));
            }
            st32(self.count_at, last);
        }
    }
}

/// A node's block, as an op uses it: its room's address and its side area's.
#[derive(Clone, Copy)]
struct Block {
    room: usize,
    area: usize,
}

/// Where the ops that write a node's cells (an open, a leaf, a close) take the widths from: the state's (`Run`, any
/// language), or constants (`Fixed`, a language whose widths the caller knows, as a generated parser knows the one
/// it makes): then a cell is one store of its width at a constant offset, not a word read and written under the
/// state's mask. The bytes are the same.
pub trait CellWidths {
    /// Array `a`'s width (0 slots, 1 gaps, 2 lengths).
    fn w(s: &State, a: usize) -> u32;
    /// Column `r`'s offset in a block's room (1 slots, 2 gaps, 3 lengths, 4 checkpoints).
    fn at(s: &State, r: usize) -> u32;
    /// Array `a`'s escape: its widest value.
    fn esc(s: &State, a: usize) -> u64;
    /// The cell at `a` of array `k`, `v` (at most its escape `m`).
    ///
    /// # Safety
    /// As `cell_put`'s.
    unsafe fn put(s: &State, k: usize, a: usize, m: u64, v: u64);
}

/// The state's widths.
pub struct Run;

impl CellWidths for Run {
    #[inline(always)]
    fn w(s: &State, a: usize) -> u32 {
        s.w[a]
    }
    #[inline(always)]
    fn at(s: &State, r: usize) -> u32 {
        s.at[r]
    }
    #[inline(always)]
    fn esc(s: &State, a: usize) -> u64 {
        s.esc[a]
    }
    #[inline(always)]
    unsafe fn put(_: &State, _: usize, a: usize, m: u64, v: u64) {
        // SAFETY: the caller's.
        unsafe { cell_put_masked(a, m, v) }
    }
}

/// Widths known to the caller: slots `S`, gaps `G`, lengths `L` bytes, the state's own (a debug build checks).
pub struct Fixed<const S: u32, const G: u32, const L: u32>;

impl<const S: u32, const G: u32, const L: u32> Fixed<S, G, L> {
    const W: [u32; 3] = [S, G, L];
    const AT: [u32; 5] = [0, Z, Z * (1 + S), Z * (1 + S + G), Z * (1 + S + G + L)];
}

impl<const S: u32, const G: u32, const L: u32> CellWidths for Fixed<S, G, L> {
    #[inline(always)]
    fn w(s: &State, a: usize) -> u32 {
        debug_assert_eq!(s.w, Self::W, "the state's widths are the caller's");
        let _ = s;
        Self::W[a]
    }
    #[inline(always)]
    fn at(s: &State, r: usize) -> u32 {
        debug_assert_eq!(s.at[r], Self::AT[r], "the state's columns are the caller's widths'");
        let _ = s;
        Self::AT[r]
    }
    #[inline(always)]
    fn esc(_: &State, a: usize) -> u64 {
        mask(u64::from(Self::W[a]))
    }
    #[inline(always)]
    unsafe fn put(_: &State, k: usize, a: usize, m: u64, v: u64) {
        // SAFETY: the caller's; a store of the cell's width, the bytes past it kept as the masked word keeps them.
        unsafe {
            match Self::W[k] {
                1 => st8(a, v as u32),
                2 => st16(a, v as u32),
                4 => st32(a, v as u32),
                8 => st64(a, v),
                _ => cell_put_masked(a, m, v),
            }
        }
    }
}

impl State {
    /// Part `k`'s address.
    #[inline(always)]
    fn part(&self, k: usize) -> usize {
        (self as *const State as usize).wrapping_add(self.parts[k].at as usize)
    }
    fn len(&self, k: usize) -> u64 {
        self.parts[k].len
    }
    fn set_len(&mut self, k: usize, len: u64) {
        self.parts[k].len = len;
    }
    /// A part's length grown to `len`.
    fn grow(&mut self, k: usize, len: u64) {
        if self.parts[k].len < len {
            self.parts[k].len = len;
        }
    }

    /// The first failure kept; `code`.
    fn fail<T>(&mut self, code: u32, node: u32) -> Result<T, u32> {
        if self.bad == 0 {
            self.bad = code;
            self.bad_node = node;
        }
        Err(code)
    }

    /// Fails as full unless part `k` holds `need` bytes.
    fn room(&mut self, k: usize, need: u64) -> R {
        if need > self.parts[k].cap {
            return self.fail(fail::FULL + k as u32, NONE);
        }
        Ok(())
    }

    fn z(&self) -> bool {
        self.flags & 2 != 0
    }

    /// Where the blocks' rooms lie: in 16, the tape; in 16z, their own part.
    fn rooms(&self) -> usize {
        if self.z() { self.part(part::ROOMS) } else { self.part(part::OUT) }
    }

    /// Block `b`'s table entry.
    fn entry(&self, b: u32) -> usize {
        self.part(part::TABLE) + b as usize * TABLE as usize
    }

    /// Block `b`'s room and side area.
    unsafe fn block(&self, b: u32) -> Block {
        let e = self.entry(b);
        // SAFETY: the caller's: block `b` is made.
        unsafe {
            Block {
                room: self.rooms().wrapping_add(ld32(e) as usize),
                area: self.part(part::SIDES).wrapping_add((ld32(e + 4) as usize).wrapping_mul(SIDE_AREA as usize)),
            }
        }
    }

    /// The cell of node `j` of a block at `room` in array `a`.
    fn cell(&self, room: usize, a: usize, j: u32) -> usize {
        room + self.at[a + 1] as usize + j as usize * self.w[a] as usize
    }

    // ---- a node's cells -------------------------------------------------------------------------------------

    /// Node `j` of `block`'s value `v` in array `a` (0 slots, 1 gaps, 2 lengths): its cell, or its escape and its
    /// entry in its block's side list `a + 1`. A cell written again (`set_start`) drops the entry it had.
    ///
    /// Its widths `X`'s.
    #[inline(always)]
    unsafe fn put_at_x<X: CellWidths>(&mut self, a: usize, j: u32, v: u64, block: Block) {
        let esc = X::esc(self, a);
        // SAFETY: the caller's: `block` is made, `j` a row of it.
        unsafe {
            let list = List::of(block.area, a + 1);
            if v >= esc {
                let k = list.place(j);
                if list.is(k, j) {
                    st64(list.vs + 8 * k as usize, v);
                } else {
                    list.insert(k, j, v);
                }
            } else if list.count != 0 && list.j(list.count - 1) >= j {
                let k = list.find(j);
                if list.is(k, j) {
                    list.remove(k);
                }
            }
            X::put(self, a, block.room + X::at(self, a + 1) as usize + j as usize * X::w(self, a) as usize, esc, v.min(esc));
        }
    }

    /// Node `i`'s value in array `a` (`w_put`).
    unsafe fn put(&mut self, a: usize, i: u32, v: u64) -> R {
        // SAFETY: the caller's.
        unsafe { self.put_x::<Run>(a, i, v) }
    }

    /// `put`, its widths `X`'s.
    #[inline(always)]
    unsafe fn put_x<X: CellWidths>(&mut self, a: usize, i: u32, v: u64) -> R {
        // SAFETY: the caller's: node `i`'s block is made.
        unsafe {
            let block = self.block(i / Z);
            self.put_at_x::<X>(a, i % Z, v, block);
        }
        Ok(())
    }

    /// Node `i`'s value in array `a` as written: its cell, or its side-list entry where the cell is the escape.
    unsafe fn value(&self, a: usize, i: u32) -> u64 {
        let (b, j) = (i / Z, i % Z);
        // SAFETY: as `put`'s.
        unsafe {
            let block = self.block(b);
            let v = cell_get(self.cell(block.room, a, j), u64::from(self.w[a]));
            if v < self.esc[a] {
                return v;
            }
            let list = List::of(block.area, a + 1);
            let k = list.find(j);
            if k < list.count { list.v(k) } else { v }
        }
    }

    /// Node `j` of `block`'s code: its byte, and past the cut its side-list entry.
    unsafe fn put_code_at(&mut self, j: u32, number: u32, block: Block) {
        let cut = self.cut;
        // SAFETY: as `put_at`'s.
        unsafe {
            if number < cut {
                st8(block.room + j as usize, number);
                return;
            }
            let t = number - cut;
            st8(block.room + j as usize, cut + t / self.per);
            let list = List::of(block.area, 0);
            list.insert(list.place(j), j, u64::from(t % self.per));
        }
    }

    /// Node `j`'s checkpoint: `start` at the checkpoints' width.
    unsafe fn put_checkpoint(&mut self, room: usize, j: u32, start: u64) {
        // SAFETY: the caller's.
        unsafe { self.put_checkpoint_x::<Run>(room, j, start) }
    }

    /// `put_checkpoint`, its column's offset `X`'s.
    #[inline(always)]
    unsafe fn put_checkpoint_x<X: CellWidths>(&mut self, room: usize, j: u32, start: u64) {
        let a = room + X::at(self, 4) as usize + (j / K) as usize * self.cw as usize;
        // SAFETY: as `put_at`'s.
        unsafe { cell_put(a, u64::from(self.cw), start) };
    }

    // ---- blocks -----------------------------------------------------------------------------------------------

    /// Room for block `b` (zeroed), a side area, its table entry, its presence where kept.
    unsafe fn new_block(&mut self, b: u32) -> R {
        let stride = u64::from(self.stride);
        let b64 = u64::from(b);
        // The block's room: in 16 its place in the tape; in 16z one given back, else a new one.
        let room = if self.z() {
            let free = self.len(part::ROOM_FREE);
            if free == 0 {
                let made = self.len(part::ROOMS);
                self.room(part::ROOMS, made + stride)?;
                self.set_len(part::ROOMS, made + stride);
                made
            } else {
                self.set_len(part::ROOM_FREE, free - 4);
                // SAFETY: a room given back, in the part's length.
                u64::from(unsafe { ld32(self.part(part::ROOM_FREE) + free as usize - 4) })
            }
        } else {
            let room = b64 * stride + HEAD16;
            self.room(part::OUT, room + stride)?;
            self.grow(part::OUT, room + stride);
            room
        };
        let te = (b64 + 1) * TABLE;
        self.room(part::TABLE, te)?;
        // A side area given back, else a new one.
        let free = self.len(part::FREE);
        let area = if free == 0 {
            let made = self.len(part::SIDES);
            self.room(part::SIDES, made + SIDE_AREA)?;
            self.set_len(part::SIDES, made + SIDE_AREA);
            (made / SIDE_AREA) as u32
        } else {
            self.set_len(part::FREE, free - 4);
            // SAFETY: as the room's.
            unsafe { ld32(self.part(part::FREE) + free as usize - 4) }
        };
        // SAFETY: each in its part's room, checked above.
        unsafe {
            zero(self.rooms() + room as usize, stride);
            self.grow(part::TABLE, te);
            let e = self.entry(b);
            st32(e, room as u32);
            st32(e + 4, area);
            st32(e + 8, 0);
            let block = self.block(b);
            st64(block.area, 0);
            st64(block.area + 8, 0);
        }
        self.blocks = b + 1;
        if self.flags & 1 != 0 {
            let bytes = u64::from(self.kw + self.fw) * 8;
            let need = (b64 + 2) * bytes;
            self.room(part::PRESENCE, need)?;
            self.grow(part::PRESENCE, need);
            // SAFETY: as above.
            unsafe { zero(self.part(part::PRESENCE) + ((b64 + 1) * bytes) as usize, bytes) };
        }
        Ok(())
    }

    /// Block `b` made final, if it is: every node in it closed, the node after its last opened, the frontier past
    /// it.
    unsafe fn settle(&mut self, b: u32) -> R {
        let end = (b + 1) * Z;
        // SAFETY: block `b` is made.
        let done = unsafe { ld32(self.entry(b) + 8) };
        if done != 0 || self.n <= end || (self.frontier < end && self.flags & FINAL_AS_WRITTEN == 0) {
            return Ok(());
        }
        // The open nodes' indices ascend: the last below the block's end is the one that may be in it.
        let opens = self.part(part::OPEN);
        let node = |k: u32| {
            // SAFETY: an open node's frame.
            unsafe { ld32(opens + k as usize * OPEN as usize) }
        };
        let (mut lo, mut hi) = (0u32, self.open_len);
        while lo < hi {
            let mid = (lo + hi) >> 1;
            if node(mid) < end { lo = mid + 1 } else { hi = mid }
        }
        if lo != 0 && node(lo - 1) >= b * Z {
            return Ok(());
        }
        // SAFETY: as above.
        unsafe { self.finalize(b, Z) }
    }

    /// Block `b`, of `rows` rows, final: its presence taken, its side lists' parts laid, its side area given back;
    /// in 16z its pieces stored.
    unsafe fn finalize(&mut self, b: u32, rows: u32) -> R {
        // SAFETY: block `b` is made; the presence's room is `new_block`'s.
        unsafe {
            let e = self.entry(b);
            st32(e + 8, 1);
            let block = self.block(b);
            let codes = List::of(block.area, 0);
            let (cut, per) = (self.cut, self.per);
            let kinds = self.part(part::KINDS);
            let numbers = self.len(part::KINDS) >> 2;
            let pres = self.part(part::PRESENCE);
            let kw = u64::from(self.kw);
            let keep = self.flags & 1 != 0;
            let bytes = u64::from(self.kw + self.fw) * 8;
            let mine = pres + ((u64::from(b) + 1) * bytes) as usize;
            let set_bit = |base: usize, word0: u64, bit: u32| {
                let at = base + ((u64::from(bit >> 6) + word0) * 8) as usize;
                st64(at, ld64(at) | 1u64 << (bit & 63));
            };
            let mut low = 0u32;
            for j in 0..rows {
                let byte = ld8(block.room + j as usize);
                let mut number = byte;
                if byte >= cut {
                    // The next entry of the codes' side list, where it is this node's.
                    let mut r = 0u32;
                    if low < codes.count {
                        if codes.j(low) == j {
                            r = codes.v(low) as u32;
                        }
                        low += 1;
                    }
                    number = cut + (byte - cut) * per + r;
                }
                let kf = if u64::from(number) < numbers { ld32(kinds + 4 * number as usize) } else { 0 };
                let (kind, field) = (kf & KIND_MASK, kf >> 16);
                // A MISSING code sets the fault bit too (`presence::kind_word`).
                let fault = kf & FAULT != 0;
                set_bit(pres, 0, kind);
                set_bit(pres, kw, field);
                if fault {
                    set_bit(pres, 0, u32::from(ERROR_CODE));
                }
                if keep {
                    set_bit(mine, 0, kind);
                    set_bit(mine, kw, field);
                    if fault {
                        set_bit(mine, 0, u32::from(ERROR_CODE));
                    }
                }
                // An ERROR node or a MISSING one listed in the ERRORS part, beside the fault bit it set.
                if kind == u32::from(ERROR_CODE) || fault {
                    let len = self.len(part::ERRORS);
                    self.room(part::ERRORS, len + 4)?;
                    st32(self.part(part::ERRORS) + len as usize, b * Z + j);
                    self.set_len(part::ERRORS, len + 4);
                }
            }
            for q in 0..4 {
                self.lay(b, q, rows)?;
            }
            // Its side area, given back.
            let fl = self.len(part::FREE);
            self.room(part::FREE, fl + 4)?;
            st32(self.part(part::FREE) + fl as usize, ld32(e + 4));
            self.set_len(part::FREE, fl + 4);
            st32(e + 4, NONE);
            if self.z() {
                self.zblock(b, rows)?;
            }
        }
        Ok(())
    }

    /// Block `b`'s side list `q` as stored, appended to `laid`: a u16 count a sub-block, each entry's offset in its
    /// sub-block, then the values at `vw[q]` bytes.
    unsafe fn lay(&mut self, b: u32, q: usize, rows: u32) -> R {
        // SAFETY: block `b` is made, its side area not yet given back.
        unsafe {
            let list = List::of(self.block(b).area, q);
            let count = list.count;
            let laid = self.part(part::LAID);
            let start = self.len(part::LAID);
            let vw = u64::from(self.vw[q]);
            let nsub = rows.div_ceil(BLOCK);
            let ends = 2 * u64::from(nsub);
            let len = if count != 0 { ends + u64::from(count) + u64::from(count) * vw } else { 0 };
            self.room(part::LAID, start + len)?;
            if count != 0 {
                let at = laid + start as usize;
                let mut k = 0u32;
                for s in 0..nsub {
                    while k < count && list.j(k) / BLOCK == s {
                        k += 1;
                    }
                    st16(at + 2 * s as usize, k);
                }
                let offs = at + ends as usize;
                for m in 0..count {
                    st8(offs + m as usize, list.j(m));
                }
                let values = offs + count as usize;
                for m in 0..count {
                    cell_put(values + (u64::from(m) * vw) as usize, vw, list.v(m));
                }
            }
            self.set_len(part::LAID, start + len);
            let need = (u64::from(b) + 1) * LAID_AT;
            self.room(part::LAID_AT, need)?;
            self.grow(part::LAID_AT, need);
            let e = self.part(part::LAID_AT) + (u64::from(b) * LAID_AT) as usize + 8 * q;
            st32(e, start as u32);
            st32(e + 4, len as u32);
        }
        Ok(())
    }

    /// Fails unless node `i` is written and its block not final.
    unsafe fn changeable(&mut self, i: u32) -> R {
        // SAFETY: node `i`'s block is made where `i` is below the count.
        if i >= self.n || self.flags & FINAL_AS_WRITTEN != 0 || unsafe { ld32(self.entry(i / Z) + 8) } != 0 {
            return self.fail(fail::FINAL, i);
        }
        Ok(())
    }

    // ---- nodes ------------------------------------------------------------------------------------------------

    /// Node `n`, of `number`, opened at `start`: its code, its gap, its checkpoint, the slot of the node before it;
    /// pushed on the open nodes where `push`. Its index, its row and its block.
    unsafe fn open_node(&mut self, number: u32, start: u64, push: bool) -> Result<(u32, u32, Block), u32> {
        // SAFETY: the caller's.
        unsafe { self.open_node_x::<Run>(number, start, push) }
    }

    /// `open_node`, its widths `X`'s.
    #[inline(always)]
    unsafe fn open_node_x<X: CellWidths>(&mut self, number: u32, start: u64, push: bool) -> Result<(u32, u32, Block), u32> {
        let (i, prev) = (self.n, self.prev_start);
        if start < prev || start > self.source_len || i.wrapping_add(1) >= MAX_NODES {
            return self.fail(fail::BOUNDS, i);
        }
        let open_len = self.open_len;
        if i != 0 && open_len == 0 {
            return self.fail(fail::OUTSIDE, i);
        }
        self.room(part::OPEN, (u64::from(open_len) + 1) * OPEN)?;
        let (b, j) = (i / Z, i % Z);
        let first = j == 0;
        // SAFETY: the block is made here where the node is its first; each frame in the OPEN part's room.
        unsafe {
            if first {
                self.new_block(b)?;
            }
            let block = self.block(b);
            // The node before is a leaf unless it is the parent: its slot describes this node.
            if open_len != 0 {
                let t = self.part(part::OPEN) + (open_len as usize - 1) * OPEN as usize;
                let (p, idx, last) = (ld32(t), ld32(t + 4), ld32(t + 8));
                st32(t + 4, idx.wrapping_add(1));
                st32(t + 8, i);
                if i.wrapping_sub(1) != p {
                    let v = if idx.is_multiple_of(K) {
                        u64::from(i.wrapping_sub(p)) << 2 | 3
                    } else {
                        u64::from(i.wrapping_sub(last)) << 2 | 1
                    };
                    if first {
                        self.put_x::<X>(0, i - 1, v)?;
                    } else {
                        self.put_at_x::<X>(0, j - 1, v, block);
                    }
                }
            }
            self.put_code_at(j, number, block);
            let gap = if i == 0 { 0 } else { start.wrapping_sub(prev) };
            self.put_at_x::<X>(1, j, gap, block);
            if j.is_multiple_of(K) {
                self.put_checkpoint_x::<X>(block.room, j, start);
            }
            self.prev_start = start;
            self.n = i + 1;
            if push {
                let t = self.part(part::OPEN) + open_len as usize * OPEN as usize;
                st32(t, i);
                st32(t + 4, 0);
                st32(t + 8, NONE);
                st64(t + 16, start);
                self.open_len = open_len + 1;
            }
            // The block before has its last slot now.
            if first && b != 0 {
                self.settle(b - 1)?;
            }
            Ok((i, j, block))
        }
    }

    unsafe fn close(&mut self, end: u64) -> R {
        // SAFETY: the caller's.
        unsafe { self.close_x::<Run>(end) }
    }

    /// `close`, its widths `X`'s.
    #[inline(always)]
    unsafe fn close_x<X: CellWidths>(&mut self, end: u64) -> R {
        let (open_len, n) = (self.open_len, self.n);
        if open_len == 0 {
            return self.fail(fail::NO_OPEN, n);
        }
        self.open_len = open_len - 1;
        let t = self.part(part::OPEN) + (open_len as usize - 1) * OPEN as usize;
        // SAFETY: the innermost open frame; its node's block is made.
        unsafe {
            let (i, start) = (ld32(t), ld64(t + 16));
            if end < start || end > self.source_len {
                return self.fail(fail::BOUNDS, i);
            }
            let (b, j) = (i / Z, i % Z);
            let block = self.block(b);
            self.put_at_x::<X>(2, j, end - start, block);
            let size = n - i;
            if size > 1 {
                self.put_at_x::<X>(0, j, u64::from(size) << 1, block);
            }
            // The node's block is final only once a node past its end is made.
            if n <= (b + 1) * Z {
                return Ok(());
            }
            self.settle(b)
        }
    }

    unsafe fn leaf(&mut self, number: u32, start: u64, end: u64) -> R {
        // SAFETY: the caller's.
        unsafe { self.leaf_x::<Run>(number, start, end) }
    }

    /// `leaf`, its widths `X`'s.
    #[inline(always)]
    unsafe fn leaf_x<X: CellWidths>(&mut self, number: u32, start: u64, end: u64) -> R {
        // SAFETY: the open's block, made by it.
        unsafe {
            let (i, j, block) = self.open_node_x::<X>(number, start, false)?;
            if end < start || end > self.source_len {
                return self.fail(fail::BOUNDS, i);
            }
            self.put_at_x::<X>(2, j, end - start, block);
        }
        Ok(())
    }

    /// A node of one leaf child, both at once (`walk`'s `string` with no escape: the string and its content): the
    /// node from `start` to `end`, a leaf of its parent with no frame of its own, its size and length written as it
    /// is; its child, the node after it, from `cs` to `ce`. The same slots as `open`, `leaf`, `close`.
    #[inline(always)]
    unsafe fn leaf_in_x<X: CellWidths>(&mut self, number: u32, child: u32, start: u64, cs: u64, ce: u64, end: u64) -> R {
        // SAFETY: the open's block, made by it; the child's, made here where it is its first.
        unsafe {
            let (i, j, block) = self.open_node_x::<X>(number, start, false)?;
            if cs < start || ce < cs || end < ce || end > self.source_len || i.wrapping_add(2) >= MAX_NODES {
                return self.fail(fail::BOUNDS, i);
            }
            self.put_at_x::<X>(2, j, end - start, block);
            self.put_at_x::<X>(0, j, 2 << 1, block);
            // The child: the node's first, so no slot of the node before describes it.
            let c = i + 1;
            let (cb, cj) = (c / Z, c % Z);
            if cj == 0 {
                self.new_block(cb)?;
            }
            let cblock = self.block(cb);
            self.put_code_at(cj, child, cblock);
            self.put_at_x::<X>(1, cj, cs - start, cblock);
            if cj.is_multiple_of(K) {
                self.put_checkpoint_x::<X>(cblock.room, cj, cs);
            }
            self.put_at_x::<X>(2, cj, ce - cs, cblock);
            self.prev_start = cs;
            self.n = c + 1;
            // The block before has its last slot now.
            if cj == 0 && cb != 0 {
                self.settle(cb - 1)?;
            }
        }
        Ok(())
    }

    unsafe fn commit(&mut self) -> R {
        let was = self.frontier / Z;
        self.frontier = self.n;
        for b in was..self.n / Z {
            // SAFETY: each block below the count is made.
            unsafe { self.settle(b)? };
        }
        Ok(())
    }

    unsafe fn retag(&mut self, i: u32, number: u32) -> R {
        // SAFETY: node `i` is written, its block not final.
        unsafe {
            self.changeable(i)?;
            let block = self.block(i / Z);
            let j = i % Z;
            let codes = List::of(block.area, 0);
            let k = codes.find(j);
            if codes.is(k, j) {
                codes.remove(k);
            }
            self.put_code_at(j, number, block);
        }
        Ok(())
    }

    unsafe fn set_start(&mut self, i: u32, start: u64) -> R {
        let opens = self.part(part::OPEN);
        // The innermost open frame of node `i`.
        // SAFETY: the open frames.
        let found = (0..self.open_len).rev().find(|&k| unsafe { ld32(opens + k as usize * OPEN as usize) } == i);
        let Some(fk) = found else { return self.fail(fail::NOT_OPEN, i) };
        // SAFETY: node `i` (and the next, where there is one) written, its block not final.
        unsafe {
            self.changeable(i)?;
            let has_next = i + 1 < self.n;
            if has_next {
                self.changeable(i + 1)?;
            }
            let t = opens + fk as usize * OPEN as usize;
            let old = ld64(t + 16);
            let prev = if i != 0 { old.wrapping_sub(self.value(1, i)) } else { 0 };
            let next = if has_next { old.wrapping_add(self.value(1, i + 1)) } else { self.source_len };
            if start < prev || start > next {
                return self.fail(fail::BOUNDS, i);
            }
            if i != 0 {
                self.put(1, i, start - prev)?;
            }
            if has_next {
                self.put(1, i + 1, next - start)?;
            } else {
                self.prev_start = start;
            }
            let j = i % Z;
            if j.is_multiple_of(K) {
                let room = self.block(i / Z).room;
                self.put_checkpoint(room, j, start);
            }
            st64(t + 16, start);
        }
        Ok(())
    }

    // ---- the document's end -----------------------------------------------------------------------------------

    /// `finish`'s first half: the last node's slot, every block final. A host reserves the tail's room after it.
    unsafe fn finish_blocks(&mut self) -> R {
        if self.bad != 0 {
            return Err(self.bad);
        }
        let n = self.n;
        if self.open_len != 0 {
            return self.fail(fail::UNCLOSED, n);
        }
        // SAFETY: each block below the count is made.
        unsafe {
            // The last node in preorder is a leaf with nothing after it.
            if n != 0 {
                self.put(0, n - 1, 1)?;
            }
            for b in 0..n.div_ceil(Z) {
                if ld32(self.entry(b) + 8) == 0 {
                    self.finalize(b, (n - b * Z).min(Z))?;
                }
            }
        }
        Ok(())
    }

    /// `finish`'s second half: 16z's tail (`zfinish`), or 16's: the last block's columns moved down to its rows,
    /// then the side lists' parts in block order, their starts, the newline index, the header.
    unsafe fn finish_tail(&mut self) -> R {
        if self.bad != 0 {
            return Err(self.bad);
        }
        let n = self.n;
        let count = n.div_ceil(Z);
        if self.z() {
            // SAFETY: the caller's.
            return unsafe { self.zfinish(count) };
        }
        let out = self.part(part::OUT);
        let w = self.w.map(u64::from);
        let cw = u64::from(self.cw);
        let befores = [1, 1 + w[0], 1 + w[0] + w[1], 1 + w[0] + w[1] + w[2]];
        let mut blocks_end = HEAD16;
        // SAFETY: the tape's room, reserved for the tail (`f16::Writer::finish_ops`) and checked below.
        unsafe {
            if count != 0 {
                // The last block's columns down to its rows.
                let lb = count - 1;
                let rows = u64::from(n - lb * Z);
                let room = self.block(lb).room;
                for r in 1..5 {
                    let len = if r == 4 { rows.div_ceil(u64::from(K)) * cw } else { rows * w[r - 1] };
                    let to = room + (rows * befores[r - 1]) as usize;
                    copy(to, room + self.at[r] as usize, len);
                    if r == 4 {
                        blocks_end = (to + len as usize - out) as u64;
                    }
                }
            }
            // The tail: each side list's parts in block order, their starts, the newline index.
            let lines_len = self.len(part::LINES);
            let need = blocks_end + self.len(part::LAID) + (u64::from(count) + 1) * 16 + lines_len + 8 * 6 + 8;
            self.room(part::OUT, need)?;
            st64(out + blocks_end as usize, 0);
            let mut pos = blocks_end + 8;
            let (laid, la) = (self.part(part::LAID), self.part(part::LAID_AT));
            let mut tail = [0u64; 10];
            for (q, t) in tail[..4].iter_mut().enumerate() {
                *t = pos;
                for b in 0..count as usize {
                    let e = la + b * LAID_AT as usize + 8 * q;
                    let (st, ln) = (ld32(e), u64::from(ld32(e + 4)));
                    copy(out + pos as usize, laid + st as usize, ln);
                    pos += ln;
                }
                st64(out + pos as usize, 0);
                pos += 8;
            }
            // The padding to the part starts' alignment is zeros too (the bytes there were the last block's).
            st64(out + pos as usize, 0);
            pos = (pos + 3) & !3;
            for (q, t) in tail[4..8].iter_mut().enumerate() {
                *t = pos;
                pos = starts(out, pos, la, q, count);
            }
            let lines_at = pos;
            copy(out + lines_at as usize, self.part(part::LINES), lines_len);
            st64(out + (lines_at + lines_len) as usize, 0);
            let end = lines_at + lines_len + 8;
            self.set_len(part::OUT, end);
            tail[8] = lines_at;
            tail[9] = end;
            self.write_head(out, MAGIC16, &tail);
        }
        Ok(())
    }

    /// The header at `out`: `magic`, the document's constants, and the tail's offsets.
    unsafe fn write_head(&self, out: usize, magic: u64, tail: &[u64; 10]) {
        // SAFETY: the header's bytes, the tape's first.
        unsafe {
            zero(out, HEAD16);
            st64(out, magic);
            st8(out + 8, self.tag);
            st8(out + 9, self.mark);
            st8(out + 10, self.cw);
            st32(out + 12, self.newlines);
            st64(out + 16, self.source_len);
            st64(out + 24, u64::from(self.n));
            for k in 0..4 {
                st8(out + 32 + k, self.vw[k]);
            }
            for k in 0..3 {
                st8(out + 36 + k, self.w[k]);
            }
            st32(out + 40, self.stride);
            st32(out + 44, HEAD16 as u32);
            for (t, &v) in tail.iter().enumerate() {
                st32(out + 48 + 4 * t, v as u32);
            }
        }
    }

    // ---- 16z --------------------------------------------------------------------------------------------------

    /// 16z's block `b`, final: its four pieces (each column at its rows, then its side list's part as laid) stored
    /// in turn at the payload's end (`zpiece`), its checkpoints kept, its room given back.
    unsafe fn zblock(&mut self, b: u32, rows: u32) -> R {
        let rows64 = u64::from(rows);
        // SAFETY: block `b` is made and laid; each copy in its part's room, checked first.
        unsafe {
            let room = self.block(b).room;
            let laid = self.part(part::LAID);
            let la = self.part(part::LAID_AT) + (u64::from(b) * LAID_AT) as usize;
            for q in 0..4u32 {
                // A column's cell: a byte (codes), else its array's width.
                let cell = if q == 0 { 1 } else { u64::from(self.w[q as usize - 1]) };
                let clen = rows64 * cell;
                let e = la + 8 * q as usize;
                let (lst, llen) = (ld32(e), u64::from(ld32(e + 4)));
                let raw = clen + llen;
                let pos = self.len(part::OUT);
                self.room(part::OUT, pos + raw + raw + raw / 255 + 16)?;
                let j = (b << 2) + q;
                let pneed = (u64::from(j) + 1) * PIECE;
                self.room(part::PIECES, pneed)?;
                self.grow(part::PIECES, pneed);
                let src = self.part(part::OUT) + pos as usize;
                copy(src, room + self.at[q as usize] as usize, clen);
                copy(src + clen as usize, laid + lst as usize, llen);
                self.zpiece(j, pos, raw)?;
            }
            // Its checkpoints, at their place among the blocks'.
            let cw = u64::from(self.cw);
            let c0 = u64::from(b) * u64::from(Z / K) * cw;
            let clen = rows64.div_ceil(u64::from(K)) * cw;
            self.room(part::CKPT, c0 + clen)?;
            copy(self.part(part::CKPT) + c0 as usize, room + self.at[4] as usize, clen);
            self.grow(part::CKPT, c0 + clen);
            // The laid parts are in its pieces; its room serves the next block.
            self.set_len(part::LAID, 0);
            let fl = self.len(part::ROOM_FREE);
            self.room(part::ROOM_FREE, fl + 4)?;
            st32(self.part(part::ROOM_FREE) + fl as usize, ld32(self.entry(b)));
            self.set_len(part::ROOM_FREE, fl + 4);
        }
        Ok(())
    }

    /// Piece `j`, its `raw` bytes at the payload's end (`pos` in the tape), stored as 16z stores it: a repeat of an
    /// earlier piece of the same bytes (found by their hash, then compared: a raw piece's bytes with these, an LZ4
    /// one's with these compressed), else one LZ4 block where that saves a tenth, else as they are.
    unsafe fn zpiece(&mut self, j: u32, pos: u64, raw: u64) -> R {
        let out = self.part(part::OUT);
        let src = out + pos as usize;
        let dst = src + raw as usize;
        let pieces = self.part(part::PIECES);
        let pe = pieces + (u64::from(j) * PIECE) as usize;
        let start = (pos - HEAD16) as u32;
        let mut compressed: Option<u64> = None;
        let mut h = 0u64;
        let mut next = NONE;
        // SAFETY: the piece, its room for LZ4's bytes after it, and the piece index, reserved by `zblock`.
        unsafe {
            let mut compress = |s: &mut State| *compressed.get_or_insert_with(|| s.lz4(src, raw, dst));
            let entry = |s: &mut State, len: u64, kind: u32, next: u32, h: u64| {
                st32(pe, start);
                st32(pe + 4, len as u32);
                st32(pe + 8, raw as u32);
                st32(pe + 12, kind);
                st32(pe + 16, next);
                st64(pe + 24, h);
                s.set_len(part::OUT, pos + len);
            };
            if raw >= REPEAT_MIN {
                h = hash(src, raw);
                let slot = self.part(part::SEEN) + 4 * ((h as u32) & (self.zbuckets - 1)) as usize;
                let head = ld32(slot);
                let mut e = head;
                while e != NONE {
                    let ee = pieces + (u64::from(e) * PIECE) as usize;
                    if ld64(ee + 24) == h && ld32(ee + 8) == raw as u32 {
                        let stored = out + HEAD16 as usize + ld32(ee) as usize;
                        let same = if ld32(ee + 12) == RAW {
                            eq(stored, src, raw)
                        } else {
                            let z = compress(self);
                            z == u64::from(ld32(ee + 4)) && eq(stored, dst, z)
                        };
                        if same {
                            st32(src, e);
                            entry(self, 4, REPEAT, NONE, h);
                            return Ok(());
                        }
                    }
                    e = ld32(ee + 16);
                }
                // The first piece of these bytes: into its bucket.
                next = head;
                st32(slot, j);
            }
            let (mut kind, mut len) = (RAW, raw);
            if raw >= LZ4_MIN {
                let z = compress(self);
                if z * 10 <= raw * 9 {
                    copy(src, dst, z);
                    (kind, len) = (LZ4, z);
                }
            }
            entry(self, len, kind, next, h);
        }
        Ok(())
    }

    /// 16z's tail after its payload: each side list's part starts by block, the checkpoints, the newline index, the
    /// piece index (starts, lengths, kinds); then the header, which says the tape it decodes to.
    unsafe fn zfinish(&mut self, count: u32) -> R {
        let out = self.part(part::OUT);
        let pos0 = self.len(part::OUT);
        let la = self.part(part::LAID_AT);
        let (clen, llen) = (self.len(part::CKPT), self.len(part::LINES));
        let pieces = self.part(part::PIECES);
        let c64 = u64::from(count);
        let np = c64 * 4;
        self.room(part::OUT, pos0 + (c64 + 1) * 16 + clen + llen + np * 9 + 3 + 8 + 8 + 3 + 8)?;
        // SAFETY: the tape's room, checked above.
        unsafe {
            st64(out + pos0 as usize, 0);
            let parts_at = (pos0 + 3) & !3;
            let mut pos = parts_at;
            let mut totals = [0u64; 4];
            for (q, total) in totals.iter_mut().enumerate() {
                pos = starts(out, pos, la, q, count);
                // The last start is the list's parts' total.
                *total = u64::from(ld32(out + pos as usize - 4));
            }
            let ckpt_at = pos;
            copy(out + ckpt_at as usize, self.part(part::CKPT), clen);
            st64(out + (ckpt_at + clen) as usize, 0);
            let lines_at = ckpt_at + clen + 8;
            copy(out + lines_at as usize, self.part(part::LINES), llen);
            let e = lines_at + llen;
            st64(out + e as usize, 0);
            st64(out + e as usize + 8, 0);
            let starts_at = (e + 8 + 3) & !3;
            let lens = starts_at + np * 4;
            let kinds = lens + np * 4;
            let end = kinds + np;
            for k in 0..np as usize {
                let pe = pieces + k * PIECE as usize;
                st32(out + starts_at as usize + 4 * k, ld32(pe));
                st32(out + lens as usize + 4 * k, ld32(pe + 4));
                st8(out + kinds as usize + k, ld32(pe + 12));
            }
            self.set_len(part::OUT, end);
            // The tape it decodes to: its blocks' end, then its tail as format 16 places it.
            let mut be = HEAD16;
            if count != 0 {
                let lb = c64 - 1;
                let rows = u64::from(self.n) - lb * u64::from(Z);
                let w = self.w.map(u64::from);
                be = lb * u64::from(self.stride) + HEAD16 + rows * (w[0] + w[1] + w[2] + 1) + rows.div_ceil(u64::from(K)) * u64::from(self.cw);
            }
            let mut tail = [0u64; 10];
            let mut at = be + 8;
            for (q, t) in totals.iter().enumerate() {
                tail[q] = at;
                at += t + 8;
            }
            at = (at + 3) & !3;
            for t in &mut tail[4..8] {
                *t = at;
                at += (c64 + 1) * 4;
            }
            tail[8] = at;
            tail[9] = at + llen + 8;
            self.write_head(out, MAGIC16Z, &tail);
            for (t, v) in [parts_at, ckpt_at, lines_at, starts_at, lens, kinds, end].into_iter().enumerate() {
                st32(out + 88 + 4 * t, v as u32);
            }
        }
        Ok(())
    }
}

/// Side list `q`'s part starts, a u32 a block and one past the last, at `pos` in the tape at `out`, from the laid
/// parts' lengths at `la`; where they end.
unsafe fn starts(out: usize, mut pos: u64, la: usize, q: usize, count: u32) -> u64 {
    let mut acc = 0u32;
    for b in 0..=count as usize {
        // SAFETY: the caller's: the tape's room, and each block's laid entry.
        unsafe {
            st32(out + pos as usize, acc);
            if b < count as usize {
                acc = acc.wrapping_add(ld32(la + b * LAID_AT as usize + 8 * q + 4));
            }
        }
        pos += 4;
    }
    pos
}

/// A 64-bit hash of `n` bytes, a word at a time (the seen pieces').
unsafe fn hash(src: usize, n: u64) -> u64 {
    let mut h = n;
    let mut mix = |x: u64| {
        let x = (h ^ x).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        h = x ^ (x >> 29);
    };
    let mut k = 0u64;
    // SAFETY: the caller's: the bytes, and the word past them, are readable.
    unsafe {
        while k + 8 <= n {
            mix(ld64(src + k as usize));
            k += 8;
        }
        if n != k {
            mix(cell_get(src + k as usize, n - k));
        }
    }
    h
}

/// Whether `n` bytes at `a` and at `b` are the same, a word at a time.
unsafe fn eq(a: usize, b: usize, n: u64) -> bool {
    let mut k = 0u64;
    // SAFETY: as `hash`'s.
    unsafe {
        while k + 8 <= n {
            if ld64(a + k as usize) != ld64(b + k as usize) {
                return false;
            }
            k += 8;
        }
        cell_get(a + k as usize, n - k) == cell_get(b + k as usize, n - k)
    }
}

// ---- LZ4 HC, level 4 (decision 0434 §6) ---------------------------------------------------------------------------
//
// liblz4 1.10.0's `LZ4HC_compress_hashChain` with 8 attempts, no dictionary, no pattern analysis and no chain swap,
// as `LZ4_compress_HC(.., 4)` runs it: the same sequences, so the same bytes. Positions are offsets from the piece's
// first byte; an index is a position plus the piece's base (`zbase`).

const MINMATCH: i64 = 4;
const MFLIMIT: i64 = 12;
const LASTLITERALS: i64 = 5;
const OPTIMAL_ML: i64 = 18;
const DISTANCE_MAX: u64 = 65535;
const CHAIN_AT: usize = 4 << LZ_HASH_LOG;

/// The piece's byte address of position `p`.
#[inline(always)]
fn at(src: usize, p: i64) -> usize {
    src.wrapping_add(p as usize)
}

/// LZ4 HC's hash of a 4-byte word: its hash table's byte offset.
#[inline(always)]
fn lz_hash(x: u32) -> usize {
    ((x.wrapping_mul(2_654_435_761) >> (32 - LZ_HASH_LOG)) as usize) << 2
}

/// A match: its length and offset.
#[derive(Clone, Copy, Default)]
struct Match {
    len: i64,
    off: u64,
}

/// The output of a piece's compression: where it is written, and the sequences' cursor and anchor.
struct Lz {
    src: usize,
    ip: i64,
    anchor: i64,
    op: usize,
}

impl Lz {
    /// LZ4's length past 15: `len / 255` bytes of 255, then `len % 255`.
    unsafe fn length(&mut self, len: i64) {
        let (k, rest) = (len as u64 / 255, len as u64 % 255);
        // SAFETY: the output's room (`lz4_bound`).
        unsafe {
            fill(self.op, k, u64::MAX);
            st8(self.op + k as usize, rest as u32);
        }
        self.op += k as usize + 1;
    }

    /// `LZ4HC_encodeSequence`: the literals from the anchor to the cursor, then `m`; the cursor past it, the anchor
    /// at it.
    unsafe fn encode(&mut self, m: Match) {
        let len = self.ip - self.anchor;
        let token = self.op;
        self.op += 1;
        // SAFETY: as `length`'s.
        unsafe {
            let mut tok = len;
            if len >= 15 {
                self.length(len - 15);
                tok = 15;
            }
            tok <<= 4;
            copy(self.op, at(self.src, self.anchor), len as u64);
            self.op += len as usize;
            st16(self.op, m.off as u32);
            self.op += 2;
            let l = m.len - MINMATCH;
            if l >= 15 {
                self.length(l - 15);
                tok += 15;
            } else {
                tok += l;
            }
            st8(token, tok as u32);
        }
        self.ip += m.len;
        self.anchor = self.ip;
    }
}

impl State {
    /// `LZ4HC_InsertAndGetWiderMatch` at level 4 for the piece at `src`: the positions before `ip` inserted, then up
    /// to 8 entries of `ip`'s chain tried for a match longer than `longest`, reaching back to `low` and on to
    /// `high`. Its match (`longest` long if none) and back count.
    unsafe fn lz_find(&mut self, src: usize, ip: i64, low: i64, high: i64, longest: i64) -> (Match, i64) {
        let lz = self.part(part::LZ);
        let chain = lz + CHAIN_AT;
        let g = u64::from(self.zbase);
        let ip_index = g.wrapping_add(ip as u64);
        // SAFETY: the compressor's tables, and the piece's bytes (3 before it, 8 after readable).
        unsafe {
            // The positions before `ip` in the hash table and the chain (`LZ4HC_Insert`).
            let mut v = u64::from(self.znext);
            while v < ip_index {
                let slot = lz + lz_hash(ld32(at(src, v.wrapping_sub(g) as i64)));
                let delta = v.wrapping_sub(u64::from(ld32(slot))).min(DISTANCE_MAX);
                st16(chain + (((v & 0xffff) as usize) << 1), delta as u32);
                st32(slot, v as u32);
                v += 1;
            }
            self.znext = ip_index as u32;
            let lowest = if g + DISTANCE_MAX + 1 > ip_index { g } else { ip_index - DISTANCE_MAX };
            let look_back = ip - low;
            let pattern = ld32(at(src, ip));
            let mut m = u64::from(ld32(lz + lz_hash(pattern)));
            let mut tries = LZ_ATTEMPTS;
            let (mut best, mut back) = (Match { len: longest, off: 0 }, 0i64);
            while m >= lowest && tries > 0 {
                tries -= 1;
                let mp = m.wrapping_sub(g) as i64;
                let a = ld16(at(src, low + best.len - 1));
                let b = ld16(at(src, mp - look_back + best.len - 1));
                if a == b && ld32(at(src, mp)) == pattern {
                    let bk = lz_count_back(src, ip, mp, low);
                    let ml = lz_count(src, ip + MINMATCH, mp + MINMATCH, high) + MINMATCH - bk;
                    if ml > best.len {
                        best = Match { len: ml, off: ip_index.wrapping_sub(m) };
                        back = bk;
                    }
                }
                m = m.wrapping_sub(u64::from(ld16(chain + (((m & 0xffff) as usize) << 1))));
            }
            self.zoff = best.off as u32;
            self.zback = back as u32;
            (best, back)
        }
    }

    /// The `n` bytes at `src` as one LZ4 block at `dst` (`LZ4_compress_HC(.., 4)`'s bytes); its length. `dst` has
    /// room for `lz4_bound(n)` bytes and the 8 past a cell; `src`'s 3 bytes before it and 8 after are readable.
    unsafe fn lz4(&mut self, src: usize, n: u64, dst: usize) -> u64 {
        // `LZ4HC_init_internal`, the context reused: this piece's indices start 64 KB past the last one's end.
        let mut start = u64::from(self.zbase) + u64::from(self.zprev);
        if start > 1 << 30 {
            // SAFETY: the compressor's hash table.
            unsafe { zero(self.part(part::LZ), 4 << LZ_HASH_LOG) };
            start = 0;
        }
        let g = (start + 65536) as u32;
        self.zbase = g;
        self.znext = g;
        self.zprev = n as u32;
        let n = n as i64;
        let mut z = Lz { src, ip: 0, anchor: 0, op: dst };
        let (mflimit, matchlimit) = (n - MFLIMIT, n - LASTLITERALS);
        // SAFETY: as `lz_find`'s, and the output's room.
        unsafe {
            if n > MFLIMIT {
                'main: while z.ip <= mflimit {
                    let (m, _) = self.lz_find(src, z.ip, z.ip, matchlimit, MINMATCH - 1);
                    if m.len < MINMATCH {
                        z.ip += 1;
                        continue;
                    }
                    let (mut start0, mut m0, mut m1) = (z.ip, m, m);
                    let (mut start2, mut m2, mut start3, mut m3);
                    'search2: loop {
                        if z.ip + m1.len <= mflimit {
                            let s2 = z.ip + m1.len - 2;
                            let (m, b) = self.lz_find(src, s2, z.ip, matchlimit, m1.len);
                            (start2, m2) = (s2 + b, m);
                        } else {
                            (start2, m2) = (0, Match::default());
                        }
                        if m2.len <= m1.len {
                            z.encode(m1);
                            continue 'main;
                        }
                        if start0 < z.ip && start2 < z.ip + m0.len {
                            z.ip = start0;
                            m1 = m0;
                        }
                        if start2 - z.ip < 3 {
                            z.ip = start2;
                            m1 = m2;
                            continue 'search2;
                        }
                        'search3: loop {
                            if start2 - z.ip < OPTIMAL_ML {
                                let gap = start2 - z.ip;
                                let mut new_ml = m1.len.min(OPTIMAL_ML);
                                if z.ip + new_ml > start2 + m2.len - MINMATCH {
                                    new_ml = gap + m2.len - MINMATCH;
                                }
                                let correction = new_ml - gap;
                                if correction > 0 {
                                    start2 += correction;
                                    m2.len -= correction;
                                }
                            }
                            if start2 + m2.len <= mflimit {
                                let s3 = start2 + m2.len - 3;
                                let (m, b) = self.lz_find(src, s3, start2, matchlimit, m2.len);
                                (start3, m3) = (s3 + b, m);
                            } else {
                                (start3, m3) = (0, Match::default());
                            }
                            if m3.len <= m2.len {
                                // No better third: the first two encoded.
                                if start2 < z.ip + m1.len {
                                    m1.len = start2 - z.ip;
                                }
                                z.encode(m1);
                                z.ip = start2;
                                z.encode(m2);
                                continue 'main;
                            }
                            let e1 = z.ip + m1.len;
                            if start3 < e1 + 3 {
                                if start3 >= e1 {
                                    // The first written now; the second dropped, the third becomes the first.
                                    if start2 < e1 {
                                        let correction = e1 - start2;
                                        start2 += correction;
                                        m2.len -= correction;
                                        if m2.len < MINMATCH {
                                            start2 = start3;
                                            m2 = m3;
                                        }
                                    }
                                    z.encode(m1);
                                    z.ip = start3;
                                    m1 = m3;
                                    start0 = start2;
                                    m0 = m2;
                                    continue 'search2;
                                }
                                start2 = start3;
                                m2 = m3;
                                continue 'search3;
                            }
                            // Three ascending matches: the first written, the second becomes the first, the third
                            // the second.
                            if start2 < e1 {
                                let gap = start2 - z.ip;
                                if gap < OPTIMAL_ML {
                                    m1.len = m1.len.min(OPTIMAL_ML);
                                    if z.ip + m1.len > start2 + m2.len - MINMATCH {
                                        m1.len = gap + m2.len - MINMATCH;
                                    }
                                    let correction = m1.len - gap;
                                    if correction > 0 {
                                        start2 += correction;
                                        m2.len -= correction;
                                    }
                                } else {
                                    m1.len = gap;
                                }
                            }
                            z.encode(m1);
                            z.ip = start2;
                            m1 = m2;
                            start2 = start3;
                            m2 = m3;
                        }
                    }
                }
            }
            // The last literals.
            let run = n - z.anchor;
            let token = z.op;
            z.op += 1;
            if run >= 15 {
                st8(token, 0xf0);
                z.length(run - 15);
            } else {
                st8(token, (run << 4) as u32);
            }
            copy(z.op, at(src, z.anchor), run as u64);
            (z.op + run as usize - dst) as u64
        }
    }
}

/// `LZ4HC_countBack`: minus the bytes before `ip` and `mp` that are the same, down to `low` and to the piece's first
/// byte; a word at a time, the last word's top bytes by its leading zeros.
unsafe fn lz_count_back(src: usize, ip: i64, mp: i64, low: i64) -> i64 {
    let min = (low - ip).max(-mp);
    let mut back = 0i64;
    // SAFETY: the piece's bytes, 3 before it readable.
    unsafe {
        while back - min > 3 {
            let v = ld32(at(src, ip + back - 4)) ^ ld32(at(src, mp + back - 4));
            if v != 0 {
                return back - i64::from(v.leading_zeros() >> 3);
            }
            back -= 4;
        }
        let rest = back - min;
        if rest > 0 {
            let v = ld32(at(src, ip + back - 4)) ^ ld32(at(src, mp + back - 4));
            back -= i64::from(v.leading_zeros() >> 3).min(rest);
        }
    }
    back
}

/// `LZ4_count`: the bytes from `p` and `m` that are the same, up to `limit`, a word at a time.
unsafe fn lz_count(src: usize, p: i64, m: i64, limit: i64) -> i64 {
    let mut k = 0i64;
    // SAFETY: the piece's bytes, 8 after it readable.
    unsafe {
        while p + k + 8 <= limit {
            let v = ld64(at(src, p + k)) ^ ld64(at(src, m + k));
            if v != 0 {
                return k + i64::from(v.trailing_zeros() >> 3);
            }
            k += 8;
        }
        let rest = (limit - (p + k)).max(0);
        let v = cell_get(at(src, p + k), rest as u64) ^ cell_get(at(src, m + k), rest as u64);
        k + i64::from(v.trailing_zeros() >> 3).min(rest)
    }
}

// ---- the ops, as the JIT's and the device's are called ------------------------------------------------------------

/// The state at `desc`.
///
/// # Safety
/// `desc` is a writer's state, each part at its offset from it with its room.
#[inline(always)]
unsafe fn state<'s>(desc: *const u8) -> &'s mut State {
    // SAFETY: the caller's.
    unsafe { &mut *(desc as *mut State) }
}

/// A node at the end, in preorder, open.
///
/// # Safety
/// `desc` is a writer's state (`state`).
#[inline(always)]
pub unsafe fn w_open(desc: *const u8, number: u32, start: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).open_node(number, start, true).map(|_| ())) }
}

/// The innermost open node's end: its length, and an inner node's slot.
///
/// # Safety
/// As `w_open`'s.
#[inline(always)]
pub unsafe fn w_close(desc: *const u8, end: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).close(end)) }
}

/// A closed node with no children: `open` then `close`, without push, pop or settle.
///
/// # Safety
/// As `w_open`'s.
#[inline(always)]
pub unsafe fn w_leaf(desc: *const u8, number: u32, start: u64, end: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).leaf(number, start, end)) }
}

/// A walk's hold on the writer, as the lean parser's `Walk` holds its tape's columns: the block it writes (its room
/// and side area), the next node, the start before, and the open nodes' frames, kept by the walk as its own. An open,
/// a leaf and a close store the node's cells through them, at the widths `X` (`Fixed`: a store of the cell's width at
/// a constant offset), with no test of their bounds: as `Walk`'s, the caller's are in order (`begin`). The state is
/// reached only to begin a block (and settle the one before) and to close a node of a block before: then its own op
/// writes the node, the cursor's fields given to it and taken back. The bytes are the state's ops'.
pub struct Cursor<X: CellWidths> {
    s: *mut State,
    room: usize,
    area: usize,
    n: u32,
    prev_start: u64,
    source_len: u64,
    opens: usize,
    open_len: u32,
    x: core::marker::PhantomData<X>,
}

impl<X: CellWidths> Cursor<X> {
    /// The writer's state `desc`, held.
    ///
    /// # Safety
    /// `desc` is a writer's state, its room reserved for the walk (`Writer::reserve_run`: each node, each open frame),
    /// of widths `X`; nothing else writes it while the cursor is held. The walk's nodes are in order, as a parser's
    /// are: each start at or past the one before, each end at or past its node's start, every one within the source,
    /// and a node is open for each node but the first (a debug build checks each).
    #[inline(always)]
    pub unsafe fn begin(desc: *mut u8) -> Self {
        // SAFETY: the caller's.
        let s: *mut State = unsafe { state(desc) };
        // SAFETY: as above; the block of the node before is made.
        unsafe {
            let st = &*s;
            let (room, area) = if st.n == 0 {
                (0, 0)
            } else {
                let b = st.block((st.n - 1) / Z);
                (b.room, b.area)
            };
            Cursor { s, room, area, n: st.n, prev_start: st.prev_start, source_len: st.source_len, opens: st.part(part::OPEN), open_len: st.open_len, x: core::marker::PhantomData }
        }
    }

    /// The state, its fields the cursor's.
    #[inline(always)]
    fn give(&mut self) -> &mut State {
        // SAFETY: `begin`'s: the state is the cursor's while held.
        let st = unsafe { &mut *self.s };
        st.n = self.n;
        st.prev_start = self.prev_start;
        st.open_len = self.open_len;
        st
    }

    /// The fields back from the state, after its op: the block of the node before's.
    #[inline(always)]
    fn take(&mut self) {
        // SAFETY: as `give`'s; a node is written, its block made.
        unsafe {
            let st = &*self.s;
            self.n = st.n;
            self.prev_start = st.prev_start;
            self.open_len = st.open_len;
            if st.n != 0 && st.bad == 0 {
                let b = st.block((st.n - 1) / Z);
                (self.room, self.area) = (b.room, b.area);
            }
        }
    }

    #[inline(always)]
    fn block(&self) -> Block {
        Block { room: self.room, area: self.area }
    }

    /// Row `j`'s value `v` in array `a` of the held block.
    #[inline(always)]
    unsafe fn put(&mut self, a: usize, j: u32, v: u64) {
        let block = self.block();
        // SAFETY: the held block is made, `j` a row of it.
        unsafe { (*self.s).put_at_x::<X>(a, j, v, block) }
    }

    /// Block `n / Z` made and held, its first node next (`walk`'s `base[n / STRIDE]` at a stride's first node): format
    /// 16's room, table entry, side area and presence (`new_block`).
    #[inline(always)]
    unsafe fn start_block(&mut self) {
        let b = self.n / Z;
        // SAFETY: `begin`'s: the parts' room reserved; `new_block` reads none of the cursor's fields.
        unsafe {
            let st = &mut *self.s;
            let r = st.new_block(b);
            debug_assert!(r.is_ok(), "the writer's block, its room reserved");
            let block = st.block(b);
            (self.room, self.area) = (block.room, block.area);
        }
    }

    /// The block before the held one final where it is (`settle`), once the held block's first node is written.
    #[inline(always)]
    unsafe fn settle_before(&mut self) {
        let b = (self.n - 1) / Z;
        // SAFETY: `begin`'s; the state's fields the cursor's for it.
        unsafe {
            self.give();
            let r = (*self.s).settle(b - 1);
            debug_assert!(r.is_ok(), "the writer's settle, its room reserved");
        }
    }

    /// Node `n` of `number` at `start`, in the held block, its first where `FIRST` (its block made by `start_block`):
    /// the slot of the node before, its code, gap and checkpoint; its frame pushed where `push`. Its row.
    #[inline(always)]
    unsafe fn node<const FIRST: bool>(&mut self, number: u32, start: u64, push: bool) -> u32 {
        let i = self.n;
        let j = i % Z;
        debug_assert!(start >= self.prev_start && start <= self.source_len, "the walk's starts in order, in the source");
        debug_assert!((j == 0) == FIRST && i != 0 && self.open_len != 0 && i + 1 < MAX_NODES, "the cursor's node is in its block, inside the root");
        // SAFETY: the caller's: the held block is node `i`'s; each frame in the OPEN part's reserved room.
        unsafe {
            if self.open_len != 0 {
                let t = self.opens + (self.open_len as usize - 1) * OPEN as usize;
                let (p, idx, last) = (ld32(t), ld32(t + 4), ld32(t + 8));
                st32(t + 4, idx.wrapping_add(1));
                st32(t + 8, i);
                if i.wrapping_sub(1) != p {
                    let v = if idx.is_multiple_of(K) { u64::from(i.wrapping_sub(p)) << 2 | 3 } else { u64::from(i.wrapping_sub(last)) << 2 | 1 };
                    if FIRST {
                        // The node before is the block before's last.
                        let r = (*self.s).put_x::<X>(0, i - 1, v);
                        debug_assert!(r.is_ok());
                    } else {
                        self.put(0, j - 1, v);
                    }
                }
            }
            let block = self.block();
            (*self.s).put_code_at(j, number, block);
            self.put(1, j, start - self.prev_start);
            if j.is_multiple_of(K) {
                (*self.s).put_checkpoint_x::<X>(self.room, j, start);
            }
            self.prev_start = start;
            self.n = i + 1;
            if push {
                let t = self.opens + self.open_len as usize * OPEN as usize;
                st32(t, i);
                st32(t + 4, 0);
                st32(t + 8, NONE);
                st64(t + 16, start);
                self.open_len += 1;
            }
        }
        j
    }

    /// A node opened at `start` (`w_open`).
    ///
    /// # Safety
    /// As `begin`'s.
    #[inline(always)]
    pub unsafe fn open(&mut self, number: u32, start: u64) {
        // SAFETY: the caller's.
        unsafe {
            if self.n == 0 {
                // The root: the file's first node, its block the first (`walk`'s document).
                let r = self.give().open_node_x::<X>(number, start, true);
                debug_assert!(r.is_ok(), "the writer's open, its room reserved");
                self.take();
            } else if self.n.is_multiple_of(Z) {
                self.start_block();
                self.node::<true>(number, start, true);
                self.settle_before();
            } else {
                self.node::<false>(number, start, true);
            }
        }
    }

    /// A leaf from `start` to `end` (`w_leaf`).
    ///
    /// # Safety
    /// As `begin`'s.
    #[inline(always)]
    pub unsafe fn leaf(&mut self, number: u32, start: u64, end: u64) {
        debug_assert!(end >= start && end <= self.source_len, "a leaf's end past its start, in the source");
        // SAFETY: the caller's.
        unsafe {
            if self.n == 0 {
                // The file's one node.
                let r = self.give().leaf_x::<X>(number, start, end);
                debug_assert!(r.is_ok(), "the writer's leaf, its room reserved");
                self.take();
            } else if self.n.is_multiple_of(Z) {
                self.start_block();
                let j = self.node::<true>(number, start, false);
                self.put(2, j, end - start);
                self.settle_before();
            } else {
                let j = self.node::<false>(number, start, false);
                self.put(2, j, end - start);
            }
        }
    }

    /// A node of one leaf child, both at once (`w_leaf_in`): the node from `start` to `end`, its child from `cs` to
    /// `ce`.
    ///
    /// # Safety
    /// As `begin`'s.
    #[inline(always)]
    pub unsafe fn leaf_in(&mut self, number: u32, child: u32, start: u64, cs: u64, ce: u64, end: u64) {
        debug_assert!(cs >= start && ce >= cs && end >= ce && end <= self.source_len, "the node's and its child's bounds in order");
        // SAFETY: the caller's.
        unsafe {
            if self.n == 0 {
                // The file's one node and its child.
                let r = self.give().leaf_in_x::<X>(number, child, start, cs, ce, end);
                debug_assert!(r.is_ok(), "the writer's node of one leaf, its room reserved");
                self.take();
                return;
            }
            let first = self.n.is_multiple_of(Z);
            let j = if first {
                self.start_block();
                self.node::<true>(number, start, false)
            } else {
                self.node::<false>(number, start, false)
            };
            self.put(2, j, end - start);
            self.put(0, j, 2 << 1);
            if first {
                self.settle_before();
            }
            // The child: the node's first, so no slot of the node before describes it; its block's first where the
            // node is the block's last.
            let cj = j + 1;
            let child_first = cj == Z;
            if child_first {
                self.start_block();
            }
            let cj = cj % Z;
            let block = self.block();
            (*self.s).put_code_at(cj, child, block);
            self.put(1, cj, cs - start);
            if cj.is_multiple_of(K) {
                (*self.s).put_checkpoint_x::<X>(self.room, cj, cs);
            }
            self.put(2, cj, ce - cs);
            self.prev_start = cs;
            self.n += 1;
            if child_first {
                self.settle_before();
            }
        }
    }

    /// The innermost open node closed at `end` (`w_close`): its length, and an inner node's size.
    ///
    /// # Safety
    /// As `begin`'s; a node is open.
    #[inline(always)]
    pub unsafe fn close(&mut self, end: u64) {
        debug_assert!(self.open_len != 0, "a node is open");
        // SAFETY: the caller's: the innermost open frame.
        unsafe {
            let t = self.opens + (self.open_len as usize - 1) * OPEN as usize;
            let (i, start) = (ld32(t), ld64(t + 16));
            debug_assert!(end >= start && end <= self.source_len, "a close's end past its node's start, in the source");
            // The node in the held block: its cells there, and no block to settle.
            if i / Z == (self.n - 1) / Z {
                self.open_len -= 1;
                let j = i % Z;
                self.put(2, j, end - start);
                let size = self.n - i;
                if size > 1 {
                    self.put(0, j, u64::from(size) << 1);
                }
                return;
            }
            // A node in a block before (`walk`'s close at `ord`): its cells stored in its block, made already; then
            // the block final once a node past it is made and its nodes are closed (`settle`, format 16's).
            self.open_len -= 1;
            let (b, j) = (i / Z, i % Z);
            let st = &mut *self.s;
            let block = st.block(b);
            st.put_at_x::<X>(2, j, end - start, block);
            let size = self.n - i;
            if size > 1 {
                st.put_at_x::<X>(0, j, u64::from(size) << 1, block);
            }
            if self.n > (b + 1) * Z {
                self.give();
                let r = (*self.s).settle(b);
                debug_assert!(r.is_ok(), "the writer's settle, its room reserved");
                self.take();
            }
        }
    }
}

impl<X: CellWidths> Drop for Cursor<X> {
    /// The state's fields the walk's.
    fn drop(&mut self) {
        self.give();
    }
}

/// `w_open`, the language's widths `X`'s (`Fixed`): a caller that knows them, as a generated parser knows its own.
///
/// # Safety
/// As `w_open`'s; the state's widths are `X`'s.
#[inline(always)]
pub unsafe fn w_open_with<X: CellWidths>(desc: *const u8, number: u32, start: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).open_node_x::<X>(number, start, true).map(|_| ())) }
}

/// `w_close`, as `w_open_with`.
///
/// # Safety
/// As `w_close`'s; the state's widths are `X`'s.
#[inline(always)]
pub unsafe fn w_close_with<X: CellWidths>(desc: *const u8, end: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).close_x::<X>(end)) }
}

/// `w_leaf`, as `w_open_with`.
///
/// # Safety
/// As `w_leaf`'s; the state's widths are `X`'s.
#[inline(always)]
pub unsafe fn w_leaf_with<X: CellWidths>(desc: *const u8, number: u32, start: u64, end: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).leaf_x::<X>(number, start, end)) }
}

/// A node of one leaf child, both written at once (`leaf_in_x`), as `w_open_with`.
///
/// # Safety
/// As `w_leaf`'s; the state's widths are `X`'s.
#[inline(always)]
pub unsafe fn w_leaf_in_with<X: CellWidths>(desc: *const u8, number: u32, child: u32, start: u64, cs: u64, ce: u64, end: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).leaf_in_x::<X>(number, child, start, cs, ce, end)) }
}

/// The nodes so far final but for the open ones; the blocks the frontier passed settled.
///
/// # Safety
/// As `w_open`'s.
#[inline(always)]
pub unsafe fn w_commit(desc: *const u8) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).commit()) }
}

/// Node `i`'s code given again; its block not final.
///
/// # Safety
/// As `w_open`'s.
pub unsafe fn w_retag(desc: *const u8, i: u32, number: u32) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).retag(i, number)) }
}

/// Open node `i`'s start given again: its gap, its checkpoint and the next node's gap.
///
/// # Safety
/// As `w_open`'s.
pub unsafe fn w_set_start(desc: *const u8, i: u32, start: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).set_start(i, start)) }
}

/// The tape: `w_finish_blocks`, then `w_finish_tail`.
///
/// # Safety
/// As `w_open`'s, the tail's room reserved.
pub unsafe fn w_finish(desc: *const u8) -> u32 {
    // SAFETY: the caller's.
    unsafe {
        let s = state(desc);
        code(s.finish_blocks().and_then(|()| s.finish_tail()))
    }
}

/// `finish`'s first half: the last node's slot, every block final.
///
/// # Safety
/// As `w_open`'s.
pub unsafe fn w_finish_blocks(desc: *const u8) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).finish_blocks()) }
}

/// `finish`'s second half: the tail and the header.
///
/// # Safety
/// As `w_finish`'s.
pub unsafe fn w_finish_tail(desc: *const u8) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).finish_tail()) }
}

/// The failure `code` at `node`, the first kept; returns `code`.
///
/// # Safety
/// As `w_open`'s.
#[cold]
pub unsafe fn w_fail(desc: *const u8, failure: u32, node: u32) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).fail(failure, node)) }
}

/// Node `i`'s value `v` in array `a` (0 slots, 1 gaps, 2 lengths): its cell, or its side list's entry.
///
/// # Safety
/// As `w_open`'s; node `i`'s block made.
pub unsafe fn w_put(desc: *const u8, a: u32, i: u32, v: u64) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).put(a as usize, i, v)) }
}

/// Node `i`'s code byte, and its side-list entry for `number`.
///
/// # Safety
/// As `w_put`'s.
pub unsafe fn w_put_code(desc: *const u8, i: u32, number: u32) -> u32 {
    // SAFETY: the caller's.
    unsafe {
        let s = state(desc);
        let block = s.block(i / Z);
        s.put_code_at(i % Z, number, block);
    }
    0
}

/// Room for block `b`, a side area, its table entry.
///
/// # Safety
/// As `w_open`'s.
pub unsafe fn w_new_block(desc: *const u8, b: u32) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).new_block(b)) }
}

/// Block `b` made final, if it is.
///
/// # Safety
/// As `w_open`'s; block `b` made.
pub unsafe fn w_settle(desc: *const u8, b: u32) -> u32 {
    // SAFETY: the caller's.
    unsafe { code(state(desc).settle(b)) }
}

/// The `n` bytes at `src` as one LZ4 block at `dst`; its length.
///
/// # Safety
/// `desc` is a writer's state with the compressor's tables; `dst` has room for `lz4_bound(n)` bytes and 8 more,
/// and `src`'s 3 bytes before it and 8 after are readable.
pub unsafe fn w_lz4(desc: *const u8, src: u64, n: u64, dst: u64) -> u64 {
    // SAFETY: the caller's.
    unsafe { state(desc).lz4(src as usize, n, dst as usize) }
}
