//! Tier 1's byte-local values on the block's byte vectors, before any mask is made (the CPU JIT,
//! `native_cpu_block`), as simdjson's stage 1 keeps its classes in vector registers.
//!
//! A value is **byte-local** when each of its bytes is a function of the block's bytes at most 16
//! back: a byte test, a class, a plane, the logic of byte-local values, and a `prev` by up to 16
//! of one without a `prev` of its own. Such a value is computed per 16-byte chunk:
//!
//! - **Shared lookups.** A table's two nibble lookups are made once per chunk, for every class
//!   taken from it.
//! - **A `prev` on bytes.** `prev(v, k)` is `v`'s chunk and the chunk before it, shifted by `k`
//!   bytes (one `ext` on aarch64). The chunk before the block's first is chunk 0: `v` over the 16
//!   bytes before the block, which are the previous block's, or zeros before a file. No carried
//!   word (simdjson carries that chunk's input as a vector instead).
//! - **One mask at the edge.** A value becomes a 64-bit mask only where a value outside the
//!   vectors, an output or a carried field reads it.
//!
//! The plan (`Plan::of`) also says which values are known on a window of ASCII bytes, for the
//! full-block body's one branch: when the block and the 16 bytes before it are all below 0x80,
//! a body with those values folded runs (its UTF-8 checks, typically, are all zero).

use std::collections::HashMap;
use std::rc::Rc;

use crate::native_cpu_block::{Sym, bitmask};
use crate::block_native::{BlockInst, BlockNative};
use cranelift_codegen::ir::{ConstantData, InstBuilder, MemFlags, Value, condcodes::IntCC, types};
use cranelift_frontend::FunctionBuilder;

// The plan this lowers (`lane_plan`, the analysis Metal reads too), and the bit test's default, the JIT's options'.
pub use crate::lane_plan::*;
pub use crate::native_cpu_codegen::BIT_TEST_DEFAULT;

/// A value's 16 bytes: known (all zero or all ones), or a vector, complemented or not (a
/// compare gives the complement free; `and` with one is a `bic`).
/// A value as bytes (`Lanes::as_bytes`): set where `v & bits` is nonzero; `v` is `base` outside
/// `bits`; `combined`, made of more than one rule.
#[derive(Clone, Copy)]
struct Bytes {
    v: Value,
    bits: u8,
    combined: bool,
    base: Value,
}

#[derive(Clone, Copy)]
pub enum Lane {
    Known(bool),
    V(Value, bool),
}

impl Lane {
    fn not(self) -> Lane {
        match self {
            Lane::Known(b) => Lane::Known(!b),
            Lane::V(v, neg) => Lane::V(v, !neg),
        }
    }

    fn and(f: &mut FunctionBuilder<'_>, a: Lane, b: Lane) -> Lane {
        match (a, b) {
            (Lane::Known(false), _) | (_, Lane::Known(false)) => Lane::Known(false),
            (Lane::Known(true), s) | (s, Lane::Known(true)) => s,
            (Lane::V(x, false), Lane::V(y, false)) => Lane::V(f.ins().band(x, y), false),
            (Lane::V(x, false), Lane::V(y, true)) | (Lane::V(y, true), Lane::V(x, false)) => Lane::V(f.ins().band_not(x, y), false),
            (Lane::V(x, true), Lane::V(y, true)) => Lane::V(f.ins().bor(x, y), true),
        }
    }

    fn xor(f: &mut FunctionBuilder<'_>, a: Lane, b: Lane) -> Lane {
        match (a, b) {
            (Lane::Known(x), Lane::Known(y)) => Lane::Known(x != y),
            (Lane::Known(false), s) | (s, Lane::Known(false)) => s,
            (Lane::Known(true), s) | (s, Lane::Known(true)) => s.not(),
            (Lane::V(x, nx), Lane::V(y, ny)) => Lane::V(f.ins().bxor(x, y), nx != ny),
        }
    }
}

/// The byte vectors of one block's body, and each byte-local value's chunks, made on demand.
/// Chunks 1 to 4 are the block's. A value is made at a lag (`get_at`): its chunk `c` at lag `L`
/// is its bytes `L` before chunk `c`'s, each leaf from the bytes loaded that far back. So
/// `prev(a, k)` at lag `L` is `a` at lag `L + k`, simdjson's `prev<N>` on the input, for any
/// depth of `prev`s within `MAX_LAG`.
#[derive(Clone)]
pub struct Lanes<'a> {
    native: &'a BlockNative,
    plan: &'a Plan,
    /// The first stream position of chunk 1 and the stream's length outside a full block's
    /// body (`None` there): for `within`, and for a `prev`'s cut before the stream.
    position: Option<(Value, Value)>,
    /// Each (chunk, lag)'s bytes, loaded once.
    bytes: HashMap<(usize, u8), Value>,
    base: Value,
    nibble: Value,
    zero16: Value,
    nibbles: HashMap<(usize, u8), (Value, Value)>,
    /// Chunks' high nibbles alone, for a table whose low half is all ones (`lookup`).
    highs: HashMap<(usize, u8), Value>,
    /// Each pair of tables' AND at (chunk, lag), shared by the pairs of their bits.
    pairs: HashMap<(u16, u16, usize, u8), Value>,
    /// Values read by `prev`s of one lag only (`only_prev`).
    only_prevs: HashMap<usize, bool>,
    /// Values as bytes (`as_bytes`), at (value, chunk, lag).
    bytes_of: HashMap<(usize, usize, u8), Option<Bytes>>,
    /// Each table's lookup at (chunk, lag), shared by its classes.
    lookups: HashMap<(usize, usize, u8), Value>,
    /// Each table's two halves, and each splatted byte, made once in the body.
    tables: Vec<Option<(Value, Value)>>,
    splats: [Option<Value>; 256],
    index: Option<Value>,
    memo: HashMap<(usize, usize, u8), Lane>,
    /// The body's classes packed (`body_pack`): their tables, and each table's lookups.
    pack: Option<Rc<BodyPack>>,
    packed: Vec<Option<(Value, Value)>>,
    packed_lookups: HashMap<(usize, usize, u8), Value>,
    tests: HashMap<(usize, usize, u8), Lane>,
    /// Values known all through the window (`assuming`): at lag 0, its own bytes.
    assume: Option<Rc<HashMap<usize, bool>>>,
    /// Each range compare (`range`): its low byte, span, chunk and lag.
    ranges: HashMap<(u8, u8, usize, u8), Value>,
}

impl<'a> Lanes<'a> {
    /// `bytes`: the block's chunks at lag 0 (index 0: the 16 bytes before it), when already
    /// loaded. `edge`: where the body may run on a file's first or last block, the block's index
    /// and the file's length.
    pub fn new(f: &mut FunctionBuilder<'_>, native: &'a BlockNative, plan: &'a Plan, base: Value, edge: Option<(Value, Value)>, bytes: Option<[Option<Value>; 5]>, nibble: Value) -> Self {
        let c = f.ins().iconst(types::I8, 0);
        let zero16 = f.ins().splat(types::I8X16, c);
        let mut loaded = HashMap::new();
        for (k, b) in bytes.unwrap_or([None; 5]).into_iter().enumerate() {
            if let Some(b) = b {
                loaded.insert(if k == 0 { (1, 16) } else { (k, 0) }, b);
            }
        }
        Lanes {
            native,
            plan,
            position: edge.map(|(i, len)| (f.ins().ishl_imm(i, 6), len)),
            bytes: loaded,
            base,
            nibble,
            zero16,
            nibbles: HashMap::new(),
            highs: HashMap::new(),
            pairs: HashMap::new(),
            bytes_of: HashMap::new(),
            only_prevs: HashMap::new(),
            lookups: HashMap::new(),
            tables: vec![None; native.tables.len()],
            splats: [None; 256],
            index: None,
            memo: HashMap::new(),
            pack: None,
            packed: Vec::new(),
            packed_lookups: HashMap::new(),
            tests: HashMap::new(),
            assume: None,
            ranges: HashMap::new(),
        }
    }

    /// The lanes of an edge block's part (`native_cpu_block`'s `Emit::edge_part`, 0383): `within`
    /// and a `prev`'s cut from block `i`'s position, and nothing the body made read again (its
    /// bytes, lookups, chunks, constants), so that none of it is live to the body's end.
    pub fn on_edge(&mut self, f: &mut FunctionBuilder<'_>, i: Value, len: Value, nibble: Value) {
        self.position = Some((f.ins().ishl_imm(i, 6), len));
        self.nibble = nibble;
        let c = f.ins().iconst(types::I8, 0);
        self.zero16 = f.ins().splat(types::I8X16, c);
        self.bytes.clear();
        self.nibbles.clear();
        self.highs.clear();
        self.lookups.clear();
        self.pairs.clear();
        self.bytes_of.clear();
        self.tables = vec![None; self.native.tables.len()];
        self.splats = [None; 256];
        self.index = None;
        self.memo.clear();
        self.packed = vec![None; self.packed.len()];
        self.packed_lookups.clear();
        self.tests.clear();
        self.ranges.clear();
    }

    /// A window's values (`native_cpu_bytes`): its 64 bytes from `base`, the stream's position
    /// `start`, anywhere in the file; `within` and a `prev`'s cut from `start`.
    pub fn window(f: &mut FunctionBuilder<'_>, native: &'a BlockNative, plan: &'a Plan, base: Value, start: Value, len: Value, nibble: Value) -> Self {
        let c = f.ins().iconst(types::I8, 0);
        let zero16 = f.ins().splat(types::I8X16, c);
        Lanes {
            native,
            plan,
            position: Some((start, len)),
            bytes: HashMap::new(),
            base,
            nibble,
            zero16,
            nibbles: HashMap::new(),
            highs: HashMap::new(),
            pairs: HashMap::new(),
            bytes_of: HashMap::new(),
            only_prevs: HashMap::new(),
            lookups: HashMap::new(),
            tables: vec![None; native.tables.len()],
            splats: [None; 256],
            index: None,
            memo: HashMap::new(),
            pack: None,
            packed: Vec::new(),
            packed_lookups: HashMap::new(),
            tests: HashMap::new(),
            assume: None,
            ranges: HashMap::new(),
        }
    }

    /// The block's first byte.
    pub fn base(&self) -> Value {
        self.base
    }

    /// The bytes `lag` before chunk `c`'s, loaded once (an unaligned load: the window holds
    /// `MAX_LAG` bytes before a block, `FRONT` zeros before a file).
    fn bytes(&mut self, f: &mut FunctionBuilder<'_>, c: usize, lag: u8) -> Value {
        // Chunk 0 is chunk 1's 16 bytes back: one key, so a carried chunk 0 is read as itself.
        if c == 0 {
            return self.bytes(f, 1, lag + 16);
        }
        if let Some(&v) = self.bytes.get(&(c, lag)) {
            return v;
        }
        // Inside the chunk before and this one, both in registers: an `ext` of the two, as
        // simdjson's `prev<N>` of its input (chunk 0, the bytes before the block, perhaps the
        // block before's last chunk, carried: `Lanes::new`).
        let chunk = |c: usize| if c == 1 { (1, 16) } else { (c - 1, 0) };
        if (1..16).contains(&lag)
            && let (Some(&back), Some(&here)) = (self.bytes.get(&chunk(c)), self.bytes.get(&(c, 0)))
        {
            let lanes: Vec<u8> = (0..16u8).map(|l| 16 - lag + l).collect();
            let mask = f.func.dfg.immediates.push(ConstantData::from(&lanes[..]));
            let v = f.ins().shuffle(back, here, mask);
            self.bytes.insert((c, lag), v);
            return v;
        }
        let v = f.ins().load(types::I8X16, MemFlags::new().with_notrap(), self.base, 16 * c as i32 - 16 - i32::from(lag));
        self.bytes.insert((c, lag), v);
        v
    }

    /// Table `t`'s class bytes at chunk `c`, lag `lag`, shared by its classes.
    fn lookup(&mut self, f: &mut FunctionBuilder<'_>, t: usize, c: usize, lag: u8) -> Value {
        if let Some(&v) = self.lookups.get(&(t, c, lag)) {
            return v;
        }
        // A table of high nibbles only (its low half all ones): one lookup, as simdjson's
        // `byte_2_high`.
        if self.native.tables[t].lo == [0xff; 16] {
            let hi = self.high_at(f, c, lag);
            let thi = match self.tables[t] {
                Some((_, thi)) => thi,
                None => {
                    let thi = f.func.dfg.constants.insert(ConstantData::from(&self.native.tables[t].hi[..]));
                    let thi = f.ins().vconst(types::I8X16, thi);
                    self.tables[t] = Some((thi, thi));
                    thi
                }
            };
            let v = f.ins().swizzle(thi, hi);
            self.lookups.insert((t, c, lag), v);
            return v;
        }
        let (lo, hi) = self.nibbles_at(f, c, lag);
        let (tlo, thi) = match self.tables[t] {
            Some(x) => x,
            None => {
                let table = &self.native.tables[t];
                let tlo = f.func.dfg.constants.insert(ConstantData::from(&table.lo[..]));
                let thi = f.func.dfg.constants.insert(ConstantData::from(&table.hi[..]));
                let x = (f.ins().vconst(types::I8X16, tlo), f.ins().vconst(types::I8X16, thi));
                self.tables[t] = Some(x);
                x
            }
        };
        let l = f.ins().swizzle(tlo, lo);
        let h = f.ins().swizzle(thi, hi);
        let v = f.ins().band(l, h);
        self.lookups.insert((t, c, lag), v);
        v
    }

    /// This body's classes by the pack `p` (`body_pack`): their lookups its tables'.
    pub fn packed(mut self, p: Option<Rc<BodyPack>>) -> Self {
        self.packed = vec![None; p.as_ref().map_or(0, |p| p.packing.tables.len())];
        self.pack = p;
        self
    }

    /// Packed table `t`'s class bytes at chunk `c`, lag `lag`, shared by the tests that read it.
    fn packed_lookup(&mut self, f: &mut FunctionBuilder<'_>, t: usize, c: usize, lag: u8) -> Value {
        if let Some(&v) = self.packed_lookups.get(&(t, c, lag)) {
            return v;
        }
        let (lo, hi) = self.nibbles_at(f, c, lag);
        let (tlo, thi) = match self.packed[t] {
            Some(x) => x,
            None => {
                let table = &self.pack.as_ref().expect("a pack").packing.tables[t];
                let tlo = f.func.dfg.constants.insert(ConstantData::from(&table.lo[..]));
                let thi = f.func.dfg.constants.insert(ConstantData::from(&table.hi[..]));
                let x = (f.ins().vconst(types::I8X16, tlo), f.ins().vconst(types::I8X16, thi));
                self.packed[t] = Some(x);
                x
            }
        };
        let l = f.ins().swizzle(tlo, lo);
        let h = f.ins().swizzle(thi, hi);
        let v = f.ins().band(l, h);
        self.packed_lookups.insert((t, c, lag), v);
        v
    }

    /// The pack's test `k` at chunk `c`, lag `lag`: its range's subtract and compare, its
    /// tables' bits, ORed, complemented where the test is.
    fn test(&mut self, f: &mut FunctionBuilder<'_>, k: usize, c: usize, lag: u8) -> Lane {
        if let Some(&l) = self.tests.get(&(k, c, lag)) {
            return l;
        }
        let t = self.pack.as_ref().expect("a pack").packing.tests[k].clone();
        let mut acc = Lane::Known(false);
        let or = |f: &mut FunctionBuilder<'_>, a: Lane, b: Lane| Lane::and(f, a.not(), b.not()).not();
        if let Some((lo, span)) = t.range {
            let lane = self.range(f, lo, span, c, lag);
            acc = or(f, acc, lane);
        }
        // A table's bits tested once, however many of its parts the set takes.
        let mut tables: Vec<(u16, u8)> = Vec::new();
        for &(tb, bits) in &t.tables {
            match tables.iter_mut().find(|x| x.0 == tb) {
                Some(x) => x.1 |= bits,
                None => tables.push((tb, bits)),
            }
        }
        for (tb, bits) in tables {
            let v = self.packed_lookup(f, usize::from(tb), c, lag);
            // `none_of` is the class, complemented: its compare is the bytes outside it.
            let lane = self.none_of(f, v, bits);
            acc = or(f, acc, lane);
        }
        let lane = if t.negated { acc.not() } else { acc };
        self.tests.insert((k, c, lag), lane);
        lane
    }

    /// The bytes `lo ..= lo + span` (wrapping) at chunk `c`, lag `lag`: one compare where `lo`
    /// is zero, `span` is, or the range runs to 0xff, else a subtract and a compare; made once.
    fn range(&mut self, f: &mut FunctionBuilder<'_>, lo: u8, span: u8, c: usize, lag: u8) -> Lane {
        if span == 255 {
            return Lane::Known(true);
        }
        if let Some(&v) = self.ranges.get(&(lo, span, c, lag)) {
            return Lane::V(v, false);
        }
        let b = self.bytes(f, c, lag);
        let v = if span == 0 {
            let s = self.splat(f, lo);
            f.ins().icmp(IntCC::Equal, b, s)
        } else if u16::from(lo) + u16::from(span) == 0xff {
            // Up to 0xff: one compare, as simdjson's `prev2 >= 0xe0` (its saturating subtract).
            let s = self.splat(f, lo);
            f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, b, s)
        } else {
            let d = if lo == 0 {
                b
            } else {
                let from = self.splat(f, lo);
                f.ins().isub(b, from)
            };
            let sp = self.splat(f, span);
            f.ins().icmp(IntCC::UnsignedLessThanOrEqual, d, sp)
        };
        self.ranges.insert((lo, span, c, lag), v);
        Lane::V(v, false)
    }

    /// The pack's extra set `k` (`body_pack`'s `extras`) at chunk `c`.
    pub fn extra(&mut self, f: &mut FunctionBuilder<'_>, k: usize, c: usize) -> Lane {
        let at = self.pack.as_ref().expect("a pack").extra + k;
        self.test(f, at, c, 0)
    }

    /// Chunk `c`'s nibbles at lag `lag`, made once.
    fn nibbles_at(&mut self, f: &mut FunctionBuilder<'_>, c: usize, lag: u8) -> (Value, Value) {
        match self.nibbles.get(&(c, lag)) {
            Some(&x) => x,
            None => {
                let b = self.bytes(f, c, lag);
                let x = (f.ins().band(b, self.nibble), self.high_at(f, c, lag));
                self.nibbles.insert((c, lag), x);
                x
            }
        }
    }

    /// Chunk `c`'s high nibbles at lag `lag`, made once.
    fn high_at(&mut self, f: &mut FunctionBuilder<'_>, c: usize, lag: u8) -> Value {
        if let Some(&(_, h)) = self.nibbles.get(&(c, lag)) {
            return h;
        }
        if let Some(&h) = self.highs.get(&(c, lag)) {
            return h;
        }
        let b = self.bytes(f, c, lag);
        let h = f.ins().ushr_imm(b, 4);
        self.highs.insert((c, lag), h);
        h
    }

    fn splat(&mut self, f: &mut FunctionBuilder<'_>, byte: u8) -> Value {
        if byte == 0 {
            return self.zero16;
        }
        if let Some(v) = self.splats[usize::from(byte)] {
            return v;
        }
        let c = f.ins().iconst(types::I8, i64::from(byte));
        let v = f.ins().splat(types::I8X16, c);
        self.splats[usize::from(byte)] = Some(v);
        v
    }

    /// The bytes with any of `bits` set in `v`. Where the backend tests bits in one instruction
    /// (`Options::bit_test`: NEON's `vtst`, as the lean lexer's window reads its classes), the test
    /// itself; else its complement, a compare with zero, the polarity free.
    /// Whether value `x` is read by `prev(x, k)`s of one `k` only: no other value or output
    /// reads it (`Prev`'s `lagged_only`).
    fn only_prev(&mut self, x: usize) -> bool {
        if let Some(&b) = self.only_prevs.get(&x) {
            return b;
        }
        let mut ks = Vec::new();
        let mut other = self.native.outputs.contains(&(x as u16));
        for inst in &self.native.insts {
            if inst.operands().contains(&x) {
                match *inst {
                    BlockInst::Prev(a, k) if usize::from(a) == x => ks.push(k),
                    _ => other = true,
                }
            }
        }
        ks.dedup();
        let b = !other && ks.len() == 1;
        self.only_prevs.insert(x, b);
        b
    }

    /// A pair of tables' AND at chunk `c`, lag `lag`: the byte before's classes and this
    /// byte's, made once for every pair of their bits.
    fn pair_and(&mut self, f: &mut FunctionBuilder<'_>, before: u16, table: u16, c: usize, lag: u8) -> Value {
        if let Some(&v) = self.pairs.get(&(before, table, c, lag)) {
            return v;
        }
        let p = self.lookup(f, usize::from(before), c, lag + 1);
        let t = self.lookup(f, usize::from(table), c, lag);
        let v = f.ins().band(p, t);
        self.pairs.insert((before, table, c, lag), v);
        v
    }

    /// Value `at` as bytes (decision 0405, simdjson's `special_cases ^ must_be_2_3_continuation`):
    /// `(v, bits, combined, base)`, set where `v & bits` is nonzero, and `v` equal to `base`
    /// outside `bits`.
    /// - A pair: its tables' AND and its bits.
    /// - An `xor` of a pair of one bit and any lane: that bit of the AND flipped where the lane
    ///   is set (an `and`, an `eor`), the rest of the AND as it was.
    /// - An `or` of two with no bit in common, from one AND: their bytes in one (none made where
    ///   one is the AND itself, as simdjson's `special_cases`).
    ///
    /// `combined`: some `or` or `xor` was made so, its tests saved.
    fn as_bytes(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize, lag: u8) -> Option<Bytes> {
        if let Some(&v) = self.bytes_of.get(&(at, c, lag)) {
            return v;
        }
        let o = |x: u16| usize::from(x);
        let v = match self.native.insts[at] {
            BlockInst::Pair { before, table, bits } => {
                let v = self.pair_and(f, before, table, c, lag);
                Some(Bytes { v, bits, combined: false, base: v })
            }
            BlockInst::Const(0) => Some(Bytes { v: self.zero16, bits: 0, combined: false, base: self.zero16 }),
            BlockInst::Or(a, b) => match (self.as_bytes(f, o(a), c, lag), self.as_bytes(f, o(b), c, lag)) {
                (Some(x), Some(y)) if x.bits == 0 => Some(y),
                (Some(x), Some(y)) if y.bits == 0 => Some(x),
                (Some(x), Some(y)) if x.bits & y.bits == 0 && x.base == y.base => {
                    // Outside its bits each is the base: the two's XOR with the base is each
                    // in its bits.
                    let v = if x.v == x.base {
                        y.v
                    } else if y.v == y.base {
                        x.v
                    } else {
                        let both = f.ins().bxor(x.v, y.v);
                        f.ins().bxor(both, x.base)
                    };
                    Some(Bytes { v, bits: x.bits | y.bits, combined: true, base: x.base })
                }
                _ => None,
            },
            BlockInst::Xor(a, b) => {
                let one = |r: Option<Bytes>| r.filter(|r| r.bits.count_ones() == 1);
                let (x, other) = match one(self.as_bytes(f, o(a), c, lag)) {
                    Some(x) => (Some(x), o(b)),
                    None => (one(self.as_bytes(f, o(b), c, lag)), o(a)),
                };
                match x {
                    Some(x) => {
                        let y = self.get_at(f, other, c, lag);
                        let y = self.polar(f, y, false);
                        let sel = self.splat(f, x.bits);
                        let flip = f.ins().band(y, sel);
                        Some(Bytes { v: f.ins().bxor(x.v, flip), bits: x.bits, combined: true, base: x.base })
                    }
                    None => None,
                }
            }
            _ => None,
        };
        self.bytes_of.insert((at, c, lag), v);
        v
    }

    fn none_of(&mut self, f: &mut FunctionBuilder<'_>, v: Value, bits: u8) -> Lane {
        let sel = self.splat(f, bits);
        let picked = f.ins().band(v, sel);
        if crate::native_cpu_codegen::options().bit_test.load(std::sync::atomic::Ordering::Relaxed) {
            return Lane::V(f.ins().icmp(IntCC::NotEqual, picked, self.zero16), false);
        }
        Lane::V(f.ins().icmp(IntCC::Equal, picked, self.zero16), true)
    }

    /// A lane as a plain or complemented vector of the polarity `neg`.
    fn polar(&mut self, f: &mut FunctionBuilder<'_>, x: Lane, neg: bool) -> Value {
        match x {
            Lane::Known(b) => {
                if b != neg {
                    self.splat(f, 0xff)
                } else {
                    self.zero16
                }
            }
            Lane::V(v, n) if n == neg => v,
            Lane::V(v, _) => f.ins().bnot(v),
        }
    }

    /// Instruction `at`'s chunk `c`.
    pub fn get(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize) -> Lane {
        self.get_at(f, at, c, 0)
    }

    /// `get`, `lag` bytes back: another output read in this window, its bytes and compares
    /// shared (`Bytewise::short_words`).
    pub fn get_lagged(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize, lag: u8) -> Lane {
        self.get_at(f, at, c, lag)
    }

    /// These values, with values `known` (`native_cpu_bytes::known_if_zero`) what they are
    /// at every byte of the window: its own bytes, lag 0, not those a `prev` reads before it.
    /// Its loads and lookups made already are kept, the values made from them made again.
    pub fn assuming(mut self, known: Rc<HashMap<usize, bool>>) -> Self {
        self.memo.clear();
        self.bytes_of.clear();
        self.assume = Some(known);
        self
    }

    /// These values with nothing assumed again: what was made from values the assumption may
    /// change (`changed`) at lag 0 dropped, the rest kept.
    pub fn unassuming(mut self, changed: &[bool]) -> Self {
        self.assume = None;
        self.memo.retain(|&(at, _, lag), _| lag != 0 || !changed[at]);
        self.bytes_of.clear();
        self
    }

    /// Instruction `at`'s chunk `c`, `lag` bytes back.
    fn get_at(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize, lag: u8) -> Lane {
        if lag == 0
            && let Some(&b) = self.assume.as_ref().and_then(|k| k.get(&at))
        {
            return Lane::Known(b);
        }
        if let Some(&l) = self.memo.get(&(at, c, lag)) {
            return l;
        }
        let lane = self.make(f, at, c, lag);
        self.memo.insert((at, c, lag), lane);
        lane
    }

    fn make(&mut self, f: &mut FunctionBuilder<'_>, at: usize, c: usize, lag: u8) -> Lane {
        let o = |x: u16| usize::from(x);
        match self.native.insts[at] {
            BlockInst::Const(x) => Lane::Known(x != 0),
            BlockInst::Within(d) => self.within(f, d, c, lag),
            BlockInst::EqByte(byte) => {
                let b = self.bytes(f, c, lag);
                let s = self.splat(f, byte);
                Lane::V(f.ins().icmp(IntCC::Equal, b, s), false)
            }
            // A class of the body's pack (`body_pack`): its test, its lookup the pack's.
            BlockInst::Class { table, bits } if self.pack.as_ref().is_some_and(|p| p.of.contains_key(&(table, bits))) => {
                let k = self.pack.as_ref().expect("a pack").of[&(table, bits)];
                self.test(f, k, c, lag)
            }
            // A class of one run of bytes, its table not looked up here already: a subtract
            // and a compare, as a compiler lowers a range test (a digit's `b - '0' <= 9`).
            BlockInst::Class { table, bits } if !self.lookups.contains_key(&(usize::from(table), c, lag)) && byte_range(&self.native.tables[usize::from(table)], bits).is_some() => {
                let (lo, hi) = byte_range(&self.native.tables[usize::from(table)], bits).expect("a range");
                self.range(f, lo, hi.wrapping_sub(lo), c, lag)
            }
            BlockInst::Class { table, bits } => {
                let v = self.lookup(f, usize::from(table), c, lag);
                self.none_of(f, v, bits)
            }
            BlockInst::Plane(k) => {
                let b = self.bytes(f, c, lag);
                self.none_of(f, b, 1 << k)
            }
            BlockInst::PlaneBefore(k) => {
                let b = self.bytes(f, c, lag + 1);
                self.none_of(f, b, 1 << k)
            }
            BlockInst::Pair { before, table, bits } => {
                let both = self.pair_and(f, before, table, c, lag);
                self.none_of(f, both, bits)
            }
            BlockInst::Not(a) => self.get_at(f, o(a), c, lag).not(),
            BlockInst::And(a, b) => {
                // A `within` the other conjuncts need no test of: off the file the byte it
                // guards is zero, and one of them is false on it (`unguarded`).
                // (A full block's body has no `within` to test: `position` none.)
                if let Some(kept) = self.position.and_then(|_| unguarded(self.native, at)) {
                    let mut acc = Lane::Known(true);
                    for x in kept {
                        let v = self.get_at(f, x, c, lag);
                        acc = Lane::and(f, acc, v);
                    }
                    return acc;
                }
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::and(f, x, y)
            }
            BlockInst::AndNot(a, b) => {
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::and(f, x, y.not())
            }
            // Pair rules ORed and XORed as bytes, tested once (`as_bytes`).
            BlockInst::Or(..) | BlockInst::Xor(..) if let Some(Bytes { v, bits, combined: true, .. }) = self.as_bytes(f, at, c, lag) => self.none_of(f, v, bits),
            BlockInst::Or(a, b) => {
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::and(f, x.not(), y.not()).not()
            }
            BlockInst::Xor(a, b) => {
                let (x, y) = (self.get_at(f, o(a), c, lag), self.get_at(f, o(b), c, lag));
                Lane::xor(f, x, y)
            }
            BlockInst::Prev(a, k) => {
                // simdjson's `prev<N>`: the chunk before's last `k` bytes, then this chunk's first
                // `16 - k`, one `ext`. The first chunk's chunk before is chunk 0, the 16 bytes
                // before the block (the previous block's last chunk, simdjson's `prev_input`),
                // made as any chunk is; before a file they are zeros, read as no bits.
                // A value that only `prev`s of this lag read, made from its own bytes: made from
                // the bytes `k` back instead (`bytes`, an `ext` of the chunks in registers), as
                // simdjson makes `prev2 >= 0xe0` from `prev<2>(input)` (decision 0417). An `ext`
                // of its values would make it on five chunks, chunk 0 the fifth.
                let lagged_only = lag == 0 && k < 16 && self.only_prev(o(a)) && reads_here(self.native, o(a), 0);
                if !lagged_only && (c >= 2 || (c == 1 && lag == 0 && reads_here(self.native, o(a), 0))) {
                    let (back, here) = (self.get_at(f, o(a), c - 1, lag), self.get_at(f, o(a), c, lag));
                    let v = self.ext(f, back, here, k);
                    if c == 1
                        && self.plan.at_zero[o(a)]
                        && let Some(cut) = self.from(f, c, lag, k)
                    {
                        return Lane::and(f, v, cut);
                    }
                    return v;
                }
                // The first chunk of a value read `lag` back, or of one that reads bytes back
                // itself: its chunk before would need bytes before the window, so `a` is made `k`
                // bytes further back instead.
                let v = self.get_at(f, o(a), c, lag + k);
                // Before the stream a `prev` reads no bits: where `a` may hold on the zeros
                // before a file, cut this value's positions less than `k` (a first block).
                if self.plan.at_zero[o(a)]
                    && let Some(cut) = self.from(f, c, lag, k)
                {
                    return Lane::and(f, v, cut);
                }
                v
            }
            _ => unreachable!("not byte-local"),
        }
    }

    /// Bytes `16 - k ..` of `back`, then `here`'s first `16 - k`: one `ext`.
    fn ext(&mut self, f: &mut FunctionBuilder<'_>, back: Lane, here: Lane, k: u8) -> Lane {
        if let (Lane::Known(x), Lane::Known(y)) = (back, here)
            && x == y
        {
            return Lane::Known(x);
        }
        let neg = match (back, here) {
            (_, Lane::V(_, n)) | (Lane::V(_, n), _) => n,
            _ => false,
        };
        let (x, y) = (self.polar(f, back, neg), self.polar(f, here, neg));
        let lanes: Vec<u8> = (0..16u8).map(|l| 16 - k + l).collect();
        let mask = f.func.dfg.immediates.push(ConstantData::from(&lanes[..]));
        Lane::V(f.ins().shuffle(x, y, mask), neg)
    }

    /// The lanes' indexes, 0 to 15.
    fn index(&mut self, f: &mut FunctionBuilder<'_>) -> Value {
        if let Some(v) = self.index {
            return v;
        }
        let index: Vec<u8> = (0..16).collect();
        let index = f.func.dfg.constants.insert(ConstantData::from(&index[..]));
        let v = f.ins().vconst(types::I8X16, index);
        self.index = Some(v);
        v
    }

    /// Chunk `c`'s first stream position, and `x` less it clamped to `0..=16` as a splat: a lane
    /// index bound.
    fn bound(f: &mut FunctionBuilder<'_>, start: Value, c: usize, lag: u8, x: Value) -> Value {
        let b = f.ins().iadd_imm(start, 16 * c as i64 - 16 - i64::from(lag));
        let x = f.ins().isub(x, b);
        let (zero, sixteen) = (f.ins().iconst(types::I64, 0), f.ins().iconst(types::I64, 16));
        let x = f.ins().smax(x, zero);
        let x = f.ins().smin(x, sixteen);
        let x = f.ins().ireduce(types::I8, x);
        f.ins().splat(types::I8X16, x)
    }

    /// The lanes of chunk `c`, `lag` back, at stream positions `k` or more: `None` in a full
    /// block's body, where all are (its positions are at least `64 - MAX_LAG`).
    fn from(&mut self, f: &mut FunctionBuilder<'_>, c: usize, lag: u8, k: u8) -> Option<Lane> {
        let (i, _) = self.position?;
        let at = f.ins().iconst(types::I64, i64::from(k));
        let lo = Self::bound(f, i, c, lag, at);
        let index = self.index(f);
        Some(Lane::V(f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index, lo), false))
    }

    /// `within(d)`'s chunk `c` at lag `lag`: the lanes at stream positions `p` (less the lag)
    /// with `d <= p < len + d`. In a full block's body all are: from the second block on, `p` is
    /// at least 48 for any lag up to 16, and `d <= 48` (`lane_plan`). Elsewhere `lo <= j < hi`,
    /// the range's ends less the chunk's first position, clamped: two compares with the indexes.
    fn within(&mut self, f: &mut FunctionBuilder<'_>, d: u8, c: usize, lag: u8) -> Lane {
        let Some((i, len)) = self.position else {
            return Lane::Known(true);
        };
        let d64 = f.ins().iconst(types::I64, i64::from(d));
        let lo = Self::bound(f, i, c, lag, d64);
        let end = f.ins().iadd_imm(len, i64::from(d));
        let hi = Self::bound(f, i, c, lag, end);
        let index = self.index(f);
        let from = f.ins().icmp(IntCC::UnsignedGreaterThanOrEqual, index, lo);
        let to = f.ins().icmp(IntCC::UnsignedLessThan, index, hi);
        Lane::V(f.ins().band(from, to), false)
    }

    /// A short window's one chunk of a value as a mask of `stride` bits a lane
    /// (`native_cpu_block::short_mask`), complemented as it is.
    pub fn mask_short(&mut self, f: &mut FunctionBuilder<'_>, part: Lane, weights: Value, stride: u32) -> Sym {
        if stride == 1 {
            return self.mask(f, &[part], weights);
        }
        match part {
            Lane::Known(b) => Sym::Known(if b { u64::MAX } else { 0 }),
            Lane::V(_, neg) => {
                let v = self.polar(f, part, neg);
                Sym::Value(crate::native_cpu_block::short_mask(f, v, weights), neg)
            }
        }
    }

    /// Whether any lane of values `xs` in the block's four chunks is set, as a word nonzero
    /// where one is: the chunks ORed, then the vector's two halves, no reduction to bits. A value made as bytes (`as_bytes`) is ORed as its bytes, untested,
    /// as simdjson ORs its error bytes into its error register: a nonzero byte is a set lane.
    pub fn any_of(&mut self, f: &mut FunctionBuilder<'_>, xs: &[usize]) -> Sym {
        match self.any_bytes(f, xs) {
            None => Sym::Known(0),
            Some(v) => {
                let halves = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little), v);
                let (lo, hi) = (f.ins().extractlane(halves, 0), f.ins().extractlane(halves, 1));
                Sym::Value(f.ins().bor(lo, hi), false)
            }
        }
    }

    /// Values `xs` in the block's four chunks ORed on the vectors (`any_of`): a byte nonzero
    /// where a lane is set, unreduced, for simdjson's error register (decision 0406). `None`:
    /// no lane can be.
    pub fn any_bytes(&mut self, f: &mut FunctionBuilder<'_>, xs: &[usize]) -> Option<Value> {
        let mut parts = Vec::new();
        for &x in xs {
            for c in 1..5 {
                let lane = match self.as_bytes(f, x, c, 0) {
                    Some(b) if b.bits == 0xff => Lane::V(b.v, false),
                    Some(b) if b.bits != 0 => {
                        let sel = self.splat(f, b.bits);
                        Lane::V(f.ins().band(b.v, sel), false)
                    }
                    Some(_) => Lane::Known(false),
                    None => self.get(f, x, c),
                };
                parts.push(lane);
            }
        }
        if parts.iter().any(|p| matches!(p, Lane::Known(true))) {
            return Some(self.splat(f, 0xff));
        }
        let vs: Vec<Value> = parts.iter().filter(|p| matches!(p, Lane::V(..))).map(|&p| self.polar(f, p, false)).collect();
        let (&first, rest) = vs.split_first()?;
        Some(rest.iter().fold(first, |acc, &v| f.ins().bor(acc, v)))
    }

    /// The block's four chunks of a value as a 64-bit mask, complemented as they are.
    pub fn mask(&mut self, f: &mut FunctionBuilder<'_>, parts: &[Lane], weights: Value) -> Sym {
        if let Some(Lane::Known(b)) = parts.first().copied()
            && parts.iter().all(|p| matches!(p, Lane::Known(x) if *x == b))
        {
            return Sym::Known(if b { u64::MAX } else { 0 });
        }
        let neg = parts.iter().find_map(|p| if let Lane::V(_, n) = p { Some(*n) } else { None }).unwrap_or(false);
        let vs: Vec<Value> = parts.iter().map(|&p| self.polar(f, p, neg)).collect();
        Sym::Value(bitmask(f, &vs, weights), neg)
    }

}

/// Whether the block's bytes, and the 16 before it when a value reads them, are all ASCII: the
/// full body's one branch. Gives the test and the chunks it loaded, for the bodies after it.
pub fn all_ascii(f: &mut FunctionBuilder<'_>, native: &BlockNative, plan: &Plan, base: Value, chunk0: Option<Value>) -> (Value, [Option<Value>; 5]) {
    let back = reads_before(native, plan);
    let mut bytes = [None; 5];
    let mut any: Option<Value> = None;
    for (c, slot) in bytes.iter_mut().enumerate().skip(if back { 0 } else { 1 }) {
        let b = match chunk0 {
            Some(v) if c == 0 => v,
            _ => f.ins().load(types::I8X16, MemFlags::new().with_notrap(), base, 16 * c as i32 - 16),
        };
        *slot = Some(b);
        any = Some(match any {
            None => b,
            Some(x) => f.ins().bor(x, b),
        });
    }
    let any = any.expect("four chunks");
    let words = f.ins().bitcast(types::I64X2, MemFlags::new().with_endianness(cranelift_codegen::ir::Endianness::Little), any);
    let (a, b) = (f.ins().extractlane(words, 0), f.ins().extractlane(words, 1));
    let both = f.ins().bor(a, b);
    let high = f.ins().band_imm(both, 0x8080_8080_8080_8080_u64 as i64);
    (f.ins().icmp_imm(IntCC::Equal, high, 0), bytes)
}
