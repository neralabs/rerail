//! Block programs on Metal (`block`), both ways the transducer runs on the device:
//!
//! - **the interpreter** (`metal_block_program.metal`): the set program's shape, one dispatch
//!   running every operation in order;
//! - **generated MSL** from the native IR (`BlockNative`): the operations written out as one
//!   straight-line kernel, with no loop, no switch and no branch.
//!
//! Each lane owns `W` consecutive source bytes and builds its own `W`-bit word of every value
//! (`LaneWidth`: 16, 32 or 64), so a SIMD group's 32 lanes work on 32 words at once, not one bit
//! each. A byte compare is SWAR over the lane's `uint4`s; the logic is word logic.
use super::*;
use crate::code_rule_dsl::transducer::packed::block::{BlockOp, BlockProgram, mask_tail};
use crate::code_rule_dsl::transducer::packed::native_ir::{BlockInst, BlockNative};
use metal::MTLSize;

const SOURCE: &str = include_str!("metal_block_program.metal");
/// Lanes per threadgroup: one SIMD group.
const GROUP: usize = 32;

/// The bytes each lane owns, and so the width of its mask words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum LaneWidth {
    W16,
    W32,
    W64,
}

impl LaneWidth {
    pub(in crate::code_rule_dsl) const ALL: [LaneWidth; 3] = [LaneWidth::W16, LaneWidth::W32, LaneWidth::W64];

    pub(in crate::code_rule_dsl) fn bytes(self) -> usize {
        match self {
            LaneWidth::W16 => 16,
            LaneWidth::W32 => 32,
            LaneWidth::W64 => 64,
        }
    }

    /// The MSL type of a lane word.
    fn word(self) -> &'static str {
        match self {
            LaneWidth::W16 => "ushort",
            LaneWidth::W32 => "uint",
            LaneWidth::W64 => "ulong",
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlockArgs {
    ops: u32,
    lanes: u32,
    pad: [u32; 2],
}

fn encoded(op: &BlockOp) -> [u32; 4] {
    match *op {
        BlockOp::Eq(byte) => [0, 0, 0, u32::from(byte)],
        BlockOp::Const(c) => [1, 0, c as u32, (c >> 32) as u32],
        BlockOp::Class { table, bits } => [7, 0, 0, u32::from(bits) | (u32::from(table) << 8)],
        BlockOp::Not(a) => [2, u32::from(a), 0, 0],
        BlockOp::And(a, b) => [3, u32::from(a), u32::from(b), 0],
        BlockOp::Or(a, b) => [4, u32::from(a), u32::from(b), 0],
        BlockOp::Xor(a, b) => [5, u32::from(a), u32::from(b), 0],
        BlockOp::AndNot(a, b) => [6, u32::from(a), u32::from(b), 0],
    }
}

/// The interpreter: every output's words over `bytes`, as 64-bit words of the stream.
pub(in crate::code_rule_dsl) fn interpret(
    program: &BlockProgram,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    let steps: Vec<[u32; 4]> = program.ops.iter().map(encoded).collect();
    let name = format!("block_program_{}", width.bytes());
    let pipeline = pipeline_named(SOURCE, &name)?;
    let slots = program.ops.len();
    // Every table as 32 bytes (`lo`, then `hi`); one zero table when there are none.
    let mut tables: Vec<u8> = program.tables.iter().flat_map(|t| t.lo.iter().chain(&t.hi).copied()).collect();
    tables.resize(tables.len().max(32), 0);
    let words = dispatch(&pipeline, bytes, width, slots, |encoder, lanes| {
        let args = BlockArgs { ops: slots as u32, lanes, pad: [0; 2] };
        encoder.set_bytes(2, std::mem::size_of::<BlockArgs>() as u64, std::ptr::from_ref(&args).cast());
        let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
        let steps_buffer = MappedBuffer::from_slice(device, &steps)?;
        encoder.set_buffer(3, Some(&steps_buffer.buffer), 0);
        let tables_buffer = MappedBuffer::from_slice(device, &tables)?;
        encoder.set_buffer(4, Some(&tables_buffer.buffer), 0);
        Ok((steps_buffer, tables_buffer))
    })?;
    Ok(program.outputs.iter().map(|&v| finish(&words[usize::from(v)], bytes.len())).collect())
}

/// SWAR byte compare, emitted once into every generated kernel (the interpreter's `eq16`).
const EQ16: &str = "static inline uint eq16(uint4 v, uint c4) {
  const uint4 x = v ^ c4;
  const uint4 z = ~(((x & 0x7f7f7f7fu) + 0x7f7f7f7fu) | x) & 0x80808080u;
  const uint4 g = (((z >> 7u) * 0x00204081u) >> 21u) & 0xfu;
  return g.x | (g.y << 4u) | (g.z << 8u) | (g.w << 12u);
}
";

/// How generated MSL computes `class` (being evaluated; `docs/parser-generators/12`):
///
/// - `Lookup`: simdjson's form, byte by byte: two lookups per byte in `constant` tables give each
///   byte's class bits, then a SWAR nonzero test of the selected bits gives the mask.
/// - `Planes`: the lane's bytes transposed into eight bit-planes (plane `k`: bit `k` of every
///   byte), then each class is a boolean formula of the planes: an OR over class bits of a
///   function of the low four planes and one of the high four, each written out by Shannon
///   expansion. Only word operations; no memory but the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::code_rule_dsl) enum ClassStrategy {
    Lookup,
    Planes,
}

impl ClassStrategy {
    pub(in crate::code_rule_dsl) const ALL: [ClassStrategy; 2] = [ClassStrategy::Lookup, ClassStrategy::Planes];
}

/// SWAR nonzero bytes: 16 bytes that are not zero as 16 bits.
const NZ16: &str = "static inline uint nz16(uint4 x) {
  const uint4 z = (((x & 0x7f7f7f7fu) + 0x7f7f7f7fu) | x) & 0x80808080u;
  const uint4 g = (((z >> 7u) * 0x00204081u) >> 21u) & 0xfu;
  return g.x | (g.y << 4u) | (g.z << 8u) | (g.w << 12u);
}
";

/// Hacker's Delight's 8x8 bit transpose: byte `i` bit `j` to byte `j` bit `i`.
const TRANSPOSE8: &str = "static inline ulong transpose8(ulong x) {
  ulong t = (x ^ (x >> 7u)) & 0x00AA00AA00AA00AAul;
  x = x ^ t ^ (t << 7u);
  t = (x ^ (x >> 14u)) & 0x0000CCCC0000CCCCul;
  x = x ^ t ^ (t << 14u);
  t = (x ^ (x >> 28u)) & 0x00000000F0F0F0F0ul;
  return x ^ t ^ (t << 28u);
}
";

/// The native IR as one straight-line MSL kernel, `tempo_block`, for lanes of `width`.
pub(in crate::code_rule_dsl) fn generate(native: &BlockNative, width: LaneWidth) -> String {
    generate_with(native, width, ClassStrategy::Lookup)
}

/// A class formula's value: a constant, or a named lane word.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Term {
    Zero,
    Ones,
    Named(String),
}

/// Formulas over planes, written out as named lane words and shared.
struct Formulas {
    m: &'static str,
    lines: Vec<String>,
    next: usize,
    memo: std::collections::HashMap<(usize, usize, u16), Term>,
    combined: std::collections::HashMap<(String, Term, Term), Term>,
}

impl Formulas {
    fn emit(&mut self, rhs: String) -> Term {
        let name = format!("f{}", self.next);
        self.next += 1;
        self.lines.push(format!("  const {} {name} = {}({rhs});\n", self.m, self.m));
        Term::Named(name)
    }

    /// The function of planes `base..base + vars` whose truth table (indexed by the planes' bits)
    /// is the low `2^vars` bits of `tt`, by Shannon expansion on the highest plane.
    fn function(&mut self, base: usize, vars: usize, tt: u16) -> Term {
        let size = 1u32 << vars;
        let full = if size == 16 { u16::MAX } else { (1u16 << size) - 1 };
        let tt = tt & full;
        if tt == 0 {
            return Term::Zero;
        }
        if tt == full {
            return Term::Ones;
        }
        if let Some(t) = self.memo.get(&(base, vars, tt)) {
            return t.clone();
        }
        let half = size / 2;
        let low_mask = (1u16 << half) - 1;
        let (t0, t1) = (tt & low_mask, (tt >> half) & low_mask);
        let f0 = self.function(base, vars - 1, t0);
        let f1 = self.function(base, vars - 1, t1);
        let x = format!("p{}", base + vars - 1);
        let complement = t1 == !t0 & low_mask;
        let term = self.combine(&x, f0, f1, complement);
        self.memo.insert((base, vars, tt), term.clone());
        term
    }

    /// `x ? f1 : f0` bitwise, folded.
    fn combine(&mut self, x: &str, f0: Term, f1: Term, complement: bool) -> Term {
        let key = (x.to_owned(), f0.clone(), f1.clone());
        if let Some(t) = self.combined.get(&key) {
            return t.clone();
        }
        let n = |t: &Term| match t {
            Term::Named(n) => n.clone(),
            _ => unreachable!(),
        };
        let term = match (&f0, &f1) {
            _ if f0 == f1 => f0.clone(),
            (Term::Zero, Term::Ones) => Term::Named(x.to_owned()),
            (Term::Ones, Term::Zero) => self.emit(format!("~{x}")),
            (Term::Zero, _) => self.emit(format!("{x} & {}", n(&f1))),
            (_, Term::Zero) => self.emit(format!("{} & ~{x}", n(&f0))),
            (_, Term::Ones) => self.emit(format!("{x} | {}", n(&f0))),
            (Term::Ones, _) => self.emit(format!("{} | ~{x}", n(&f1))),
            _ if complement => self.emit(format!("{x} ^ {}", n(&f0))),
            _ => self.emit(format!("{a} ^ (({a} ^ {b}) & {x})", a = n(&f0), b = n(&f1))),
        };
        self.combined.insert(key, term.clone());
        term
    }

    fn and(&mut self, a: Term, b: Term) -> Term {
        match (a, b) {
            (Term::Zero, _) | (_, Term::Zero) => Term::Zero,
            (Term::Ones, t) | (t, Term::Ones) => t,
            (Term::Named(a), Term::Named(b)) => self.emit(format!("{a} & {b}")),
        }
    }

    fn or(&mut self, a: Term, b: Term) -> Term {
        match (a, b) {
            (Term::Ones, _) | (_, Term::Ones) => Term::Ones,
            (Term::Zero, t) | (t, Term::Zero) => t,
            (Term::Named(a), Term::Named(b)) => self.emit(format!("{a} | {b}")),
        }
    }

    fn word(&self, t: &Term) -> String {
        match t {
            Term::Zero => format!("{}(0)", self.m),
            Term::Ones => format!("{}(~{}(0))", self.m, self.m),
            Term::Named(n) => n.clone(),
        }
    }
}

/// The lane's eight bit-planes `p0..p7` from its chunks `c0..`: `transpose8` on each 8 bytes,
/// then byte `k` of every group gathered into plane `k` (for 64-byte lanes, an 8x8 byte
/// transpose).
fn planes(width: LaneWidth) -> String {
    let m = width.word();
    let groups = width.bytes() / 8;
    let mut out = String::new();
    for g in 0..groups {
        let (c, half) = (g / 2, if g % 2 == 0 { ("x", "y") } else { ("z", "w") });
        out.push_str(&format!(
            "  ulong t{g} = transpose8(ulong(c{c}.{}) | (ulong(c{c}.{}) << 32u));\n",
            half.0, half.1
        ));
    }
    if groups == 8 {
        for (sh, mask, step) in [(32, 0x0000_0000_FFFF_FFFFu64, 4), (16, 0x0000_FFFF_0000_FFFF, 2), (8, 0x00FF_00FF_00FF_00FF, 1)] {
            for g in (0..8).filter(|g| g & step == 0) {
                let h = g + step;
                out.push_str(&format!(
                    "  {{ const ulong a = t{g}, b = t{h}; t{g} = (a & {mask:#018x}ul) | ((b & {mask:#018x}ul) << {sh}u); \
                     t{h} = ((a >> {sh}u) & {mask:#018x}ul) | (b & ~{mask:#018x}ul); }}\n"
                ));
            }
        }
        for k in 0..8 {
            out.push_str(&format!("  const ulong p{k} = t{k};\n"));
        }
    } else {
        for k in 0..8 {
            let parts: Vec<String> = (0..groups)
                .map(|g| {
                    let byte = format!("((t{g} >> {}u) & 0xfful)", k * 8);
                    if g == 0 { byte } else { format!("({byte} << {}u)", g * 8) }
                })
                .collect();
            out.push_str(&format!("  const {m} p{k} = {m}({});\n", parts.join(" | ")));
        }
    }
    out
}

/// One table's class bytes of a `uint4` by lookups: byte `j`'s `lo[b & 15] & hi[b >> 4]`.
fn lookup_helper(t: usize, table: &super::block::ClassTable) -> String {
    let hex = |xs: &[u8]| xs.iter().map(|x| format!("{x:#04x}")).collect::<Vec<_>>().join(", ");
    let mut out = format!(
        "constant uchar T{t}[32] = {{{}, {}}};\nstatic inline uint4 classes_t{t}(uint4 v) {{\n  uint4 k;\n",
        hex(&table.lo),
        hex(&table.hi)
    );
    for lane in ["x", "y", "z", "w"] {
        let bytes: Vec<String> = (0..4)
            .map(|j| {
                let b = format!("uint(T{t}[(v.{lane} >> {}u) & 15u] & T{t}[16u + ((v.{lane} >> {}u) & 15u)])", 8 * j, 8 * j + 4);
                if j == 0 { b } else { format!("({b} << {}u)", 8 * j) }
            })
            .collect();
        out.push_str(&format!("  k.{lane} = {};\n", bytes.join(" | ")));
    }
    out.push_str("  return k;\n}\n");
    out
}

/// The native IR as one straight-line MSL kernel, `tempo_block`, for lanes of `width`, with
/// `class` computed by `strategy`.
pub(in crate::code_rule_dsl) fn generate_with(native: &BlockNative, width: LaneWidth, strategy: ClassStrategy) -> String {
    let m = width.word();
    let chunks = width.bytes() / 16;
    let bits = width.bytes();
    let classes = native.insts.iter().any(|i| matches!(i, BlockInst::Class { .. }));
    let mut helpers = String::new();
    if classes {
        match strategy {
            ClassStrategy::Lookup => {
                helpers.push_str(NZ16);
                for (t, table) in native.tables.iter().enumerate() {
                    helpers.push_str(&lookup_helper(t, table));
                }
            }
            ClassStrategy::Planes => helpers.push_str(TRANSPOSE8),
        }
    }
    let mut out = format!(
        "#include <metal_stdlib>\nusing namespace metal;\n\n{EQ16}\n{helpers}\n\
         kernel void tempo_block(\n\
         \x20   device const uint4* bytes [[buffer(0)]],\n\
         \x20   device {m}* out [[buffer(1)]],\n\
         \x20   constant uint& lanes [[buffer(2)]],\n\
         \x20   uint lane [[thread_position_in_grid]]) {{\n"
    );
    if native.insts.iter().any(|i| matches!(i, BlockInst::EqByte(_) | BlockInst::Class { .. })) {
        for c in 0..chunks {
            out.push_str(&format!("  const uint4 c{c} = bytes[lane * {chunks}u + {c}u];\n"));
        }
    }
    let mut formulas = Formulas { m, lines: vec![], next: 0, memo: Default::default(), combined: Default::default() };
    if classes {
        match strategy {
            ClassStrategy::Lookup => {
                let used: std::collections::BTreeSet<u16> = native
                    .insts
                    .iter()
                    .filter_map(|i| if let BlockInst::Class { table, .. } = *i { Some(table) } else { None })
                    .collect();
                for t in used {
                    for c in 0..chunks {
                        out.push_str(&format!("  const uint4 k{t}_{c} = classes_t{t}(c{c});\n"));
                    }
                }
            }
            ClassStrategy::Planes => out.push_str(&planes(width)),
        }
    }
    for (at, inst) in native.insts.iter().enumerate() {
        let rhs = match *inst {
            BlockInst::Class { table, bits: sel } if strategy == ClassStrategy::Lookup => {
                let s4 = u32::from(sel) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("{m}(nz16(k{table}_{c} & {s4:#010x}u))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            BlockInst::Class { table, bits: sel } => {
                let t = native.tables[usize::from(table)];
                let mut word = Term::Zero;
                for c in (0..8).filter(|c| sel >> c & 1 == 1) {
                    let lo = (0..16).fold(0u16, |a, n| a | (u16::from(t.lo[n] >> c & 1) << n));
                    let hi = (0..16).fold(0u16, |a, n| a | (u16::from(t.hi[n] >> c & 1) << n));
                    let l = formulas.function(0, 4, lo);
                    let h = formulas.function(4, 4, hi);
                    let both = formulas.and(l, h);
                    word = formulas.or(word, both);
                }
                for line in formulas.lines.drain(..) {
                    out.push_str(&line);
                }
                formulas.word(&word)
            }
            BlockInst::EqByte(byte) => {
                let c4 = u32::from(byte) * 0x0101_0101;
                (0..chunks)
                    .map(|c| {
                        let part = format!("{m}(eq16(c{c}, {c4:#010x}u))");
                        if c == 0 { part } else { format!("({part} << {}u)", c * 16) }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
            BlockInst::Const(c) if bits == 64 => format!("{c:#018x}ul"),
            BlockInst::Const(c) => format!("{m}({c:#018x}ul >> ((lane * {bits}u) & 63u))"),
            BlockInst::Not(a) => format!("{m}(~v{a})"),
            BlockInst::And(a, b) => format!("v{a} & v{b}"),
            BlockInst::Or(a, b) => format!("v{a} | v{b}"),
            BlockInst::Xor(a, b) => format!("v{a} ^ v{b}"),
            BlockInst::AndNot(a, b) => format!("{m}(v{a} & ~v{b})"),
        };
        out.push_str(&format!("  const {m} v{at} = {rhs};\n"));
    }
    for (k, v) in native.outputs.iter().enumerate() {
        out.push_str(&format!("  out[{k}u * lanes + lane] = v{v};\n"));
    }
    out.push_str("}\n");
    out
}

/// Generated MSL run: every output's words over `bytes`, as 64-bit words of the stream.
pub(in crate::code_rule_dsl) fn run_generated(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
) -> Result<Vec<Vec<u64>>, Error> {
    run_generated_with(native, bytes, width, ClassStrategy::Lookup).map(|(words, _)| words)
}

/// Generated MSL run with `class` by `strategy`: the outputs, and the dispatch's GPU seconds.
pub(in crate::code_rule_dsl) fn run_generated_with(
    native: &BlockNative,
    bytes: &[u8],
    width: LaneWidth,
    strategy: ClassStrategy,
) -> Result<(Vec<Vec<u64>>, f64), Error> {
    let pipeline = pipeline_named(&generate_with(native, width, strategy), "tempo_block")?;
    let mut seconds = 0.0;
    let words = dispatch_timed(&pipeline, bytes, width, native.outputs.len(), &mut seconds, |encoder, lanes| {
        encoder.set_bytes(2, std::mem::size_of::<u32>() as u64, std::ptr::from_ref(&lanes).cast());
        Ok(())
    })?;
    Ok((words.iter().map(|w| finish(w, bytes.len())).collect(), seconds))
}

/// One dispatch over `bytes`, padded to whole SIMD groups of lanes, with `slots` slots of lane
/// words; returns each slot as 64-bit words of the stream (lane words are little-endian and in
/// lane order, so they are the stream's words).
fn dispatch<T>(
    pipeline: &metal::ComputePipelineState,
    bytes: &[u8],
    width: LaneWidth,
    slots: usize,
    bind: impl FnOnce(&metal::ComputeCommandEncoderRef, u32) -> Result<T, Error>,
) -> Result<Vec<Vec<u64>>, Error> {
    dispatch_timed(pipeline, bytes, width, slots, &mut 0.0, bind)
}

/// `dispatch`, also giving the command's GPU seconds.
fn dispatch_timed<T>(
    pipeline: &metal::ComputePipelineState,
    bytes: &[u8],
    width: LaneWidth,
    slots: usize,
    seconds: &mut f64,
    bind: impl FnOnce(&metal::ComputeCommandEncoderRef, u32) -> Result<T, Error>,
) -> Result<Vec<Vec<u64>>, Error> {
    let device = METAL_DEVICE.as_ref().ok_or(Error::UnsupportedBackend)?;
    let _scope = super::metal_backend::launch::LaunchScope::enter();
    let span = width.bytes() * GROUP;
    let padded_len = bytes.len().div_ceil(span).max(1) * span;
    let mut flat = bytes.to_vec();
    flat.resize(padded_len, 0);
    let lanes = padded_len / width.bytes();
    let slot_bytes = padded_len / 8;
    let source = MappedBuffer::from_slice(device, &flat)?;
    let values = MappedBuffer::zeroed::<u8>(device, slots.max(1) * slot_bytes)?;
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    command.set_label("Rerail block program");
    let encoder = command.new_compute_command_encoder();
    encoder.set_label("block_program");
    encoder.set_compute_pipeline_state(pipeline);
    encoder.set_buffer(0, Some(&source.buffer), 0);
    encoder.set_buffer(1, Some(&values.buffer), 0);
    let _kept = bind(encoder, lanes as u32)?;
    encoder.checked_dispatch_thread_groups(
        command,
        MTLSize::new((lanes / GROUP) as u64, 1, 1),
        MTLSize::new(GROUP as u64, 1, 1),
    );
    encoder.end_encoding();
    commit_checked(command)?;
    wait_checked(command)?;
    *seconds = super::metal_backend::launch::gpu_seconds(command);
    let base = values.buffer.contents().cast::<u8>();
    let words = bytes.len().div_ceil(64);
    Ok((0..slots)
        .map(|slot| {
            // SAFETY: the buffer holds `slots * slot_bytes` bytes, written by the kernel.
            let raw = unsafe { std::slice::from_raw_parts(base.add(slot * slot_bytes), slot_bytes) };
            raw.chunks(8).take(words).map(|w| u64::from_le_bytes(w.try_into().unwrap())).collect()
        })
        .collect())
}

fn finish(words: &[u64], len: usize) -> Vec<u64> {
    let mut words = words.to_vec();
    mask_tail(&mut words, len);
    words
}
